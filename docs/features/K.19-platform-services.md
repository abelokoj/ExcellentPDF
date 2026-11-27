# Ph1: 9/60 | K.19: Platform services

**Tag:** K.19; **Priority:** P0; **Phase:** 1 (Windows), Phase 2 (macOS, Linux), Phase 8 (tablets); **Needs:** K.1; **Code:** `crates/ecclesia-platform`
**Plan:** §27 entry `#### Ph1: 9/60 | K.19`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Feature crates need OS services—single-instance activation, file-type registration, printing, and system speech synthesis—without coupling feature logic directly to Win32, AppKit, or Linux IPC APIs.

## User story

As a user opening PDFs from Explorer or another app, I want a second launch to hand its files to my running window and OS features like printing and speech to work through native platform integrations without breaking cross-platform tests.

## UI sketch

- No dedicated UI of its own; surfaces through OS shell integration ("Open with Ecclesia PDF", Default Apps registration), the native print dialog (`O.22`), and Read Aloud (`U.5`).
- Error states (unavailable system speech synthesizer, failed IPC connect, cancelled print dialog) propagate structured errors to the calling feature UI.

## Mode

Every mode; invisible infrastructure layer.

## Speed and size impact

- Startup: single-instance named pipe / Unix socket check runs once at launch; printing and speech backends initialize lazily on first use.
- Budgets touched: cold start (plan §13.1).
- Installer size: thin OS wrappers using system libraries already linked by the app.

## Design

- `ecclesia_platform::instance`: per-user single-instance IPC channel (named pipe on Windows, Unix domain socket on macOS/Linux) forwarding launch arguments and file paths.
- `ecclesia_platform::associations`: Windows per-user ProgID and Default Apps registration for supported document extensions.
- `ecclesia_platform::print`: native print job / page-banding integration for `O.22`.
- `ecclesia_platform::speech`: `SpeechSynth` trait and `system_synth()` factory (Windows SAPI / macOS / Linux speech backends, returning an explicit error when unavailable; `MockSpeechSynth` restricted to tests).

## Acceptance tests

- [x] `a_second_launch_hands_its_message_to_the_first`: single-instance IPC delivers launch messages end to end.
- [x] `registering_offers_every_type_and_lists_the_app_in_default_apps`: file-type registration covers supported formats.
- [ ] Full cross-platform trait set (clipboard rich formats, key store, notifications, memory-pressure events) and integration tests on macOS and Linux.

## Open

- Single instance, file associations, print, and speech synthesis are implemented; key store, notifications, memory-pressure events, and macOS/Linux file-association tests remain open.
