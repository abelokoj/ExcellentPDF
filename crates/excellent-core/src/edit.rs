//! Edits to a document (K.14): the changes one operation makes, as the operation log writes them and as MuPDF
//! applies them. One [`Operation`] is one step of undo, however many changes it holds (B.5: a move of several
//! entries undoes at once).

use excellent_mupdf::{Edit, NewOutlineEntry, OutlineChange, OutlineStyle, StampArt};
use serde_json::{Map, Value, json};

use crate::annot::Annotation;

/// One change to a document. Outline entries are named by their path: their index on each level from the top, as
/// the outline is when the change is applied; annotations by their page and id.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Change {
    /// A new outline entry before the entry at `at`, or one past the last entry of its level (B.1).
    InsertOutlineEntry {
        /// Where it goes.
        at: Vec<usize>,
        /// Its title.
        title: String,
        /// Where it opens, as [`excellent_mupdf::Document::link_uri`] makes it.
        uri: String,
    },
    /// A changed outline entry (B.2, B.4, B.5); `None` keeps the entry's own.
    UpdateOutlineEntry {
        /// The entry.
        at: Vec<usize>,
        /// A new title.
        title: Option<String>,
        /// A new destination.
        uri: Option<String>,
        /// Whether it shows the entries under it.
        open: bool,
    },
    /// The outline entry at `at` deleted, with the entries under it (B.2).
    DeleteOutlineEntry {
        /// The entry.
        at: Vec<usize>,
    },
    /// The outline entry at `at` drawn bold, italic or in a colour (B.7).
    StyleOutlineEntry {
        /// The entry.
        at: Vec<usize>,
        /// How it is drawn.
        style: OutlineStyle,
    },
    /// The outline entry at `from` moved, with the entries under it, to `to`: a path in the outline once it has left
    /// (B.3).
    MoveOutlineEntry {
        /// The entry.
        from: Vec<usize>,
        /// Where it goes.
        to: Vec<usize>,
    },
    /// A new annotation (A.1 to A.28).
    CreateAnnotation(Annotation),
    /// Every property of an annotation written again; it is found by its page and id, and its kind stays.
    UpdateAnnotation(Annotation),
    /// The annotation with id `id` on page `page` deleted.
    DeleteAnnotation {
        /// Its page, counting from zero.
        page: usize,
        /// Its id.
        id: String,
    },
    /// The stamp with id `id` on page `page` drawn as `art` (A.20): after the stamp's
    /// [`Change::CreateAnnotation`], in the same operation.
    StampArt {
        /// Its page, counting from zero.
        page: usize,
        /// Its id.
        id: String,
        /// What it is drawn as.
        art: StampArt,
    },
    /// The layer named `name` shown or hidden, locked or not (A.22), as other readers open the file.
    SetLayer {
        /// Its name.
        name: String,
        /// Whether what is on it is drawn.
        visible: bool,
        /// Whether readers keep it from being shown or hidden.
        locked: bool,
    },
}

/// The pages whose look an operation changes: what is drawn again when it is made, undone or redone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pages {
    /// These, each once, in order; none for an outline's changes.
    These(Vec<usize>),
    /// Every page: a layer was shown or hidden (A.22).
    Every,
}

/// Changes made, and undone, together: one step of the undo history.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// What the user did, as undo names it ("Rename outline entry").
    pub name: String,
    /// What it changes, in order.
    pub changes: Vec<Change>,
}

impl Change {
    /// The page whose look it changes: an annotation's; `None` for the outline and a layer (which is every page's,
    /// [`Operation::pages`]).
    pub fn page(&self) -> Option<usize> {
        match self {
            Self::CreateAnnotation(annotation) | Self::UpdateAnnotation(annotation) => {
                Some(annotation.page)
            }
            Self::DeleteAnnotation { page, .. } | Self::StampArt { page, .. } => Some(*page),
            Self::InsertOutlineEntry { .. }
            | Self::UpdateOutlineEntry { .. }
            | Self::StyleOutlineEntry { .. }
            | Self::DeleteOutlineEntry { .. }
            | Self::MoveOutlineEntry { .. }
            | Self::SetLayer { .. } => None,
        }
    }

