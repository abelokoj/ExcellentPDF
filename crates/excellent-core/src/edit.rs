//! Edits to a document (K.14): the changes one operation makes, as the operation log writes them and as MuPDF
//! applies them. One [`Operation`] is one step of undo, however many changes it holds (B.5: a move of several
//! entries undoes at once).

use excellent_mupdf::{Edit, NewOutlineEntry, OutlineChange};
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
            Self::DeleteOutlineEntry { at } => edit.delete_outline_entry(at),
            Self::MoveOutlineEntry { from, to } => edit.move_outline_entry(from, to),
            Self::CreateAnnotation(annotation) => {
                edit.create_annot(annotation.page, &annotation.to_data())
            }
            Self::UpdateAnnotation(annotation) => {
                edit.update_annot(annotation.page, &annotation.to_data())
            }
            Self::DeleteAnnotation { page, id } => edit.delete_annot(*page, id),
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
            Change::MoveOutlineEntry {
                from: vec![0],
                to: vec![0, 3],
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
        let value = json!({"kind": "outline.delete", "target": [-1]});
        assert_eq!(Change::from_json(&value), None);
    }
}
