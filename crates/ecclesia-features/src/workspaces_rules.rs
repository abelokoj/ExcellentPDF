//! Workspace <-> Library Collection bi-directional conversion (plan W.10) and
//! workspace templates + path-based auto-routing rules (plan W.12).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ecclesia_store::library;
use ecclesia_store::workspace::{self, Panes, SavedTab, Tabs, WorkspaceId};
use ecclesia_store::workspaces::{self, Look};
use ecclesia_store::{Result as StoreResult, Store};
use rusqlite::{Connection, Transaction, params};

/// Opens a library collection as a new workspace, preserving exact file order (plan W.10).
///
/// # Errors
///
/// Returns [`ecclesia_store::Error`] if the collection does not exist or SQLite fails.
pub fn open_collection_as_workspace(
    store: &Store,
    collection_id: i64,
    look_override: Option<Look>,
) -> StoreResult<Option<WorkspaceId>> {
    store
        .write(move |tx| {
            let Some(collection) = library::collection(tx, collection_id)? else {
                return Ok(None);
            };
            let paths = library::collection_items(tx, collection_id)?;
            let look = look_override.unwrap_or(Look {
                name: collection.name,
                icon: collection.icon,
                color: collection.color,
            });
            let ws_id = workspaces::create(tx, &look)?;
            let open: Vec<SavedTab> = paths
                .into_iter()
                .map(|path| SavedTab {
                    path,
                    view: "{\"page\":0}".to_owned(),
                    pinned: false,
                    group: None,
                })
                .collect();
            let active = (!open.is_empty()).then_some(0);
            let panes = Panes {
                layout: None,
                panes: vec![Tabs {
                    open,
                    active,
                    ..Tabs::default()
                }],
            };
            workspace::save_panes(tx, ws_id, &panes)?;
            Ok(Some(ws_id))
        })
        .wait()
}

/// Saves a workspace's open tabs (across panes, in strip order) as a library collection,
/// preserving file order and deduplicating repeated tabs (plan W.10).
///
/// # Errors
///
/// Returns [`ecclesia_store::Error`] if the workspace does not exist or SQLite fails.
pub fn save_workspace_as_collection(
    store: &Store,
    workspace_id: WorkspaceId,
    parent_collection: Option<i64>,
    name_override: Option<&str>,
) -> StoreResult<Option<i64>> {
    let name_override = name_override.map(str::to_owned);
    store
        .write(move |tx| {
            let list = workspaces::list(tx)?;
            let Some(ws) = list.into_iter().find(|w| w.id == workspace_id) else {
                return Ok(None);
            };
            let panes = workspace::load_panes(tx, workspace_id)?;
            let mut seen = HashSet::new();
            let mut ordered_paths = Vec::new();
            for pane in panes.panes {
                for tab in pane.open {
                    if seen.insert(tab.path.clone()) {
                        ordered_paths.push(tab.path);
                    }
                }
            }
            let name = name_override.unwrap_or_else(|| {
                if ws.name.trim().is_empty() {
                    "Workspace".to_owned()
                } else {
                    ws.name
                }
            });
            let col_id = library::create_collection(
                tx,
                parent_collection,
                &name,
                ws.icon.as_deref(),
                ws.color.as_deref(),
            )?;
            library::set_collection_items(tx, col_id, &ordered_paths)?;
            Ok(Some(col_id))
        })
        .wait()
}

/// One tab slot inside a [`WorkspaceTemplate`] (plan W.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateItem {
    /// Display role / title (e.g. `"Syllabus"`, `"Textbook"`, `"Notebook"`).
    pub title: String,
    /// Optional concrete file path or default template path.
    pub path: PathBuf,
    /// Initial view JSON (`{"page":0}`).
    pub view: String,
    /// Whether the tab starts pinned.
    pub pinned: bool,
}

/// Reusable workspace template (plan W.12, e.g. `"Course"` with syllabus, textbook, notebook).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTemplate {
    /// Stable identifier.
    pub id: String,
    /// Template name.
    pub name: String,
    /// Optional icon.
    pub icon: Option<String>,
    /// Optional color.
    pub color: Option<String>,
    /// Optional split-pane layout JSON.
    pub layout: Option<String>,
    /// Ordered template items.
    pub items: Vec<TemplateItem>,
    /// Creation timestamp (UTC seconds since 1970).
    pub created_at: i64,
}

