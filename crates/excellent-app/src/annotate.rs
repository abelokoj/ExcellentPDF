//! Annotating from the pages (plan A): the canvas's `markup` event (A.1) made into one undoable edit of the shown
//! document. The model, merging into a markup the new one touches, is `excellent-features`' `annotate::markup`.

use excellent_core::annot::{Markup, Style, page_annotations};
use excellent_core::edit::Operation;
use excellent_features::annotate::markup::{self, Marking, Request};
use nsh_core::prelude::*;

use crate::editing::Editing;

/// The canvas asked for a markup over the selected text (`markup`): it is made, merged with the markups it
/// touches, as one step of undo.
pub(crate) fn on_canvas_event(editing: Editing, event: &NativeEvent) {
    if &*event.name != "markup" {
        return;
    }
    let Some(request) = Request::parse(&event.data) else {
        log::warn!("markup: a request that doesn't parse: {}", event.data);
        return;
    };
    let Some(document) = editing.document() else {
        return;
    };
    let shared = match document.mupdf() {
        Ok(shared) => shared,
        Err(err) => {
            log::warn!("markup: cannot read the document: {err}");
            return;
        }
    };
    let author = author();
    let marking = Marking {
        kind: request.kind,
        style: Style {
            color: Some(request.color),
            ..Style::default()
        },
        author: &author,
    };
    let mut changes = Vec::new();
    {
        // Let go of the document before the edit, which takes it again.
        let mupdf = shared.lock().expect("document lock poisoned");
        for page in request.pages {
            let existing = page_annotations(&mupdf, page.page).unwrap_or_else(|err| {
                log::warn!(
                    "markup: cannot read page {}'s annotations: {err}",
                    page.page
                );
                Vec::new()
            });
            changes.extend(markup::mark(
                marking, page.page, page.quads, page.quote, &existing,
            ));
        }
    }
    editing.apply(&Operation::new(name(request.kind), changes));
}

/// The edit's name, as undo shows it.
fn name(kind: Markup) -> String {
    match kind {
        Markup::Highlight => t!("markup.highlight"),
        Markup::Underline => t!("markup.underline"),
        Markup::Squiggly => t!("markup.squiggly"),
        Markup::StrikeOut => t!("markup.strikeout"),
    }
}

/// Who annotations are by: the system's user name.
// shortcut: the account's name until a setting for the author's name (the plan has none yet); ask the maintainer.
fn author() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}
