//! Hybrid Logical Clock (HLC), LWW registers, RGA text CRDT, and document state merge (plan Y.1, W.13).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::SyncError;

/// Hybrid Logical Clock timestamp ordered totally by `(wall_ms, logical, device_id)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HlcTimestamp {
    /// Physical wall clock milliseconds since Unix epoch.
    pub wall_ms: u64,
    /// Monotonic Lamport logical counter for ties within the same millisecond.
    pub logical: u32,
    /// Stable originating device identifier used as final tie-breaker.
    pub device_id: String,
}

impl HlcTimestamp {
    /// Creates an initial timestamp for `device_id`.
    #[must_use]
    pub fn new(device_id: &str, wall_ms: u64) -> Self {
        Self {
            wall_ms,
            logical: 0,
            device_id: device_id.to_owned(),
        }
    }

    /// Advances the local clock for a new local event at `now_ms`.
    #[must_use]
    pub fn tick(&mut self, now_ms: u64) -> Self {
        if now_ms > self.wall_ms {
            self.wall_ms = now_ms;
            self.logical = 0;
        } else {
            self.logical = self.logical.saturating_add(1);
        }
        self.clone()
    }

    /// Merges a received remote timestamp at local physical time `now_ms`.
    #[must_use]
    pub fn receive(&mut self, remote: &Self, now_ms: u64) -> Self {
        let max_wall = self.wall_ms.max(remote.wall_ms).max(now_ms);
        if max_wall == self.wall_ms && max_wall == remote.wall_ms {
            self.logical = self.logical.max(remote.logical).saturating_add(1);
        } else if max_wall == self.wall_ms {
            self.logical = self.logical.saturating_add(1);
        } else if max_wall == remote.wall_ms {
            self.logical = remote.logical.saturating_add(1);
        } else {
            self.logical = 0;
        }
        self.wall_ms = max_wall;
        self.clone()
    }

    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "wall_ms": self.wall_ms,
            "logical": self.logical,
            "device_id": self.device_id,
        })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidFormat`] if required fields are missing.
    pub fn from_json(v: &Value) -> Result<Self, SyncError> {
        let wall_ms = v
            .get("wall_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| SyncError::InvalidFormat("hlc missing wall_ms".to_owned()))?;
        let logical = v
            .get("logical")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| SyncError::InvalidFormat("hlc missing logical".to_owned()))?;
        let device_id = v
            .get("device_id")
            .and_then(Value::as_str)
            .ok_or_else(|| SyncError::InvalidFormat("hlc missing device_id".to_owned()))?
            .to_owned();
        Ok(Self {
            wall_ms,
            logical,
            device_id,
        })
    }
}

/// Last-Writer-Wins register ordered by [`HlcTimestamp`] with optional tombstone semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwwRegister<T> {
    /// Stored value (`None` represents a tombstone delete).
    pub value: Option<T>,
    /// Timestamp of the winning write.
    pub stamp: HlcTimestamp,
}

impl<T: Clone> LwwRegister<T> {
    /// Creates an active register holding `value` at `stamp`.
    #[must_use]
    pub const fn set(value: T, stamp: HlcTimestamp) -> Self {
        Self {
            value: Some(value),
            stamp,
        }
    }

    /// Creates a tombstone register at `stamp`.
    #[must_use]
    pub const fn tombstone(stamp: HlcTimestamp) -> Self {
        Self { value: None, stamp }
    }

    /// Merges `candidate` if its [`HlcTimestamp`] strictly succeeds `self.stamp`.
    pub fn merge_candidate(&mut self, value: Option<T>, stamp: &HlcTimestamp) {
        if stamp > &self.stamp {
            self.value = value;
            self.stamp = stamp.clone();
        }
    }

    /// Commutative, associative, idempotent state merge.
    pub fn merge(&mut self, other: &Self) {
        if other.stamp > self.stamp {
            self.value = other.value.clone();
            self.stamp = other.stamp.clone();
        }
    }
}

/// Globally unique identifier for a sequence CRDT element.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CrdtOpId {
    /// Monotonic Lamport clock.
    pub clock: u64,
    /// Originating device id.
    pub device_id: String,
}

impl CrdtOpId {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "clock": self.clock, "device_id": self.device_id })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidFormat`] if fields are missing.
    pub fn from_json(v: &Value) -> Result<Self, SyncError> {
        let clock = v
            .get("clock")
            .and_then(Value::as_u64)
            .ok_or_else(|| SyncError::InvalidFormat("crdt op id missing clock".to_owned()))?;
        let device_id = v
            .get("device_id")
            .and_then(Value::as_str)
            .ok_or_else(|| SyncError::InvalidFormat("crdt op id missing device_id".to_owned()))?
            .to_owned();
        Ok(Self { clock, device_id })
    }
}

