# Ph0: 6/13 | K.8: Page canvas on Windows

**Tag:** K.8; **Priority:** P0; **Phase:** 0–1; **Needs:** K.7; **Code:** `crates/ecclesia-canvas-win/`
**Plan:** §27 entry `Ph0: 6/13 | K.8`, §7.1–§7.3, Appendix C.4.

## Problem

NSH's cross-platform pointer events lack raw Windows pen pressure, tilt, and coalesced history samples, and high-throughput PDF tile composition needs a native Direct2D/DXGI child window.

## User story

As a Windows reader or Surface stylus user, I want smooth flip-model Direct2D page rendering and full-fidelity `WM_POINTER*` pen input with a software GDI fallback when GPU swap chains fail, so that scrolling and writing feel immediate on every PC.

## UI sketch

Hosted inside NSH via the `ecclesia.page-canvas` native component, receiving JSON props/commands and emitting selection, navigation, link, and ink events.

## Mode

All modes.

## Speed and size impact

D3D11/Direct2D device initializes off the critical first-paint path; flip-model swap chain uses frame-latency waitable objects.

## Design

- Native Win32 child `HWND` registered as `ecclesia.page-canvas`.
- `WM_POINTERDOWN`/`UPDATE`/`UP` with `GetPointerPenInfo` and `GetPointerPenInfoHistory` normalized into `PointerSample`.
- Direct2D composition swap chain with automatic GDI software-blit fallback.

## Acceptance tests

- [x] `ecclesia-canvas-win` pointer normalization, placement, and saved view unit tests.
- [ ] Manual Surface Pro hardware latency verification (`<= 1 display frame`).
