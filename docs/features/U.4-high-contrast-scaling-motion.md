# Ph1: 58/60 | U.4: High contrast, color-blind-safe colors, scaling and reduced motion

**Tag:** U.4; **Priority:** P0; **Phase:** 1; **Needs:** K.17; **Code:** `ecclesia-app::theme`,
`ecclesia-platform::display`
**Plan:** §27.18 entry `Ph1: 58/60 | U.4`.

## Problem

A reader who sets Windows to high contrast, larger text or less motion expects every app to follow. A theme the app
chose must not override the contrast the reader needs.

## User story

As a reader with low vision, I want the app to switch to high-contrast colours when I turn on Windows' high contrast,
grow with my text size, and stop animating when I ask for less motion.

## UI sketch

- **Settings, Appearance:** **Follow the system's high contrast** (on by default). While Windows' high contrast is
  on, every theme uses its own high-contrast side (K.17's contrast 7:1 for text, 4.5:1 for the rest); turning it off
  in Windows brings the chosen contrast back within two seconds, with no restart.
- Text size (Settings, Accessibility, Text size in Windows) and display scaling scale the whole window: NSH reads both.
- With Windows' "Animation effects" off, the outline's chevrons and the dialogs' fades jump instead of moving.

## Mode

All modes.

## Speed and size impact

- Startup: one `SystemParametersInfoW` call before the window shows.
- Running: the same call every two seconds on the UI thread (microseconds; **not measured**). Nothing to the
  installer.

## Design

- `ecclesia-platform::display::high_contrast` reads `SPI_GETHIGHCONTRAST` (`HCF_HIGHCONTRASTON`); `false` off
  Windows until Phase 2.
- `theme::follow_system` keeps whether the system's high contrast is in force (the setting `follow_system_contrast`,
  and the system's), and `theme::tokens` gives the theme's high-contrast side while it is. Every path that applies a
  theme (the root, the pickers, an edit of the settings file) goes through `tokens`, so none of them can undo it.
  The shell looks again every two seconds and when the settings change.
- Scaling and text size: NSH's Windows host maps the monitor's DPI and `TextScaleFactor` into the environment; the
  app's sizes are points, so everything scales. The window's layout holds at 640 × 400 points (a 1280 × 800 screen
  at 200 %): below 900 points of window width the sidebar and inspector become drawers over the document
  (`shell::COMPACT_BELOW`), since both docks open (260 + 300 points) would leave the page less than 340.
- Reduced motion: NSH's environment carries `SPI_GETCLIENTAREAANIMATION`; its dialogs honour it. The app's own style
  transition (the outline chevron) sets its duration to zero while `reduce_motion()` is on.
- Colour alone: the find bar's current hit differs from the others in lightness as well as hue (L ≈ 0.58 against
  0.88), so it reads in every kind of colour blindness; selection and focus use the theme's ring and borders, not
  hue alone.
- Not built: color-blind-safe **annotation** palettes come with the annotation tools (A.*, Phase 3), which own the
  palettes; macOS and Linux high contrast come in Phase 2; Windows' own high-contrast colours (`COLOR_WINDOW` and
  the like) are not used: the theme's high-contrast side is, as K.17 designed it. The contrast check at 200 % is the
  K.17 test plus the layout test below; no screenshot-based check exists yet.

## Acceptance tests

- [x] `every_theme_passes_contrast` (K.17; every theme's standard and high-contrast sides)
- [x] `the_window_works_at_double_scaling`
- [ ] every screen passes the contrast check at 200 % scaling: the token check and the layout test pass; **not
  looked at by hand** at 200 % yet
