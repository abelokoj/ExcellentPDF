//! XFDF import and export (A.24): annotations as ISO 19444-1 XML, which Acrobat and most PDF tools read and write.
//! An annotation is written from [`AnnotData`], the flat form MuPDF reads and writes, so every kind the model keeps
//! goes out and comes back the same way the file does; positions are converted between page space and PDF space (the
//! file's own coordinates, y up) with each page's [`PageSpace`]. What only this app keeps (tags, the anchor, the
//! category, the layer) rides in attributes of our own namespace, which other readers pass over.
//!
//! Import merges by id (`/NM`): an annotation already here is updated, a new one added ([`merge`]). FDF, the same
//! thing in PDF syntax, is read and written by MuPDF (`excellent_core::annot::fdf_annotations`,
//! `excellent_mupdf::Document::to_fdf`) and merged the same way.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use excellent_core::anchor::new_id;
use excellent_core::annot::Annotation;
use excellent_core::edit::Change;
use excellent_mupdf::{
    Align, AnnotData, AnnotKind, Intent, LineEnd, Matrix, PagePoint, PageRect, Quad,
};
use quick_xml::escape::{escape, resolve_predefined_entity};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::Value;

/// XFDF's namespace.
const XFDF: &str = "http://ns.adobe.com/xfdf/";

/// Our own namespace, for what only this app keeps.
const OURS: &str = "urn:excellentpdf:xfdf";

/// Our attribute holding the tags, anchor and category (the annotation's private key, as JSON).
const DATA: &str = "xpdf:data";

/// Our attribute naming the layer (A.22).
const LAYER: &str = "xpdf:layer";

/// Every line end, to name and read them (XFDF names them as PDF does).
const ENDS: [LineEnd; 10] = [
    LineEnd::None,
    LineEnd::Square,
    LineEnd::Circle,
    LineEnd::Diamond,
    LineEnd::OpenArrow,
    LineEnd::ClosedArrow,
    LineEnd::Butt,
    LineEnd::ROpenArrow,
    LineEnd::RClosedArrow,
    LineEnd::Slash,
];

/// Why an XFDF file can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadXfdfError(String);

impl fmt::Display for ReadXfdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a readable XFDF file: {}", self.0)
    }
}

impl std::error::Error for ReadXfdfError {}

/// A page's map between page space (as annotations are read here) and PDF space (as XFDF writes them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSpace {
    /// PDF space to page space.
    to_page: Matrix,
    /// Page space to PDF space.
    to_pdf: Matrix,
}

impl PageSpace {
    /// The page whose PDF-to-page map is `to_page` (MuPDF's page transform); a map that can't be undone counts as
    /// none.
    #[must_use]
    pub fn new(to_page: Matrix) -> Self {
        let det = to_page.a * to_page.d - to_page.b * to_page.c;
        if det.abs() < f32::EPSILON {
            return Self {
                to_page: Matrix::IDENTITY,
                to_pdf: Matrix::IDENTITY,
            };
        }
        let (a, b, c, d) = (
            to_page.d / det,
            -to_page.b / det,
            -to_page.c / det,
            to_page.a / det,
        );
        Self {
            to_page,
            to_pdf: Matrix {
                a,
                b,
                c,
                d,
                e: -(to_page.e * a + to_page.f * c),
                f: -(to_page.e * b + to_page.f * d),
            },
        }
    }

    fn pdf(&self, p: PagePoint) -> (f32, f32) {
        map(&self.to_pdf, p.x, p.y)
    }

    fn page(&self, x: f32, y: f32) -> PagePoint {
        let (x, y) = map(&self.to_page, x, y);
        PagePoint { x, y }
    }

    /// `rect` (page space) as PDF space's `x0,y0,x1,y1`, its corners sorted.
    fn pdf_rect(&self, rect: PageRect) -> [f32; 4] {
        let corners = [
            self.pdf(PagePoint {
                x: rect.x0,
                y: rect.y0,
            }),
            self.pdf(PagePoint {
                x: rect.x1,
                y: rect.y1,
            }),
        ];
        [
            corners[0].0.min(corners[1].0),
            corners[0].1.min(corners[1].1),
            corners[0].0.max(corners[1].0),
            corners[0].1.max(corners[1].1),
        ]
    }

    /// PDF space's `[x0, y0, x1, y1]` as a page rectangle.
    fn page_rect(&self, r: [f32; 4]) -> PageRect {
        let (a, b) = (self.page(r[0], r[1]), self.page(r[2], r[3]));
        PageRect {
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        }
    }
}

fn map(m: &Matrix, x: f32, y: f32) -> (f32, f32) {
    (x * m.a + y * m.c + m.e, x * m.b + y * m.d + m.f)
}

/// What importing found: the changes to make here, one step of undo, and how many of each.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    /// The edits, parents before the replies and group members that name them.
    pub changes: Vec<Change>,
    /// Annotations new here.
    pub added: usize,
    /// Annotations here that the file changes.
    pub updated: usize,
    /// Annotations here already as the file has them.
    pub unchanged: usize,
    /// Annotations left out: locked here (A.12), or on a page the document lacks.
    pub skipped: usize,
}

