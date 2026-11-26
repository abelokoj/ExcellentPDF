//! Annotation history timeline scrubber from the K.14 operation log and point-in-time
//! single-annotation or whole-document restore (plan A.27).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use excellent_core::annot::Annotation;
use excellent_core::edit::{Change, Operation};
use serde_json::Value;

/// Error returned when reading or parsing an operation log for timeline history.
#[derive(Debug)]
#[non_exhaustive]
pub enum HistoryError {
    /// Filesystem I/O failure.
    Io(std::io::Error),
    /// The file does not begin with a valid K.14 oplog start header.
    InvalidLogHeader,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot read operation log: {err}"),
            Self::InvalidLogHeader => f.write_str("not a valid ExcellentPDF operation log"),
        }
    }
}

impl std::error::Error for HistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidLogHeader => None,
        }
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// One change recorded inside an oplog operation relevant to annotation history.
#[derive(Debug, Clone, PartialEq)]
enum AnnotDelta {
    Create(Annotation),
    Update(Annotation),
    Delete { page: usize, id: String },
}

/// One entry in the chronological annotation history timeline (plan A.27).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// Zero-based operation sequence number from the oplog.
    pub op_id: u64,
    /// Wall-clock timestamp in UTC milliseconds since 1970.
    pub timestamp_ms: u64,
    /// Human-readable operation name (e.g. `"Highlight"`, `"Delete annotation"`).
    pub name: String,
    /// Distinct authors of annotations touched by this operation.
    pub authors: Vec<String>,
    /// IDs of annotations created, updated, or deleted by this operation.
    pub affected_ids: Vec<String>,
    /// Whether this operation is active in the final state (false if currently undone).
    pub active: bool,
    deltas: Vec<AnnotDelta>,
}

/// Reconstructed annotation history timeline supporting arbitrary time/step scrubbing and
/// non-destructive point-in-time restore operations (plan A.27).
#[derive(Debug, Clone, Default)]
pub struct AnnotationTimeline {
    base: BTreeMap<String, Annotation>,
    ops: Vec<HistoryEntry>,
    events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineEvent {
    Apply(usize),
    Undo,
    Redo,
}

impl AnnotationTimeline {
    /// Reads and parses an `.oplog` file at `path`, starting from `base_annotations`.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError`] if the file cannot be read or lacks a valid oplog header.
    pub fn from_file(path: &Path, base_annotations: &[Annotation]) -> Result<Self, HistoryError> {
        let text = fs::read_to_string(path)?;
        Self::parse(&text, base_annotations)
    }

    /// Parses K.14 NDJSON oplog text, starting from `base_annotations`.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::InvalidLogHeader`] if the first line is not a schema-1 start record.
    pub fn parse(
        oplog_ndjson: &str,
        base_annotations: &[Annotation],
    ) -> Result<Self, HistoryError> {
        let mut lines = oplog_ndjson.lines().filter(|l| !l.trim().is_empty());
        let Some(first_line) = lines.next() else {
            return Err(HistoryError::InvalidLogHeader);
        };
        let first_val: Value =
            serde_json::from_str(first_line).map_err(|_| HistoryError::InvalidLogHeader)?;
        let is_start = first_val.get("kind").and_then(Value::as_str) == Some("start")
            && first_val.get("schema").and_then(Value::as_u64) == Some(1);
        if !is_start {
            return Err(HistoryError::InvalidLogHeader);
        }

        let mut base = BTreeMap::new();
        for a in base_annotations {
            base.insert(a.id.clone(), a.clone());
        }

        let mut ops: Vec<HistoryEntry> = Vec::new();
        let mut events: Vec<TimelineEvent> = Vec::new();
        let mut done_stack: Vec<usize> = Vec::new();
        let mut undone_stack: Vec<usize> = Vec::new();

        for line in lines {
            let Ok(val) = serde_json::from_str::<Value>(line) else {
                break; // Tolerate torn trailing line just like oplog::read
            };
            let Some(kind) = val.get("kind").and_then(Value::as_str) else {
                break;
            };
            match kind {
                "operation" => {
                    let Some(op_id) = val.get("op").and_then(Value::as_u64) else {
                        break;
                    };
                    let timestamp_ms = val.get("time").and_then(Value::as_u64).unwrap_or(0);
                    let name = val
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Edit")
                        .to_owned();
                    let (deltas, authors, affected_ids) = parse_annot_deltas(&val);
                    let idx = ops.len();
                    ops.push(HistoryEntry {
                        op_id,
                        timestamp_ms,
                        name,
                        authors,
                        affected_ids,
                        active: true,
                        deltas,
                    });
                    for undone_idx in undone_stack.drain(..) {
                        if let Some(entry) = ops.get_mut(undone_idx) {
                            entry.active = false;
                        }
                    }
                    done_stack.push(idx);
                    events.push(TimelineEvent::Apply(idx));
                }
                "undo" => {
                    if let Some(idx) = done_stack.pop() {
                        if let Some(entry) = ops.get_mut(idx) {
                            entry.active = false;
                        }
                        undone_stack.push(idx);
                        events.push(TimelineEvent::Undo);
                    }
                }
                "redo" => {
                    if let Some(idx) = undone_stack.pop() {
                        if let Some(entry) = ops.get_mut(idx) {
                            entry.active = true;
                        }
                        done_stack.push(idx);
                        events.push(TimelineEvent::Redo);
                    }
                }
                "saved" => {}
                _ => break,
            }
        }

        Ok(Self { base, ops, events })
    }

