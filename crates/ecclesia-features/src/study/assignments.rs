//! Assignments, checklists, due dates, and restart-safe reminders (plan S.14).
//!
//! Persists assignments attached to documents or page ranges in SQLite, tracks per-item
//! checklist completion, evaluates pending reminders relative to the current UTC timestamp
//! across restarts without duplicate firing, and surfaces calendar entries for NSH `Calendar`
//! and `notebook::planners` (N.11).

use std::fmt;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use ecclesia_core::anchor::new_id;
use rusqlite::{Connection, OptionalExtension, params};

/// Lifecycle status of an assignment (plan S.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AssignmentStatus {
    /// Not yet started.
    #[default]
    Pending,
    /// Work in progress.
    InProgress,
    /// All work completed.
    Completed,
}

impl AssignmentStatus {
    /// Database key string.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    /// Parses a stored status key.
    #[must_use]
    pub fn parse(key: &str) -> Self {
        match key {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

/// Whether a checklist item is checked off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CheckState {
    /// Still open.
    #[default]
    Open,
    /// Checked off.
    Done,
}

/// A single checklist item inside an [`Assignment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    /// Stable item identifier.
    pub id: String,
    /// Checklist item description.
    pub text: String,
    /// Completion state.
    pub state: CheckState,
}

/// An assignment or study deadline attached to a document or page range (plan S.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// Unique assignment ID.
    pub id: String,
    /// Assignment title.
    pub title: String,
    /// Optional path to the linked PDF or `.expdf` notebook.
    pub doc_path: Option<PathBuf>,
    /// Optional 1-based page range (`start..=end`).
    pub page_range: Option<RangeInclusive<usize>>,
    /// Due timestamp in UTC seconds since 1970.
    pub due_unix_secs: i64,
    /// How many seconds before `due_unix_secs` the reminder should fire (default `3600` = 1 hour).
    pub reminder_lead_secs: u64,
    /// Optional timestamp when reminder was already fired (prevents duplicate notifications).
    pub reminder_fired_at: Option<i64>,
    /// Completion status.
    pub status: AssignmentStatus,
    /// Sub-tasks checklist.
    pub checklist: Vec<ChecklistItem>,
}

/// A due reminder notification ready to emit via OS notifications (K.19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentReminder {
    /// Assignment identifier.
    pub assignment_id: String,
    /// Notification title.
    pub title: String,
    /// Due timestamp in UTC seconds.
    pub due_unix_secs: i64,
    /// Optional target document path.
    pub doc_path: Option<PathBuf>,
}

/// Errors produced by [`AssignmentStore`].
#[derive(Debug)]
#[non_exhaustive]
pub enum AssignmentError {
    /// SQLite database error.
    Sqlite(rusqlite::Error),
    /// Referenced assignment ID does not exist.
    NotFound(String),
}

impl fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "assignments database error: {err}"),
            Self::NotFound(id) => write!(f, "assignment not found: {id}"),
        }
    }
}

impl std::error::Error for AssignmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::NotFound(_) => None,
        }
    }
}

impl From<rusqlite::Error> for AssignmentError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

/// Convenience result alias for assignment operations.
pub type Result<T> = std::result::Result<T, AssignmentError>;

/// SQLite-backed store for assignments, checklists, and restart-safe due reminders (plan S.14).
pub struct AssignmentStore {
    conn: Connection,
}

impl fmt::Debug for AssignmentStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssignmentStore").finish_non_exhaustive()
    }
}

impl AssignmentStore {
    /// Opens an in-memory assignment store.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] if schema creation fails.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    /// Opens or creates a persistent assignment store at `db_path`.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] if opening or migrating the database fails.
    pub fn open(db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        Self::from_connection(conn)
    }

