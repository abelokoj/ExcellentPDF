//! The user's settings (K.16, plan §9.2): `settings.toml`, one file of TOML. Only values that differ from the default
//! are written; keys this version doesn't know are kept as they are, so a newer version's settings survive an older
//! one. The file is written atomically (a temporary file renamed over it).
//!
//! It lives in the app's data folder (`%APPDATA%\<app id>` on Windows, where NSH keeps the app's own files), or next
//! to the executable when a `settings.toml` is already there: portable mode, as SumatraPDF does it.
//!
//! The running app follows the file ([`Live`]): an edit made while it runs is read in, and what shows a setting
//! changes with it. A file that doesn't parse is never written over; once it is fixed, it is read in and kept again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

use nsh_core::prelude::*;

use crate::APP_ID;

/// The settings file's name, in either place.
const FILE_NAME: &str = "settings.toml";

/// How often the running app looks for an edit of the file.
const WATCH_PERIOD: Duration = Duration::from_secs(1);

/// Settings as read from their file, and where to write them back.
#[derive(Debug, Default)]
pub(crate) struct Settings {
    /// `None`: kept in memory only (the tests, a host with no data folder, or a file that didn't parse and so must
    /// not be overwritten).
    path: Option<PathBuf>,
    table: toml::Table,
    /// The file followed for edits, even one that didn't parse.
    source: Option<PathBuf>,
    /// The file's modified time when it was last read or written.
    modified: Option<SystemTime>,
}

impl Settings {
    /// The settings in `path`: none yet when the file doesn't exist. A file that can't be read or parsed is left
    /// alone (logged) and the defaults are used, without saving over it.
    pub(crate) fn load(path: PathBuf) -> Self {
        let unreadable = Self {
            modified: modified_time(&path),
            source: Some(path.clone()),
            ..Self::default()
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                log::warn!(
                    "settings: can't read {}: {err}; using the defaults",
                    path.display()
                );
                return unreadable;
            }
        };
        match text.parse::<toml::Table>() {
            Ok(table) => Self {
                path: Some(path),
                table,
                ..unreadable
            },
            Err(err) => {
                log::warn!(
                    "settings: {} isn't valid TOML, so it won't be changed until it is fixed: {err}",
                    path.display()
                );
                unreadable
            }
        }
    }

    /// The string `key` holds, if it holds one.
    pub(crate) fn text(&self, key: &str) -> Option<&str> {
        self.table.get(key).and_then(toml::Value::as_str)
    }

    /// Sets `key` to `value`, or removes it (`None`: the default).
    pub(crate) fn set_text(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.table.insert(key.to_owned(), value.into()),
            None => self.table.remove(key),
        };
    }

    /// The number `key` holds, written with or without a decimal point, if it holds one.
    pub(crate) fn number(&self, key: &str) -> Option<f64> {
        match self.table.get(key)? {
            toml::Value::Float(value) => Some(*value),
            toml::Value::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// Sets `key` to `value`, or removes it (`None`: the default).
    pub(crate) fn set_number(&mut self, key: &str, value: Option<f64>) {
        match value {
            Some(value) => self.table.insert(key.to_owned(), value.into()),
            None => self.table.remove(key),
        };
    }

    /// The table `key` holds, if it holds one.
    pub(crate) fn table(&self, key: &str) -> Option<&toml::Table> {
        self.table.get(key).and_then(toml::Value::as_table)
    }

    /// The yes or no `key` holds, if it holds one.
    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        self.table.get(key).and_then(toml::Value::as_bool)
    }

    /// Sets `key` to `value`, or removes it (`None`: the default).
    pub(crate) fn set_flag(&mut self, key: &str, value: Option<bool>) {
        match value {
            Some(value) => self.table.insert(key.to_owned(), value.into()),
            None => self.table.remove(key),
        };
    }

    /// Writes the settings back to their file, if they have one. Failures are logged: a setting not kept is no reason
    /// to stop the user.
    pub(crate) fn save(&mut self) {
        let Some(path) = &self.path else { return };
        match write_atomically(path, &self.table.to_string()) {
            Ok(()) => self.modified = modified_time(path),
            Err(err) => log::warn!("settings: can't write {}: {err}", path.display()),
        }
    }

    /// Reads the file again if it changed since it was last read or written, and says whether any setting changed. A
    /// file that doesn't parse now is logged, and the settings stay as they are.
    pub(crate) fn reload_if_changed(&mut self) -> bool {
        let Some(source) = &self.source else {
            return false;
        };
        let modified = modified_time(source);
        if modified == self.modified {
            return false;
        }
        let fresh = Self::load(source.clone());
        if fresh.path.is_none() {
            self.modified = fresh.modified;
            return false;
        }
        let changed = fresh.table != self.table;
        *self = fresh;
        changed
    }
}

