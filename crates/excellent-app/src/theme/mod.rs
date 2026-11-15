//! Design tokens and themes (plan K.17, §10.3).
//!
//! A theme is a family (`app/themes/<id>.toml`: a hue, a chroma and a tint, with optional overrides) in standard or
//! high contrast, shown light, dark or as the system is. [`palette`] makes a family's tokens: shadcn/ui's, which NSH
//! draws its theme with, plus the app's own (`page-area`, `success`, `warning`, the mode accents and the annotation
//! colours). [`apply`] puts a choice into effect at once, with no restart: every view that reads `theme()` follows,
//! natively. [`picker`] shows the choices and previews each one live as it is highlighted.

mod palette;
mod picker;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use nsh_base::{Theme, theme};
use nsh_core::color::Rgba;
use nsh_core::element::{AdaptiveColor, Tokens, set_color_scheme, set_tokens};
use nsh_core::platform::Appearance;
use serde_json::{Map, Value, json};

pub(crate) use palette::Contrast;
use palette::{Hues, Side};
use picker::{appearance_choice, appearance_id, theme_choice, theme_id};
pub(crate) use picker::{appearance_name, appearance_picker, theme_picker};

use crate::settings::{self, Settings};

/// A built-in theme family.
#[derive(Debug)]
pub(crate) struct Family {
    /// Its stable id: its file's name, and what `settings.toml` keeps.
    pub(crate) id: &'static str,
    /// The localization key of its name.
    name: &'static str,
    /// Its file.
    source: &'static str,
}

/// Every built-in family (plan §10.3), neutral first.
pub(crate) const FAMILIES: [Family; 7] = [
    Family {
        id: "neutral",
        name: "theme.family.neutral",
        source: include_str!("../../../../app/themes/neutral.toml"),
    },
    Family {
        id: "aurora",
        name: "theme.family.aurora",
        source: include_str!("../../../../app/themes/aurora.toml"),
    },
    Family {
        id: "coral",
        name: "theme.family.coral",
        source: include_str!("../../../../app/themes/coral.toml"),
    },
    Family {
        id: "lagoon",
        name: "theme.family.lagoon",
        source: include_str!("../../../../app/themes/lagoon.toml"),
    },
    Family {
        id: "citrus",
        name: "theme.family.citrus",
        source: include_str!("../../../../app/themes/citrus.toml"),
    },
    Family {
        id: "grape",
        name: "theme.family.grape",
        source: include_str!("../../../../app/themes/grape.toml"),
    },
    Family {
        id: "forest",
        name: "theme.family.forest",
        source: include_str!("../../../../app/themes/forest.toml"),
    },
];

impl Family {
    /// The built-in family `id`.
    pub(crate) fn find(id: &str) -> Option<&'static Family> {
        FAMILIES.iter().find(|family| family.id == id)
    }

    /// Its name, in the app's language.
    pub(crate) fn name(&self) -> String {
        nsh_core::t!(self.name)
    }
}

/// What the user chose: a family, its contrast, and light, dark or the system's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThemeChoice {
    pub(crate) family: &'static Family,
    pub(crate) contrast: Contrast,
    pub(crate) appearance: Appearance,
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self {
            family: &FAMILIES[0],
            contrast: Contrast::Standard,
            appearance: Appearance::System,
        }
    }
}

impl PartialEq for Family {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Family {}

impl ThemeChoice {
    /// The choice kept in `settings` (keys `theme` and `appearance`); a value this version doesn't know is logged
    /// and the default used.
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        let mut choice = Self::default();
        if let Some(id) = settings.text(THEME_KEY) {
            match theme_choice(id, choice) {
                Some(theme) => choice = theme,
                None => log::warn!("settings: no theme `{id}`; using the default"),
            }
        }
        if let Some(id) = settings.text(APPEARANCE_KEY) {
            match appearance_choice(id, choice) {
                Some(appearance) => choice = appearance,
                None => log::warn!("settings: no appearance `{id}`; using the system's"),
            }
        }
        choice
    }

    /// Keeps the choice in `settings`, writing only what differs from the default.
    pub(crate) fn store(self, settings: &mut Settings) {
        let default = Self::default();
        let theme = theme_id(self);
        let appearance = appearance_id(self);
        settings.set_text(
            THEME_KEY,
            (theme != theme_id(default)).then_some(theme.as_str()),
        );
        settings.set_text(
            APPEARANCE_KEY,
            (appearance != appearance_id(default)).then_some(appearance.as_str()),
        );
    }
}

/// The settings keys of the choice.
const THEME_KEY: &str = "theme";
const APPEARANCE_KEY: &str = "appearance";

