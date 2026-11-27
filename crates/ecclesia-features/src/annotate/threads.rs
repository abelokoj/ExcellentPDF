//! Comment threads (A.21), as Acrobat keeps them: a reply is a note (`Text`) whose `/IRT` names what it answers, not
//! drawn on the page; a review status is a reply with a `/State`, hidden. A thread is an annotation on the page and
//! every reply under it, however deep. Pure: the app reads the page and applies the changes.

use ecclesia_core::annot::{Annotation, Boxed, Shape, Status, Style};
use ecclesia_core::edit::Change;
use ecclesia_mupdf::PageRect;

use super::edit::bounds;
use super::notes::normalize;

/// A new reply to `parent` by `author`, saying `text`; `None` when the text is blank.
#[must_use]
pub fn reply(parent: &Annotation, text: &str, author: &str) -> Option<Change> {
    let text = normalize(text);
    if text.trim().is_empty() {
        return None;
    }
    let mut reply = answer(parent, author);
    reply.contents = text;
    Some(Change::CreateAnnotation(reply))
}

/// A new review status on `parent`, set by `author`; `said` is what readers that show statuses as replies read
/// (Acrobat's "Accepted set by Ada"). `None` when the status is already `parent`'s, of the `replies` given.
#[must_use]
pub fn set_status(
    parent: &Annotation,
    replies: &[Annotation],
    status: Status,
    author: &str,
    said: &str,
) -> Option<Change> {
    let now = status_of(replies, &parent.id);
    if now == Some(status) || (now.is_none() && status == Status::Cleared) {
        return None;
    }
    let mut set = answer(parent, author);
    set.state = Some(status);
    said.clone_into(&mut set.contents);
    Some(Change::CreateAnnotation(set))
}

/// A note by `author` answering `parent`, where `parent` is on the page (readers place replies there).
fn answer(parent: &Annotation, author: &str) -> Annotation {
    let rect = bounds(parent).unwrap_or(PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 20.0,
        y1: 20.0,
    });
    let mut reply = Annotation::new(
        parent.page,
        Shape::Boxed(Boxed::Note, rect),
        Style {
            color: parent.style.color.or(Style::default().color),
            ..Style::default()
        },
        author,
    );
    reply.reply_to = Some(parent.id.clone());
    reply
}

/// The text replies under the annotation `id`, of `replies`, however deep, oldest first: what its thread reads.
/// Statuses aren't among them; [`status_of`] reads those.
#[must_use]
pub fn thread<'a>(replies: &'a [Annotation], id: &str) -> Vec<&'a Annotation> {
    let mut found = Vec::new();
    let mut parents = vec![id];
    // Each round takes one level; a loop of replies (a broken file) ends when no new reply turns up.
    while !parents.is_empty() {
        let level: Vec<&Annotation> = replies
            .iter()
            .filter(|reply| {
                reply
                    .reply_to
                    .as_deref()
                    .is_some_and(|to| parents.contains(&to))
                    && !found.iter().any(|seen: &&Annotation| seen.id == reply.id)
            })
            .collect();
        parents = level.iter().map(|&reply| reply.id.as_str()).collect();
        found.extend(level);
    }
    found.retain(|reply| reply.state.is_none());
    found.sort_by_key(|reply| reply.created);
    found
}

/// The review status of the annotation `id`, the latest of the statuses `replies` set on it; `None` when none is
/// set, or the latest took it back. The Marked model's ticks are not a review status.
#[must_use]
pub fn status_of(replies: &[Annotation], id: &str) -> Option<Status> {
    replies
        .iter()
        .filter(|reply| reply.reply_to.as_deref() == Some(id))
        .filter_map(|reply| Some((reply.state.filter(|state| !state.is_mark())?, reply)))
        .max_by_key(|(_, reply)| (reply.modified, reply.created))
        .map(|(status, _)| status)
        .filter(|status| *status != Status::Cleared)
}

