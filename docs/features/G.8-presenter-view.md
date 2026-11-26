# Ph4: 43/44 | G.8: Presenter view

**Tag:** G.8; **Priority:** P0; **Phase:** 4; **Needs:** G.1, K.19; **Code:** `excellent-features::present::presenter`
**Plan:** §27 entry `#### Ph4: 43/44 | G.8`.

## Problem

Presenters using a projector need a private screen with current/next slide previews, speaker notes, an elapsed timer, and the wall clock while the audience sees only the slide—and the session must survive an HDMI cable being bumped or unplugged.

## User story

As a conference speaker, I want synchronized audience and presenter windows across any two monitors that keep running seamlessly even if the projector disconnects.

## UI sketch

- Second window automatically opens on the primary display (audience window on secondary, with a Swap Displays button) showing current slide, next slide, speaker notes, timer, and clock.

## Mode

Present mode.

## Speed and size impact

- Startup: zero cost; multi-monitor state is pure Rust with zero polling.

## Design

- `PresenterViewSync` coordinates `PresentSession`, `PresenterNotes` (from annotations or `---`-separated notes text), `PresenterTimer`, and `DisplayInfo` monitor assignments (`swap_displays`, `on_displays_changed`, `presenter_snapshot`).

## Acceptance tests

- [x] `presenter_view_syncs_two_monitors_and_survives_display_unplug`: verifies automatic dual-monitor assignment, display swapping, lockstep slide/notes/timer synchronization, graceful single-monitor degradation when the projector unplugs, and automatic dual-monitor restoration on reconnect.