/// `annotations` (each on a page of `spaces`) as an XFDF file for the PDF named `file`.
#[must_use]
pub fn write(annotations: &[Annotation], spaces: &[PageSpace], file: &str) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xfdf xmlns=\"{XFDF}\" xmlns:xpdf=\"{OURS}\" \
         xml:space=\"preserve\">\n<annots>\n"
    );
    for annotation in annotations {
        if let Some(space) = spaces.get(annotation.page) {
            element(&mut out, &annotation.to_data(), annotation.page, space);
        }
    }
    let _ = writeln!(out, "</annots>\n<f href=\"{}\"/>\n</xfdf>", escape(file));
    out
}

/// The XFDF element name of `kind`; `None` for a kind the model doesn't keep.
fn element_name(kind: AnnotKind) -> Option<&'static str> {
    Some(match kind {
        AnnotKind::Text => "text",
        AnnotKind::FreeText => "freetext",
        AnnotKind::Line => "line",
        AnnotKind::Square => "square",
        AnnotKind::Circle => "circle",
        AnnotKind::Polygon => "polygon",
        AnnotKind::PolyLine => "polyline",
        AnnotKind::Highlight => "highlight",
        AnnotKind::Underline => "underline",
        AnnotKind::Squiggly => "squiggly",
        AnnotKind::StrikeOut => "strikeout",
        AnnotKind::Stamp => "stamp",
        AnnotKind::Caret => "caret",
        AnnotKind::Ink => "ink",
        _ => return None,
    })
}

/// The kind XFDF names `name`.
fn kind_named(name: &str) -> Option<AnnotKind> {
    [
        AnnotKind::Text,
        AnnotKind::FreeText,
        AnnotKind::Line,
        AnnotKind::Square,
        AnnotKind::Circle,
        AnnotKind::Polygon,
        AnnotKind::PolyLine,
        AnnotKind::Highlight,
        AnnotKind::Underline,
        AnnotKind::Squiggly,
        AnnotKind::StrikeOut,
        AnnotKind::Stamp,
        AnnotKind::Caret,
        AnnotKind::Ink,
    ]
    .into_iter()
    .find(|&kind| element_name(kind) == Some(name))
}

/// Whether `kind` has a border or stroke width.
fn has_width(kind: AnnotKind) -> bool {
    matches!(
        kind,
        AnnotKind::FreeText
            | AnnotKind::Square
            | AnnotKind::Circle
            | AnnotKind::Ink
            | AnnotKind::Line
            | AnnotKind::Polygon
            | AnnotKind::PolyLine
    )
}

/// The page rectangle `data` covers: its rectangle, or what holds its quads, strokes or points (and a callout's
/// tip), its line's width around them.
fn covers(data: &AnnotData) -> PageRect {
    let mut points: Vec<PagePoint> = data
        .quads
        .iter()
        .flat_map(|q| [q.ul, q.ur, q.ll, q.lr])
        .chain(data.strokes.iter().flatten().copied())
        .chain(data.points.iter().copied())
        .collect();
    if points.is_empty() {
        if data.callout.is_empty() {
            return data.rect;
        }
        points.extend(&data.callout);
        points.push(PagePoint {
            x: data.rect.x0,
            y: data.rect.y0,
        });
        points.push(PagePoint {
            x: data.rect.x1,
            y: data.rect.y1,
        });
    }
    let pad = if has_width(data.kind) {
        data.border / 2.0
    } else {
        0.0
    };
    let mut rect = PageRect {
        x0: f32::MAX,
        y0: f32::MAX,
        x1: f32::MIN,
        y1: f32::MIN,
    };
    for p in points {
        rect.x0 = rect.x0.min(p.x - pad);
        rect.y0 = rect.y0.min(p.y - pad);
        rect.x1 = rect.x1.max(p.x + pad);
        rect.y1 = rect.y1.max(p.y + pad);
    }
    rect
}

