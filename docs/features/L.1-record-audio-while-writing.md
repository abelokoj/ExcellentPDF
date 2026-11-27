# Ph4: 25/44 | L.1: Record audio while writing

**Tag:** L.1; **Priority:** P0; **Phase:** 4; **Needs:** H.9, N.4; **Code:** `ecclesia-features::audio`
**Plan:** §27 entry `#### Ph4: 25/44 | L.1`.

## Problem

Students taking handwritten lecture notes miss spoken context and need audio synchronized to their pen strokes so tapping a note jumps straight to the explanation.

## User story

As a student in a lecture hall, I want to record audio while handwriting notes and tap any stroke written 10 minutes into the lecture to hear that exact moment (±0.5 s) with strokes highlighting in sync.

## UI sketch

- Write toolbar: Record/Pause/Stop mic button with live elapsed indicator; audio scrubber bar during playback; tapping any stroke seeks playback to its recorded timestamp.
- Requests microphone permission where required (macOS, iPad, Android).

## Mode

Write and Study modes.

## Speed and size impact

- Startup: zero cost; audio capture/playback loads only when the user starts recording or playback.
- Audio stores inside `.expdf` under `assets/audio/<id>.opus` or `.wav`.

## Design

- `audio::device`: mockable `AudioCapture` and `AudioPlayback` traits plus `MicPermission` (`Granted`, `Denied`, `PromptRequired`, `NotRequired`) and `MockAudioCapture`/`MockAudioPlayback`.
- `audio::codec`: 16-bit PCM WAV encoder/decoder and RFC 7845 Ogg/Opus container muxer/demuxer with CRC-32 verification and `.expdf` `assets/audio/` bundling (`bundle_audio_in_notebook`).
- `audio::sync`: `AudioTimeline` tracks pause/resume `RecordingSegment`s on the ink sample clock, maps any `ecclesia_ink::stroke::Stroke` to its playback offset, seeks playback, and reports highlighted strokes and page marks during playback.

## Acceptance tests

- [x] `tapping_stroke_written_ten_minutes_into_lecture_seeks_within_half_second`: records a 15-minute lecture with a 30-second pause, taps a stroke written at 10 minutes into recorded audio, and verifies playback seeks within ±0.5 s and highlights active strokes.
- [x] `wav_and_ogg_opus_round_trip_and_bundle_in_expdf`: verifies WAV and Ogg/Opus round-trip codecs and `.expdf` archive persistence.
