# Ph9: 10/13 | Y.2: Same settings on every OS

**Tag:** Y.2; **Priority:** P1; **Phase:** 9; **Needs:** K.16, Y.1; **Code:** `ecclesia-features::sync::settings`
**Plan:** §27 entry `#### Ph9: 10/13 | Y.2`.

## Problem

Raw settings files contain OS-specific window handles, local cache paths, and `Ctrl+` vs `Cmd+` shortcuts that break or overwrite platform preferences when synced across Windows, macOS, and Linux.

## User story

As a cross-platform user, I want my pens, palettes, stamps, templates, and shortcuts to sync across Windows, macOS, and Linux while keeping OS-specific overrides isolated.

## UI sketch

- Settings -> Sync -> "Sync settings, pens, and keymap across devices" toggle with per-OS override indicator.

## Mode

All modes.

## Speed and size impact

- Startup: sanitizes and materializes JSON settings only on settings load/save.
- Budgets touched: negligible (< 1 ms).
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::sync::settings::{PortableSettings, PortablePen, TargetOs, sanitize_local_settings, materialize_for_os, merge_portable_settings}`.

## Acceptance tests

- [ ] `portable_settings_share_pen_from_windows_to_macos`: A pen added on Windows syncs to macOS while stripping `window_hwnd`, mapping `Ctrl+F` -> `Cmd+F`, and preserving macOS overrides.
