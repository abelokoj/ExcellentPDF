//! The palette every theme family is made from (plan §10.3): one set of OKLCH lightness levels per variant (light,
//! dark, and the two high-contrast ones), coloured by a family's hue. Surfaces take a faint tint of the hue, accents
//! (primary, focus ring) its full chroma; the document area, danger, success, warning and the mode accents keep
//! their own hues in every family, so the chrome stays calm and a mode reads the same in any theme.
//!
//! The levels were chosen so that every pair the chrome draws text or a control with passes WCAG AA (4.5:1 for text,
//! 3:1 for controls) in the standard variants and 7:1 and 4.5:1 in the high-contrast ones, for any hue;
//! `super::tests::every_theme_passes_contrast` holds every built-in family to that.

/// Standard colours, or the high-contrast set (plan §15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum Contrast {
    /// AA contrast; tinted surfaces.
    #[default]
    Standard,
    /// 7:1 text on pure black or white.
    High,
}

/// How a family colours the palette (its file's `hue`, `chroma` and `tint`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hues {
    /// OKLCH hue, in degrees.
    pub(crate) hue: f32,
    /// The accents' OKLCH chroma.
    pub(crate) chroma: f32,
    /// How much of the hue the surfaces take, from 0 (none) to 1.
    pub(crate) tint: f32,
}

/// The mode accents (plan §10.2), what is drawn on them, and their hues: read blue, annotate amber, write violet,
/// study green, present coral.
pub(crate) const MODES: [(&str, &str, f32); 5] = [
    ("mode-read", "mode-read-foreground", 255.0),
    ("mode-annotate", "mode-annotate-foreground", 70.0),
    ("mode-write", "mode-write-foreground", 295.0),
    ("mode-study", "mode-study-foreground", 150.0),
    ("mode-present", "mode-present-foreground", 30.0),
];

/// Colours that are the same in every theme and appearance: the default highlighter and pen colours (plan §10.3's
/// annotation colours). They are content, drawn on white paper, so they don't follow the theme.
pub(crate) const ANNOTATION: [(&str, &str); 9] = [
    ("annot-yellow", "#fde047"),
    ("annot-green", "#86efac"),
    ("annot-blue", "#93c5fd"),
    ("annot-pink", "#f9a8d4"),
    ("annot-orange", "#fdba74"),
    ("annot-purple", "#c4b5fd"),
    ("ink-black", "#1b1b1f"),
    ("ink-blue", "#1a3d8f"),
    ("ink-red", "#c62828"),
];

/// The OKLCH lightness of each role in one variant.
struct Levels {
    background: f32,
    foreground: f32,
    card: f32,
    popover: f32,
    primary: f32,
    /// The accents' share of the family's chroma (lighter accents take less, or they glow).
    primary_chroma: f32,
    on_primary: f32,
    secondary: f32,
    on_secondary: f32,
    muted: f32,
    muted_foreground: f32,
    accent: f32,
    on_accent: f32,
    border: f32,
    input: f32,
    sidebar: f32,
    sidebar_accent: f32,
    sidebar_foreground: f32,
    page_area: f32,
    destructive: f32,
    success: f32,
    warning: f32,
    mode: f32,
    on_mode: f32,
}

const LIGHT: Levels = Levels {
    background: 0.985,
    foreground: 0.21,
    card: 0.998,
    popover: 0.998,
    primary: 0.50,
    primary_chroma: 1.0,
    on_primary: 0.985,
    secondary: 0.955,
    on_secondary: 0.25,
    muted: 0.955,
    muted_foreground: 0.48,
    accent: 0.93,
    on_accent: 0.24,
    border: 0.90,
    input: 0.62,
    sidebar: 0.962,
    sidebar_accent: 0.915,
    sidebar_foreground: 0.21,
    page_area: 0.885,
    destructive: 0.53,
    success: 0.50,
    warning: 0.50,
    mode: 0.53,
    on_mode: 0.99,
};

const DARK: Levels = Levels {
    background: 0.17,
    foreground: 0.955,
    card: 0.205,
    popover: 0.225,
    primary: 0.74,
    primary_chroma: 0.85,
    on_primary: 0.17,
    secondary: 0.265,
    on_secondary: 0.955,
    muted: 0.25,
    muted_foreground: 0.73,
    accent: 0.295,
    on_accent: 0.965,
    border: 0.30,
    input: 0.52,
    sidebar: 0.20,
    sidebar_accent: 0.29,
    sidebar_foreground: 0.95,
    page_area: 0.13,
    destructive: 0.70,
    success: 0.74,
    warning: 0.80,
    mode: 0.74,
    on_mode: 0.17,
};

const CONTRAST_LIGHT: Levels = Levels {
    background: 1.0,
    foreground: 0.0,
    card: 1.0,
    popover: 1.0,
    primary: 0.38,
    primary_chroma: 0.85,
    on_primary: 1.0,
    secondary: 0.94,
    on_secondary: 0.0,
    muted: 0.94,
    muted_foreground: 0.32,
    accent: 0.90,
    on_accent: 0.0,
    border: 0.55,
    input: 0.40,
    sidebar: 0.965,
    sidebar_accent: 0.88,
    sidebar_foreground: 0.0,
    page_area: 0.80,
    destructive: 0.42,
    success: 0.38,
    warning: 0.40,
    mode: 0.40,
    on_mode: 1.0,
};