/// The choice the user kept last time (K.16's settings file).
pub(crate) fn saved() -> ThemeChoice {
    ThemeChoice::from_settings(&settings::shared())
}

/// Keeps `choice` for next time.
pub(crate) fn save(choice: ThemeChoice) {
    let mut settings = settings::shared();
    choice.store(&mut settings);
    settings.save();
}

/// Puts `choice` into effect now: the window and everything in it change, with no restart.
pub(crate) fn apply(choice: ThemeChoice) {
    set_tokens(tokens(choice));
    set_color_scheme(choice.appearance);
}

/// The tokens `choice` draws with (for the root, before the window shows): its high-contrast side while the
/// system's high contrast is followed and on.
pub(crate) fn tokens(choice: ThemeChoice) -> Tokens {
    let contrast = if SYSTEM_HIGH.load(Ordering::Relaxed) {
        Contrast::High
    } else {
        choice.contrast
    };
    theme_of(choice.family, contrast).into_tokens()
}

/// The setting (on by default) that puts every theme on its high-contrast side while the system's is on (U.4).
pub(crate) const FOLLOW_SYSTEM_CONTRAST: &str = "follow_system_contrast";

/// Whether the system's high contrast is in force now ([`follow_system`] keeps it).
static SYSTEM_HIGH: AtomicBool = AtomicBool::new(false);

/// Reads the system's high contrast, as `settings` allow; `true` when that changed what [`tokens`] gives, so the
/// theme should be applied again.
pub(crate) fn follow_system(settings: &Settings) -> bool {
    let on = settings.flag(FOLLOW_SYSTEM_CONTRAST).unwrap_or(true)
        && excellent_platform::display::high_contrast();
    SYSTEM_HIGH.swap(on, Ordering::Relaxed) != on
}

/// The page area's colour now, `#rrggbb`, for the page canvas, which draws it itself (reading it subscribes).
pub(crate) fn page_area_hex() -> String {
    hex(theme().color("page-area").unwrap_or(theme().muted))
}

