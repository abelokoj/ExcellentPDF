//! Annotation colours (A.3): each tool's palette, its colour and opacity, and the custom colours used lately.
//!
//! A highlighter is drawn under the text, so its colours are light, for black text to read on them; a line (an
//! underline, a squiggle, a strikeout, a proposal's caret) is drawn over or beside it, so its colours are strong, to
//! stand out on white paper. Each comes in the standard set (the theme's annotation colours, plan §10.3) and a
//! colour-blind-safe one (Okabe and Ito's, U.4).

use serde_json::Value;

/// The tools that have a colour of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tool {
    /// Highlight.
    Highlight,
    /// Underline.
    Underline,
    /// Squiggly underline.
    Squiggly,
    /// Strikeout.
    StrikeOut,
    /// Replace and insert text (A.2): the strikeout and caret of a proposal.
    Proposal,
}

impl Tool {
    /// Every tool, in the bar's order.
    pub const ALL: [Self; 5] = [
        Self::Highlight,
        Self::Underline,
        Self::Squiggly,
        Self::StrikeOut,
        Self::Proposal,
    ];

    /// Its name in settings and events.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::Squiggly => "squiggly",
            Self::StrikeOut => "strikeOut",
            Self::Proposal => "proposal",
        }
    }

    /// The tool named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// Whether it is drawn under the text, so it takes light colours.
    fn is_highlighter(self) -> bool {
        self == Self::Highlight
    }

    /// Which of its palette's swatches it starts with: yellow for a highlight, blue for an underline and a proposal
    /// (Acrobat's), red for a squiggle and a strikeout.
    fn first_swatch(self) -> usize {
        match self {
            Self::Highlight | Self::Squiggly | Self::StrikeOut => 0,
            Self::Underline | Self::Proposal => 1,
        }
    }
}

/// A set of colours to choose from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Palette {
    /// The theme's annotation colours.
    #[default]
    Standard,
    /// Okabe and Ito's colours, told apart with any colour vision (U.4).
    ColorBlindSafe,
}

/// Highlighters, `0xRRGGBB`: the theme's annotation colours (`annot-yellow`, `-green`, `-blue`, `-pink`, `-orange`).
const STANDARD_LIGHT: [u32; 5] = [0xfde047, 0x86efac, 0x93c5fd, 0xf9a8d4, 0xfdba74];
/// Lines: red, blue, green, purple, near black (the theme's `ink-red`, `ink-blue` and `ink-black` among them).
const STANDARD_STRONG: [u32; 5] = [0xc62828, 0x1a3d8f, 0x2e7d32, 0x7b1fa2, 0x1b1b1f];
/// Okabe and Ito's yellow, sky blue, orange, bluish green and reddish purple.
const SAFE_LIGHT: [u32; 5] = [0xf0e442, 0x56b4e9, 0xe69f00, 0x009e73, 0xcc79a7];
/// Okabe and Ito's vermillion, blue, bluish green, reddish purple and black.
const SAFE_STRONG: [u32; 5] = [0xd55e00, 0x0072b2, 0x009e73, 0xcc79a7, 0x000000];

/// How many custom colours are kept.
const RECENT: usize = 6;

/// The lowest opacity a tool takes: below it the mark is gone.
pub const MIN_OPACITY: f32 = 0.1;

impl Palette {
    /// Its name in settings.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::ColorBlindSafe => "color-blind-safe",
        }
    }

    /// The palette named `name`; the standard one for anything else.
    #[must_use]
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("color-blind-safe") => Self::ColorBlindSafe,
            _ => Self::Standard,
        }
    }

    /// The colours `tool` offers, `0xRRGGBB`.
    #[must_use]
    pub fn swatches(self, tool: Tool) -> [u32; 5] {
        match (self, tool.is_highlighter()) {
            (Self::Standard, true) => STANDARD_LIGHT,
            (Self::Standard, false) => STANDARD_STRONG,
            (Self::ColorBlindSafe, true) => SAFE_LIGHT,
            (Self::ColorBlindSafe, false) => SAFE_STRONG,
        }
    }
}

/// A tool's colour, `0xRRGGBB`, and opacity, from [`MIN_OPACITY`] to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolStyle {
    /// Its colour.
    pub color: u32,
    /// Its opacity.
    pub opacity: f32,
}

