# Ph1: 42/60 | V.6: Closing panes

**Tag:** V.6; **Priority:** P0; **Phase:** 1; **Needs:** V.3; **Code:** `ecclesia-app::panes` (`layout`, `grid`), `ecclesia-app::shell`, `ecclesia-app::commands`
**Plan:** §27 entry `Ph1: 42/60 | V.6` and §11.15.

## Problem

Closing a pane must not throw away the tabs in it, or the edits in them.

## User story

As a reader, I want to close a pane and keep its tabs, or close them, as I choose.

## UI sketch

- **Close pane** (Ctrl+Alt+W), or the close button on the pane's header. A pane left with no tab closes too.
- Settings window, Tabs, "Closing a pane": **Move its tabs** (the default) to the pane beside it, or **Close its
  tabs** (they wait in Reopen closed tab). Key: `close_pane_tabs` = `"move"` or `"close"`.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `Layout::close` picks the pane beside; `TabState::take_all` / `adopt` / `remember_closed` move the tabs.
- Edits are saved 2 seconds after the last one and whenever a pane stops showing a file (K.15), so no prompt is
  shown on close; the plan's "prompts" step is not built (listed in the plan).

## Acceptance tests

- [x] `closing_a_pane_hands_its_tabs_to_the_one_beside_it`: the pane beside takes the closing pane's place
- [x] `a_closing_pane_gives_up_every_tab_the_shown_one_last`: a closing pane gives up every tab
- [x] `closing_tabs_reads_its_setting`: the setting reads "move" or "close"