/// One node in the RGA sequence CRDT for collaborative note bodies (plan Y.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgaNode {
    /// Unique element id.
    pub id: CrdtOpId,
    /// Parent element id (`None` = root).
    pub parent: Option<CrdtOpId>,
    /// Single character or text atom inserted by this op.
    pub ch: char,
    /// Tombstone marker when deleted.
    pub deleted: bool,
}

/// Deterministic RGA text CRDT that converges identically under arbitrary out-of-order delivery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteTextCrdt {
    nodes: BTreeMap<CrdtOpId, RgaNode>,
    children_by_parent: BTreeMap<Option<CrdtOpId>, BTreeSet<CrdtOpId>>,
    tombstones: BTreeSet<CrdtOpId>,
}

impl NoteTextCrdt {
    /// Creates an empty text CRDT.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `ch` after `parent` with unique `id`.
    pub fn insert(&mut self, id: CrdtOpId, parent: Option<CrdtOpId>, ch: char) {
        let deleted = self.tombstones.contains(&id);
        self.nodes.entry(id.clone()).or_insert(RgaNode {
            id: id.clone(),
            parent: parent.clone(),
            ch,
            deleted,
        });
        self.children_by_parent
            .entry(parent)
            .or_default()
            .insert(id);
    }

    /// Marks `target` as tombstoned (safe even if `target` arrives later).
    pub fn delete(&mut self, target: &CrdtOpId) {
        self.tombstones.insert(target.clone());
        if let Some(node) = self.nodes.get_mut(target) {
            node.deleted = true;
        }
    }

    /// Appends a whole string sequentially starting after `initial_parent`, returning the last inserted id.
    pub fn insert_str(
        &mut self,
        device_id: &str,
        start_clock: &mut u64,
        mut parent: Option<CrdtOpId>,
        text: &str,
    ) -> Option<CrdtOpId> {
        for ch in text.chars() {
            *start_clock += 1;
            let id = CrdtOpId {
                clock: *start_clock,
                device_id: device_id.to_owned(),
            };
            self.insert(id.clone(), parent, ch);
            parent = Some(id);
        }
        parent
    }

    /// Renders the visible text in deterministic RGA traversal order.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.visit_children(&None, &mut out);
        out
    }

    /// Commutative, associative, idempotent merge with another [`NoteTextCrdt`].
    pub fn merge(&mut self, other: &Self) {
        for tomb in &other.tombstones {
            self.delete(tomb);
        }
        for node in other.nodes.values() {
            self.insert(node.id.clone(), node.parent.clone(), node.ch);
            if node.deleted {
                self.delete(&node.id);
            }
        }
    }

    /// Returns the ordered visible [`CrdtOpId`]s matching the rendered text.
    #[must_use]
    pub fn visible_ids(&self) -> Vec<CrdtOpId> {
        let mut ids = Vec::new();
        self.collect_visible_ids(&None, &mut ids);
        ids
    }

    fn visit_children(&self, parent: &Option<CrdtOpId>, out: &mut String) {
        let Some(children) = self.children_by_parent.get(parent) else {
            return;
        };
        // Descending CrdtOpId order guarantees deterministic RGA interleaving.
        for cid in children.iter().rev() {
            if let Some(node) = self.nodes.get(cid)
                && !node.deleted
                && !self.tombstones.contains(cid)
            {
                out.push(node.ch);
            }
            self.visit_children(&Some(cid.clone()), out);
        }
    }

    fn collect_visible_ids(&self, parent: &Option<CrdtOpId>, out: &mut Vec<CrdtOpId>) {
        let Some(children) = self.children_by_parent.get(parent) else {
            return;
        };
        for cid in children.iter().rev() {
            if let Some(node) = self.nodes.get(cid)
                && !node.deleted
                && !self.tombstones.contains(cid)
            {
                out.push(cid.clone());
            }
            self.collect_visible_ids(&Some(cid.clone()), out);
        }
    }
}

