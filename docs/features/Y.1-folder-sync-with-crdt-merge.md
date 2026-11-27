# Ph9: 9/13 | Y.1: Folder sync with CRDT merge

**Tag:** Y.1; **Priority:** P1; **Phase:** 9; **Needs:** K.14, K.28; **Code:** `ecclesia-features::sync`
**Plan:** §27 entry `#### Ph9: 9/13 | Y.1`.

## Problem

Cloud sync folders (Syncthing, Dropbox, OneDrive, iCloud) corrupt SQLite or JSON files whenever two offline devices write to the same file before syncing.

## User story

As a multi-device user, I want annotations, notes, and bookmarks to sync through any folder using per-device append-only CRDT operation logs with optional encryption so that offline edits always merge cleanly without accounts or conflicts.

## UI sketch

- Settings -> Sync folder picker, optional passphrase/key toggle, and conflict-cleanup button.

## Mode

All modes.

## Speed and size impact

- Startup: sync reads/appends `.ecclesia/<scope>/<device-id>.oplog` asynchronously.
- Budgets touched: zero cold-start blocking.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::sync::crdt::{HlcTimestamp, LwwRegister, NoteTextCrdt, SyncOp, SyncedDocumentState}` and `ecclesia-features::sync::folder::{FolderSync, PayloadSecurity}`.
- Each device writes strictly to its own `<device-id>.oplog`; conflicted copy artifacts are ignored and cleaned up.

## Acceptance tests

- [ ] `offline_devices_sync_and_converge_under_arbitrary_interleavings`: Verifies forward, reversed, and pseudo-randomly shuffled op interleavings converge to identical annotations and RGA note text, plus encrypted oplog + conflict cleanup.
