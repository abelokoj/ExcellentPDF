//! A workspace's tabs (plan §11.16, "Storage"): the open tabs in strip order with the one shown, their named groups
//! (R.13), and the recently closed ones. A tab is its file and the app's View state, which this crate keeps as the app's JSON without
//! reading it. The tabs are kept per pane of the split view (V.7, [`Panes`]), with the panes' layout, also the app's
//! JSON.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Result;

/// The most closed tabs a workspace keeps (§11.16).
pub const CLOSED_KEPT: usize = 50;

/// A workspace, by its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspaceId(i64);

impl WorkspaceId {
    /// The first workspace, which every database has from its first version.
    pub const DEFAULT: Self = Self(1);
}

/// A tab as it is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTab {
    /// Its file.
    pub path: PathBuf,
    /// Its View state, as the app wrote it.
    pub view: String,
    /// Whether it is pinned.
    pub pinned: bool,
    /// Its group: a place in [`Tabs::groups`]. Closed tabs keep none.
    pub group: Option<usize>,
}

/// A named tab group as it is saved (R.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedGroup {
    /// Its name; may be empty.
    pub name: String,
    /// Its colour, as the app names it.
    pub color: String,
    /// Whether its tabs are hidden.
    pub collapsed: bool,
}

/// A recently closed tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTab {
    /// The tab.
    pub tab: SavedTab,
    /// Its place in the strip when it closed.
    pub position: usize,
}

/// A workspace's tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tabs {
    /// The open tabs, in strip order.
    pub open: Vec<SavedTab>,
    /// The place in `open` of the tab shown.
    pub active: Option<usize>,
    /// The groups the open tabs name.
    pub groups: Vec<SavedGroup>,
    /// The closed tabs, newest first; only the first [`CLOSED_KEPT`] are saved.
    pub closed: Vec<ClosedTab>,
}

/// A workspace's panes (V.7): the layout the app saved, and each pane's tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panes {
    /// The panes' arrangement, as the app wrote it; none until it saves one.
    pub layout: Option<String>,
    /// Each cell's tabs, by cell number (a closed cell's are empty); a session from before the split view has one.
    pub panes: Vec<Tabs>,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            layout: None,
            panes: vec![Tabs::default()],
        }
    }
}

/// Reads `workspace`'s panes and their tabs; a workspace that isn't there has one pane without tabs.
///
/// # Errors
///
/// SQLite failed.
pub fn load_panes(conn: &Connection, workspace: WorkspaceId) -> Result<Panes> {
    let layout: Option<String> = conn
        .query_row(
            "SELECT layout FROM workspace WHERE id = ?1",
            [workspace.0],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(pane) FROM (SELECT pane FROM workspace_pane WHERE workspace = ?1
         UNION ALL SELECT pane FROM workspace_tab WHERE workspace = ?1)",
        [workspace.0],
        |row| row.get(0),
    )?;
    let panes = (0..=last.unwrap_or(0).max(0))
        .map(|pane| load_pane(conn, workspace, pane))
        .collect::<Result<_>>()?;
    Ok(Panes { layout, panes })
}

/// Replaces `workspace`'s saved panes and tabs with `panes`.
///
/// shortcut: rewrites every row of the workspace, a few milliseconds for a hundred tabs; write only the changed
/// rows if workspaces of thousands of tabs make saving slow.
///
/// # Errors
///
/// SQLite failed; the caller's transaction then rolls back, keeping what was saved before.
pub fn save_panes(tx: &Transaction<'_>, workspace: WorkspaceId, panes: &Panes) -> Result<()> {
    tx.execute(
        "UPDATE workspace SET layout = ?2 WHERE id = ?1",
        params![workspace.0, panes.layout],
    )?;
    for table in ["workspace_pane", "workspace_tab", "closed_tab", "tab_group"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE workspace = ?1"),
            [workspace.0],
        )?;
    }
    for (pane, tabs) in (0_i64..).zip(&panes.panes) {
        save_pane(tx, workspace, pane, tabs)?;
    }
    Ok(())
}

