//! The annotation model (plan K.14, step 1): an annotation as the app thinks of it, whatever kind it is and whichever
//! file keeps it. Every annotation tool (A.1 to A.28) makes and changes these; [`crate::edit::Change`] writes them to
//! the PDF inside one operation, so they undo, are logged and are saved like every other edit.
//!
//! Positions are in page space: points from the page's top-left corner, y down, before the reader's rotation, as
//! MuPDF reports the page. An annotation is named by its id (the PDF's `/NM`), never by its place in the page's list.
//! The anchor, tags (K.28) and category (A.17) live in a private key of the annotation's dictionary, so they travel with the file.
//!
//! Only the kinds the app makes are modelled; the others (links, form fields, sounds, other apps' own kinds) are left
//! in the file as they are: [`Annotation::from_data`] gives `None` for them and the model never writes them.

use std::time::{SystemTime, UNIX_EPOCH};

pub use excellent_mupdf::{Align, LineEnd};
use excellent_mupdf::{AnnotData, AnnotKind, Document, Intent, PagePoint, PageRect, Quad};
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
    /// A box of text drawn on the page (A.6).
    FreeText,
    /// Text typed onto the page as on a typewriter, with no box (A.6): a free-text annotation whose intent says so.
    Typewriter,
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
    /// A text box with a leader line to the point it is about, its tip (A.7): a free-text annotation whose intent
    /// says so. The line runs from the tip to the box's edge, so it follows the box when the box moves.
    Callout(PageRect, PagePoint),
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
            Self::Boxed(Boxed::FreeText | Boxed::Typewriter, _) => AnnotKind::FreeText,
            Self::Boxed(Boxed::Square, _) => AnnotKind::Square,
            Self::Boxed(Boxed::Circle, _) => AnnotKind::Circle,
            Self::Boxed(Boxed::Stamp, _) => AnnotKind::Stamp,
            Self::Boxed(Boxed::Caret, _) => AnnotKind::Caret,
            Self::Ink(_) => AnnotKind::Ink,
            Self::Line(..) => AnnotKind::Line,
            Self::Path(Path::Polygon, _) => AnnotKind::Polygon,
            Self::Path(Path::PolyLine, _) => AnnotKind::PolyLine,
            Self::Callout(..) => AnnotKind::FreeText,
        }
    }

    /// Whether the kind has a border or stroke width (PDF's `/BS`); the others draw at their own.
    #[must_use]
    pub fn has_width(&self) -> bool {
        matches!(
            self,
            Self::Boxed(Boxed::FreeText | Boxed::Square | Boxed::Circle, _)
                | Self::Callout(..)
                | Self::Ink(_)
                | Self::Line(..)
                | Self::Path(..)
        )
    }
}

/// A text box's font: one of the standard fonts every PDF reader has, so the box looks the same in each (A.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Font {
    /// Helvetica.
    #[default]
    Sans,
    /// Times.
    Serif,
    /// Courier, a typewriter's.
    Mono,
}

impl Font {
    /// Every font, in the order the inspector offers them.
    pub const ALL: [Self; 3] = [Self::Sans, Self::Serif, Self::Mono];

    /// The name MuPDF gives it in a text box's `/DA`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Sans => "Helv",
            Self::Serif => "TiRo",
            Self::Mono => "Cour",
        }
    }

    /// The font MuPDF names `name`; sans for any other.
    #[must_use]
    pub fn of(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|font| font.name() == name)
            .unwrap_or_default()
    }
}

/// How a text box's text is drawn (A.6): PDF's `/DA` and `/Q`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Its font.
    pub font: Font,
    /// Its size in points.
    pub size: f32,
    /// Its colour, RGB from 0 to 1; a text box's border is drawn in it too.
    pub color: [f32; 3],
    /// How its lines are aligned.
    pub align: Align,
}

impl TextStyle {
    /// The size a text box's text gets when its file gives none (MuPDF's and Acrobat's).
    pub const SIZE: f32 = 12.0;
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: Font::Sans,
            size: Self::SIZE,
            color: [0.0; 3],
            align: Align::Left,
        }
    }
}