    /// Chronological list of logged operations (who changed what and when).
    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.ops
    }

    /// Scrubs the timeline to `step_count` operations (`0` = base document, `entries().len()` = latest).
    #[must_use]
    pub fn snapshot_at_step(&self, step_count: usize) -> Vec<Annotation> {
        let limit = step_count.min(self.ops.len());
        let mut applied = Vec::new();
        let mut undone = Vec::new();
        for &ev in &self.events {
            match ev {
                TimelineEvent::Apply(idx) => {
                    if idx >= limit {
                        break;
                    }
                    undone.clear();
                    applied.push(idx);
                }
                TimelineEvent::Undo => {
                    if let Some(idx) = applied.pop() {
                        undone.push(idx);
                    }
                }
                TimelineEvent::Redo => {
                    if let Some(idx) = undone.pop() {
                        applied.push(idx);
                    }
                }
            }
        }
        self.materialize(&applied)
    }

    /// Scrubs the timeline to the document state as of `timestamp_ms` (UTC milliseconds since 1970).
    #[must_use]
    pub fn snapshot_at_time(&self, timestamp_ms: u64) -> Vec<Annotation> {
        let step = self
            .ops
            .iter()
            .take_while(|entry| entry.timestamp_ms <= timestamp_ms)
            .count();
        self.snapshot_at_step(step)
    }

    /// Creates a new undoable [`Operation`] restoring a single annotation `target_id` to its state
    /// at `timestamp_ms`. Returns `None` if the annotation already matches that historical state.
    #[must_use]
    pub fn restore_annotation_at_time(
        &self,
        current_annotations: &[Annotation],
        target_id: &str,
        timestamp_ms: u64,
    ) -> Option<Operation> {
        let past = self.snapshot_at_time(timestamp_ms);
        let past_annot = past.into_iter().find(|a| a.id == target_id);
        let curr_annot = current_annotations.iter().find(|a| a.id == target_id);

        let change = match (past_annot, curr_annot) {
            (Some(old), None) => Change::CreateAnnotation(old),
            (Some(old), Some(cur)) if old != *cur => Change::UpdateAnnotation(old),
            (None, Some(cur)) => Change::DeleteAnnotation {
                page: cur.page,
                id: cur.id.clone(),
            },
            _ => return None,
        };
        Some(Operation::new(
            "Restore annotation from history",
            vec![change],
        ))
    }

    /// Creates a single undoable [`Operation`] restoring the entire document's annotations to
    /// their state at `timestamp_ms`. Returns `None` if nothing differs.
    #[must_use]
    pub fn restore_document_at_time(
        &self,
        current_annotations: &[Annotation],
        timestamp_ms: u64,
    ) -> Option<Operation> {
        let past = self.snapshot_at_time(timestamp_ms);
        let past_map: BTreeMap<&str, &Annotation> =
            past.iter().map(|a| (a.id.as_str(), a)).collect();
        let curr_map: BTreeMap<&str, &Annotation> = current_annotations
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();

        let mut changes = Vec::new();
        for (&id, &cur) in &curr_map {
            if !past_map.contains_key(id) {
                changes.push(Change::DeleteAnnotation {
                    page: cur.page,
                    id: cur.id.clone(),
                });
            }
        }
        for (&id, &old) in &past_map {
            match curr_map.get(id) {
                None => changes.push(Change::CreateAnnotation(old.clone())),
                Some(&cur) if *old != *cur => changes.push(Change::UpdateAnnotation(old.clone())),
                Some(_) => {}
            }
        }
        if changes.is_empty() {
            None
        } else {
            Some(Operation::new("Restore document annotations", changes))
        }
    }

