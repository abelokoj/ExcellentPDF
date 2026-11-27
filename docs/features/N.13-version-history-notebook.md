# Ph5: 22/45 | N.13: Version history per notebook

**Tag:** N.13; **Priority:** P2; **Phase:** 5; **Needs:** K.14, N.4; **Code:** `ecclesia-features::notebook::versions`
**Plan:** §27 entry `#### Ph5: 22/45 | N.13: Version history per notebook`.

## Problem

Long-lived `.expdf` study notebooks accumulate weeks of edits; accidentally deleting or restructuring pages needs a self-contained way to inspect, diff, copy a page from, or restore an earlier version.

## User story

As a notebook user, I want version snapshots kept inside my `.expdf` file with age-based thinning so I can diff versions, copy an old page, or restore yesterday's version—and undo the restore if needed.

## UI sketch

- File > **Notebook Version History…** (`notebook.version_history`) lists `SnapshotMeta` entries with timestamps, operation summaries, page count deltas (`VersionDiff`), **Copy Page**, and **Restore Version**.

## Mode

All modes for `.expdf` notebooks.

## Speed and size impact

- Startup: zero cost; snapshots live under `assets/versions/` inside `.expdf` and load on demand.
- Storage: `thin_snapshots` keeps all snapshots < 24 h, 1/day < 30 d, and 1/week older.

## Design

- **Model (`ecclesia-features::notebook::versions`):**
  - `create_snapshot`, `list_snapshots`, `diff_snapshots`, `copy_page_from_snapshot`, `thin_snapshots`, and `restore_snapshot` returning `RestoreUndo`.

## Acceptance tests

- [x] `notebook::versions::tests::restore_yesterdays_version_works_and_is_undoable`: verifies restoring a prior snapshot and undoing the restore back to the current version.
- [x] `notebook::versions::tests::thins_old_snapshots_by_age_policy`: verifies 24h/30d/weekly age thinning.