impl ToolStyle {
    /// `color` at `opacity`, kept in range.
    #[must_use]
    pub fn new(color: u32, opacity: f32) -> Self {
        Self {
            color: color & 0xff_ffff,
            opacity: if opacity.is_finite() {
                opacity.clamp(MIN_OPACITY, 1.0)
            } else {
                1.0
            },
        }
    }

    /// Its colour as RGB from 0 to 1.
    #[must_use]
    pub fn rgb(self) -> [f32; 3] {
        [16, 8, 0].map(|shift| f32::from(((self.color >> shift) & 0xff) as u8) / 255.0)
    }
}

/// Every tool's colours: the palette, what each tool was given, and the custom colours used lately.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Colors {
    /// The palette the tools offer.
    pub palette: Palette,
    /// The tools given a colour or opacity of their own; the others start from their palette.
    pub chosen: Vec<(Tool, ToolStyle)>,
    /// Custom colours, most recent first, none of them in the palette.
    pub recent: Vec<u32>,
}

impl Colors {
    /// `tool`'s colour and opacity: what it was given, or its palette's first colour, opaque.
    #[must_use]
    pub fn style(&self, tool: Tool) -> ToolStyle {
        self.chosen
            .iter()
            .find(|(chosen, _)| *chosen == tool)
            .map_or_else(
                || ToolStyle::new(self.palette.swatches(tool)[tool.first_swatch()], 1.0),
                |(_, style)| *style,
            )
    }

    /// Gives `tool` `style`. A colour that isn't one of the palette's goes first among the recent ones.
    pub fn choose(&mut self, tool: Tool, style: ToolStyle) {
        let style = ToolStyle::new(style.color, style.opacity);
        match self.chosen.iter_mut().find(|(chosen, _)| *chosen == tool) {
            Some((_, kept)) => *kept = style,
            None => self.chosen.push((tool, style)),
        }
        if !self.palette.swatches(tool).contains(&style.color) {
            self.recent.retain(|&color| color != style.color);
            self.recent.insert(0, style.color);
            self.recent.truncate(RECENT);
        }
    }

    /// Gives `tool` `color`, keeping its opacity.
    pub fn choose_color(&mut self, tool: Tool, color: u32) {
        let opacity = self.style(tool).opacity;
        self.choose(tool, ToolStyle::new(color, opacity));
    }
}

impl Colors {
    /// As settings keep it and the page canvas reads it: `{palette, tools: {<tool>: {color: "#rrggbb", opacity}},
    /// recent: ["#rrggbb"]}`; the standard palette is left out.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut tools = serde_json::Map::new();
        for (tool, style) in &self.chosen {
            tools.insert(
                tool.name().to_owned(),
                serde_json::json!({ "color": hex(style.color), "opacity": style.opacity }),
            );
        }
        let mut json = serde_json::json!({
            "tools": tools,
            "recent": self.recent.iter().map(|&color| hex(color)).collect::<Vec<_>>(),
        });
        if self.palette != Palette::Standard {
            json["palette"] = self.palette.name().into();
        }
        json
    }

    /// The colours in `value`, written as [`Self::to_json`] writes them. What doesn't parse is left out: a tool
    /// without a colour, an unknown palette (the standard one), a recent colour that is none.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        let palette = Palette::parse(value.get("palette").and_then(Value::as_str));
        let mut chosen = Vec::new();
        if let Some(tools) = value.get("tools").and_then(Value::as_object) {
            for (name, style) in tools {
                let Some(tool) = Tool::parse(name) else {
                    continue;
                };
                let Some(color) = style
                    .get("color")
                    .and_then(Value::as_str)
                    .and_then(parse_hex)
                else {
                    continue;
                };
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "an opacity from 0 to 1 fits an f32"
                )]
                let opacity = style
                    .get("opacity")
                    .and_then(Value::as_f64)
                    .map_or(1.0, |o| o as f32);
                chosen.push((tool, ToolStyle::new(color, opacity)));
            }
        }
        chosen.sort_by_key(|(tool, _)| *tool);
        let recent = value
            .get("recent")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|color| color.as_str().and_then(parse_hex))
            .take(RECENT)
            .collect();
        Self {
            palette,
            chosen,
            recent,
        }
    }
}