/// Synced tab snapshot stored inside a [`SyncedPane`] (plan W.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedTab {
    /// Portable path or token (mapped per device).
    pub path_token: String,
    /// View state JSON.
    pub view: String,
    /// Whether the tab is pinned.
    pub pinned: bool,
    /// Group index in [`SyncedPane::groups`].
    pub group: Option<usize>,
}

/// Synced tab group snapshot (plan W.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedTabGroup {
    /// Group name.
    pub name: String,
    /// Group color.
    pub color: String,
    /// Whether collapsed.
    pub collapsed: bool,
}

/// Synced split pane snapshot (plan W.13).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncedPane {
    /// Ordered open tabs.
    pub open: Vec<SyncedTab>,
    /// Active tab index.
    pub active: Option<usize>,
    /// Groups in this pane.
    pub groups: Vec<SyncedTabGroup>,
}

/// Synced workspace snapshot (plan W.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedWorkspace {
    /// Stable cross-device workspace sync identifier.
    pub sync_id: String,
    /// Workspace display name.
    pub name: String,
    /// Optional icon.
    pub icon: Option<String>,
    /// Optional color.
    pub color: Option<String>,
    /// Optional pane split layout JSON.
    pub layout: Option<String>,
    /// Display order position.
    pub position: i64,
    /// Panes and tabs.
    pub panes: Vec<SyncedPane>,
}

/// Synced bookmark snapshot (plan Y.1).
#[derive(Debug, Clone, PartialEq)]
pub struct SyncedBookmark {
    /// Bookmark identifier.
    pub id: String,
    /// Portable document identifier or path token.
    pub doc_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// X coordinate in page points.
    pub x: f32,
    /// Y coordinate in page points.
    pub y: f32,
    /// Bookmark title.
    pub title: String,
    /// Bookmark note.
    pub note: String,
    /// Bookmark color.
    pub color: String,
    /// Bookmark tags.
    pub tags: Vec<String>,
}

/// Synced note metadata (title, anchor, tags) paired with [`NoteTextCrdt`] body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedNoteMeta {
    /// Note identifier.
    pub note_id: String,
    /// Note title.
    pub title: String,
    /// Optional portable document token.
    pub file_token: Option<String>,
    /// Optional zero-based page index.
    pub page: Option<usize>,
    /// Tags.
    pub tags: Vec<String>,
}

/// Concrete operation recorded in a device `.oplog` or transmitted over the live relay (plan Y.1, Y.4, W.13).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SyncOpKind {
    /// Creates or updates an annotation payload keyed by `(doc_id, annot_id)`.
    UpsertAnnotation {
        /// Document identifier.
        doc_id: String,
        /// Stable annotation `/NM` identifier.
        annot_id: String,
        /// Annotation JSON payload.
        payload: Value,
    },
    /// Tombstones an annotation keyed by `(doc_id, annot_id)`.
    DeleteAnnotation {
        /// Document identifier.
        doc_id: String,
        /// Annotation identifier.
        annot_id: String,
    },
    /// Creates or updates note metadata.
    UpsertNoteMeta(SyncedNoteMeta),
    /// Inserts one character into a note's [`NoteTextCrdt`].
    NoteTextInsert {
        /// Target note id.
        note_id: String,
        /// Unique element id.
        op_id: CrdtOpId,
        /// Parent element id.
        parent: Option<CrdtOpId>,
        /// Inserted character.
        ch: char,
    },
    /// Deletes a character in a note's [`NoteTextCrdt`].
    NoteTextDelete {
        /// Target note id.
        note_id: String,
        /// Target element id.
        target: CrdtOpId,
    },
    /// Tombstones a note.
    DeleteNote {
        /// Target note id.
        note_id: String,
    },
    /// Creates or updates a bookmark.
    UpsertBookmark(SyncedBookmark),
    /// Tombstones a bookmark.
    DeleteBookmark {
        /// Bookmark identifier.
        bookmark_id: String,
    },
    /// Creates or updates a workspace across devices (plan W.13).
    UpsertWorkspace(SyncedWorkspace),
    /// Tombstones a workspace across devices.
    DeleteWorkspace {
        /// Workspace sync identifier.
        workspace_sync_id: String,
    },
}

/// Timestamped CRDT operation record.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncOp {
    /// HLC timestamp providing a strict total order across devices.
    pub stamp: HlcTimestamp,
    /// Operation payload.
    pub kind: SyncOpKind,
}

