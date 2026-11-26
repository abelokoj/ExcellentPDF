//! Study persistence in SQLite (plan S.1–S.5, S.7, S.11, S.12): flashcards, decks, FSRS spaced-repetition state,
//! the append-only review log, reading-time sessions and daily study goals.
//!
//! Statistics and streaks (S.12) are computed live from `study_review_log` and `study_reading_log` rather than stored
//! twice. Call [`ensure_schema`] once on any connection or transaction before querying, or include [`SCHEMA_SQL`] in a
//! numbered migration.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Result;
use crate::workspace::file_id;

/// DDL creating every study table and index idempotently.
pub const SCHEMA_SQL: &str = r"
CREATE TABLE IF NOT EXISTS study_deck (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    license TEXT NOT NULL DEFAULT '',
    builtin INTEGER NOT NULL DEFAULT 0,
    installed_total INTEGER NOT NULL DEFAULT 0,
    missing_definitions_json TEXT NOT NULL DEFAULT '[]',
    created INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE IF NOT EXISTS study_card (
    id TEXT PRIMARY KEY,
    deck_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    front TEXT NOT NULL,
    back TEXT NOT NULL,
    cloze_ordinal INTEGER,
    occlusion_json TEXT,
    source_file INTEGER REFERENCES file (id) ON DELETE SET NULL,
    source_path TEXT,
    source_page INTEGER,
    anchor_json TEXT,
    media_json TEXT NOT NULL DEFAULT '[]',
    tags_json TEXT NOT NULL DEFAULT '[]',
    extra_json TEXT NOT NULL DEFAULT '{}',
    created INTEGER NOT NULL,
    modified INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS study_card_by_deck ON study_card (deck_id, created);

CREATE TABLE IF NOT EXISTS study_srs (
    card_id TEXT PRIMARY KEY REFERENCES study_card (id) ON DELETE CASCADE,
    state INTEGER NOT NULL DEFAULT 0,
    stability REAL NOT NULL DEFAULT 0.0,
    difficulty REAL NOT NULL DEFAULT 0.0,
    elapsed_days INTEGER NOT NULL DEFAULT 0,
    scheduled_days INTEGER NOT NULL DEFAULT 0,
    reps INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,
    due INTEGER NOT NULL DEFAULT 0,
    last_review INTEGER,
    learned INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX IF NOT EXISTS study_srs_by_due ON study_srs (due);

CREATE TABLE IF NOT EXISTS study_review_log (
    id INTEGER PRIMARY KEY,
    card_id TEXT NOT NULL REFERENCES study_card (id) ON DELETE CASCADE,
    rating INTEGER NOT NULL,
    state INTEGER NOT NULL,
    due INTEGER NOT NULL,
    stability REAL NOT NULL,
    difficulty REAL NOT NULL,
    elapsed_days INTEGER NOT NULL,
    scheduled_days INTEGER NOT NULL,
    reviewed_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX IF NOT EXISTS study_review_log_by_time ON study_review_log (reviewed_at);
CREATE INDEX IF NOT EXISTS study_review_log_by_card ON study_review_log (card_id, reviewed_at);

CREATE TABLE IF NOT EXISTS study_reading_log (
    id INTEGER PRIMARY KEY,
    file INTEGER REFERENCES file (id) ON DELETE SET NULL,
    pages_read INTEGER NOT NULL DEFAULT 0,
    duration_secs INTEGER NOT NULL DEFAULT 0,
    recorded_at INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS study_reading_log_by_time ON study_reading_log (recorded_at);

CREATE TABLE IF NOT EXISTS study_goals (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    daily_cards INTEGER NOT NULL DEFAULT 20,
    daily_pages INTEGER NOT NULL DEFAULT 10,
    daily_minutes INTEGER NOT NULL DEFAULT 30
) STRICT;
";

/// A persisted study or vocabulary deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckRecord {
    /// Stable deck identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Source URL or attribution note.
    pub source: String,
    /// Licence summary.
    pub license: String,
    /// Whether this deck is one of the built-in study word lists.
    pub builtin: bool,
    /// Total words accounted for when a built-in pack was installed.
    pub installed_total: u32,
    /// JSON array of words whose definitions were absent when installing a pack.
    pub missing_definitions_json: String,
    /// Creation timestamp (UTC seconds since 1970).
    pub created: i64,
}

/// A persisted flashcard, cloze card, image-occlusion card or vocabulary card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardRecord {
    /// Stable UUID.
    pub id: String,
    /// Owning deck identifier.
    pub deck_id: String,
    /// Discriminator: `"basic"`, `"cloze"`, `"image_occlusion"`, or `"vocab"`.
    pub kind: String,
    /// Front prompt or cloze template.
    pub front: String,
    /// Back answer or extra note.
    pub back: String,
    /// Cloze ordinal (`1`, `2`, …) for cloze cards.
    pub cloze_ordinal: Option<u32>,
    /// Serialized occlusion specification JSON for image-occlusion cards.
    pub occlusion_json: Option<String>,
    /// Source document path (if created from a document highlight, note or lasso).
    pub source_path: Option<String>,
    /// Zero-based source page index.
    pub source_page: Option<usize>,
    /// Serialized K.28 text-quote anchor + bounding rectangle JSON.
    pub anchor_json: Option<String>,
    /// Serialized media attachment descriptors JSON.
    pub media_json: String,
    /// Serialized tags array JSON.
    pub tags_json: String,
    /// Extra feature-specific metadata JSON (e.g. vocabulary dictionary id, context, deck ids).
    pub extra_json: String,
    /// Creation timestamp (UTC seconds since 1970).
    pub created: i64,
    /// Last modification timestamp (UTC seconds since 1970).
    pub modified: i64,
}

/// Persisted FSRS-5 scheduling state for a card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SrsRecord {
    /// Scheduling state code: 0 New, 1 Learning, 2 Review, 3 Relearning.
    pub state: u8,
    /// Memory stability in days.
    pub stability: f64,
    /// Item difficulty in `[1.0, 10.0]` (0.0 for unreviewed new cards).
    pub difficulty: f64,
    /// Days elapsed between the previous review and the latest review.
    pub elapsed_days: u32,
    /// Scheduled interval in days until the next review.
    pub scheduled_days: u32,
    /// Total completed repetitions.
    pub reps: u32,
    /// Total lapses (`Again` grades on reviewed cards).
    pub lapses: u32,
    /// Due timestamp (UTC seconds since 1970).
    pub due: i64,
    /// Timestamp of the last review, if any.
    pub last_review: Option<i64>,
    /// Whether the user or scheduler marked the item learned.
    pub learned: bool,
}

impl Default for SrsRecord {
    fn default() -> Self {
        Self {
            state: 0,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: 0,
            due: 0,
            last_review: None,
            learned: false,
        }
    }
}

/// One immutable review log row (source of truth for FSRS optimization and S.12 statistics).
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewLogRecord {
    /// Row identifier (`0` before insertion).
    pub id: i64,
    /// Reviewed card id.
    pub card_id: String,
    /// Grade given: 1 Again, 2 Hard, 3 Good, 4 Easy.
    pub rating: u8,
    /// Card state at the time of review.
    pub state: u8,
    /// Due timestamp computed by the review.
    pub due: i64,
    /// Stability after the review.
    pub stability: f64,
    /// Difficulty after the review.
    pub difficulty: f64,
    /// Elapsed days since the previous review.
    pub elapsed_days: u32,
    /// Scheduled interval in days.
    pub scheduled_days: u32,
    /// Timestamp when the review occurred (UTC seconds since 1970).
    pub reviewed_at: i64,
    /// Time spent answering the card, in milliseconds.
    pub duration_ms: u64,
}

/// One reading-time log entry (source of truth for S.12 pages/minutes goals and streaks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingLogRecord {
    /// Row identifier (`0` before insertion).
    pub id: i64,
    /// Document path, if associated with a file.
    pub path: Option<String>,
    /// Number of pages read in this slice.
    pub pages_read: u32,
    /// Active reading duration in seconds.
    pub duration_secs: u64,
    /// Timestamp when this slice was recorded (UTC seconds since 1970).
    pub recorded_at: i64,
}

/// Daily study goal targets (plan S.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudyGoalsRecord {
    /// Target flashcards reviewed per day.
    pub daily_cards: u32,
    /// Target pages read per day.
    pub daily_pages: u32,
    /// Target study/reading minutes per day.
    pub daily_minutes: u32,
}