/// How an annotation is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Its colour, RGB from 0 to 1; `None` for none (a shape's outline left out). A text box's is its background.
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

/// A reply's review state in a comment thread (A.21): PDF's `/State`, with its `/StateModel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// The reviewer agrees.
    Accepted,
    /// The reviewer disagrees.
    Rejected,
    /// The comment no longer applies.
    Cancelled,
    /// What it asks for is done.
    Completed,
    /// A status set earlier is taken back (PDF's `None`).
    Cleared,
    /// Ticked off by the reader (the Marked model, which Acrobat's check mark writes).
    Marked,
    /// The tick taken off again.
    Unmarked,
}

impl Status {
    /// The review statuses a reader sets, in Acrobat's order.
    pub const REVIEW: [Self; 5] = [
        Self::Accepted,
        Self::Rejected,
        Self::Cancelled,
        Self::Completed,
        Self::Cleared,
    ];

    /// Its name in the file (`/State`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Accepted => "Accepted",
            Self::Rejected => "Rejected",
            Self::Cancelled => "Cancelled",
            Self::Completed => "Completed",
            Self::Cleared => "None",
            Self::Marked => "Marked",
            Self::Unmarked => "Unmarked",
        }
    }

    /// The status the file names `name`, or `None` for a name no model has.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Marked, Self::Unmarked]
            .into_iter()
            .chain(Self::REVIEW)
            .find(|status| status.name() == name)
    }

    /// Whether it belongs to the Marked model rather than the Review one.
    #[must_use]
    pub fn is_mark(self) -> bool {
        matches!(self, Self::Marked | Self::Unmarked)
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
    /// The id of the annotation on its page it goes with: a "replace text" strikeout names its caret (A.2).
    pub group: Option<String>,
    /// The id of the annotation on its page it answers: a reply in a comment thread (A.21). Replies aren't drawn on
    /// the page; they are read in their thread.
    pub reply_to: Option<String>,
    /// A reply's review status (A.21); `None` for a reply that is only text, and for everything else.
    pub state: Option<Status>,
    /// The name of the layer it is on (A.22): hidden with it; `None` for none.
    pub layer: Option<String>,
    /// Its semantic category's key (A.17), such as `claim`; `None` for none.
    pub category: Option<String>,
    /// Its icon's name (PDF's `/Name`): a sticky note's (A.5) or a stamp's; empty for none, which readers draw as
    /// the kind's default.
    pub icon: String,
    /// Whether other readers show its note open (PDF's `/Open`): a sticky note's own, or its popup's (A.4, A.5).
    pub open: bool,
    /// A text box's text style (A.6); the default for the other kinds, which don't keep one.
    pub text: TextStyle,
    /// How a line, polyline or polygon starts and ends (A.8), or a callout's leader line ends at its tip (the first,
    /// A.7); none for the other kinds.
    pub ends: [LineEnd; 2],
    /// A rectangle's, ellipse's or polygon's fill, RGB from 0 to 1; `None` for none (A.9).
    pub fill: Option<[f32; 3]>,
    /// Its border's dash pattern, dash and gap lengths in points; empty for a solid border (A.9).
    pub dash: Vec<f32>,
    /// A closed shape's cloudy border's intensity, from 0 to 2; `None` for a plain border (A.10).
    pub cloud: Option<f32>,
    /// Whether it is locked (PDF's `/F` Locked): not to be moved, resized, deleted or restyled; its note can still
    /// be written (A.12).
    pub locked: bool,
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
            group: None,
            reply_to: None,
            state: None,
            layer: None,
            category: None,
            icon: String::new(),
            open: false,
            text: TextStyle::default(),
            ends: [LineEnd::None; 2],
            fill: None,
            dash: Vec::new(),
            cloud: None,
            locked: false,
        }
    }

    /// Whether it answers another annotation in a comment thread (A.21), and so isn't on the page.
    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.reply_to.is_some()
    }

    /// Whether it is a text box, boxed or typed (A.6).
    #[must_use]
    pub fn is_text_box(&self) -> bool {
        matches!(
            self.shape,
            Shape::Boxed(Boxed::FreeText | Boxed::Typewriter, _) | Shape::Callout(..)
        )
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
            AnnotKind::FreeText if data.intent == Intent::Typewriter => {
                Shape::Boxed(Boxed::Typewriter, data.rect)
            }
            AnnotKind::FreeText if data.intent == Intent::Callout => {
                // One with no line yet points at its box's top left.
                let tip = data.callout.first().copied().unwrap_or(PagePoint {
                    x: data.rect.x0,
                    y: data.rect.y0,
                });
                Shape::Callout(data.rect, tip)
            }
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
        let category =
            private.and_then(|object| Some(object.get("category")?.as_str()?.to_owned()));
        let text = if data.kind == AnnotKind::FreeText {
            TextStyle {
                font: Font::of(&data.font),
                size: if data.font_size > 0.0 {
                    data.font_size
                } else {
                    TextStyle::SIZE
                },
                color: rgb(&data.text_color).unwrap_or_default(),
                align: data.align,
            }
        } else {
            TextStyle::default()
        };
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
            group: (!data.group.is_empty()).then(|| data.group.clone()),
            reply_to: (!data.reply_to.is_empty()).then(|| data.reply_to.clone()),
            state: Status::from_name(&data.state),
            layer: (!data.layer.is_empty()).then(|| data.layer.clone()),
            category,
            icon: data.icon.clone(),
            open: data.open,
            text,
            ends: data.ends,
            fill: rgb(&data.fill),
            dash: data.dash.clone(),
            cloud: data.cloud,
            locked: data.locked,
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
            Shape::Callout(r, _) => rect = *r,
        }
        let text = self.is_text_box().then_some(self.text);
        AnnotData {
            kind: self.shape.kind(),
            id: self.id.clone(),
            author: self.author.clone(),
            contents: self.contents.clone(),
            app_data: self.private_json(),
            group: self.group.clone().unwrap_or_default(),
            reply_to: self.reply_to.clone().unwrap_or_default(),
            state: self
                .state
                .map(|state| state.name().to_owned())
                .unwrap_or_default(),
            layer: self.layer.clone().unwrap_or_default(),
            icon: self.icon.clone(),
            open: self.open,
            intent: match self.shape {
                Shape::Boxed(Boxed::Typewriter, _) => Intent::Typewriter,
                Shape::Callout(..) => Intent::Callout,
                _ => Intent::Plain,
            },
            ends: self.ends,
            callout: match self.shape {
                Shape::Callout(_, tip) => vec![tip],
                _ => Vec::new(),
            },
            align: text.map(|text| text.align).unwrap_or_default(),
            font: text
                .map(|text| text.font.name().to_owned())
                .unwrap_or_default(),
            font_size: text.map_or(0.0, |text| text.size),
            text_color: text.map(|text| Vec::from(text.color)).unwrap_or_default(),
            fill: self.fill.map(Vec::from).unwrap_or_default(),
            dash: self.dash.clone(),
            cloud: self.cloud,
            locked: self.locked,
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

    /// The private key's text: the tags, the anchor and the category, empty when there are none.
    fn private_json(&self) -> String {
        let mut object = Map::new();
        if !self.tags.is_empty() {
            object.insert("tags".into(), json!(self.tags));
        }
        if let Some(anchor) = &self.anchor {
            object.insert("anchor".into(), quote_to_json(anchor));
        }
        if let Some(category) = &self.category {
            object.insert("category".into(), json!(category));
        }
        if object.is_empty() {
            String::new()
        } else {
            Value::Object(object).to_string()
        }
    }

    /// The annotation as the operation log writes it, and the annotation clipboard (A.13).
    #[must_use]
    pub fn to_json(&self) -> Value {
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
            Shape::Callout(r, tip) => (
                "Callout".to_owned(),
                json!([r.x0, r.y0, r.x1, r.y1, tip.x, tip.y]),
            ),
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
            "group": self.group,
            "reply_to": self.reply_to,
            "state": self.state.map(Status::name),
            "layer": self.layer,
            "category": self.category,
            "icon": self.icon,
            "open": self.open,
            "ends": self.ends.map(|end| format!("{end:?}")),
            "fill": self.fill,
            "dash": self.dash,
            "cloud": self.cloud,
            "locked": self.locked,
            "text": self.is_text_box().then(|| json!({
                "font": self.text.font.name(),
                "size": self.text.size,
                "color": self.text.color,
                "align": format!("{:?}", self.text.align),
            })),
        })
    }

    /// The annotation the log, or the annotation clipboard, wrote as `value`.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
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
            "Typewriter" => Shape::Boxed(Boxed::Typewriter, rect_from_json(geometry)?),
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
            "Callout" => match floats(geometry)?[..] {
                [x0, y0, x1, y1, x, y] => {
                    Shape::Callout(PageRect { x0, y0, x1, y1 }, PagePoint { x, y })
                }
                _ => return None,
            },
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
            // Logs written before A.2 have no group, and before A.5 no icon or open state.
            group: object
                .get("group")
                .and_then(Value::as_str)
                .map(str::to_owned),
            // Before A.21 none answered another.
            reply_to: text("reply_to"),
            state: text("state").as_deref().and_then(Status::from_name),
            // Before A.22 none was on a layer.
            layer: text("layer"),
            // Before A.17 none had a category.
            category: text("category"),
            icon: text("icon").unwrap_or_default(),
            open: object.get("open").and_then(Value::as_bool).unwrap_or(false),
            // Before A.6 no text box kept a style.
            text: object
                .get("text")
                .and_then(text_style_from_json)
                .unwrap_or_default(),
            // Before A.7 none kept its line endings.
            ends: object
                .get("ends")
                .and_then(ends_from_json)
                .unwrap_or_default(),
            // Before A.9 none kept a fill or dashes.
            fill: object
                .get("fill")
                .and_then(floats)
                .and_then(|fill| <[f32; 3]>::try_from(fill).ok()),
            dash: object.get("dash").and_then(floats).unwrap_or_default(),
            // Before A.10 none had a cloudy border.
            cloud: object
                .get("cloud")
                .and_then(Value::as_f64)
                .map(|intensity| intensity as f32),
            // Before A.12 none was locked.
            locked: object
                .get("locked")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        })
    }
}

