//! Favourite pens (H.7, plan §8.4): a bar of saved pens, each a kind, a colour, a width and how it answers pressure.
//! One is in the hand; the reader picks another, adds the one in hand, edits, removes or reorders them, and can take
//! the bar to another machine as a JSON file. The bar lives in `settings.toml` (`[ink]`), so it travels with the
//! settings wherever they sync (Y.2).

use std::fmt;

use ecclesia_ink::pens::{DEFAULT_SIZE, HIGHLIGHTER, INK, Pen, PenKind};
use serde_json::{Value, json};

/// The most pens the bar holds: a toolbar's width of them.
pub const MAX: usize = 12;

/// The widths a pen may have, in points.
pub const SIZES: (f32, f32) = (0.25, 24.0);

/// The version an exported bar says it is, so a later format can still read this one.
const FORMAT: u64 = 1;

/// How a pen answers pressure: its width curve's exponent made smaller (a light touch already writes broad) or larger
/// (only a firm hand does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Response {
    /// Broad with little pressure.
    Light,
    /// The pen kind's own curve.
    #[default]
    Normal,
    /// Broad only under a firm hand.
    Firm,
}

impl Response {
    /// Every response, lightest first.
    pub const ALL: [Self; 3] = [Self::Light, Self::Normal, Self::Firm];

    /// Its name in settings and files.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Normal => "normal",
            Self::Firm => "firm",
        }
    }

    /// The response named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }

    /// What it multiplies the pen's width curve's exponent by.
    #[must_use]
    pub fn exponent(self) -> f32 {
        match self {
            Self::Light => 0.6,
            Self::Normal => 1.0,
            Self::Firm => 1.6,
        }
    }
}

/// One saved pen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Favorite {
    /// Its kind (H.2): width curve, texture and blend.
    pub kind: PenKind,
    /// Its colour, `0xRRGGBB`.
    pub color: u32,
    /// Its width at its widest, in points.
    pub size: f32,
    /// How it answers pressure.
    pub response: Response,
}

impl Favorite {
    /// A pen of `kind` as it comes: the kind's own colour, the default width and response.
    #[must_use]
    pub fn of(kind: PenKind) -> Self {
        Self {
            kind,
            color: kind.pen(DEFAULT_SIZE).color,
            size: DEFAULT_SIZE,
            response: Response::Normal,
        }
    }

    /// The pen it draws with.
    #[must_use]
    pub fn pen(&self) -> Pen {
        let mut pen = self.kind.pen(self.size);
        pen.color = self.color;
        pen.curve.exponent *= self.response.exponent();
        pen
    }

    fn to_json(self) -> Value {
        json!({
            "kind": self.kind.to_string(),
            "color": format!("#{:06x}", self.color),
            "size": self.size,
            "response": self.response.name(),
        })
    }

    /// The pen `value` describes; `None` without a known kind. A colour, width or response that is missing or wrong
    /// comes as the kind has it.
    fn from_json(value: &Value) -> Option<Self> {
        let kind = PenKind::parse(value.get("kind")?.as_str()?)?;
        let base = Self::of(kind);
        let color = value
            .get("color")
            .and_then(Value::as_str)
            .and_then(|text| text.strip_prefix('#'))
            .filter(|hex| hex.len() == 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .unwrap_or(base.color);
        let size = value
            .get("size")
            .and_then(Value::as_f64)
            .map_or(base.size, |size| (size as f32).clamp(SIZES.0, SIZES.1));
        let response = value
            .get("response")
            .and_then(Value::as_str)
            .and_then(Response::parse)
            .unwrap_or_default();
        Some(Self {
            kind,
            color,
            size,
            response,
        })
    }
}

/// The bar: the saved pens in order, and the one in hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Favorites {
    pens: Vec<Favorite>,
    chosen: usize,
}

impl Default for Favorites {
    /// A fountain pen in blue ink, a black ballpoint, a pencil and a highlighter.
    fn default() -> Self {
        Self {
            pens: vec![
                Favorite {
                    color: INK,
                    ..Favorite::of(PenKind::Fountain)
                },
                Favorite {
                    color: 0x1f1f1f,
                    size: 1.6,
                    ..Favorite::of(PenKind::Ballpoint)
                },
                Favorite::of(PenKind::Pencil),
                Favorite {
                    color: HIGHLIGHTER,
                    ..Favorite::of(PenKind::Highlighter)
                },
            ],
            chosen: 0,
        }
    }
}