/// `color` written `#rrggbb`.
#[must_use]
pub fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0xff_ffff)
}

/// The colour `#rrggbb` (or `rrggbb`) in `text`.
#[must_use]
pub fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG's contrast ratio of two colours.
    fn contrast(a: u32, b: u32) -> f32 {
        let luminance = |color: u32| {
            let [r, g, b] = ToolStyle::new(color, 1.0).rgb().map(|c| {
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_palette_reads_on_paper() {
        for palette in [Palette::Standard, Palette::ColorBlindSafe] {
            for tool in Tool::ALL {
                let swatches = palette.swatches(tool);
                for (index, &color) in swatches.iter().enumerate() {
                    assert!(!swatches[..index].contains(&color), "{color:06x} twice");
                    if tool.is_highlighter() {
                        // Black text on the highlight.
                        assert!(contrast(color, 0) >= 4.5, "{color:06x} under black text");
                    } else {
                        // A line on white paper.
                        assert!(contrast(color, 0xff_ffff) >= 3.0, "{color:06x} on white");
                    }
                }
            }
        }
    }

    #[test]
    fn the_color_blind_safe_set_is_okabe_and_ito_s() {
        const OKABE_ITO: [u32; 8] = [
            0xe69f00, 0x56b4e9, 0x009e73, 0xf0e442, 0x0072b2, 0xd55e00, 0xcc79a7, 0x000000,
        ];
        for tool in Tool::ALL {
            for color in Palette::ColorBlindSafe.swatches(tool) {
                assert!(OKABE_ITO.contains(&color), "{color:06x}");
            }
        }
    }

    #[test]
    fn a_tool_starts_from_its_palette_and_keeps_what_it_is_given() {
        let mut colors = Colors::default();
        assert_eq!(colors.style(Tool::Highlight).color, 0xfde047);
        assert_eq!(colors.style(Tool::Proposal).color, 0x1a3d8f);
        colors.palette = Palette::ColorBlindSafe;
        assert_eq!(colors.style(Tool::Highlight).color, 0xf0e442);
        colors.choose(Tool::Underline, ToolStyle::new(0x0072b2, 0.5));
        assert_eq!(colors.style(Tool::Underline), ToolStyle::new(0x0072b2, 0.5));
        colors.choose_color(Tool::Underline, 0xd55e00);
        assert!((colors.style(Tool::Underline).opacity - 0.5).abs() < f32::EPSILON);
        assert!(
            colors.recent.is_empty(),
            "the palette's own colours aren't custom"
        );
    }

    #[test]
    fn custom_colors_are_remembered_newest_first() {
        let mut colors = Colors::default();
        for color in 1..=8 {
            colors.choose_color(Tool::Highlight, color);
        }
        colors.choose_color(Tool::StrikeOut, 5);
        assert_eq!(colors.recent, vec![5, 8, 7, 6, 4, 3]);
    }

    #[test]
    fn opacity_and_hex_stay_in_range() {
        assert!((ToolStyle::new(0, 0.0).opacity - MIN_OPACITY).abs() < f32::EPSILON);
        assert!((ToolStyle::new(0, f32::NAN).opacity - 1.0).abs() < f32::EPSILON);
        assert_eq!(hex(0x0a0b0c), "#0a0b0c");
        assert_eq!(parse_hex(" #FDE047 "), Some(0xfde047));
        assert_eq!(parse_hex("#fff"), None);
        assert_eq!(Tool::parse("strikeOut"), Some(Tool::StrikeOut));
        assert_eq!(Palette::parse(Some("nope")), Palette::Standard);
    }

    #[test]
    fn colors_go_through_json_and_back() {
        let mut colors = Colors {
            palette: Palette::ColorBlindSafe,
            ..Colors::default()
        };
        colors.choose(Tool::Highlight, ToolStyle::new(0x123456, 0.5));
        colors.choose(Tool::Proposal, ToolStyle::new(0x0072b2, 1.0));
        assert_eq!(Colors::from_json(&colors.to_json()), colors);
        let odd = serde_json::json!({ "tools": { "pen": { "color": "#000000" }, "underline": {} }, "recent": [3] });
        assert_eq!(Colors::from_json(&odd), Colors::default());
    }
}
