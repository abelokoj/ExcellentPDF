//! Library persistence in SQLite (plan F.6–F.9, W.10): collections, tags, read status, metadata,
//! duplicate-detection hashes, and watched folders.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, params, types::Value as SqlValue};

use crate::Result;
use crate::workspace::file_id;

/// Reading progress status for a library document (plan F.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReadStatus {
    /// Not started yet.
    #[default]
    Unread,
    /// Currently in progress.
    Reading,
    /// Finished reading.
    Finished,
}

impl ReadStatus {
    /// Stable SQL / serialization string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Reading => "reading",
            Self::Finished => "finished",
        }
    }

    /// Parses a [`ReadStatus`] from text (`"done"` is accepted as an alias for `"finished"`).
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "unread" => Some(Self::Unread),
            "reading" => Some(Self::Reading),
            "finished" | "done" => Some(Self::Finished),
            _ => None,
        }
    }
}

/// Persisted metadata and status for a document in the user's library.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItem {
    /// File path on disk.
    pub path: PathBuf,
    /// Document title (falls back to filename when empty).
    pub title: String,
    /// Author string (e.g. `"Ada Lovelace; Grace Hopper"`).
    pub authors: String,
    /// Publication year, if known.
    pub year: Option<i32>,
    /// Extracted or fetched DOI, if any.
    pub doi: String,
    /// Extracted or fetched arXiv identifier, if any.
    pub arxiv_id: String,
    /// Extracted or fetched ISBN, if any.
    pub isbn: String,
    /// Current read status.
    pub read_status: ReadStatus,
    /// Reading progress in `[0.0, 1.0]`.
    pub progress: f64,
    /// Total page count.
    pub page_count: usize,
    /// Lowercase SHA-256 hex digest of the file bytes for exact duplicate detection (F.8).
    pub content_hash: String,
    /// MinHash signature over first-page text shingles for near-duplicate detection (F.8).
    pub minhash: Vec<u64>,
    /// File size in bytes.
    pub file_size: u64,
    /// File modification timestamp (UTC seconds since 1970).
    pub modified_at: i64,
    /// Timestamp when added to the library (UTC seconds since 1970).
    pub added_at: i64,
    /// Timestamp when last opened (UTC seconds since 1970).
    pub last_opened: i64,
    /// Extra JSON metadata (publisher, journal, venue, etc.).
    pub meta_json: String,
    /// Library tags attached to this file.
    pub tags: Vec<String>,
}

impl Default for LibraryItem {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            title: String::new(),
            authors: String::new(),
            year: None,
            doi: String::new(),
            arxiv_id: String::new(),
            isbn: String::new(),
            read_status: ReadStatus::Unread,
            progress: 0.0,
            page_count: 0,
            content_hash: String::new(),
            minhash: Vec::new(),
            file_size: 0,
            modified_at: 0,
            added_at: 0,
            last_opened: 0,
            meta_json: "{}".to_owned(),
            tags: Vec::new(),
        }
    }
}

/// Filter criteria for listing library items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryFilter {
    /// Restrict to items in this collection id.
    pub collection_id: Option<i64>,
    /// Restrict to items carrying this tag (case-insensitive).
    pub tag: Option<String>,
    /// Restrict to items with this read status.
    pub read_status: Option<ReadStatus>,
    /// Substring filter across path, title, authors, DOI, arXiv, ISBN.
    pub query: Option<String>,
    /// Maximum items to return (`None` returns all).
    pub limit: Option<usize>,
    /// Pagination offset.
    pub offset: usize,
}

/// A nested collection of library documents (plan F.6, W.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    /// Row id.
    pub id: i64,
    /// Optional parent collection id for nesting.
    pub parent: Option<i64>,
    /// Display name.
    pub name: String,
    /// Optional icon string.
    pub icon: Option<String>,
    /// Optional color name or hex.
    pub color: Option<String>,
    /// Sort position within the parent.
    pub position: usize,
    /// Number of items directly in this collection.
    pub item_count: usize,
}

/// Whether a watched folder is scanned shallowly or recursively (plan F.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum WatchDepth {
    /// Scan only the folder's immediate entries.
    Shallow,
    /// Scan subdirectories recursively.
    #[default]
    Recursive,
}

