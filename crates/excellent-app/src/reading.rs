//! The reading aids' settings (R.19-R.21), the app's side: the page canvas draws the reading ruler, focus mode and
//! auto-scroll itself; the app hands it their settings and keeps what the reader changes.
//!
//! Settings: `reading_ruler_height` (pixels at 96 dpi), `reading_ruler_color` (`#rrggbb`), `reading_ruler_opacity`
//! (0 to 1), `reading_ruler_invert` (yes or no), `focus_scope` (`"paragraph"` or `"line"`) and `auto_scroll_speed`
//! (pixels a second at 96 dpi). Each missing or wrong one is its default; the canvas brings each into range.
//!
//! Smart dark mode (R.14) and the reading themes (R.15) are here too, as the canvas recolours the pages:
//! `document_colors` is `"off"` (the document's own colours), `"smart"` (photos and artwork kept) or `"all"`, and
//! `reading_theme` picks the ink and paper ([`ReadingTheme::id`]; `"app"`, the app theme's, by default), with
//! `reading_ink` and `reading_paper` (`#rrggbb`) for `"custom"`.
//!
//! Books that reflow (R.28: EPUB, FB2, MOBI, HTML, Markdown) are laid out with their text at `reflow_font_size`
//! points.
//!
//! Citation and equation previews (R.25) show after the pointer rests `ref_preview_delay` milliseconds on a link or
//! citation, unless `ref_preview` is no.

use excellent_core::darkmode::themes::is_dark;
use excellent_core::darkmode::{Palette, Pictures, ReadingTheme, Rgb};
use excellent_features::read::autoscroll::DEFAULT_SPEED;
use nsh_base::theme;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::settings::{self, Live};
use crate::theme as themes;

pub(crate) const RULER_HEIGHT: &str = "reading_ruler_height";
pub(crate) const RULER_COLOR: &str = "reading_ruler_color";
pub(crate) const RULER_OPACITY: &str = "reading_ruler_opacity";
pub(crate) const RULER_INVERT: &str = "reading_ruler_invert";
pub(crate) const FOCUS_SCOPE: &str = "focus_scope";
pub(crate) const AUTO_SCROLL_SPEED: &str = "auto_scroll_speed";
pub(crate) const DOCUMENT_COLORS: &str = "document_colors";
pub(crate) const READING_THEME: &str = "reading_theme";
pub(crate) const READING_INK: &str = "reading_ink";
pub(crate) const READING_PAPER: &str = "reading_paper";
pub(crate) const REFLOW_FONT_SIZE: &str = "reflow_font_size";
pub(crate) const REF_PREVIEW: &str = "ref_preview";
pub(crate) const REF_PREVIEW_DELAY: &str = "ref_preview_delay";
/// How long the pointer rests before a preview shows, in milliseconds: Enhanced's `CitationHoverDelay`.
pub(crate) const DEFAULT_PREVIEW_DELAY: f64 = 300.0;
/// The longest delay the settings offer, in milliseconds.
pub(crate) const MAX_PREVIEW_DELAY: f64 = 2000.0;

/// The reading aids' settings, followed while the app runs, and the reading theme shown for a moment while the reader
/// moves through the reading-theme bar.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reading {
    live: Live,
    preview: Signal<Option<ReadingTheme>>,
}

impl Reading {
    pub(crate) fn new(live: Live) -> Self {
        Self {
            live,
            preview: signal(None),
        }
    }

    /// The canvas's `readingRuler` prop, `{height, color, opacity, invert}`: the values the settings hold.
    pub(crate) fn ruler_prop(self) -> JsonValue {
        self.live.read(|settings| {
            let mut ruler = serde_json::Map::new();
            let invert = settings.flag(RULER_INVERT).unwrap_or(false);
            ruler.insert("invert".into(), invert.into());
            for key in [RULER_HEIGHT, RULER_OPACITY] {
                if let Some(value) = settings.number(key) {
                    ruler.insert(prop_name(key).into(), value.into());
                }
            }
            if let Some(color) = settings.text(RULER_COLOR) {
                ruler.insert("color".into(), color.into());
            }
            JsonValue::Object(ruler)
        })
    }

    /// The canvas's `focusScope` prop: the setting, or `null` for the default.
    pub(crate) fn focus_scope_prop(self) -> JsonValue {
        self.live
            .read(|settings| settings.text(FOCUS_SCOPE).map(JsonValue::from))
            .unwrap_or(JsonValue::Null)
    }

