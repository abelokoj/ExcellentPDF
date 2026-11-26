//! Full-text FTS5 index, ranked snippet search, and saved searches (plan F.2–F.5).
//!
//! Backs `fts_entry` with two FTS5 virtual tables: `fts_index` (`unicode61 remove_diacritics 2` for token, prefix,
//! phrase, and boolean queries) and `fts_trigram` (`trigram` for substring and fuzzy candidate matching).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, params, types::Value as SqlValue};

use crate::Result;
use crate::workspace::file_id;

/// Kind of indexed content stored in the full-text index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    /// Extracted PDF/document page text (F.2, F.3).
    Page,
    /// Annotation comment or highlighted quote (F.3).
    Annotation,
    /// Recognized handwriting ink text (H.23, F.3).
    Handwriting,
    /// Audio/lecture transcript segment (L.2, F.3).
    Transcript,
    /// Markdown note body (F.12, F.3).
    Note,
}

impl SourceKind {
    /// Stable SQL text representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Annotation => "annotation",
            Self::Handwriting => "handwriting",
            Self::Transcript => "transcript",
            Self::Note => "note",
        }
    }

    /// Parses a [`SourceKind`] from its stored text form.
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "page" => Some(Self::Page),
            "annotation" => Some(Self::Annotation),
            "handwriting" => Some(Self::Handwriting),
            "transcript" => Some(Self::Transcript),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// One document unit to insert into the full-text index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtsEntryInput {
    /// Associated document path, if any (`None` for standalone notes).
    pub path: Option<PathBuf>,
    /// Kind of indexed content.
    pub source_kind: SourceKind,
    /// Stable identifier within its source (e.g. annotation `/NM`, note id, stroke id).
    pub source_id: String,
    /// Zero-based page index (`0` for non-paged sources).
    pub page: usize,
    /// Author name (for annotations/notes/documents).
    pub author: String,
    /// Color string such as `#ffff00` or `"yellow"`.
    pub color: String,
    /// Tags associated with the entry.
    pub tags: Vec<String>,
    /// Creation/modification timestamp in UTC seconds since 1970.
    pub created_at: i64,
    /// JSON-serialized quad coordinates so hits open at the exact spot.
    pub quads_json: String,
    /// Title or heading of the item.
    pub title: String,
    /// Main searchable text body.
    pub body: String,
}

/// One hit returned from an FTS5 or filtered library search.
#[derive(Debug, Clone, PartialEq)]
pub struct FtsHit {
    /// Backing `fts_entry` row id.
    pub id: i64,
    /// Associated document path, if any.
    pub path: Option<PathBuf>,
    /// Kind of matched source.
    pub source_kind: SourceKind,
    /// Identifier within its source.
    pub source_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Author name.
    pub author: String,
    /// Color string.
    pub color: String,
    /// Tags on the entry.
    pub tags: Vec<String>,
    /// Creation timestamp (UTC seconds since 1970).
    pub created_at: i64,
    /// JSON-serialized quads.
    pub quads_json: String,
    /// Entry title.
    pub title: String,
    /// Highlighted snippet or excerpt around the match.
    pub snippet: String,
    /// Full body text of the entry (for regex post-filtering or exact inspection).
    pub body: String,
    /// Relevance score (lower BM25 values are more relevant in SQLite FTS5; `0.0` for non-ranked filters).
    pub rank: f64,
}

/// Structured SQL filters paired with an FTS5 query (plan F.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    /// Restrict to specific source kinds (empty means all).
    pub source_kinds: Vec<SourceKind>,
    /// Restrict to entries whose author matches (case-insensitive substring).
    pub author: Option<String>,
    /// Restrict to entries whose color matches (case-insensitive exact or hex).
    pub color: Option<String>,
    /// Restrict to entries carrying all of these tags (case-insensitive).
    pub tags: Vec<String>,
    /// Minimum timestamp (inclusive, UTC seconds).
    pub after: Option<i64>,
    /// Maximum timestamp (inclusive, UTC seconds).
    pub before: Option<i64>,
    /// Minimum 0-based page index (inclusive).
    pub page_min: Option<usize>,
    /// Maximum 0-based page index (inclusive).
    pub page_max: Option<usize>,
    /// Restrict to files whose path starts with this prefix (case-insensitive).
    pub path_prefix: Option<String>,
    /// Restrict to a specific file path.
    pub file_path: Option<PathBuf>,
    /// Restrict to files belonging to a specific library collection id.
    pub collection_id: Option<i64>,
}

