//! Transferring annotations from another version of the shown PDF (A.16): the reader picks the other version, both
//! are read on the render workers, `excellent-features`' `annotate::transfer` finds each annotation again, and a
//! review says how many were placed, moved and kept in place and names those that weren't found, before they are
//! added as one step of undo.

use std::path::PathBuf;
use std::sync::Arc;

use excellent_core::annot::{load_all, load_text};
use excellent_core::document::Document;
use excellent_core::edit::Operation;
use excellent_core::shared::{pool, registry};
use excellent_features::annotate::transfer::{self, Fate, Transferred};
use nsh_base::alert_dialog;
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::editing::show_message;

/// The annotations not found that the review names; the rest are counted.
const NAMED: usize = 5;

/// The characters of an annotation's text the review shows.
const NAME_LENGTH: usize = 48;

impl AnnotateState {
    /// Asks for another version of the shown PDF, and offers its annotations here.
    pub(crate) fn transfer(self) {
        let Some(target) = self.editing.document() else {
            return;
        };
        let dialog = OpenDialog::files()
            .title(t!("transfer.pick"))
            .filter(FileFilter::new(t!("transfer.filter"), ["pdf"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let source = match answer.await {
                Ok(Some(paths)) => paths.into_iter().next(),
                Ok(None) => None,
                Err(err) => {
                    log::warn!("transfer: the dialog failed: {err}");
                    None
                }
            };
            let Some(source) = source else {
                return;
            };
            if source == target.path() {
                show_message(t!("transfer.same"), t!("transfer.same_detail"));
                return;
            }
            match read(source, Arc::clone(&target)).await {
                Ok(found) => self.review(target.path().to_path_buf(), found),
                Err(err) => {
                    log::warn!("transfer: {err}");
                    show_message(t!("transfer.failed"), err);
                }
            }
        });
    }

    /// Says what would be added to the file at `target`, and adds it when the reader agrees.
    fn review(self, target: PathBuf, found: Vec<Transferred>) {
        let placed = transfer::count(&found, Fate::Placed);
        let moved = transfer::count(&found, Fate::Moved);
        let in_place = transfer::count(&found, Fate::InPlace);
        let orphaned = transfer::count(&found, Fate::Orphaned);
        let adding = placed + moved + in_place;
        if adding == 0 {
            show_message(
                t!("transfer.nothing"),
                t!("transfer.nothing_detail", count = found.len().to_string()),
            );
            return;
        }
        let mut detail = t!(
            "transfer.summary",
            placed = placed.to_string(),
            moved = moved.to_string(),
            in_place = in_place.to_string(),
            orphaned = orphaned.to_string()
        );
        let names: Vec<String> = found
            .iter()
            .filter(|t| t.fate == Fate::Orphaned)
            .take(NAMED)
            .map(name)
            .collect();
        if !names.is_empty() {
            detail.push_str("\n\n");
            detail.push_str(&t!("transfer.orphans", names = names.join("; ")));
        }
        let answer = alert_dialog()
            .title(t!("transfer.title", count = adding.to_string()))
            .description(detail)
            .cancel("cancel", t!("transfer.cancel"))
            .action("add", t!("transfer.add", count = adding.to_string()))
            .show();
        spawn(async move {
            let agreed = answer
                .await
                .is_ok_and(|answer| answer.action.as_deref() == Some("add"));
            // Added only to the file the review was about, should another tab be shown meanwhile.
            let shown = self.editing.document();
            if agreed && shown.is_some_and(|document| document.path() == target.as_path()) {
                self.editing.apply(&Operation::new(
                    t!("transfer.operation"),
                    transfer::changes(&found),
                ));
            }
        });
    }
}

/// Reads the annotations of the file at `source` and both documents' text, and finds each annotation in `target`.
async fn read(source: PathBuf, target: Arc<Document>) -> Result<Vec<Transferred>, String> {
    let source = registry().open(&source).map_err(|err| err.to_string())?;
    let from = source.mupdf().map_err(|err| err.to_string())?;
    let to = target.mupdf().map_err(|err| err.to_string())?;
    let annotations = load_all(pool(), Arc::clone(&from))
        .await
        .map_err(|err| err.to_string())?;
    let source_text = load_text(pool(), from)
        .await
        .map_err(|err| err.to_string())?;
    let target_text = load_text(pool(), to).await.map_err(|err| err.to_string())?;
    blocking(move || transfer::transfer(&annotations, source_text, target_text))
        .await
        .map_err(|err| err.to_string())
}

/// How the review names an annotation that wasn't found: its text, cut short, or its kind.
fn name(transferred: &Transferred) -> String {
    let text = &transferred.row.text;
    if text.is_empty() {
        return t!(&format!("annotations.kind.{}", transferred.row.kind.name()));
    }
    let mut short: String = text.chars().take(NAME_LENGTH).collect();
    if text.chars().count() > NAME_LENGTH {
        short.push('…');
    }
    format!("“{short}”")
}
