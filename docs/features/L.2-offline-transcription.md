# Ph5: 23/45 | L.2: Offline transcription

**Tag:** L.2; **Priority:** P1; **Phase:** 5; **Needs:** L.1, K.21, F.3; **Code:** `excellent-features::audio::transcribe`
**Plan:** §27 entry `#### Ph5: 23/45 | L.2: Offline transcription`.

## Problem

Recorded lecture audio (`L.1`) is hard to skim or search by keyword unless transcribed to timestamped text linked to the pages that were open.

## User story

As a student reviewing a recorded lecture, I want offline transcription (Whisper model packs) to index spoken words against timestamps and open pages so searching a term jumps right to that moment and page.

## UI sketch

- Audio side panel shows synced `TranscriptSegment` rows, model selector (`Tiny`..`Medium` packs), **Export SRT / VTT**, and click-to-seek.
- Global Search (`F.3`) queries `TranscriptStore::search`.

## Mode

Write / Study (Audio side panel).

## Speed and size impact

- Startup: zero cost; runs in background after recording; Whisper weights remain optional packs.
- Storage: SQLite FTS5 (`audio_transcript_segment` + `audio_transcript_fts`).

## Design

- **Model (`excellent-features::audio::transcribe`):**
  - `Transcriber` trait + `WhisperModelSize` / `WhisperModelPack` + `MockTranscriber`.
  - `link_segments_to_timeline` binds each segment to its active page via `AudioTimeline::page_marks`.
  - `TranscriptStore` indexes segments in FTS5 and exports/imports `.srt` and `.vtt`.

## Acceptance tests

- [x] `audio::transcribe::tests::searching_spoken_word_finds_moment_and_page`: verifies timeline page linking, FTS5 spoken-word search returning timestamp + page, and SRT/VTT round-trip.