impl Default for StudyGoalsRecord {
    fn default() -> Self {
        Self {
            daily_cards: 20,
            daily_pages: 10,
            daily_minutes: 30,
        }
    }
}

/// Creates the study tables and indices if they do not exist yet.
///
/// # Errors
///
/// SQLite failed to execute the schema DDL.
pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// Inserts or updates a study deck.
///
/// # Errors
///
/// SQLite failed.
pub fn upsert_deck(tx: &Transaction<'_>, deck: &DeckRecord) -> Result<()> {
    tx.execute(
        "INSERT INTO study_deck (id, name, description, source, license, builtin, installed_total, missing_definitions_json, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            source = excluded.source,
            license = excluded.license,
            builtin = excluded.builtin,
            installed_total = excluded.installed_total,
            missing_definitions_json = excluded.missing_definitions_json",
        params![
            deck.id,
            deck.name,
            deck.description,
            deck.source,
            deck.license,
            i64::from(deck.builtin),
            i64::from(deck.installed_total),
            deck.missing_definitions_json,
            deck.created,
        ],
    )?;
    Ok(())
}

/// Lists all decks ordered by creation timestamp then name.
///
/// # Errors
///
/// SQLite failed.
pub fn list_decks(conn: &Connection) -> Result<Vec<DeckRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, source, license, builtin, installed_total, missing_definitions_json, created
         FROM study_deck ORDER BY created, name, id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DeckRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                source: row.get(3)?,
                license: row.get(4)?,
                builtin: row.get::<_, i64>(5)? != 0,
                installed_total: row.get::<_, u32>(6)?,
                missing_definitions_json: row.get(7)?,
                created: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes a deck by id.