impl WatchDepth {
    /// Returns `true` when subdirectories are scanned.
    #[must_use]
    pub const fn is_recursive(self) -> bool {
        matches!(self, Self::Recursive)
    }

    /// Converts a persisted SQLite flag into a [`WatchDepth`].
    #[must_use]
    pub const fn from_flag(recursive: bool) -> Self {
        if recursive {
            Self::Recursive
        } else {
            Self::Shallow
        }
    }
}

/// A folder watched for incremental reconciliation (plan F.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedFolder {
    /// Row id.
    pub id: i64,
    /// Directory path.
    pub path: PathBuf,
    /// Whether subdirectories are scanned recursively.
    pub depth: WatchDepth,
    /// Last completed scan timestamp (UTC seconds since 1970).
    pub last_scan: i64,
}

/// Inserts or updates a [`LibraryItem`] and its tags.
///
/// # Errors
///
/// SQLite failed.
pub fn upsert_item(tx: &Transaction<'_>, item: &LibraryItem) -> Result<()> {
    let fid = file_id(tx, &item.path)?;
    let minhash_json = serde_json::to_string(&item.minhash).unwrap_or_else(|_| "[]".to_owned());
    let page_count = i64::try_from(item.page_count).unwrap_or(i64::MAX);
    let file_size = i64::try_from(item.file_size).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO library_item (
            file, title, authors, year, doi, arxiv_id, isbn, read_status, progress,
            page_count, content_hash, minhash_json, file_size, modified_at, added_at, last_opened, meta_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT (file) DO UPDATE SET
            title = excluded.title,
            authors = excluded.authors,
            year = excluded.year,
            doi = excluded.doi,
            arxiv_id = excluded.arxiv_id,
            isbn = excluded.isbn,
            read_status = excluded.read_status,
            progress = excluded.progress,
            page_count = excluded.page_count,
            content_hash = excluded.content_hash,
            minhash_json = excluded.minhash_json,
            file_size = excluded.file_size,
            modified_at = excluded.modified_at,
            last_opened = MAX(library_item.last_opened, excluded.last_opened),
            meta_json = excluded.meta_json",
        params![
            fid,
            item.title,
            item.authors,
            item.year,
            item.doi,
            item.arxiv_id,
            item.isbn,
            item.read_status.as_str(),
            item.progress.clamp(0.0, 1.0),
            page_count,
            item.content_hash,
            minhash_json,
            file_size,
            item.modified_at,
            item.added_at,
            item.last_opened,
            item.meta_json,
        ],
    )?;
    replace_tags_for_file_id(tx, fid, &item.tags)?;
    Ok(())
}

