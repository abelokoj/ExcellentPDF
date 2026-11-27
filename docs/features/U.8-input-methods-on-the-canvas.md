# Ph3: 49/49 | U.8: Input methods on the canvas

**Tag:** U.8; **Priority:** P0; **Phase:** 3; **Needs:** K.8; **Code:** `ecclesia-features::notebook::ime`, `ecclesia-app::notebooks`
**Plan:** §27 entry `#### Ph3: 49/49 | U.8` and Appendix D.1.4; `dev notes/dev_decisions.md` DD26. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Writers using Chinese, Japanese, Korean, or dead-key input methods need preedit composition, candidate-window positioning, and commit events to work seamlessly when typing text blocks or editing annotations and form fields on a page.

## User story

As a CJK reader and note-taker, I want my system input method's preedit text and candidate list to work cleanly when composing typed text blocks and annotations, so that I can write in my language without garbled or dropped characters.

## UI sketch

- Typed text blocks (`N.6`), annotation notes (`A.6`), and all dialog inputs accept OS IME composition and commit events.
- Preedit text displays inline while composing and commits cleanly into the active `TextBlock` without splitting characters or losing selection state.
- Works with keyboard navigation and screen readers via NSH's native text input controls.

## Mode

Write, Annotate, and Form-filling workflows (all modes where text entry occurs).

## Speed and size impact

- Startup: zero startup cost; IME bridge state is created only when a text block editor opens.
- Budgets touched: none at startup or idle.
- Installer size: pure Rust model in `ecclesia-features`; relies on OS IME frameworks via NSH.

## Design

- Model (`ecclesia_features::notebook::ime`): `ImeBridge` and `ImeEvent` (`Preedit`, `Commit`, `Cancel`) tracking active composition ranges and committing UTF-8 text cleanly into `TextBlock`.
- View (`ecclesia-app::notebooks::text_block_dialog`): wires `ImeBridge` with NSH text input (Windows IMM32/TSF, macOS `NSTextInputClient`, GTK `GtkIMContext` per DD26).
- Direct canvas IMM32 / `NSTextInputClient` / `GtkIMContext` candidate-window placement for future direct-on-page editing (forms `O.11`, text runs `O.5`) remains tracked as open work.

## Acceptance tests

- [x] `ime_bridge_commits_into_text_block`: IME preedit and commit events update a typed text block cleanly.
- [x] `notebook::ime::tests`: unit tests verify preedit, replacement, cancellation, and UTF-8 boundary handling.
- [ ] Tried by hand with CJK input methods on Windows, macOS, and Linux in text boxes and form fields.

## Open

- `ImeBridge` is wired into typed text blocks (`N.6`) and all dialog/inspector text fields use NSH's native IME; direct on-canvas candidate-window placement for Windows IMM32/TSF, macOS, Linux, and tablets plus hand-testing with CJK IMEs remain open.
