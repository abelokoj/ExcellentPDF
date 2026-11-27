# Ph4: 44/44 | U.5: Read aloud

**Tag:** U.5; **Priority:** P0 (parity); **Phase:** 4; **Needs:** K.13, K.19; **Code:** `ecclesia-platform::speech`, `ecclesia-features::read_aloud`
**Plan:** §27 entry `#### Ph4: 44/44 | U.5` and `sumatra_pdf_enhanced_plan.md` §5.13. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers with visual fatigue, dyslexia or multitasking workflows need the app to read documents aloud from the current page, cursor or selection while highlighting each spoken word and sentence accurately across page turns.

## User story

As a reader, I want to start read-aloud from the page top, cursor or selection, adjust voice and speed, pause or resume, and watch word and sentence highlights track the spoken words continuously across an entire chapter.

## UI sketch

- Triggered via Read/Study menu, optional toolbar button, context menu ("Read from here", "Read selection"), keyboard shortcut and command palette (`read_aloud.start`, `read_aloud.pause_resume`, `read_aloud.stop`).
- Floating playback bar shows Play/Pause, Stop, speed selector (`0.5×`–`3.0×`) and voice picker; canvas draws sentence and word highlight quads from `ReadAloudHighlight`.
- Offline-ready using OS voices; error state surfaces a clear message if no voice is available.

## Mode

All modes (Read, Annotate, Study; optional toolbar toggle).

## Speed and size impact

- Startup: zero startup cost; COM/SAPI 5 initializes lazily only when Read Aloud is invoked.
- Budgets touched (plan §13.1): none at startup; word-boundary mapping uses pre-built O(1) index arrays per page.
- Installer size: zero new crates; uses existing workspace `windows` COM bindings on Windows.

## Design

- **Platform trait & Windows backend** (`ecclesia-platform::speech`): `SpeechSynth`, `VoiceInfo`, `SpeechEvent` (`WordBoundary`, `Finished`, `Error`), `SpeechError`, `clamp_speed`, `speed_to_sapi_rate`, `MockSpeechSynth`, and `WindowsSpeechSynth` (SAPI 5 COM `ISpVoice` / `ISpEventSource` / `ISpObjectTokenCategory` with RAII `Drop` and UTF-16-to-char offset mapping).
- **Mapping & playback state machine** (`ecclesia-features::read_aloud`): `SpokenPage` (maps cleaned reading-order text back to exact K.13 `PageText` character indices, joining hyphenated line wraps), `sentence_range` (handling abbreviations like `e.g.`, CJK punctuation and paragraph breaks), `cursor_start_char`, `ReadAloudHighlight`, `ResumePoint`, `PlaybackState`, and `ReadAloudSession<S: SpeechSynth>` advancing seamlessly across pages and skipping empty/image-only pages.

## Acceptance tests

- [x] `read_aloud::tests::highlighting_tracks_every_spoken_word_across_a_full_chapter`: word and sentence K.13 quads track every spoken word continuously across a multi-page chapter (including hyphenated wraps and blank figure pages).
- [x] `read_aloud::tests::sentence_range_handles_abbreviations_and_cursor_stop_resume`: cursor start, abbreviation-aware sentence ranges, pause/resume and stop/continue resume at the exact word.
- [x] `ecclesia-platform::speech::tests::windows_sapi_initializes_and_enumerates_voices`: Windows SAPI 5 backend initializes cleanly and enumerates system voices.