/// A number as XFDF writes it: up to four decimals, no trailing zeros.
fn number(n: f32) -> String {
    let text = format!("{n:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn numbers(values: impl IntoIterator<Item = f32>) -> String {
    values.into_iter().map(number).collect::<Vec<_>>().join(",")
}

/// `points` in PDF space as `x,y;x,y`.
fn point_list(points: &[PagePoint], space: &PageSpace) -> String {
    points
        .iter()
        .map(|&p| {
            let (x, y) = space.pdf(p);
            format!("{},{}", number(x), number(y))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// `rgb` (0 to 1) as `#RRGGBB`; `None` unless it has three parts.
fn hex(rgb: &[f32]) -> Option<String> {
    let [r, g, b] = rgb else {
        return None;
    };
    // The cast can't truncate: each part is clamped to 0..=255 first.
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!("#{:02X}{:02X}{:02X}", byte(*r), byte(*g), byte(*b)))
}

/// One annotation's element.
fn element(out: &mut String, data: &AnnotData, page: usize, space: &PageSpace) {
    let Some(name) = element_name(data.kind) else {
        return;
    };
    let mut attrs: Vec<(&str, String)> = vec![
        ("page", page.to_string()),
        ("rect", numbers(space.pdf_rect(covers(data)))),
        ("name", data.id.clone()),
    ];
    let mut push = |key: &'static str, value: String| {
        if !value.is_empty() {
            attrs.push((key, value));
        }
    };
    push("title", data.author.clone());
    push("creationdate", pdf_date(data.created));
    push("date", pdf_date(data.modified));
    push("color", hex(&data.color).unwrap_or_default());
    if data.opacity < 1.0 {
        push("opacity", number(data.opacity));
    }
    let mut flags = vec!["print"];
    if !data.state.is_empty() {
        flags.extend(["hidden", "nozoom", "norotate"]);
    }
    if data.locked {
        flags.push("locked");
    }
    push("flags", flags.join(","));
    if !data.reply_to.is_empty() {
        push("inreplyto", data.reply_to.clone());
    } else if !data.group.is_empty() {
        push("inreplyto", data.group.clone());
        push("replyType", "group".to_owned());
    }
    if !data.state.is_empty() {
        let model = if matches!(data.state.as_str(), "Marked" | "Unmarked") {
            "Marked"
        } else {
            "Review"
        };
        push("state", data.state.clone());
        push("statemodel", model.to_owned());
    }
    if matches!(data.kind, AnnotKind::Text | AnnotKind::Stamp) {
        push("icon", data.icon.clone());
    }
    if has_width(data.kind) {
        push("width", number(data.border));
        push("interior-color", hex(&data.fill).unwrap_or_default());
        if let Some(intensity) = data.cloud {
            push("style", "cloudy".to_owned());
            push("intensity", number(intensity));
        } else if !data.dash.is_empty() {
            push("style", "dash".to_owned());
            push("dashes", numbers(data.dash.iter().copied()));
        }
    }
    // A line's, polyline's or polygon's ends, or a callout's at its tip.
    if data.ends != [LineEnd::None; 2] {
        push("head", format!("{:?}", data.ends[0]));
        push("tail", format!("{:?}", data.ends[1]));
    }
    match data.kind {
        AnnotKind::Line => {
            if let [start, end] = data.points[..] {
                push("start", point_list(&[start], space).replace(';', ","));
                push("end", point_list(&[end], space).replace(';', ","));
            }
        }
        AnnotKind::Highlight
        | AnnotKind::Underline
        | AnnotKind::Squiggly
        | AnnotKind::StrikeOut => {
            // QuadPoints' order, as Acrobat writes it: upper left, upper right, lower left, lower right.
            let corners: Vec<f32> = data
                .quads
                .iter()
                .flat_map(|q| [q.ul, q.ur, q.ll, q.lr])
                .flat_map(|p| {
                    let (x, y) = space.pdf(p);
                    [x, y]
                })
                .collect();
            push("coords", numbers(corners));
        }
        AnnotKind::FreeText => {
            push("justification", align_number(data.align).to_owned());
            match data.intent {
                Intent::Typewriter => push("intent", "FreeTextTypeWriter".to_owned()),
                Intent::Callout => {
                    push("intent", "FreeTextCallout".to_owned());
                    if let Some(&tip) = data.callout.first() {
                        let edge = PagePoint {
                            x: tip.x.clamp(data.rect.x0, data.rect.x1),
                            y: tip.y.clamp(data.rect.y0, data.rect.y1),
                        };
                        push("callout", point_list(&[tip, edge], space).replace(';', ","));
                        // How far the box sits inside the rectangle the line widens (`/RD`: left, top, right, bottom).
                        let outer = space.pdf_rect(covers(data));
                        let inner = space.pdf_rect(data.rect);
                        push(
                            "fringe",
                            numbers([
                                inner[0] - outer[0],
                                outer[3] - inner[3],
                                outer[2] - inner[2],
                                inner[1] - outer[1],
                            ]),
                        );
                    }
                }
                Intent::Plain => {}
            }
        }
        _ => {}
    }
    push(DATA, data.app_data.clone());
    push(LAYER, data.layer.clone());

    let _ = write!(out, "<{name}");
    for (key, value) in &attrs {
        let _ = write!(out, " {key}=\"{}\"", escape(value.as_str()));
    }
    out.push_str(">\n");
    if !data.contents.is_empty() {
        let _ = writeln!(
            out,
            "<contents>{}</contents>",
            escape(data.contents.as_str())
        );
    }
    if data.kind == AnnotKind::FreeText {
        let font = if data.font.is_empty() {
            "Helv"
        } else {
            &data.font
        };
        let size = if data.font_size > 0.0 {
            data.font_size
        } else {
            12.0
        };
        let color = match data.text_color[..] {
            [r, g, b] => [r, g, b],
            _ => [0.0; 3],
        };
        let _ = writeln!(
            out,
            "<defaultappearance>/{font} {} Tf {} rg</defaultappearance>",
            number(size),
            color.map(number).join(" ")
        );
    }
    match data.kind {
        AnnotKind::Polygon | AnnotKind::PolyLine => {
            let _ = writeln!(
                out,
                "<vertices>{}</vertices>",
                point_list(&data.points, space)
            );
        }
        AnnotKind::Ink => {
            out.push_str("<inklist>");
            for stroke in &data.strokes {
                let _ = write!(out, "<gesture>{}</gesture>", point_list(stroke, space));
            }
            out.push_str("</inklist>\n");
        }
        _ => {}
    }
    let _ = writeln!(out, "</{name}>");
}

fn align_number(align: Align) -> &'static str {
    match align {
        Align::Left => "0",
        Align::Center => "1",
        Align::Right => "2",
    }
}

/// An annotation element being read: its attributes and the text of its children.
#[derive(Default)]
struct Pending {
    kind: Option<AnnotKind>,
    attrs: BTreeMap<String, String>,
    contents: String,
    rich: String,
    appearance: String,
    vertices: String,
    gestures: Vec<String>,
}

/// Which child's text is being read.
#[derive(Clone, Copy, PartialEq)]
enum Reading {
    Nothing,
    Contents,
    Rich,
    Appearance,
    Vertices,
    Gesture,
}

/// The annotations of the XFDF file `xml`, in page space by `spaces`; those for pages `spaces` lacks, and kinds the
/// model doesn't keep, are left out. One without an id gets a new one.
///
/// # Errors
///
/// `xml` isn't well-formed XML.
pub fn read(xml: &str, spaces: &[PageSpace]) -> Result<Vec<Annotation>, ReadXfdfError> {
    let mut reader = Reader::from_str(xml);
    let mut found = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut into = Reading::Nothing;
    // How deep inside the annotation element the reader is, so its own end is told from its children's.
    let mut depth = 0usize;
    let mut in_annots = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|err| ReadXfdfError(err.to_string()))?;
        match event {
            Event::Start(start) => {
                let name = local(&start);
                if let Some(current) = pending.as_mut() {
                    depth += 1;
                    into = match name.as_str() {
                        "contents" => Reading::Contents,
                        "contents-richtext" => Reading::Rich,
                        "defaultappearance" => Reading::Appearance,
                        "vertices" => Reading::Vertices,
                        "gesture" => {
                            current.gestures.push(String::new());
                            Reading::Gesture
                        }
                        _ if into == Reading::Rich => Reading::Rich,
                        _ => Reading::Nothing,
                    };
                } else if name == "annots" {
                    in_annots = true;
                } else if in_annots {
                    pending = Some(begin(&start, &name)?);
                    depth = 0;
                }
            }
            Event::Empty(start) => {
                let name = local(&start);
                if pending.is_none() && in_annots {
                    let done = begin(&start, &name)?;
                    found.extend(finish(done, spaces));
                }
            }
            Event::Text(text) => {
                if let Some(current) = pending.as_mut() {
                    take(current, into, &text.xml_content(XmlVersion::Implicit1_0));
                }
            }
            // `&lt;` and the like come apart from the text around them.
            Event::GeneralRef(reference) => {
                if let Some(current) = pending.as_mut() {
                    let name = reference.xml_content(XmlVersion::Implicit1_0);
                    let resolved = match reference
                        .resolve_char_ref()
                        .map_err(|err| ReadXfdfError(err.to_string()))?
                    {
                        Some(char) => char.to_string(),
                        None => resolve_predefined_entity(&name)
                            .ok_or_else(|| ReadXfdfError(format!("unknown entity &{name};")))?
                            .to_owned(),
                    };
                    take(current, into, &resolved);
                }
            }
            Event::CData(data) => {
                if let Some(current) = pending.as_mut() {
                    take(current, into, &data);
                }
            }
            Event::End(end) => {
                if pending.is_some() {
                    if depth == 0 {
                        found.extend(pending.take().and_then(|done| finish(done, spaces)));
                        into = Reading::Nothing;
                    } else {
                        depth -= 1;
                        into = if into == Reading::Rich && depth > 0 {
                            Reading::Rich
                        } else {
                            Reading::Nothing
                        };
                    }
                } else if end.local_name().as_ref() == "annots" {
                    in_annots = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(found)
}

/// `start`'s name without its prefix.
fn local(start: &BytesStart<'_>) -> String {
    start.local_name().as_ref().to_owned()
}

/// An annotation element begun: its kind (none for one the model doesn't keep) and attributes.
fn begin(start: &BytesStart<'_>, name: &str) -> Result<Pending, ReadXfdfError> {
    let mut attrs = BTreeMap::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|err| ReadXfdfError(err.to_string()))?;
        let key = attr.key.as_ref().to_owned();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| ReadXfdfError(err.to_string()))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok(Pending {
        kind: kind_named(name),
        attrs,
        ..Pending::default()
    })
}

fn take(current: &mut Pending, into: Reading, text: &str) {
    let target = match into {
        Reading::Contents => &mut current.contents,
        Reading::Rich => &mut current.rich,
        Reading::Appearance => &mut current.appearance,
        Reading::Vertices => &mut current.vertices,
        Reading::Gesture => match current.gestures.last_mut() {
            Some(gesture) => gesture,
            None => return,
        },
        Reading::Nothing => return,
    };
    target.push_str(text);
}

/// The numbers of `text`, however they are separated (commas, semicolons, spaces).
fn parse_numbers(text: &str) -> Vec<f32> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter_map(|part| part.trim().parse().ok())
        .collect()
}

