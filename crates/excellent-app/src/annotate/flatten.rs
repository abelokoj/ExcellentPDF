//! Flatten on export (A.25): a copy of the shown PDF, its edits included, with its annotations drawn into the pages
//! and taken out, so every reader shows them and none can change them. MuPDF bakes the copy
//! (`excellent_mupdf::Document::save_flattened`); the open document is never flattened.

use std::path::{Path, PathBuf};

use excellent_core::document::SharedMupdf;
use excellent_mupdf::Forms;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::editing::show_message;

impl AnnotateState {
    /// Asks where to save a flattened copy of the shown PDF, and writes it there.
    pub(crate) fn export_flattened(self) {
        let Some(document) = self.editing.document() else {
            return;
        };
        let original = document.path().to_path_buf();
        let stem = original
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dialog = SaveDialog::new()
            .title(t!("flatten.title"))
            .file_name(t!("flatten.file_name", name = stem))
            .filter(FileFilter::new(t!("flatten.filter"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(err) => {
                    log::warn!("flatten: the dialog failed: {err}");
                    return;
                }
            };
            // Flattening over the open file would take its annotations away for good.
            if same_file(&path, &original) {
                show_message(t!("flatten.same"), t!("flatten.same_detail"));
                return;
            }
            let mupdf = match document.mupdf() {
                Ok(mupdf) => mupdf,
                Err(err) => {
                    show_message(
                        t!("flatten.failed", name = file_name(&path)),
                        err.to_string(),
                    );
                    return;
                }
            };
            let target = path.clone();
            let written = blocking(move || write(&mupdf, &target)).await;
            match written
                .map_err(|err| err.to_string())
                .and_then(|result| result)
            {
                Ok(()) => show_message(
                    t!("flatten.done"),
                    t!("flatten.done_detail", name = file_name(&path)),
                ),
                Err(err) => {
                    log::warn!("flatten: {err}");
                    show_message(t!("flatten.failed", name = file_name(&path)), err);
                }
            }
        });
    }
}

/// Writes the flattened copy beside `path`, then renames it over `path`, so a failure never leaves half a file.
fn write(mupdf: &SharedMupdf, path: &Path) -> Result<(), String> {
    let temporary = path.with_extension("pdf.tmp");
    let saved = mupdf
        .lock()
        .expect("document lock poisoned")
        .save_flattened(&temporary, Forms::Keep);
    if let Err(err) = saved {
        let _ = std::fs::remove_file(&temporary);
        return Err(err.to_string());
    }
    std::fs::rename(&temporary, path).map_err(|err| err.to_string())
}

/// Whether `a` and `b` name the same file, however they are spelled.
fn same_file(a: &Path, b: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    canonical(a) == canonical(b)
}

/// `path`'s file name, for the messages.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
