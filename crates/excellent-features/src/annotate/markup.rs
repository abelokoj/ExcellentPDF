//! Text markup (A.1): a highlight, underline, squiggly or strikeout over selected text, as the changes that make it.
//!
//! Marking text next to a markup of the same kind and colour extends that markup instead of stacking a second one
//! on top: the new lines join it, a line that touches one of its lines becomes one longer line, and further markups
//! the new text bridges are folded in and deleted.

use excellent_core::anchor::Quote;
use excellent_core::annot::{Annotation, Markup, Shape, Style, now};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect, Quad};
use serde_json::Value;

/// How much two lines' heights must overlap, as a share of the smaller, to be the same line.
const SAME_LINE: f32 = 0.5;

/// The widest gap between two runs on one line that still touch, as a share of the line's height (about a space).
const TOUCHING_GAP: f32 = 0.25;

/// What to mark: the kind, how it looks and who marks it.
#[derive(Debug, Clone, Copy)]
pub struct Marking<'a> {
    /// Highlight, underline, squiggly or strikeout.
    pub kind: Markup,
    /// Its colour and opacity.
    pub style: Style,
    /// Who marks it.
    pub author: &'a str,
}

/// A markup the page canvas asks for (its `markup` event): what to mark, in which colour, and on which pages.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Highlight, underline, squiggly or strikeout.
    pub kind: Markup,
    /// RGB from 0 to 1.
    pub color: [f32; 3],
    /// The selection on each page it covers.
    pub pages: Vec<PageRequest>,
}

/// One page of a [`Request`].
#[derive(Debug, Clone, PartialEq)]
pub struct PageRequest {
    /// The page, counting from zero.
    pub page: usize,
    /// One quad per line.
    pub quads: Vec<Quad>,
    /// The text under them.
    pub quote: Option<Quote>,
}

impl Request {
    /// The request in `value`, the event's data: `{kind, color: [r, g, b], pages: [{page, quads: [[ulx, uly, urx,
    /// ury, llx, lly, lrx, lry]], quote: {prefix, exact, suffix, start}}]}`. `None` when the kind or colour is
    /// missing; a page or quad that doesn't parse is left out.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let kind = match value.get("kind")?.as_str()? {
            "highlight" => Markup::Highlight,
            "underline" => Markup::Underline,
            "squiggly" => Markup::Squiggly,
            "strikeOut" => Markup::StrikeOut,
            _ => return None,
        };
        let color: [f32; 3] = floats(value.get("color")?)?.try_into().ok()?;
        if color.iter().any(|c| !(0.0..=1.0).contains(c)) {
            return None;
        }
        let pages = value
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(PageRequest::parse)
            .collect();
        Some(Self { kind, color, pages })
    }
}

impl PageRequest {
    fn parse(value: &Value) -> Option<Self> {
        let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
        let quads: Vec<Quad> = value
            .get("quads")?
            .as_array()?
            .iter()
            .filter_map(|quad| {
                let [ulx, uly, urx, ury, llx, lly, lrx, lry]: [f32; 8] =
                    floats(quad)?.try_into().ok()?;
                let at = |x, y| PagePoint { x, y };
                Some(Quad {
                    ul: at(ulx, uly),
                    ur: at(urx, ury),
                    ll: at(llx, lly),
                    lr: at(lrx, lry),
                })
            })
            .collect();
        let quote = value.get("quote").and_then(|quote| {
            let text = |key: &str| quote.get(key).and_then(Value::as_str).map(str::to_owned);
            Some(Quote {
                prefix: text("prefix").unwrap_or_default(),
                exact: text("exact")?,
                suffix: text("suffix").unwrap_or_default(),
                start: usize::try_from(quote.get("start")?.as_u64()?).ok()?,
            })
        });
        Some(Self { page, quads, quote })
    }
}

/// The finite numbers of a JSON array, as `f32`; `None` when one isn't.
fn floats(value: &Value) -> Option<Vec<f32>> {
    value
        .as_array()?
        .iter()
        .map(|n| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "page points and colour channels fit an f32"
            )]
            n.as_f64().filter(|n| n.is_finite()).map(|n| n as f32)
        })
        .collect()
}

