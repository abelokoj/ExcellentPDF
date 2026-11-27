//! Edits to the shown document (K.14, K.15), as the app makes them: each one applied as one step of undo, undo and
//! redo, and saving. The file is saved 2 s after the last edit, when another tab is shown and as the app quits, on a
//! background thread except at quit. When a file opens with edits a crash left unsaved, they are brought back at
//! once, and a bar over the pages lists them for the reader to keep or discard (A.15). Until then they aren't saved:
//! closed undecided, they are brought back again next time.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ecclesia_core::document::{self, Document};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_core::shared::registry;
use ecclesia_features::annotate::layers;
use nsh_base::alert_dialog;
use nsh_core::element::desktop::on_app_quit;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::tabs::TabState;

/// How long the reader must stop editing before the file is saved.
const SAVE_DELAY: Duration = Duration::from_secs(2);

/// What edits the shown document, and what waits to be saved.
#[derive(Clone, Copy)]
pub(crate) struct Editing {
    tabs: TabState,
    /// Goes up with every edit made, undone, redone, recovered or saved here: views of what edits change (the
    /// outline) read the document again when it moves.
    revision: Signal<u64>,
    /// The documents edited since they were last saved: a tab shown no more is still saved.
    unsaved: Stored<Vec<Arc<Document>>>,
    /// The save waiting for the reader to stop editing.
    pending: Stored<Option<Spawned<()>>>,
    /// The files whose unsaved edits from a crash were looked for this run.
    asked: Stored<Vec<PathBuf>>,
    /// Edits brought back from a crash that the reader hasn't kept or discarded yet.
    recovered: Signal<Vec<Recovered>>,
    /// Most recent on-disk modification times written by Ecclesia PDF itself, so the external file
    /// watcher absorbs our own saves without triggering a reload.
    last_internal_save: Stored<Vec<(PathBuf, SystemTime)>>,
}

/// `operation` with each new annotation on its author's layer, when the document has one (A.22): a reviewer who has
/// a layer keeps annotating on it.
fn on_author_layers<'a>(document: &Document, operation: &'a Operation) -> Cow<'a, Operation> {
    let creates = operation
        .changes
        .iter()
        .any(|change| matches!(change, Change::CreateAnnotation(_)));
    if !creates {
        return Cow::Borrowed(operation);
    }
    let all = match document.mupdf() {
        Ok(shared) => shared
            .lock()
            .expect("document lock poisoned")
            .layers()
            .map_err(|err| log::warn!("layers: cannot read them: {err}"))
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    if all.is_empty() {
        return Cow::Borrowed(operation);
    }
    Cow::Owned(Operation::new(
        operation.name.clone(),
        layers::place_new(&operation.changes, &all),
    ))
}

/// Edits a crash left unsaved, brought back when their file opened (A.15).
#[derive(Debug, Clone, PartialEq)]
struct Recovered {
    path: PathBuf,
    /// What each edit was, in the order made.
    changes: Vec<String>,
}

impl Editing {
    /// Edits for `tabs`' shown document, saved as the module says, until the current component goes. `revision` is
    /// the window's: every pane's views follow every pane's edits, since two panes may show one document (V.1).
    pub(crate) fn new(tabs: TabState, revision: Signal<u64>) -> Self {
        let editing = Self {
            tabs,
            revision,
            unsaved: stored(Vec::new()),
            pending: stored(None),
            asked: stored(Vec::new()),
            recovered: signal(Vec::new()),
            last_internal_save: stored(Vec::new()),
        };
        // Another tab shown (or this one closed): what was edited is saved, and a file with a crash's edits gets
        // them back.
        effect(move || {
            let shown = tabs.shown_path();
            untracked(|| {
                editing.save_now();
                if let Some(path) = shown {
                    editing.recover(&path);
                }
            });
        });
        on_app_quit(move || {
            editing.save_at_quit();
            async {}
        });
        editing
    }

    /// The edit count views follow (tracked).
    pub(crate) fn revision(self) -> u64 {
        self.revision.get()
    }

    /// The shown document, when it can be edited.
    pub(crate) fn document(self) -> Option<Arc<Document>> {
        let path = untracked(|| self.tabs.shown_path())?;
        let resolved = crate::expdf::resolve_open_path(&path);
        registry()
            .open(&resolved)
            .ok()
            .filter(|document| document.is_editable())
    }

