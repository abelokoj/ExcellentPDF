//! Text markup (A.1): a highlight, underline, squiggly or strikeout over selected text, as the changes that make it.
//!
//! Marking text next to a markup of the same kind and colour extends that markup instead of stacking a second one
//! on top: the new lines join it, a line that touches one of its lines becomes one longer line, and further markups
//! the new text bridges are folded in and deleted.

use ecclesia_core::anchor::Quote;
use ecclesia_core::annot::{Annotation, BlockHighlightShape, Boxed, Markup, Shape, Style, now};
use ecclesia_core::edit::Change;
use ecclesia_mupdf::{PagePoint, PageRect, Quad};
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
    /// From 0 to 1 (A.3).
    pub opacity: f32,
    /// Whether this request marks a rectangular block selection (preserving exact quad geometry
    /// without merging into adjacent single-line text markups).
    pub block: bool,
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
        let color = color(value.get("color")?)?;
        let opacity = opacity(value);
        let block = value.get("block").and_then(Value::as_bool).unwrap_or(false);
        let pages = value
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(PageRequest::parse)
            .collect();
        Some(Self {
            kind,
            color,
            opacity,
            block,
            pages,
        })
    }
}

impl PageRequest {
    /// The page in `value`: `{page, quads, quote}`, as in [`Request::parse`]. A quad that doesn't parse is left out.
    pub(super) fn parse(value: &Value) -> Option<Self> {
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

/// An RGB colour from 0 to 1 in `value`, `[r, g, b]`.
pub(super) fn color(value: &Value) -> Option<[f32; 3]> {
    let color: [f32; 3] = floats(value)?.try_into().ok()?;
    color
        .iter()
        .all(|c| (0.0..=1.0).contains(c))
        .then_some(color)
}

/// The `opacity` in `value`, from 0 to 1; opaque when there is none (A.3).
pub(super) fn opacity(value: &Value) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an opacity from 0 to 1 fits an f32"
    )]
    value
        .get("opacity")
        .and_then(Value::as_f64)
        .filter(|o| o.is_finite())
        .map_or(1.0, |o| o.clamp(0.0, 1.0) as f32)
}

