# Ph4: 23/44 | N.10: Quick note from a global shortcut

**Tag:** N.10; **Priority:** P1; **Phase:** 4; **Needs:** N.4, K.19; **Code:** `excellent-features::notebook::quicknote`
**Plan:** §27 entry `#### Ph4: 23/44 | N.10`.

## Problem

Users want to capture a fleeting thought or meeting snippet immediately from any app without hunting for a window or creating a new file first.

## User story

As a busy knowledge worker, I want a system-wide shortcut to open a writable page in my Inbox notebook within 300 ms, with an optional tray icon when the main window is closed.

## UI sketch

- Settings > Quick Note: global shortcut picker (warns if `register_global_shortcut` reports a conflict so user can pick another) and opt-in tray icon toggle (`TrayBehavior::Disabled` by default).
- Trigger opens a compact window (`520 x 680`) directly on a new page in `Inbox.expdf`.

## Mode

Global desktop trigger; opens in Write mode.

## Speed and size impact

- Caches the open `Inbox.expdf` in memory and appends pages incrementally for sub-300 ms open latency.
- Tray icon stays disabled unless explicitly enabled by the user.

## Design

- `GlobalShortcutHost` trait bridges NSH's `register_global_shortcut` / `set_tray` (`nsh_core::element::desktop`).
- `QuickNoteInbox` reports `ShortcutStatus::Conflict` if a shortcut is taken, creates or appends to `Inbox.expdf` atomically on `trigger()`, and measures elapsed latency against `QUICK_NOTE_LATENCY_BUDGET_MS` (300 ms).

## Acceptance tests

- [x] `quick_note_reports_shortcut_conflicts_and_opens_writable_page_under_300ms`: verifies conflict reporting, rebinding, tray opt-in, atomic Inbox creation/append, and sub-300 ms trigger latency.
