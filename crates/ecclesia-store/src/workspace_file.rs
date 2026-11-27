//! A workspace as a file of its own (W.11): its name, icon and colour, its panes' layout and its tabs, as a small JSON
//! file to keep or to take to another computer. A tab's file under the folder the workspace file is saved in is
//! written relative to that folder, with `/`, so the workspace file and the papers can move together; any other
//! stays as it is. Closed tabs stay behind: they are history, not the workspace.
//!
//! Reading takes whatever a file holds as untrusted: places that point past the end are dropped, and what is missing
//! takes its default.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::workspace::{Panes, SavedGroup, SavedTab, Tabs};
use crate::workspaces::Look;

/// What a workspace file says it is.
const KIND: &str = "ecclesiapdf-workspace";

/// The format's version; a newer one is refused rather than guessed at.
const VERSION: u64 = 1;

/// Why a file can't be read as a workspace.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    /// It isn't JSON.
    Json(serde_json::Error),
    /// It is JSON, but not a workspace.
    NotWorkspace,
    /// A newer version of the app wrote it.
    TooNew {
        /// The file's version.
        version: u64,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(err) => write!(f, "not json: {err}"),
            Self::NotWorkspace => f.write_str("not a workspace file"),
            Self::TooNew { version } => {
                write!(
                    f,
                    "workspace file version {version} is newer than {VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(err) => Some(err),
            Self::NotWorkspace | Self::TooNew { .. } => None,
        }
    }
}

/// The workspace with `look` and `panes` as a file saved in the folder `root`.
#[must_use]
pub fn to_json(look: &Look, panes: &Panes, root: &Path) -> String {
    let value = json!({
        "kind": KIND,
        "version": VERSION,
        "name": look.name,
        "icon": look.icon,
        "color": look.color,
        "layout": panes.layout,
        "panes": panes.panes.iter().map(|tabs| pane_json(tabs, root)).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&value).expect("a JSON value with string keys always serializes")
}

/// The workspace in `json`, a file read from the folder `root`: its look and its panes.
///
/// # Errors
///
/// The text isn't JSON, isn't a workspace, or a newer version of the app wrote it.
pub fn from_json(json: &str, root: &Path) -> Result<(Look, Panes), ReadError> {
    let value: Value = serde_json::from_str(json).map_err(ReadError::Json)?;
    if value.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err(ReadError::NotWorkspace);
    }
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(ReadError::NotWorkspace)?;
    if version > VERSION {
        return Err(ReadError::TooNew { version });
    }
    let look = Look {
        name: text(&value, "name").unwrap_or_default(),
        icon: text(&value, "icon"),
        color: text(&value, "color"),
    };
    let mut panes: Vec<Tabs> = items(&value, "panes")
        .map(|pane| read_pane(pane, root))
        .collect();
    if panes.is_empty() {
        panes.push(Tabs::default());
    }
    let panes = Panes {
        layout: text(&value, "layout"),
        panes,
    };
    Ok((look, panes))
}

fn pane_json(tabs: &Tabs, root: &Path) -> Value {
    let groups: Vec<Value> = tabs
        .groups
        .iter()
        .map(|group| {
            json!({
                "name": group.name,
                "color": group.color,
                "collapsed": group.collapsed,
            })
        })
        .collect();
    let open: Vec<Value> = tabs
        .open
        .iter()
        .map(|tab| {
            json!({
                "path": write_path(&tab.path, root),
                "view": tab.view,
                "pinned": tab.pinned,
                "group": tab.group,
            })
        })
        .collect();
    json!({ "active": tabs.active, "groups": groups, "tabs": open })
}

fn read_pane(value: &Value, root: &Path) -> Tabs {
    let groups: Vec<SavedGroup> = items(value, "groups")
        .map(|group| SavedGroup {
            name: text(group, "name").unwrap_or_default(),
            color: text(group, "color").unwrap_or_default(),
            collapsed: flag(group, "collapsed"),
        })
        .collect();
    let open: Vec<SavedTab> = items(value, "tabs")
        .filter_map(|tab| {
            let path = tab.get("path")?.as_str()?;
            Some(SavedTab {
                path: read_path(path, root),
                view: text(tab, "view").unwrap_or_else(|| "{}".to_owned()),
                pinned: flag(tab, "pinned"),
                group: place(tab, "group").filter(|&group| group < groups.len()),
            })
        })
        .collect();
    let active = place(value, "active").filter(|&active| active < open.len());
    Tabs {
        open,
        active,
        groups,
        closed: Vec::new(),
    }
}