fn parse_points(text: &str, space: &PageSpace) -> Vec<PagePoint> {
    parse_numbers(text)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| space.page(x, y))
        .collect()
}

/// `#RRGGBB` as RGB from 0 to 1; empty for anything else.
fn parse_hex(text: &str) -> Vec<f32> {
    let Some(digits) = text.strip_prefix('#').filter(|d| d.len() == 6) else {
        return Vec::new();
    };
    (0..3)
        .map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).map(|v| f32::from(v) / 255.0))
        .collect::<Result<_, _>>()
        .unwrap_or_default()
}

/// A text box's font, size and colour from its `/DA`.
fn parse_appearance(da: &str) -> (String, f32, Vec<f32>) {
    let words: Vec<&str> = da.split_whitespace().collect();
    let (mut font, mut size, mut color) = (String::new(), 0.0, Vec::new());
    for (i, word) in words.iter().enumerate() {
        let before = |back: usize| i.checked_sub(back).and_then(|at| words.get(at)).copied();
        match *word {
            "Tf" => {
                font = before(2)
                    .and_then(|name| name.strip_prefix('/'))
                    .unwrap_or_default()
                    .to_owned();
                size = before(1).and_then(|n| n.parse().ok()).unwrap_or(0.0);
            }
            "rg" => {
                color = (1..=3)
                    .rev()
                    .filter_map(|back| before(back)?.parse().ok())
                    .collect();
            }
            "g" => {
                color = before(1)
                    .and_then(|n| n.parse::<f32>().ok())
                    .map(|gray| vec![gray; 3])
                    .unwrap_or_default();
            }
            _ => {}
        }
    }
    (font, size, color)
}

