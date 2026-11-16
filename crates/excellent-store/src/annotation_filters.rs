//! Each file's annotation filter (plan A.19): the colours, categories, tags, authors and dates its Annotations tab
//! was showing, kept as the app's own JSON so the filter is there again when the file is opened.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::Result;
use crate::workspace::file_id;

/// Keeps `filter` as the annotation filter of the file at `path`; an empty `filter` forgets it.
///
/// # Errors
///
/// SQLite failed.
pub fn save(tx: &Transaction<'_>, path: &Path, filter: &str) -> Result<()> {
    let file = file_id(tx, path)?;
    if filter.is_empty() {
        tx.execute("DELETE FROM annotation_filter WHERE file = ?1", [file])?;
    } else {
        tx.execute(
            "INSERT INTO annotation_filter (file, filter) VALUES (?1, ?2)
             ON CONFLICT (file) DO UPDATE SET filter = excluded.filter",
            rusqlite::params![file, filter],
        )?;
    }
    Ok(())
}

/// The annotation filter kept for the file at `path`, if any.
///
/// # Errors
///
/// SQLite failed.
pub fn load(conn: &Connection, path: &Path) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT a.filter FROM annotation_filter a JOIN file f ON f.id = a.file WHERE f.path = ?1",
            [path.to_string_lossy()],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn save_now(conn: &mut Connection, path: &str, filter: &str) {
        let tx = conn.transaction().expect("transaction");
        save(&tx, Path::new(path), filter).expect("save");
        tx.commit().expect("commit");
    }

    #[test]
    fn a_filter_is_kept_per_file() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("foreign keys");
        migrations::migrate(&mut conn).expect("migrate");
        let (a, b) = (Path::new("C:/papers/a.pdf"), Path::new("C:/papers/b.pdf"));
        assert_eq!(load(&conn, a).expect("load"), None);
        save_now(&mut conn, "C:/papers/a.pdf", r#"{"show_only":true}"#);
        save_now(&mut conn, "C:/papers/b.pdf", r#"{"query":"x"}"#);
        save_now(&mut conn, "C:/papers/a.pdf", r#"{"query":"y"}"#);
        assert_eq!(
            load(&conn, a).expect("load").as_deref(),
            Some(r#"{"query":"y"}"#)
        );
        assert_eq!(
            load(&conn, b).expect("load").as_deref(),
            Some(r#"{"query":"x"}"#)
        );
        save_now(&mut conn, "C:/papers/a.pdf", "");
        assert_eq!(load(&conn, a).expect("load"), None, "forgotten");
    }
}
