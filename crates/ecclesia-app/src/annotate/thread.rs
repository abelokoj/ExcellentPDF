//! An annotation's comment thread in the inspector (A.21): its review status, its replies oldest first, and a field
//! to answer it. Each status and each reply is one step of undo; the view is built again from the document after
//! every edit, as the rest of the inspector is. The model is `ecclesia-features`' `annotate::threads`.

use ecclesia_core::annot::{Annotation, Status, page_replies};
use ecclesia_features::annotate::inspector::date;
use ecclesia_features::annotate::threads;
use nsh_base::{input, theme};
use nsh_core::prelude::*;

use super::{AnnotateState, author};
use crate::bookmarks::{caption, field};
use crate::settings_window::{segment, segmented};
use crate::tabs::{Tone, action_button};

impl AnnotateState {
    /// The replies on page `page` of the shown document, statuses among them; none when it can't be read.
    fn replies(self, page: usize) -> Vec<Annotation> {
        let Some(document) = self.editing.document() else {
            return Vec::new();
        };
        let Ok(shared) = document.mupdf() else {
            return Vec::new();
        };
        let mupdf = shared.lock().expect("document lock poisoned");
        page_replies(&mupdf, page)
            .map_err(|err| log::warn!("thread: cannot read page {page}'s replies: {err}"))
            .unwrap_or_default()
    }
}

/// `status`'s label key.
pub(super) fn status_key(status: Status) -> &'static str {
    match status {
        Status::Accepted => "threads.status.accepted",
        Status::Rejected => "threads.status.rejected",
        Status::Cancelled => "threads.status.cancelled",
        Status::Completed => "threads.status.completed",
        Status::Cleared => "threads.status.none",
        Status::Marked => "threads.status.marked",
        Status::Unmarked => "threads.status.unmarked",
    }
}

/// The thread of `annotation`: its status to set, its replies, and the reply field.
pub(super) fn thread_view(state: AnnotateState, annotation: &Annotation) -> View {
    let replies = state.replies(annotation.page);
    let status = threads::status_of(&replies, &annotation.id);
    let statuses = Status::REVIEW
        .into_iter()
        .fold(segmented(), |group, choice| {
            let replies = replies.clone();
            group.child(segment(
                status_key(choice),
                move || status.unwrap_or(Status::Cleared) == choice,
                move || {
                    let who = author();
                    let said = t!(
                        "threads.set_by",
                        status = t!(status_key(choice)),
                        author = who.clone()
                    );
                    state.change_note(t!("threads.status_edit"), |parent| {
                        threads::set_status(parent, &replies, choice, &who, &said)
                    });
                },
            ))
        });
    let mut panel = view()
        .gap_2()
        .child(caption("threads.status"))
        .child(statuses)
        .child(caption("threads.title"));
    let thread = threads::thread(&replies, &annotation.id);
    if thread.is_empty() {
        panel = panel.child(
            text(t!("threads.empty"))
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        );
    }
    for reply in thread {
        panel = panel.child(reply_card(reply));
    }
    let draft = signal(String::new());
    let send = move || {
        let text = untracked(|| draft.get());
        state.change_note(t!("threads.reply_edit"), |parent| {
            threads::reply(parent, &text, &author())
        });
    };
    panel
        .child(
            field(input().value(move || draft.get()))
                .multiline(true)
                .min_h(px(64.0))
                .py_2()
                .on_change(move |text: &str| draft.set(text.to_owned()))
                .placeholder(t!("threads.placeholder"))
                .label(t!("threads.reply_label")),
        )
        .child(
            view()
                .flex_row()
                .child(view().flex_1())
                .child(action_button("threads.reply", Tone::Plain, send)),
        )
}

/// One reply: who wrote it and when, then what it says, set off by a rule.
fn reply_card(reply: &Annotation) -> impl IntoElement {
    let when = date(reply.modified.max(reply.created));
    let by = if reply.author.is_empty() {
        t!("threads.someone")
    } else {
        reply.author.clone()
    };
    let heading = if when.is_empty() {
        by
    } else {
        t!("threads.by_when", author = by, when = when)
    };
    view()
        .flex_row()
        .gap_2()
        .child(
            view()
                .w(px(2.0))
                .rounded(px(1.0))
                .bg(move || theme().border),
        )
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text(heading)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(reply.contents.clone())
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                ),
        )
}
