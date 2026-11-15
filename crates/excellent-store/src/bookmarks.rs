//! Personal bookmarks (plan B.6): places the reader saved in a file, with a title, a note, a colour and tags. They
//! live here, not in the file, so every format has them. A file renamed or moved finds its bookmarks again by its
//! content ([`find_again`]).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Result;
use crate::workspace::file_id;

/// Where a bookmark opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    /// The page, from 0.
    pub page: usize,
    /// The point at the top left of the view, in page points.
    pub x: f32,
    /// See `x`.
    pub y: f32,
    /// The zoom (1 is 100 %), or `None` for the reader's own.
    pub zoom: Option<f32>,
}

/// What the reader can change about a bookmark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Details {
    /// Its name in the list.
    pub title: String,
    /// The reader's note; may be empty.
    pub note: String,
    /// Its colour's id (the app's palette).
    pub color: String,
    /// Its tags, each once, in the reader's order.
    pub tags: Vec<String>,
}

/// A saved bookmark.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    /// Its id in the library.
    pub id: i64,
    /// Where it opens.
    pub place: Place,
    /// The text selected when it was made, which re-anchors it when the file changes (K.28).
    pub quote: Option<String>,
    /// Its title, note, colour and tags.
    pub details: Details,
    /// When it was made, in seconds since 1970.
    pub created: i64,
}

