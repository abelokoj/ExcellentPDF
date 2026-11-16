//! Which keys run which command (plan K.16): the command table's defaults, with the user's own over them.
//!
//! The user's keys live in `settings.toml`, one table per command they change:
//!
//! ```toml
//! [shortcuts."go.to_page"]
//! keys = ["mod+alt+g"]   # anywhere in the window
//! page_keys = []         # while the pages have the keyboard: none
//! ```
//!
//! A list replaces that command's defaults of the same kind; `[]` unbinds them. A key two commands claim goes to the one
//! the user set, else to the earlier row. Keys that cannot be bound (not a key, or one the platform keeps, such as F11)
//! and names that are no command are reported in the log, never bound. An edit of the file takes effect at once.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use nsh_core::keymap::{Keystroke, apple_keys};
use nsh_core::platform::Os;
use nsh_core::prelude::*;

use crate::commands::{COMMANDS, Command, CommandId};
use crate::settings;

/// The settings table that holds the user's keys.
const SETTINGS_KEY: &str = "shortcuts";

/// Where a key works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Anywhere in the window, unless a text field uses the key.
    Window,
    /// Only while the pages have the keyboard focus; elsewhere the key goes on to what has it (an arrow key moves
    /// through a list).
    Page,
}

/// One key bound to one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    /// What it runs.
    pub(crate) id: CommandId,
    /// The key, as NSH's keymap writes it (`mod+shift+l`).
    pub(crate) keys: String,
    /// Where it works.
    pub(crate) scope: Scope,
}

/// Why something the user wrote was not bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    /// `[shortcuts."name"]` names no command.
    UnknownCommand(String),
    /// A command's entry is not a table of key lists.
    NotKeyLists(&'static str),
    /// A key that is no key, or one the platform keeps for itself.
    BadKey {
        command: &'static str,
        keys: String,
        why: String,
    },
    /// Two commands claim one key; the first keeps it.
    Conflict {
        keys: String,
        kept: &'static str,
        dropped: &'static str,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(name) => write!(f, "no command is named {name:?}"),
            Self::NotKeyLists(name) => write!(
                f,
                "{name:?} should hold `keys` and `page_keys`, lists of keys"
            ),
            Self::BadKey { command, keys, why } => {
                write!(f, "{keys:?} for {command:?} cannot be bound: {why}")
            }
            Self::Conflict {
                keys,
                kept,
                dropped,
            } => write!(f, "{keys:?} runs {kept:?}, not {dropped:?} too"),
        }
    }
}

/// A command's keys of both kinds, the user's where they gave them.
#[derive(Debug, Default)]
struct Chosen {
    keys: Option<Vec<String>>,
    page_keys: Option<Vec<String>>,
}

/// Every key the app binds, and what was not bound.
#[derive(Debug)]
pub(crate) struct Keymap {
    bindings: Vec<Binding>,
    problems: Vec<Problem>,
}

impl Keymap {
    /// Resolves `commands`' default keys with the user's `overrides` (the `shortcuts` table) over them, for `os`, with
    /// Apple's key names (`mod` is Command) or not.
    pub(crate) fn new(
        commands: &[Command],
        overrides: Option<&toml::Table>,
        os: Os,
        apple: bool,
    ) -> Self {
        let mut problems = Vec::new();
        let chosen = overrides.map_or_else(HashMap::new, |table| {
            user_keys(commands, table, &mut problems)
        });
        // The user's commands go first, so their keys win.
        let (users, defaults): (Vec<&Command>, Vec<&Command>) =
            commands.iter().partition(|c| chosen.contains_key(c.name));
        let mut bindings = Vec::new();
        let mut taken: HashMap<Keystroke, &'static str> = HashMap::new();
        for command in users.into_iter().chain(defaults) {
            let user = chosen.get(command.name);
            let window = user.and_then(|u| u.keys.as_deref());
            let page = user.and_then(|u| u.page_keys.as_deref());
            let window = window.map_or_else(|| owned(command.keys), <[String]>::to_vec);
            let page = page.map_or_else(|| owned(command.page_keys), <[String]>::to_vec);
            let wanted = window
                .into_iter()
                .map(|k| (k, Scope::Window))
                .chain(page.into_iter().map(|k| (k, Scope::Page)));
            for (keys, scope) in wanted {
                let stroke = match Keystroke::parse_with(&keys, apple) {
                    Ok(stroke) => stroke,
                    Err(error) => {
                        problems.push(Problem::BadKey {
                            command: command.name,
                            keys,
                            why: error.to_string(),
                        });
                        continue;
                    }
                };
                if let Some(why) = stroke.reserved(os) {
                    problems.push(Problem::BadKey {
                        command: command.name,
                        keys,
                        why: why.to_owned(),
                    });
                    continue;
                }
                match taken.get(&stroke) {
                    // The same key twice for one command (`mod+=` and `mod+plus`) binds once.
                    Some(&kept) if kept == command.name => {}
                    Some(&kept) => problems.push(Problem::Conflict {
                        keys,
                        kept,
                        dropped: command.name,
                    }),
                    None => {
                        taken.insert(stroke, command.name);
                        bindings.push(Binding {
                            id: command.id,
                            keys,
                            scope,
                        });
                    }
                }
            }
        }
        Self { bindings, problems }
    }

    /// Every key bound.
    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// What the user wrote that was not bound.
    pub(crate) fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// The key hints show for `id`: its first window key, else its first page key.
    pub(crate) fn first_keys(&self, id: CommandId) -> Option<&str> {
        let first = |scope| {
            self.bindings
                .iter()
                .find(|b| b.id == id && b.scope == scope)
        };
        first(Scope::Window)
            .or_else(|| first(Scope::Page))
            .map(|b| b.keys.as_str())
    }
}

