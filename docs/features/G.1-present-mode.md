# Ph4: 38/44 | G.1: Present mode

**Tag:** G.1; **Priority:** P0; **Phase:** 4; **Needs:** R.19; **Code:** `excellent-features::present::session`
**Plan:** §27 entry `#### Ph4: 38/44 | G.1`.

## Problem

Presenting slides in standard reading chrome distracts the audience and requires manual key configuration for USB clickers.

## User story

As a lecturer, I want a full-screen single-page present mode that works out of the box with any USB clicker and shows the toolbar only when I move the pointer.

## UI sketch

- Full-screen fit-page display; floating present toolbar appears on pointer movement and auto-hides after 2.5 s idle.
- Clicker keys: `PageDown`/`Right`/`Down`/`Space`/`Enter` (next), `PageUp`/`Left`/`Up`/`Backspace` (prev), `Home`/`End`, `B`/`.` (black screen), `W`/`,` (white screen), `Escape` (exit).

## Mode

Present mode (`F5` / command palette `present.start`).

## Speed and size impact

- Startup: zero cost.
- Pure state machine; zero re-renders for toolbar visibility or blanking overlays.

## Design

- `PresentSession`, `ClickerKey::from_key_name`, `ScreenBlank::{None, Black, White}`, `ToolbarVisibility::{Hidden, Visible}`, `PresentAction`.

## Acceptance tests

- [x] `usb_clicker_keys_navigate_blank_and_exit_without_setup`: verifies PgUp/PgDn, black/white blanking (`B`, `.`, `W`), pointer-triggered toolbar auto-hide, and `Escape` exit.