/// How many text replies the annotation `id` has, of `replies`.
#[must_use]
pub fn count(replies: &[Annotation], id: &str) -> usize {
    thread(replies, id).len()
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::{Annotation, Boxed, Shape, Status, Style};
    use ecclesia_core::edit::Change;
    use ecclesia_mupdf::PageRect;

    use super::*;

    fn note() -> Annotation {
        Annotation::new(
            2,
            Shape::Boxed(
                Boxed::Square,
                PageRect {
                    x0: 10.0,
                    y0: 20.0,
                    x1: 60.0,
                    y1: 50.0,
                },
            ),
            Style::default(),
            "Ada",
        )
    }

    fn made(change: Option<Change>) -> Annotation {
        match change {
            Some(Change::CreateAnnotation(annotation)) => annotation,
            other => panic!("expected a new annotation, got {other:?}"),
        }
    }

    #[test]
    fn a_reply_is_a_note_on_its_parent_that_names_it() {
        let parent = note();
        let reply = made(reply(&parent, "Agreed.\r\n", "Grace"));
        assert_eq!(reply.reply_to.as_deref(), Some(parent.id.as_str()));
        assert_eq!(reply.page, 2);
        assert!(
            matches!(reply.shape, Shape::Boxed(Boxed::Note, r) if r.x0 == 10.0 && r.y1 == 50.0)
        );
        assert_eq!(reply.contents, "Agreed.");
        assert_eq!(reply.author, "Grace");
        assert!(reply.is_reply() && reply.state.is_none());
        assert!(
            super::reply(&parent, "  \n", "Grace").is_none(),
            "a blank reply is none"
        );
    }

    #[test]
    fn a_thread_reads_its_replies_however_deep_oldest_first() {
        let parent = note();
        let mut first = made(reply(&parent, "one", "Grace"));
        first.created = 10;
        let mut deeper = made(reply(&first, "two", "Ada"));
        deeper.created = 20;
        let mut other = made(reply(&note(), "elsewhere", "Ada"));
        other.created = 5;
        let mut status = made(set_status(&parent, &[], Status::Accepted, "Ada", "set"));
        status.created = 30;
        let replies = [deeper.clone(), other, status, first.clone()];
        let read: Vec<&str> = thread(&replies, &parent.id)
            .iter()
            .map(|reply| reply.contents.as_str())
            .collect();
        assert_eq!(read, ["one", "two"]);
        assert_eq!(count(&replies, &parent.id), 2);
    }

    #[test]
    fn a_loop_of_replies_ends() {
        let mut a = made(reply(&note(), "a", "Ada"));
        let mut b = made(reply(&a, "b", "Ada"));
        a.reply_to = Some(b.id.clone());
        b.created = 1;
        assert_eq!(thread(&[a.clone(), b], &a.id).len(), 2);
    }

    #[test]
    fn the_latest_status_counts_and_clearing_takes_it_back() {
        let parent = note();
        assert_eq!(status_of(&[], &parent.id), None);
        assert!(
            set_status(&parent, &[], Status::Cleared, "Ada", "x").is_none(),
            "nothing to clear"
        );
        let mut accepted = made(set_status(
            &parent,
            &[],
            Status::Accepted,
            "Ada",
            "Accepted set by Ada",
        ));
        accepted.modified = 10;
        assert_eq!(accepted.state, Some(Status::Accepted));
        assert_eq!(accepted.contents, "Accepted set by Ada");
        let one = [accepted.clone()];
        assert_eq!(status_of(&one, &parent.id), Some(Status::Accepted));
        assert!(
            set_status(&parent, &one, Status::Accepted, "Ada", "x").is_none(),
            "already so"
        );
        let mut marked = made(set_status(&parent, &one, Status::Marked, "Ada", "x"));
        marked.modified = 20;
        let mut rejected = made(set_status(&parent, &one, Status::Rejected, "Grace", "x"));
        rejected.modified = 30;
        let three = [rejected.clone(), accepted.clone(), marked];
        assert_eq!(
            status_of(&three, &parent.id),
            Some(Status::Rejected),
            "a tick is no status"
        );
        let mut cleared = made(set_status(&parent, &three, Status::Cleared, "Ada", "x"));
        cleared.modified = 40;
        assert_eq!(status_of(&[rejected, accepted, cleared], &parent.id), None);
    }
}