/// Reads the tabs of `workspace`'s pane `pane`.
fn load_pane(conn: &Connection, workspace: WorkspaceId, pane: i64) -> Result<Tabs> {
    let active: Option<usize> = conn
        .query_row(
            "SELECT active_tab FROM workspace_pane WHERE workspace = ?1 AND pane = ?2",
            params![workspace.0, pane],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let mut group_rows = conn.prepare(
        "SELECT id, name, color, collapsed FROM tab_group WHERE workspace = ?1 AND pane = ?2 ORDER BY position",
    )?;
    let mut place_of = HashMap::new();
    let mut groups = Vec::new();
    for row in group_rows.query_map(params![workspace.0, pane], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            SavedGroup {
                name: row.get(1)?,
                color: row.get(2)?,
                collapsed: row.get(3)?,
            },
        ))
    })? {
        let (id, group) = row?;
        place_of.insert(id, groups.len());
        groups.push(group);
    }
    let mut open_tabs = conn.prepare(
        "SELECT f.path, t.view, t.pinned, t.tab_group FROM workspace_tab t JOIN file f ON f.id = t.file
         WHERE t.workspace = ?1 AND t.pane = ?2 ORDER BY t.position",
    )?;
    let open: Vec<SavedTab> = open_tabs
        .query_map(params![workspace.0, pane], |row| {
            Ok(SavedTab {
                path: PathBuf::from(row.get::<_, String>(0)?),
                view: row.get(1)?,
                pinned: row.get(2)?,
                group: row
                    .get::<_, Option<i64>>(3)?
                    .and_then(|id| place_of.get(&id).copied()),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut closed_tabs = conn.prepare(
        "SELECT f.path, c.view, c.pinned, c.position FROM closed_tab c JOIN file f ON f.id = c.file
         WHERE c.workspace = ?1 AND c.pane = ?2 ORDER BY c.rank",
    )?;
    let closed = closed_tabs
        .query_map(params![workspace.0, pane], |row| {
            Ok(ClosedTab {
                tab: SavedTab {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    view: row.get(1)?,
                    pinned: row.get(2)?,
                    group: None,
                },
                position: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Tabs {
        active: active.filter(|&index| index < open.len()),
        open,
        groups,
        closed,
    })
}

/// Writes the tabs of `workspace`'s pane `pane`, whose rows [`save_panes`] has just deleted.
fn save_pane(tx: &Transaction<'_>, workspace: WorkspaceId, pane: i64, tabs: &Tabs) -> Result<()> {
    tx.execute(
        "INSERT INTO workspace_pane (workspace, pane, active_tab) VALUES (?1, ?2, ?3)",
        params![workspace.0, pane, tabs.active],
    )?;
    let mut insert_group = tx.prepare(
        "INSERT INTO tab_group (workspace, pane, position, name, color, collapsed) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         RETURNING id",
    )?;
    let group_ids = tabs
        .groups
        .iter()
        .enumerate()
        .map(|(position, group)| {
            insert_group.query_row(
                params![
                    workspace.0,
                    pane,
                    position,
                    group.name,
                    group.color,
                    group.collapsed
                ],
                |row| row.get::<_, i64>(0),
            )
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut insert_open = tx.prepare(
        "INSERT INTO workspace_tab (workspace, pane, position, file, view, pinned, tab_group)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (position, tab) in tabs.open.iter().enumerate() {
        let file = file_id(tx, &tab.path)?;
        let group = tab.group.and_then(|place| group_ids.get(place));
        insert_open.execute(params![
            workspace.0,
            pane,
            position,
            file,
            tab.view,
            tab.pinned,
            group
        ])?;
    }
    let mut insert_closed = tx.prepare(
        "INSERT INTO closed_tab (workspace, pane, rank, position, file, view, pinned)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (rank, closed) in tabs.closed.iter().take(CLOSED_KEPT).enumerate() {
        let file = file_id(tx, &closed.tab.path)?;
        insert_closed.execute(params![
            workspace.0,
            pane,
            rank,
            closed.position,
            file,
            closed.tab.view,
            closed.tab.pinned
        ])?;
    }
    Ok(())
}

/// The row of the file at `path`, made if it isn't there.
///
/// shortcut: a path that isn't valid Unicode is saved with its odd characters replaced, so that tab won't reopen;
/// store the platform's bytes if such names turn up.
pub(crate) fn file_id(tx: &Transaction<'_>, path: &std::path::Path) -> Result<i64> {
    Ok(tx.query_row(
        "INSERT INTO file (path) VALUES (?1) ON CONFLICT (path) DO UPDATE SET path = excluded.path RETURNING id",
        [path.to_string_lossy()],
        |row| row.get(0),
    )?)
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

    fn tab(name: &str, page: usize, pinned: bool) -> SavedTab {
        SavedTab {
            path: PathBuf::from(format!("C:/papers/{name}.pdf")),
            view: format!("{{\"page\":{page}}}"),
            pinned,
            group: None,
        }
    }

    fn grouped(name: &str, group: usize) -> SavedTab {
        SavedTab {
            group: Some(group),
            ..tab(name, 0, false)
        }
    }

    fn save_all(conn: &mut Connection, panes: &Panes) {
        let tx = conn.transaction().expect("transaction");
        save_panes(&tx, WorkspaceId::DEFAULT, panes).expect("save");
        tx.commit().expect("commit");
    }

    /// Saves `tabs` as the one pane.
    fn save(conn: &mut Connection, tabs: &Tabs) {
        save_all(
            conn,
            &Panes {
                layout: None,
                panes: vec![tabs.clone()],
            },
        );
    }

    /// The first pane's tabs.
    fn load_tabs(conn: &Connection, workspace: WorkspaceId) -> Result<Tabs> {
        let mut panes = load_panes(conn, workspace)?.panes;
        assert!(!panes.is_empty(), "there is always a pane");
        Ok(panes.swap_remove(0))
    }

    #[test]
    fn each_pane_keeps_its_own_tabs_and_the_layout() {
        let mut conn = database();
        assert_eq!(
            load_panes(&conn, WorkspaceId::DEFAULT).expect("load"),
            Panes::default()
        );
        let panes = Panes {
            layout: Some(r#"{"arrangement":"rows"}"#.to_owned()),
            panes: vec![
                Tabs {
                    open: vec![tab("a", 3, true), grouped("b", 0)],
                    active: Some(1),
                    groups: vec![SavedGroup {
                        name: "Thesis".to_owned(),
                        color: "blue".to_owned(),
                        collapsed: false,
                    }],
                    closed: vec![ClosedTab {
                        tab: tab("c", 4, false),
                        position: 0,
                    }],
                },
                Tabs::default(),
                Tabs {
                    open: vec![tab("a", 8, false)],
                    active: Some(0),
                    ..Tabs::default()
                },
            ],
        };
        save_all(&mut conn, &panes);
        assert_eq!(
            load_panes(&conn, WorkspaceId::DEFAULT).expect("load"),
            panes
        );
        // Back to one pane: the others' rows go.
        save(&mut conn, &panes.panes[2]);
        let one = load_panes(&conn, WorkspaceId::DEFAULT).expect("load");
        assert_eq!(
            (one.layout, one.panes),
            (None, vec![panes.panes[2].clone()])
        );
    }

    #[test]
    fn tabs_come_back_as_they_were_saved() {
        let mut conn = database();
        assert_eq!(
            load_tabs(&conn, WorkspaceId::DEFAULT).expect("load"),
            Tabs::default()
        );
        let tabs = Tabs {
            open: vec![
                tab("a", 3, true),
                grouped("b", 0),
                grouped("e", 0),
                grouped("f", 1),
                tab("a", 9, false),
            ],
            active: Some(4),
            groups: vec![
                SavedGroup {
                    name: "Thesis".to_owned(),
                    color: "blue".to_owned(),
                    collapsed: false,
                },
                SavedGroup {
                    name: String::new(),
                    color: "red".to_owned(),
                    collapsed: true,
                },
            ],
            closed: vec![ClosedTab {
                tab: tab("c", 4, false),
                position: 1,
            }],
        };
        save(&mut conn, &tabs);
        assert_eq!(load_tabs(&conn, WorkspaceId::DEFAULT).expect("load"), tabs);
        let fewer = Tabs {
            open: vec![tab("b", 1, false)],
            active: Some(0),
            ..Tabs::default()
        };
        save(&mut conn, &fewer);
        assert_eq!(load_tabs(&conn, WorkspaceId::DEFAULT).expect("load"), fewer);
    }

    #[test]
    fn only_the_newest_closed_tabs_are_kept() {
        let mut conn = database();
        let closed: Vec<ClosedTab> = (0..CLOSED_KEPT + 7)
            .map(|n| ClosedTab {
                tab: tab(&n.to_string(), n, false),
                position: 0,
            })
            .collect();
        save(
            &mut conn,
            &Tabs {
                closed: closed.clone(),
                ..Tabs::default()
            },
        );
        let loaded = load_tabs(&conn, WorkspaceId::DEFAULT).expect("load");
        assert_eq!(loaded.closed, closed[..CLOSED_KEPT]);
    }

    #[test]
    fn a_version_1_session_keeps_its_tabs_through_the_upgrade() {
        let mut conn = Connection::open_in_memory().expect("open");
        migrations::migrate_to(&mut conn, 1).expect("version 1");
        conn.execute_batch(
            "INSERT INTO file (id, path) VALUES (1, 'C:/papers/a.pdf');
             INSERT INTO workspace_tab (workspace, position, file, view, pinned) VALUES (1, 0, 1, '{}', 1);
             UPDATE workspace SET active_tab = 0;",
        )
        .expect("a version 1 session");
        migrations::migrate(&mut conn).expect("upgrade");
        let tabs = load_tabs(&conn, WorkspaceId::DEFAULT).expect("load");
        assert_eq!(
            tabs,
            Tabs {
                open: vec![SavedTab {
                    path: PathBuf::from("C:/papers/a.pdf"),
                    view: "{}".to_owned(),
                    pinned: true,
                    group: None,
                }],
                active: Some(0),
                ..Tabs::default()
            }
        );
    }

    #[test]
    fn an_active_tab_past_the_end_is_none() {
        let mut conn = database();
        save(
            &mut conn,
            &Tabs {
                open: vec![tab("a", 0, false)],
                active: Some(0),
                ..Tabs::default()
            },
        );
        conn.execute("UPDATE workspace_pane SET active_tab = 5", [])
            .expect("corrupt");
        assert_eq!(
            load_tabs(&conn, WorkspaceId::DEFAULT).expect("load").active,
            None
        );
    }
}
