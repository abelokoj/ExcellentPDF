# Ph1: 6/60 | K.16: Settings, command table and shortcuts

**Tag:** K.16; **Priority:** P0; **Phase:** 1; **Needs:** K.5; **Code:** `ecclesia-app::{settings, commands, keymap, settings_window}`
**Plan:** §27 entry `#### Ph1: 6/60 | K.16` and §10.2 (commands and shortcuts). The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Every toolbar button, menu item, shortcut, and preference would otherwise invent its own storage format, key parser, and dispatch path, causing drift between the UI and keyboard bindings and risking data loss when users hand-edit their configuration.

## User story

As a reader, I want one searchable command palette, predictable keyboard shortcuts that I can override or unbind in `settings.toml`, and a Settings window that persists changes safely, so that every action is reachable and my preferences survive upgrades.

## UI sketch

- **Command palette** (`Ctrl+K` / `Ctrl+Shift+P`): searchable list of all currently available commands with their active shortcut badges.
- **Settings window** (`Ctrl+,`): grouped sections (Appearance, Reading, Printing, Tabs, Home, Packs, Sandbox, Shortcuts) writing only non-default keys to `settings.toml`.
- **Portable mode:** if `settings.toml` exists beside `ecclesiapdf.exe`, it is used instead of `%APPDATA%\com.ecclesiapdf.app\settings.toml`.
- **Error state:** a malformed `settings.toml` is logged and never overwritten; invalid shortcut bindings are reported and skipped.
- Every control has an accessible name and full keyboard access.

## Mode

Every mode. Opened on demand via `Ctrl+K` (command palette) or `Ctrl+,` (Settings).

## Speed and size impact

- Startup: parses `settings.toml` once before window creation (sub-millisecond) and initializes the command keymap.
- Budgets touched: cold start (plan §13.1). Keep Settings UI views lazy until opened.
- Installer size: pure Rust over existing `toml` and `serde` dependencies; no pack needed.

## Design

- Model (`ecclesia-app::settings`, `ecclesia-app::keymap`): sparse TOML table preserving unknown keys, atomic temp-file-and-rename writes, file watcher ignoring self-writes, and conflict-resolving shortcut table (`keys` for window scope, `page_keys` for canvas focus).
- Command table (`ecclesia-app::commands`): central `CommandId` enum with stable string IDs, localized label keys, group metadata, default keys, and `is_available` predicates.
- View (`ecclesia-app::settings_window`, `ecclesia-app::palette`): NSH dialogs built directly from the command table and `Live` settings signal.

## Acceptance tests

- [x] `keeps_unknown_keys_and_writes_only_what_is_set`: sparse atomic persistence preserves unknown keys.
- [x] `a_broken_file_is_never_overwritten`: syntax errors in `settings.toml` are never clobbered.
- [x] `the_palette_lists_every_command_and_runs_one`: command palette lists and dispatches commands.
- [x] `the_defaults_bind_with_no_problem_on_any_platform`: default shortcut bindings validate cleanly.
- [x] `a_users_key_wins_over_a_default_and_an_empty_list_unbinds`: user overrides win conflicts and `[]` unbinds.
- [x] `what_cannot_be_bound_is_reported_not_bound`: invalid/reserved bindings are reported safely.
- [x] `only_window_commands_are_offered_without_a_document`: `is_available` filters document-only commands.
- [x] `an_edit_of_the_file_is_read_in_and_its_own_saves_are_not`: external edits hot-reload without save loops.
- [x] `a_file_broken_by_an_edit_is_kept_out_until_fixed`: broken live edits are ignored until repaired.
- [x] `the_settings_window_opens_from_its_key_and_hands_over_to_the_theme_picker`: Settings window opens via `Ctrl+,` and hands off to theme picker.
- [x] `every_class_of_the_controls_is_known`: every setting control kind is validated.

## Open

- Inline shortcut editor inside the Settings window; SumatraPDF Enhanced settings import (`SumatraPDFEnhanced-settings.txt`); maintainer hand-acceptance.