    /// Makes the change inside an operation.
    pub(crate) fn apply(&self, edit: &mut Edit<'_>) -> excellent_mupdf::Result<()> {
        match self {
            Self::InsertOutlineEntry { at, title, uri } => {
                edit.insert_outline_entry(at, NewOutlineEntry { title, uri })
            }
            Self::UpdateOutlineEntry {
                at,
                title,
                uri,
                open,
            } => edit.update_outline_entry(
                at,
                OutlineChange {
                    title: title.as_deref(),
                    uri: uri.as_deref(),
                    open: *open,
                },
            ),
            Self::StyleOutlineEntry { at, style } => edit.style_outline_entry(at, *style),
            Self::DeleteOutlineEntry { at } => edit.delete_outline_entry(at),
            Self::MoveOutlineEntry { from, to } => edit.move_outline_entry(from, to),
            Self::CreateAnnotation(annotation) => {
                edit.create_annot(annotation.page, &annotation.to_data())
            }
            Self::UpdateAnnotation(annotation) => {
                edit.update_annot(annotation.page, &annotation.to_data())
            }
            Self::DeleteAnnotation { page, id } => edit.delete_annot(*page, id),
            Self::StampArt { page, id, art } => edit.set_stamp_art(*page, id, art),
            Self::SetLayer {
                name,
                visible,
                locked,
            } => edit.set_layer(name, *visible, *locked),
        }
    }

    /// The change as the operation log writes it.
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::InsertOutlineEntry { at, title, uri } => {
                json!({"kind": "outline.insert", "target": at, "title": title, "uri": uri})
            }
            Self::UpdateOutlineEntry {
                at,
                title,
                uri,
                open,
            } => {
                json!({"kind": "outline.update", "target": at, "title": title, "uri": uri, "open": open})
            }
            Self::StyleOutlineEntry { at, style } => {
                json!({"kind": "outline.style", "target": at, "bold": style.bold, "italic": style.italic,
                    "color": style.color.map(hex)})
            }
            Self::DeleteOutlineEntry { at } => json!({"kind": "outline.delete", "target": at}),
            Self::MoveOutlineEntry { from, to } => {
                json!({"kind": "outline.move", "target": from, "to": to})
            }
            Self::CreateAnnotation(annotation) => {
                json!({"kind": "annot.create", "target": annotation.id, "annotation": annotation.to_json()})
            }
            Self::UpdateAnnotation(annotation) => {
                json!({"kind": "annot.update", "target": annotation.id, "annotation": annotation.to_json()})
            }
            Self::DeleteAnnotation { page, id } => {
                json!({"kind": "annot.delete", "target": id, "page": page})
            }
            Self::StampArt { page, id, art } => {
                let (kind, bytes) = match art {
                    StampArt::Page(bytes) => ("page", bytes),
                    StampArt::Image(bytes) => ("image", bytes),
                };
                json!({"kind": "annot.stamp_art", "target": id, "page": page, "art": kind,
                    "bytes": to_hex(bytes)})
            }
            Self::SetLayer {
                name,
                visible,
                locked,
            } => {
                json!({"kind": "layer.set", "target": name, "visible": visible, "locked": locked})
            }
        }
    }

    /// The change the operation log wrote as `value`; `None` when it isn't one.
    pub(crate) fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let target = || path(object.get("target")?);
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        let annotation = || Annotation::from_json(object.get("annotation")?);
        Some(match object.get("kind")?.as_str()? {
            "outline.insert" => Self::InsertOutlineEntry {
                at: target()?,
                title: text("title")?,
                uri: text("uri")?,
            },
            "outline.update" => Self::UpdateOutlineEntry {
                at: target()?,
                title: text("title"),
                uri: text("uri"),
                open: object.get("open")?.as_bool()?,
            },
            "outline.style" => Self::StyleOutlineEntry {
                at: target()?,
                style: OutlineStyle {
                    bold: object.get("bold")?.as_bool()?,
                    italic: object.get("italic")?.as_bool()?,
                    color: match object.get("color")? {
                        Value::Null => None,
                        value => Some(rgb(value.as_str()?)?),
                    },
                },
            },
            "outline.delete" => Self::DeleteOutlineEntry { at: target()? },
            "outline.move" => Self::MoveOutlineEntry {
                from: target()?,
                to: path(object.get("to")?)?,
            },
            "annot.create" => Self::CreateAnnotation(annotation()?),
            "annot.update" => Self::UpdateAnnotation(annotation()?),
            "annot.delete" => Self::DeleteAnnotation {
                page: usize::try_from(object.get("page")?.as_u64()?).ok()?,
                id: text("target")?,
            },
            "annot.stamp_art" => {
                let bytes = from_hex(object.get("bytes")?.as_str()?)?;
                Self::StampArt {
                    page: usize::try_from(object.get("page")?.as_u64()?).ok()?,
                    id: text("target")?,
                    art: match object.get("art")?.as_str()? {
                        "page" => StampArt::Page(bytes),
                        "image" => StampArt::Image(bytes),
                        _ => return None,
                    },
                }
            }
            "layer.set" => Self::SetLayer {
                name: text("target")?,
                visible: object.get("visible")?.as_bool()?,
                locked: object.get("locked")?.as_bool()?,
            },
            _ => return None,
        })
    }
}

