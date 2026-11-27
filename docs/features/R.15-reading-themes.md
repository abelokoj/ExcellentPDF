# Ph1: 26/60 | R.15: Reading themes

**Tag:** R.15; **Priority:** P0; **Phase:** 1; **Needs:** R.14, K.17; **Code:** `ecclesia-core::darkmode::themes`,
`ecclesia-app::reading` (`reading_theme`), `ecclesia-app::reading_theme_bar`, `ecclesia-app::settings_window`
**Plan:** §27 entry `Ph1: 26/60 | R.15`.

## Problem

Smart dark mode (R.14) draws the pages in the app theme's colours, and only those. Readers want paper and ink of
their own: sepia for long evenings, pure black for an OLED screen, white on black for low vision, without changing
the colours of the whole app.

## User story

As a reader, I want to pick the paper and ink the pages are drawn in, seeing each choice on the page I am reading
before I keep it, so that I can read comfortably in any light and still get text with enough contrast.

## UI sketch

- Command palette: "Choose reading theme" (`view.reading_theme`) opens the reading-theme bar. Settings window
  (Ctrl+,), Reading, "Reading theme": a button with the current theme's name that opens the same bar.
- The bar takes the place of the palette button in the title bar, outside the page canvas (a window over the app,
  which a dialog would have to hide): a radio group of swatches, each a small page in the theme's paper with an "A"
  in its ink and the theme's name, then Cancel and Keep. Left and Right (or a click) move between swatches and show
  each theme on the open page at once; Enter or Keep keeps it, Escape or Cancel goes back. Each swatch is a radio
  with its theme's name as its accessible name; the group is one Tab stop.
- Themes: App theme (the default: the app theme's text and background, R.14's colours), Light, Sepia, Dark, OLED
  black, High contrast, Custom.
- Settings window, Reading, "Custom theme": paper and ink as `#rrggbb` fields with a swatch each, and the contrast of
  the pair in words ("Contrast 7.5 : 1, readable", or "too low for text (needs 4.5)" in the destructive colour).
- States: choosing a theme while the pages are in the document's own colours switches page colours to Smart. While
  the bar is open the pages show the theme under the highlight, even with page colours off.

## Mode

All modes.

## Speed and size impact

- Startup: nothing; the bar is built when it opens.
- Previewing a theme changes the page colours, which renders the visible tiles again (as R.14 does on a change).
  Not measured (K.24).
- Installer: code and strings only.

## Design

- Model (`ecclesia-core::darkmode::themes`, pure Rust): `ReadingTheme` (ids `app`, `light`, `sepia`, `dark`,
  `oled`, `contrast`, `custom`), each built-in one's `Palette` (ink, paper, link), `Palette::custom` (light links on
  dark paper), and WCAG 2's `contrast` and `AA_TEXT`. Black text maps exactly to the ink (`recolor`), so a palette
  that passes AA gives pages that do.
- App (`reading.rs`): `reading_theme`, `reading_ink` and `reading_paper` in `settings.toml`; the `pageColors` prop is
  built from the theme shown (the previewed one while the bar is open, else the kept one). The bar
  (`reading_theme_bar.rs`) keeps a preview signal and clears it however it closes.

## Acceptance tests

From the entry's **Done when** ("each theme renders the corpus with WCAG AA text contrast"):

- [x] `each_reading_theme_renders_the_corpus_with_aa_text_contrast`: a tile of the corpus, rendered in every built-in
  theme, has the theme's paper and text at 4.5 : 1 or more.
- [x] `every_built_in_theme_passes_aa`, `contrast_matches_wcag`,
  `a_custom_palette_is_judged_and_gets_light_links_on_dark_paper`, `reading_theme_ids_round_trip`: the model.
- [x] `hex_colours_round_trip`: the custom colours' settings.
- [ ] Per-document override (needs K.20, remembering per document).
- [ ] Reflowable formats themed through MuPDF's user stylesheet (step 2); they are recoloured like PDFs meanwhile.
- [ ] Live preview tried by hand on the Surface Pro.
