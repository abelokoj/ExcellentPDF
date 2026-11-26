//! Edits to the shown document (K.14, K.15), as the app makes them: each one applied as one step of undo, undo and
//! redo, and saving. The file is saved 2 s after the last edit, when another tab is shown and as the app quits, on a
//! background thread except at quit. When a file opens with edits a crash left unsaved, they are brought back at
//! once, and a bar over the pages lists them for the reader to keep or discard (A.15). Until then they aren't saved:
//! closed undecided, they are brought back again next time.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use excellent_core::document::{self, Document};
use excellent_core::edit::{Change, Operation};
use excellent_core::shared::registry;
use excellent_features::annotate::layers;
use nsh_base::alert_dialog;
use nsh_core::element::desktop::on_app_quit;
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
                                saved.push(document);
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
                    saved.iter().for_each(crate::annotate::tag_index::index);
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
            Ok(()) => self.revision.update(|revision| *revision += 1),
            Err(err) => show_problem(t!("recover.discard_failed"), &err),
        }
    }

    /// The edits brought back for the file at `path` are decided: its bar goes.
    fn settle(self, path: &Path) {
        let waiting = untracked(|| {
            self.recovered
                .with(|all| all.iter().any(|r| r.path == path))
        });
        if waiting {
            self.recovered.update(|all| all.retain(|r| r.path != path));
        }
    }
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
            .show()
            .await;
        if let Err(err) = shown {
            log::warn!("cannot show a message: {err}");
        }
    });
}
