//! Markdown notes with `[[wiki-links]]`, backlink graph, and rename propagation (plan F.12).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Result;
use crate::workspace::file_id;

/// A Markdown note stored in the library database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Stable identifier (slug or UUID).
    pub id: String,
    /// Unique note title (case-insensitive) used by `[[title]]` wiki-links.
    pub title: String,
    /// Markdown body text.
    pub body: String,
    /// Optional anchored document path.
    pub file: Option<PathBuf>,
    /// Optional anchored zero-based page index.
    pub page: Option<usize>,
    /// Optional anchored annotation `/NM` id.
    pub annotation_id: Option<String>,
    /// Note tags.
    pub tags: Vec<String>,
    /// Creation timestamp (UTC seconds since 1970).
    pub created_at: i64,
    /// Last modification timestamp (UTC seconds since 1970).
    pub modified_at: i64,
}

/// A parsed `[[wiki-link]]` target inside a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Link to another Markdown note by title: `[[Note Title]]`.
    Note {
        /// Referenced note title (with optional alias stripped).
        title: String,
    },
    /// Link to a document, optional page (1-based or 0-based normalized by caller), and optional annotation:
    /// `[[doc.pdf#page=3]]` or `[[doc.pdf#3&annot=id]]`.
    Document {
        /// Document name or path before `#`.
        doc: String,
        /// Optional page number after `#`.
        page: Option<usize>,
        /// Optional annotation id.
        annot: Option<String>,
    },
}

/// One stored edge in the note link graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLink {
    /// Source note id.
    pub source_note: String,
    /// Source note title.
    pub source_title: String,
    /// Ordinal position within the source note.
    pub position: usize,
    /// Raw link inner text (before `|alias` if any).
    pub target_raw: String,
    /// Resolved target note id, if it resolves to an existing note.
    pub target_note: Option<String>,
    /// Resolved target file path, if it is a document link matching a known file.
    pub target_file: Option<PathBuf>,
    /// Target page, if specified.
    pub target_page: Option<usize>,
    /// Target annotation id, if specified.
    pub target_annot: Option<String>,
}

/// Extracts all raw `[[wiki-link]]` targets from Markdown `body` in order, stripping `|alias` display labels.
#[must_use]
pub fn extract_wiki_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("]]") else {
            break;
        };
        let inner = &after_open[..end];
        // Ignore nested openings or newlines inside brackets.
        if !inner.contains('\n') && !inner.contains("[[") {
            let target = inner.split('|').next().unwrap_or("").trim();
            if !target.is_empty() {
                links.push(target.to_owned());
            }
        }
        rest = &after_open[end + 2..];
    }
    links
}

/// Parses a raw link target string into a [`LinkTarget`].
#[must_use]
pub fn parse_link_target(raw: &str) -> LinkTarget {
    let trimmed = raw.trim();
    if let Some((doc, frag)) = trimmed.split_once('#') {
        let doc = doc.trim().to_owned();
        let mut page = None;
        let mut annot = None;
        for part in frag.split('&') {
            let part = part.trim();
            if let Some(val) = part.strip_prefix("page=") {
                page = val.parse::<usize>().ok();
            } else if let Some(val) = part.strip_prefix("annot=") {
                if !val.is_empty() {
                    annot = Some(val.to_owned());
                }
            } else if let Some(val) = part.strip_prefix('p') {
                if page.is_none() {
                    page = val.parse::<usize>().ok();
                }
            } else if page.is_none()
                && let Ok(num) = part.parse::<usize>()
            {
                page = Some(num);
            }
        }
        LinkTarget::Document { doc, page, annot }
    } else {
        LinkTarget::Note {
            title: trimmed.to_owned(),
        }
    }
}

