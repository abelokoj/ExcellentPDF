//! The schema's versions (plan K.20, step 2). Each migration is a SQL batch run in a transaction of its own, which
//! also sets SQLite's `user_version` to its number, so a database is always at exactly one version. Migrations are
//! only ever added at the end; a shipped one never changes.

use rusqlite::Connection;

use crate::{Error, Result};

/// Every migration, oldest first: number `n` is `MIGRATIONS[n - 1]`.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_workspaces.sql"),
    include_str!("migrations/0002_tab_groups.sql"),
    include_str!("migrations/0003_recent.sql"),
    include_str!("migrations/0004_bookmarks.sql"),
    include_str!("migrations/0005_panes.sql"),
    include_str!("migrations/0006_annotation_tags.sql"),
    include_str!("migrations/0007_annotation_filters.sql"),
    include_str!("migrations/0008_library_fts_notes.sql"),
    include_str!("migrations/0009_recent_progress.sql"),
];

/// The newest version this build knows.
pub(crate) fn latest() -> u32 {
    u32::try_from(MIGRATIONS.len()).expect("fewer than 2^32 migrations")
}

/// The database's version.
pub(crate) fn version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Brings the database to the newest version.
///
/// # Errors
///
/// A migration failed (its transaction is rolled back, so the database stays at the version before it), or a newer
/// app wrote the database.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_to(conn, latest())
}

/// Brings the database up to `target`, which is at most [`latest`].
pub(crate) fn migrate_to(conn: &mut Connection, target: u32) -> Result<()> {
    let current = version(conn)?;
    if current > latest() {
        return Err(Error::TooNew {
            version: current,
            known: latest(),
        });
    }
    for (number, sql) in (1..=target).zip(MIGRATIONS).skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", number)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every table, index and trigger, as SQLite keeps their definitions.
    fn schema(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
            .expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows")
    }

    #[test]
    fn a_database_of_every_earlier_version_upgrades_to_the_new_schema() {
        let mut fresh = Connection::open_in_memory().expect("open");
        migrate(&mut fresh).expect("fresh");
        assert_eq!(version(&fresh).expect("version"), latest());
        for from in 0..latest() {
            let mut old = Connection::open_in_memory().expect("open");
            migrate_to(&mut old, from).expect("old version");
            assert_eq!(version(&old).expect("version"), from);
            migrate(&mut old).expect("upgrade");
            assert_eq!(version(&old).expect("version"), latest(), "from {from}");
            assert_eq!(schema(&old), schema(&fresh), "from {from}");
        }
    }

    #[test]
    fn a_newer_database_is_left_alone() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "user_version", latest() + 1)
            .expect("version");
        assert!(matches!(migrate(&mut conn), Err(Error::TooNew { .. })));
        assert!(schema(&conn).is_empty());
    }

    #[test]
    fn migrating_again_changes_nothing() {
        let mut conn = Connection::open_in_memory().expect("open");
        migrate(&mut conn).expect("first");
        let before = schema(&conn);
        migrate(&mut conn).expect("second");
        assert_eq!(schema(&conn), before);
    }
}