const CONTRAST_DARK: Levels = Levels {
    background: 0.0,
    foreground: 1.0,
    card: 0.10,
    popover: 0.13,
    primary: 0.86,
    primary_chroma: 0.85,
    on_primary: 0.0,
    secondary: 0.20,
    on_secondary: 1.0,
    muted: 0.17,
    muted_foreground: 0.86,
    accent: 0.27,
    on_accent: 1.0,
    border: 0.60,
    input: 0.70,
    sidebar: 0.09,
    sidebar_accent: 0.25,
    sidebar_foreground: 1.0,
    page_area: 0.0,
    destructive: 0.80,
    success: 0.85,
    warning: 0.88,
    mode: 0.84,
    on_mode: 0.0,
};

/// Light or dark appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Light,
    Dark,
}

/// The tokens of one variant, as CSS colours, in a fixed order.
pub(crate) fn tokens(hues: Hues, contrast: Contrast, side: Side) -> Vec<(&'static str, String)> {
    let levels = match (contrast, side) {
        (Contrast::Standard, Side::Light) => &LIGHT,
        (Contrast::Standard, Side::Dark) => &DARK,
        (Contrast::High, Side::Light) => &CONTRAST_LIGHT,
        (Contrast::High, Side::Dark) => &CONTRAST_DARK,
    };
    let Hues { hue, chroma, tint } = hues;
    // A surface's chroma at full tint; accents' are shares of the family's chroma.
    let surface = |l: f32, c: f32| oklch(l, c * tint, hue);
    let accent = |l: f32, share: f32| oklch(l, chroma * share, hue);
    let fixed = |l: f32, c: f32, h: f32| oklch(l, c, h);
    let lv = levels;
    let primary = accent(lv.primary, lv.primary_chroma);
    let on_primary = surface(lv.on_primary, 0.005);
    let ring = accent(lv.primary, 0.9);
    let border = surface(lv.border, 0.01);
    let mut tokens = vec![
        ("background", surface(lv.background, 0.005)),
        ("foreground", surface(lv.foreground, 0.015)),
        ("card", surface(lv.card, 0.003)),
        ("card-foreground", surface(lv.foreground, 0.015)),
        ("popover", surface(lv.popover, 0.003)),
        ("popover-foreground", surface(lv.foreground, 0.015)),
        ("primary", primary),
        ("primary-foreground", on_primary.clone()),
        ("secondary", surface(lv.secondary, 0.012)),
        ("secondary-foreground", surface(lv.on_secondary, 0.02)),
        ("muted", surface(lv.muted, 0.01)),
        ("muted-foreground", surface(lv.muted_foreground, 0.02)),
        ("accent", surface(lv.accent, 0.03)),
        ("accent-foreground", surface(lv.on_accent, 0.05)),
        ("destructive", fixed(lv.destructive, 0.20, 27.0)),
        ("border", border.clone()),
        ("input", surface(lv.input, 0.015)),
        ("ring", ring.clone()),
        ("sidebar", surface(lv.sidebar, 0.01)),
        ("sidebar-foreground", surface(lv.sidebar_foreground, 0.015)),
        ("sidebar-primary", accent(lv.primary, 1.0)),
        ("sidebar-primary-foreground", on_primary),
        ("sidebar-accent", surface(lv.sidebar_accent, 0.025)),
        (
            "sidebar-accent-foreground",
            surface(lv.sidebar_foreground, 0.03),
        ),
        ("sidebar-border", border),
        ("sidebar-ring", ring),
        ("page-area", surface(lv.page_area, 0.004)),
        ("success", fixed(lv.success, 0.13, 150.0)),
        ("success-foreground", fixed(lv.on_primary, 0.01, 150.0)),
        ("warning", fixed(lv.warning, 0.14, 70.0)),
        ("warning-foreground", fixed(lv.on_primary, 0.01, 70.0)),
    ];
    for (mode, on_mode, mode_hue) in MODES {
        tokens.push((mode, fixed(lv.mode, 0.16, mode_hue)));
        tokens.push((on_mode, fixed(lv.on_mode, 0.01, mode_hue)));
    }
    tokens
}

/// An OKLCH colour as CSS writes it.
fn oklch(l: f32, c: f32, h: f32) -> String {
    format!("oklch({l:.3} {c:.4} {h:.1})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_css_oklch() {
        assert_eq!(oklch(0.5, 0.17, 268.0), "oklch(0.500 0.1700 268.0)");
    }

    #[test]
    fn every_variant_has_the_same_tokens() {
        let hues = Hues {
            hue: 200.0,
            chroma: 0.1,
            tint: 1.0,
        };
        let names =
            |c, s| -> Vec<&str> { tokens(hues, c, s).into_iter().map(|(n, _)| n).collect() };
        let light = names(Contrast::Standard, Side::Light);
        for (c, s) in [
            (Contrast::Standard, Side::Dark),
            (Contrast::High, Side::Light),
            (Contrast::High, Side::Dark),
        ] {
            assert_eq!(names(c, s), light);
        }
    }
}
