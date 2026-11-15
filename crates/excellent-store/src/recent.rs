//! Recent and pinned files (plan R.10): every file the reader has shown, when, and the page they left it at. Pinned
//! files stay until unpinned; of the others the newest [`RECENT_KEPT`] are kept.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction, params};

use crate::Result;
use crate::workspace::file_id;

/// The most unpinned files the list keeps.
pub const RECENT_KEPT: usize = 500;

/// A file of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFile {
    /// The file.
    pub path: PathBuf,
    /// The page it was left at, from 0.
    pub page: usize,
    /// When it was last shown, in seconds since 1970.
    pub seen: i64,
    /// It stays on the list, at the top.
    pub pinned: bool,
}

/// The list: pinned files first, then the most recently shown.
///
/// # Errors
///
/// SQLite failed.
pub fn load(conn: &Connection) -> Result<Vec<RecentFile>> {
    let mut rows = conn.prepare(
        "SELECT f.path, r.page, r.seen, r.pinned FROM recent r JOIN file f ON f.id = r.file
         ORDER BY r.pinned DESC, r.seen DESC, r.file DESC",
    )?;
    let files = rows
        .query_map([], |row| {
            Ok(RecentFile {
                path: PathBuf::from(row.get::<_, String>(0)?),
                page: row.get(1)?,
                seen: row.get(2)?,
                pinned: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(files)
}

/// Notes that `path` was shown at `seen`, at `page`; the oldest unpinned files past [`RECENT_KEPT`] go.
///
/// # Errors
///
/// SQLite failed.
pub fn visit(tx: &Transaction<'_>, path: &Path, page: usize, seen: i64) -> Result<()> {
    let file = file_id(tx, path)?;
    tx.execute(
        "INSERT INTO recent (file, seen, page) VALUES (?1, ?2, ?3)
         ON CONFLICT (file) DO UPDATE SET seen = excluded.seen, page = excluded.page",
        params![file, seen, page],
    )?;
    tx.execute(
        "DELETE FROM recent WHERE pinned = 0 AND file NOT IN
         (SELECT file FROM recent WHERE pinned = 0 ORDER BY seen DESC, file DESC LIMIT ?1)",
        [RECENT_KEPT],
    )?;
    Ok(())
}

/// Pins or unpins `path`, if it is on the list.
///
/// # Errors
///
/// SQLite failed.
pub fn set_pinned(tx: &Transaction<'_>, path: &Path, pinned: bool) -> Result<()> {
    tx.execute(
        "UPDATE recent SET pinned = ?2 WHERE file = (SELECT id FROM file WHERE path = ?1)",
        params![path.to_string_lossy(), pinned],
    )?;
    Ok(())
}

/// Takes `path` off the list, pinned or not.
///
/// # Errors
///
/// SQLite failed.
pub fn remove(tx: &Transaction<'_>, path: &Path) -> Result<()> {
    tx.execute(
        "DELETE FROM recent WHERE file = (SELECT id FROM file WHERE path = ?1)",
        [path.to_string_lossy()],
    )?;
    Ok(())
}

/// Takes every unpinned file off the list.
///
/// # Errors
///
/// SQLite failed.
pub fn clear(tx: &Transaction<'_>) -> Result<()> {
    tx.execute("DELETE FROM recent WHERE pinned = 0", [])?;
    Ok(())
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

    fn change(conn: &mut Connection, f: impl FnOnce(&Transaction<'_>) -> Result<()>) {
        let tx = conn.transaction().expect("transaction");
        f(&tx).expect("change");
        tx.commit().expect("commit");
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("C:/papers/{name}.pdf"))
    }

    fn names(conn: &Connection) -> Vec<String> {
        load(conn)
            .expect("load")
            .into_iter()
            .map(|file| {
                let name = file
                    .path
                    .file_stem()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned();
                format!("{name}{}{}", file.page, if file.pinned { "*" } else { "" })
            })
            .collect()
    }

    #[test]
    fn the_list_puts_pinned_files_first_then_the_newest() {
        let mut conn = database();
        change(&mut conn, |tx| visit(tx, &path("a"), 3, 10));
        change(&mut conn, |tx| visit(tx, &path("b"), 0, 20));
        change(&mut conn, |tx| visit(tx, &path("c"), 7, 30));
        assert_eq!(names(&conn), ["c7", "b0", "a3"]);
        // Shown again: newest, at its new page.
        change(&mut conn, |tx| visit(tx, &path("a"), 5, 40));
        change(&mut conn, |tx| set_pinned(tx, &path("b"), true));
        assert_eq!(names(&conn), ["b0*", "a5", "c7"]);
        change(&mut conn, |tx| remove(tx, &path("a")));
        change(&mut conn, clear);
        assert_eq!(names(&conn), ["b0*"]);
    }

    #[test]
    fn only_the_newest_unpinned_files_are_kept() {
        let mut conn = database();
        change(&mut conn, |tx| visit(tx, &path("pinned"), 0, 0));
        change(&mut conn, |tx| set_pinned(tx, &path("pinned"), true));
        change(&mut conn, |tx| {
            for n in 0..RECENT_KEPT + 3 {
                let seen = i64::try_from(n).expect("small") + 1;
                visit(tx, &path(&n.to_string()), 0, seen)?;
            }
            Ok(())
        });
        let files = load(&conn).expect("load");
        assert_eq!(files.len(), RECENT_KEPT + 1);
        assert!(files[0].pinned);
        assert_eq!(files[1].path, path(&(RECENT_KEPT + 2).to_string()));
        assert!(!files.iter().any(|file| file.path == path("2")));
    }
}
