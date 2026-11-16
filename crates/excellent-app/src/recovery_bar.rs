//! The "Recovered changes" bar (A.15): a row above the pages of a file whose unsaved edits a crash left and the app
//! brought back. It lists them, kind by kind, and keeps them (saved like any edit) or discards them (the file stays
//! as it is on disk). Editing the file keeps them too.

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::editing::Editing;
use crate::reading_theme_bar::bar_button;

/// The bar for the shown file of `editing`, while it has edits brought back.
pub(crate) fn recovery_bar(editing: Editing) -> impl IntoElement {
    let listed = move || {
        let changes = editing.recovered().unwrap_or_default();
        let kinds = summary(&changes)
            .into_iter()
            .map(|(name, count)| t!("recover.change", name = name, count = count.to_string()))
            .collect::<Vec<_>>()
            .join(t!("recover.separator").as_str());
        t!("recover.bar", changes = kinds)
    };
    view()
        .label(t!("recover.label"))
        .min_h(px(40.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(6.0))
        .bg(move || theme().accent)
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(listed)
                .flex_1()
                .number_of_lines(2u32)
                .text_size(px(13.0))
                .text_color(move || theme().accent_foreground),
        )
        .child(bar_button("recover.discard", false, move || {
            editing.discard_recovered()
        }))
        .child(bar_button("recover.keep", true, move || {
            editing.keep_recovered()
        }))
}

/// Each kind of edit in `changes` once, with how many there are, in the order first made.
fn summary(changes: &[String]) -> Vec<(String, usize)> {
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for change in changes {
        match kinds.iter_mut().find(|(name, _)| name == change) {
            Some((_, count)) => *count += 1,
            None => kinds.push((change.clone(), 1)),
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_counts_each_kind_of_change_in_the_order_first_made() {
        let changes = ["Highlight", "Sticky note", "Highlight", "Highlight"].map(String::from);
        assert_eq!(
            summary(&changes),
            [("Highlight".to_owned(), 3), ("Sticky note".to_owned(), 1)]
        );
        assert!(summary(&[]).is_empty());
    }
}