impl Operation {
    /// An operation named `name` making `changes`.
    pub fn new(name: impl Into<String>, changes: Vec<Change>) -> Self {
        Self {
            name: name.into(),
            changes,
        }
    }

    /// The pages whose look it changes: the pages of its annotations (an outline is no page's), or every page when
    /// it shows or hides a layer.
    pub fn pages(&self) -> Pages {
        if self
            .changes
            .iter()
            .any(|change| matches!(change, Change::SetLayer { .. }))
        {
            return Pages::Every;
        }
        let mut pages: Vec<usize> = self.changes.iter().filter_map(Change::page).collect();
        pages.sort_unstable();
        pages.dedup();
        Pages::These(pages)
    }

    /// Makes every change, in order, inside one MuPDF operation.
    pub(crate) fn apply(
        &self,
        document: &mut excellent_mupdf::Document,
    ) -> excellent_mupdf::Result<()> {
        document.operation(&self.name, |edit| {
            self.changes
                .iter()
                .try_for_each(|change| change.apply(edit))
        })
    }

    /// Its fields as the operation log writes them, beside the log's own.
    pub(crate) fn write_json(&self, record: &mut Map<String, Value>) {
        record.insert("name".into(), self.name.clone().into());
        let changes = self.changes.iter().map(Change::to_json).collect();
        record.insert("changes".into(), Value::Array(changes));
    }

    /// The operation the log wrote in `record`.
    pub(crate) fn read_json(record: &Map<String, Value>) -> Option<Self> {
        let name = record.get("name")?.as_str()?.to_owned();
        let changes = record
            .get("changes")?
            .as_array()?
            .iter()
            .map(Change::from_json)
            .collect::<Option<Vec<_>>>()?;
        Some(Self { name, changes })
    }
}

/// Bytes as the log writes them: two lower-case hex digits each.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// The bytes the log wrote with [`to_hex`].
fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// A colour as the log writes it: `#rrggbb`.
fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour the log wrote as `#rrggbb`.
fn rgb(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// An outline path the log wrote.
fn path(value: &Value) -> Option<Vec<usize>> {
    value
        .as_array()?
        .iter()
        .map(|index| usize::try_from(index.as_u64()?).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_read_back_as_they_were_written() {
        let changes = vec![
            Change::InsertOutlineEntry {
                at: vec![2, 0],
                title: "Café \"quoted\"".into(),
                uri: "#page=3&zoom=150,72,92".into(),
            },
            Change::UpdateOutlineEntry {
                at: vec![0],
                title: None,
                uri: Some("#page=1".into()),
                open: true,
            },
            Change::DeleteOutlineEntry { at: vec![1, 4, 2] },
            Change::StyleOutlineEntry {
                at: vec![3],
                style: OutlineStyle {
                    bold: true,
                    italic: false,
                    color: Some([0xd9, 0x3a, 0x0b]),
                },
            },
            Change::StyleOutlineEntry {
                at: vec![3, 1],
                style: OutlineStyle::default(),
            },
            Change::MoveOutlineEntry {
                from: vec![0],
                to: vec![0, 3],
            },
            Change::StampArt {
                page: 2,
                id: "stamp-1".into(),
                art: StampArt::Page(vec![0, 0x25, 0xff, 7]),
            },
            Change::StampArt {
                page: 0,
                id: "stamp-2".into(),
                art: StampArt::Image(Vec::new()),
            },
            Change::SetLayer {
                name: "Ada's \"notes\"".into(),
                visible: false,
                locked: true,
            },
        ];
        let operation = Operation::new("Several", changes);
        let mut record = Map::new();
        operation.write_json(&mut record);
        let text = Value::Object(record).to_string();
        let read: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            Operation::read_json(read.as_object().expect("object")),
            Some(operation)
        );
    }

    #[test]
    fn an_unknown_change_is_not_read() {
        let value = json!({"kind": "page.rotate", "target": [0]});
        assert_eq!(Change::from_json(&value), None);
        let value = json!({"kind": "annot.stamp_art", "target": "a", "page": 0, "art": "page", "bytes": "abc"});
        assert_eq!(Change::from_json(&value), None, "odd hex");
        let value = json!({"kind": "outline.delete", "target": [-1]});
        assert_eq!(Change::from_json(&value), None);
        let value = json!({"kind": "outline.style", "target": [0], "bold": true, "italic": false, "color": "red"});
        assert_eq!(Change::from_json(&value), None);
    }
}