/// The settings file, for the user to edit by hand: created first when it doesn't exist yet. A file that doesn't parse
/// is given too, so it can be fixed. `None` when there is no settings file (no data folder, or the tests).
pub(crate) fn file_to_edit() -> Option<PathBuf> {
    let mut settings = shared();
    let source = settings.source.clone()?;
    if !source.exists() {
        settings.save();
    }
    Some(source)
}

/// The settings as the UI follows them. Reading through [`Live::read`] subscribes, so a view or prop that shows a
/// setting changes when the settings window or an edit of the file changes it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Live {
    revision: Signal<u64>,
}

impl Live {
    /// Starts following the settings file while the current component lives.
    pub(crate) fn new() -> Self {
        let live = Self {
            revision: signal(0),
        };
        if shared().source.is_some() {
            // shortcut: one look at the file's modified time a second; use the OS's change notifications if K.24's
            // idle measurements show it.
            set_interval(WATCH_PERIOD, move || {
                let changed = shared().reload_if_changed();
                if changed {
                    live.changed();
                }
            });
        }
        live
    }

    /// `f` of the settings now; reading in a reactive scope subscribes to them.
    pub(crate) fn read<T>(self, f: impl FnOnce(&Settings) -> T) -> T {
        self.revision.get();
        f(&shared())
    }

    /// Changes the settings with `f`, keeps them, and updates everything that shows them.
    pub(crate) fn change(self, f: impl FnOnce(&mut Settings)) {
        {
            let mut settings = shared();
            f(&mut settings);
            settings.save();
        }
        self.changed();
    }

    fn changed(self) {
        self.revision.update(|revision| *revision += 1);
    }
}

/// When the file at `path` was last modified, if it exists.
fn modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes `text` to a temporary file beside `path`, then renames it over `path`, so a crash never leaves half a file.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}

/// Where the settings file is.
fn location() -> Option<PathBuf> {
    Some(folder()?.join(FILE_NAME))
}

/// Where the app keeps its files (settings, `library.db`): next to the executable when a settings file is there
/// (portable mode), else the app's data folder. `None` when the system has no data folder for the app.
pub(crate) fn folder() -> Option<PathBuf> {
    portable_folder().or_else(|| Some(nsh_engine::data_dir()?.join(APP_ID)))
}

/// The executable's folder, when the app runs portable from it (a settings file is there).
pub(crate) fn portable_folder() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.to_path_buf()))
        .filter(|dir| dir.join(FILE_NAME).is_file())
}

static SHARED: OnceLock<Mutex<Settings>> = OnceLock::new();

/// Reads the user's settings file. The desktop binary calls it before the app starts; without it (the tests) the
/// settings are the defaults and are never written.
pub(crate) fn load_shared() {
    let settings = location().map(Settings::load).unwrap_or_default();
    let _ = SHARED.set(Mutex::new(settings));
}