/// Why an exported bar couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// It isn't JSON.
    NotJson,
    /// It is JSON, but not a bar of pens (no `pens` list).
    NotPens,
    /// A format newer than this app reads.
    Newer(u64),
    /// It holds no pen this app knows.
    Empty,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => f.write_str("the file isn't JSON"),
            Self::NotPens => f.write_str("the file holds no list of pens"),
            Self::Newer(version) => {
                write!(f, "the file is format {version}, newer than this app reads")
            }
            Self::Empty => f.write_str("the file holds no pen this app knows"),
        }
    }
}

impl std::error::Error for ImportError {}

impl Favorites {
    /// The saved pens, in the bar's order.
    #[must_use]
    pub fn pens(&self) -> &[Favorite] {
        &self.pens
    }

    /// Which one is in hand.
    #[must_use]
    pub fn chosen(&self) -> usize {
        self.chosen
    }

    /// The pen in hand.
    #[must_use]
    pub fn in_hand(&self) -> Favorite {
        self.pens[self.chosen]
    }

    /// Takes pen `index` in hand; nothing when there is none.
    pub fn choose(&mut self, index: usize) {
        if index < self.pens.len() {
            self.chosen = index;
        }
    }

    /// Takes the first saved pen of `kind` in hand, or, with none, saves one as it comes and takes it (the pen
    /// commands of H.2). Returns whether the bar changed.
    pub fn choose_kind(&mut self, kind: PenKind) -> bool {
        if let Some(index) = self.pens.iter().position(|pen| pen.kind == kind) {
            self.chosen = index;
            return false;
        }
        self.add(Favorite::of(kind)).is_some()
    }

    /// Saves `pen` at the end of the bar and takes it in hand; `None` when the bar is full.
    pub fn add(&mut self, pen: Favorite) -> Option<usize> {
        if self.pens.len() >= MAX {
            return None;
        }
        self.pens.push(pen);
        self.chosen = self.pens.len() - 1;
        Some(self.chosen)
    }

    /// Changes pen `index` to `pen`; nothing when there is none.
    pub fn set(&mut self, index: usize, pen: Favorite) {
        if let Some(slot) = self.pens.get_mut(index) {
            *slot = Favorite {
                size: pen.size.clamp(SIZES.0, SIZES.1),
                ..pen
            };
        }
    }

    /// Removes pen `index`; the last pen stays, so there is always one in hand.
    pub fn remove(&mut self, index: usize) {
        if self.pens.len() <= 1 || index >= self.pens.len() {
            return;
        }
        self.pens.remove(index);
        if self.chosen > index || self.chosen >= self.pens.len() {
            self.chosen = self.chosen.saturating_sub(1);
        }
    }

