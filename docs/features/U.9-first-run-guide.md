# Ph1: 60/60 | U.9: First-run guide and tips

**Tag:** U.9; **Priority:** P1; **Phase:** 1; **Needs:** K.18; **Code:** `excellent-features::onboarding` (model),
`excellent-app::onboarding` (view)
**Plan:** §27.18 entry `Ph1: 60/60 | U.9`.

## Problem

A new reader sees a clean window and doesn't know that every command is one key away, or where the reading options
are. The app hides its chrome on purpose (plan §2.1, rule 3), so it has to say once where things are.

## User story

As a first-time reader, I want three short steps that show me how to open a document, make reading mine and find any
command, and then a tip at the moment it helps, so I learn the app without a manual and without being held up.

## UI sketch

- **Guide** (a dialog card, the quick dialogs' look): "Step 1 of 3", a title, two lines of text with the reader's own
  keys (from the keymap, so a rebound key shows as bound). Steps: **Open a document** (Ctrl+O, drop files), **Pick a mode, make
  reading yours** (the modes bar and its Customize button, then layouts, page colours and the ruler in Settings), **Every command, one key away**
  (Ctrl+K). Buttons: **Skip** (first, so Enter on the first focus doesn't move on unseen), **Open a document** (first
  step only: closes the guide and opens the file dialog), **Back**, **Next** / **Start reading** (primary). Escape
  skips.
- **Tips** (a 36-point strip below the pages, the print strip's look): "Tip: press Ctrl+K to find any command by
  typing its name." the first time a document shows; "Tip: Ctrl+Shift+A lists every open tab; type to find one." the
  first time two tabs are open. Buttons **Turn off tips** and **Got it**. One tip at a time; never while the guide is
  open.
- **Getting the guide back:** the palette's **Show the getting-started guide**; Settings, **Getting started**, **Show
  the guide**. No Help menu yet: the plan's "replay guide in Help" is the palette and the
  settings section until a menu exists.
- **Settings, Getting started:** **Show tips** (on by default).

## Mode

Every mode; it lies over the window, not in a mode's tools.

## Speed and size impact

- Startup: one settings flag is read when the window is built; the guide's dialog is built only when it shows. No
  timing measured.
- Tips: one effect on the active pane's tab count; a settings write the one time a tip shows.
- Installer: a few kilobytes of code and 25 strings per language.

## Design

- **Model** (`excellent-features::onboarding`, pure Rust): `Step` (Open, Read, Palette) and `Guide` (where the reader
  is: `next`, `back`, `is_first`, `is_last`); `opens_at_start(seen, launch_files)`, the rule that keeps the guide off
  any file the launch opens; `Tip` and `SeenTips` (the seen list as the settings file keeps it: ids separated by
  commas, unknown ids dropped).
- **View** (`excellent-app::onboarding`): `Onboarding` (signals for the guide and the tip, and the settings), the
  guide dialog, the tip strip. Only the desktop binary offers them (`offer_first_run_guide`), so the shell's headless
  tests never meet a dialog they didn't open.
- **Files from the command line:** a start with files doesn't open the guide and doesn't count it seen; it waits for
  a start without files. A later launch that brings files closes the guide (again not counted seen), so the files
  show at once.
- **Command:** `help.guide` (`CommandId::ReplayGuide`), no default key.
- **Settings:** `guide_seen` (flag, set on Skip, Escape, Start reading or Open a document), `tips_seen` (text),
  `show_tips` (flag, default on; in the settings window). A tip counts as seen when it shows.
- **Needs:** K.18 (modes and toolbars) is not built. The plan's "modes" step teaches the reading options instead,
  and the replay lives in the palette and settings; when K.18 lands, the second step names the modes and their
  toolbars, and the guide gets a toolbar button (flagged in `dev notes/dev_decisions.md`, DD7).

## Acceptance tests

- [x] `the_guide_never_opens_over_files_from_the_command_line` (the rule; the guide never blocks a file passed on the
  command line)
- [x] `the_guide_steps_forward_and_back_within_its_steps`
- [x] `each_tip_shows_once_and_the_seen_list_survives_the_settings_file`
- [x] `the_guide_opens_at_first_start_steps_and_comes_back_from_the_palette` (headless: the guide, Skip, the palette
  command, Escape)
- [ ] **Not tried by hand**: the guide on a real first start, a file opened from Explorer while it shows, the tips.
