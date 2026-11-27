//! W3C Web Annotation export and import (A.28): annotations as JSON-LD in the Web Annotation Data Model, so web
//! annotation tools (Hypothesis and the like) and research pipelines can read them. Each annotation targets the PDF
//! with a page fragment (RFC 3778) and, when it marks text, a `TextQuoteSelector` and `TextPositionSelector` from its
//! K.28 anchor; its note and tags are textual bodies. The whole annotation also rides in a property of our own, so a
//! file exported here comes back without loss.
//!
//! Import gives back what the file holds: annotations exported here as they were (merge them by id, as XFDF does,
//! with [`super::xfdf::merge`]), and other tools' annotations of a text quote as highlights to be found in the
//! document's text (A.16's [`super::transfer::transfer`] places them).

use std::fmt;

use ecclesia_core::anchor::{Quote, new_id};
use ecclesia_core::annot::{Annotation, Markup, Shape, Style};
use ecclesia_mupdf::PageRect;
use serde_json::{Map, Value, json};

/// The Web Annotation Data Model's JSON-LD context.
pub const CONTEXT: &str = "http://www.w3.org/ns/anno.jsonld";

/// Our property holding the whole annotation, as the operation log writes it: an absolute IRI, which JSON-LD allows
/// beside a context's terms.
const OURS: &str = "urn:ecclesiapdf:annotation";

/// The fragment syntax of PDF page selectors.
const PDF_FRAGMENT: &str = "http://tools.ietf.org/rfc/rfc3778";

/// Why a W3C Web Annotation file can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadW3cError(String);

impl fmt::Display for ReadW3cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a readable Web Annotation file: {}", self.0)
    }
}

impl std::error::Error for ReadW3cError {}

/// What a W3C Web Annotation file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    /// Annotations exported here, as they were: merge them by id.
    pub own: Vec<Annotation>,
    /// Other tools' annotations of a text quote, as highlights with that quote as their anchor and no place yet: find
    /// them in the document's text.
    pub quoted: Vec<Annotation>,
    /// Annotations neither kind: no text quote, or a target that isn't this document's text.
    pub skipped: usize,
}

/// `annotations` as an `AnnotationCollection` targeting the PDF at `source` (a file name or URL).
#[must_use]
pub fn write(annotations: &[Annotation], source: &str, label: &str) -> Value {
    let items: Vec<Value> = annotations.iter().map(|a| one(a, source)).collect();
    json!({
        "@context": CONTEXT,
        "type": "AnnotationCollection",
        "label": label,
        "total": items.len(),
        "first": {
            "type": "AnnotationPage",
            "startIndex": 0,
            "items": items,
        },
    })
}

fn iri(id: &str) -> String {
    format!("urn:uuid:{id}")
}

fn one(annotation: &Annotation, source: &str) -> Value {
    let mut object = Map::new();
    object.insert("id".into(), json!(iri(&annotation.id)));
    object.insert("type".into(), json!("Annotation"));
    let motivation = if annotation.is_reply() {
        "replying"
    } else if matches!(annotation.shape, Shape::Markup(..)) {
        "highlighting"
    } else {
        "commenting"
    };
    object.insert("motivation".into(), json!(motivation));
    if !annotation.author.is_empty() {
        object.insert(
            "creator".into(),
            json!({"type": "Person", "name": annotation.author}),
        );
    }
    if annotation.created != 0 {
        object.insert("created".into(), json!(iso_date(annotation.created)));
    }
    if annotation.modified != 0 {
        object.insert("modified".into(), json!(iso_date(annotation.modified)));
    }
    let mut bodies = Vec::new();
    if !annotation.contents.is_empty() {
        bodies.push(json!({
            "type": "TextualBody",
            "value": annotation.contents,
            "format": "text/plain",
            "purpose": "commenting",
        }));
    }
    for tag in &annotation.tags {
        bodies.push(json!({"type": "TextualBody", "value": tag, "purpose": "tagging"}));
    }
    if let Some(state) = annotation.state {
        bodies.push(json!({"type": "TextualBody", "value": state.name(), "purpose": "assessing"}));
    }
    match bodies.len() {
        0 => {}
        1 => {
            object.insert("body".into(), bodies.remove(0));
        }
        _ => {
            object.insert("body".into(), Value::Array(bodies));
        }
    }
    let target = match &annotation.reply_to {
        // A reply is about its parent.
        Some(parent) => json!(iri(parent)),
        None => {
            let mut fragment = format!("page={}", annotation.page + 1);
            if let Some(r) = bounds(&annotation.shape) {
                fragment.push_str(&format!(
                    "&viewrect={},{},{},{}",
                    r.x0.round(),
                    r.y0.round(),
                    (r.x1 - r.x0).round(),
                    (r.y1 - r.y0).round()
                ));
            }
            let mut selectors = vec![json!({
                "type": "FragmentSelector",
                "conformsTo": PDF_FRAGMENT,
                "value": fragment,
            })];
            if let Some(quote) = &annotation.anchor {
                selectors.push(json!({
                    "type": "TextQuoteSelector",
                    "exact": quote.exact,
                    "prefix": quote.prefix,
                    "suffix": quote.suffix,
                }));
                selectors.push(json!({
                    "type": "TextPositionSelector",
                    "start": quote.start,
                    "end": quote.start + quote.exact.chars().count(),
                }));
            }
            json!({"source": source, "selector": selectors})
        }
    };
    object.insert("target".into(), target);
    object.insert(OURS.into(), annotation.to_json());
    Value::Object(object)
}