    /// Moves pen `from` to place `to` (a drag along the bar), the pen in hand kept in hand.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.pens.len() {
            return;
        }
        let to = to.min(self.pens.len() - 1);
        let in_hand = self.pens[self.chosen];
        let held = self.chosen == from;
        let pen = self.pens.remove(from);
        self.pens.insert(to, pen);
        self.chosen = if held {
            to
        } else {
            self.pens
                .iter()
                .position(|p| *p == in_hand)
                .unwrap_or_default()
        };
    }

    /// The bar as settings keep it: `{chosen, pens: [{kind, color, size, response}]}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "chosen": self.chosen,
            "pens": self.pens.iter().map(|pen| pen.to_json()).collect::<Vec<_>>(),
        })
    }

    /// The bar `value` holds, as [`Favorites::to_json`] writes it; pens it can't read are left out, and the default
    /// bar comes back when none is left.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        let pens: Vec<Favorite> = value
            .get("pens")
            .and_then(Value::as_array)
            .map(|pens| {
                pens.iter()
                    .filter_map(Favorite::from_json)
                    .take(MAX)
                    .collect()
            })
            .unwrap_or_default();
        if pens.is_empty() {
            return Self::default();
        }
        let chosen = value
            .get("chosen")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n < pens.len())
            .unwrap_or_default();
        Self { pens, chosen }
    }

    /// The bar as a file to take elsewhere: JSON with its format's version.
    #[must_use]
    pub fn export(&self) -> String {
        let file = json!({
            "format": FORMAT,
            "app": "EcclesiaPDF",
            "pens": self.pens.iter().map(|pen| pen.to_json()).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// The bar a file [`Favorites::export`] wrote holds, its first pen in hand.
    ///
    /// # Errors
    ///
    /// The text isn't JSON, holds no list of pens, is a newer format, or holds no pen this app knows.
    pub fn import(text: &str) -> Result<Self, ImportError> {
        let value: Value = serde_json::from_str(text).map_err(|_| ImportError::NotJson)?;
        let version = value
            .get("format")
            .and_then(Value::as_u64)
            .unwrap_or(FORMAT);
        if version > FORMAT {
            return Err(ImportError::Newer(version));
        }
        let list = value
            .get("pens")
            .and_then(Value::as_array)
            .ok_or(ImportError::NotPens)?;
        let pens: Vec<Favorite> = list
            .iter()
            .filter_map(Favorite::from_json)
            .take(MAX)
            .collect();
        if pens.is_empty() {
            return Err(ImportError::Empty);
        }
        Ok(Self { pens, chosen: 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_goes_through_settings_unchanged() {
        let mut bar = Favorites::default();
        bar.add(Favorite {
            kind: PenKind::Brush,
            color: 0xaa3300,
            size: 6.5,
            response: Response::Firm,
        });
        bar.move_to(4, 1);
        // As settings.toml keeps it: through TOML and back.
        let toml = toml::Value::try_from(bar.to_json()).expect("as TOML");
        let back: Value = serde_json::to_value(toml).expect("as JSON");
        assert_eq!(Favorites::from_json(&back), bar);
    }

    #[test]
    fn export_and_import_give_the_same_pens() {
        let bar = Favorites::default();
        let file = bar.export();
        let read = Favorites::import(&file).expect("imports");
        assert_eq!(read.pens(), bar.pens());
        assert_eq!(Favorites::import("not json"), Err(ImportError::NotJson));
        assert_eq!(Favorites::import("{}"), Err(ImportError::NotPens));
        assert_eq!(
            Favorites::import(r#"{"format": 9, "pens": []}"#),
            Err(ImportError::Newer(9))
        );
        assert_eq!(
            Favorites::import(r#"{"pens": [{"kind": "quill"}]}"#),
            Err(ImportError::Empty)
        );
    }

    #[test]
    fn reordering_keeps_the_pen_in_hand() {
        let mut bar = Favorites::default();
        bar.choose(2);
        let in_hand = bar.in_hand();
        bar.move_to(0, 3);
        assert_eq!(bar.in_hand(), in_hand, "another pen moved");
        bar.move_to(bar.chosen(), 0);
        assert_eq!(
            (bar.chosen(), bar.in_hand()),
            (0, in_hand),
            "the pen in hand moved"
        );
    }

    #[test]
    fn the_bar_keeps_one_pen_and_at_most_max() {
        let mut bar = Favorites::default();
        while bar.pens().len() > 1 {
            bar.remove(0);
        }
        bar.remove(0);
        assert_eq!(bar.pens().len(), 1, "the last pen stays");
        for _ in 0..MAX {
            bar.add(Favorite::of(PenKind::Marker));
        }
        assert_eq!(bar.pens().len(), MAX);
        assert_eq!(bar.add(Favorite::of(PenKind::Marker)), None, "full");
    }

    #[test]
    fn a_kind_command_takes_its_first_pen_or_saves_one() {
        let mut bar = Favorites::default();
        assert!(!bar.choose_kind(PenKind::Pencil));
        assert_eq!(bar.in_hand().kind, PenKind::Pencil);
        assert!(bar.choose_kind(PenKind::Calligraphy), "saved as it comes");
        assert_eq!(bar.in_hand(), Favorite::of(PenKind::Calligraphy));
    }

    #[test]
    fn a_pen_takes_its_colour_and_response() {
        let firm = Favorite {
            response: Response::Firm,
            color: 0x123456,
            ..Favorite::of(PenKind::Fountain)
        };
        let normal = Favorite::of(PenKind::Fountain).pen();
        assert_eq!(firm.pen().color, 0x123456);
        assert!(firm.pen().curve.exponent > normal.curve.exponent);
        let wrong = Favorite::from_json(&json!({"kind": "pencil", "size": 900, "color": "red"}))
            .expect("a known kind");
        assert_eq!(wrong.size, SIZES.1, "clamped");
        assert_eq!(
            wrong.color,
            Favorite::of(PenKind::Pencil).color,
            "the kind's own"
        );
    }
}