/// The changes that mark `quads` (one per line of the selection) on page `page`, whose markups now are `existing`.
/// `quote` is the text under them, to find the markup again (K.28). Empty when there is nothing to mark.
#[must_use]
pub fn mark(
    marking: Marking<'_>,
    page: usize,
    quads: Vec<Quad>,
    quote: Option<Quote>,
    existing: &[Annotation],
) -> Vec<Change> {
    if quads.is_empty() {
        return Vec::new();
    }
    let mut touched = existing.iter().filter(|annotation| {
        annotation.page == page
            && annotation.style.color == marking.style.color
            && matches!(&annotation.shape, Shape::Markup(kind, theirs)
                if *kind == marking.kind && any_touch(theirs, &quads))
    });
    let Some(first) = touched.next() else {
        let mut annotation = Annotation::new(
            page,
            Shape::Markup(marking.kind, join(quads)),
            marking.style,
            marking.author,
        );
        annotation.anchor = quote;
        return vec![Change::CreateAnnotation(annotation)];
    };
    let others: Vec<&Annotation> = touched.collect();
    let mut all = quads_of(first);
    for other in &others {
        all.extend(quads_of(other));
    }
    all.extend(quads);
    let mut merged = first.clone();
    // shortcut: the first markup's anchor is kept, not one for the joined text; quote the joined text once K.28's
    // re-anchoring reads anchors.
    merged.shape = Shape::Markup(marking.kind, join(all));
    merged.modified = now();
    let mut changes = vec![Change::UpdateAnnotation(merged)];
    changes.extend(others.into_iter().map(|other| Change::DeleteAnnotation {
        page,
        id: other.id.clone(),
    }));
    changes
}

fn quads_of(annotation: &Annotation) -> Vec<Quad> {
    match &annotation.shape {
        Shape::Markup(_, quads) => quads.clone(),
        _ => Vec::new(),
    }
}

fn any_touch(theirs: &[Quad], ours: &[Quad]) -> bool {
    theirs
        .iter()
        .any(|a| ours.iter().any(|b| touch(&a.bounds(), &b.bounds())))
}

/// Whether two runs are on the same line and meet or nearly meet.
// shortcut: compared on their upright bounds, so runs on slanted lines may not join; compare along the line if
// rotated text is marked often.
fn touch(a: &PageRect, b: &PageRect) -> bool {
    let height = (a.y1 - a.y0).min(b.y1 - b.y0).max(f32::EPSILON);
    let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    let gap = a.x0.max(b.x0) - a.x1.min(b.x1);
    overlap >= SAME_LINE * height && gap <= TOUCHING_GAP * height
}

/// `quads` with the runs that touch made one: from the leftmost's left edge to the rightmost's right edge.
fn join(quads: Vec<Quad>) -> Vec<Quad> {
    let mut out: Vec<Quad> = Vec::with_capacity(quads.len());
    for quad in quads {
        let mut quad = quad;
        // Joining can make a run touch one kept earlier, so keep joining until none does.
        while let Some(at) = out
            .iter()
            .position(|kept| touch(&kept.bounds(), &quad.bounds()))
        {
            quad = union(&out.swap_remove(at), &quad);
        }
        out.push(quad);
    }
    out.sort_by(|a, b| {
        let (a, b) = (a.bounds(), b.bounds());
        a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0))
    });
    out
}

