//! The annotation model (plan K.14, step 1): an annotation as the app thinks of it, whatever kind it is and whichever
//! file keeps it. Every annotation tool (A.1 to A.28) makes and changes these; [`crate::edit::Change`] writes them to
//! the PDF inside one operation, so they undo, are logged and are saved like every other edit.
//!
//! Positions are in page space: points from the page's top-left corner, y down, before the reader's rotation, as
//! MuPDF reports the page. An annotation is named by its id (the PDF's `/NM`), never by its place in the page's list.
//! The anchor and tags (K.28) live in a private key of the annotation's dictionary, so they travel with the file.
//!
//! Only the kinds the app makes are modelled; the others (links, form fields, sounds, other apps' own kinds) are left
//! in the file as they are: [`Annotation::from_data`] gives `None` for them and the model never writes them.

use std::time::{SystemTime, UNIX_EPOCH};

use excellent_mupdf::{AnnotData, AnnotKind, Document, PagePoint, PageRect, Quad};
use serde_json::{Map, Value, json};

use crate::anchor::{Quote, new_id};

/// Text markup over a run of text (A.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Markup {
    /// A highlight.
    Highlight,
    /// An underline.
    Underline,
    /// A squiggly underline.
    Squiggly,
    /// A strikeout.
    StrikeOut,
}

/// An annotation placed by a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Boxed {
    /// A sticky note's icon (A.5).
    Note,
    /// Text drawn on the page (A.6).
    FreeText,
    /// A rectangle (A.8).
    Square,
    /// An ellipse (A.8).
    Circle,
    /// A stamp (A.10).
    Stamp,
    /// An insertion caret (A.2).
    Caret,
}

/// An annotation drawn through points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Path {
    /// A closed polygon.
    Polygon,
    /// An open polyline.
    PolyLine,
}

/// What an annotation looks like on the page and where: its kind and its geometry together, so a kind never carries
/// the wrong geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Markup over text, one quad per line run.
    Markup(Markup, Vec<Quad>),
    /// A kind placed by a rectangle.
    Boxed(Boxed, PageRect),
    /// Freehand ink, one list of points per stroke.
    Ink(Vec<Vec<PagePoint>>),
    /// A straight line from one end to the other.
    Line(PagePoint, PagePoint),
    /// A polygon or polyline through its vertices.
    Path(Path, Vec<PagePoint>),
}

impl Shape {
    /// MuPDF's kind for it.
    fn kind(&self) -> AnnotKind {
        match self {
            Self::Markup(Markup::Highlight, _) => AnnotKind::Highlight,
            Self::Markup(Markup::Underline, _) => AnnotKind::Underline,
            Self::Markup(Markup::Squiggly, _) => AnnotKind::Squiggly,
            Self::Markup(Markup::StrikeOut, _) => AnnotKind::StrikeOut,
            Self::Boxed(Boxed::Note, _) => AnnotKind::Text,
            Self::Boxed(Boxed::FreeText, _) => AnnotKind::FreeText,
            Self::Boxed(Boxed::Square, _) => AnnotKind::Square,
            Self::Boxed(Boxed::Circle, _) => AnnotKind::Circle,
            Self::Boxed(Boxed::Stamp, _) => AnnotKind::Stamp,
            Self::Boxed(Boxed::Caret, _) => AnnotKind::Caret,
            Self::Ink(_) => AnnotKind::Ink,
            Self::Line(..) => AnnotKind::Line,
            Self::Path(Path::Polygon, _) => AnnotKind::Polygon,
            Self::Path(Path::PolyLine, _) => AnnotKind::PolyLine,
        }
    }

    /// Whether the kind has a border or stroke width (PDF's `/BS`); the others draw at their own.
    #[must_use]
    pub fn has_width(&self) -> bool {
        matches!(
            self,
            Self::Boxed(Boxed::FreeText | Boxed::Square | Boxed::Circle, _)
                | Self::Ink(_)
                | Self::Line(..)
                | Self::Path(..)
        )
    }
}