///
/// # Errors
///
/// SQLite failed.
pub fn delete_deck(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let affected = tx.execute("DELETE FROM study_deck WHERE id = ?1", [id])?;
    Ok(affected > 0)
}

/// Inserts or updates a study card and ensures it has an entry in `study_srs`.
///
/// # Errors
///
/// SQLite failed.
pub fn upsert_card(tx: &Transaction<'_>, card: &CardRecord) -> Result<()> {
    let source_file = match card.source_path.as_deref() {
        Some(path) if !path.is_empty() => Some(file_id(tx, Path::new(path))?),
        _ => None,
    };
    tx.execute(
        "INSERT INTO study_card (
            id, deck_id, kind, front, back, cloze_ordinal, occlusion_json,
            source_file, source_path, source_page, anchor_json, media_json, tags_json, extra_json,
            created, modified
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT (id) DO UPDATE SET
            deck_id = excluded.deck_id,
            kind = excluded.kind,
            front = excluded.front,
            back = excluded.back,
            cloze_ordinal = excluded.cloze_ordinal,
            occlusion_json = excluded.occlusion_json,
            source_file = excluded.source_file,
            source_path = excluded.source_path,
            source_page = excluded.source_page,
            anchor_json = excluded.anchor_json,
            media_json = excluded.media_json,
            tags_json = excluded.tags_json,
            extra_json = excluded.extra_json,
            modified = excluded.modified",
        params![
            card.id,
            card.deck_id,
            card.kind,
            card.front,
            card.back,
            card.cloze_ordinal,
            card.occlusion_json,
            source_file,
            card.source_path,
            card.source_page,
            card.anchor_json,
            card.media_json,
            card.tags_json,
            card.extra_json,
            card.created,
            card.modified,
        ],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO study_srs (card_id, due) VALUES (?1, ?2)",
        params![card.id, card.created],
    )?;
    Ok(())
}

/// Loads a single card by id.
///
/// # Errors
///
/// SQLite failed.
pub fn get_card(conn: &Connection, id: &str) -> Result<Option<CardRecord>> {
    let row = conn
        .query_row(
            "SELECT id, deck_id, kind, front, back, cloze_ordinal, occlusion_json,
                    source_path, source_page, anchor_json, media_json, tags_json, extra_json,
                    created, modified
             FROM study_card WHERE id = ?1",
            [id],
            map_card_row,
        )
        .optional()?;
    Ok(row)
}