/// Inserts or updates `note`, refreshes its outgoing `note_link` graph, and resolves any incoming unresolved
/// links whose raw target matches `note.title`.
///
/// # Errors
///
/// SQLite failed.
pub fn upsert_note(tx: &Transaction<'_>, note: &Note) -> Result<()> {
    let fid = match note.file.as_deref() {
        Some(path) => Some(file_id(tx, path)?),
        None => None,
    };
    let tags_json = serde_json::to_string(&note.tags).unwrap_or_else(|_| "[]".to_owned());
    let page = note.page.map(|p| i64::try_from(p).unwrap_or(i64::MAX));
    tx.execute(
        "INSERT INTO note (id, title, body, file, page, annotation_id, tags_json, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET
            title = excluded.title,
            body = excluded.body,
            file = excluded.file,
            page = excluded.page,
            annotation_id = excluded.annotation_id,
            tags_json = excluded.tags_json,
            modified_at = excluded.modified_at",
        params![
            note.id,
            note.title,
            note.body,
            fid,
            page,
            note.annotation_id,
            tags_json,
            note.created_at,
            note.modified_at,
        ],
    )?;

    rebuild_links_for_note(tx, &note.id, &note.body)?;

    // Resolve previously dangling links in other notes that now match this note's title.
    tx.execute(
        "UPDATE note_link SET target_note = ?1
         WHERE target_note IS NULL AND target_file IS NULL AND target_raw = ?2 COLLATE NOCASE",
        params![note.id, note.title],
    )?;
    Ok(())
}

