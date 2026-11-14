//! Reading themes (plan R.15): the paper and ink the pages are drawn in, chosen apart from the app's theme.
//!
//! Each built-in theme passes WCAG AA for body text (4.5 : 1, ink and link on paper); a custom one is told whether it
//! does ([`Palette::passes_aa`]). The pixels are mapped by [`super::recolor`], so black text comes out exactly as the
//! ink: the contrast of the palette is the contrast of the page's text.

use super::{PageColors, Pictures, Rgb};

/// WCAG's least contrast for body text at level AA.
pub const AA_TEXT: f32 = 4.5;

/// Links on dark paper: a light blue that keeps AA on every dark built-in paper.
const LINK_ON_DARK: Rgb = [0x8a, 0xb4, 0xf8];

/// A reading theme, as `reading_theme` names it in `settings.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReadingTheme {
    /// The app theme's text on its background.
    #[default]
    App,
    /// Near-black on white.
    Light,
    /// Brown ink on warm paper.
    Sepia,
    /// Light grey on dark grey.
    Dark,
    /// Grey on pure black, which an OLED screen leaves unlit.
    Oled,
    /// White on black, yellow links.
    HighContrast,
    /// The reader's own paper and ink.
    Custom,
}

impl ReadingTheme {
    /// Every theme, in the order they are offered.
    pub const ALL: [Self; 7] = [
        Self::App,
        Self::Light,
        Self::Sepia,
        Self::Dark,
        Self::Oled,
        Self::HighContrast,
        Self::Custom,
    ];

    /// Its value in `settings.toml`.
    pub fn id(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Light => "light",
            Self::Sepia => "sepia",
            Self::Dark => "dark",
            Self::Oled => "oled",
            Self::HighContrast => "contrast",
            Self::Custom => "custom",
        }
    }

    /// The theme `id` names, if any.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.id() == id)
    }

    /// Its colours; `None` for the two whose colours come from elsewhere (the app theme, the reader's own).
    pub fn palette(self) -> Option<Palette> {
        let (ink, paper, link) = match self {
            Self::App | Self::Custom => return None,
            Self::Light => ([0x1a, 0x1a, 0x1a], [0xff, 0xff, 0xff], None),
            Self::Sepia => ([0x5b, 0x46, 0x36], [0xf4, 0xec, 0xd8], None),
            Self::Dark => ([0xdc, 0xdc, 0xdc], [0x1e, 0x1e, 0x1e], Some(LINK_ON_DARK)),
            Self::Oled => ([0xc8, 0xc8, 0xc8], [0x00, 0x00, 0x00], Some(LINK_ON_DARK)),
            Self::HighContrast => (
                [0xff, 0xff, 0xff],
                [0x00, 0x00, 0x00],
                Some([0xff, 0xff, 0x00]),
            ),
        };
        Some(Palette { ink, paper, link })
    }
}

/// A reading theme's colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Palette {
    /// What black text becomes.
    pub ink: Rgb,
    /// What white paper becomes.
    pub paper: Rgb,
    /// What blue links become; `None` maps them with the rest (on light paper the document's blue reads well).
    pub link: Option<Rgb>,
}

impl Palette {
    /// The reader's own colours, with light links when the paper is dark.
    pub fn custom(ink: Rgb, paper: Rgb) -> Self {
        Self {
            ink,
            paper,
            link: is_dark(paper).then_some(LINK_ON_DARK),
        }
    }

    /// The colours the page canvas draws in.
    pub fn page_colors(self, pictures: Pictures) -> PageColors {
        PageColors {
            ink: self.ink,
            paper: self.paper,
            link: self.link,
            pictures,
        }
    }

    /// The contrast of the text on the paper, from 1 to 21.
    pub fn text_contrast(&self) -> f32 {
        contrast(self.ink, self.paper)
    }

    /// Whether text and links pass WCAG AA on the paper.
    pub fn passes_aa(&self) -> bool {
        self.text_contrast() >= AA_TEXT
            && self
                .link
                .is_none_or(|link| contrast(link, self.paper) >= AA_TEXT)
    }
}

/// WCAG 2's contrast ratio of two colours, from 1 (the same) to 21 (black and white).
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Whether text on `paper` should be light: white stands out on it more than black does.
pub fn is_dark(paper: Rgb) -> bool {
    contrast(paper, [0xff; 3]) > contrast(paper, [0; 3])
}

/// WCAG 2's relative luminance.
fn luminance(rgb: Rgb) -> f32 {
    let [r, g, b] = rgb.map(|c| {
        let c = f32::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_theme_passes_aa() {
        for theme in ReadingTheme::ALL {
            if let Some(palette) = theme.palette() {
                assert!(
                    palette.passes_aa(),
                    "{theme:?}: text {:.2} : 1",
                    palette.text_contrast()
                );
            }
        }
    }

    #[test]
    fn reading_theme_ids_round_trip() {
        for theme in ReadingTheme::ALL {
            assert_eq!(ReadingTheme::parse(theme.id()), Some(theme));
        }
        assert_eq!(ReadingTheme::parse("plaid"), None);
        assert_eq!(ReadingTheme::default(), ReadingTheme::App);
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast([0; 3], [0xff; 3]) - 21.0).abs() < 0.01);
        assert!((contrast([0x77; 3], [0xff; 3]) - 4.48).abs() < 0.01);
        assert_eq!(contrast([0x33; 3], [0x33; 3]), 1.0);
    }

    #[test]
    fn a_custom_palette_is_judged_and_gets_light_links_on_dark_paper() {
        let dim = Palette::custom([0x55; 3], [0x33; 3]);
        assert!(!dim.passes_aa());
        assert_eq!(dim.link, Some(LINK_ON_DARK));
        let paper = Palette::custom([0x22; 3], [0xfa, 0xf8, 0xf0]);
        assert!(paper.passes_aa());
        assert_eq!(paper.link, None);
    }
}
