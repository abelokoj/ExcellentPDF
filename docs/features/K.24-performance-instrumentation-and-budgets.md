# Ph0: 10/13 | K.24: Performance instrumentation and budgets

**Tag:** K.24; **Priority:** P0; **Phase:** 0 onward; **Needs:** K.1; **Code:** `tests/budgets.ps1`, `tests/perf/`, `tests/size/`
**Plan:** §27 entry `Ph0: 10/13 | K.24`, §13.1, Appendix D.3.10.

## Problem

Without automated size, startup, render-open, and memory budgets enforced in CI, performance regressions slip in unnoticed across pull requests.

## User story

As a maintainer, I want every pull request measured against explicit §13.1 size and performance budgets, so that regressions over 10 % fail CI immediately.

## UI sketch

Reported in the GitHub Actions job summary table by `tests/budgets.ps1`.

## Mode

All modes (measured on release builds).

## Speed and size impact

Zero runtime overhead in release builds beyond debug-gated first-page timing log lines used by the budget harness.

## Design

- **`tests/size/budgets.json`:** `core_installer_mb` (15 MB ceiling, LZX:21 cabinet baseline tracked).
- **`tests/perf/budgets.json`:** §13.1 ceilings for cold start (300 ms), open typical PDF (150 ms), open 10,000-page PDF (400 ms), 60/120 fps scroll, 1-frame pen latency, 100 ms search index, 60 MB idle memory, 500 MB 20-tab memory, and 50 ms UI stall max, all with `10 %` regression tolerance.
- **`tests/budgets.ps1`:** validates budget schemas, measures LZX cabinet size, median cold start, median first-page render latency, and median idle working-set memory on `tests/corpus/zlib-manual.pdf`, failing on > 10 % regression against recorded baselines.

## Acceptance tests

- [x] `tests/budgets.ps1`: measures core installer size, cold start, first-page render, and idle memory against `tests/size/budgets.json` and `tests/perf/budgets.json` (`.github/workflows/windows.yml` `budgets` job).
- [x] `tests/perf/README.md`: documents trace privacy contract and idle energy rules.