/// The app's keys for the settings now. They are resolved again only when the user's keys change, which also logs
/// their problems once.
pub(crate) fn current() -> Arc<Keymap> {
    static CACHE: Mutex<Option<(Option<toml::Table>, Arc<Keymap>)>> = Mutex::new(None);
    let overrides = settings::shared().table(SETTINGS_KEY).cloned();
    let mut cache = CACHE.lock().expect("keymap lock poisoned");
    match &*cache {
        Some((seen, keymap)) if *seen == overrides => return Arc::clone(keymap),
        _ => {}
    }
    let os = try_os().unwrap_or(Os::Windows);
    let apple = try_apple_keys().unwrap_or_else(|| apple_keys(os, false));
    let keymap = Arc::new(Keymap::new(COMMANDS, overrides.as_ref(), os, apple));
    for problem in keymap.problems() {
        log::warn!("settings.toml [{SETTINGS_KEY}]: {problem}");
    }
    *cache = Some((overrides, Arc::clone(&keymap)));
    keymap
}

fn owned(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|&k| k.to_owned()).collect()
}

/// The user's key lists by command name; entries that are not are reported.
fn user_keys(
    commands: &[Command],
    table: &toml::Table,
    problems: &mut Vec<Problem>,
) -> HashMap<&'static str, Chosen> {
    let mut chosen = HashMap::new();
    for (name, entry) in table {
        let Some(command) = commands.iter().find(|c| c.name == name) else {
            problems.push(Problem::UnknownCommand(name.clone()));
            continue;
        };
        let lists = entry.as_table().and_then(|entry| {
            let list = |field| match entry.get(field) {
                None => Some(None),
                Some(value) => strings(value).map(Some),
            };
            let known = entry.keys().all(|k| k == "keys" || k == "page_keys");
            known.then_some(())?;
            Some(Chosen {
                keys: list("keys")?,
                page_keys: list("page_keys")?,
            })
        });
        match lists {
            Some(lists) => {
                chosen.insert(command.name, lists);
            }
            None => problems.push(Problem::NotKeyLists(command.name)),
        }
    }
    chosen
}

/// `value` as a list of strings, if it is one.
fn strings(value: &toml::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap(overrides: &str) -> Keymap {
        let table: toml::Table = overrides.parse().expect("valid toml");
        Keymap::new(COMMANDS, Some(&table), Os::Windows, false)
    }

    fn runs(keymap: &Keymap, keys: &str) -> Option<CommandId> {
        keymap
            .bindings()
            .iter()
            .find(|b| b.keys == keys)
            .map(|b| b.id)
    }

    #[test]
    fn the_defaults_bind_with_no_problem_on_any_platform() {
        for (os, apple) in [(Os::Windows, false), (Os::MacOs, true), (Os::Linux, false)] {
            let keymap = Keymap::new(COMMANDS, None, os, apple);
            assert_eq!(keymap.problems(), [], "{os:?}");
            let defaults: usize = COMMANDS
                .iter()
                .map(|c| c.keys.len() + c.page_keys.len())
                .sum();
            assert_eq!(keymap.bindings().len(), defaults, "{os:?}");
        }
    }

    #[test]
    fn a_users_key_wins_over_a_default_and_an_empty_list_unbinds() {
        let keymap = keymap(
            r#"
            ["go.to_page"]
            keys = ["mod+k"]
            page_keys = []
            "#,
        );
        assert_eq!(runs(&keymap, "mod+k"), Some(CommandId::GoToPage));
        assert_eq!(runs(&keymap, "mod+g"), None);
        assert_eq!(runs(&keymap, "g"), None);
        assert_eq!(
            runs(&keymap, "mod+shift+p"),
            Some(CommandId::OpenPalette),
            "the palette keeps its other key"
        );
        assert_eq!(
            keymap.problems(),
            [Problem::Conflict {
                keys: "mod+k".into(),
                kept: "go.to_page",
                dropped: "palette.open",
            }]
        );
        assert_eq!(
            keymap.first_keys(CommandId::OpenPalette),
            Some("mod+shift+p")
        );
    }

    #[test]
    fn a_command_without_window_keys_shows_its_page_key() {
        let keymap = keymap("");
        assert_eq!(keymap.first_keys(CommandId::LeaveModes), Some("escape"));
        assert_eq!(keymap.first_keys(CommandId::GoToPage), Some("mod+g"));
    }

    #[test]
    fn what_cannot_be_bound_is_reported_not_bound() {
        let keymap = keymap(
            r#"
            ["view.sidebar"]
            keys = ["f11", "mod+nokey", "f12"]
            ["no.such_command"]
            keys = ["mod+alt+shift+9"]
            ["view.inspector"]
            keys = "mod+i"
            ["view.theme"]
            key = ["t"]
            "#,
        );
        assert_eq!(runs(&keymap, "f12"), Some(CommandId::ToggleSidebar));
        assert_eq!(runs(&keymap, "f11"), None);
        assert_eq!(runs(&keymap, "mod+alt+shift+9"), None);
        assert_eq!(runs(&keymap, "mod+alt+b"), Some(CommandId::ToggleInspector));
        let problems = keymap.problems();
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(problems.contains(&Problem::UnknownCommand("no.such_command".into())));
        assert!(problems.contains(&Problem::NotKeyLists("view.inspector")));
        assert!(problems.contains(&Problem::NotKeyLists("view.theme")));
        let bad: Vec<&str> = problems
            .iter()
            .filter_map(|p| match p {
                Problem::BadKey { keys, .. } => Some(keys.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(bad, ["f11", "mod+nokey"]);
    }
}