/// Renames the note `note_id` to `new_title` and rewrites every `[[old_title]]` / `[[old_title|alias]]`
/// across all notes in the database (plan F.12). Returns the number of notes whose bodies were updated.
///
/// # Errors
///
/// SQLite failed, or `note_id` does not exist.
pub fn rename_note(
    tx: &Transaction<'_>,
    note_id: &str,
    new_title: &str,
    modified_at: i64,
) -> Result<usize> {
    let old_title: Option<String> = tx
        .query_row("SELECT title FROM note WHERE id = ?1", [note_id], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(old_title) = old_title else {
        return Ok(0);
    };
    let new_trimmed = new_title.trim();
    if old_title == new_trimmed {
        return Ok(0);
    }

    tx.execute(
        "UPDATE note SET title = ?2, modified_at = ?3 WHERE id = ?1",
        params![note_id, new_trimmed, modified_at],
    )?;

    let mut stmt = tx.prepare("SELECT id, body FROM note")?;
    let all_notes: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut updated_count = 0usize;
    for (id, body) in all_notes {
        let rewritten = rewrite_wiki_links(&body, &old_title, new_trimmed);
        if rewritten != body {
            tx.execute(
                "UPDATE note SET body = ?2, modified_at = ?3 WHERE id = ?1",
                params![id, rewritten, modified_at],
            )?;
            rebuild_links_for_note(tx, &id, &rewritten)?;
            updated_count += 1;
        }
    }
    // Ensure any incoming links pointing to `note_id` have updated raw text as well.
    tx.execute(
        "UPDATE note_link SET target_raw = ?2 WHERE target_note = ?1",
        params![note_id, new_trimmed],
    )?;
    Ok(updated_count)
}

/// Rewrites `[[old_title]]` and `[[old_title|alias]]` (case-insensitive match on title) in `body` to `new_title`.
#[must_use]
pub fn rewrite_wiki_links(body: &str, old_title: &str, new_title: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("]]") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after_open[..end];
        if !inner.contains('\n') && !inner.contains("[[") {
            let (target, alias) = match inner.split_once('|') {
                Some((t, a)) => (t.trim(), Some(a)),
                None => (inner.trim(), None),
            };
            if target.eq_ignore_ascii_case(old_title) {
                out.push_str("[[");
                out.push_str(new_title);
                if let Some(alias) = alias {
                    out.push('|');
                    out.push_str(alias);
                }
                out.push_str("]]");
                rest = &after_open[end + 2..];
                continue;
            }
        }
        out.push_str("[[");
        out.push_str(inner);
        out.push_str("]]");
        rest = &after_open[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Fetches a note by id.
///
/// # Errors
///
/// SQLite failed.
pub fn note(conn: &Connection, id: &str) -> Result<Option<Note>> {
    Ok(conn
        .query_row(
            "SELECT n.id, n.title, n.body, f.path, n.page, n.annotation_id, n.tags_json, n.created_at, n.modified_at
             FROM note n LEFT JOIN file f ON f.id = n.file WHERE n.id = ?1",
            [id],
            map_note_row,
        )
        .optional()?)
}

/// Fetches a note by title (case-insensitive).
///
/// # Errors
///
/// SQLite failed.
pub fn note_by_title(conn: &Connection, title: &str) -> Result<Option<Note>> {
    Ok(conn
        .query_row(
            "SELECT n.id, n.title, n.body, f.path, n.page, n.annotation_id, n.tags_json, n.created_at, n.modified_at
             FROM note n LEFT JOIN file f ON f.id = n.file WHERE n.title = ?1 COLLATE NOCASE",
            [title.trim()],
            map_note_row,
        )
        .optional()?)
}

/// Lists all notes ordered by most recently modified first.
///
/// # Errors
///
/// SQLite failed.
pub fn list_notes(conn: &Connection) -> Result<Vec<Note>> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.title, n.body, f.path, n.page, n.annotation_id, n.tags_json, n.created_at, n.modified_at
         FROM note n LEFT JOIN file f ON f.id = n.file ORDER BY n.modified_at DESC, n.title COLLATE NOCASE ASC",
    )?;
    let rows = stmt
        .query_map([], map_note_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Lists all notes anchored to a specific file path.
///
/// # Errors
///
/// SQLite failed.
pub fn notes_for_file(conn: &Connection, path: &Path) -> Result<Vec<Note>> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.title, n.body, f.path, n.page, n.annotation_id, n.tags_json, n.created_at, n.modified_at
         FROM note n JOIN file f ON f.id = n.file WHERE f.path = ?1
         ORDER BY COALESCE(n.page, 0) ASC, n.modified_at DESC",
    )?;
    let rows = stmt
        .query_map([path.to_string_lossy()], map_note_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes a note by id. Incoming links from other notes become unresolved (`target_note = NULL`).
///
/// # Errors
///
/// SQLite failed.
pub fn delete_note(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let deleted = tx.execute("DELETE FROM note WHERE id = ?1", [id])?;
    Ok(deleted > 0)
}

/// Returns all incoming backlinks pointing to `note_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn backlinks_for_note(conn: &Connection, note_id: &str) -> Result<Vec<NoteLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.source_note, s.title, l.position, l.target_raw, l.target_note, f.path, l.target_page, l.target_annot
         FROM note_link l
         JOIN note s ON s.id = l.source_note
         LEFT JOIN file f ON f.id = l.target_file
         WHERE l.target_note = ?1
         ORDER BY s.title COLLATE NOCASE ASC, l.position ASC",
    )?;
    let rows = stmt
        .query_map([note_id], map_link_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Returns all outgoing links from `note_id`.
///
/// # Errors
///
/// SQLite failed.
pub fn outgoing_links(conn: &Connection, note_id: &str) -> Result<Vec<NoteLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.source_note, s.title, l.position, l.target_raw, l.target_note, f.path, l.target_page, l.target_annot
         FROM note_link l
         JOIN note s ON s.id = l.source_note
         LEFT JOIN file f ON f.id = l.target_file
         WHERE l.source_note = ?1
         ORDER BY l.position ASC",
    )?;
    let rows = stmt
        .query_map([note_id], map_link_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Returns all unresolved links (`target_note IS NULL AND target_file IS NULL`) across all notes.
///
/// # Errors
///
/// SQLite failed.
pub fn unresolved_links(conn: &Connection) -> Result<Vec<NoteLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.source_note, s.title, l.position, l.target_raw, l.target_note, f.path, l.target_page, l.target_annot
         FROM note_link l
         JOIN note s ON s.id = l.source_note
         LEFT JOIN file f ON f.id = l.target_file
         WHERE l.target_note IS NULL AND l.target_file IS NULL
         ORDER BY l.target_raw COLLATE NOCASE ASC, s.title COLLATE NOCASE ASC",
    )?;
    let rows = stmt
        .query_map([], map_link_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn rebuild_links_for_note(tx: &Transaction<'_>, note_id: &str, body: &str) -> Result<()> {
    tx.execute("DELETE FROM note_link WHERE source_note = ?1", [note_id])?;
    let raw_links = extract_wiki_links(body);
    let mut insert = tx.prepare(
        "INSERT INTO note_link (
            source_note, position, target_raw, target_note, target_file, target_page, target_annot
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (pos, raw) in raw_links.into_iter().enumerate() {
        let (target_note, target_file, target_page, target_annot) = match parse_link_target(&raw) {
            LinkTarget::Note { title } => {
                let target_id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM note WHERE title = ?1 COLLATE NOCASE",
                        [title],
                        |row| row.get(0),
                    )
                    .optional()?;
                (target_id, None, None, None)
            }
            LinkTarget::Document { doc, page, annot } => {
                let fid: Option<i64> = if doc.is_empty() {
                    None
                } else {
                    tx.query_row(
                        "SELECT id FROM file WHERE path = ?1 OR lower(path) LIKE ?2 LIMIT 1",
                        params![doc, format!("%/{}", doc.to_lowercase())],
                        |row| row.get(0),
                    )
                    .optional()?
                };
                let page_i64 = page.map(|p| i64::try_from(p).unwrap_or(i64::MAX));
                (None, fid, page_i64, annot)
            }
        };
        let position = i64::try_from(pos).unwrap_or(i64::MAX);
        insert.execute(params![
            note_id,
            position,
            raw,
            target_note,
            target_file,
            target_page,
            target_annot,
        ])?;
    }
    Ok(())
}

fn map_note_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let tags_json: String = row.get(6)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        file: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        page: row
            .get::<_, Option<i64>>(4)?
            .and_then(|p| usize::try_from(p).ok()),
        annotation_id: row.get(5)?,
        tags,
        created_at: row.get(7)?,
        modified_at: row.get(8)?,
    })
}