/// The app's settings.
pub(crate) fn shared() -> MutexGuard<'static, Settings> {
    SHARED
        .get_or_init(Mutex::default)
        .lock()
        .expect("settings lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "excellentpdf-settings-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn keeps_unknown_keys_and_writes_only_what_is_set() {
        let path = scratch("round-trip");
        write_atomically(&path, "future = 3\ntheme = \"coral\"\n").expect("writes");
        let mut settings = Settings::load(path.clone());
        assert_eq!(settings.text("theme"), Some("coral"));
        settings.set_text("theme", None);
        settings.set_text("appearance", Some("dark"));
        settings.save();
        let saved = fs::read_to_string(&path).expect("reads back");
        assert_eq!(saved.parse::<toml::Table>().expect("valid"), {
            let mut want = toml::Table::new();
            want.insert("future".into(), 3.into());
            want.insert("appearance".into(), "dark".into());
            want
        });
        let _ = fs::remove_dir_all(path.parent().expect("has a folder"));
    }

    #[test]
    fn a_broken_file_is_never_overwritten() {
        let path = scratch("broken");
        write_atomically(&path, "theme = [unclosed").expect("writes");
        let mut settings = Settings::load(path.clone());
        assert_eq!(settings.text("theme"), None);
        settings.set_text("theme", Some("coral"));
        settings.save();
        assert_eq!(
            fs::read_to_string(&path).expect("reads"),
            "theme = [unclosed"
        );
        let _ = fs::remove_dir_all(path.parent().expect("has a folder"));
    }

    #[test]
    fn numbers_read_with_or_without_a_point_and_flags_only_as_flags() {
        let mut settings = Settings {
            table: "a = 48\nb = 0.4\nc = \"48\"\nd = true\n"
                .parse()
                .expect("valid"),
            ..Settings::default()
        };
        assert_eq!(settings.number("a"), Some(48.0));
        assert_eq!(settings.number("b"), Some(0.4));
        assert_eq!(settings.number("c"), None);
        assert_eq!(settings.flag("d"), Some(true));
        assert_eq!(settings.flag("a"), None);
        settings.set_number("a", None);
        settings.set_flag("e", Some(false));
        assert_eq!(settings.number("a"), None);
        assert_eq!(settings.flag("e"), Some(false));
    }

    /// Writes `text` to `path` with a modified time `seconds` from now, as an editor would later.
    fn edit(path: &Path, text: &str, seconds: u64) {
        fs::write(path, text).expect("writes");
        let at = SystemTime::now() + Duration::from_secs(seconds);
        fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(at))
            .expect("sets the time");
    }

    #[test]
    fn an_edit_of_the_file_is_read_in_and_its_own_saves_are_not() {
        let path = scratch("reload");
        write_atomically(&path, "theme = \"coral\"\n").expect("writes");
        let mut settings = Settings::load(path.clone());
        assert!(!settings.reload_if_changed(), "nothing changed yet");
        settings.set_text("appearance", Some("dark"));
        settings.save();
        assert!(!settings.reload_if_changed(), "its own save is no edit");
        edit(&path, "theme = \"grape\"\n", 10);
        assert!(settings.reload_if_changed());
        assert_eq!(settings.text("theme"), Some("grape"));
        assert_eq!(settings.text("appearance"), None);
        edit(&path, "theme = \"grape\"\n", 20);
        assert!(!settings.reload_if_changed(), "touched, but the same");
        let _ = fs::remove_dir_all(path.parent().expect("has a folder"));
    }

    #[test]
    fn a_file_broken_by_an_edit_is_kept_out_until_fixed() {
        let path = scratch("reload-broken");
        write_atomically(&path, "theme = \"coral\"\n").expect("writes");
        let mut settings = Settings::load(path.clone());
        edit(&path, "theme = [unclosed", 10);
        assert!(!settings.reload_if_changed());
        assert_eq!(
            settings.text("theme"),
            Some("coral"),
            "the last good settings stay"
        );
        edit(&path, "theme = \"lagoon\"\n", 20);
        assert!(settings.reload_if_changed());
        settings.set_text("appearance", Some("light"));
        settings.save();
        let kept = Settings::load(path.clone());
        assert_eq!(kept.text("theme"), Some("lagoon"));
        assert_eq!(kept.text("appearance"), Some("light"), "writable again");
        let _ = fs::remove_dir_all(path.parent().expect("has a folder"));
    }

    #[test]
    fn a_missing_file_starts_empty_and_is_created_on_save() {
        let path = scratch("missing");
        let mut settings = Settings::load(path.clone());
        settings.set_text("theme", Some("grape:high"));
        settings.save();
        assert_eq!(
            fs::read_to_string(&path).expect("created"),
            "theme = \"grape:high\"\n"
        );
        let _ = fs::remove_dir_all(path.parent().expect("has a folder"));
    }
}