    /// Has the views draw the shown document again, its file unchanged: which annotations it hides changed (A.19).
    pub(crate) fn redraw(self) {
        self.revision.update(|revision| *revision += 1);
    }

    /// Whether the document at `path` currently has unsaved local edits in memory (including
    /// unkept crash-recovered edits).
    pub(crate) fn has_unsaved_changes(self, path: &Path) -> bool {
        let in_unsaved = self.unsaved.with(|unsaved| {
            unsaved.iter().any(|doc| {
                (doc.path() == path || same_file(doc.path(), path)) && doc.has_unsaved_changes()
            })
        });
        if in_unsaved {
            return true;
        }
        untracked(|| {
            self.recovered.with(|recovered| {
                recovered.iter().any(|entry| {
                    let resolved = crate::expdf::resolve_open_path(&entry.path);
                    entry.path == path || resolved == path || same_file(&resolved, path)
                })
            })
        })
    }

    /// Records that Ecclesia PDF just saved `path` with on-disk modification time `mtime` so the
    /// external file watcher ignores our own save.
    pub(crate) fn note_saved(self, path: &Path, mtime: SystemTime) {
        self.last_internal_save.with(|entries| {
            if let Some((_, recorded)) = entries
                .iter_mut()
                .find(|(existing, _)| existing == path || same_file(existing, path))
            {
                *recorded = mtime;
            } else {
                entries.push((path.to_path_buf(), mtime));
            }
        });
    }

    /// Whether `stamp_mtime` matches Ecclesia PDF's most recent internal save of `path`.
    pub(crate) fn was_recently_saved(self, path: &Path, stamp_mtime: SystemTime) -> bool {
        /// Tolerance for the fallback timestamp sampled via `SystemTime::now()` right after a save
        /// if `metadata().modified()` was temporarily unavailable.
        const FALLBACK_WINDOW: Duration = Duration::from_secs(1);

        self.last_internal_save.with(|entries| {
            entries.iter().any(|(existing, recorded)| {
                if existing != path && !same_file(existing, path) {
                    return false;
                }
                stamp_mtime == *recorded
                    || recorded
                        .duration_since(stamp_mtime)
                        .is_ok_and(|delta| delta <= FALLBACK_WINDOW)
            })
        })
    }

    /// Clears stale local edit tracking after `path` is reloaded from disk and bumps the revision
    /// so outline, thumbnails, annotations, and SyncTeX all refresh.
    pub(crate) fn on_external_reload(self, path: &Path) {
        self.unsaved.with(|unsaved| {
            unsaved.retain(|doc| doc.path() != path && !same_file(doc.path(), path));
        });
        self.settle(path);
        self.tabs.set_dirty_resolved(path, false);
        self.redraw();
    }

    /// Whether the shown document can be edited (a PDF).
    pub(crate) fn can_edit(self) -> bool {
        self.document().is_some()
    }

    /// Makes `operation` on the shown document, as one step of undo. Returns whether it was made; a failure is shown
    /// to the reader.
    pub(crate) fn apply(self, operation: &Operation) -> bool {
        let Some(document) = self.document() else {
            return false;
        };
        let operation = on_author_layers(&document, operation);
        match document.apply(&operation) {
            Ok(()) => {
                self.edited(document);
                true
            }
            Err(err) => {
                show_problem(t!("edit.failed", action = operation.name.clone()), &err);
                false
            }
        }
    }

    /// Takes back the shown document's last edit.
    pub(crate) fn undo(self) {
        self.step(Document::undo, "edit.undo_failed");
    }

    /// Makes the shown document's last undone edit again.
    pub(crate) fn redo(self) {
        self.step(Document::redo, "edit.redo_failed");
    }

    fn step(self, step: fn(&Document) -> document::Result<bool>, problem: &'static str) {
        let Some(document) = self.document() else {
            return;
        };
        match step(&document) {
            Ok(true) => self.edited(document),
            Ok(false) => {}
            Err(err) => show_problem(t!(problem), &err),
        }
    }