/// How an annotation is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Its colour, RGB from 0 to 1; `None` for none (a shape's outline left out).
    pub color: Option<[f32; 3]>,
    /// 0 (clear) to 1.
    pub opacity: f32,
    /// Its border or stroke width in points, for the kinds that have one ([`Shape::has_width`]).
    pub width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Some([1.0, 0.85, 0.0]),
            opacity: 1.0,
            width: 1.0,
        }
    }
}

/// An annotation as the app keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// Its id, a UUID kept in the PDF's `/NM`.
    pub id: String,
    /// Its page, counting from zero.
    pub page: usize,
    /// Its kind and where it is.
    pub shape: Shape,
    /// How it is drawn.
    pub style: Style,
    /// Who made it.
    pub author: String,
    /// When it was made, in seconds since 1970 (UTC).
    pub created: i64,
    /// When it last changed, as `created`.
    pub modified: i64,
    /// Its text: a note's, a text box's, a highlight's comment.
    pub contents: String,
    /// The reader's tags (A.25).
    pub tags: Vec<String>,
    /// The text under it, to find it again in another version of the document (K.28).
    pub anchor: Option<Quote>,
}

impl Annotation {
    /// A new annotation on page `page`, with a new id, made and changed now.
    #[must_use]
    pub fn new(page: usize, shape: Shape, style: Style, author: &str) -> Self {
        let now = now();
        Self {
            id: new_id(),
            page,
            shape,
            style,
            author: author.to_owned(),
            created: now,
            modified: now,
            contents: String::new(),
            tags: Vec::new(),
            anchor: None,
        }
    }

    /// The annotation `data` (read from page `page`) is, or `None` when the model doesn't keep its kind.
    #[must_use]
    pub fn from_data(page: usize, data: &AnnotData) -> Option<Self> {
        let shape = match data.kind {
            AnnotKind::Highlight => Shape::Markup(Markup::Highlight, data.quads.clone()),
            AnnotKind::Underline => Shape::Markup(Markup::Underline, data.quads.clone()),
            AnnotKind::Squiggly => Shape::Markup(Markup::Squiggly, data.quads.clone()),
            AnnotKind::StrikeOut => Shape::Markup(Markup::StrikeOut, data.quads.clone()),
            AnnotKind::Text => Shape::Boxed(Boxed::Note, data.rect),
            AnnotKind::FreeText => Shape::Boxed(Boxed::FreeText, data.rect),
            AnnotKind::Square => Shape::Boxed(Boxed::Square, data.rect),
            AnnotKind::Circle => Shape::Boxed(Boxed::Circle, data.rect),
            AnnotKind::Stamp => Shape::Boxed(Boxed::Stamp, data.rect),
            AnnotKind::Caret => Shape::Boxed(Boxed::Caret, data.rect),
            AnnotKind::Ink => Shape::Ink(data.strokes.clone()),
            AnnotKind::Line => match data.points[..] {
                [a, b] => Shape::Line(a, b),
                _ => return None,
            },
            AnnotKind::Polygon => Shape::Path(Path::Polygon, data.points.clone()),
            AnnotKind::PolyLine => Shape::Path(Path::PolyLine, data.points.clone()),
            _ => return None,
        };
        let private = serde_json::from_str::<Value>(&data.app_data).ok();
        let private = private.as_ref().and_then(Value::as_object);
        let tags = private
            .and_then(|object| object.get("tags")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|tag| tag.as_str().map(str::to_owned))
            .collect();
        let anchor = private.and_then(|object| quote_from_json(object.get("anchor")?));
        Some(Self {
            id: data.id.clone(),
            page,
            style: Style {
                color: rgb(&data.color),
                opacity: data.opacity,
                width: if shape.has_width() { data.border } else { 0.0 },
            },
            shape,
            author: data.author.clone(),
            created: data.created,
            modified: data.modified,
            contents: data.contents.clone(),
            tags,
            anchor,
        })
    }