/// The annotation `done` describes; `None` for a kind the model doesn't keep, a page `spaces` lacks, or geometry it
/// can't make out.
fn finish(done: Pending, spaces: &[PageSpace]) -> Option<Annotation> {
    let kind = done.kind?;
    let attr = |key: &str| done.attrs.get(key).map(String::as_str).unwrap_or_default();
    let page: usize = attr("page").parse().ok()?;
    let space = spaces.get(page)?;
    let rect_numbers = parse_numbers(attr("rect"));
    let outer: [f32; 4] = rect_numbers.get(..4)?.try_into().ok()?;
    let mut inner = outer;
    let fringe = parse_numbers(attr("fringe"));
    if let [left, top, right, bottom] = fringe[..] {
        inner = [
            outer[0] + left,
            outer[1] + bottom,
            outer[2] - right,
            outer[3] - top,
        ];
    }
    let flags: Vec<&str> = attr("flags").split(',').map(str::trim).collect();
    let group = attr("replyType").eq_ignore_ascii_case("group");
    let (font, font_size, text_color) = parse_appearance(&done.appearance);
    let end = |key: &str| {
        ENDS.into_iter()
            .find(|e| format!("{e:?}") == attr(key))
            .unwrap_or_default()
    };
    let mut data = AnnotData {
        kind,
        id: attr("name").to_owned(),
        author: attr("title").to_owned(),
        contents: if done.contents.is_empty() {
            done.rich.trim().to_owned()
        } else {
            done.contents
        },
        app_data: attr(DATA).to_owned(),
        group: if group {
            attr("inreplyto").to_owned()
        } else {
            String::new()
        },
        reply_to: if group {
            String::new()
        } else {
            attr("inreplyto").to_owned()
        },
        state: attr("state").to_owned(),
        layer: attr(LAYER).to_owned(),
        icon: attr("icon").to_owned(),
        open: false,
        intent: match attr("intent") {
            "FreeTextTypeWriter" => Intent::Typewriter,
            "FreeTextCallout" => Intent::Callout,
            _ => Intent::Plain,
        },
        align: match attr("justification") {
            "1" => Align::Center,
            "2" => Align::Right,
            _ => Align::Left,
        },
        font,
        font_size,
        text_color,
        fill: parse_hex(attr("interior-color")),
        dash: if attr("style") == "dash" {
            parse_numbers(attr("dashes"))
        } else {
            Vec::new()
        },
        cloud: (attr("style") == "cloudy").then(|| attr("intensity").parse().unwrap_or(1.0)),
        locked: flags.contains(&"locked"),
        ends: [end("head"), end("tail")],
        callout: parse_points(attr("callout"), space)
            .into_iter()
            .take(1)
            .collect(),
        rect: space.page_rect(inner),
        color: parse_hex(attr("color")),
        opacity: attr("opacity").parse().unwrap_or(1.0),
        border: attr("width").parse().unwrap_or(1.0),
        created: parse_pdf_date(attr("creationdate")),
        modified: parse_pdf_date(attr("date")),
        quads: Vec::new(),
        strokes: Vec::new(),
        points: Vec::new(),
    };
    match kind {
        AnnotKind::Highlight
        | AnnotKind::Underline
        | AnnotKind::Squiggly
        | AnnotKind::StrikeOut => {
            data.quads = parse_points(attr("coords"), space)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&[ul, ur, ll, lr]| Quad { ul, ur, ll, lr })
                .collect();
            if data.quads.is_empty() {
                return None;
            }
        }
        AnnotKind::Line => {
            let start = parse_points(attr("start"), space);
            let end = parse_points(attr("end"), space);
            data.points = start
                .into_iter()
                .take(1)
                .chain(end.into_iter().take(1))
                .collect();
        }
        AnnotKind::Polygon | AnnotKind::PolyLine => {
            data.points = parse_points(&done.vertices, space);
        }
        AnnotKind::Ink => {
            data.strokes = done
                .gestures
                .iter()
                .map(|gesture| parse_points(gesture, space))
                .filter(|stroke| !stroke.is_empty())
                .collect();
        }
        _ => {}
    }
    if data.id.is_empty() {
        data.id = new_id();
    }
    Annotation::from_data(page, &data)
}