/// The annotations the model keeps on page `page` of `document`, in the page's order, the replies in comment threads
/// left out (they aren't on the page: [`page_replies`]). Other kinds are skipped, and stay in the file as they are.
///
/// # Errors
///
/// MuPDF can't load the page or read an annotation.
pub fn page_annotations(
    document: &Document,
    page: usize,
) -> excellent_mupdf::Result<Vec<Annotation>> {
    let mut found = read_page(document, page)?;
    found.retain(|annotation| !annotation.is_reply());
    Ok(found)
}

/// The replies in comment threads on page `page` of `document` (A.21), statuses among them, in the page's order.
///
/// # Errors
///
/// MuPDF can't load the page or read an annotation.
pub fn page_replies(document: &Document, page: usize) -> excellent_mupdf::Result<Vec<Annotation>> {
    let mut found = read_page(document, page)?;
    found.retain(Annotation::is_reply);
    Ok(found)
}

/// The annotations of the FDF file `bytes`, as they would read on `target`'s pages (A.24), replies among them; those
/// for pages `target` lacks, and kinds the model doesn't keep, are left out.
///
/// # Errors
///
/// `bytes` isn't an FDF file, `target` isn't a PDF, or MuPDF can't read them.
pub fn fdf_annotations(
    target: &Document,
    bytes: &[u8],
) -> excellent_mupdf::Result<Vec<Annotation>> {
    let fdf = Document::from_bytes(bytes, "application/pdf")?;
    let holder = target.with_fdf_annotations(&fdf)?;
    let mut found = Vec::new();
    for page in 0..holder.page_count()? {
        found.extend(read_page(&holder, page)?);
    }
    Ok(found)
}