    /// The canvas's `autoScrollSpeed` prop: the setting, or `null` for the default.
    pub(crate) fn speed_prop(self) -> JsonValue {
        self.live
            .read(|settings| settings.number(AUTO_SCROLL_SPEED))
            .map_or(JsonValue::Null, JsonValue::from)
    }

    /// The canvas's `reflowFontSize` prop (R.28): the setting, or `null` for the default.
    pub(crate) fn reflow_font_size_prop(self) -> JsonValue {
        self.live
            .read(|settings| settings.number(REFLOW_FONT_SIZE))
            .map_or(JsonValue::Null, JsonValue::from)
    }

    /// The canvas's `hoverDelay` prop (R.25): the delay in milliseconds, or `false` when previews are off.
    pub(crate) fn hover_delay_prop(self) -> JsonValue {
        self.live.read(|settings| {
            if settings.flag(REF_PREVIEW) == Some(false) {
                return JsonValue::Bool(false);
            }
            let delay = settings
                .number(REF_PREVIEW_DELAY)
                .unwrap_or(DEFAULT_PREVIEW_DELAY);
            delay.clamp(0.0, MAX_PREVIEW_DELAY).into()
        })
    }

    /// How the pages take the theme's colours now (R.14).
    pub(crate) fn document_colors(self) -> DocumentColors {
        self.live.read(|settings| {
            settings
                .text(DOCUMENT_COLORS)
                .and_then(DocumentColors::of)
                .unwrap_or_default()
        })
    }

    /// Keeps `colors` as the way the pages take the theme's colours.
    pub(crate) fn set_document_colors(self, colors: DocumentColors) {
        self.live.change(|settings| {
            let is_default = colors == DocumentColors::default();
            settings.set_text(DOCUMENT_COLORS, (!is_default).then_some(colors.id()));
        });
    }

    /// The pages in the theme's colours (keeping pictures), or in the document's own again.
    pub(crate) fn toggle_document_colors(self) {
        self.set_document_colors(match self.document_colors() {
            DocumentColors::Own => DocumentColors::Smart,
            DocumentColors::Smart | DocumentColors::Everything => DocumentColors::Own,
        });
    }

    /// The reading theme kept in the settings (R.15).
    pub(crate) fn reading_theme(self) -> ReadingTheme {
        self.live.read(|settings| {
            settings
                .text(READING_THEME)
                .and_then(ReadingTheme::parse)
                .unwrap_or_default()
        })
    }

    /// Keeps `reading_theme` as the reading theme. Choosing one says the reader wants the pages in it, so pages in
    /// the document's own colours switch to smart.
    pub(crate) fn set_reading_theme(self, reading_theme: ReadingTheme) {
        let is_default = reading_theme == ReadingTheme::default();
        let own = untracked(|| self.document_colors()) == DocumentColors::Own;
        self.live.change(|settings| {
            settings.set_text(READING_THEME, (!is_default).then_some(reading_theme.id()));
            if own {
                settings.set_text(DOCUMENT_COLORS, Some(DocumentColors::Smart.id()));
            }
        });
    }

    /// Shows `reading_theme` on the pages without keeping it (`None`: the kept one again).
    pub(crate) fn preview(self, reading_theme: Option<ReadingTheme>) {
        self.preview.set(reading_theme);
    }

    /// The colours of `theme`: the app theme's text, background and (on dark paper) primary colour, the reader's own,
    /// or the built-in ones.
    pub(crate) fn palette(self, reading_theme: ReadingTheme) -> Palette {
        match reading_theme {
            ReadingTheme::App => {
                let paper = themes::rgb(theme().background);
                Palette {
                    ink: themes::rgb(theme().foreground),
                    paper,
                    link: is_dark(paper).then(|| themes::rgb(theme().primary)),
                }
            }
            ReadingTheme::Custom => self.custom_palette(),
            built_in => built_in
                .palette()
                .expect("every other theme has colours of its own"),
        }
    }

    /// The reader's own ink and paper; each one unset or not `#rrggbb` is the light theme's.
    pub(crate) fn custom_palette(self) -> Palette {
        let light = ReadingTheme::Light
            .palette()
            .expect("the light theme has colours of its own");
        let (ink, paper) = self.live.read(|settings| {
            let color = |key| settings.text(key).and_then(parse_hex);
            (color(READING_INK), color(READING_PAPER))
        });
        Palette::custom(ink.unwrap_or(light.ink), paper.unwrap_or(light.paper))
    }