/// `seconds` since 1970 as a PDF date in UTC (`D:YYYYMMDDHHmmSS+00'00'`); empty for 0, a file that doesn't say.
fn pdf_date(seconds: i64) -> String {
    if seconds == 0 {
        return String::new();
    }
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = super::inspector::civil(days);
    format!(
        "D:{year:04}{month:02}{day:02}{:02}{:02}{:02}+00'00'",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// A PDF date (`D:YYYYMMDDHHmmSSOHH'mm'`, every part after the year optional) as seconds since 1970; 0 for one
/// that can't be read.
fn parse_pdf_date(text: &str) -> i64 {
    let text = text.strip_prefix("D:").unwrap_or(text);
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let part = |from: usize, len: usize, default: i64| {
        digits
            .get(from..from + len)
            .and_then(|p| p.parse().ok())
            .unwrap_or(default)
    };
    let year = part(0, 4, 0);
    if year == 0 {
        return 0;
    }
    let (month, day) = (part(4, 2, 1), part(6, 2, 1));
    let (hour, minute, second) = (part(8, 2, 0), part(10, 2, 0), part(12, 2, 0));
    let zone = &text[digits.len()..];
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let numbers: Vec<i64> = zone[1..]
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|p| p.parse().ok())
                .collect();
            let minutes =
                numbers.first().copied().unwrap_or(0) * 60 + numbers.get(1).copied().unwrap_or(0);
            if sign == '-' {
                -minutes * 60
            } else {
                minutes * 60
            }
        }
        _ => 0,
    };
    days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset
}

/// The days from 1970-01-01 to the given date, in the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// What importing `imported` into a document holding `existing` and `pages` pages changes: one with an id already
/// here updates it (unless it is locked here, or already the same), a new one is added.
#[must_use]
pub fn merge(existing: &[Annotation], imported: Vec<Annotation>, pages: usize) -> Merged {
    let mut merged = Merged::default();
    // Deletes (of what moved page) first, then what names no other annotation, then what does.
    let (mut deletes, mut first, mut then) = (Vec::new(), Vec::new(), Vec::new());
    for annotation in imported {
        if annotation.page >= pages {
            merged.skipped += 1;
            continue;
        }
        // Replies and group members name their parent, which must be there before them.
        let names_another = annotation.reply_to.is_some() || annotation.group.is_some();
        let changes = if names_another { &mut then } else { &mut first };
        match existing.iter().find(|here| here.id == annotation.id) {
            Some(here) if here.locked => merged.skipped += 1,
            Some(here) if same(here, &annotation) => merged.unchanged += 1,
            Some(here) if here.page == annotation.page => {
                merged.updated += 1;
                changes.push(Change::UpdateAnnotation(annotation));
            }
            Some(here) => {
                merged.updated += 1;
                deletes.push(Change::DeleteAnnotation {
                    page: here.page,
                    id: here.id.clone(),
                });
                changes.push(Change::CreateAnnotation(annotation));
            }
            None => {
                merged.added += 1;
                changes.push(Change::CreateAnnotation(annotation));
            }
        }
    }
    deletes.append(&mut first);
    deletes.append(&mut then);
    merged.changes = deletes;
    merged
}