/// What `shape` covers on its page, in points.
fn bounds(shape: &Shape) -> Option<PageRect> {
    let points: Vec<(f32, f32)> = match shape {
        Shape::Boxed(_, r) | Shape::Callout(r, _) => return Some(*r),
        Shape::Markup(_, quads) => quads
            .iter()
            .flat_map(|q| [q.ul, q.ur, q.ll, q.lr])
            .map(|p| (p.x, p.y))
            .collect(),
        Shape::Ink(strokes) => strokes.iter().flatten().map(|p| (p.x, p.y)).collect(),
        Shape::Line(a, b) => vec![(a.x, a.y), (b.x, b.y)],
        Shape::Path(_, points) => points.iter().map(|p| (p.x, p.y)).collect(),
    };
    let first = *points.first()?;
    Some(points.iter().fold(
        PageRect {
            x0: first.0,
            y0: first.1,
            x1: first.0,
            y1: first.1,
        },
        |r, &(x, y)| PageRect {
            x0: r.x0.min(x),
            y0: r.y0.min(y),
            x1: r.x1.max(x),
            y1: r.y1.max(y),
        },
    ))
}

/// The annotations of the W3C Web Annotation JSON `text`: a collection (its first page's items), a page, a list or
/// one annotation.
///
/// # Errors
///
/// `text` isn't JSON, or holds no annotation.
pub fn read(text: &str) -> Result<Imported, ReadW3cError> {
    let value: Value = serde_json::from_str(text).map_err(|err| ReadW3cError(err.to_string()))?;
    let items = items(&value).ok_or_else(|| ReadW3cError("no annotations in it".into()))?;
    let mut imported = Imported::default();
    for item in items {
        if let Some(annotation) = item.get(OURS).and_then(Annotation::from_json) {
            imported.own.push(annotation);
        } else if let Some(annotation) = quoted(item) {
            imported.quoted.push(annotation);
        } else {
            imported.skipped += 1;
        }
    }
    Ok(imported)
}

/// The annotations `value` holds, wherever the model lets them be.
fn items(value: &Value) -> Option<&[Value]> {
    match value {
        Value::Array(items) => Some(items),
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some("AnnotationCollection") => object.get("first").and_then(items),
            Some("AnnotationPage") => object.get("items")?.as_array().map(Vec::as_slice),
            Some("Annotation") => Some(std::slice::from_ref(value)),
            _ => None,
        },
        _ => None,
    }
}