    fn materialize(&self, applied_indices: &[usize]) -> Vec<Annotation> {
        let mut state = self.base.clone();
        for &idx in applied_indices {
            if let Some(entry) = self.ops.get(idx) {
                for delta in &entry.deltas {
                    match delta {
                        AnnotDelta::Create(a) | AnnotDelta::Update(a) => {
                            state.insert(a.id.clone(), a.clone());
                        }
                        AnnotDelta::Delete { id, .. } => {
                            state.remove(id);
                        }
                    }
                }
            }
        }
        let mut out: Vec<Annotation> = state.into_values().collect();
        out.sort_by_key(|a| (a.page, a.created));
        out
    }
}

fn parse_annot_deltas(op_record: &Value) -> (Vec<AnnotDelta>, Vec<String>, Vec<String>) {
    let mut deltas = Vec::new();
    let mut authors = Vec::new();
    let mut affected = Vec::new();

    let Some(changes) = op_record.get("changes").and_then(Value::as_array) else {
        return (deltas, authors, affected);
    };
    for ch in changes {
        let Some(kind) = ch.get("kind").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "annot.create" => {
                if let Some(annot) = ch.get("annotation").and_then(Annotation::from_json) {
                    push_unique(&mut authors, &annot.author);
                    push_unique(&mut affected, &annot.id);
                    deltas.push(AnnotDelta::Create(annot));
                }
            }
            "annot.update" => {
                if let Some(annot) = ch.get("annotation").and_then(Annotation::from_json) {
                    push_unique(&mut authors, &annot.author);
                    push_unique(&mut affected, &annot.id);
                    deltas.push(AnnotDelta::Update(annot));
                }
            }
            "annot.delete" => {
                if let (Some(id), Some(page)) = (
                    ch.get("target").and_then(Value::as_str),
                    ch.get("page")
                        .and_then(Value::as_u64)
                        .and_then(|p| usize::try_from(p).ok()),
                ) {
                    push_unique(&mut affected, id);
                    deltas.push(AnnotDelta::Delete {
                        page,
                        id: id.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    (deltas, authors, affected)
}

fn push_unique(list: &mut Vec<String>, item: &str) {
    if !item.is_empty() && !list.iter().any(|existing| existing == item) {
        list.push(item.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Boxed, Shape, Style};
    use excellent_mupdf::PageRect;

    use super::*;

    #[test]
    fn restoring_an_annotation_deleted_a_week_ago_works() {
        const WEEK_MS: u64 = 7 * 24 * 60 * 60 * 1000;
        let t0 = 1_700_000_000_000_u64;
        let t_week_later = t0 + WEEK_MS;

        let mut note = Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 20.0,
                    y0: 30.0,
                    x1: 40.0,
                    y1: 50.0,
                },
            ),
            Style::default(),
            "Ada",
        );
        note.contents = "Important lemma".to_owned();
        let note_id = note.id.clone();

        let log = format!(
            "{{\"kind\":\"start\",\"schema\":1,\"file\":{{\"size\":100,\"modified\":1,\"sample\":2}}}}\n\
             {{\"kind\":\"operation\",\"op\":0,\"time\":{t0},\"name\":\"Add note\",\"changes\":[{{\"kind\":\"annot.create\",\"target\":\"{note_id}\",\"annotation\":{}}}]}}\n\
             {{\"kind\":\"operation\",\"op\":1,\"time\":{t_week_later},\"name\":\"Delete note\",\"changes\":[{{\"kind\":\"annot.delete\",\"target\":\"{note_id}\",\"page\":0}}]}}\n",
            note.to_json()
        );

        let timeline = AnnotationTimeline::parse(&log, &[]).expect("parse");
        assert_eq!(timeline.entries().len(), 2);
        assert_eq!(timeline.entries()[0].authors, ["Ada"]);

        // Scrubbing to t0 shows the annotation; scrubbing to a week later shows it deleted.
        let at_creation = timeline.snapshot_at_time(t0);
        assert_eq!(at_creation.len(), 1);
        assert_eq!(at_creation[0].contents, "Important lemma");

        let current = timeline.snapshot_at_time(t_week_later);
        assert!(current.is_empty());

        // Restoring the deleted annotation produces a CreateAnnotation operation.
        let restore_op = timeline
            .restore_annotation_at_time(&current, &note_id, t0)
            .expect("restore op");
        assert_eq!(restore_op.changes, vec![Change::CreateAnnotation(note)]);
    }
}
