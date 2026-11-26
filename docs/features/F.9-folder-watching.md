# Ph5: 35/45 | F.9: Folder watching

**Tag:** F.9; **Priority:** P1; **Phase:** 5; **Needs:** F.6; **Code:** `excellent-features::library::watch`, `excellent-store::library`
**Plan:** §27 entry `#### Ph5: 35/45 | F.9`.

## Problem

Manually re-importing PDFs saved into download or reference folders is tedious and leaves the library out of sync with disk.

## User story

As a reader, I want ExcellentPDF to watch chosen folders and automatically add, update, and remove library entries as files change on disk, so that my library stays current.

## UI sketch

- Library settings panel listing watched folders with recursive toggle, "Add folder…", and "Scan now".

## Mode

All modes.

## Speed and size impact

- Startup: background watcher starts only if watched folders exist.
- Budgets touched: incremental `(file_size, modified_at)` diff skips unchanged files without re-reading PDFs.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::library::watch::{diff_folder, reconcile_folder, reconcile_all, FolderWatcher, WatchEvent, ReconcileReport}`.
- Storage: `watched_folder` table in `0008_library_fts_notes.sql`.

## Acceptance tests

- [x] `incremental_reconcile_detects_added_modified_and_removed_files`: Verifies added, unchanged-skip, modified, and removed document reconciliation against `Store`.