    /// Keeps `color` as the custom theme's ink or paper (`key`), `None` to forget it.
    pub(crate) fn set_custom_color(self, key: &'static str, color: Option<Rgb>) {
        self.live
            .change(|settings| settings.set_text(key, color.map(hex).as_deref()));
    }

    /// The canvas's `pageColors` prop: `null` for the document's own colours, else `{ink, paper, link, pictures}`
    /// from the reading theme (the one previewed, while the reader moves through the bar). Reading it subscribes to
    /// the settings, the preview and the app theme.
    pub(crate) fn page_colors_prop(self) -> JsonValue {
        let preview = self.preview.get();
        let pictures = match (self.document_colors(), preview) {
            (DocumentColors::Own, None) => return JsonValue::Null,
            (DocumentColors::Own | DocumentColors::Smart, _) => Pictures::Keep,
            (DocumentColors::Everything, _) => Pictures::Recolor,
        };
        let colors = self
            .palette(preview.unwrap_or_else(|| self.reading_theme()))
            .page_colors(pictures);
        let mut prop = serde_json::json!({
            "ink": hex(colors.ink),
            "paper": hex(colors.paper),
            "pictures": match colors.pictures {
                Pictures::Keep => "keep",
                Pictures::Recolor => "recolor",
            },
        });
        if let Some(link) = colors.link {
            prop["link"] = hex(link).into();
        }
        prop
    }

    /// Switches between tinting the band and dimming the page around it, and keeps the choice.
    pub(crate) fn toggle_invert(self) {
        self.live.change(|settings| {
            let invert = !settings.flag(RULER_INVERT).unwrap_or(false);
            settings.set_flag(RULER_INVERT, invert.then_some(true));
        });
    }

    /// Keeps the auto-scroll speed the reader picked (the canvas's `autoScrollSpeed` event). The canvas has it
    /// already, so nothing that shows the settings needs telling.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name != "autoScrollSpeed" {
            return;
        }
        if let Some(speed) = event.data.get("speed").and_then(JsonValue::as_f64) {
            let mut settings = settings::shared();
            let changed = speed != f64::from(DEFAULT_SPEED);
            settings.set_number(AUTO_SCROLL_SPEED, changed.then_some(speed));
            settings.save();
        }
    }
}

/// How the pages take the theme's colours (R.14), as `document_colors` keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DocumentColors {
    /// The document's own colours.
    #[default]
    Own,
    /// The theme's text and background; photos and artwork keep their colours (Enhanced's smart dark mode).
    Smart,
    /// The theme's text and background, pictures included.
    Everything,
}

impl DocumentColors {
    /// Every choice, in the order the settings window shows them.
    pub(crate) const ALL: [Self; 3] = [Self::Own, Self::Smart, Self::Everything];

    /// Its value in `settings.toml`.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Own => "off",
            Self::Smart => "smart",
            Self::Everything => "all",
        }
    }

    fn of(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|colors| colors.id() == id)
    }

    /// The localization key of its name.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Own => "settings.document_colors.own",
            Self::Smart => "settings.document_colors.smart",
            Self::Everything => "settings.document_colors.everything",
        }
    }
}

/// `#rrggbb` (either case) as a colour.
pub(crate) fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.trim().strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some([16, 8, 0].map(|shift| ((value >> shift) & 0xff) as u8))
}

/// A colour as `#rrggbb`.
pub(crate) fn hex([r, g, b]: Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The `readingRuler` prop's name for a ruler setting (`reading_ruler_height`: `height`).
fn prop_name(key: &str) -> &str {
    key.strip_prefix("reading_ruler_").unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_colours_round_trip_and_unknown_values_are_the_default() {
        for colors in DocumentColors::ALL {
            assert_eq!(DocumentColors::of(colors.id()), Some(colors));
        }
        assert_eq!(DocumentColors::of("sepia"), None);
        assert_eq!(DocumentColors::default().id(), "off");
    }

    #[test]
    fn hex_colours_round_trip() {
        assert_eq!(parse_hex("#F4ecD8"), Some([0xf4, 0xec, 0xd8]));
        assert_eq!(hex([0xf4, 0xec, 0xd8]), "#f4ecd8");
        assert_eq!(parse_hex("f4ecd8"), None);
        assert_eq!(parse_hex("#fff"), None);
        assert_eq!(parse_hex("#f4ecdz"), None);
    }
}