fn map_link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteLink> {
    Ok(NoteLink {
        source_note: row.get(0)?,
        source_title: row.get(1)?,
        position: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        target_raw: row.get(3)?,
        target_note: row.get(4)?,
        target_file: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
        target_page: row
            .get::<_, Option<i64>>(6)?
            .and_then(|p| usize::try_from(p).ok()),
        target_annot: row.get(7)?,
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
    fn wiki_links_backlinks_and_renaming_update_all_references() {
        let mut conn = database();
        let tx = conn.transaction().expect("tx");
        upsert_note(
            &tx,
            &Note {
                id: "n1".to_owned(),
                title: "Transformers".to_owned(),
                body: "See [[Attention Mechanism]] and [[Missing Concept|alias]].".to_owned(),
                file: Some(PathBuf::from("C:/papers/a.pdf")),
                page: Some(2),
                annotation_id: None,
                tags: vec!["ml".to_owned()],
                created_at: 10,
                modified_at: 10,
            },
        )
        .expect("n1");
        upsert_note(
            &tx,
            &Note {
                id: "n2".to_owned(),
                title: "Attention Mechanism".to_owned(),
                body: "Back to [[Transformers]] and [[a.pdf#page=3]].".to_owned(),
                file: None,
                page: None,
                annotation_id: None,
                tags: Vec::new(),
                created_at: 11,
                modified_at: 11,
            },
        )
        .expect("n2");
        tx.commit().expect("commit");

        let backlinks = backlinks_for_note(&conn, "n2").expect("backlinks");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].source_note, "n1");

        let unresolved = unresolved_links(&conn).expect("unresolved");
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].target_raw, "Missing Concept");

        let tx = conn.transaction().expect("tx");
        let updated = rename_note(&tx, "n2", "Self-Attention", 20).expect("rename");
        tx.commit().expect("commit");
        assert_eq!(updated, 1);

        let n1 = note(&conn, "n1").expect("get").expect("n1");
        assert!(n1.body.contains("[[Self-Attention]]"));
        let backlinks_after = backlinks_for_note(&conn, "n2").expect("backlinks");
        assert_eq!(backlinks_after.len(), 1);
        assert_eq!(backlinks_after[0].target_raw, "Self-Attention");
    }
}