/// Lists cards, optionally filtered by `deck_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn list_cards(conn: &Connection, deck_id: Option<&str>) -> Result<Vec<CardRecord>> {
    match deck_id {
        Some(deck) => {
            let mut stmt = conn.prepare(
                "SELECT id, deck_id, kind, front, back, cloze_ordinal, occlusion_json,
                        source_path, source_page, anchor_json, media_json, tags_json, extra_json,
                        created, modified
                 FROM study_card WHERE deck_id = ?1 ORDER BY created, id",
            )?;
            let rows = stmt
                .query_map([deck], map_card_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, deck_id, kind, front, back, cloze_ordinal, occlusion_json,
                        source_path, source_page, anchor_json, media_json, tags_json, extra_json,
                        created, modified
                 FROM study_card ORDER BY created, id",
            )?;
            let rows = stmt
                .query_map([], map_card_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }
}

/// Deletes a card (cascading to its SRS state and review log).
///
/// # Errors
///
/// SQLite failed.
pub fn delete_card(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let affected = tx.execute("DELETE FROM study_card WHERE id = ?1", [id])?;
    Ok(affected > 0)
}

/// Saves the FSRS scheduling state for `card_id`.
///
/// # Errors
///
/// SQLite failed (including foreign-key failure if `card_id` does not exist).
pub fn save_srs(tx: &Transaction<'_>, card_id: &str, srs: &SrsRecord) -> Result<()> {
    tx.execute(
        "INSERT INTO study_srs (
            card_id, state, stability, difficulty, elapsed_days, scheduled_days,
            reps, lapses, due, last_review, learned
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (card_id) DO UPDATE SET
            state = excluded.state,
            stability = excluded.stability,
            difficulty = excluded.difficulty,
            elapsed_days = excluded.elapsed_days,
            scheduled_days = excluded.scheduled_days,
            reps = excluded.reps,
            lapses = excluded.lapses,
            due = excluded.due,
            last_review = excluded.last_review,
            learned = excluded.learned",
        params![
            card_id,
            i64::from(srs.state),
            srs.stability,
            srs.difficulty,
            i64::from(srs.elapsed_days),
            i64::from(srs.scheduled_days),
            i64::from(srs.reps),
            i64::from(srs.lapses),
            srs.due,
            srs.last_review,
            i64::from(srs.learned),
        ],
    )?;
    Ok(())
}

/// Loads the FSRS scheduling state for `card_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn load_srs(conn: &Connection, card_id: &str) -> Result<Option<SrsRecord>> {
    let row = conn
        .query_row(
            "SELECT state, stability, difficulty, elapsed_days, scheduled_days,
                    reps, lapses, due, last_review, learned
             FROM study_srs WHERE card_id = ?1",
            [card_id],
            map_srs_row,
        )
        .optional()?;
    Ok(row)
}

/// Returns cards due at or before `now` (UTC seconds), ordered oldest-due first, optionally restricted to `deck_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn due_cards(
    conn: &Connection,
    deck_id: Option<&str>,
    now: i64,
    limit: Option<usize>,
) -> Result<Vec<(CardRecord, SrsRecord)>> {
    let cap = i64::try_from(limit.unwrap_or(usize::MAX)).unwrap_or(i64::MAX);
    match deck_id {
        Some(deck) => {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.deck_id, c.kind, c.front, c.back, c.cloze_ordinal, c.occlusion_json,
                        c.source_path, c.source_page, c.anchor_json, c.media_json, c.tags_json, c.extra_json,
                        c.created, c.modified,
                        s.state, s.stability, s.difficulty, s.elapsed_days, s.scheduled_days,
                        s.reps, s.lapses, s.due, s.last_review, s.learned
                 FROM study_card c JOIN study_srs s ON s.card_id = c.id
                 WHERE c.deck_id = ?1 AND s.due <= ?2
                 ORDER BY s.due ASC, c.created ASC, c.id ASC
                 LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(params![deck, now, cap], map_card_with_srs)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.deck_id, c.kind, c.front, c.back, c.cloze_ordinal, c.occlusion_json,
                        c.source_path, c.source_page, c.anchor_json, c.media_json, c.tags_json, c.extra_json,
                        c.created, c.modified,
                        s.state, s.stability, s.difficulty, s.elapsed_days, s.scheduled_days,
                        s.reps, s.lapses, s.due, s.last_review, s.learned
                 FROM study_card c JOIN study_srs s ON s.card_id = c.id
                 WHERE s.due <= ?1
                 ORDER BY s.due ASC, c.created ASC, c.id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![now, cap], map_card_with_srs)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }
}