fn union(a: &Quad, b: &Quad) -> Quad {
    let (left, right) = if a.bounds().x0 <= b.bounds().x0 {
        (a, b)
    } else {
        (b, a)
    };
    let right = if right.bounds().x1 >= left.bounds().x1 {
        right
    } else {
        left
    };
    Quad {
        ul: left.ul,
        ll: left.ll,
        ur: right.ur,
        lr: right.lr,
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::PagePoint;

    use super::*;

    fn run(x0: f32, x1: f32, line: usize) -> Quad {
        #[expect(clippy::cast_precision_loss, reason = "a test's few lines")]
        let y0 = 100.0 + 20.0 * line as f32;
        let y1 = y0 + 12.0;
        Quad {
            ul: PagePoint { x: x0, y: y0 },
            ur: PagePoint { x: x1, y: y0 },
            ll: PagePoint { x: x0, y: y1 },
            lr: PagePoint { x: x1, y: y1 },
        }
    }

    fn marking(color: [f32; 3]) -> Marking<'static> {
        Marking {
            kind: Markup::Highlight,
            style: Style {
                color: Some(color),
                ..Style::default()
            },
            author: "reader",
        }
    }

    const YELLOW: [f32; 3] = [1.0, 0.83, 0.0];
    const GREEN: [f32; 3] = [0.49, 0.85, 0.34];

    fn made(changes: &[Change]) -> Annotation {
        match changes {
            [Change::CreateAnnotation(annotation)] => annotation.clone(),
            other => panic!("expected one new markup, got {other:?}"),
        }
    }

    #[test]
    fn marking_text_makes_one_markup_over_its_lines() {
        let quote = Quote::of("the quick fox", 4..9);
        let changes = mark(
            marking(YELLOW),
            2,
            vec![run(10.0, 50.0, 0), run(10.0, 30.0, 1)],
            quote.clone(),
            &[],
        );
        let annotation = made(&changes);
        assert_eq!(annotation.page, 2);
        assert_eq!(annotation.anchor, quote);
        assert!(
            matches!(&annotation.shape, Shape::Markup(Markup::Highlight, quads) if quads.len() == 2)
        );
    }

    #[test]
    fn extending_a_highlight_merges_into_it() {
        let old = made(&mark(
            marking(YELLOW),
            0,
            vec![run(10.0, 50.0, 0)],
            None,
            &[],
        ));
        let changes = mark(
            marking(YELLOW),
            0,
            vec![run(51.0, 90.0, 0)],
            None,
            std::slice::from_ref(&old),
        );
        let [Change::UpdateAnnotation(merged)] = changes.as_slice() else {
            panic!("expected the old markup changed, got {changes:?}");
        };
        assert_eq!(merged.id, old.id);
        let Shape::Markup(_, quads) = &merged.shape else {
            panic!("a markup");
        };
        assert_eq!(quads.len(), 1, "one line, one run");
        let bounds = quads[0].bounds();
        assert!((bounds.x0 - 10.0).abs() < f32::EPSILON && (bounds.x1 - 90.0).abs() < f32::EPSILON);
    }

    #[test]
    fn another_colour_or_line_stacks_instead() {
        let old = made(&mark(
            marking(YELLOW),
            0,
            vec![run(10.0, 50.0, 0)],
            None,
            &[],
        ));
        let existing = std::slice::from_ref(&old);
        made(&mark(
            marking(GREEN),
            0,
            vec![run(51.0, 90.0, 0)],
            None,
            existing,
        ));
        made(&mark(
            marking(YELLOW),
            0,
            vec![run(10.0, 50.0, 2)],
            None,
            existing,
        ));
        made(&mark(
            marking(YELLOW),
            0,
            vec![run(80.0, 90.0, 0)],
            None,
            existing,
        ));
    }

    #[test]
    fn bridging_two_highlights_folds_them_into_one() {
        let left = made(&mark(
            marking(YELLOW),
            0,
            vec![run(10.0, 30.0, 0)],
            None,
            &[],
        ));
        let right = made(&mark(
            marking(YELLOW),
            0,
            vec![run(60.0, 90.0, 0)],
            None,
            &[],
        ));
        let changes = mark(
            marking(YELLOW),
            0,
            vec![run(31.0, 59.0, 0)],
            None,
            &[left.clone(), right.clone()],
        );
        assert!(
            matches!(&changes[0], Change::UpdateAnnotation(a) if a.id == left.id
            && matches!(&a.shape, Shape::Markup(_, quads) if quads.len() == 1))
        );
        assert!(matches!(&changes[1], Change::DeleteAnnotation { id, .. } if *id == right.id));
    }

    #[test]
    fn the_canvas_request_is_read() {
        let value = serde_json::json!({
            "kind": "strikeOut",
            "color": [1.0, 0.5, 0.0],
            "pages": [
                {"page": 3, "quads": [[1, 2, 3, 2, 1, 4, 3, 4], [1, 2]],
                 "quote": {"prefix": "a ", "exact": "word", "suffix": " b", "start": 7}},
                {"quads": []}
            ]
        });
        let request = Request::parse(&value).expect("a request");
        assert_eq!(request.kind, Markup::StrikeOut);
        assert_eq!(
            request.pages.len(),
            1,
            "a page without a number is left out"
        );
        let page = &request.pages[0];
        assert_eq!(
            (page.page, page.quads.len()),
            (3, 1),
            "a short quad is left out"
        );
        assert_eq!(page.quads[0].lr, PagePoint { x: 3.0, y: 4.0 });
        assert_eq!(page.quote.as_ref().map(|q| q.start), Some(7));
        let wrong_kind = serde_json::json!({"kind": "circle", "color": [0, 0, 0]});
        assert!(Request::parse(&wrong_kind).is_none());
        let wrong_color = serde_json::json!({"kind": "highlight", "color": [2, 0, 0]});
        assert!(Request::parse(&wrong_color).is_none());
    }

    #[test]
    fn nothing_selected_marks_nothing() {
        assert!(mark(marking(YELLOW), 0, Vec::new(), None, &[]).is_empty());
    }
}
