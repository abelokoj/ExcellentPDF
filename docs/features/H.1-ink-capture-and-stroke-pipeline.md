# Ph0: 13/13 | H.1: Ink capture and stroke pipeline

**Tag:** H.1; **Priority:** P0; **Phase:** 0 spike (T8), 3 build; **Needs:** K.8; **Code:** `excellent-ink`
(`sample`, `width`, `predict`, `smooth`, `stroke`, `session`), `excellent-canvas::page::canvas` (pen input, preview,
commit), `excellent-features::annotate::ink` (the committed annotation)
**Plan:** §8.2 and §27 entry `#### Ph0: 13/13 | H.1`.

## Problem

T8's spike drew ink on the Windows canvas, but the strokes lived only in memory: no tilt or timing, a linear width,
no smoothing or prediction, nothing saved, and no way to test a pen session without a pen.

## User story

As a note-taker writing with a pen, I want my strokes to appear under the nib at once, to look smooth, to follow how
hard and how fast I press, and to stay in the document as ink, each one undoable.

## UI sketch

No new controls: write on a page with the pen. Each stroke shows as it is drawn, runs a few pixels ahead of the nib
while moving, and becomes an ink annotation when the pen lifts (Undo removes it).

## Mode

Any mode with a document (the pen moves the app to Write when "pen to write" is on).

## Speed and size impact

Nothing at startup. Per sample: one width from the curve and a constant-velocity prediction over the last eight
samples. On pen-up: smoothing and flattening of one stroke (microseconds), one event to the app.

## Design

- **Capture (step 1):** `PenSample` carries tilt and the sample's own time (Windows: `POINTER_PEN_INFO` tilt and the
  message time); platforms that don't say are timed as samples arrive.
- **Preview (step 2):** unchanged from T8: the compositor trail on Windows, the canvas's own frame elsewhere.
- **Prediction (step 3):** `predict` extrapolates at the velocity of the last 24 ms, at most 24 pixels, 16 ms ahead;
  drawn on the canvas's frame only and replaced by the next frame.
- **Smoothing (step 4):** `filter` steadies the middle of a stroke, `catmull_rom` gives the plan's Béziers, `flatten`
  turns them into points within 0.1 pt (Wang's formula).
- **Width (step 5):** `WidthCurve`: min + (max − min) × p^1.25, widened by tilt, thinned by speed.
- **Commit (step 6):** on pen-up the canvas sends `ink` (`{page, points: [[x, y, width]], color}`); the app adds an
  ink annotation as one step of undo, with the stroke's mean width, until H.9 writes per-point widths. The canvas
  keeps showing the stroke until its page is drawn again with the annotation.
- **Sessions (§16):** `Session` reads and writes a plain-text sample format; `EXCELLENTPDF_RECORD_INK=<folder>`
  records every finished stroke; `tests/sessions/*.session` are replayed by `recorded_sessions_replay`.

## Acceptance tests

- [x] The pipeline's steps: `tilt_gives_altitude_and_azimuth`, `the_response_curve_needs_a_firmer_hand`,
  `leaning_widens_and_speed_thins`, `a_steady_pen_is_followed_ahead`, `the_spline_passes_through_every_point`,
  `flattening_stays_within_the_tolerance`.
- [x] Sessions replay: `a_session_goes_out_and_back`, `recorded_sessions_replay`.
- [x] Commit: `a_finished_stroke_becomes_one_ink_annotation`.
- [ ] Pen-down to pixels within one display frame on the reference Windows device (measured, §13): not measured.

## Open

- The only session fixture is synthetic; record one on the reference device (DD24).
- No OS prediction is used (Windows has none; iOS and Android come later); GTK and AppKit give no tilt or sample
  time yet.
- The saved annotation has one width (the mean) until H.9; ink is not limited to a mode.