impl SyncOp {
    /// Serializes to compact JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let kind_val = match &self.kind {
            SyncOpKind::UpsertAnnotation {
                doc_id,
                annot_id,
                payload,
            } => json!({
                "type": "upsert_annotation",
                "doc_id": doc_id,
                "annot_id": annot_id,
                "payload": payload,
            }),
            SyncOpKind::DeleteAnnotation { doc_id, annot_id } => json!({
                "type": "delete_annotation",
                "doc_id": doc_id,
                "annot_id": annot_id,
            }),
            SyncOpKind::UpsertNoteMeta(meta) => json!({
                "type": "upsert_note_meta",
                "note_id": meta.note_id,
                "title": meta.title,
                "file_token": meta.file_token,
                "page": meta.page,
                "tags": meta.tags,
            }),
            SyncOpKind::NoteTextInsert {
                note_id,
                op_id,
                parent,
                ch,
            } => json!({
                "type": "note_text_insert",
                "note_id": note_id,
                "op_id": op_id.to_json(),
                "parent": parent.as_ref().map(CrdtOpId::to_json),
                "ch": ch.to_string(),
            }),
            SyncOpKind::NoteTextDelete { note_id, target } => json!({
                "type": "note_text_delete",
                "note_id": note_id,
                "target": target.to_json(),
            }),
            SyncOpKind::DeleteNote { note_id } => json!({
                "type": "delete_note",
                "note_id": note_id,
            }),
            SyncOpKind::UpsertBookmark(bm) => json!({
                "type": "upsert_bookmark",
                "id": bm.id,
                "doc_id": bm.doc_id,
                "page": bm.page,
                "x": bm.x,
                "y": bm.y,
                "title": bm.title,
                "note": bm.note,
                "color": bm.color,
                "tags": bm.tags,
            }),
            SyncOpKind::DeleteBookmark { bookmark_id } => json!({
                "type": "delete_bookmark",
                "bookmark_id": bookmark_id,
            }),
            SyncOpKind::UpsertWorkspace(ws) => json!({
                "type": "upsert_workspace",
                "workspace": workspace_to_json(ws),
            }),
            SyncOpKind::DeleteWorkspace { workspace_sync_id } => json!({
                "type": "delete_workspace",
                "workspace_sync_id": workspace_sync_id,
            }),
        };
        json!({
            "stamp": self.stamp.to_json(),
            "kind": kind_val,
        })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidFormat`] if the JSON object is malformed.
    pub fn from_json(v: &Value) -> Result<Self, SyncError> {
        let stamp = HlcTimestamp::from_json(
            v.get("stamp")
                .ok_or_else(|| SyncError::InvalidFormat("op missing stamp".to_owned()))?,
        )?;
        let k = v
            .get("kind")
            .ok_or_else(|| SyncError::InvalidFormat("op missing kind".to_owned()))?;
        let ty = k
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| SyncError::InvalidFormat("op missing kind.type".to_owned()))?;

        let kind = match ty {
            "upsert_annotation" => SyncOpKind::UpsertAnnotation {
                doc_id: req_str(k, "doc_id")?,
                annot_id: req_str(k, "annot_id")?,
                payload: k.get("payload").cloned().unwrap_or(Value::Null),
            },
            "delete_annotation" => SyncOpKind::DeleteAnnotation {
                doc_id: req_str(k, "doc_id")?,
                annot_id: req_str(k, "annot_id")?,
            },
            "upsert_note_meta" => SyncOpKind::UpsertNoteMeta(SyncedNoteMeta {
                note_id: req_str(k, "note_id")?,
                title: req_str(k, "title")?,
                file_token: k
                    .get("file_token")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                page: k
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok()),
                tags: k
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            "note_text_insert" => {
                let ch_str = req_str(k, "ch")?;
                let ch = ch_str
                    .chars()
                    .next()
                    .ok_or_else(|| SyncError::InvalidFormat("empty char".to_owned()))?;
                let parent = match k.get("parent") {
                    Some(pv) if !pv.is_null() => Some(CrdtOpId::from_json(pv)?),
                    _ => None,
                };
                SyncOpKind::NoteTextInsert {
                    note_id: req_str(k, "note_id")?,
                    op_id: CrdtOpId::from_json(
                        k.get("op_id")
                            .ok_or_else(|| SyncError::InvalidFormat("missing op_id".to_owned()))?,
                    )?,
                    parent,
                    ch,
                }
            }
            "note_text_delete" => SyncOpKind::NoteTextDelete {
                note_id: req_str(k, "note_id")?,
                target: CrdtOpId::from_json(
                    k.get("target")
                        .ok_or_else(|| SyncError::InvalidFormat("missing target".to_owned()))?,
                )?,
            },
            "delete_note" => SyncOpKind::DeleteNote {
                note_id: req_str(k, "note_id")?,
            },
            "upsert_bookmark" => SyncOpKind::UpsertBookmark(SyncedBookmark {
                id: req_str(k, "id")?,
                doc_id: req_str(k, "doc_id")?,
                page: k
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(0),
                x: k.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                y: k.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                title: k
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                note: k
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                color: k
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                tags: k
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            "delete_bookmark" => SyncOpKind::DeleteBookmark {
                bookmark_id: req_str(k, "bookmark_id")?,
            },
            "upsert_workspace" => SyncOpKind::UpsertWorkspace(workspace_from_json(
                k.get("workspace")
                    .ok_or_else(|| SyncError::InvalidFormat("missing workspace".to_owned()))?,
            )?),
            "delete_workspace" => SyncOpKind::DeleteWorkspace {
                workspace_sync_id: req_str(k, "workspace_sync_id")?,
            },
            other => {
                return Err(SyncError::InvalidFormat(format!(
                    "unknown sync op kind: {other}"
                )));
            }
        };

        Ok(Self { stamp, kind })
    }
}

/// Complete CRDT state for a synced note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedNoteState {
    /// Last-writer-wins note metadata (tombstoned on delete).
    pub meta: LwwRegister<SyncedNoteMeta>,
    /// RGA character sequence for note body.
    pub body: NoteTextCrdt,
}

/// Materialized CRDT state across annotations, notes, bookmarks, and workspaces (plan Y.1, W.13).
#[derive(Debug, Clone, Default)]
pub struct SyncedDocumentState {
    annotations: BTreeMap<(String, String), LwwRegister<Value>>,
    notes: BTreeMap<String, SyncedNoteState>,
    bookmarks: BTreeMap<String, LwwRegister<SyncedBookmark>>,
    workspaces: BTreeMap<String, LwwRegister<SyncedWorkspace>>,
}

impl SyncedDocumentState {
    /// Creates an empty CRDT state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replays an arbitrary slice of [`SyncOp`]s into a converged [`SyncedDocumentState`].
    #[must_use]
    pub fn replay_ops(ops: &[SyncOp]) -> Self {
        let mut state = Self::new();
        for op in ops {
            state.apply_op(op);
        }
        state
    }

    /// Applies one [`SyncOp`] idempotently and commutatively.
    pub fn apply_op(&mut self, op: &SyncOp) {
        match &op.kind {
            SyncOpKind::UpsertAnnotation {
                doc_id,
                annot_id,
                payload,
            } => {
                let key = (doc_id.clone(), annot_id.clone());
                match self.annotations.get_mut(&key) {
                    Some(reg) => reg.merge_candidate(Some(payload.clone()), &op.stamp),
                    None => {
                        self.annotations
                            .insert(key, LwwRegister::set(payload.clone(), op.stamp.clone()));
                    }
                }
            }
            SyncOpKind::DeleteAnnotation { doc_id, annot_id } => {
                let key = (doc_id.clone(), annot_id.clone());
                match self.annotations.get_mut(&key) {
                    Some(reg) => reg.merge_candidate(None, &op.stamp),
                    None => {
                        self.annotations
                            .insert(key, LwwRegister::tombstone(op.stamp.clone()));
                    }
                }
            }
            SyncOpKind::UpsertNoteMeta(meta) => {
                let entry =
                    self.notes
                        .entry(meta.note_id.clone())
                        .or_insert_with(|| SyncedNoteState {
                            meta: LwwRegister::set(meta.clone(), op.stamp.clone()),
                            body: NoteTextCrdt::new(),
                        });
                entry.meta.merge_candidate(Some(meta.clone()), &op.stamp);
            }
            SyncOpKind::NoteTextInsert {
                note_id,
                op_id,
                parent,
                ch,
            } => {
                let entry = self
                    .notes
                    .entry(note_id.clone())
                    .or_insert_with(|| SyncedNoteState {
                        meta: LwwRegister::set(
                            SyncedNoteMeta {
                                note_id: note_id.clone(),
                                title: note_id.clone(),
                                file_token: None,
                                page: None,
                                tags: Vec::new(),
                            },
                            HlcTimestamp::new(&op.stamp.device_id, 0),
                        ),
                        body: NoteTextCrdt::new(),
                    });
                entry.body.insert(op_id.clone(), parent.clone(), *ch);
            }
            SyncOpKind::NoteTextDelete { note_id, target } => {
                let entry = self
                    .notes
                    .entry(note_id.clone())
                    .or_insert_with(|| SyncedNoteState {
                        meta: LwwRegister::tombstone(HlcTimestamp::new(&op.stamp.device_id, 0)),
                        body: NoteTextCrdt::new(),
                    });
                entry.body.delete(target);
            }
            SyncOpKind::DeleteNote { note_id } => {
                let entry = self
                    .notes
                    .entry(note_id.clone())
                    .or_insert_with(|| SyncedNoteState {
                        meta: LwwRegister::tombstone(op.stamp.clone()),
                        body: NoteTextCrdt::new(),
                    });
                entry.meta.merge_candidate(None, &op.stamp);
            }
            SyncOpKind::UpsertBookmark(bm) => match self.bookmarks.get_mut(&bm.id) {
                Some(reg) => reg.merge_candidate(Some(bm.clone()), &op.stamp),
                None => {
                    self.bookmarks.insert(
                        bm.id.clone(),
                        LwwRegister::set(bm.clone(), op.stamp.clone()),
                    );
                }
            },
            SyncOpKind::DeleteBookmark { bookmark_id } => {
                match self.bookmarks.get_mut(bookmark_id) {
                    Some(reg) => reg.merge_candidate(None, &op.stamp),
                    None => {
                        self.bookmarks.insert(
                            bookmark_id.clone(),
                            LwwRegister::tombstone(op.stamp.clone()),
                        );
                    }
                }
            }
            SyncOpKind::UpsertWorkspace(ws) => match self.workspaces.get_mut(&ws.sync_id) {
                Some(reg) => reg.merge_candidate(Some(ws.clone()), &op.stamp),
                None => {
                    self.workspaces.insert(
                        ws.sync_id.clone(),
                        LwwRegister::set(ws.clone(), op.stamp.clone()),
                    );
                }
            },
            SyncOpKind::DeleteWorkspace { workspace_sync_id } => {
                match self.workspaces.get_mut(workspace_sync_id) {
                    Some(reg) => reg.merge_candidate(None, &op.stamp),
                    None => {
                        self.workspaces.insert(
                            workspace_sync_id.clone(),
                            LwwRegister::tombstone(op.stamp.clone()),
                        );
                    }
                }
            }
        }
    }

    /// Commutative, associative, idempotent state-based merge with `other`.
    pub fn merge(&mut self, other: &Self) {
        for (key, reg) in &other.annotations {
            match self.annotations.get_mut(key) {
                Some(local) => local.merge(reg),
                None => {
                    self.annotations.insert(key.clone(), reg.clone());
                }
            }
        }
        for (note_id, note) in &other.notes {
            match self.notes.get_mut(note_id) {
                Some(local) => {
                    local.meta.merge(&note.meta);
                    local.body.merge(&note.body);
                }
                None => {
                    self.notes.insert(note_id.clone(), note.clone());
                }
            }
        }
        for (bm_id, reg) in &other.bookmarks {
            match self.bookmarks.get_mut(bm_id) {
                Some(local) => local.merge(reg),
                None => {
                    self.bookmarks.insert(bm_id.clone(), reg.clone());
                }
            }
        }
        for (ws_id, reg) in &other.workspaces {
            match self.workspaces.get_mut(ws_id) {
                Some(local) => local.merge(reg),
                None => {
                    self.workspaces.insert(ws_id.clone(), reg.clone());
                }
            }
        }
    }

    /// Returns all active (non-tombstoned) annotations for `doc_id` keyed by `annot_id`.
    #[must_use]
    pub fn active_annotations(&self, doc_id: &str) -> BTreeMap<String, Value> {
        self.annotations
            .iter()
            .filter(|((d, _), _)| d == doc_id)
            .filter_map(|((_, aid), reg)| reg.value.as_ref().map(|v| (aid.clone(), v.clone())))
            .collect()
    }

    /// Returns active note metadata and rendered text body for `note_id`, if not tombstoned.
    #[must_use]
    pub fn active_note(&self, note_id: &str) -> Option<(&SyncedNoteMeta, String)> {
        let state = self.notes.get(note_id)?;
        let meta = state.meta.value.as_ref()?;
        Some((meta, state.body.render()))
    }

    /// Returns all active (non-tombstoned) note ids.
    #[must_use]
    pub fn active_note_ids(&self) -> Vec<String> {
        self.notes
            .iter()
            .filter_map(|(id, n)| n.meta.value.as_ref().map(|_| id.clone()))
            .collect()
    }

    /// Returns all active bookmarks.
    #[must_use]
    pub fn active_bookmarks(&self) -> Vec<&SyncedBookmark> {
        self.bookmarks
            .values()
            .filter_map(|reg| reg.value.as_ref())
            .collect()
    }

    /// Returns all active workspaces sorted by `(position, sync_id)`.
    #[must_use]
    pub fn active_workspaces(&self) -> Vec<&SyncedWorkspace> {
        let mut out: Vec<&SyncedWorkspace> = self
            .workspaces
            .values()
            .filter_map(|reg| reg.value.as_ref())
            .collect();
        out.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.sync_id.cmp(&b.sync_id))
        });
        out
    }
}

fn req_str(obj: &Value, key: &str) -> Result<String, SyncError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| SyncError::InvalidFormat(format!("missing string field {key}")))
}

fn workspace_to_json(ws: &SyncedWorkspace) -> Value {
    let panes: Vec<Value> = ws
        .panes
        .iter()
        .map(|p| {
            let open: Vec<Value> = p
                .open
                .iter()
                .map(|t| {
                    json!({
                        "path_token": t.path_token,
                        "view": t.view,
                        "pinned": t.pinned,
                        "group": t.group,
                    })
                })
                .collect();
            let groups: Vec<Value> = p
                .groups
                .iter()
                .map(|g| {
                    json!({
                        "name": g.name,
                        "color": g.color,
                        "collapsed": g.collapsed,
                    })
                })
                .collect();
            json!({
                "open": open,
                "active": p.active,
                "groups": groups,
            })
        })
        .collect();
    json!({
        "sync_id": ws.sync_id,
        "name": ws.name,
        "icon": ws.icon,
        "color": ws.color,
        "layout": ws.layout,
        "position": ws.position,
        "panes": panes,
    })
}

fn workspace_from_json(v: &Value) -> Result<SyncedWorkspace, SyncError> {
    let sync_id = req_str(v, "sync_id")?;
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut panes = Vec::new();
    if let Some(arr) = v.get("panes").and_then(Value::as_array) {
        for p in arr {
            let mut open = Vec::new();
            if let Some(tabs) = p.get("open").and_then(Value::as_array) {
                for t in tabs {
                    open.push(SyncedTab {
                        path_token: req_str(t, "path_token")?,
                        view: t
                            .get("view")
                            .and_then(Value::as_str)
                            .unwrap_or("{\"page\":0}")
                            .to_owned(),
                        pinned: t.get("pinned").and_then(Value::as_bool).unwrap_or(false),
                        group: t
                            .get("group")
                            .and_then(Value::as_u64)
                            .and_then(|g| usize::try_from(g).ok()),
                    });
                }
            }
            let mut groups = Vec::new();
            if let Some(gs) = p.get("groups").and_then(Value::as_array) {
                for g in gs {
                    groups.push(SyncedTabGroup {
                        name: g
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        color: g
                            .get("color")
                            .and_then(Value::as_str)
                            .unwrap_or("blue")
                            .to_owned(),
                        collapsed: g.get("collapsed").and_then(Value::as_bool).unwrap_or(false),
                    });
                }
            }
            panes.push(SyncedPane {
                open,
                active: p
                    .get("active")
                    .and_then(Value::as_u64)
                    .and_then(|a| usize::try_from(a).ok()),
                groups,
            });
        }
    }
    Ok(SyncedWorkspace {
        sync_id,
        name,
        icon: v.get("icon").and_then(Value::as_str).map(str::to_owned),
        color: v.get("color").and_then(Value::as_str).map(str::to_owned),
        layout: v.get("layout").and_then(Value::as_str).map(str::to_owned),
        position: v.get("position").and_then(Value::as_i64).unwrap_or(0),
        panes,
    })
}