/// Every annotation the model keeps on page `page`, replies and all.
fn read_page(document: &Document, page: usize) -> excellent_mupdf::Result<Vec<Annotation>> {
    let loaded = document.load_page(page)?;
    let mut found = Vec::new();
    for annot in loaded.annots()? {
        if let Some(annotation) = Annotation::from_data(page, &annot.data()?) {
            found.push(annotation);
        }
    }
    Ok(found)
}

/// Every page's annotations of the document `mupdf`, replies in threads (A.21) among them, in page order, for the
/// annotation list (A.14). Runs at
/// background priority, taking the document's lock one page at a time so the pages on screen still draw, and can be
/// cancelled.
pub fn load_all(
    pool: &crate::render::RenderPool,
    mupdf: crate::document::SharedMupdf,
) -> crate::render::Job<Vec<Annotation>> {
    pool.submit(crate::render::Priority::Background, move |cookie| {
        let count = mupdf.lock().expect("document lock poisoned").page_count()?;
        let mut all = Vec::new();
        for page in 0..count {
            cookie.check()?;
            let document = mupdf.lock().expect("document lock poisoned");
            all.extend(read_page(&document, page)?);
        }
        Ok(all)
    })
}

/// Every page's text with its layout, in page order, to find annotations' text in another version (A.16). Runs at
/// background priority, taking the document's lock one page at a time, and can be cancelled.
pub fn load_text(
    pool: &crate::render::RenderPool,
    mupdf: crate::document::SharedMupdf,
) -> crate::render::Job<Vec<excellent_mupdf::TextLayout>> {
    pool.submit(crate::render::Priority::Background, move |cookie| {
        let count = mupdf.lock().expect("document lock poisoned").page_count()?;
        let mut pages = Vec::with_capacity(count);
        for page in 0..count {
            cookie.check()?;
            let document = mupdf.lock().expect("document lock poisoned");
            pages.push(document.load_page(page)?.text(Some(cookie))?.layout());
        }
        Ok(pages)
    })
}