/// The finite numbers of a JSON array, as `f32`; `None` when one isn't.
pub(super) fn floats(value: &Value) -> Option<Vec<f32>> {
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

/// The changes that mark a rectangular block selection (`quads`) on page `page` as a resizable,
/// re-shapable PDF block highlight annotation (a filled `/Square` with `/BM /Multiply` so dark
/// text underneath stays crisp). Empty when `quads` is empty.
#[must_use]
pub fn mark_block(
    marking: Marking<'_>,
    page: usize,
    quads: Vec<Quad>,
    quote: Option<Quote>,
) -> Vec<Change> {
    let Some(rect) = union_bounds(&quads) else {
        return Vec::new();
    };
    let mut annotation = Annotation::new(
        page,
        Shape::Boxed(Boxed::Square, rect),
        Style {
            color: None,
            opacity: marking.style.opacity,
            width: 0.0,
        },
        marking.author,
    );
    annotation.fill = marking.style.color;
    annotation.multiply = true;
    annotation.block_highlight = Some(BlockHighlightShape::Rectangle);
    annotation.anchor = quote;
    vec![Change::CreateAnnotation(annotation)]
}

/// The change that recolours a text markup's colour or a block highlight's background fill;
/// `None` when `annotation` is locked, not a highlight target, or already uses `color`.
#[must_use]
pub fn set_highlight_color(annotation: &Annotation, color: [f32; 3]) -> Option<Change> {
    if annotation.locked {
        return None;
    }
    let color = color.map(|channel| channel.clamp(0.0, 1.0));
    let mut after = annotation.clone();
    if matches!(annotation.shape, Shape::Markup(..)) {
        after.style.color = Some(color);
    } else if annotation.block_highlight.is_some() {
        after.fill = Some(color);
    } else {
        return None;
    }
    super::edit::change(annotation, after)
}

/// The change that sets a block highlight's background fill (`None` clears the fill);
/// `None` when `annotation` is locked, not a block highlight, or unchanged.
#[must_use]
pub fn set_block_fill(annotation: &Annotation, fill: Option<[f32; 3]>) -> Option<Change> {
    if annotation.locked || annotation.block_highlight.is_none() {
        return None;
    }
    let mut after = annotation.clone();
    after.fill = fill.map(|rgb| rgb.map(|channel| channel.clamp(0.0, 1.0)));
    super::edit::change(annotation, after)
}

/// The change that sets a block highlight's border colour (`None` hides the border);
/// `None` when `annotation` is locked, not a block highlight, or unchanged.
#[must_use]
pub fn set_block_edge(annotation: &Annotation, edge: Option<[f32; 3]>) -> Option<Change> {
    if annotation.locked || annotation.block_highlight.is_none() {
        return None;
    }
    let mut after = annotation.clone();
    after.style.color = edge.map(|rgb| rgb.map(|channel| channel.clamp(0.0, 1.0)));
    after.style.width = if after.style.color.is_some() {
        after.style.width.max(1.5)
    } else {
        0.0
    };
    super::edit::change(annotation, after)
}

/// The change that reshapes a block highlight to `shape` while keeping its bounding rectangle;
/// `None` when `annotation` is locked, not a block highlight, has no bounds, or is unchanged.
#[must_use]
pub fn set_block_shape(annotation: &Annotation, shape: BlockHighlightShape) -> Option<Change> {
    if annotation.locked || annotation.block_highlight.is_none() {
        return None;
    }
    let rect = super::edit::bounds(annotation)?;
    let (new_shape, cloud) = shape.into_shape(rect);
    let mut after = annotation.clone();
    after.shape = new_shape;
    after.cloud = cloud;
    after.block_highlight = Some(shape);
    if shape == BlockHighlightShape::Cloud && after.style.color.is_none() {
        after.style.color = after.fill;
        if after.style.color.is_some() {
            after.style.width = after.style.width.max(1.5);
        }
    }
    super::edit::change(annotation, after)
}

/// The change that converts a text markup between highlight, underline, squiggly and strikeout
/// while preserving its quads, colour, comment and anchor; `None` when locked, not text markup,
/// or already `kind`.
#[must_use]
pub fn set_markup_kind(annotation: &Annotation, kind: Markup) -> Option<Change> {
    if annotation.locked {
        return None;
    }
    let Shape::Markup(current, quads) = &annotation.shape else {
        return None;
    };
    if *current == kind {
        return None;
    }
    let mut after = annotation.clone();
    after.shape = Shape::Markup(kind, quads.clone());
    super::edit::change(annotation, after)
}

/// Normalized bounding rectangle covering every quad in `quads`; `None` when `quads` is empty.
fn union_bounds(quads: &[Quad]) -> Option<PageRect> {
    let mut iter = quads.iter().map(Quad::bounds);
    let first = iter.next()?;
    let mut out = PageRect {
        x0: first.x0.min(first.x1),
        y0: first.y0.min(first.y1),
        x1: first.x0.max(first.x1),
        y1: first.y0.max(first.y1),
    };
    for b in iter {
        out.x0 = out.x0.min(b.x0).min(b.x1);
        out.y0 = out.y0.min(b.y0).min(b.y1);
        out.x1 = out.x1.max(b.x0).max(b.x1);
        out.y1 = out.y1.max(b.y0).max(b.y1);
    }
    Some(out)
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
    use ecclesia_mupdf::PagePoint;

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
        assert!(mark_block(marking(YELLOW), 0, Vec::new(), None).is_empty());
    }

    #[test]
    fn block_request_parses_flag_and_creates_resizable_block_highlight() {
        let value = serde_json::json!({
            "kind": "highlight",
            "block": true,
            "color": [1.0, 0.8, 0.15],
            "opacity": 0.45,
            "pages": [
                {"page": 1, "quads": [[20, 40, 220, 40, 20, 180, 220, 180]], "quote": null}
            ]
        });
        let request = Request::parse(&value).expect("block request parses");
        assert!(request.block);
        assert_eq!(request.kind, Markup::Highlight);
        assert_eq!(request.pages.len(), 1);
        assert_eq!(request.pages[0].quads.len(), 1);

        let block_quad = request.pages[0].quads[0];
        let quote = Quote::of("hello block world", 6..11);
        let changes = mark_block(marking(YELLOW), 1, vec![block_quad], quote.clone());
        let annotation = made(&changes);
        assert_eq!(annotation.page, 1);
        assert_eq!(annotation.anchor, quote);
        assert_eq!(annotation.fill, Some(YELLOW));
        assert_eq!(annotation.style.color, None);
        assert_eq!(annotation.style.width, 0.0);
        assert!(annotation.multiply);
        assert_eq!(
            annotation.block_highlight,
            Some(BlockHighlightShape::Rectangle)
        );
        assert_eq!(
            annotation.shape,
            Shape::Boxed(
                Boxed::Square,
                PageRect {
                    x0: 20.0,
                    y0: 40.0,
                    x1: 220.0,
                    y1: 180.0,
                },
            )
        );

        let plain = serde_json::json!({
            "kind": "highlight",
            "color": [1.0, 0.8, 0.15],
            "pages": []
        });
        assert!(!Request::parse(&plain).expect("plain request parses").block);
    }

    #[test]
    fn highlight_and_block_edit_helpers_update_style_shape_and_kind() {
        let updated = |change: Option<Change>| match change {
            Some(Change::UpdateAnnotation(annotation)) => annotation,
            other => panic!("expected an update, got {other:?}"),
        };

        // Text markup recolouring and kind switching.
        let quote = Quote::of("the quick fox", 4..9);
        let mut text_hl = made(&mark(
            marking(YELLOW),
            0,
            vec![run(10.0, 80.0, 0)],
            quote.clone(),
            &[],
        ));
        text_hl.contents = "keep me".to_owned();
        let recoloured = updated(set_highlight_color(&text_hl, GREEN));
        assert_eq!(recoloured.style.color, Some(GREEN));
        assert!(set_highlight_color(&recoloured, GREEN).is_none());

        let underlined = updated(set_markup_kind(&recoloured, Markup::Underline));
        assert!(matches!(
            underlined.shape,
            Shape::Markup(Markup::Underline, ref quads) if quads.len() == 1
        ));
        assert_eq!(underlined.style.color, Some(GREEN));
        assert_eq!(underlined.contents, "keep me");
        assert_eq!(underlined.anchor, quote);
        assert!(set_markup_kind(&underlined, Markup::Underline).is_none());

        // Block highlight helpers: colour, fill, edge, and shapes.
        let block = made(&mark_block(
            marking(YELLOW),
            2,
            vec![run(20.0, 120.0, 0), run(20.0, 120.0, 2)],
            None,
        ));
        assert!(set_markup_kind(&block, Markup::StrikeOut).is_none());

        let green_block = updated(set_highlight_color(&block, GREEN));
        assert_eq!(green_block.fill, Some(GREEN));
        assert!(set_highlight_color(&green_block, GREEN).is_none());

        let unfilled = updated(set_block_fill(&green_block, None));
        assert_eq!(unfilled.fill, None);
        assert!(set_block_fill(&unfilled, None).is_none());

        let bordered = updated(set_block_edge(&green_block, Some([0.1, 0.2, 0.9])));
        assert_eq!(bordered.style.color, Some([0.1, 0.2, 0.9]));
        assert_eq!(bordered.style.width, 1.5);
        let borderless = updated(set_block_edge(&bordered, None));
        assert_eq!(borderless.style.color, None);
        assert_eq!(borderless.style.width, 0.0);

        let rounded = updated(set_block_shape(&block, BlockHighlightShape::RoundedRect));
        assert_eq!(
            rounded.block_highlight,
            Some(BlockHighlightShape::RoundedRect)
        );
        assert!(matches!(
            rounded.shape,
            Shape::Path(ecclesia_core::annot::Path::Polygon, ref pts) if pts.len() == 32
        ));
        assert_eq!(rounded.cloud, None);

        let oval = updated(set_block_shape(&rounded, BlockHighlightShape::Oval));
        assert_eq!(oval.block_highlight, Some(BlockHighlightShape::Oval));
        assert!(matches!(oval.shape, Shape::Boxed(Boxed::Circle, _)));

        let cloud = updated(set_block_shape(&block, BlockHighlightShape::Cloud));
        assert_eq!(cloud.block_highlight, Some(BlockHighlightShape::Cloud));
        assert_eq!(cloud.cloud, Some(1.0));
        assert_eq!(
            cloud.style.color,
            Some(YELLOW),
            "cloud auto-enables a subtle border when borderless"
        );
        assert_eq!(cloud.style.width, 1.5);

        // Locked annotations reject all edits.
        let mut locked = block;
        locked.locked = true;
        assert!(set_highlight_color(&locked, GREEN).is_none());
        assert!(set_block_fill(&locked, None).is_none());
        assert!(set_block_edge(&locked, Some(GREEN)).is_none());
        assert!(set_block_shape(&locked, BlockHighlightShape::Oval).is_none());
    }
}
