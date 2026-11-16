//! Annotation exchange (A.24): the shown PDF's annotations written to an XFDF or FDF file, and such a file's annotations
//! merged in by id as one step of undo. `excellent-features`' `annotate::xfdf` reads and writes XFDF; MuPDF reads and
//! writes FDF (`excellent_core::annot::fdf_annotations`, `excellent_mupdf::Document::to_fdf`). The files are read and
//! written on the workers, the document's lock taken only while its pages are read.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use excellent_core::annot::{Annotation, fdf_annotations, load_all};
use excellent_core::document::{Document, SharedMupdf};
use excellent_core::edit::Operation;
use excellent_core::shared::pool;
use excellent_features::annotate::xfdf::{self, Merged, PageSpace};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::editing::show_message;
use crate::settings::write_atomically;

/// The two formats, by the file's extension: FDF for `.fdf`, XFDF for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Xfdf,
    Fdf,
}

impl Format {
    /// The format of the file at `path`, read from its extension, or from its first bytes when given.
    fn of(path: &Path, bytes: Option<&[u8]>) -> Self {
        let named_fdf = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("fdf"));
        if named_fdf || bytes.is_some_and(|bytes| bytes.starts_with(b"%FDF")) {
            Self::Fdf
        } else {
            Self::Xfdf
        }
    }
}

impl AnnotateState {
    /// Asks where to write the shown PDF's annotations, and writes them there as XFDF or FDF.
    pub(crate) fn export_annotations(self) {
        let Some(document) = self.editing.document() else {
            return;
        };
        let stem = document
            .path()
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dialog = SaveDialog::new()
            .file_name(format!("{stem}.xfdf"))
            .filter(FileFilter::new(t!("exchange.filter_xfdf"), ["xfdf"]))
            .filter(FileFilter::new(t!("exchange.filter_fdf"), ["fdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(err) => {
                    log::warn!("exchange: the dialog failed: {err}");
                    return;
                }
            };
            match export(&document, path.clone()).await {
                Ok(count) => show_message(
                    t!("exchange.exported", count = count.to_string()),
                    t!("exchange.exported_detail", name = file_name(&path)),
                ),
                Err(err) => {
                    log::warn!("exchange: export: {err}");
                    show_message(t!("exchange.export_failed", name = file_name(&path)), err);
                }
            }
        });
    }

    /// Asks for an XFDF or FDF file and merges its annotations into the shown PDF, then says what changed.
    pub(crate) fn import_annotations(self) {
        let Some(document) = self.editing.document() else {
            return;
        };
        let dialog = OpenDialog::files()
            .title(t!("exchange.pick"))
            .filter(FileFilter::new(t!("exchange.filter_any"), ["xfdf", "fdf"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(paths)) => paths.into_iter().next(),
                Ok(None) => None,
                Err(err) => {
                    log::warn!("exchange: the dialog failed: {err}");
                    None
                }
            };
            let Some(path) = path else {
                return;
            };
            match import(&document, path.clone()).await {
                Ok(merged) => self.merge(document.path().to_path_buf(), merged),
                Err(err) => {
                    log::warn!("exchange: import: {err}");
                    show_message(t!("exchange.import_failed", name = file_name(&path)), err);
                }
            }
        });
    }

    /// Applies `merged` to the file at `target`, should it still be the one shown, and says what changed.
    fn merge(self, target: PathBuf, merged: Merged) {
        let shown = self.editing.document();
        if !shown.is_some_and(|document| document.path() == target.as_path()) {
            return;
        }
        let Merged {
            changes,
            added,
            updated,
            unchanged,
            skipped,
        } = merged;
        if !changes.is_empty() {
            self.editing
                .apply(&Operation::new(t!("exchange.import_edit"), changes));
        }
        let title = if added + updated == 0 {
            t!("exchange.nothing")
        } else {
            t!("exchange.imported", count = (added + updated).to_string())
        };
        show_message(
            title,
            t!(
                "exchange.summary",
                added = added.to_string(),
                updated = updated.to_string(),
                unchanged = unchanged.to_string(),
                skipped = skipped.to_string()
            ),
        );
    }
}

/// Writes `document`'s annotations to `path` in the format its extension names; how many were written.
async fn export(document: &Document, path: PathBuf) -> Result<usize, String> {
    let mupdf = document.mupdf().map_err(|err| err.to_string())?;
    let annotations = load_all(pool(), Arc::clone(&mupdf))
        .await
        .map_err(|err| err.to_string())?;
    let file = file_name(document.path());
    blocking(move || -> Result<usize, String> {
        let count = annotations.len();
        let bytes = match Format::of(&path, None) {
            Format::Fdf => lock(&mupdf).to_fdf().map_err(|err| err.to_string())?,
            Format::Xfdf => xfdf::write(&annotations, &spaces(&mupdf)?, &file).into_bytes(),
        };
        write_atomically(&path, bytes).map_err(|err| err.to_string())?;
        Ok(count)
    })
    .await
    .map_err(|err| err.to_string())?
}

/// Reads the annotations of the file at `path` and finds what merging them into `document` changes.
async fn import(document: &Document, path: PathBuf) -> Result<Merged, String> {
    let mupdf = document.mupdf().map_err(|err| err.to_string())?;
    let existing = load_all(pool(), Arc::clone(&mupdf))
        .await
        .map_err(|err| err.to_string())?;
    blocking(move || -> Result<Merged, String> {
        let bytes = std::fs::read(&path).map_err(|err| err.to_string())?;
        let spaces = spaces(&mupdf)?;
        let imported: Vec<Annotation> = match Format::of(&path, Some(&bytes)) {
            Format::Fdf => fdf_annotations(&lock(&mupdf), &bytes).map_err(|err| err.to_string())?,
            Format::Xfdf => {
                let text = String::from_utf8_lossy(&bytes);
                xfdf::read(&text, &spaces).map_err(|err| err.to_string())?
            }
        };
        Ok(xfdf::merge(&existing, imported, spaces.len()))
    })
    .await
    .map_err(|err| err.to_string())?
}

/// Each page's map between page space and PDF space, taking the document's lock once.
fn spaces(mupdf: &SharedMupdf) -> Result<Vec<PageSpace>, String> {
    let document = lock(mupdf);
    let count = document.page_count().map_err(|err| err.to_string())?;
    (0..count)
        .map(|page| {
            document
                .page_transform(page)
                .map(PageSpace::new)
                .map_err(|err| err.to_string())
        })
        .collect()
}

fn lock(mupdf: &SharedMupdf) -> std::sync::MutexGuard<'_, excellent_mupdf::Document> {
    mupdf.lock().expect("document lock poisoned")
}

/// `path`'s file name, for the messages.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_is_told_by_extension_or_header() {
        assert_eq!(Format::of(Path::new("a.FDF"), None), Format::Fdf);
        assert_eq!(Format::of(Path::new("a.xfdf"), None), Format::Xfdf);
        assert_eq!(Format::of(Path::new("a"), Some(b"%FDF-1.2")), Format::Fdf);
        assert_eq!(Format::of(Path::new("a.xml"), Some(b"<?xml")), Format::Xfdf);
    }
}