/// Fetches a single [`LibraryItem`] by file path.
///
/// # Errors
///
/// SQLite failed.
pub fn item(conn: &Connection, path: &Path) -> Result<Option<LibraryItem>> {
    let item = conn
        .query_row(
            "SELECT f.path, l.title, l.authors, l.year, l.doi, l.arxiv_id, l.isbn,
                    l.read_status, l.progress, l.page_count, l.content_hash, l.minhash_json,
                    l.file_size, l.modified_at, l.added_at, l.last_opened, l.meta_json, f.id
             FROM library_item l JOIN file f ON f.id = l.file
             WHERE f.path = ?1",
            [path.to_string_lossy()],
            map_item_row,
        )
        .optional()?;
    match item {
        Some((mut record, fid)) => {
            record.tags = load_tags(conn, fid)?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

/// Removes a [`LibraryItem`] by path (the underlying `file` row stays intact if referenced by workspaces/history).
///
/// # Errors
///
/// SQLite failed.
pub fn delete_item(tx: &Transaction<'_>, path: &Path) -> Result<bool> {
    let deleted = tx.execute(
        "DELETE FROM library_item WHERE file = (SELECT id FROM file WHERE path = ?1)",
        [path.to_string_lossy()],
    )?;
    Ok(deleted > 0)
}

/// Lists library items matching `filter`.
///
/// # Errors
///
/// SQLite failed.
pub fn list_items(conn: &Connection, filter: &LibraryFilter) -> Result<Vec<LibraryItem>> {
    let mut sql = String::from(
        "SELECT f.path, l.title, l.authors, l.year, l.doi, l.arxiv_id, l.isbn,
                l.read_status, l.progress, l.page_count, l.content_hash, l.minhash_json,
                l.file_size, l.modified_at, l.added_at, l.last_opened, l.meta_json, f.id
         FROM library_item l
         JOIN file f ON f.id = l.file",
    );
    let mut args: Vec<SqlValue> = Vec::new();
    if let Some(cid) = filter.collection_id {
        sql.push_str(" JOIN library_collection_item lci ON lci.file = f.id AND lci.collection = ?");
        args.push(SqlValue::Integer(cid));
    }
    sql.push_str(" WHERE 1 = 1");
    if let Some(tag) = &filter.tag {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM library_file_tag t WHERE t.file = f.id AND lower(t.tag) = ?)",
        );
        args.push(SqlValue::Text(tag.trim().to_lowercase()));
    }
    if let Some(status) = filter.read_status {
        sql.push_str(" AND l.read_status = ?");
        args.push(SqlValue::Text(status.as_str().to_owned()));
    }
    if let Some(query) = &filter.query {
        let q = query.trim().to_lowercase();
        if !q.is_empty() {
            sql.push_str(
                " AND (instr(lower(l.title), ?) > 0
                  OR instr(lower(l.authors), ?) > 0
                  OR instr(lower(f.path), ?) > 0
                  OR instr(lower(l.doi), ?) > 0
                  OR instr(lower(l.arxiv_id), ?) > 0
                  OR instr(lower(l.isbn), ?) > 0)",
            );
            for _ in 0..6 {
                args.push(SqlValue::Text(q.clone()));
            }
        }
    }
    if filter.collection_id.is_some() {
        sql.push_str(" ORDER BY lci.position ASC, f.id ASC");
    } else {
        sql.push_str(" ORDER BY l.added_at DESC, l.title COLLATE NOCASE ASC, f.id ASC");
    }
    if let Some(limit) = filter.limit {
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push(SqlValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        args.push(SqlValue::Integer(
            i64::try_from(filter.offset).unwrap_or(i64::MAX),
        ));
    }

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(rusqlite::params_from_iter(args), map_item_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (item, fid) in &mut rows {
        item.tags = load_tags(conn, *fid)?;
    }
    Ok(rows.into_iter().map(|(item, _)| item).collect())
}

/// Counts total items in the library.
///
/// # Errors
///
/// SQLite failed.
pub fn count_items(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM library_item", [], |row| row.get(0))?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// Updates the reading status and progress (`0.0..=1.0`) of an existing library item.
///
/// # Errors
///
/// SQLite failed.
pub fn set_read_status(
    tx: &Transaction<'_>,
    path: &Path,
    status: ReadStatus,
    progress: f64,
) -> Result<bool> {
    let updated = tx.execute(
        "UPDATE library_item SET read_status = ?2, progress = ?3
         WHERE file = (SELECT id FROM file WHERE path = ?1)",
        params![
            path.to_string_lossy(),
            status.as_str(),
            progress.clamp(0.0, 1.0)
        ],
    )?;
    Ok(updated > 0)
}

/// Replaces all library tags on `path`.
///
/// # Errors
///
/// SQLite failed.
pub fn set_file_tags(tx: &Transaction<'_>, path: &Path, tags: &[String]) -> Result<()> {
    let fid = file_id(tx, path)?;
    replace_tags_for_file_id(tx, fid, tags)
}

/// Adds a single library tag to `path`.
///
/// # Errors
///
/// SQLite failed.
pub fn add_file_tag(tx: &Transaction<'_>, path: &Path, tag: &str) -> Result<()> {
    let trimmed = tag.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let fid = file_id(tx, path)?;
    tx.execute(
        "INSERT OR IGNORE INTO library_file_tag (file, tag) VALUES (?1, ?2)",
        params![fid, trimmed],
    )?;
    Ok(())
}

/// Removes a library tag from `path`.
///
/// # Errors
///
/// SQLite failed.
pub fn remove_file_tag(tx: &Transaction<'_>, path: &Path, tag: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM library_file_tag
         WHERE file = (SELECT id FROM file WHERE path = ?1) AND lower(tag) = lower(?2)",
        params![path.to_string_lossy(), tag.trim()],
    )?;
    Ok(())
}

/// Lists all distinct tags across the library with item counts, sorted alphabetically.
///
/// # Errors
///
/// SQLite failed.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<(String, usize)>> {
    let mut stmt = conn.prepare(
        "SELECT tag, COUNT(*) FROM library_file_tag GROUP BY lower(tag) ORDER BY lower(tag) ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Creates a new library collection (optionally nested under `parent`) and returns its id.
///
/// # Errors
///
/// SQLite failed.
pub fn create_collection(
    tx: &Transaction<'_>,
    parent: Option<i64>,
    name: &str,
    icon: Option<&str>,
    color: Option<&str>,
) -> Result<i64> {
    let id = tx.query_row(
        "INSERT INTO library_collection (parent, name, icon, color, position)
         VALUES (?1, ?2, ?3, ?4,
                 (SELECT COALESCE(MAX(position), -1) + 1 FROM library_collection WHERE parent IS ?1))
         RETURNING id",
        params![parent, name, icon, color],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Fetches a collection by id.
///
/// # Errors
///
/// SQLite failed.
pub fn collection(conn: &Connection, id: i64) -> Result<Option<Collection>> {
    Ok(conn
        .query_row(
            "SELECT c.id, c.parent, c.name, c.icon, c.color, c.position,
                    (SELECT COUNT(*) FROM library_collection_item i WHERE i.collection = c.id)
             FROM library_collection c WHERE c.id = ?1",
            [id],
            map_collection_row,
        )
        .optional()?)
}

/// Lists all library collections in tree/position order.
///
/// # Errors
///
/// SQLite failed.
pub fn list_collections(conn: &Connection) -> Result<Vec<Collection>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.parent, c.name, c.icon, c.color, c.position,
                (SELECT COUNT(*) FROM library_collection_item i WHERE i.collection = c.id)
         FROM library_collection c
         ORDER BY COALESCE(c.parent, 0) ASC, c.position ASC, c.id ASC",
    )?;
    let rows = stmt
        .query_map([], map_collection_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Renames a collection. Returns `true` if the collection existed.
///
/// # Errors
///
/// SQLite failed.
pub fn rename_collection(tx: &Transaction<'_>, id: i64, name: &str) -> Result<bool> {
    let updated = tx.execute(
        "UPDATE library_collection SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(updated > 0)
}

/// Deletes a collection and its child collections (documents stay in the library).
///
/// # Errors
///
/// SQLite failed.
pub fn delete_collection(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let deleted = tx.execute("DELETE FROM library_collection WHERE id = ?1", [id])?;
    Ok(deleted > 0)
}

/// Appends `path` to the end of `collection_id` if not already present.
///
/// # Errors
///
/// SQLite failed.
pub fn add_to_collection(tx: &Transaction<'_>, collection_id: i64, path: &Path) -> Result<()> {
    let fid = file_id(tx, path)?;
    tx.execute(
        "INSERT OR IGNORE INTO library_collection_item (collection, file, position)
         VALUES (?1, ?2,
                 (SELECT COALESCE(MAX(position), -1) + 1 FROM library_collection_item WHERE collection = ?1))",
        params![collection_id, fid],
    )?;
    Ok(())
}

/// Replaces all items of `collection_id` with `paths` in their exact slice order (plan W.10).
///
/// # Errors
///
/// SQLite failed.
pub fn set_collection_items(
    tx: &Transaction<'_>,
    collection_id: i64,
    paths: &[PathBuf],
) -> Result<()> {
    tx.execute(
        "DELETE FROM library_collection_item WHERE collection = ?1",
        [collection_id],
    )?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO library_collection_item (collection, file, position)
         VALUES (?1, ?2, ?3)",
    )?;
    for (position, path) in (0_i64..).zip(paths) {
        let fid = file_id(tx, path)?;
        insert.execute(params![collection_id, fid, position])?;
    }
    Ok(())
}

/// Removes `path` from `collection_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn remove_from_collection(tx: &Transaction<'_>, collection_id: i64, path: &Path) -> Result<()> {
    tx.execute(
        "DELETE FROM library_collection_item
         WHERE collection = ?1 AND file = (SELECT id FROM file WHERE path = ?2)",
        params![collection_id, path.to_string_lossy()],
    )?;
    Ok(())
}

/// Returns all file paths in `collection_id` in their saved order (plan W.10).
///
/// # Errors
///
/// SQLite failed.
pub fn collection_items(conn: &Connection, collection_id: i64) -> Result<Vec<PathBuf>> {
    let mut stmt = conn.prepare(
        "SELECT f.path FROM library_collection_item i
         JOIN file f ON f.id = i.file
         WHERE i.collection = ?1
         ORDER BY i.position ASC, f.id ASC",
    )?;
    let rows = stmt
        .query_map([collection_id], |row| {
            Ok(PathBuf::from(row.get::<_, String>(0)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Registers or updates a watched folder (plan F.9).
///
/// # Errors
///
/// SQLite failed.
pub fn add_watched_folder(tx: &Transaction<'_>, path: &Path, depth: WatchDepth) -> Result<i64> {
    let id = tx.query_row(
        "INSERT INTO watched_folder (path, recursive, last_scan)
         VALUES (?1, ?2, 0)
         ON CONFLICT (path) DO UPDATE SET recursive = excluded.recursive
         RETURNING id",
        params![path.to_string_lossy(), i64::from(depth.is_recursive())],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Removes a watched folder.
///
/// # Errors
///
/// SQLite failed.
pub fn remove_watched_folder(tx: &Transaction<'_>, path: &Path) -> Result<bool> {
    let deleted = tx.execute(
        "DELETE FROM watched_folder WHERE path = ?1",
        [path.to_string_lossy()],
    )?;
    Ok(deleted > 0)
}

/// Lists all registered watched folders.
///
/// # Errors
///
/// SQLite failed.
pub fn list_watched_folders(conn: &Connection) -> Result<Vec<WatchedFolder>> {
    let mut stmt =
        conn.prepare("SELECT id, path, recursive, last_scan FROM watched_folder ORDER BY id ASC")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WatchedFolder {
                id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                depth: WatchDepth::from_flag(row.get::<_, i64>(2)? != 0),
                last_scan: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Updates the `last_scan` timestamp of a watched folder.
///
/// # Errors
///
/// SQLite failed.
pub fn mark_folder_scanned(tx: &Transaction<'_>, path: &Path, timestamp: i64) -> Result<()> {
    tx.execute(
        "UPDATE watched_folder SET last_scan = ?2 WHERE path = ?1",
        params![path.to_string_lossy(), timestamp],
    )?;
    Ok(())
}

/// Finds all library paths sharing the same exact `content_hash` (plan F.8).
///
/// # Errors
///
/// SQLite failed.
pub fn find_by_hash(conn: &Connection, content_hash: &str) -> Result<Vec<PathBuf>> {
    if content_hash.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT f.path FROM library_item l JOIN file f ON f.id = l.file
         WHERE l.content_hash = ?1 ORDER BY f.path ASC",
    )?;
    let rows = stmt
        .query_map([content_hash], |row| {
            Ok(PathBuf::from(row.get::<_, String>(0)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn replace_tags_for_file_id(tx: &Transaction<'_>, fid: i64, tags: &[String]) -> Result<()> {
    tx.execute("DELETE FROM library_file_tag WHERE file = ?1", [fid])?;
    let mut stmt =
        tx.prepare("INSERT OR IGNORE INTO library_file_tag (file, tag) VALUES (?1, ?2)")?;
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            stmt.execute(params![fid, trimmed])?;
        }
    }
    Ok(())
}

fn load_tags(conn: &Connection, fid: i64) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT tag FROM library_file_tag WHERE file = ?1 ORDER BY tag COLLATE NOCASE")?;
    let rows = stmt
        .query_map([fid], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn map_item_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(LibraryItem, i64)> {
    let status_str: String = row.get(7)?;
    let minhash_json: String = row.get(11)?;
    let minhash: Vec<u64> = serde_json::from_str(&minhash_json).unwrap_or_default();
    let item = LibraryItem {
        path: PathBuf::from(row.get::<_, String>(0)?),
        title: row.get(1)?,
        authors: row.get(2)?,
        year: row.get(3)?,
        doi: row.get(4)?,
        arxiv_id: row.get(5)?,
        isbn: row.get(6)?,
        read_status: ReadStatus::from_name(&status_str).unwrap_or_default(),
        progress: row.get(8)?,
        page_count: usize::try_from(row.get::<_, i64>(9)?).unwrap_or(0),
        content_hash: row.get(10)?,
        minhash,
        file_size: u64::try_from(row.get::<_, i64>(12)?).unwrap_or(0),
        modified_at: row.get(13)?,
        added_at: row.get(14)?,
        last_opened: row.get(15)?,
        meta_json: row.get(16)?,
        tags: Vec::new(),
    };
    let fid: i64 = row.get(17)?;
    Ok((item, fid))
}

fn map_collection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Collection> {
    Ok(Collection {
        id: row.get(0)?,
        parent: row.get(1)?,
        name: row.get(2)?,
        icon: row.get(3)?,
        color: row.get(4)?,
        position: usize::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
        item_count: usize::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
    })
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
    fn library_items_tags_and_status_round_trip() {
        let mut conn = database();
        let tx = conn.transaction().expect("tx");
        upsert_item(
            &tx,
            &LibraryItem {
                path: PathBuf::from("C:/papers/attention.pdf"),
                title: "Attention Is All You Need".to_owned(),
                authors: "Vaswani et al.".to_owned(),
                year: Some(2017),
                doi: "10.5555/3295222.3295349".to_owned(),
                read_status: ReadStatus::Reading,
                progress: 0.45,
                page_count: 15,
                content_hash: "abc123".to_owned(),
                minhash: vec![1, 2, 3, 4],
                tags: vec!["Transformers".to_owned(), "NLP".to_owned()],
                ..LibraryItem::default()
            },
        )
        .expect("upsert");
        tx.commit().expect("commit");

        let loaded = item(&conn, Path::new("C:/papers/attention.pdf"))
            .expect("get")
            .expect("present");
        assert_eq!(loaded.title, "Attention Is All You Need");
        assert_eq!(loaded.read_status, ReadStatus::Reading);
        assert_eq!(loaded.minhash, [1, 2, 3, 4]);
        assert_eq!(loaded.tags, ["NLP", "Transformers"]);

        let tx = conn.transaction().expect("tx");
        set_read_status(
            &tx,
            Path::new("C:/papers/attention.pdf"),
            ReadStatus::Finished,
            1.0,
        )
        .expect("status");
        add_file_tag(&tx, Path::new("C:/papers/attention.pdf"), "Classic").expect("tag");
        tx.commit().expect("commit");

        let filtered = list_items(
            &conn,
            &LibraryFilter {
                read_status: Some(ReadStatus::Finished),
                tag: Some("classic".to_owned()),
                ..LibraryFilter::default()
            },
        )
        .expect("list");
        assert_eq!(filtered.len(), 1);
        assert_eq!(count_items(&conn).expect("count"), 1);
        assert_eq!(
            find_by_hash(&conn, "abc123").expect("hash"),
            vec![PathBuf::from("C:/papers/attention.pdf")]
        );
    }

    #[test]
    fn nested_collections_preserve_file_order() {
        let mut conn = database();
        let tx = conn.transaction().expect("tx");
        let root = create_collection(&tx, None, "PhD", Some("📚"), Some("blue")).expect("root");
        let sub = create_collection(&tx, Some(root), "Chapter 1", None, None).expect("sub");
        let paths = vec![
            PathBuf::from("C:/p/3.pdf"),
            PathBuf::from("C:/p/1.pdf"),
            PathBuf::from("C:/p/2.pdf"),
        ];
        set_collection_items(&tx, sub, &paths).expect("set items");
        tx.commit().expect("commit");

        assert_eq!(collection_items(&conn, sub).expect("items"), paths);
        let cols = list_collections(&conn).expect("cols");
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[1].parent, Some(root));
        assert_eq!(cols[1].item_count, 3);
    }
}