/// Counts cards due at or before `now` (UTC seconds), optionally restricted to `deck_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn due_count(conn: &Connection, deck_id: Option<&str>, now: i64) -> Result<usize> {
    let count: usize = match deck_id {
        Some(deck) => conn.query_row(
            "SELECT COUNT(*) FROM study_card c JOIN study_srs s ON s.card_id = c.id
             WHERE c.deck_id = ?1 AND s.due <= ?2",
            params![deck, now],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM study_srs WHERE due <= ?1",
            [now],
            |row| row.get(0),
        )?,
    };
    Ok(count)
}

/// Appends one review event to `study_review_log` and returns its row id.
///
/// # Errors
///
/// SQLite failed.
pub fn append_review_log(tx: &Transaction<'_>, entry: &ReviewLogRecord) -> Result<i64> {
    let duration = i64::try_from(entry.duration_ms).unwrap_or(i64::MAX);
    let id = tx.query_row(
        "INSERT INTO study_review_log (
            card_id, rating, state, due, stability, difficulty,
            elapsed_days, scheduled_days, reviewed_at, duration_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
        params![
            entry.card_id,
            i64::from(entry.rating),
            i64::from(entry.state),
            entry.due,
            entry.stability,
            entry.difficulty,
            i64::from(entry.elapsed_days),
            i64::from(entry.scheduled_days),
            entry.reviewed_at,
            duration,
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Lists review log entries in chronological order, optionally bounded by `[since, until]`.
///
/// # Errors
///
/// SQLite failed.
pub fn list_review_log(
    conn: &Connection,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<Vec<ReviewLogRecord>> {
    let from = since.unwrap_or(i64::MIN);
    let to = until.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT id, card_id, rating, state, due, stability, difficulty,
                elapsed_days, scheduled_days, reviewed_at, duration_ms
         FROM study_review_log
         WHERE reviewed_at >= ?1 AND reviewed_at <= ?2
         ORDER BY reviewed_at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![from, to], |row| {
            Ok(ReviewLogRecord {
                id: row.get(0)?,
                card_id: row.get(1)?,
                rating: u8::try_from(row.get::<_, i64>(2)?).unwrap_or(1),
                state: u8::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                due: row.get(4)?,
                stability: row.get(5)?,
                difficulty: row.get(6)?,
                elapsed_days: row.get::<_, u32>(7)?,
                scheduled_days: row.get::<_, u32>(8)?,
                reviewed_at: row.get(9)?,
                duration_ms: u64::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Appends one reading-time entry to `study_reading_log` and returns its row id.
///
/// # Errors
///
/// SQLite failed.
pub fn append_reading_log(tx: &Transaction<'_>, entry: &ReadingLogRecord) -> Result<i64> {
    let file = match entry.path.as_deref() {
        Some(path) if !path.is_empty() => Some(file_id(tx, Path::new(path))?),
        _ => None,
    };
    let duration = i64::try_from(entry.duration_secs).unwrap_or(i64::MAX);
    let id = tx.query_row(
        "INSERT INTO study_reading_log (file, pages_read, duration_secs, recorded_at)
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
        params![
            file,
            i64::from(entry.pages_read),
            duration,
            entry.recorded_at
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Lists reading-time entries in chronological order, optionally bounded by `[since, until]`.
///
/// # Errors
///
/// SQLite failed.
pub fn list_reading_log(
    conn: &Connection,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<Vec<ReadingLogRecord>> {
    let from = since.unwrap_or(i64::MIN);
    let to = until.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT r.id, f.path, r.pages_read, r.duration_secs, r.recorded_at
         FROM study_reading_log r LEFT JOIN file f ON f.id = r.file
         WHERE r.recorded_at >= ?1 AND r.recorded_at <= ?2
         ORDER BY r.recorded_at ASC, r.id ASC",
    )?;
    let rows = stmt
        .query_map(params![from, to], |row| {
            Ok(ReadingLogRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                pages_read: row.get::<_, u32>(2)?,
                duration_secs: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                recorded_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Saves the user's daily study goals.
///
/// # Errors
///
/// SQLite failed.
pub fn save_goals(tx: &Transaction<'_>, goals: StudyGoalsRecord) -> Result<()> {
    tx.execute(
        "INSERT INTO study_goals (id, daily_cards, daily_pages, daily_minutes)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET
            daily_cards = excluded.daily_cards,
            daily_pages = excluded.daily_pages,
            daily_minutes = excluded.daily_minutes",
        params![
            i64::from(goals.daily_cards),
            i64::from(goals.daily_pages),
            i64::from(goals.daily_minutes),
        ],
    )?;
    Ok(())
}

/// Loads the user's daily study goals, returning defaults if none have been saved yet.
///
/// # Errors
///
/// SQLite failed.
pub fn load_goals(conn: &Connection) -> Result<StudyGoalsRecord> {
    let found = conn
        .query_row(
            "SELECT daily_cards, daily_pages, daily_minutes FROM study_goals WHERE id = 1",
            [],
            |row| {
                Ok(StudyGoalsRecord {
                    daily_cards: row.get::<_, u32>(0)?,
                    daily_pages: row.get::<_, u32>(1)?,
                    daily_minutes: row.get::<_, u32>(2)?,
                })
            },
        )
        .optional()?;
    Ok(found.unwrap_or_default())
}

fn map_card_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CardRecord> {
    Ok(CardRecord {
        id: row.get(0)?,
        deck_id: row.get(1)?,
        kind: row.get(2)?,
        front: row.get(3)?,
        back: row.get(4)?,
        cloze_ordinal: row.get(5)?,
        occlusion_json: row.get(6)?,
        source_path: row.get(7)?,
        source_page: row.get(8)?,
        anchor_json: row.get(9)?,
        media_json: row.get(10)?,
        tags_json: row.get(11)?,
        extra_json: row.get(12)?,
        created: row.get(13)?,
        modified: row.get(14)?,
    })
}

fn map_srs_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SrsRecord> {
    Ok(SrsRecord {
        state: u8::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
        stability: row.get(1)?,
        difficulty: row.get(2)?,
        elapsed_days: row.get::<_, u32>(3)?,
        scheduled_days: row.get::<_, u32>(4)?,
        reps: row.get::<_, u32>(5)?,
        lapses: row.get::<_, u32>(6)?,
        due: row.get(7)?,
        last_review: row.get(8)?,
        learned: row.get::<_, i64>(9)? != 0,
    })
}

fn map_card_with_srs(row: &rusqlite::Row<'_>) -> rusqlite::Result<(CardRecord, SrsRecord)> {
    let card = map_card_row(row)?;
    let srs = SrsRecord {
        state: u8::try_from(row.get::<_, i64>(15)?).unwrap_or(0),
        stability: row.get(16)?,
        difficulty: row.get(17)?,
        elapsed_days: row.get::<_, u32>(18)?,
        scheduled_days: row.get::<_, u32>(19)?,
        reps: row.get::<_, u32>(20)?,
        lapses: row.get::<_, u32>(21)?,
        due: row.get(22)?,
        last_review: row.get(23)?,
        learned: row.get::<_, i64>(24)? != 0,
    };
    Ok((card, srs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("enable foreign keys");
        migrations::migrate(&mut conn).expect("migrate base schema");
        ensure_schema(&conn).expect("ensure study schema");
        conn
    }

    fn change<T>(conn: &mut Connection, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> T {
        let tx = conn.transaction().expect("begin tx");
        let out = f(&tx).expect("run tx");
        tx.commit().expect("commit tx");
        out
    }

    #[test]
    fn schema_is_idempotent_and_round_trips_decks_cards_srs_and_logs() {
        let mut conn = database();
        ensure_schema(&conn).expect("second ensure_schema is idempotent");

        let deck = DeckRecord {
            id: "deck-1".to_owned(),
            name: "Algorithms".to_owned(),
            description: "CLRS notes".to_owned(),
            source: String::new(),
            license: String::new(),
            builtin: false,
            installed_total: 0,
            missing_definitions_json: "[]".to_owned(),
            created: 1_700_000_000,
        };
        change(&mut conn, |tx| upsert_deck(tx, &deck));
        assert_eq!(list_decks(&conn).expect("list decks"), vec![deck]);

        let card = CardRecord {
            id: "card-1".to_owned(),
            deck_id: "deck-1".to_owned(),
            kind: "basic".to_owned(),
            front: "What is amortized O(1)?".to_owned(),
            back: "Average time per op over worst-case sequence.".to_owned(),
            cloze_ordinal: None,
            occlusion_json: None,
            source_path: Some("C:/papers/clrs.pdf".to_owned()),
            source_page: Some(42),
            anchor_json: Some(r#"{"exact":"amortized"}"#.to_owned()),
            media_json: "[]".to_owned(),
            tags_json: r#"["cs","algorithms"]"#.to_owned(),
            extra_json: "{}".to_owned(),
            created: 1_700_000_100,
            modified: 1_700_000_100,
        };
        change(&mut conn, |tx| upsert_card(tx, &card));
        assert_eq!(get_card(&conn, "card-1").expect("get"), Some(card.clone()));
        assert_eq!(due_count(&conn, None, 1_700_000_100).expect("due"), 1);

        let updated_srs = SrsRecord {
            state: 2,
            stability: 4.5,
            difficulty: 5.1,
            elapsed_days: 0,
            scheduled_days: 5,
            reps: 1,
            lapses: 0,
            due: 1_700_432_100,
            last_review: Some(1_700_000_100),
            learned: false,
        };
        change(&mut conn, |tx| save_srs(tx, "card-1", &updated_srs));
        assert_eq!(
            load_srs(&conn, "card-1").expect("load srs"),
            Some(updated_srs)
        );
        assert_eq!(
            due_count(&conn, Some("deck-1"), 1_700_000_200).expect("due"),
            0
        );
        assert_eq!(
            due_cards(&conn, Some("deck-1"), 1_700_500_000, Some(10))
                .expect("due cards")
                .len(),
            1
        );

        let log = ReviewLogRecord {
            id: 0,
            card_id: "card-1".to_owned(),
            rating: 3,
            state: 2,
            due: 1_700_432_100,
            stability: 4.5,
            difficulty: 5.1,
            elapsed_days: 0,
            scheduled_days: 5,
            reviewed_at: 1_700_000_100,
            duration_ms: 2_300,
        };
        let log_id = change(&mut conn, |tx| append_review_log(tx, &log));
        assert!(log_id > 0);
        let logs = list_review_log(&conn, None, None).expect("list logs");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].duration_ms, 2_300);

        let reading = ReadingLogRecord {
            id: 0,
            path: Some("C:/papers/clrs.pdf".to_owned()),
            pages_read: 6,
            duration_secs: 900,
            recorded_at: 1_700_000_500,
        };
        change(&mut conn, |tx| append_reading_log(tx, &reading));
        let readings = list_reading_log(&conn, None, None).expect("reading log");
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].pages_read, 6);

        let goals = StudyGoalsRecord {
            daily_cards: 35,
            daily_pages: 15,
            daily_minutes: 45,
        };
        change(&mut conn, |tx| save_goals(tx, goals));
        assert_eq!(load_goals(&conn).expect("goals"), goals);

        assert!(change(&mut conn, |tx| delete_card(tx, "card-1")));
        assert_eq!(get_card(&conn, "card-1").expect("deleted"), None);
        assert!(
            list_review_log(&conn, None, None)
                .expect("cascaded")
                .is_empty()
        );
        assert!(change(&mut conn, |tx| delete_deck(tx, "deck-1")));
    }
}