/// The bookmarks of the file at `path`, in reading order.
///
/// # Errors
///
/// SQLite failed.
pub fn list(conn: &Connection, path: &Path) -> Result<Vec<Bookmark>> {
    let mut rows = conn.prepare(
        "SELECT b.id, b.page, b.x, b.y, b.zoom, b.quote, b.title, b.note, b.color, b.created
         FROM bookmark b JOIN file f ON f.id = b.file WHERE f.path = ?1
         ORDER BY b.page, b.y, b.x, b.id",
    )?;
    let mut bookmarks = rows
        .query_map([path.to_string_lossy()], |row| {
            Ok(Bookmark {
                id: row.get(0)?,
                place: Place {
                    page: row.get(1)?,
                    x: row.get::<_, f64>(2)? as f32,
                    y: row.get::<_, f64>(3)? as f32,
                    zoom: row.get::<_, Option<f64>>(4)?.map(|zoom| zoom as f32),
                },
                quote: row.get(5)?,
                details: Details {
                    title: row.get(6)?,
                    note: row.get(7)?,
                    color: row.get(8)?,
                    tags: Vec::new(),
                },
                created: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tags =
        conn.prepare("SELECT tag FROM bookmark_tag WHERE bookmark = ?1 ORDER BY rowid")?;
    for bookmark in &mut bookmarks {
        bookmark.details.tags = tags
            .query_map([bookmark.id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(bookmarks)
}

/// Saves a bookmark in the file at `path`, made at `created`, and returns its id.
///
/// # Errors
///
/// SQLite failed.
pub fn add(
    tx: &Transaction<'_>,
    path: &Path,
    place: Place,
    quote: Option<&str>,
    details: &Details,
    created: i64,
) -> Result<i64> {
    let file = file_id(tx, path)?;
    let id = tx.query_row(
        "INSERT INTO bookmark (file, page, x, y, zoom, quote, title, note, color, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
        params![
            file,
            place.page,
            f64::from(place.x),
            f64::from(place.y),
            place.zoom.map(f64::from),
            quote,
            details.title,
            details.note,
            details.color,
            created
        ],
        |row| row.get(0),
    )?;
    set_tags(tx, id, &details.tags)?;
    Ok(id)
}

/// Gives the bookmark `id` these `details`.
///
/// # Errors
///
/// SQLite failed.
pub fn update(tx: &Transaction<'_>, id: i64, details: &Details) -> Result<()> {
    tx.execute(
        "UPDATE bookmark SET title = ?2, note = ?3, color = ?4 WHERE id = ?1",
        params![id, details.title, details.note, details.color],
    )?;
    set_tags(tx, id, &details.tags)
}

/// Deletes the bookmark `id`.
///
/// # Errors
///
/// SQLite failed.
pub fn delete(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM bookmark WHERE id = ?1", [id])?;
    Ok(())
}

fn set_tags(tx: &Transaction<'_>, id: i64, tags: &[String]) -> Result<()> {
    tx.execute("DELETE FROM bookmark_tag WHERE bookmark = ?1", [id])?;
    let mut insert =
        tx.prepare("INSERT OR IGNORE INTO bookmark_tag (bookmark, tag) VALUES (?1, ?2)")?;
    for tag in tags {
        insert.execute(params![id, tag])?;
    }
    Ok(())
}

/// Notes that the file at `path` holds `content` (`FileId::content_key`), and when it has no bookmarks takes those of
/// a file with the same content that is gone (`exists` says it isn't there): the file was renamed or moved. Returns
/// whether bookmarks were found again.
///
/// shortcut: a file edited and then renamed before it is shown again has new content, so its bookmarks stay under
/// the old name; K.28's re-anchoring by text finds them when it comes.
///
/// # Errors
///
/// SQLite failed.
pub fn find_again(
    tx: &Transaction<'_>,
    path: &Path,
    content: u64,
    exists: impl Fn(&Path) -> bool,
) -> Result<bool> {
    let content = i64::from_ne_bytes(content.to_ne_bytes());
    let file = file_id(tx, path)?;
    tx.execute(
        "UPDATE file SET content = ?2 WHERE id = ?1",
        params![file, content],
    )?;
    let has_own: Option<i64> = tx
        .query_row(
            "SELECT id FROM bookmark WHERE file = ?1 LIMIT 1",
            [file],
            |row| row.get(0),
        )
        .optional()?;
    if has_own.is_some() {
        return Ok(false);
    }
    let mut others = tx.prepare(
        "SELECT f.id, f.path FROM file f WHERE f.content = ?1 AND f.id <> ?2
         AND EXISTS (SELECT 1 FROM bookmark b WHERE b.file = f.id) ORDER BY f.id DESC",
    )?;
    let gone = others
        .query_map(params![content, file], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .find(|(_, old)| !exists(Path::new(old)));
    let Some((old, _)) = gone else {
        return Ok(false);
    };
    tx.execute(
        "UPDATE bookmark SET file = ?2 WHERE file = ?1",
        params![old, file],
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::migrations;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("foreign keys");
        migrations::migrate(&mut conn).expect("migrate");
        conn
    }

    fn change<T>(conn: &mut Connection, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> T {
        let tx = conn.transaction().expect("transaction");
        let value = f(&tx).expect("change");
        tx.commit().expect("commit");
        value
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("C:/papers/{name}.pdf"))
    }

    fn place(page: usize, y: f32) -> Place {
        Place {
            page,
            x: 0.0,
            y,
            zoom: None,
        }
    }

    fn details(title: &str, tags: &[&str]) -> Details {
        Details {
            title: title.to_owned(),
            note: String::new(),
            color: "blue".to_owned(),
            tags: tags.iter().map(|&tag| tag.to_owned()).collect(),
        }
    }

    fn titles(conn: &Connection, name: &str) -> Vec<String> {
        list(conn, &path(name))
            .expect("list")
            .into_iter()
            .map(|bookmark| bookmark.details.title)
            .collect()
    }

    #[test]
    fn bookmarks_are_listed_in_reading_order_with_their_details() {
        let mut conn = database();
        let later = change(&mut conn, |tx| {
            add(
                tx,
                &path("a"),
                place(4, 10.0),
                None,
                &details("Later", &[]),
                1,
            )
        });
        change(&mut conn, |tx| {
            add(
                tx,
                &path("a"),
                place(2, 300.0),
                Some("the quote"),
                &details("Lower", &["x"]),
                2,
            )
        });
        change(&mut conn, |tx| {
            add(
                tx,
                &path("a"),
                place(2, 50.0),
                None,
                &details("Upper", &[]),
                3,
            )
        });
        change(&mut conn, |tx| {
            add(
                tx,
                &path("b"),
                place(0, 0.0),
                None,
                &details("Other", &[]),
                4,
            )
        });
        assert_eq!(titles(&conn, "a"), ["Upper", "Lower", "Later"]);
        let lower = &list(&conn, &path("a")).expect("list")[1];
        assert_eq!(lower.quote.as_deref(), Some("the quote"));
        assert_eq!(lower.details.tags, ["x"]);
        assert_eq!(lower.place, place(2, 300.0));

        let edited = Details {
            note: "Read again".to_owned(),
            color: "red".to_owned(),
            ..details("Later on", &["method", "to read", "method"])
        };
        change(&mut conn, |tx| update(tx, later, &edited));
        let shown = list(&conn, &path("a")).expect("list").pop().expect("one");
        assert_eq!(shown.details.title, "Later on");
        assert_eq!(shown.details.note, "Read again");
        assert_eq!(shown.details.color, "red");
        assert_eq!(shown.details.tags, ["method", "to read"], "each tag once");

        change(&mut conn, |tx| delete(tx, later));
        assert_eq!(titles(&conn, "a"), ["Upper", "Lower"]);
    }

    #[test]
    fn a_renamed_file_finds_its_bookmarks_by_its_content() {
        let mut conn = database();
        change(&mut conn, |tx| find_again(tx, &path("old"), 7, |_| true));
        change(&mut conn, |tx| {
            add(
                tx,
                &path("old"),
                place(1, 0.0),
                None,
                &details("Kept", &[]),
                1,
            )
        });
        // Another file with other content takes nothing.
        assert!(!change(&mut conn, |tx| find_again(
            tx,
            &path("unrelated"),
            8,
            |_| false
        )));
        // The same bytes under a new name, while the old file is still there: a copy, which takes nothing.
        assert!(!change(&mut conn, |tx| find_again(
            tx,
            &path("copy"),
            7,
            |_| true
        )));
        assert!(titles(&conn, "copy").is_empty());
        // The old file is gone: renamed.
        assert!(change(&mut conn, |tx| find_again(
            tx,
            &path("new"),
            7,
            |p| p != path("old")
        )));
        assert_eq!(titles(&conn, "new"), ["Kept"]);
        assert!(titles(&conn, "old").is_empty());
        // A file with bookmarks of its own keeps them.
        assert!(!change(&mut conn, |tx| find_again(
            tx,
            &path("new"),
            7,
            |_| false
        )));
        assert_eq!(titles(&conn, "new"), ["Kept"]);
    }
}