    /// The annotation as MuPDF writes it.
    #[must_use]
    pub fn to_data(&self) -> AnnotData {
        let (mut quads, mut strokes, mut points, mut rect) =
            (Vec::new(), Vec::new(), Vec::new(), PageRect::default());
        match &self.shape {
            Shape::Markup(_, q) => quads.clone_from(q),
            Shape::Boxed(_, r) => rect = *r,
            Shape::Ink(s) => strokes.clone_from(s),
            Shape::Line(a, b) => points = vec![*a, *b],
            Shape::Path(_, v) => points.clone_from(v),
        }
        AnnotData {
            kind: self.shape.kind(),
            id: self.id.clone(),
            author: self.author.clone(),
            contents: self.contents.clone(),
            app_data: self.private_json(),
            rect,
            color: self.style.color.map(Vec::from).unwrap_or_default(),
            opacity: self.style.opacity,
            border: self.style.width,
            created: self.created,
            modified: self.modified,
            quads,
            strokes,
            points,
        }
    }

    /// The private key's text: the tags and the anchor, empty when there are neither.
    fn private_json(&self) -> String {
        let mut object = Map::new();
        if !self.tags.is_empty() {
            object.insert("tags".into(), json!(self.tags));
        }
        if let Some(anchor) = &self.anchor {
            object.insert("anchor".into(), quote_to_json(anchor));
        }
        if object.is_empty() {
            String::new()
        } else {
            Value::Object(object).to_string()
        }
    }

    /// The annotation as the operation log writes it.
    pub(crate) fn to_json(&self) -> Value {
        let (shape, geometry) = match &self.shape {
            Shape::Markup(kind, quads) => (
                format!("{kind:?}"),
                Value::Array(quads.iter().map(quad_to_json).collect()),
            ),
            Shape::Boxed(kind, r) => (format!("{kind:?}"), json!([r.x0, r.y0, r.x1, r.y1])),
            Shape::Ink(strokes) => (
                "Ink".to_owned(),
                Value::Array(strokes.iter().map(|s| points_to_json(s)).collect()),
            ),
            Shape::Line(a, b) => ("Line".to_owned(), points_to_json(&[*a, *b])),
            Shape::Path(kind, vertices) => (format!("{kind:?}"), points_to_json(vertices)),
        };
        json!({
            "id": self.id,
            "page": self.page,
            "shape": shape,
            "geometry": geometry,
            "color": self.style.color,
            "opacity": self.style.opacity,
            "width": self.style.width,
            "author": self.author,
            "created": self.created,
            "modified": self.modified,
            "contents": self.contents,
            "tags": self.tags,
            "anchor": self.anchor.as_ref().map(quote_to_json),
        })
    }

    /// The annotation the log wrote as `value`.
    pub(crate) fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |key: &str| Some(object.get(key)?.as_str()?.to_owned());
        let number = |key: &str| object.get(key)?.as_f64().map(|n| n as f32);
        let geometry = object.get("geometry")?;
        let shape = match object.get("shape")?.as_str()? {
            "Highlight" => Shape::Markup(Markup::Highlight, quads_from_json(geometry)?),
            "Underline" => Shape::Markup(Markup::Underline, quads_from_json(geometry)?),
            "Squiggly" => Shape::Markup(Markup::Squiggly, quads_from_json(geometry)?),
            "StrikeOut" => Shape::Markup(Markup::StrikeOut, quads_from_json(geometry)?),
            "Note" => Shape::Boxed(Boxed::Note, rect_from_json(geometry)?),
            "FreeText" => Shape::Boxed(Boxed::FreeText, rect_from_json(geometry)?),
            "Square" => Shape::Boxed(Boxed::Square, rect_from_json(geometry)?),
            "Circle" => Shape::Boxed(Boxed::Circle, rect_from_json(geometry)?),
            "Stamp" => Shape::Boxed(Boxed::Stamp, rect_from_json(geometry)?),
            "Caret" => Shape::Boxed(Boxed::Caret, rect_from_json(geometry)?),
            "Ink" => Shape::Ink(
                geometry
                    .as_array()?
                    .iter()
                    .map(points_from_json)
                    .collect::<Option<_>>()?,
            ),
            "Line" => match points_from_json(geometry)?[..] {
                [a, b] => Shape::Line(a, b),
                _ => return None,
            },
            "Polygon" => Shape::Path(Path::Polygon, points_from_json(geometry)?),
            "PolyLine" => Shape::Path(Path::PolyLine, points_from_json(geometry)?),
            _ => return None,
        };
        let color = match object.get("color")? {
            Value::Null => None,
            value => Some(<[f32; 3]>::try_from(floats(value)?).ok()?),
        };
        Some(Self {
            id: text("id")?,
            page: usize::try_from(object.get("page")?.as_u64()?).ok()?,
            shape,
            style: Style {
                color,
                opacity: number("opacity")?,
                width: number("width")?,
            },
            author: text("author")?,
            created: object.get("created")?.as_i64()?,
            modified: object.get("modified")?.as_i64()?,
            contents: text("contents")?,
            tags: object
                .get("tags")?
                .as_array()?
                .iter()
                .map(|tag| Some(tag.as_str()?.to_owned()))
                .collect::<Option<_>>()?,
            anchor: match object.get("anchor")? {
                Value::Null => None,
                value => Some(quote_from_json(value)?),
            },
        })
    }
}