/// Whether `a` and `b` are the same annotation to a hundredth of a point: what a trip through PDF space leaves.
fn same(a: &Annotation, b: &Annotation) -> bool {
    rounded(a.to_json()) == rounded(b.to_json())
}

fn rounded(value: Value) -> Value {
    match value {
        Value::Number(n) => n.as_f64().map_or(Value::Number(n), |f| {
            serde_json::json!((f * 100.0).round() / 100.0)
        }),
        Value::Array(items) => Value::Array(items.into_iter().map(rounded).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, rounded(v))).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Boxed, Markup, Path, Shape, Status, Style};

    use super::*;

    /// A US Letter page as MuPDF maps it: y flipped about its height.
    fn letter() -> PageSpace {
        PageSpace::new(Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 792.0,
        })
    }

    fn at(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> PageRect {
        PageRect { x0, y0, x1, y1 }
    }

    fn style() -> Style {
        Style {
            color: Some([1.0, 0.0, 0.0]),
            opacity: 0.5,
            width: 2.0,
        }
    }

    /// One of each kind the model keeps, with what only this app keeps on some.
    fn every_kind() -> Vec<Annotation> {
        let quad = Quad {
            ul: at(72.0, 100.0),
            ur: at(200.0, 100.0),
            ll: at(72.0, 112.0),
            lr: at(200.0, 112.0),
        };
        let mut highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, vec![quad]),
            style(),
            "Ada",
        );
        highlight.contents = "Check <this> & \"that\"".into();
        highlight.tags = vec!["legal".into()];
        highlight.category = Some("question".into());
        highlight.layer = Some("Ada".into());
        highlight.created = 1_763_300_000;
        highlight.modified = 1_763_300_060;
        let mut note = Annotation::new(
            0,
            Shape::Boxed(Boxed::Note, rect(300.0, 300.0, 320.0, 320.0)),
            style(),
            "Ada",
        );
        note.icon = "Comment".into();
        let mut reply = Annotation::new(
            0,
            Shape::Boxed(Boxed::Note, rect(300.0, 300.0, 320.0, 320.0)),
            style(),
            "Grace",
        );
        reply.reply_to = Some(note.id.clone());
        reply.contents = "Agreed".into();
        let mut status = reply.clone();
        status.id = new_id();
        status.state = Some(Status::Accepted);
        let mut text_box = Annotation::new(
            1,
            Shape::Boxed(Boxed::FreeText, rect(100.0, 100.0, 250.0, 150.0)),
            style(),
            "Ada",
        );
        text_box.contents = "Box".into();
        text_box.text.size = 14.0;
        text_box.text.color = [0.0, 0.0, 1.0];
        text_box.text.align = Align::Center;
        let callout = Annotation::new(
            1,
            Shape::Callout(rect(300.0, 100.0, 450.0, 150.0), at(250.0, 300.0)),
            style(),
            "Ada",
        );
        let mut square = Annotation::new(
            1,
            Shape::Boxed(Boxed::Square, rect(50.0, 400.0, 150.0, 500.0)),
            style(),
            "Ada",
        );
        square.fill = Some([0.0, 1.0, 0.0]);
        square.dash = vec![3.0, 2.0];
        let mut cloud = Annotation::new(
            1,
            Shape::Boxed(Boxed::Circle, rect(200.0, 400.0, 300.0, 500.0)),
            style(),
            "Ada",
        );
        cloud.cloud = Some(1.0);
        cloud.locked = true;
        let mut line = Annotation::new(
            0,
            Shape::Line(at(10.0, 10.0), at(100.0, 50.0)),
            style(),
            "Ada",
        );
        line.ends = [LineEnd::None, LineEnd::ClosedArrow];
        let polygon = Annotation::new(
            0,
            Shape::Path(
                Path::Polygon,
                vec![at(10.0, 600.0), at(60.0, 650.0), at(10.0, 700.0)],
            ),
            style(),
            "Ada",
        );
        let ink = Annotation::new(
            0,
            Shape::Ink(vec![
                vec![at(400.0, 600.0), at(410.0, 605.5)],
                vec![at(420.0, 600.0), at(430.0, 610.0)],
            ]),
            style(),
            "Ada",
        );
        let mut stamp = Annotation::new(
            0,
            Shape::Boxed(Boxed::Stamp, rect(400.0, 50.0, 550.0, 90.0)),
            style(),
            "Ada",
        );
        stamp.icon = "Approved".into();
        vec![
            highlight, note, reply, status, text_box, callout, square, cloud, line, polygon, ink,
            stamp,
        ]
    }

    #[test]
    fn every_kind_comes_back_as_it_went_out() {
        let spaces = [letter(), letter()];
        let all = every_kind();
        let xml = write(&all, &spaces, "report & notes.pdf");
        assert!(xml.contains("<f href=\"report &amp; notes.pdf\"/>"));
        assert!(xml.contains("Check &lt;this&gt; &amp; &quot;that&quot;"));
        let back = read(&xml, &spaces).expect("read");
        assert_eq!(back.len(), all.len());
        for (went, came) in all.iter().zip(&back) {
            // What the file keeps of it: a note has no width, say.
            let kept = Annotation::from_data(went.page, &went.to_data()).expect("kept kind");
            assert!(same(&kept, came), "{kept:#?}\ncame back as\n{came:#?}");
        }
    }

    #[test]
    fn positions_are_written_in_pdf_space() {
        let quad = Quad {
            ul: at(72.0, 100.0),
            ur: at(200.0, 100.0),
            ll: at(72.0, 112.0),
            lr: at(200.0, 112.0),
        };
        let highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, vec![quad]),
            style(),
            "Ada",
        );
        let xml = write(&[highlight], &[letter()], "a.pdf");
        assert!(xml.contains("rect=\"72,680,200,692\""), "{xml}");
        assert!(
            xml.contains("coords=\"72,692,200,692,72,680,200,680\""),
            "{xml}"
        );
    }

    #[test]
    fn acrobat_s_xfdf_reads() {
        let xml = r##"<?xml version="1.0" encoding="UTF-8"?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve"><annots>
<square color="#0000FF" creationdate="D:20261116093000-05'00'" flags="print" date="D:20261116093100-05'00'"
 name="b8f3" page="0" rect="100,600,200,700" subject="Rectangle" title="Grace" width="3" interior-color="#FFFF00">
<contents-richtext><body xmlns="http://www.w3.org/1999/xhtml"><p>Look <b>here</b></p></body></contents-richtext>
<popup flags="print,nozoom,norotate" open="no" page="0" rect="612,500,792,620"/></square>
<text name="r1" page="0" rect="0,0,20,20" inreplyto="b8f3" replyType="reply" title="Ada"><contents>Done &#233; &amp; &#x2713;</contents></text>
<link page="0" rect="0,0,10,10"/>
<square name="far" page="7" rect="0,0,10,10"/>
</annots><f href="x.pdf"/></xfdf>"##;
        let found = read(xml, &[letter()]).expect("read");
        assert_eq!(
            found.len(),
            2,
            "the link and the square on a missing page stay out"
        );
        let square = &found[0];
        assert_eq!(square.id, "b8f3");
        assert_eq!(square.author, "Grace");
        assert_eq!(square.contents, "Look here");
        assert_eq!(
            square.shape,
            Shape::Boxed(Boxed::Square, rect(100.0, 92.0, 200.0, 192.0))
        );
        assert_eq!(square.style.color, Some([0.0, 0.0, 1.0]));
        assert_eq!(square.style.width, 3.0);
        assert_eq!(square.fill, Some([1.0, 1.0, 0.0]));
        assert_eq!(square.created, 1_794_839_400, "2026-11-16 14:30 UTC");
        assert_eq!(found[1].reply_to.as_deref(), Some("b8f3"));
        assert_eq!(
            found[1].contents, "Done é & ✓",
            "character and entity references"
        );
        assert!(
            read(
                "<xfdf><annots><square page=\"0\"></text></annots></xfdf>",
                &[letter()]
            )
            .is_err()
        );
    }

    #[test]
    fn dates_go_both_ways() {
        assert_eq!(pdf_date(1_794_839_400), "D:20261116143000+00'00'");
        assert_eq!(parse_pdf_date("D:20261116143000+00'00'"), 1_794_839_400);
        assert_eq!(parse_pdf_date("D:20261116093000-05'00'"), 1_794_839_400);
        assert_eq!(
            parse_pdf_date("D:2026"),
            parse_pdf_date("D:20260101000000Z")
        );
        assert_eq!(parse_pdf_date("yesterday"), 0);
        assert_eq!(pdf_date(0), "");
    }

    #[test]
    fn importing_merges_by_id() {
        let here = every_kind();
        let mut moved = here[0].clone();
        moved.contents = "Changed".into();
        let mut elsewhere = here[6].clone();
        elsewhere.page = 0;
        let locked = here[7].clone();
        let mut new = here[11].clone();
        new.id = new_id();
        let mut far = here[1].clone();
        far.id = new_id();
        far.page = 9;
        let reply = here[2].clone();
        let merged = merge(
            &here,
            vec![reply, moved, elsewhere, here[1].clone(), locked, new, far],
            2,
        );
        assert_eq!(
            (
                merged.added,
                merged.updated,
                merged.unchanged,
                merged.skipped
            ),
            (1, 2, 2, 2),
            "{merged:#?}"
        );
        assert!(matches!(
            merged.changes.last(),
            Some(Change::CreateAnnotation(_)) | Some(Change::UpdateAnnotation(_))
        ),);
        assert!(
            merged
                .changes
                .iter()
                .any(|c| matches!(c, Change::DeleteAnnotation { page: 1, .. }))
        );
    }
}
