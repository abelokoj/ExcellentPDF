//! A workspace to a file and back (W.11): "Export workspace…" saves the shown one as a small JSON file
//! (`ecclesia_store::workspace_file`); "Import workspace…" makes a new workspace from one and shows it. Tabs whose
//! files aren't where the file says come in greyed out, with "Locate…" (R.9), and the notice says how many.

use std::path::{Path, PathBuf};

use ecclesia_store::workspace;
use ecclesia_store::workspace_file::{self, ReadError};
use ecclesia_store::workspaces::{self, Look};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::{Dialog, Workspaces, display_name, name_or_default};
use crate::panes::CELLS;
use crate::settings;

/// The largest workspace file read: a workspace is a few kilobytes, so anything this big isn't one.
const MOST_BYTES: u64 = 8 * 1024 * 1024;

impl Workspaces {
    /// Asks where to save the shown workspace, and saves it there; the notice says how it went.
    pub(super) fn export(self) {
        let Some(shown) = self.shown() else {
            return;
        };
        let name = display_name(&shown);
        let look = Look {
            name: shown.name,
            icon: shown.icon,
            color: shown.color,
        };
        let panes = self.keeper.current();
        let dialog = SaveDialog::new()
            .file_name(format!("{}.json", file_safe(&name)))
            .filter(FileFilter::new(t!("workspace.file_filter"), ["json"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let notice = match answer.await {
                Ok(Some(path)) => {
                    let root = path.parent().unwrap_or(Path::new(""));
                    let json = workspace_file::to_json(&look, &panes, root);
                    match settings::write_atomically(&path, &json) {
                        Ok(()) => t!("workspace.exported", name = name, file = file_name(&path)),
                        Err(err) => {
                            log::warn!("workspace export: {err}");
                            t!("workspace.export_failed", file = file_name(&path))
                        }
                    }
                }
                Ok(None) => return,
                Err(err) => {
                    log::warn!("workspace export: the dialog failed: {err}");
                    t!("workspace.dialog_failed")
                }
            };
            self.tell(notice);
        });
    }

    /// Asks for a workspace file, and shows the workspace it holds as a new one.
    pub(super) fn import(self) {
        let dialog =
            OpenDialog::files().filter(FileFilter::new(t!("workspace.file_filter"), ["json"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(paths)) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(None) => return,
                Err(err) => {
                    log::warn!("workspace import: the dialog failed: {err}");
                    self.tell(t!("workspace.dialog_failed"));
                    return;
                }
            };
            match read(&path) {
                Ok((look, panes)) => self.adopt(look, panes),
                Err(notice) => self.tell(notice),
            }
        });
    }

    /// Makes a workspace of `look` and `panes` and shows it; says so when some of its files are missing.
    fn adopt(self, look: Look, mut panes: workspace::Panes) {
        panes.panes.truncate(CELLS);
        let missing = panes
            .panes
            .iter()
            .flat_map(|tabs| &tabs.open)
            .filter(|tab| !tab.path.is_file())
            .count();
        let name = name_or_default(&look.name);
        let made = self.write(move |tx| {
            let id = workspaces::create(tx, &look)?;
            workspace::save_panes(tx, id, &panes)?;
            Ok(id)
        });
        let Some(id) = made else {
            self.tell(t!("workspace.import_failed_save", name = name));
            return;
        };
        self.reload();
        self.switch(id);
        if missing > 0 {
            self.tell(t!(
                "workspace.imported_missing",
                name = name,
                count = missing
            ));
        }
    }

    /// Opens the notice with `message`.
    fn tell(self, message: String) {
        self.notice.set(message);
        self.set_dialog(Some(Dialog::Notice));
    }
}

/// The workspace in the file at `path`, or what to tell the reader.
fn read(path: &Path) -> Result<(Look, workspace::Panes), String> {
    let file = file_name(path);
    let refused = |err: &dyn std::fmt::Display| {
        log::warn!("workspace import: {}: {err}", path.display());
        t!("workspace.import_failed", file = file)
    };
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MOST_BYTES => {
            return Err(t!("workspace.import_failed", file = file));
        }
        Ok(_) => {}
        Err(err) => return Err(refused(&err)),
    }
    let json = std::fs::read_to_string(path).map_err(|err| refused(&err))?;
    let root = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    workspace_file::from_json(&json, &root).map_err(|err| match err {
        ReadError::TooNew { .. } => t!("workspace.import_too_new", file = file),
        err => refused(&err),
    })
}

/// A file's name, for the notices.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `name` with the characters a file name can't have on some system replaced.
fn file_safe(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.trim().trim_end_matches('.');
    if safe.is_empty() {
        "workspace".to_owned()
    } else {
        safe.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_name_makes_a_file_name() {
        assert_eq!(file_safe("Tax 2025"), "Tax 2025");
        assert_eq!(file_safe("A/B: notes?"), "A-B- notes-");
        assert_eq!(file_safe(" ... "), "workspace");
    }
}
