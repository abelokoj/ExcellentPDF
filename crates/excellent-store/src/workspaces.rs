//! The set of workspaces (plan §11.16; W.1 to W.7): their names, icons, colours and order, and the one each window
//! shows. What a workspace holds (its panes and tabs) is [`crate::workspace`]'s.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Result;
use crate::workspace::WorkspaceId;

/// `meta`'s key for the workspace the window showed last (W.7). shortcut: one window; W.8's windows each get a key.
const SHOWN_KEY: &str = "workspace.shown";

/// A workspace as the switcher lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Its row.
    pub id: WorkspaceId,
    /// Its name; empty for the first workspace until the reader names it (the app shows its default name).
    pub name: String,
    /// Its icon: an emoji or a symbol, as the reader typed it.
    pub icon: Option<String>,
    /// Its colour, as the app names it.
    pub color: Option<String>,
    /// How many tabs it has open, in every pane.
    pub tabs: usize,
}

/// What a workspace looks like: its name, icon and colour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    /// Its name.
    pub name: String,
    /// Its icon, if any.
    pub icon: Option<String>,
    /// Its colour, if any.
    pub color: Option<String>,
}

/// Every workspace, in the reader's order. There is always one.
///
/// # Errors
///
/// SQLite failed.
pub fn list(conn: &Connection) -> Result<Vec<Workspace>> {
    let mut query = conn.prepare(
        "SELECT w.id, w.name, w.icon, w.color, (SELECT COUNT(*) FROM workspace_tab t WHERE t.workspace = w.id)
         FROM workspace w ORDER BY w.position, w.id",
    )?;
    let rows = query.query_map([], |row| {
        Ok(Workspace {
            id: WorkspaceId(row.get(0)?),
            name: row.get(1)?,
            icon: row.get(2)?,
            color: row.get(3)?,
            tabs: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Adds a workspace with no tabs after the others.
///
/// # Errors
///
/// SQLite failed.
pub fn create(tx: &Transaction<'_>, look: &Look) -> Result<WorkspaceId> {
    let id = tx.query_row(
        "INSERT INTO workspace (name, icon, color, position)
         VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), -1) + 1 FROM workspace)) RETURNING id",
        params![look.name, look.icon, look.color],
        |row| row.get(0),
    )?;
    Ok(WorkspaceId(id))
}

/// Gives `workspace` a new name, icon and colour.
///
/// # Errors
///
/// SQLite failed.
pub fn restyle(tx: &Transaction<'_>, workspace: WorkspaceId, look: &Look) -> Result<()> {
    tx.execute(
        "UPDATE workspace SET name = ?2, icon = ?3, color = ?4 WHERE id = ?1",
        params![workspace.0, look.name, look.icon, look.color],
    )?;
    Ok(())
}

/// Puts the workspaces in the order of `order`; any left out keep their places after them.
///
/// # Errors
///
/// SQLite failed.
pub fn reorder(tx: &Transaction<'_>, order: &[WorkspaceId]) -> Result<()> {
    let mut update = tx.prepare("UPDATE workspace SET position = ?2 WHERE id = ?1")?;
    for (position, id) in (0_i64..).zip(order) {
        update.execute(params![id.0, position])?;
    }
    Ok(())
}

/// Deletes `workspace` with its tabs. The last workspace is never deleted: `false` then.
///
/// # Errors
///
/// SQLite failed.
pub fn delete(tx: &Transaction<'_>, workspace: WorkspaceId) -> Result<bool> {
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM workspace", [], |row| row.get(0))?;
    if count <= 1 {
        return Ok(false);
    }
    Ok(tx.execute("DELETE FROM workspace WHERE id = ?1", [workspace.0])? > 0)
}

/// The workspace the window showed last, if it is still there (W.7).
///
/// # Errors
///
/// SQLite failed.
pub fn shown(conn: &Connection) -> Result<Option<WorkspaceId>> {
    Ok(conn
        .query_row(
            "SELECT w.id FROM meta m JOIN workspace w ON w.id = CAST(m.value AS INTEGER) WHERE m.key = ?1",
            [SHOWN_KEY],
            |row| row.get(0),
        )
        .optional()?
        .map(WorkspaceId))
}

/// Notes that the window shows `workspace`.
///
/// # Errors
///
/// SQLite failed.
pub fn set_shown(tx: &Transaction<'_>, workspace: WorkspaceId) -> Result<()> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![SHOWN_KEY, workspace.0.to_string()],
    )?;
    Ok(())
}