impl WorkspaceTemplate {
    /// Built-in `"Course"` template preset: syllabus, textbook, and notebook (plan W.12).
    #[must_use]
    pub fn course_preset(base_dir: &Path) -> Self {
        Self {
            id: "preset-course".to_owned(),
            name: "Course".to_owned(),
            icon: Some("🎓".to_owned()),
            color: Some("blue".to_owned()),
            layout: None,
            items: vec![
                TemplateItem {
                    title: "Syllabus".to_owned(),
                    path: base_dir.join("syllabus.pdf"),
                    view: "{\"page\":0}".to_owned(),
                    pinned: true,
                },
                TemplateItem {
                    title: "Textbook".to_owned(),
                    path: base_dir.join("textbook.pdf"),
                    view: "{\"page\":0}".to_owned(),
                    pinned: false,
                },
                TemplateItem {
                    title: "Notebook".to_owned(),
                    path: base_dir.join("notebook.expdf"),
                    view: "{\"page\":0}".to_owned(),
                    pinned: false,
                },
            ],
            created_at: 0,
        }
    }
}

/// Path-based auto-routing rule: files opened from `folder_prefix` open in the target workspace (plan W.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingRule {
    /// Stable rule id.
    pub id: String,
    /// Human-readable rule name.
    pub name: String,
    /// Folder prefix to match (case-insensitive, separator-normalized).
    pub folder_prefix: PathBuf,
    /// Target workspace id, if pinned to a specific existing workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// Fallback or target workspace name (matched if `workspace_id` is `None`).
    pub workspace_name: String,
    /// Higher priority rules win when multiple prefixes match.
    pub priority: i32,
    /// Whether this rule is active.
    pub enabled: bool,
}