    /// The document changed: views read it again, and it is saved once the reader stops. Edits brought back from a
    /// crash are kept with it.
    fn edited(self, document: Arc<Document>) {
        self.settle(document.path());
        self.tabs.set_dirty_resolved(document.path(), true);
        self.revision.update(|revision| *revision += 1);
        self.unsaved.with(|unsaved| {
            if !unsaved.iter().any(|kept| Arc::ptr_eq(kept, &document)) {
                unsaved.push(document);
            }
        });
        if let Some(task) = self.pending.replace(None) {
            task.cancel();
        }
        self.pending
            .replace(Some(set_timeout(SAVE_DELAY, move || self.save_now())));
    }

    /// Saves every edited document now, on a background thread; a file that can't be saved is shown to the reader
    /// and stays unsaved (its log keeps the edits).
    pub(crate) fn save_now(self) {
        if let Some(task) = self.pending.replace(None) {
            task.cancel();
        }
        let documents = self.unsaved.replace(Vec::new());
        if documents.is_empty() {
            return;
        }
        spawn(async move {
            let saving = blocking(move || {
                let (mut saved, mut failures) = (Vec::new(), Vec::new());
                for document in documents {
                    match document.save() {
                        Ok(()) => {
                            if let Err(err) =
                                crate::expdf::sync_working_pdf_to_expdf(document.path())
                            {
                                failures.push((document.path().to_path_buf(), err));
                            } else {
                                let mtime = std::fs::metadata(document.path())
                                    .and_then(|meta| meta.modified())
                                    .unwrap_or_else(|_| SystemTime::now());
                                saved.push((document, mtime));
                            }
                        }
                        Err(err) => {
                            failures.push((document.path().to_path_buf(), err.to_string()));
                        }
                    }
                }
                (saved, failures)
            });
            match saving.await {
                Ok((saved, failures)) => {
                    for (document, mtime) in &saved {
                        self.note_saved(document.path(), *mtime);
                        crate::annotate::tag_index::index(document);
                        self.tabs.set_dirty_resolved(document.path(), false);
                    }
                    for (path, err) in failures {
                        log::warn!("save: {}: {err}", path.display());
                        show_message(t!("save.failed", name = file_name(&path)), err);
                    }
                }
                Err(err) => log::error!("save: the saving thread failed: {err}"),
            }
            // A file rewritten whole is a new document underneath: views read it again.
            self.revision.update(|revision| *revision += 1);
        });
    }