/// Where an annotation is and how it looks, as the mini-map (R.17) marks it: read without the rest of the
/// annotation, so marking a whole document stays quick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    /// Its kind.
    pub kind: AnnotKind,
    /// Where it is, in page points.
    pub rect: PageRect,
    /// Its colour, `0xRRGGBB`, if it has one.
    pub color: Option<u32>,
}

/// The marks of page `page`'s annotations: every kind but links, pop-ups and form fields, which aren't the reader's
/// own marks.
///
/// # Errors
///
/// MuPDF can't load the page or read an annotation.
pub fn page_marks(document: &Document, page: usize) -> excellent_mupdf::Result<Vec<Mark>> {
    let loaded = document.load_page(page)?;
    let mut marks = Vec::new();
    for annot in loaded.annots()? {
        let kind = annot.kind()?;
        if matches!(kind, AnnotKind::Link | AnnotKind::Popup | AnnotKind::Widget) {
            continue;
        }
        let data = annot.data()?;
        marks.push(Mark {
            kind,
            rect: data.rect,
            color: rgb(&data.color).map(|[r, g, b]| {
                let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
                (byte(r) << 16) | (byte(g) << 8) | byte(b)
            }),
        });
    }
    Ok(marks)
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

fn ends_from_json(value: &Value) -> Option<[LineEnd; 2]> {
    let end = |value: &Value| {
        let name = value.as_str()?;
        LineEnd::ALL
            .into_iter()
            .find(|end| format!("{end:?}") == name)
    };
    match value.as_array()?.as_slice() {
        [start, finish] => Some([end(start)?, end(finish)?]),
        _ => None,
    }
}

fn text_style_from_json(value: &Value) -> Option<TextStyle> {
    let object = value.as_object()?;
    Some(TextStyle {
        font: Font::of(object.get("font")?.as_str()?),
        size: object.get("size")?.as_f64()? as f32,
        color: <[f32; 3]>::try_from(floats(object.get("color")?)?).ok()?,
        align: match object.get("align")?.as_str()? {
            "Center" => Align::Center,
            "Right" => Align::Right,
            _ => Align::Left,
        },
    })
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