/// `path` as the file writes it: relative to `root`, with `/`, when it is under it.
fn write_path(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        _ => path.to_string_lossy().into_owned(),
    }
}

/// A path as the file wrote it, relative ones under `root`, with this system's separators.
fn read_path(text: &str, root: &Path) -> PathBuf {
    let path = Path::new(text);
    if path.is_absolute() || path.has_root() {
        return path.to_path_buf();
    }
    text.split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn items<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn place(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|place| usize::try_from(place).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Docs\Thesis")
        } else {
            PathBuf::from("/home/reader/Thesis")
        }
    }

    fn elsewhere() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"D:\Library\book.pdf")
        } else {
            PathBuf::from("/srv/library/book.pdf")
        }
    }

    fn tab(path: PathBuf, group: Option<usize>) -> SavedTab {
        SavedTab {
            path,
            view: "{\"page\":3,\"zoom\":1.5}".to_owned(),
            pinned: group.is_none(),
            group,
        }
    }

    fn workspace() -> (Look, Panes) {
        let look = Look {
            name: "Thesis".to_owned(),
            icon: Some("🎓".to_owned()),
            color: Some("green".to_owned()),
        };
        let left = Tabs {
            open: vec![
                tab(root().join("papers").join("a.pdf"), None),
                tab(elsewhere(), Some(0)),
            ],
            active: Some(1),
            groups: vec![SavedGroup {
                name: "Reading".to_owned(),
                color: "blue".to_owned(),
                collapsed: true,
            }],
            closed: Vec::new(),
        };
        let right = Tabs {
            open: vec![tab(root().join("draft.pdf"), None)],
            active: Some(0),
            ..Tabs::default()
        };
        let panes = Panes {
            layout: Some("{\"columns\":2}".to_owned()),
            panes: vec![left, right],
        };
        (look, panes)
    }

    #[test]
    fn export_then_import_round_trips_exactly() {
        let (look, panes) = workspace();
        let json = to_json(&look, &panes, &root());
        assert!(
            json.contains("\"papers/a.pdf\""),
            "under the folder: relative, with /"
        );
        assert_eq!(from_json(&json, &root()).expect("read"), (look, panes));
    }

    #[test]
    fn the_file_moves_with_its_papers() {
        let (look, panes) = workspace();
        let json = to_json(&look, &panes, &root());
        let moved = root().parent().expect("parent").join("Moved");
        let (_, read) = from_json(&json, &moved).expect("read");
        assert_eq!(
            read.panes[0].open[0].path,
            moved.join("papers").join("a.pdf")
        );
        assert_eq!(
            read.panes[0].open[1].path,
            elsewhere(),
            "outside: as it was"
        );
    }

    #[test]
    fn a_file_that_is_not_a_workspace_is_refused() {
        assert!(matches!(
            from_json("not json", &root()),
            Err(ReadError::Json(_))
        ));
        assert!(matches!(
            from_json("{\"kind\":\"toolbars\",\"version\":1}", &root()),
            Err(ReadError::NotWorkspace)
        ));
        assert!(matches!(
            from_json(&format!("{{\"kind\":\"{KIND}\",\"version\":2}}"), &root()),
            Err(ReadError::TooNew { version: 2 })
        ));
    }

    #[test]
    fn places_past_the_end_are_dropped() {
        let json = format!(
            "{{\"kind\":\"{KIND}\",\"version\":1,\"panes\":[{{\"active\":7,\"tabs\":[{{\"path\":\"a.pdf\",\"group\":3}},{{\"view\":\"{{}}\"}}]}}]}}"
        );
        let (look, panes) = from_json(&json, &root()).expect("read");
        assert_eq!(look, Look::default());
        let pane = &panes.panes[0];
        assert_eq!(pane.open.len(), 1, "a tab without a path is dropped");
        assert_eq!(pane.open[0].group, None);
        assert_eq!(pane.active, None);
        let empty = format!("{{\"kind\":\"{KIND}\",\"version\":1}}");
        assert_eq!(
            from_json(&empty, &root()).expect("read").1,
            Panes::default()
        );
    }
}
