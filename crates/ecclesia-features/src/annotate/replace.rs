//! Replace and insert text (A.2): proposed edits a reader leaves on the page, as Acrobat makes them. Insert is a
//! caret where the text goes, holding the proposed text; replace is a strikeout over the selection grouped with such
//! a caret at its end (the strikeout's `/IRT` names the caret, with `/RT /Group`), so readers show the pair as one
//! "replace text" comment.

use ecclesia_core::anchor::Quote;
use ecclesia_core::annot::{Annotation, Boxed, Markup, Shape, Style};
use ecclesia_core::edit::Change;
use ecclesia_mupdf::{PagePoint, PageRect, Quad};
use serde_json::Value;

use super::markup::PageRequest;

/// MuPDF draws a caret this size about its rectangle's centre (`pdf-appearance.c`), whatever the rectangle.
const CARET: (f32, f32) = (20.0, 14.0);

/// The proposed text and how it is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Proposal<'a> {
    /// The text proposed, which the caret holds.
    pub text: &'a str,
    /// Its colour and opacity.
    pub style: Style,
    /// Who proposes it.
    pub author: &'a str,
}

/// Whether text goes in, or replaces the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A caret at a place in the text.
    Insert,
    /// A strikeout over the selection and a caret at its end.
    Replace,
}

/// What the page canvas asks to propose (its `textEdit` event); the app asks the reader for the text.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Insert or replace.
    pub mode: Mode,
    /// RGB from 0 to 1.
    pub color: [f32; 3],
    /// From 0 to 1 (A.3).
    pub opacity: f32,
    /// The page, the quads struck out (none for an insert) and the text under them.
    pub page: PageRequest,
    /// Where the caret goes.
    pub caret: PageRect,
    /// The text selected, to start the replacement from; empty for an insert.
    pub selected: String,
}

impl Request {
    /// The request in `value`, the event's data: `{mode: "insert" | "replace", color: [r, g, b], page, quads,
    /// quote, foot: [x, y], selected}`, `foot` being the bottom of the edge the text goes in at. `None` when a part is missing, or a replacement
    /// has nothing to strike out.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let mode = match value.get("mode")?.as_str()? {
            "insert" => Mode::Insert,
            "replace" => Mode::Replace,
            _ => return None,
        };
        let color = super::markup::color(value.get("color")?)?;
        let page = PageRequest::parse(value)?;
        if mode == Mode::Replace && page.quads.is_empty() {
            return None;
        }
        let [x, y]: [f32; 2] = super::markup::floats(value.get("foot")?)?.try_into().ok()?;
        let selected = value
            .get("selected")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Some(Self {
            mode,
            color,
            opacity: super::markup::opacity(value),
            page,
            caret: caret(PagePoint { x, y }),
            selected,
        })
    }
}

/// The caret's rectangle for text going in at an edge whose bottom is `foot` (a character's side, at the line's
/// bottom): centred there, so the caret's point sits on the line.
// shortcut: drawn upright on rotated text (MuPDF's caret has no angle); turn it with the line if rotated text is
// edited often.
#[must_use]
pub fn caret(foot: PagePoint) -> PageRect {
    let (w, h) = CARET;
    PageRect {
        x0: foot.x - w / 2.0,
        y0: foot.y - h / 2.0,
        x1: foot.x + w / 2.0,
        y1: foot.y + h / 2.0,
    }
}

/// The changes that propose inserting `proposal` at `caret` on page `page`. Empty when the text is blank.
#[must_use]
pub fn insert(proposal: Proposal<'_>, page: usize, caret: PageRect) -> Vec<Change> {
    if proposal.text.trim().is_empty() {
        return Vec::new();
    }
    let mut annotation = Annotation::new(
        page,
        Shape::Boxed(Boxed::Caret, caret),
        proposal.style,
        proposal.author,
    );
    annotation.contents = proposal.text.to_owned();
    vec![Change::CreateAnnotation(annotation)]
}

