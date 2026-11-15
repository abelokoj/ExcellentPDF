//! Edits to the shown document (K.14, K.15), as the app makes them: each one applied as one step of undo, undo and
//! redo, and saving. The file is saved 2 s after the last edit, when another tab is shown and as the app quits, on a
//! background thread except at quit. When a file opens with edits a crash left unsaved, the reader is offered them
//! back.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use excellent_core::document::{self, Document};
use excellent_core::edit::Operation;
use excellent_core::shared::registry;
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
    /// The files whose unsaved edits from a crash the reader was asked about this run.
    asked: Stored<Vec<PathBuf>>,
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
        };
        // Another tab shown (or this one closed): what was edited is saved, and a file with a crash's edits offers
        // them.
        effect(move || {
            let shown = tabs.shown_path();
            untracked(|| {
                editing.save_now();
                if let Some(path) = shown {
                    editing.offer_recovery(&path);
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
        registry()
            .open(&path)
            .ok()
            .filter(|document| document.is_editable())
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
        match document.apply(operation) {
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

    /// The document changed: views read it again, and it is saved once the reader stops.
    fn edited(self, document: Arc<Document>) {
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
                documents
                    .into_iter()
                    .filter_map(|document| {
                        let err = document.save().err()?;
                        Some((document.path().to_path_buf(), err.to_string()))
                    })
                    .collect::<Vec<_>>()
            });
            match saving.await {
                Ok(failures) => {
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
            }
        }
    }

    /// Offers the edits a crash left unsaved in the file at `path`, once a run.
    fn offer_recovery(self, path: &Path) {
        let Some(document) = registry().open(path).ok() else {
            return;
        };
        let asked_before = self.asked.with(|asked| {
            let before = asked.iter().any(|asked| asked == path);
            if !before {
                asked.push(path.to_path_buf());
            }
            before
        });
        if asked_before || !document.is_recoverable() {
            return;
        }
        let name = file_name(path);
        spawn(async move {
            let choice = alert_dialog()
                .title(t!("recover.title", name = name))
                .description(t!("recover.body"))
                .action("recover", t!("recover.recover"))
                .destructive("discard", t!("recover.discard"))
                .show()
                .await;
            match choice.ok().and_then(|result| result.action).as_deref() {
                Some("recover") => match document.recover() {
                    Ok(()) => self.edited(document),
                    Err(err) => show_problem(t!("recover.failed"), &err),
                },
                Some("discard") => document.discard_recovery(),
                // Closed without a choice: asked again when the file next opens.
                _ => {}
            }
        });
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