/// Saves or updates a [`WorkspaceTemplate`] in `library.db` (plan W.12).
///
/// # Errors
///
/// SQLite failed.
pub fn save_template(
    tx: &Transaction<'_>,
    template: &WorkspaceTemplate,
) -> ecclesia_store::Result<()> {
    let items_json = serde_json::to_string(
        &template
            .items
            .iter()
            .map(|it| {
                serde_json::json!({
                    "title": it.title,
                    "path": it.path.to_string_lossy(),
                    "view": it.view,
                    "pinned": it.pinned,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_owned());

    tx.execute(
        "INSERT INTO workspace_template (id, name, icon, color, layout, items_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            icon = excluded.icon,
            color = excluded.color,
            layout = excluded.layout,
            items_json = excluded.items_json",
        params![
            template.id,
            template.name,
            template.icon,
            template.color,
            template.layout,
            items_json,
            template.created_at,
        ],
    )?;
    Ok(())
}

/// Lists all saved workspace templates.
///
/// # Errors
///
/// SQLite failed.
pub fn list_templates(conn: &Connection) -> ecclesia_store::Result<Vec<WorkspaceTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, icon, color, layout, items_json, created_at
         FROM workspace_template ORDER BY created_at ASC, name COLLATE NOCASE ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let items_json: String = row.get(5)?;
            let items = parse_template_items(&items_json);
            Ok(WorkspaceTemplate {
                id: row.get(0)?,
                name: row.get(1)?,
                icon: row.get(2)?,
                color: row.get(3)?,
                layout: row.get(4)?,
                items,
                created_at: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Instantiates a [`WorkspaceTemplate`] as a real workspace in `store`, returning its [`WorkspaceId`].
///
/// # Errors
///
/// SQLite failed.
pub fn instantiate_template(
    store: &Store,
    template: &WorkspaceTemplate,
    workspace_name: Option<&str>,
) -> StoreResult<WorkspaceId> {
    let template = template.clone();
    let workspace_name = workspace_name.map(str::to_owned);
    store
        .write(move |tx| {
            let look = Look {
                name: workspace_name.unwrap_or(template.name),
                icon: template.icon,
                color: template.color,
            };
            let ws_id = workspaces::create(tx, &look)?;
            let open: Vec<SavedTab> = template
                .items
                .into_iter()
                .map(|it| SavedTab {
                    path: it.path,
                    view: if it.view.is_empty() {
                        "{\"page\":0}".to_owned()
                    } else {
                        it.view
                    },
                    pinned: it.pinned,
                    group: None,
                })
                .collect();
            let active = (!open.is_empty()).then_some(0);
            let panes = Panes {
                layout: template.layout,
                panes: vec![Tabs {
                    open,
                    active,
                    ..Tabs::default()
                }],
            };
            workspace::save_panes(tx, ws_id, &panes)?;
            Ok(ws_id)
        })
        .wait()
}

/// Saves or updates a path-based auto-routing rule (plan W.12).
///
/// # Errors
///
/// SQLite failed.
pub fn save_rule(tx: &Transaction<'_>, rule: &RoutingRule) -> ecclesia_store::Result<()> {
    let resolved_row_id: Option<i64> = if let Some(target_id) = rule.workspace_id {
        let mut stmt = tx.prepare("SELECT id FROM workspace ORDER BY position, id")?;
        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let all_ws = workspaces::list(tx)?;
        all_ws
            .iter()
            .position(|w| w.id == target_id)
            .and_then(|pos| ids.get(pos).copied())
    } else {
        None
    };

    tx.execute(
        "INSERT INTO workspace_rule (id, name, folder_prefix, workspace_id, workspace_name, priority, enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            folder_prefix = excluded.folder_prefix,
            workspace_id = excluded.workspace_id,
            workspace_name = excluded.workspace_name,
            priority = excluded.priority,
            enabled = excluded.enabled",
        params![
            rule.id,
            rule.name,
            rule.folder_prefix.to_string_lossy(),
            resolved_row_id,
            rule.workspace_name,
            rule.priority,
            i64::from(rule.enabled),
        ],
    )?;
    Ok(())
}

/// Lists all auto-routing rules ordered by descending priority.
///
/// # Errors
///
/// SQLite failed.
pub fn list_rules(conn: &Connection) -> ecclesia_store::Result<Vec<RoutingRule>> {
    let all_ws = workspaces::list(conn)?;
    let mut id_stmt = conn.prepare("SELECT id, name FROM workspace")?;
    let ws_names: Vec<(i64, String)> = id_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut stmt = conn.prepare(
        "SELECT id, name, folder_prefix, workspace_id, workspace_name, priority, enabled
         FROM workspace_rule ORDER BY priority DESC, id ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let row_ws_id: Option<i64> = row.get(3)?;
            let ws_name: String = row.get(4)?;
            let matched_ws_id = row_ws_id.and_then(|raw_id| {
                ws_names
                    .iter()
                    .find(|(id, _)| *id == raw_id)
                    .and_then(|(_, n)| all_ws.iter().find(|w| w.name == *n).map(|w| w.id))
            });
            Ok(RoutingRule {
                id: row.get(0)?,
                name: row.get(1)?,
                folder_prefix: PathBuf::from(row.get::<_, String>(2)?),
                workspace_id: matched_ws_id,
                workspace_name: ws_name,
                priority: row.get(5)?,
                enabled: row.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes an auto-routing rule by id.
///
/// # Errors
///
/// SQLite failed.
pub fn delete_rule(tx: &Transaction<'_>, id: &str) -> ecclesia_store::Result<bool> {
    let deleted = tx.execute("DELETE FROM workspace_rule WHERE id = ?1", [id])?;
    Ok(deleted > 0)
}

/// Evaluates `rules` against a newly opened `file_path` and returns the highest-priority
/// matching rule (or longest prefix on tie) (plan W.12).
#[must_use]
pub fn match_rule<'a>(rules: &'a [RoutingRule], file_path: &Path) -> Option<&'a RoutingRule> {
    let norm_file = normalize_path_for_prefix(file_path);
    rules
        .iter()
        .filter(|r| r.enabled)
        .filter(|r| path_under_prefix(&norm_file, &normalize_path_for_prefix(&r.folder_prefix)))
        .max_by_key(|r| {
            (
                r.priority,
                normalize_path_for_prefix(&r.folder_prefix).len(),
            )
        })
}

/// Routes a newly opened `file_path` to its target [`WorkspaceId`] using rules stored in `conn`.
///
/// # Errors
///
/// SQLite failed.
pub fn route_file(
    conn: &Connection,
    file_path: &Path,
) -> ecclesia_store::Result<Option<WorkspaceId>> {
    let rules = list_rules(conn)?;
    let Some(rule) = match_rule(&rules, file_path) else {
        return Ok(None);
    };
    if let Some(id) = rule.workspace_id {
        return Ok(Some(id));
    }
    let all_ws = workspaces::list(conn)?;
    Ok(all_ws
        .into_iter()
        .find(|w| w.name.eq_ignore_ascii_case(&rule.workspace_name))
        .map(|w| w.id))
}

fn normalize_path_for_prefix(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/").to_lowercase();
    s.trim_end_matches('/').to_owned()
}

fn path_under_prefix(norm_file: &str, norm_prefix: &str) -> bool {
    if norm_prefix.is_empty() {
        return false;
    }
    if norm_file == norm_prefix {
        return true;
    }
    norm_file
        .strip_prefix(norm_prefix)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn parse_template_items(json_str: &str) -> Vec<TemplateItem> {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(json_str) else {
        return Vec::new();
    };
    arr.into_iter()
        .filter_map(|v| {
            let obj = v.as_object()?;
            let path = obj.get("path")?.as_str()?;
            Some(TemplateItem {
                title: obj
                    .get("title")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_owned(),
                path: PathBuf::from(path),
                view: obj
                    .get("view")
                    .and_then(|s| s.as_str())
                    .unwrap_or("{\"page\":0}")
                    .to_owned(),
                pinned: obj.get("pinned").and_then(|b| b.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ecclesia-ws-rules-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn collection_and_workspace_convert_bidirectionally_preserving_order() {
        let dir = temp_dir("w10");
        let store = Store::open(&dir.join("library.db")).expect("open");

        let paths = vec![
            PathBuf::from("C:/papers/z.pdf"),
            PathBuf::from("C:/papers/a.pdf"),
            PathBuf::from("C:/papers/m.pdf"),
        ];
        let col_id = store
            .write({
                let paths = paths.clone();
                move |tx| {
                    let cid =
                        library::create_collection(tx, None, "Reading List", Some("📖"), None)?;
                    library::set_collection_items(tx, cid, &paths)?;
                    Ok(cid)
                }
            })
            .wait()
            .expect("seed collection");

        let ws_id = open_collection_as_workspace(&store, col_id, None)
            .expect("open as ws")
            .expect("ws created");

        let panes = store
            .read(|conn| workspace::load_panes(conn, ws_id))
            .expect("load panes");
        let ws_paths: Vec<PathBuf> = panes.panes[0].open.iter().map(|t| t.path.clone()).collect();
        assert_eq!(ws_paths, paths, "workspace preserves collection order");

        let col_id_2 = save_workspace_as_collection(&store, ws_id, None, Some("RoundTrip"))
            .expect("save as col")
            .expect("col created");
        let rt_paths = store
            .read(|conn| library::collection_items(conn, col_id_2))
            .expect("col items");
        assert_eq!(rt_paths, paths, "collection preserves workspace tab order");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn templates_and_path_routing_rules_route_newly_opened_files() {
        let dir = temp_dir("w12");
        let store = Store::open(&dir.join("library.db")).expect("open");

        let course = WorkspaceTemplate::course_preset(Path::new("C:/Courses/CS101"));
        let ws_id =
            instantiate_template(&store, &course, Some("CS101")).expect("instantiate template");

        store
            .write({
                let course = course.clone();
                move |tx| {
                    save_template(tx, &course)?;
                    save_rule(
                        tx,
                        &RoutingRule {
                            id: "rule-cs101".to_owned(),
                            name: "CS101 folder".to_owned(),
                            folder_prefix: PathBuf::from(r"C:\Courses\CS101"),
                            workspace_id: None,
                            workspace_name: "CS101".to_owned(),
                            priority: 10,
                            enabled: true,
                        },
                    )?;
                    Ok(())
                }
            })
            .wait()
            .expect("save rule");

        let routed = store
            .read(|conn| route_file(conn, Path::new("c:/courses/cs101/lectures/week1.pdf")))
            .expect("route");
        assert_eq!(routed, Some(ws_id));

        let unrouted = store
            .read(|conn| route_file(conn, Path::new("C:/Other/paper.pdf")))
            .expect("unrouted");
        assert_eq!(unrouted, None);

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
