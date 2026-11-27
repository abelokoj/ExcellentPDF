//! Modes and their toolbars (plan K.18, §10.2), the model: the five modes, each mode's toolbar as a list of command
//! names, the edits the customize dialog makes, and presets as JSON a reader can share. Commands are named by their
//! stable names (`view.zoom_in`), so this module knows nothing of the app's command table: the caller says which names
//! exist, and unknown ones are left out wherever a toolbar is read.

use core::fmt;

use serde_json::{Map, Value, json};

/// A way of working with a document: each has its own toolbar and accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// Zoom, layout, find, theme.
    #[default]
    Read,
    /// Markup, comments, shapes, stamps.
    Annotate,
    /// Pens, erasers, lasso, page templates.
    Write,
    /// Flashcards, review queue, quiz, timer.
    Study,
    /// Laser, spotlight, magnifier, presenter view.
    Present,
}

impl Mode {
    /// Every mode, in the switcher's order.
    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::Annotate,
        Self::Write,
        Self::Study,
        Self::Present,
    ];

    /// Its stable id: its key in `settings.toml` and presets, and its accent's token (`mode-<id>`).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Annotate => "annotate",
            Self::Write => "write",
            Self::Study => "study",
            Self::Present => "present",
        }
    }

    /// The mode with id `id`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id)
    }

    /// The mode after this one among those `offered`, around the end (`back`: before it); itself when no other is.
    #[must_use]
    pub fn step(self, back: bool, offered: impl Fn(Self) -> bool) -> Self {
        let at = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        let n = Self::ALL.len();
        (1..n)
            .map(|k| Self::ALL[if back { (at + n - k) % n } else { (at + k) % n }])
            .find(|&mode| offered(mode))
            .unwrap_or(self)
    }
}

/// The mode after the pen touched a page: Write, when the setting allows it and Write is offered (plan K.18 step 3).
#[must_use]
pub fn after_pen_contact(current: Mode, allowed: bool, write_offered: bool) -> Mode {
    if allowed && write_offered {
        Mode::Write
    } else {
        current
    }
}

/// How a separator is written in a toolbar's text and in presets.
const SEPARATOR: &str = "|";

/// One place on a toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A command, by its stable name.
    Command(String),
    /// A thin rule between groups of commands.
    Separator,
}

impl Item {
    fn word(&self) -> &str {
        match self {
            Self::Command(name) => name,
            Self::Separator => SEPARATOR,
        }
    }
}

/// A mode's toolbar: commands and separators, in order. It is always tidy: each command once, no separator at either
/// end or next to another.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Toolbar(Vec<Item>);

impl Toolbar {
    /// The toolbar of `words` (command names and `|`), keeping only the commands `known` and tidied.
    pub fn from_words<'w>(
        words: impl IntoIterator<Item = &'w str>,
        known: impl Fn(&str) -> bool,
    ) -> Self {
        let items = words
            .into_iter()
            .filter_map(|word| match word {
                SEPARATOR => Some(Item::Separator),
                name if known(name) => Some(Item::Command(name.to_owned())),
                _ => None,
            })
            .collect();
        let mut toolbar = Self(items);
        toolbar.tidy();
        toolbar
    }

    /// The toolbar written by [`Toolbar::to_text`] (as `settings.toml` keeps it).
    pub fn parse(text: &str, known: impl Fn(&str) -> bool) -> Self {
        Self::from_words(text.split_whitespace(), known)
    }

    /// Its words separated by spaces.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.words().collect::<Vec<_>>().join(" ")
    }

    fn words(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(Item::word)
    }

    /// Its items, in order.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.0
    }

    /// Whether it has no command (only separators can't be: they are tidied away).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the command `name` is on it.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|item| matches!(item, Item::Command(n) if n == name))
    }

    /// Puts the command `name` at the end, unless it is on already.
    pub fn add(&mut self, name: &str) {
        if !self.contains(name) {
            self.0.push(Item::Command(name.to_owned()));
        }
    }

    /// Puts a separator at the end (tidied away until a command follows it, so it is added only after one).
    pub fn add_separator(&mut self) {
        if matches!(self.0.last(), Some(Item::Command(_))) {
            self.0.push(Item::Separator);
        }
    }

    /// Takes the item at `at` off (nothing when there is none).
    pub fn remove(&mut self, at: usize) {
        if at < self.0.len() {
            self.0.remove(at);
            self.tidy();
        }
    }

    /// Moves the item at `from` to `to` (clamped to the ends), the others closing up behind it.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.0.len() {
            return;
        }
        let item = self.0.remove(from);
        let to = to.min(self.0.len());
        self.0.insert(to, item);
        self.tidy();
    }

    /// Drops repeated commands and the separators at the ends or next to another.
    fn tidy(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        let mut tidy: Vec<Item> = Vec::with_capacity(self.0.len());
        for item in self.0.drain(..) {
            match item {
                Item::Command(name) if seen.contains(&name) => {}
                Item::Command(name) => {
                    seen.push(name.clone());
                    tidy.push(Item::Command(name));
                }
                Item::Separator if matches!(tidy.last(), Some(Item::Command(_))) => {
                    tidy.push(Item::Separator);
                }
                Item::Separator => {}
            }
        }
        if tidy.last() == Some(&Item::Separator) {
            tidy.pop();
        }
        self.0 = tidy;
    }
}

/// The key that marks a JSON file as a toolbar preset, with the preset's format version as its value.
const PRESET_KEY: &str = "ecclesiapdf_toolbars";
/// The preset format this version writes and reads.
const PRESET_VERSION: u64 = 1;

