# K.17 Design tokens and themes

**Tag:** K.17 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.1 · **Code:** `app/themes/`, `excellent-app::theme`
**Plan:** §27 entry `#### K.17` and §10.3 (design system), §15 (accessibility).

## Problem

The app had NSH's default neutral tokens only: no colour in the chrome, no mode accents, no high-contrast theme, and
no way to change any of it.

## User story

As a reader, I want to try themes and see each one on the whole window before I pick it, so that I choose one I like
without guessing from a name.

## UI sketch

- Command palette: **Choose theme**, **Choose appearance (light or dark)**. No default shortcut (Enhanced has none);
  K.16 lets the user bind one. Menu and toolbar entries come with the settings UI (K.16) and the View menu.
- The picker is a searchable list near the top of the window, with no scrim, so the theme shows around it. Two
  groups: **Themes** and **High contrast**. Each row shows a small window in its theme's colours and a dot on the one in
  effect.
- Moving the highlight (arrow keys, Page Up and Down, the pointer, typing to filter) applies that theme at once.
  Enter or a click keeps it. Escape, or a press outside the list, goes back to the theme in effect.
- Accessible names: the list and its search field are named after the picker; the highlighted row is announced.

## Mode

Every mode. Not visible by default: opened from the palette.

## Speed and size impact

- Startup: one family's tokens are built from an embedded TOML file (a few hundred bytes) before the window shows.
  Other families are built on first use and cached. Not measured separately.
- Installer: seven small TOML files are compiled in. `toml` is a new dependency; `serde_json` was already NSH's.

## Design

- Model (`theme::palette`, pure): `tokens(hues, contrast, side)` gives every token as a CSS `oklch()` string.
- Families (`theme::FAMILIES`): `hue`, `chroma`, `tint`, plus optional per-variant overrides; parsed with strict keys.
- View (`theme::picker`): `theme_picker` and `appearance_picker` over one `live_picker`; `list_dialog` gives the
  palette and the pickers one look.
- Apply: `set_tokens` and `set_color_scheme`, so NSH re-resolves every colour. The page canvas gets `page-area` as a
  `#rrggbb` prop.
- Settings: none yet. K.16 keeps the choice in `settings.toml`.

## Acceptance tests

- [x] `every_theme_passes_contrast`: every family, variant and appearance passes WCAG AA (high contrast: 7:1 text,
  4.5:1 controls).
- [x] `the_theme_picker_previews_live_escape_goes_back_and_enter_keeps`: switching needs no restart; the highlighted
  theme previews; Escape restores; Enter keeps.
- [ ] Density (compact, comfortable) and UI scaling (step 3): open.
