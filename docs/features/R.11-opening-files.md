# Ph1: 22/60 | R.11: Opening files: drag and drop, file associations, single instance, command line

**Tag:** R.11; **Priority:** P0; **Phase:** 1; **Needs:** K.19; **Code:** `excellent-app::open`,
`excellent-platform` (`instance`, `associations`), `excellent-app::tabs` (`Tabs::launch`)
**Plan:** §27 entry `Ph1: 22/60 | R.11`.

## Problem

A PDF reader is opened from outside far more often than from its own Open dialog: by double-clicking a file, by
dragging one onto the window, or from a script. Each must land in the window already open, not a second copy of the
app.

## User story

As a reader, I want a file I double-click, drop on the window or name on the command line to open as a tab in the
app I already have open, so that my documents stay together in one window.

## UI sketch

- Dropping files on the window opens each document in a new tab, the last shown. Files that aren't documents (by
  their first bytes, not their names) and folders are left out.
- Double-clicking a document, or running `excellentpdf a.pdf --page 5`, while the app runs: the running window comes
  to the front and shows the file (its tab if it has one), at page 5.
- Command palette: "Set as default PDF viewer" offers the app for its document types and opens Windows' Default apps
  settings at ExcellentPDF, where the user makes it the default (Windows lets no app do that itself).

## Mode

All modes.

## Speed and size impact

- Startup: one attempt to reach a running instance's pipe before anything else loads. A second launch ends there,
  before settings, the library or a window, so it is gone in the time it takes to start a process.
- The first instance keeps one thread blocked on its pipe; it uses no CPU until a launch arrives.
- A drop reads the first kilobyte of each dropped file on the UI thread.
- Installer: code and strings only.

## Design

- Command line (`open::parse`, pure): files (made absolute in the launching process) and `--page N` (from 1).
- Single instance (`excellent_platform::instance`): a named pipe, `\\.\pipe\<app id>-<user>`, inbound only, local
  clients only, its default security letting only the user write to it; a Unix socket elsewhere. The first launch
  makes it with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so two first launches at once can't both listen; the loser sends to
  the winner. Messages are JSON, at most 64 KB; one that isn't a launch with absolute paths is refused. A thread
  queues each launch, and the window's task shows it, after the sender allowed the window to come to the front.
- Drops (`open::dropped_documents`): `file:` URLs decoded as links are; a file is a document when R.28's
  `excellent_core::formats::is_document` says so, by its signature (Markdown by its name).
- File associations (`excellent_platform::associations`): for this user, a ProgID `ExcellentPDF.Document`,
  "Open with" entries for each extension, and Capabilities with RegisteredApplications so Default apps lists the app.
  The user's own choice of default (the extension's default and UserChoice) is never written. The entries are a pure
  list, tested; writing them is one loop.

## Acceptance tests

From the entry's **Done when** ("double-clicking a PDF in the file manager opens it as a tab in the running
instance"):

- [x] `a_second_launch_hands_its_message_to_the_first`: two launches through the real pipe.
- [x] `the_channel_is_per_user`.
- [x] `the_command_line_gives_absolute_files_and_a_page`, `a_launch_crosses_the_channel_unchanged`: parsing and
  the message, with bad messages refused.
- [x] `a_launch_shows_its_files_in_their_tabs_or_new_ones`: the tabs a launch leaves.
- [x] `dropped_files_are_taken_by_their_contents_not_their_names`.
- [x] `registering_offers_every_type_and_lists_the_app_in_default_apps`: the registry entries.
- [ ] Tried by hand: double-click with the app running, a drop on the pages and on the strip, "Set as default PDF
  viewer".
- [ ] Registering on install (step 2): there is no installer yet; the command does it until then.
- [ ] Dropping on a pane's edge to split (V.2).
- [ ] `--zoom` (the canvas has no zoom-to-a-value command yet), `--search` (needs find), `--split` (V.13),
  `--forward-search` (R.27): logged as unknown until then.
- [ ] macOS and Linux file types (the app bundle and the `.desktop` file, Phase 2).