/// Recorded indexing state for incremental scans (F.2, F.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIndexState {
    /// File byte size when indexed.
    pub size: u64,
    /// File modification timestamp (seconds or nanoseconds, caller-consistent) when indexed.
    pub modified_at: i64,
    /// Indexing timestamp (UTC seconds since 1970).
    pub indexed_at: i64,
}

/// A named saved search / smart folder (plan F.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// Row id.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Query text (parsed by `search::query`).
    pub query: String,
    /// Sort position in the library tree.
    pub position: usize,
    /// Creation timestamp (UTC seconds since 1970).
    pub created_at: i64,
}

/// Replaces all FTS entries of `kind` for `path` with `entries` in a single transaction.
///
/// # Errors
///
/// SQLite failed.
pub fn replace_file_entries(
    tx: &Transaction<'_>,
    path: &Path,
    kind: SourceKind,
    entries: &[FtsEntryInput],
) -> Result<()> {
    let fid = file_id(tx, path)?;
    tx.execute(
        "DELETE FROM fts_entry WHERE file = ?1 AND source_kind = ?2",
        params![fid, kind.as_str()],
    )?;
    let mut stmt = tx.prepare(
        "INSERT INTO fts_entry (
            file, source_kind, source_id, page, author, color, tags, created_at, quads_json, title, body
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for entry in entries {
        let tags_str = entry.tags.join(", ");
        let page = i64::try_from(entry.page).unwrap_or(i64::MAX);
        stmt.execute(params![
            fid,
            kind.as_str(),
            entry.source_id,
            page,
            entry.author,
            entry.color,
            tags_str,
            entry.created_at,
            entry.quads_json,
            entry.title,
            entry.body,
        ])?;
    }
    Ok(())
}

/// Inserts a single FTS entry (for example a standalone note or transcript segment) and returns its row id.
///
/// # Errors
///
/// SQLite failed.
pub fn insert_entry(tx: &Transaction<'_>, entry: &FtsEntryInput) -> Result<i64> {
    let fid = match entry.path.as_deref() {
        Some(path) => Some(file_id(tx, path)?),
        None => None,
    };
    let tags_str = entry.tags.join(", ");
    let page = i64::try_from(entry.page).unwrap_or(i64::MAX);
    let id = tx.query_row(
        "INSERT INTO fts_entry (
            file, source_kind, source_id, page, author, color, tags, created_at, quads_json, title, body
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING id",
        params![
            fid,
            entry.source_kind.as_str(),
            entry.source_id,
            page,
            entry.author,
            entry.color,
            tags_str,
            entry.created_at,
            entry.quads_json,
            entry.title,
            entry.body,
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Deletes all FTS entries associated with `path`.
///
/// # Errors
///
/// SQLite failed.
pub fn delete_file_entries(tx: &Transaction<'_>, path: &Path) -> Result<()> {
    tx.execute(
        "DELETE FROM fts_entry WHERE file = (SELECT id FROM file WHERE path = ?1)",
        [path.to_string_lossy()],
    )?;
    tx.execute(
        "DELETE FROM fts_file_state WHERE file = (SELECT id FROM file WHERE path = ?1)",
        [path.to_string_lossy()],
    )?;
    Ok(())
}

/// Deletes all FTS entries matching `kind` and `source_id` (e.g. when deleting a note).
///
/// # Errors
///
/// SQLite failed.
pub fn delete_by_source(tx: &Transaction<'_>, kind: SourceKind, source_id: &str) -> Result<usize> {
    let deleted = tx.execute(
        "DELETE FROM fts_entry WHERE source_kind = ?1 AND source_id = ?2",
        params![kind.as_str(), source_id],
    )?;
    Ok(deleted)
}

/// Returns the last recorded indexing state for `path`, if any.
///
/// # Errors
///
/// SQLite failed.
pub fn file_index_state(conn: &Connection, path: &Path) -> Result<Option<FileIndexState>> {
    let row = conn
        .query_row(
            "SELECT s.size, s.modified_at, s.indexed_at
             FROM fts_file_state s JOIN file f ON f.id = s.file
             WHERE f.path = ?1",
            [path.to_string_lossy()],
            |row| {
                let size = u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0);
                Ok(FileIndexState {
                    size,
                    modified_at: row.get(1)?,
                    indexed_at: row.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Records the indexing state for `path`.
///
/// # Errors
///
/// SQLite failed.
pub fn set_file_index_state(
    tx: &Transaction<'_>,
    path: &Path,
    state: FileIndexState,
) -> Result<()> {
    let fid = file_id(tx, path)?;
    let size = i64::try_from(state.size).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO fts_file_state (file, size, modified_at, indexed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (file) DO UPDATE SET
            size = excluded.size,
            modified_at = excluded.modified_at,
            indexed_at = excluded.indexed_at",
        params![fid, size, state.modified_at, state.indexed_at],
    )?;
    Ok(())
}

/// Returns `true` when `path` is unindexed or its `size` / `modified_at` differ from what was last indexed.
///
/// # Errors
///
/// SQLite failed.
pub fn needs_reindex(conn: &Connection, path: &Path, size: u64, modified_at: i64) -> Result<bool> {
    Ok(match file_index_state(conn, path)? {
        Some(prev) => prev.size != size || prev.modified_at != modified_at,
        None => true,
    })
}

/// Runs a ranked full-text search against `fts_index` using an FTS5 `MATCH` expression and SQL [`SearchFilters`].
///
/// # Errors
///
/// SQLite failed or rejected `fts_match`.
pub fn search_fts(
    conn: &Connection,
    fts_match: &str,
    filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<FtsHit>> {
    let mut sql = String::from(
        "SELECT e.id, f.path, e.source_kind, e.source_id, e.page, e.author, e.color, e.tags,
                e.created_at, e.quads_json, e.title,
                snippet(fts_index, 1, '<b>', '</b>', '…', 16),
                e.body, bm25(fts_index) AS rank
         FROM fts_index
         JOIN fts_entry e ON e.id = fts_index.rowid
         LEFT JOIN file f ON f.id = e.file
         WHERE fts_index MATCH ?",
    );
    let mut args: Vec<SqlValue> = vec![SqlValue::Text(fts_match.to_owned())];
    append_filters(&mut sql, &mut args, filters);
    let cap = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
    sql.push_str(" ORDER BY rank ASC, e.id ASC LIMIT ?");
    args.push(SqlValue::Integer(cap));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), map_hit_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Runs a substring search via `fts_trigram` (falling back to `INSTR` for queries shorter than 3 characters).
///
/// # Errors
///
/// SQLite failed.
pub fn search_trigram(
    conn: &Connection,
    substring: &str,
    filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<FtsHit>> {
    let trimmed = substring.trim();
    if trimmed.is_empty() {
        return search_filtered(conn, filters, limit);
    }
    let char_len = trimmed.chars().count();
    let mut args: Vec<SqlValue> = Vec::new();
    let mut sql = if char_len >= 3 {
        let escaped = format!("\"{}\"", trimmed.replace('"', "\"\""));
        args.push(SqlValue::Text(escaped));
        String::from(
            "SELECT e.id, f.path, e.source_kind, e.source_id, e.page, e.author, e.color, e.tags,
                    e.created_at, e.quads_json, e.title,
                    snippet(fts_trigram, 1, '<b>', '</b>', '…', 16),
                    e.body, bm25(fts_trigram) AS rank
             FROM fts_trigram
             JOIN fts_entry e ON e.id = fts_trigram.rowid
             LEFT JOIN file f ON f.id = e.file
             WHERE fts_trigram MATCH ?",
        )
    } else {
        let needle = trimmed.to_lowercase();
        args.push(SqlValue::Text(needle.clone()));
        args.push(SqlValue::Text(needle));
        String::from(
            "SELECT e.id, f.path, e.source_kind, e.source_id, e.page, e.author, e.color, e.tags,
                    e.created_at, e.quads_json, e.title,
                    substr(e.body, 1, 160),
                    e.body, 0.0 AS rank
             FROM fts_entry e
             LEFT JOIN file f ON f.id = e.file
             WHERE (instr(lower(e.body), ?) > 0 OR instr(lower(e.title), ?) > 0)",
        )
    };
    append_filters(&mut sql, &mut args, filters);
    let cap = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
    sql.push_str(" ORDER BY rank ASC, e.id ASC LIMIT ?");
    args.push(SqlValue::Integer(cap));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), map_hit_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Returns entries matching [`SearchFilters`] without an FTS5 text clause (used for filter-only or regex queries).
///
/// # Errors
///
/// SQLite failed.
pub fn search_filtered(
    conn: &Connection,
    filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<FtsHit>> {
    let mut sql = String::from(
        "SELECT e.id, f.path, e.source_kind, e.source_id, e.page, e.author, e.color, e.tags,
                e.created_at, e.quads_json, e.title,
                substr(e.body, 1, 160),
                e.body, 0.0 AS rank
         FROM fts_entry e
         LEFT JOIN file f ON f.id = e.file
         WHERE 1 = 1",
    );
    let mut args: Vec<SqlValue> = Vec::new();
    append_filters(&mut sql, &mut args, filters);
    let cap = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
    sql.push_str(" ORDER BY e.created_at DESC, e.id ASC LIMIT ?");
    args.push(SqlValue::Integer(cap));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), map_hit_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Saves or updates a named search / smart folder (F.5) and returns its row id.
///
/// # Errors
///
/// SQLite failed.
pub fn save_search(tx: &Transaction<'_>, name: &str, query: &str, created_at: i64) -> Result<i64> {
    let id = tx.query_row(
        "INSERT INTO saved_search (name, query, position, created_at)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(position), -1) + 1 FROM saved_search), ?3)
         ON CONFLICT (name) DO UPDATE SET query = excluded.query
         RETURNING id",
        params![name, query, created_at],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Lists all saved searches / smart folders in display order.
///
/// # Errors
///
/// SQLite failed.
pub fn list_searches(conn: &Connection) -> Result<Vec<SavedSearch>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, query, position, created_at
         FROM saved_search ORDER BY position ASC, id ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SavedSearch {
                id: row.get(0)?,
                name: row.get(1)?,
                query: row.get(2)?,
                position: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                created_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Renames a saved search. Returns `true` if a row was updated.
///
/// # Errors
///
/// SQLite failed.
pub fn rename_search(tx: &Transaction<'_>, id: i64, name: &str) -> Result<bool> {
    let changed = tx.execute(
        "UPDATE saved_search SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(changed > 0)
}

/// Deletes a saved search by id. Returns `true` if a row was removed.
///
/// # Errors
///
/// SQLite failed.
pub fn delete_search(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let deleted = tx.execute("DELETE FROM saved_search WHERE id = ?1", [id])?;
    Ok(deleted > 0)
}

fn append_filters(sql: &mut String, args: &mut Vec<SqlValue>, filters: &SearchFilters) {
    if !filters.source_kinds.is_empty() {
        sql.push_str(" AND e.source_kind IN (");
        for (idx, kind) in filters.source_kinds.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            sql.push('?');
            args.push(SqlValue::Text(kind.as_str().to_owned()));
        }
        sql.push(')');
    }
    if let Some(author) = &filters.author {
        sql.push_str(" AND instr(lower(e.author), ?) > 0");
        args.push(SqlValue::Text(author.trim().to_lowercase()));
    }
    if let Some(color) = &filters.color {
        sql.push_str(" AND lower(e.color) = ?");
        args.push(SqlValue::Text(color.trim().to_lowercase()));
    }
    for tag in &filters.tags {
        // Match either an entry-level tag or a file-level library/annotation tag.
        sql.push_str(
            " AND (instr(lower(e.tags), ?) > 0
              OR EXISTS (SELECT 1 FROM library_file_tag lt WHERE lt.file = e.file AND lower(lt.tag) = ?)
              OR EXISTS (SELECT 1 FROM annotation_tag at WHERE at.file = e.file AND at.annotation = e.source_id AND lower(at.tag) = ?))",
        );
        let lowered = tag.trim().to_lowercase();
        args.push(SqlValue::Text(lowered.clone()));
        args.push(SqlValue::Text(lowered.clone()));
        args.push(SqlValue::Text(lowered));
    }
    if let Some(after) = filters.after {
        sql.push_str(" AND e.created_at >= ?");
        args.push(SqlValue::Integer(after));
    }
    if let Some(before) = filters.before {
        sql.push_str(" AND e.created_at <= ?");
        args.push(SqlValue::Integer(before));
    }
    if let Some(min_page) = filters.page_min {
        sql.push_str(" AND e.page >= ?");
        args.push(SqlValue::Integer(
            i64::try_from(min_page).unwrap_or(i64::MAX),
        ));
    }
    if let Some(max_page) = filters.page_max {
        sql.push_str(" AND e.page <= ?");
        args.push(SqlValue::Integer(
            i64::try_from(max_page).unwrap_or(i64::MAX),
        ));
    }
    if let Some(prefix) = &filters.path_prefix {
        sql.push_str(" AND f.path IS NOT NULL AND substr(lower(f.path), 1, ?) = ?");
        let lowered = prefix.to_lowercase();
        let len = i64::try_from(lowered.chars().count()).unwrap_or(i64::MAX);
        args.push(SqlValue::Integer(len));
        args.push(SqlValue::Text(lowered));
    }
    if let Some(path) = &filters.file_path {
        sql.push_str(" AND f.path = ?");
        args.push(SqlValue::Text(path.to_string_lossy().into_owned()));
    }
    if let Some(collection_id) = filters.collection_id {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM library_collection_item lci
              WHERE lci.collection = ? AND lci.file = e.file)",
        );
        args.push(SqlValue::Integer(collection_id));
    }
}

fn map_hit_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FtsHit> {
    let kind_str: String = row.get(2)?;
    let tags_str: String = row.get(7)?;
    let tags = tags_str
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(FtsHit {
        id: row.get(0)?,
        path: row.get::<_, Option<String>>(1)?.map(PathBuf::from),
        source_kind: SourceKind::from_name(&kind_str).unwrap_or(SourceKind::Page),
        source_id: row.get(3)?,
        page: usize::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        author: row.get(5)?,
        color: row.get(6)?,
        tags,
        created_at: row.get(8)?,
        quads_json: row.get(9)?,
        title: row.get(10)?,
        snippet: row.get(11)?,
        body: row.get(12)?,
        rank: row.get(13)?,
    })
}

/// Escapes a bare token for safe inclusion inside an FTS5 phrase/token query.
#[must_use]
pub fn escape_fts5_term(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('"');
    for ch in term.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            let _ = write!(out, "{ch}");
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("foreign keys");
        migrations::migrate(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn fts5_indexes_pages_annotations_and_folds_diacritics() {
        let mut conn = database();
        let tx = conn.transaction().expect("tx");
        replace_file_entries(
            &tx,
            Path::new("C:/papers/cafe.pdf"),
            SourceKind::Page,
            &[
                FtsEntryInput {
                    path: Some(PathBuf::from("C:/papers/cafe.pdf")),
                    source_kind: SourceKind::Page,
                    source_id: "p0".to_owned(),
                    page: 0,
                    author: "Ada".to_owned(),
                    color: String::new(),
                    tags: vec!["method".to_owned()],
                    created_at: 1000,
                    quads_json: "[[10,20,30,20,10,30,30,30]]".to_owned(),
                    title: "Café Study".to_owned(),
                    body: "Résumé of Bayesian inference on café datasets.".to_owned(),
                },
                FtsEntryInput {
                    path: Some(PathBuf::from("C:/papers/cafe.pdf")),
                    source_kind: SourceKind::Page,
                    source_id: "p1".to_owned(),
                    page: 1,
                    author: "Ada".to_owned(),
                    color: String::new(),
                    tags: Vec::new(),
                    created_at: 1000,
                    quads_json: "[]".to_owned(),
                    title: "Café Study".to_owned(),
                    body: "Unrelated appendix material.".to_owned(),
                },
            ],
        )
        .expect("replace");
        set_file_index_state(
            &tx,
            Path::new("C:/papers/cafe.pdf"),
            FileIndexState {
                size: 2048,
                modified_at: 55,
                indexed_at: 1000,
            },
        )
        .expect("state");
        tx.commit().expect("commit");

        assert!(
            !needs_reindex(&conn, Path::new("C:/papers/cafe.pdf"), 2048, 55).expect("check"),
            "unchanged file skips reindex"
        );
        assert!(
            needs_reindex(&conn, Path::new("C:/papers/cafe.pdf"), 2049, 55).expect("check"),
            "changed size triggers reindex"
        );

        let hits = search_fts(&conn, "cafe", &SearchFilters::default(), 10).expect("search");
        assert_eq!(
            hits.len(),
            2,
            "both pages mention café/cafe in title or body"
        );

        let body_hits =
            search_fts(&conn, "resume", &SearchFilters::default(), 10).expect("diacritic fold");
        assert_eq!(body_hits.len(), 1);
        assert_eq!(body_hits[0].page, 0);
        assert_eq!(body_hits[0].quads_json, "[[10,20,30,20,10,30,30,30]]");

        let sub_hits =
            search_trigram(&conn, "esian", &SearchFilters::default(), 10).expect("trigram");
        assert_eq!(sub_hits.len(), 1);
        assert!(sub_hits[0].body.contains("Bayesian"));

        let short_hits = search_trigram(&conn, "fé", &SearchFilters::default(), 10).expect("short");
        assert_eq!(short_hits.len(), 2);
    }

    #[test]
    fn saved_searches_crud_round_trips() {
        let mut conn = database();
        let tx = conn.transaction().expect("tx");
        let id =
            save_search(&tx, "Recent Bayesian", "bayesian after:2026-01-01", 500).expect("save");
        save_search(&tx, "Todos", "tag:todo", 501).expect("save 2");
        tx.commit().expect("commit");

        let list = list_searches(&conn).expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Recent Bayesian");

        let tx = conn.transaction().expect("tx");
        assert!(rename_search(&tx, id, "Bayesian Papers").expect("rename"));
        assert!(delete_search(&tx, id).expect("delete"));
        tx.commit().expect("commit");

        let after = list_searches(&conn).expect("list");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Todos");
    }
}
