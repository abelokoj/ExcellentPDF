# ADR 0005: One native page canvas per platform

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D5 (plan §3)

## Context

Reading and inking need tiled GPU rendering, and pen input with pressure and tilt drawn within one display frame.

## Decision

Draw pages in one native component, `ecclesia.page-canvas`, per platform (a child window on Windows), over a shared Rust core.

## Why

NSH has no document view and no pen pressure, and ink latency needs direct OS input.

## Consequences

Each platform has its own canvas crate (`ecclesia-canvas-win`, `-mac`, `-gtk`, `-ios`, `-android`); shared logic lives in `ecclesia-canvas`. Pen input is read from the OS directly, never through NSH pointer events.