/// Another tool's annotation of a text quote as a highlight to be placed: its quote, page (from a page fragment,
/// else the first), note, tags and author.
fn quoted(item: &Value) -> Option<Annotation> {
    let target = match item.get("target")? {
        Value::Array(targets) => targets.first()?,
        target => target,
    };
    let selectors: Vec<&Value> = match target.get("selector")? {
        Value::Array(selectors) => selectors.iter().collect(),
        selector => vec![selector],
    };
    let of_type = |kind: &str| {
        selectors
            .iter()
            .find(|s| s.get("type").and_then(Value::as_str) == Some(kind))
            .copied()
    };
    let text = |selector: &Value, key: &str| {
        selector
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let quote = of_type("TextQuoteSelector")?;
    let exact = text(quote, "exact");
    if exact.trim().is_empty() {
        return None;
    }
    let start = of_type("TextPositionSelector")
        .and_then(|s| s.get("start")?.as_u64())
        .and_then(|start| usize::try_from(start).ok())
        .unwrap_or(0);
    let page = of_type("FragmentSelector")
        .map(|s| text(s, "value"))
        .and_then(|fragment| {
            fragment
                .split('&')
                .find_map(|part| part.strip_prefix("page=")?.parse::<usize>().ok())
        })
        .map_or(0, |page| page.saturating_sub(1));
    let mut annotation = Annotation::new(
        page,
        Shape::Markup(Markup::Highlight, Vec::new()),
        Style::default(),
        item.get("creator")
            .and_then(|creator| match creator {
                Value::String(name) => Some(name.as_str()),
                creator => creator.get("name")?.as_str(),
            })
            .unwrap_or_default(),
    );
    annotation.id = new_id();
    annotation.anchor = Some(Quote {
        prefix: text(quote, "prefix"),
        exact,
        suffix: text(quote, "suffix"),
        start,
    });
    let bodies: Vec<&Value> = match item.get("body") {
        Some(Value::Array(bodies)) => bodies.iter().collect(),
        Some(body) => vec![body],
        None => Vec::new(),
    };
    for body in bodies {
        let value = match body {
            Value::String(value) => value.clone(),
            body => text(body, "value"),
        };
        match body.get("purpose").and_then(Value::as_str) {
            Some("tagging") => annotation.tags.push(value),
            _ if annotation.contents.is_empty() => annotation.contents = value,
            _ => {}
        }
    }
    let date = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .and_then(parse_iso_date)
    };
    if let Some(created) = date("created") {
        annotation.created = created;
        annotation.modified = date("modified").unwrap_or(created);
    }
    Some(annotation)
}

