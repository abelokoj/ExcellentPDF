# Ph4: 39/44 | G.2: Laser with trails

**Tag:** G.2; **Priority:** P0; **Phase:** 4; **Needs:** G.1, K.8; **Code:** `ecclesia-features::present::pointers`
**Plan:** §27 entry `#### Ph4: 39/44 | G.2`.

## Problem

Presenters need a high-visibility laser dot with fading gesture trails that follows the pointer smoothly in the same frame without re-rendering PDF tiles.

## User story

As a presenter, I want a glowing laser pointer with fading trails so the audience can follow my emphasis in real time.

## UI sketch

- Present toolbar: Laser button (`L`), solid/hollow dot style, configurable width (`0.1..=32.0` pt, default `8.0` pt) and trail lifetime (`2.0 s`).

## Mode

Present mode.

## Speed and size impact

- Canvas overlay only; zero PDF re-rendering.

## Design

- `LaserPointer`, `LaserMode::{Solid, Hollow}`, `normalize_laser_width`, and `LaserStroke::is_expired` ported from SumatraPDF Enhanced's `Canvas.cpp` laser pointer rules.

## Acceptance tests

- [x] `laser_width_and_expiry_match_sumatrapdf_enhanced`: verifies `normalize_laser_width` clamping/NaN fallback, active-stroke non-expiry, exact lifetime expiry, and same-frame pointer tracking.
