# Ph4: 11/44 | H.18: Saved palettes

**Tag:** H.18; **Priority:** P1; **Phase:** 4; **Needs:** A.3; **Code:** `ecclesia-features::ink::palettes`
**Plan:** §8.4 and §27 entry `#### Ph4: 11/44 | H.18`.

## Problem

Users only had the five standard or color-blind-safe annotation swatches from A.3; there was no way to switch between curated color families (pastel, neon, earth, color-blind-safe), save custom project palettes, or share them across machines and teams.

## User story

As a note-taker and educator, I want curated built-in palettes (pastel, neon, earth, color-blind-safe) plus my own saved palettes that I can import and export as small JSON files so I can reuse and share color schemes easily.

## UI sketch

Shown in the pen/annotation color popover, Settings → Ink & Colors, and command palette (`ink.palette_import`, `ink.palette_export`, `ink.palette_choose`). Displays a palette selector dropdown (built-in palettes first, then user palettes), swatch grid with accessible swatch names and hex tooltips, Duplicate/Edit/Delete controls for user palettes, and Import/Export JSON actions.

## Mode

Write and Annotate.

## Speed and size impact

- Startup: built-in palettes are small static tables loaded on demand; user palettes are read from `settings.toml` only when Ink/Annotate color UI opens.
- Budgets touched: zero startup or frame-time impact.
- Installer size: negligible (~5 KB compiled Rust, uses existing `serde_json`).

## Design

- **Model (`ecclesia-features::ink::palettes`):**
  - `Swatch` (accessible name + 24-bit RGB color) and `ColorPalette` (`id`, `name`, `PaletteOrigin::BuiltIn | User`, up to 32 swatches).
  - Four curated built-in palettes (`built_in_palettes`): **Pastel**, **Neon**, **Earth**, and **Color-blind-safe** (exact 8-color Okabe & Ito set matching A.3 / U.4).
  - `PaletteLibrary` manages built-in (read-only) and up to 32 user palettes, active selection, duplication, settings persistence (`to_settings_json` / `from_settings_json`), and portable versioned JSON import/export for both single palettes (`ColorPalette::export` / `import`) and multi-palette bundles (`export_user_bundle` / `import_bundle`), including shorthand hex arrays.
- **Command ids (for app wiring):** `ink.palette_choose`, `ink.palette_import`, `ink.palette_export`.
- **Settings:** `[ink.palettes]` in `settings.toml` (default active `"color-blind-safe"`, empty user list).

## Acceptance tests

- [x] `palettes_import_and_export`: verifies single-palette JSON round-trip, shorthand hex array import, multi-palette bundle round-trip, duplicate-id disambiguation on import, and clean error reporting for invalid JSON, missing fields, newer format versions, empty names, and empty swatch lists.
- [x] `built_in_palettes_include_all_four_required_sets`: confirms Pastel, Neon, Earth, and Okabe–Ito Color-blind-safe built-ins.
- [x] `built_ins_are_read_only_and_settings_round_trip`: confirms built-in immutability, duplication into editable user palettes, and TOML/JSON settings round-trip.