/// `seconds` since 1970 as an ISO 8601 date and time in UTC (`2026-11-16T14:30:00Z`).
fn iso_date(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = super::inspector::civil(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// An ISO 8601 date and time (`YYYY-MM-DDTHH:MM:SS`, then `Z`, an offset or nothing for UTC; fractions of a second
/// dropped) as seconds since 1970.
fn parse_iso_date(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (
        number(11..13).unwrap_or(0),
        number(14..16).unwrap_or(0),
        number(17..19).unwrap_or(0),
    );
    // The zone follows the seconds and any fraction.
    let zone = text
        .get(19..)
        .unwrap_or_default()
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone.as_bytes().first() {
        Some(sign @ (b'+' | b'-')) => {
            let hours = zone.get(1..3)?.parse::<i64>().ok()?;
            let minutes = zone
                .get(4..6)
                .or_else(|| zone.get(3..5))
                .and_then(|m| m.parse::<i64>().ok());
            let seconds = (hours * 60 + minutes.unwrap_or(0)) * 60;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ => 0,
    };
    Some(
        super::xfdf::days_from_civil(year, month, day) * 86_400
            + hour * 3600
            + minute * 60
            + second
            - offset,
    )
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::{Boxed, Status};
    use ecclesia_mupdf::{PagePoint, Quad};

    use super::*;

    fn highlight() -> Annotation {
        let quad = Quad {
            ul: PagePoint { x: 72.0, y: 100.0 },
            ur: PagePoint { x: 200.0, y: 100.0 },
            ll: PagePoint { x: 72.0, y: 112.0 },
            lr: PagePoint { x: 200.0, y: 112.0 },
        };
        let mut highlight = Annotation::new(
            2,
            Shape::Markup(Markup::Highlight, vec![quad]),
            Style::default(),
            "Ada",
        );
        highlight.contents = "Key claim".into();
        highlight.tags = vec!["legal".into(), "todo".into()];
        highlight.anchor = Some(Quote {
            prefix: "as shown, ".into(),
            exact: "the results hold".into(),
            suffix: " in every case".into(),
            start: 1204,
        });
        highlight.created = 1_794_839_400;
        highlight.modified = 1_794_839_460;
        highlight
    }

    /// Every term of the Web Annotation context (anno.jsonld) and vocabulary the export uses: the keys and `type`
    /// and `motivation`/`purpose` values. Anything else must be an absolute IRI.
    const TERMS: &[&str] = &[
        "@context",
        "id",
        "type",
        "motivation",
        "creator",
        "name",
        "created",
        "modified",
        "body",
        "value",
        "format",
        "purpose",
        "target",
        "source",
        "selector",
        "conformsTo",
        "exact",
        "prefix",
        "suffix",
        "start",
        "end",
        "label",
        "total",
        "first",
        "items",
        "startIndex",
        "AnnotationCollection",
        "AnnotationPage",
        "Annotation",
        "Person",
        "TextualBody",
        "FragmentSelector",
        "TextQuoteSelector",
        "TextPositionSelector",
        "highlighting",
        "commenting",
        "replying",
        "tagging",
        "assessing",
    ];

    /// Checks that every key in `value`, and every `type`, `motivation` and `purpose`, is a term of the context or
    /// an absolute IRI; our own property's contents are a JSON literal and aren't walked.
    fn check_terms(value: &Value, path: &str) {
        match value {
            Value::Object(object) => {
                for (key, inner) in object {
                    assert!(
                        TERMS.contains(&key.as_str()) || key.contains(':'),
                        "{path}.{key} is no term of the context"
                    );
                    if key == OURS {
                        continue;
                    }
                    if matches!(key.as_str(), "type" | "motivation" | "purpose") {
                        let term = inner.as_str().unwrap_or_default();
                        assert!(TERMS.contains(&term), "{path}.{key} = {term} is no term");
                    }
                    check_terms(inner, &format!("{path}.{key}"));
                }
            }
            Value::Array(items) => items.iter().for_each(|item| check_terms(item, path)),
            _ => {}
        }
    }

    #[test]
    fn the_export_uses_only_the_context_s_terms() {
        let parent = highlight();
        let mut reply = Annotation::new(
            2,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 20.0,
                    y1: 20.0,
                },
            ),
            Style::default(),
            "Grace",
        );
        reply.reply_to = Some(parent.id.clone());
        reply.state = Some(Status::Accepted);
        let ink = Annotation::new(
            0,
            Shape::Ink(vec![vec![
                PagePoint { x: 1.0, y: 2.0 },
                PagePoint { x: 9.0, y: 4.0 },
            ]]),
            Style::default(),
            "",
        );
        let exported = write(&[parent.clone(), reply, ink], "report.pdf", "report.pdf");
        assert_eq!(exported["@context"], CONTEXT);
        check_terms(&exported, "$");
        let first = &exported["first"]["items"][0];
        assert_eq!(first["id"], format!("urn:uuid:{}", parent.id));
        assert_eq!(first["created"], "2026-11-16T14:30:00Z");
        assert_eq!(
            first["target"]["selector"][0]["value"],
            "page=3&viewrect=72,100,128,12"
        );
        assert_eq!(first["target"]["selector"][1]["exact"], "the results hold");
        assert_eq!(first["target"]["selector"][2]["end"], 1220);
        assert_eq!(
            exported["first"]["items"][1]["target"],
            format!("urn:uuid:{}", parent.id)
        );
    }

    #[test]
    fn our_export_comes_back_whole() {
        let all = vec![highlight()];
        let text = write(&all, "a.pdf", "a.pdf").to_string();
        let back = read(&text).expect("read");
        assert_eq!(back.own, all);
        assert!(back.quoted.is_empty());
    }

    #[test]
    fn another_tool_s_quote_becomes_a_highlight_to_place() {
        let text = r#"[{
            "@context": "http://www.w3.org/ns/anno.jsonld",
            "id": "https://example.org/anno/1",
            "type": "Annotation",
            "creator": "Grace",
            "created": "2026-11-16T09:30:00.250-05:00",
            "body": [
                {"type": "TextualBody", "value": "Is this right?", "purpose": "commenting"},
                {"type": "TextualBody", "value": "check", "purpose": "tagging"}
            ],
            "target": {
                "source": "https://example.org/paper.pdf",
                "selector": [
                    {"type": "TextQuoteSelector", "exact": "the results hold", "prefix": "as shown, "},
                    {"type": "FragmentSelector", "value": "page=4"},
                    {"type": "TextPositionSelector", "start": 99, "end": 115}
                ]
            }
        }, {
            "type": "Annotation",
            "body": "a page note",
            "target": "https://example.org/paper.pdf"
        }]"#;
        let back = read(text).expect("read");
        assert_eq!(back.skipped, 1, "no quote: nothing to place");
        let [found] = &back.quoted[..] else {
            panic!("one quote: {back:#?}");
        };
        assert_eq!(found.page, 3);
        assert_eq!(found.author, "Grace");
        assert_eq!(found.contents, "Is this right?");
        assert_eq!(found.tags, ["check"]);
        assert_eq!(found.created, 1_794_839_400);
        let anchor = found.anchor.as_ref().expect("anchor");
        assert_eq!(
            (anchor.exact.as_str(), anchor.start),
            ("the results hold", 99)
        );
        assert!(read("{}").is_err());
        assert!(read("not json").is_err());
    }

    #[test]
    fn iso_dates_go_both_ways() {
        assert_eq!(iso_date(1_794_839_400), "2026-11-16T14:30:00Z");
        assert_eq!(parse_iso_date("2026-11-16T14:30:00Z"), Some(1_794_839_400));
        assert_eq!(
            parse_iso_date("2026-11-16T15:30:00+01:00"),
            Some(1_794_839_400)
        );
        assert_eq!(parse_iso_date("yesterday"), None);
    }
}