    /// Asks where to save a copy of the shown PDF (including unsaved edits) and writes it atomically.
    pub(crate) fn save_as_copy(self) {
        let Some(document) = self.document() else {
            return;
        };
        let original = document.path().to_path_buf();
        let stem = original
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dialog = SaveDialog::new()
            .title(t!("tabs.save_new.title"))
            .file_name(format!("{stem}.pdf"))
            .filter(FileFilter::new(t!("open.filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(err) => {
                    log::warn!("save_as_copy: the dialog failed: {err}");
                    return;
                }
            };
            if same_file(&path, &original) {
                self.save_now();
                return;
            }
            let mupdf = match document.mupdf() {
                Ok(mupdf) => mupdf,
                Err(err) => {
                    show_message(t!("save.failed", name = file_name(&path)), err.to_string());
                    return;
                }
            };
            let target = path.clone();
            let written = blocking(move || -> Result<(), String> {
                let temp = target.with_extension("pdf.tmp");
                let saved = mupdf
                    .lock()
                    .expect("document lock poisoned")
                    .save_pdf(&temp);
                if let Err(err) = saved {
                    let _ = std::fs::remove_file(&temp);
                    return Err(err.to_string());
                }
                std::fs::rename(&temp, &target).map_err(|err| err.to_string())
            })
            .await;
            if let Err(err) = written
                .map_err(|err| err.to_string())
                .and_then(|result| result)
            {
                log::warn!("save_as_copy: {}: {err}", path.display());
                show_message(t!("save.failed", name = file_name(&path)), err);
            }
        });
    }

    /// Discards unsaved in-memory edits on the shown PDF, reverting it to its on-disk version.
    pub(crate) fn discard_unsaved(self) {
        if let Some(task) = self.pending.replace(None) {
            task.cancel();
        }
        let Some(document) = self.document() else {
            return;
        };
        self.unsaved.with(|unsaved| {
            unsaved.retain(|kept| !Arc::ptr_eq(kept, &document));
        });
        self.settle(document.path());
        match document.revert() {
            Ok(()) => {
                self.tabs.set_dirty_resolved(document.path(), false);
                self.revision.update(|revision| *revision += 1);
            }
            Err(err) => show_problem(t!("tabs.discard_failed"), &err),
        }
    }

    /// Saves every edited document before the app goes. On the UI thread: the app is going, and a save left to a
    /// thread could be cut off. One that fails keeps its edits in the log, offered at the next start.
    fn save_at_quit(self) {
        if let Some(task) = self.pending.replace(None) {
            task.cancel();
        }
        for document in self.unsaved.replace(Vec::new()) {
            if let Err(err) = document.save() {
                log::error!("save at quit: {}: {err}", document.path().display());
            } else if let Err(err) = crate::expdf::sync_working_pdf_to_expdf(document.path()) {
                log::error!("save expdf at quit: {}: {err}", document.path().display());
            }
        }
    }

    /// Brings back the edits a crash left unsaved in the file at `path`, once a run, for the reader to keep or
    /// discard. Checks off the UI thread so opening or switching tabs never stalls on PDF parsing.
    fn recover(self, path: &Path) {
        let asked_before = self.asked.with(|asked| {
            let before = asked.iter().any(|asked| asked == path);
            if !before {
                asked.push(path.to_path_buf());
            }
            before
        });
        if asked_before {
            return;
        }
        let path = path.to_path_buf();
        spawn(async move {
            let resolved = crate::expdf::resolve_open_path(&path);
            let outcome = blocking(move || -> Option<document::Result<Vec<String>>> {
                let document = registry().open(&resolved).ok()?;
                if !document.is_recoverable() {
                    return None;
                }
                Some(document.recover())
            })
            .await;
            match outcome {
                Ok(Some(Ok(changes))) => {
                    self.tabs.set_dirty_resolved(&path, true);
                    self.revision.update(|revision| *revision += 1);
                    self.recovered
                        .update(|recovered| recovered.push(Recovered { path, changes }));
                }
                Ok(Some(Err(err))) => show_problem(t!("recover.failed"), &err),
                Ok(None) | Err(_) => {}
            }
        });
    }

    /// The edits brought back for the shown file that the reader hasn't kept or discarded (tracked); `None` when
    /// there are none.
    pub(crate) fn recovered(self) -> Option<Vec<String>> {
        let shown = self.tabs.shown_path()?;
        self.recovered.with(|recovered| {
            recovered
                .iter()
                .find(|r| r.path == shown)
                .map(|r| r.changes.clone())
        })
    }

    /// Keeps the shown file's recovered edits: saved like any other.
    pub(crate) fn keep_recovered(self) {
        if let Some(document) = self.document() {
            self.edited(document);
        }
    }

    /// Discards the shown file's recovered edits: the file stays as it is on disk.
    pub(crate) fn discard_recovered(self) {
        let Some(document) = self.document() else {
            return;
        };
        self.settle(document.path());
        match document.revert() {
            Ok(()) => {
                self.tabs.set_dirty_resolved(document.path(), false);
                self.revision.update(|revision| *revision += 1);
            }
            Err(err) => show_problem(t!("recover.discard_failed"), &err),
        }
    }

    /// The edits brought back for the file at `path` are decided: its bar goes.
    fn settle(self, path: &Path) {
        let matches_path = |entry: &Recovered| {
            entry.path == path || {
                let resolved = crate::expdf::resolve_open_path(&entry.path);
                resolved == path || same_file(&resolved, path)
            }
        };
        let waiting = untracked(|| self.recovered.with(|all| all.iter().any(matches_path)));
        if waiting {
            self.recovered
                .update(|all| all.retain(|entry| !matches_path(entry)));
        }
    }
}

/// Whether `a` and `b` point to the same file on disk.
fn same_file(a: &Path, b: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    canonical(a) == canonical(b)
}

/// The name of the file at `path`, as the reader knows it.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Tells the reader something failed: `title`, and what went wrong.
fn show_problem(title: String, err: &document::Error) {
    log::warn!("{title}: {err}");
    show_message(title, err.to_string());
}

/// Shows the reader `title` and `detail`, with one button to close them.
pub(crate) fn show_message(title: String, detail: String) {
    spawn(async move {
        let shown = alert_dialog()
            .title(title)
            .description(detail)
            .cancel("ok", t!("dialog.ok"))
            .close_on_backdrop_press(true)
            .close_on_escape(true)
            .show()
            .await;
        if let Err(err) = shown {
            log::warn!("cannot show a message: {err}");
        }
    });
}
