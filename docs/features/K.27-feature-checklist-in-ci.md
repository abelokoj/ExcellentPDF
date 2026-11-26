# Ph0: 12/13 | K.27: Feature checklist in CI

**Tag:** K.27; **Priority:** P0; **Phase:** 0; **Needs:** K.1; **Code:** `docs/features.toml`, `tests/check_features.ps1`
**Plan:** §27 entry `Ph0: 12/13 | K.27`, Appendix C.3.2, C.3.3.

## Problem

Tracking over two hundred feature tags across multiple phases in prose alone risks claiming a feature is shipped when its tests or docs do not exist.

## User story

As a maintainer, I want a single machine-checked feature ledger in `docs/features.toml` that verifies shipped entries against real tests in `crates/` and prints per-area progress counts, so that readiness reports remain honest.

## UI sketch

None; CLI/CI script (`pwsh tests/check_features.ps1`).

## Mode

All modes.

## Speed and size impact

Zero runtime or binary impact.

## Design

- **`docs/features.toml`:** one TOML table per permanent feature tag recording `label`, `status` (`planned`, `in-progress`, `shipped`, `dropped`), `tests`, and `open` notes.
- **`tests/check_features.ps1`:** parses `docs/features.toml`, validates status values, confirms every `shipped` row names non-empty tests that exist in `crates/`, and prints a per-area milestone progress table.

## Acceptance tests

- [x] `tests/check_features.ps1`: validates `docs/features.toml` entries and prints per-area shipped/in-progress/planned/dropped summary counts.