/// The annotations the model keeps on page `page` of `document`, in the page's order. Other kinds are skipped, and
/// stay in the file as they are.
///
/// # Errors
///
/// MuPDF can't load the page or read an annotation.
pub fn page_annotations(
    document: &Document,
    page: usize,
) -> excellent_mupdf::Result<Vec<Annotation>> {
    let loaded = document.load_page(page)?;
    let mut found = Vec::new();
    for annot in loaded.annots()? {
        if let Some(annotation) = Annotation::from_data(page, &annot.data()?) {
            found.push(annotation);
        }
    }
    Ok(found)
}

/// Seconds since 1970, now.
#[must_use]
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// A colour read from a PDF as RGB: gray spread to three, CMYK converted naively.
fn rgb(color: &[f32]) -> Option<[f32; 3]> {
    match *color {
        [gray] => Some([gray; 3]),
        [r, g, b] => Some([r, g, b]),
        [c, m, y, k] => Some([
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ]),
        _ => None,
    }
}

fn quote_to_json(quote: &Quote) -> Value {
    json!({"prefix": quote.prefix, "exact": quote.exact, "suffix": quote.suffix, "start": quote.start})
}

fn quote_from_json(value: &Value) -> Option<Quote> {
    let object = value.as_object()?;
    let text = |key: &str| Some(object.get(key)?.as_str()?.to_owned());
    Some(Quote {
        prefix: text("prefix")?,
        exact: text("exact")?,
        suffix: text("suffix")?,
        start: usize::try_from(object.get("start")?.as_u64()?).ok()?,
    })
}

fn floats(value: &Value) -> Option<Vec<f32>> {
    value
        .as_array()?
        .iter()
        .map(|n| n.as_f64().map(|n| n as f32))
        .collect()
}

fn points_to_json(points: &[PagePoint]) -> Value {
    Value::Array(points.iter().map(|p| json!([p.x, p.y])).collect())
}

fn points_from_json(value: &Value) -> Option<Vec<PagePoint>> {
    value
        .as_array()?
        .iter()
        .map(|point| match floats(point)?[..] {
            [x, y] => Some(PagePoint { x, y }),
            _ => None,
        })
        .collect()
}

fn quad_to_json(q: &Quad) -> Value {
    json!([
        q.ul.x, q.ul.y, q.ur.x, q.ur.y, q.ll.x, q.ll.y, q.lr.x, q.lr.y
    ])
}

fn quads_from_json(value: &Value) -> Option<Vec<Quad>> {
    value
        .as_array()?
        .iter()
        .map(|quad| match floats(quad)?[..] {
            [a, b, c, d, e, f, g, h] => Some(Quad {
                ul: PagePoint { x: a, y: b },
                ur: PagePoint { x: c, y: d },
                ll: PagePoint { x: e, y: f },
                lr: PagePoint { x: g, y: h },
            }),
            _ => None,
        })
        .collect()
}

fn rect_from_json(value: &Value) -> Option<PageRect> {
    match floats(value)?[..] {
        [x0, y0, x1, y1] => Some(PageRect { x0, y0, x1, y1 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