/// `colour` as it shows in the current appearance, `#rrggbb`, for the page canvas, which draws in plain RGB.
pub(crate) fn hex(colour: AdaptiveColor) -> String {
    let [r, g, b] = rgb(colour);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `colour`, mixed toward `text` just enough to read at WCAG AA (4.5:1) on each of `papers`; `text` itself must pass.
/// A document's own colours (outline entries, B.7) are kept as near as the theme allows.
pub(crate) fn readable(colour: [u8; 3], papers: &[[u8; 3]], text: [u8; 3]) -> [u8; 3] {
    let passes = |c: [u8; 3]| {
        papers
            .iter()
            .all(|paper| excellent_core::darkmode::themes::contrast(c, *paper) >= 4.5)
    };
    (0..=10u8)
        .map(|step| {
            let t = f32::from(step) / 10.0;
            std::array::from_fn(|i| {
                let (from, to) = (f32::from(colour[i]), f32::from(text[i]));
                (from + (to - from) * t).round() as u8
            })
        })
        .find(|c| passes(*c))
        .unwrap_or(text)
}

/// `colour` as it shows in the current appearance, as bytes.
pub(crate) fn rgb(colour: AdaptiveColor) -> [u8; 3] {
    let Rgba { r, g, b, .. } = nsh_core::element::color(colour).to_rgb();
    [r, g, b].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

thread_local! {
    /// Themes made so far: a picker's swatches and previews ask again and again.
    static MADE: RefCell<Vec<(&'static str, Contrast, Theme)>> = const { RefCell::new(Vec::new()) };
}

/// The theme of `family` in `contrast`. A family that doesn't build (only a broken built-in file could) logs why
/// and shows NSH's default.
pub(crate) fn theme_of(family: &'static Family, contrast: Contrast) -> Theme {
    let made = MADE.with_borrow(|made| {
        made.iter()
            .find(|(id, c, _)| *id == family.id && *c == contrast)
            .map(|(_, _, theme)| theme.clone())
    });
    if let Some(theme) = made {
        return theme;
    }
    let theme = match build(family.source, contrast) {
        Ok((theme, ignored)) => {
            for what in ignored {
                log::warn!("theme {}: {what}, ignored", family.id);
            }
            theme
        }
        Err(err) => {
            log::error!("theme {}: {err}", family.id);
            Theme::default()
        }
    };
    MADE.with_borrow_mut(|made| made.push((family.id, contrast, theme.clone())));
    theme
}

/// A family's file, read.
struct FamilyFile {
    hues: Hues,
    /// Tokens it sets itself, per variant.
    overrides: Vec<(Contrast, Side, Map<String, Value>)>,
}

/// Reads a family's file: `hue`, `chroma` and `tint`, and `[light]`, `[dark]`, `[contrast.light]` and
/// `[contrast.dark]` tables of CSS colours.
fn parse(source: &str) -> Result<FamilyFile, String> {
    let table: toml::Table = source.parse().map_err(|err| format!("{err}"))?;
    let number = |key: &str| -> Result<f32, String> {
        match table.get(key) {
            Some(toml::Value::Float(n)) => Ok(*n as f32),
            Some(toml::Value::Integer(n)) => Ok(*n as f32),
            _ => Err(format!("`{key}` must be a number")),
        }
    };
    let hues = Hues {
        hue: number("hue")?,
        chroma: number("chroma")?,
        tint: number("tint")?.clamp(0.0, 1.0),
    };
    let colours = |value: &toml::Value, at: &str| -> Result<Map<String, Value>, String> {
        let toml::Value::Table(table) = value else {
            return Err(format!("`{at}` must be a table"));
        };
        table
            .iter()
            .map(|(token, colour)| match colour {
                toml::Value::String(css) => Ok((token.clone(), Value::String(css.clone()))),
                _ => Err(format!("`{at}.{token}` must be a CSS colour")),
            })
            .collect()
    };
    let mut overrides = Vec::new();
    for (key, value) in &table {
        match key.as_str() {
            "hue" | "chroma" | "tint" => {}
            "light" => overrides.push((Contrast::Standard, Side::Light, colours(value, key)?)),
            "dark" => overrides.push((Contrast::Standard, Side::Dark, colours(value, key)?)),
            "contrast" => {
                let toml::Value::Table(sides) = value else {
                    return Err("`contrast` must hold [contrast.light] and [contrast.dark]".into());
                };
                for (side, value) in sides {
                    let side_of = match side.as_str() {
                        "light" => Side::Light,
                        "dark" => Side::Dark,
                        _ => return Err(format!("unknown table `contrast.{side}`")),
                    };
                    overrides.push((
                        Contrast::High,
                        side_of,
                        colours(value, &format!("contrast.{side}"))?,
                    ));
                }
            }
            _ => return Err(format!("unknown key `{key}`")),
        }
    }
    Ok(FamilyFile { hues, overrides })
}

/// The theme of a family's file in `contrast`, with the tokens NSH could not read.
fn build(source: &str, contrast: Contrast) -> Result<(Theme, Vec<String>), String> {
    let file = parse(source)?;
    let side = |side: Side| {
        let mut tokens: Map<String, Value> = palette::tokens(file.hues, contrast, side)
            .into_iter()
            .map(|(name, css)| (name.to_owned(), Value::String(css)))
            .collect();
        for (c, s, overrides) in &file.overrides {
            if (*c, *s) == (contrast, side) {
                tokens.extend(overrides.clone());
            }
        }
        Value::Object(tokens)
    };
    let common: Map<String, Value> = palette::ANNOTATION
        .iter()
        .map(|(name, css)| ((*name).to_owned(), Value::String((*css).to_owned())))
        .collect();
    let css_vars = json!({
        "cssVars": { "theme": common, "light": side(Side::Light), "dark": side(Side::Dark) }
    });
    Theme::from_json_lenient(&css_vars.to_string()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use nsh_core::color::Hsla;

    use super::*;

    /// WCAG's relative luminance of an opaque colour.
    fn luminance(colour: Hsla) -> f32 {
        let Rgba { r, g, b, a } = colour.to_rgb();
        assert!(
            a > 0.999,
            "contrast is checked on opaque colours only: {colour:?}"
        );
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    #[test]
    fn readable_keeps_a_colour_that_passes_and_mixes_one_that_does_not() {
        use excellent_core::darkmode::themes::contrast;
        let (white, black) = ([0xff; 3], [0x11; 3]);
        let blue = [0x1f, 0x4f, 0xb6];
        assert_eq!(readable(blue, &[white], black), blue);
        let mixed = readable(blue, &[[0x1e; 3]], [0xee; 3]);
        assert_ne!(mixed, blue);
        assert!(contrast(mixed, [0x1e; 3]) >= 4.5);
    }

    fn ratio(a: Hsla, b: Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Text and what it is drawn on, as the chrome pairs them.
    const TEXT: &[(&str, &str)] = &[
        ("foreground", "background"),
        ("foreground", "sidebar"),
        ("card-foreground", "card"),
        ("popover-foreground", "popover"),
        ("primary-foreground", "primary"),
        ("secondary-foreground", "secondary"),
        ("muted-foreground", "background"),
        ("muted-foreground", "muted"),
        ("muted-foreground", "sidebar"),
        ("muted-foreground", "card"),
        ("muted-foreground", "popover"),
        ("muted-foreground", "accent"),
        ("accent-foreground", "accent"),
        ("sidebar-foreground", "sidebar"),
        ("sidebar-accent-foreground", "sidebar-accent"),
        ("sidebar-primary-foreground", "sidebar-primary"),
        ("destructive", "background"),
        ("destructive", "card"),
        ("success-foreground", "success"),
        ("warning-foreground", "warning"),
    ];

    /// Controls and focus indicators and what they are drawn on (WCAG 1.4.11).
    const CONTROLS: &[(&str, &str)] = &[
        ("ring", "background"),
        ("ring", "sidebar"),
        ("primary", "background"),
        ("input", "background"),
        ("input", "card"),
    ];

    #[test]
    fn every_theme_passes_contrast() {
        let mut failures = Vec::new();
        for family in &FAMILIES {
            for contrast in [Contrast::Standard, Contrast::High] {
                let (theme, ignored) =
                    build(family.source, contrast).expect("built-in families build");
                assert!(ignored.is_empty(), "{}: {ignored:?}", family.id);
                let (text, control) = match contrast {
                    Contrast::Standard => (4.5, 3.0),
                    Contrast::High => (7.0, 4.5),
                };
                let modes = palette::MODES.iter().flat_map(|(mode, on_mode, _)| {
                    [
                        (*on_mode, *mode, text),
                        (*mode, "background", control),
                        (*mode, "sidebar", control),
                    ]
                });
                let pairs = TEXT
                    .iter()
                    .map(|&(fg, bg)| (fg, bg, text))
                    .chain(CONTROLS.iter().map(|&(fg, bg)| (fg, bg, control)))
                    .chain(modes);
                for (fg, bg, need) in pairs {
                    let colour = |name: &str| {
                        theme
                            .color(name)
                            .unwrap_or_else(|| panic!("no token {name}"))
                    };
                    for (side, pick) in [("light", 0), ("dark", 1)] {
                        let at = |c: nsh_core::element::AdaptiveColor| {
                            if pick == 0 { c.light } else { c.dark }
                        };
                        let got = ratio(at(colour(fg)), at(colour(bg)));
                        if got < need {
                            failures.push(format!(
                                "{} {contrast:?} {side}: {fg} on {bg} is {got:.2}:1, needs {need}:1",
                                family.id
                            ));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn the_choice_round_trips_through_settings_and_the_default_writes_nothing() {
        let mut settings = Settings::default();
        let choice = ThemeChoice {
            family: Family::find("grape").expect("a built-in family"),
            contrast: Contrast::High,
            appearance: Appearance::Dark,
        };
        choice.store(&mut settings);
        assert_eq!(settings.text(THEME_KEY), Some("grape:high"));
        assert_eq!(ThemeChoice::from_settings(&settings), choice);
        ThemeChoice::default().store(&mut settings);
        assert_eq!(
            (settings.text(THEME_KEY), settings.text(APPEARANCE_KEY)),
            (None, None)
        );
        settings.set_text(THEME_KEY, Some("plaid"));
        assert_eq!(
            ThemeChoice::from_settings(&settings),
            ThemeChoice::default()
        );
    }

    #[test]
    fn families_have_unique_ids_and_english_names() {
        for (i, family) in FAMILIES.iter().enumerate() {
            assert_eq!(Family::find(family.id).map(|f| f.id), Some(family.id));
            assert!(
                FAMILIES[..i].iter().all(|other| other.id != family.id),
                "{}",
                family.id
            );
            assert!(
                crate::l10n::EN.contains(&format!("\"{}\":", family.name)),
                "{} is missing from app/locales/en.json",
                family.name
            );
        }
    }

    #[test]
    fn overrides_win_and_bad_files_are_refused() {
        let (theme, _) = build(FAMILIES[0].source, Contrast::Standard).expect("neutral builds");
        let primary = theme.color("primary").expect("primary");
        assert!(
            luminance(primary.light) < 0.05,
            "neutral's [light] primary is near black"
        );
        assert!(parse("hue = 1\nchroma = 0\ntint = 0\nmood = 3").is_err());
        assert!(parse("hue = 1\nchroma = 0\ntint = 0\n[light]\nprimary = 3").is_err());
        assert!(parse("hue = 1\nchroma = 0").is_err());
        assert!(
            parse("hue = 1\nchroma = 0\ntint = 0\n[contrast.sepia]\nprimary = \"red\"").is_err()
        );
    }
}
