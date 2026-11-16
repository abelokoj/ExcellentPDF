//! The library's index of annotation tags (plan A.18): which annotations of which files carry which tags, as each
//! file was when last saved. The tags themselves live in the file, on the annotation; this index lets a search of
//! the whole library (`tag:`, F.4) find them without opening every file.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction, params};

use crate::Result;
use crate::workspace::file_id;

/// An annotation's tags, as a file holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    /// The annotation's id (`/NM`).
    pub annotation: String,
    /// Its page, from 0.
    pub page: usize,
    /// Its tags.
    pub tags: Vec<String>,
}

/// An annotation a tag found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The file it is in.
    pub path: PathBuf,
    /// Its page, from 0.
    pub page: usize,
    /// Its id.
    pub annotation: String,
}

/// Makes `tagged` the tags of the annotations of the file at `path`, in place of those indexed before.
///
/// # Errors
///
/// SQLite failed.
pub fn index(tx: &Transaction<'_>, path: &Path, tagged: &[Tagged]) -> Result<()> {
    let file = file_id(tx, path)?;
    tx.execute("DELETE FROM annotation_tag WHERE file = ?1", [file])?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO annotation_tag (file, annotation, page, tag) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for annotation in tagged {
        for tag in &annotation.tags {
            insert.execute(params![file, annotation.annotation, annotation.page, tag])?;
        }
    }
    Ok(())
}

/// The annotations of every file tagged `tag`, whatever its case, by file and then page.
///
/// # Errors
///
/// SQLite failed.
pub fn find(conn: &Connection, tag: &str) -> Result<Vec<Found>> {
    let mut rows = conn.prepare(
        "SELECT DISTINCT f.path, t.page, t.annotation FROM annotation_tag t JOIN file f ON f.id = t.file
         WHERE t.tag = ?1 COLLATE NOCASE ORDER BY f.path, t.page, t.annotation",
    )?;
    let found = rows
        .query_map([tag.trim()], |row| {
            Ok(Found {
                path: PathBuf::from(row.get::<_, String>(0)?),
                page: row.get(1)?,
                annotation: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(found)
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

    fn write(conn: &mut Connection, path: &str, tagged: &[(&str, usize, &[&str])]) {
        let tagged: Vec<Tagged> = tagged
            .iter()
            .map(|(id, page, tags)| Tagged {
                annotation: (*id).to_owned(),
                page: *page,
                tags: tags.iter().map(|&tag| tag.to_owned()).collect(),
            })
            .collect();
        let tx = conn.transaction().expect("transaction");
        index(&tx, Path::new(path), &tagged).expect("index");
        tx.commit().expect("commit");
    }

    fn ids(conn: &Connection, tag: &str) -> Vec<(String, usize, String)> {
        find(conn, tag)
            .expect("find")
            .into_iter()
            .map(|found| {
                (
                    found.path.to_string_lossy().into_owned(),
                    found.page,
                    found.annotation,
                )
            })
            .collect()
    }

    #[test]
    fn a_tag_finds_tagged_annotations_across_documents() {
        let mut conn = database();
        write(
            &mut conn,
            "C:/papers/a.pdf",
            &[("h1", 4, &["Method", "to read"]), ("n1", 1, &["method"])],
        );
        write(&mut conn, "C:/papers/b.pdf", &[("h9", 0, &["method"])]);
        assert_eq!(
            ids(&conn, "METHOD"),
            [
                ("C:/papers/a.pdf".to_owned(), 1, "n1".to_owned()),
                ("C:/papers/a.pdf".to_owned(), 4, "h1".to_owned()),
                ("C:/papers/b.pdf".to_owned(), 0, "h9".to_owned()),
            ],
            "any case, by file then page"
        );
        // Saved again, a.pdf's tags are what it holds now.
        write(&mut conn, "C:/papers/a.pdf", &[("h1", 4, &["to read"])]);
        assert_eq!(
            ids(&conn, "method"),
            [("C:/papers/b.pdf".to_owned(), 0, "h9".to_owned())]
        );
        assert_eq!(ids(&conn, " to read ").len(), 1);
        assert!(ids(&conn, "unknown").is_empty());
    }
}
