# Ph7: 5/32 | L.4: Screen and pen recording to video

**Tag:** L.4; **Priority:** P2; **Phase:** 7; **Needs:** L.1; **Code:** `ecclesia-features::screencast`
**Plan:** §27 entry `#### Ph7: 5/32 | L.4`.

## Problem

Educators and presenters annotating slides live need synchronized video capture of rendered pages, ink strokes, cursor/laser overlays, page turns, and microphone audio without drift over long sessions.

## User story

As an educator recording a lecture over PDF slides, I want to record screen frames, pen strokes, and audio in lockstep and export a synced video file.

## UI sketch

- Recording controls in Lecture / Present modes showing elapsed duration, active page, audio level, and pause/stop actions.

## Mode

Present and Lecture recording modes.

## Speed and size impact

- Frame and PCM buffers stream directly into the `VideoEncoderBackend` with monotonic timestamp alignment against `AudioTimeline`.

## Design

- **`VideoEncoderBackend` & `MockVideoEncoder`**: Platform-abstracted muxer interface (`begin`, `push_video_frame`, `push_audio_chunk`, `finish`) enabling Windows Media Foundation / macOS AVAssetWriter / Linux GStreamer backends alongside deterministic offline testing.
- **`ScreencastSession`**: Ties `AudioTimeline` elapsed audio clock with RGBA video frames, pen stroke metadata (`StrokeEvent`), and page-switch markers; tracks maximum A/V skew (`max_av_skew_ms`).

## Acceptance tests

- [x] `thirty_minute_recording_stays_in_audio_video_sync`: simulates a 30-minute multi-page recording with pen strokes and audio chunks, asserting A/V drift stays bounded and muxed output succeeds.