/// The changes that propose replacing the text under `quads` (with `quote` its text) by `proposal`: the caret at
/// `caret`, then the strikeout grouped with it. Empty when the text is blank or nothing is struck out.
#[must_use]
pub fn replace(
    proposal: Proposal<'_>,
    page: usize,
    quads: Vec<Quad>,
    caret: PageRect,
    quote: Option<Quote>,
) -> Vec<Change> {
    if quads.is_empty() {
        return Vec::new();
    }
    let mut changes = insert(proposal, page, caret);
    let Some(Change::CreateAnnotation(made)) = changes.first() else {
        return Vec::new();
    };
    let mut strike = Annotation::new(
        page,
        Shape::Markup(Markup::StrikeOut, quads),
        proposal.style,
        proposal.author,
    );
    strike.anchor = quote;
    // After the caret: the shim finds the caret on the page by its id.
    strike.group = Some(made.id.clone());
    changes.push(Change::CreateAnnotation(strike));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(text: &str) -> Proposal<'_> {
        Proposal {
            text,
            style: Style::default(),
            author: "reader",
        }
    }

    fn line() -> Quad {
        let at = |x, y| PagePoint { x, y };
        Quad {
            ul: at(10.0, 100.0),
            ur: at(60.0, 100.0),
            ll: at(10.0, 112.0),
            lr: at(60.0, 112.0),
        }
    }

    #[test]
    fn the_caret_sits_on_the_line_s_foot() {
        let rect = caret(PagePoint { x: 60.0, y: 112.0 });
        assert!(((rect.x0 + rect.x1) / 2.0 - 60.0).abs() < f32::EPSILON);
        assert!(((rect.y0 + rect.y1) / 2.0 - 112.0).abs() < f32::EPSILON);
    }

    #[test]
    fn inserting_text_makes_a_caret_holding_it() {
        let rect = caret(PagePoint { x: 5.0, y: 5.0 });
        let changes = insert(proposal("the"), 1, rect);
        let [Change::CreateAnnotation(made)] = changes.as_slice() else {
            panic!("one caret, got {changes:?}");
        };
        assert!(matches!(made.shape, Shape::Boxed(Boxed::Caret, _)));
        assert_eq!((made.page, made.contents.as_str()), (1, "the"));
        assert!(
            insert(proposal("  "), 1, rect).is_empty(),
            "nothing to insert"
        );
    }

    #[test]
    fn replacing_text_groups_a_strikeout_with_its_caret() {
        let rect = caret(PagePoint { x: 60.0, y: 112.0 });
        let changes = replace(proposal("new"), 0, vec![line()], rect, None);
        let [
            Change::CreateAnnotation(caret),
            Change::CreateAnnotation(strike),
        ] = changes.as_slice()
        else {
            panic!("a caret then a strikeout, got {changes:?}");
        };
        assert_eq!(caret.contents, "new");
        assert!(matches!(strike.shape, Shape::Markup(Markup::StrikeOut, _)));
        assert_eq!(strike.group.as_deref(), Some(caret.id.as_str()));
        assert!(replace(proposal(""), 0, vec![line()], rect, None).is_empty());
        assert!(replace(proposal("x"), 0, Vec::new(), rect, None).is_empty());
    }

    #[test]
    fn the_canvas_request_is_read() {
        let value = serde_json::json!({
            "mode": "replace",
            "color": [0.0, 0.4, 1.0],
            "page": 2,
            "quads": [[10, 100, 60, 100, 10, 112, 60, 112]],
            "quote": {"exact": "old", "start": 4},
            "foot": [60, 112],
            "selected": "old"
        });
        let request = Request::parse(&value).expect("a request");
        assert_eq!((request.mode, request.page.page), (Mode::Replace, 2));
        assert_eq!(request.selected, "old");
        let mut insert = value.clone();
        insert["mode"] = "insert".into();
        insert["quads"] = serde_json::json!([]);
        assert_eq!(Request::parse(&insert).map(|r| r.mode), Some(Mode::Insert));
        let mut empty = value;
        empty["quads"] = serde_json::json!([]);
        assert!(
            Request::parse(&empty).is_none(),
            "a replacement strikes something out"
        );
    }
}
