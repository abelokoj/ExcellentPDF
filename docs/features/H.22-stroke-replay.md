# Ph4: 12/44 | H.22: Stroke replay

**Tag:** H.22; **Priority:** P2; **Phase:** 4; **Needs:** H.9, K.14; **Code:** `excellent-ink::replay`, `excellent-features::ink::replay`
**Plan:** §8.3 and §27 entry `#### Ph4: 12/44 | H.22`.

## Problem

Once ink strokes are committed to a page, the temporal order and rhythm in which a derivation, diagram, or solution was written are lost. Students reviewing lecture notes or teachers demonstrating step-by-step derivations cannot watch the strokes unfold in time.

## User story

As a student reviewing handwritten math derivations, I want to replay a page's ink in the exact order and timing it was written with play, pause, scrub, and speed controls so I can follow how the solution was built.

## UI sketch

Opened from the Write/Review toolbar, View menu, command palette (`ink.replay`, "Replay page strokes"), or shortcut `Alt+Shift+P`. Shows a compact floating transport bar at the bottom of the page with Play/Pause (`Space`), Stop/Rewind, Step Back/Forward by stroke, Speed selector (`0.5×`, `1×`, `1.5×`, `2×`, `4×`), and accessible timeline scrubber slider.

## Mode

Read, Write, and Study.

## Speed and size impact

- Startup: nothing runs at startup; timeline is constructed only when replay starts on the current page.
- Budgets touched: `frame_at` evaluates visible strokes in O(strokes + active points) (< 0.1 ms per 60/120 Hz frame).
- Installer size: negligible (~6 KB compiled Rust, zero new dependencies).

## Design

- **Raw sample storage (`excellent-ink::replay`):**
  - `encode_samples` / `decode_samples` pack raw `Sample` streams (origin `f64` ms + zigzag varint microsecond deltas + flags + `x, y`, optional pressure and tilt) into a compact binary format (`EXS1`) with 1 µs timing resolution.
  - `encode_samples_hex` / `raw_samples_to_json` in `excellent-features::ink::replay` serialize raw sample payloads for annotation private JSON storage (H.9 step 3).
- **Replay engine (`excellent-ink::replay::Timeline` + `excellent-features::ink::replay::ReplayPlayer`):**
  - Chronologically sorts strokes by first sample timestamp, normalizes the timeline to `0.0 ms`, and optionally compresses very long idle pauses (`max_pause_ms`, `None` for verbatim 1:1 timing).
  - `frame_at` computes complete strokes and smooth linearly interpolated in-progress stroke tips at any elapsed millisecond.
  - `ReplayPlayer` provides `play`, `pause`, `toggle_play_pause`, `stop`, `seek`, `seek_progress`, `set_speed` (`0.25×`–`8.0×`), `step_stroke_forward`, `step_stroke_backward`, and `advance(dt_ms)`.
  - Legacy ink annotations without stored raw samples receive deterministic arc-length fallback timing.
- **Command ids (for app wiring):** `ink.replay` (`Alt+Shift+P`), `ink.replay_toggle` (`Space` while replaying), `ink.replay_speed`.

## Acceptance tests

- [x] `replay_timing_matches_the_recording`: loads the recorded `synthetic-e-line-circle.session` fixture, round-trips it through private-key JSON encoding, replays at `1.0×` and `2.0×`, and proves every stroke starts and finishes at its exact recorded timestamp and total duration matches the recording.
- [x] `raw_samples_encode_and_decode_within_one_microsecond`: verifies lossless coordinate/pressure/tilt/tool round-trip and < 0.002 ms timestamp accuracy.
- [x] `timeline_replays_in_exact_recorded_order_and_interpolates_mid_stroke`, `max_pause_compresses_long_idle_gaps_without_changing_stroke_durations`, `pause_seek_and_legacy_fallback_work`: verify sorting, sub-sample interpolation, pause freeze, stroke stepping, pause clamping, and legacy stroke fallback.