/// Toolbars to share: some modes' toolbars, written as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preset {
    /// Each mode's toolbar, in [`Mode::ALL`]'s order; a mode the preset doesn't name is left as it is on import.
    pub toolbars: Vec<(Mode, Toolbar)>,
    /// Commands the preset named that this version doesn't have, left out.
    pub unknown: usize,
}

/// Why a file isn't a preset this version can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// It isn't JSON.
    NotJson,
    /// It is JSON, but not a toolbar preset.
    NotPreset,
    /// It is a preset of a later format, written by a newer version.
    Newer(u64),
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => f.write_str("not json"),
            Self::NotPreset => f.write_str("not a toolbar preset"),
            Self::Newer(version) => write!(f, "toolbar preset format {version} is newer"),
        }
    }
}

impl std::error::Error for PresetError {}

impl Preset {
    /// The preset as JSON, pretty-printed for reading and sharing.
    #[must_use]
    pub fn to_json(&self) -> String {
        let toolbars: Map<String, Value> = self
            .toolbars
            .iter()
            .map(|(mode, toolbar)| {
                (
                    mode.id().to_owned(),
                    json!(toolbar.words().collect::<Vec<_>>()),
                )
            })
            .collect();
        let preset = json!({ PRESET_KEY: PRESET_VERSION, "toolbars": toolbars });
        serde_json::to_string_pretty(&preset).unwrap_or_default()
    }

    /// The preset in `json`, keeping only the commands `known`. Modes this version doesn't have are skipped.
    ///
    /// # Errors
    ///
    /// When `json` isn't JSON, isn't a toolbar preset, or is of a newer format.
    pub fn from_json(json: &str, known: impl Fn(&str) -> bool) -> Result<Self, PresetError> {
        let value: Value = serde_json::from_str(json).map_err(|_| PresetError::NotJson)?;
        let version = value
            .get(PRESET_KEY)
            .and_then(Value::as_u64)
            .ok_or(PresetError::NotPreset)?;
        if version > PRESET_VERSION {
            return Err(PresetError::Newer(version));
        }
        let toolbars = value
            .get("toolbars")
            .and_then(Value::as_object)
            .ok_or(PresetError::NotPreset)?;
        let mut preset = Self::default();
        for mode in Mode::ALL {
            let Some(words) = toolbars.get(mode.id()).and_then(Value::as_array) else {
                continue;
            };
            let words: Vec<&str> = words.iter().filter_map(Value::as_str).collect();
            preset.unknown += words
                .iter()
                .filter(|&&word| word != SEPARATOR && !known(word))
                .count();
            preset
                .toolbars
                .push((mode, Toolbar::from_words(words, &known)));
        }
        Ok(preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str) -> bool {
        ["zoom_in", "zoom_out", "find", "full_screen"].contains(&name)
    }

    fn bar(text: &str) -> Toolbar {
        Toolbar::parse(text, known)
    }

    #[test]
    fn toolbars_keep_known_commands_once_and_tidy_separators() {
        let toolbar = bar("| zoom_in zoom_in | | gone find | ");
        assert_eq!(toolbar.to_text(), "zoom_in | find");
        assert!(toolbar.contains("find") && !toolbar.contains("gone"));
        assert!(bar("| |").is_empty());
    }

    #[test]
    fn edits_move_add_and_remove_items() {
        let mut toolbar = bar("zoom_in zoom_out | find");
        toolbar.move_item(3, 0);
        assert_eq!(toolbar.to_text(), "find zoom_in zoom_out");
        toolbar.add_separator();
        toolbar.add("full_screen");
        toolbar.add("find");
        assert_eq!(toolbar.to_text(), "find zoom_in zoom_out | full_screen");
        toolbar.remove(4);
        assert_eq!(toolbar.to_text(), "find zoom_in zoom_out");
        toolbar.move_item(0, 99);
        assert_eq!(toolbar.to_text(), "zoom_in zoom_out find");
    }

    #[test]
    fn presets_round_trip_and_leave_out_what_this_version_lacks() {
        let preset = Preset {
            toolbars: vec![
                (Mode::Read, bar("zoom_out zoom_in | find")),
                (Mode::Present, bar("full_screen")),
            ],
            unknown: 0,
        };
        assert_eq!(Preset::from_json(&preset.to_json(), known), Ok(preset));

        let shared = r#"{"ecclesiapdf_toolbars": 1, "toolbars": {"read": ["find", "laser"], "later": ["x"]}}"#;
        let read = Preset::from_json(shared, known).expect("a preset");
        assert_eq!(read.toolbars, vec![(Mode::Read, bar("find"))]);
        assert_eq!(read.unknown, 1);
    }

    #[test]
    fn files_that_are_not_presets_are_refused() {
        assert_eq!(Preset::from_json("{", known), Err(PresetError::NotJson));
        assert_eq!(Preset::from_json("{}", known), Err(PresetError::NotPreset));
        assert_eq!(
            Preset::from_json(r#"{"ecclesiapdf_toolbars": 9, "toolbars": {}}"#, known),
            Err(PresetError::Newer(9))
        );
    }

    #[test]
    fn modes_step_around_those_offered_and_the_pen_asks_for_write() {
        let offered = |mode| mode != Mode::Study;
        assert_eq!(Mode::Write.step(false, offered), Mode::Present);
        assert_eq!(Mode::Present.step(false, offered), Mode::Read);
        assert_eq!(Mode::Read.step(true, offered), Mode::Present);
        assert_eq!(Mode::Read.step(false, |m| m == Mode::Read), Mode::Read);
        assert_eq!(Mode::from_id("present"), Some(Mode::Present));
        assert_eq!(after_pen_contact(Mode::Read, true, true), Mode::Write);
        assert_eq!(after_pen_contact(Mode::Read, false, true), Mode::Read);
        assert_eq!(after_pen_contact(Mode::Read, true, false), Mode::Read);
    }
}