/// The first workspace other than `except` with an open tab of the file at `path` (W.6). Paths are compared as
/// written, ignoring case. shortcut: matches the path, not the file's identity (K.5); moved files aren't found.
///
/// # Errors
///
/// SQLite failed.
pub fn holding(conn: &Connection, path: &Path, except: WorkspaceId) -> Result<Option<WorkspaceId>> {
    Ok(conn
        .query_row(
            "SELECT w.id FROM workspace w JOIN workspace_tab t ON t.workspace = w.id JOIN file f ON f.id = t.file
             WHERE lower(f.path) = lower(?1) AND w.id <> ?2 ORDER BY w.position, w.id LIMIT 1",
            params![path.to_string_lossy(), except.0],
            |row| row.get(0),
        )
        .optional()?
        .map(WorkspaceId))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::migrations;
    use crate::workspace::{self, Panes, SavedTab, Tabs};

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("foreign keys");
        migrations::migrate(&mut conn).expect("migrate");
        conn
    }

    fn change<T>(conn: &mut Connection, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> T {
        let tx = conn.transaction().expect("transaction");
        let result = f(&tx).expect("change");
        tx.commit().expect("commit");
        result
    }

    fn look(name: &str) -> Look {
        Look {
            name: name.to_owned(),
            icon: Some("📚".to_owned()),
            color: Some("blue".to_owned()),
        }
    }

    fn one_tab(path: &str) -> Panes {
        Panes {
            layout: None,
            panes: vec![Tabs {
                open: vec![SavedTab {
                    path: PathBuf::from(path),
                    view: "{\"page\":0}".to_owned(),
                    pinned: false,
                    group: None,
                }],
                active: Some(0),
                ..Tabs::default()
            }],
        }
    }

    #[test]
    fn workspaces_are_made_listed_in_order_restyled_and_deleted() {
        let mut conn = database();
        let thesis = change(&mut conn, |tx| create(tx, &look("Thesis")));
        let tax = change(&mut conn, |tx| create(tx, &look("Tax")));
        change(&mut conn, |tx| {
            workspace::save_panes(tx, thesis, &one_tab("C:/a.pdf"))
        });
        let names = |conn: &Connection| -> Vec<(String, usize)> {
            list(conn)
                .expect("list")
                .into_iter()
                .map(|w| (w.name, w.tabs))
                .collect()
        };
        assert_eq!(
            names(&conn),
            [
                ("".to_owned(), 0),
                ("Thesis".to_owned(), 1),
                ("Tax".to_owned(), 0)
            ]
        );
        change(&mut conn, |tx| {
            reorder(tx, &[tax, WorkspaceId::DEFAULT, thesis])
        });
        change(&mut conn, |tx| restyle(tx, tax, &look("Taxes")));
        assert_eq!(
            names(&conn),
            [
                ("Taxes".to_owned(), 0),
                ("".to_owned(), 0),
                ("Thesis".to_owned(), 1)
            ]
        );
        assert!(change(&mut conn, |tx| delete(tx, thesis)));
        assert!(change(&mut conn, |tx| delete(tx, tax)));
        assert!(
            !change(&mut conn, |tx| delete(tx, WorkspaceId::DEFAULT)),
            "the last one stays"
        );
        assert_eq!(names(&conn), [("".to_owned(), 0)]);
    }

    #[test]
    fn the_shown_workspace_is_remembered_until_it_goes() {
        let mut conn = database();
        assert_eq!(shown(&conn).expect("shown"), None);
        let other = change(&mut conn, |tx| create(tx, &look("Other")));
        change(&mut conn, |tx| set_shown(tx, other));
        assert_eq!(shown(&conn).expect("shown"), Some(other));
        change(&mut conn, |tx| delete(tx, other));
        assert_eq!(shown(&conn).expect("shown"), None);
    }

    #[test]
    fn a_file_open_in_another_workspace_is_found_there() {
        let mut conn = database();
        let other = change(&mut conn, |tx| create(tx, &look("Other")));
        change(&mut conn, |tx| {
            workspace::save_panes(tx, other, &one_tab("C:/Docs/a.pdf"))
        });
        let find = |path: &str, except| holding(&conn, Path::new(path), except).expect("find");
        assert_eq!(find("c:/docs/A.pdf", WorkspaceId::DEFAULT), Some(other));
        assert_eq!(find("C:/Docs/a.pdf", other), None);
        assert_eq!(find("C:/Docs/b.pdf", WorkspaceId::DEFAULT), None);
    }
}
