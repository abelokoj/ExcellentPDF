# Ph0: 4/13 | K.4: Render workers and threading

**Tag:** K.4; **Priority:** P0; **Phase:** 0; **Needs:** K.3; **Code:** `crates/excellent-core/src/render.rs`, `crates/excellent-core/tests/render_stress.rs`
**Plan:** §27 entry `Ph0: 4/13 | K.4`, §5.3, §7.4.

## Problem

Rendering display lists, tiles, and text extraction on the UI thread stalls scrolling and ink responsiveness.

## User story

As a reader scrolling or zooming a complex PDF, I want rendering and text extraction to run on prioritized background workers that cancel immediately when a page leaves the viewport, so that the UI stays responsive at 60+ fps.

## UI sketch

None.

## Mode

All modes.

## Speed and size impact

Worker pool runs off the UI thread; `fz_cookie` abort flags cancel running vector renders within 20 ms locally.

## Design

- `RenderPool` scheduling `build_display_list`, `render_tile`, `extract_text`, and `read_text` jobs by `Priority`.
- Cancellable `Job` handles backed by `excellent_mupdf::Cookie` (`fz_cookie::abort`), with `try_take` and `when_done` UI wakeup hooks.

## Acceptance tests

- [x] `four_workers_render_one_document_ten_thousand_times` (`crates/excellent-core/tests/render_stress.rs`).
- [x] `a_cancelled_job_reports_it`, `cancelling_a_running_render_stops_it_within_20_ms`.