    /// Initializes the assignments schema on an existing SQLite `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] on DDL failure.
    pub fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS study_assignment (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                doc_path TEXT,
                page_start INTEGER,
                page_end INTEGER,
                due_unix_secs INTEGER NOT NULL,
                reminder_lead_secs INTEGER NOT NULL DEFAULT 3600,
                reminder_fired_at INTEGER,
                status TEXT NOT NULL DEFAULT 'pending'
            ) STRICT;
            CREATE INDEX IF NOT EXISTS study_assignment_due
                ON study_assignment (due_unix_secs);
            CREATE TABLE IF NOT EXISTS study_assignment_check (
                id TEXT PRIMARY KEY,
                assignment_id TEXT NOT NULL REFERENCES study_assignment (id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                text TEXT NOT NULL,
                done INTEGER NOT NULL DEFAULT 0
            ) STRICT;
            ",
        )?;
        Ok(Self { conn })
    }

    /// Inserts or replaces an assignment and its checklist items.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] on database failure.
    pub fn upsert(&mut self, assignment: &Assignment) -> Result<()> {
        let tx = self.conn.transaction()?;
        let doc_str = assignment
            .doc_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let (p_start, p_end) = match &assignment.page_range {
            Some(r) => (Some(*r.start() as i64), Some(*r.end() as i64)),
            None => (None, None),
        };
        tx.execute(
            "INSERT INTO study_assignment
                (id, title, doc_path, page_start, page_end, due_unix_secs, reminder_lead_secs, reminder_fired_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                doc_path = excluded.doc_path,
                page_start = excluded.page_start,
                page_end = excluded.page_end,
                due_unix_secs = excluded.due_unix_secs,
                reminder_lead_secs = excluded.reminder_lead_secs,
                reminder_fired_at = excluded.reminder_fired_at,
                status = excluded.status",
            params![
                assignment.id,
                assignment.title,
                doc_str,
                p_start,
                p_end,
                assignment.due_unix_secs,
                assignment.reminder_lead_secs as i64,
                assignment.reminder_fired_at,
                assignment.status.key(),
            ],
        )?;

        tx.execute(
            "DELETE FROM study_assignment_check WHERE assignment_id = ?1",
            params![assignment.id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO study_assignment_check (id, assignment_id, position, text, done)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (pos, item) in assignment.checklist.iter().enumerate() {
                let id = if item.id.is_empty() {
                    new_id()
                } else {
                    item.id.clone()
                };
                stmt.execute(params![
                    id,
                    assignment.id,
                    pos as i64,
                    item.text,
                    i64::from(item.state == CheckState::Done),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Loads one assignment by `id`.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::NotFound`] if absent, or [`AssignmentError::Sqlite`].
    pub fn get(&self, id: &str) -> Result<Assignment> {
        let row = self
            .conn
            .query_row(
                "SELECT id, title, doc_path, page_start, page_end, due_unix_secs,
                        reminder_lead_secs, reminder_fired_at, status
                 FROM study_assignment WHERE id = ?1",
                params![id],
                |r| {
                    let doc: Option<String> = r.get(2)?;
                    let ps: Option<i64> = r.get(3)?;
                    let pe: Option<i64> = r.get(4)?;
                    let lead: i64 = r.get(6)?;
                    let status_s: String = r.get(8)?;
                    Ok(Assignment {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        doc_path: doc.map(PathBuf::from),
                        page_range: match (ps, pe) {
                            (Some(a), Some(b)) => Some(
                                usize::try_from(a).unwrap_or(1)..=usize::try_from(b).unwrap_or(1),
                            ),
                            _ => None,
                        },
                        due_unix_secs: r.get(5)?,
                        reminder_lead_secs: u64::try_from(lead).unwrap_or(3600),
                        reminder_fired_at: r.get(7)?,
                        status: AssignmentStatus::parse(&status_s),
                        checklist: Vec::new(),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AssignmentError::NotFound(id.to_owned()))?;

        let mut assignment = row;
        let mut stmt = self.conn.prepare(
            "SELECT id, text, done FROM study_assignment_check
             WHERE assignment_id = ?1 ORDER BY position ASC",
        )?;
        let items = stmt.query_map(params![id], |r| {
            let done: i64 = r.get(2)?;
            Ok(ChecklistItem {
                id: r.get(0)?,
                text: r.get(1)?,
                state: if done != 0 {
                    CheckState::Done
                } else {
                    CheckState::Open
                },
            })
        })?;
        for item in items {
            assignment.checklist.push(item?);
        }
        Ok(assignment)
    }

    /// Lists all assignments sorted by `due_unix_secs` ascending.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] on database failure.
    pub fn list_all(&self) -> Result<Vec<Assignment>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM study_assignment ORDER BY due_unix_secs ASC")?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.get(&id)?);
        }
        Ok(out)
    }

    /// Toggles a checklist item's state and updates the parent assignment's status automatically.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError`] on database failure or missing ID.
    pub fn set_checklist_item_state(
        &mut self,
        assignment_id: &str,
        item_id: &str,
        state: CheckState,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE study_assignment_check SET done = ?1 WHERE assignment_id = ?2 AND id = ?3",
            params![i64::from(state == CheckState::Done), assignment_id, item_id],
        )?;
        if changed == 0 {
            return Err(AssignmentError::NotFound(format!(
                "{assignment_id}/{item_id}"
            )));
        }
        let mut assignment = self.get(assignment_id)?;
        if !assignment.checklist.is_empty() {
            let done_count = assignment
                .checklist
                .iter()
                .filter(|c| c.state == CheckState::Done)
                .count();
            assignment.status = if done_count == assignment.checklist.len() {
                AssignmentStatus::Completed
            } else if done_count > 0 {
                AssignmentStatus::InProgress
            } else {
                AssignmentStatus::Pending
            };
            self.conn.execute(
                "UPDATE study_assignment SET status = ?1 WHERE id = ?2",
                params![assignment.status.key(), assignment_id],
            )?;
        }
        Ok(())
    }

    /// Finds every uncompleted assignment whose reminder window
    /// (`now_unix_secs >= due_unix_secs - reminder_lead_secs`) has arrived and has not yet been
    /// marked fired, marks it fired atomically, and returns the reminders to emit (plan S.14
    /// **Done when**).
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] on database failure.
    pub fn poll_due_reminders(&mut self, now_unix_secs: i64) -> Result<Vec<AssignmentReminder>> {
        let tx = self.conn.transaction()?;
        let due_rows: Vec<AssignmentReminder> = {
            let mut stmt = tx.prepare(
                "SELECT id, title, due_unix_secs, doc_path
                 FROM study_assignment
                 WHERE status != 'completed'
                   AND reminder_fired_at IS NULL
                   AND (?1 >= due_unix_secs - reminder_lead_secs)
                 ORDER BY due_unix_secs ASC",
            )?;
            let mapped = stmt.query_map(params![now_unix_secs], |r| {
                let doc: Option<String> = r.get(3)?;
                Ok(AssignmentReminder {
                    assignment_id: r.get(0)?,
                    title: r.get(1)?,
                    due_unix_secs: r.get(2)?,
                    doc_path: doc.map(PathBuf::from),
                })
            })?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };

        for rem in &due_rows {
            tx.execute(
                "UPDATE study_assignment SET reminder_fired_at = ?1 WHERE id = ?2",
                params![now_unix_secs, rem.assignment_id],
            )?;
        }
        tx.commit()?;
        Ok(due_rows)
    }

    /// Lists assignments falling inside `[window_start_secs, window_end_secs]` for NSH
    /// `Calendar` or planner rendering.
    ///
    /// # Errors
    ///
    /// Returns [`AssignmentError::Sqlite`] on database failure.
    pub fn in_window(
        &self,
        window_start_secs: i64,
        window_end_secs: i64,
    ) -> Result<Vec<Assignment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM study_assignment
             WHERE due_unix_secs >= ?1 AND due_unix_secs <= ?2
             ORDER BY due_unix_secs ASC",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![window_start_secs, window_end_secs], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.get(&id)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn reminders_fire_at_exact_time_after_restart_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("ex-assignments-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        let db = dir.join("assignments.db");
        let _ = fs::remove_file(&db);

        let due = 1_700_010_000_i64;
        {
            let mut store = AssignmentStore::open(&db).expect("open");
            store
                .upsert(&Assignment {
                    id: "hw-1".into(),
                    title: "Read Chapters 3-4".into(),
                    doc_path: Some(PathBuf::from("textbook.pdf")),
                    page_range: Some(42..=75),
                    due_unix_secs: due,
                    reminder_lead_secs: 1800,
                    reminder_fired_at: None,
                    status: AssignmentStatus::Pending,
                    checklist: vec![
                        ChecklistItem {
                            id: "c1".into(),
                            text: "Chapter 3".into(),
                            state: CheckState::Open,
                        },
                        ChecklistItem {
                            id: "c2".into(),
                            text: "Chapter 4".into(),
                            state: CheckState::Open,
                        },
                    ],
                })
                .expect("upsert");
            // Before reminder window -> nothing fires.
            assert!(
                store
                    .poll_due_reminders(due - 2000)
                    .expect("poll")
                    .is_empty()
            );
        }

        // Reopen database (simulating app restart) and check inside reminder window.
        let mut reopened = AssignmentStore::open(&db).expect("reopen");
        let fired = reopened
            .poll_due_reminders(due - 900)
            .expect("poll after restart");
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].assignment_id, "hw-1");
        assert_eq!(fired[0].title, "Read Chapters 3-4");

        // Second poll does not fire the same reminder twice.
        assert!(
            reopened
                .poll_due_reminders(due - 100)
                .expect("poll 2")
                .is_empty()
        );

        reopened
            .set_checklist_item_state("hw-1", "c1", CheckState::Done)
            .expect("check c1");
        assert_eq!(
            reopened.get("hw-1").expect("get").status,
            AssignmentStatus::InProgress
        );

        drop(reopened);
        let _ = fs::remove_dir_all(&dir);
    }
}
