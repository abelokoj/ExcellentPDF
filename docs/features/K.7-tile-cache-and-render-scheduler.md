# Ph0: 5/13 | K.7: Tile cache and render scheduler

**Tag:** K.7; **Priority:** P0; **Phase:** 0–1; **Needs:** K.4, K.6; **Code:** `crates/excellent-canvas/src/tiles.rs`, `crates/excellent-canvas/src/viewport.rs`
**Plan:** §27 entry `Ph0: 5/13 | K.7`, §7.4.

## Problem

Re-rendering whole pages on every pan or zoom frame wastes CPU/GPU time and blows memory budgets on large documents.

## User story

As a reader zooming and panning smoothly through long documents, I want pages cached as 256×256 quantized-zoom tiles with LRU eviction and off-screen cancellation, so that frames appear without stalls and memory stays bounded.

## UI sketch

Visible pages show cached or low-resolution fallback tiles immediately while sharp tiles stream in asynchronously.

## Mode

All modes.

## Speed and size impact

256×256 device-pixel tiles with 1/4096 zoom bucket quantization and byte-budget LRU eviction keep memory bounded.

## Design

- Shared platform-agnostic `excellent-canvas` tile cache, viewport math, zoom-bucket quantization, and cancellation of off-screen jobs.
- Integrates with `excellent-core::render::RenderPool` via `try_take` and `when_done` wakeup callbacks.

## Acceptance tests

- [x] `excellent-canvas` tile cache and viewport unit tests.
- [ ] Full 60 fps 1,000-page vector scroll trace and memory-pressure eviction check (K.24).
