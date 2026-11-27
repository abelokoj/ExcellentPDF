# Ph1: 10/60 | K.20: Library database

**Tag:** K.20; **Priority:** P0; **Phase:** 1; **Needs:** K.1; **Code:** `crates/ecclesia-store` (`store`,
`migrations`, `workspace`), `ecclesia-app::library`, `ecclesia-app::settings::folder`
**Plan:** §27 entry `Ph1: 10/60 | K.20`, §11.16 (workspace storage), decision D7.

## Problem

The app kept nothing between runs except `settings.toml`. Sessions (R.9), tab groups (R.13), workspaces, bookmarks,
cards and notes all need a store that survives crashes and that more than one thread can read.

## User story

As a reader, I want what I had open, and later my bookmarks, notes and collections, kept safely between runs, so
that quitting the app or a crash never costs me my work.

## UI sketch

None: the database has no UI of its own. It lives next to `settings.toml` as `library.db` (in the app's data folder,
or the program's folder for a portable copy). If it can't be opened the app runs as before, without restoring tabs,
and logs a warning.

## Mode

All modes; invisible.

## Speed and size impact

- Startup: the database is opened and migrated on the UI thread when the app starts (one small file, a few queries).
  Not measured (K.24).
- Writes go to one writer thread, so the UI thread never waits on the disk; reads open a short-lived read-only
  connection (WAL lets them run beside the writer).
- Installer: SQLite is bundled (`rusqlite`'s `bundled` feature), about 1 MB of code.

## Design

- `Store::open(path)`: makes the folder, opens the file in WAL mode (checked; `Error::NoWal` otherwise) with
  `synchronous = NORMAL`, foreign keys on and a 5 s busy timeout, migrates it, and starts the writer thread.
- `Store::write(|tx| …)`: queues a job that runs in its own transaction on the writer thread and returns a `Pending`
  for its result; a failed job rolls back. `flush()` waits for every queued job. `read(|conn| …)` reads on the
  caller's thread.
- Migrations (`migrations.rs`): numbered SQL files run in order, each in its own transaction that also sets
  `user_version`, so a database is always at exactly one version. A database written by a newer app is refused
  (`Error::TooNew`) and left untouched. `0001_workspaces.sql`: `file`, `workspace` (with the default one),
  `workspace_tab`, `closed_tab`. `0002_tab_groups.sql`: `tab_group` and `workspace_tab.tab_group`. All tables are
  `STRICT`.
- `workspace.rs`: `load_tabs` and `save_tabs` for a workspace's open tabs, groups and last 50 closed tabs; a tab's
  View state is the app's JSON, kept as text.
- Tables for documents, tags, collections, bookmarks (B.6), cards (S.4) and notes are added by those features' own
  migrations when they are built, so each schema is designed with the feature that uses it.

## Acceptance tests

From the entry's **Done when** ("migration tests pass; a crash during a write leaves the database consistent"):

- [x] `a_database_of_every_earlier_version_upgrades_to_the_new_schema`, `a_newer_database_is_left_alone`,
  `migrating_again_changes_nothing`: migrations.
- [x] `a_crash_during_a_write_leaves_the_database_consistent`: a child process aborts in the middle of a large
  write; the database passes `integrity_check` and holds the last committed tabs.
- [x] `writes_are_made_in_order_and_seen_by_reads_on_other_threads`, `a_failed_write_changes_nothing`,
  `writes_are_there_after_reopening`: the writer thread.
- [x] `tabs_come_back_as_they_were_saved`, `only_the_newest_closed_tabs_are_kept`,
  `a_version_1_session_keeps_its_tabs_through_the_upgrade`, `an_active_tab_past_the_end_is_none`: workspace tabs.
- [ ] FTS5 tables (full-text search) and the tables of tags, collections, bookmarks, cards and notes, with their features.
