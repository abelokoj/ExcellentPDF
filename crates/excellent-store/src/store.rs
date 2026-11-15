//! The database's connection and its writer thread (plan K.20, step 3): WAL mode, so readers on any thread never
//! wait for the writer; one thread makes every change, each as one transaction, in the order they were asked for.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction};

use crate::{Error, Result, migrations};

/// How long a connection waits for a lock before it gives up (only checkpoints and a second writer take one).
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A change for the writer thread to make.
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// The library database: one writer thread, reads from any thread.
///
/// Dropping it lets the writer finish what was asked and stop; [`Store::flush`] waits until it has.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    jobs: mpsc::Sender<Job>,
}

impl Store {
    /// Opens the database at `path`, making it (and its folder) when it isn't there, brings its schema up to date
    /// and starts the writer thread.
    ///
    /// # Errors
    ///
    /// The folder or the file can't be made or opened, a migration failed, or a newer app wrote the database.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut conn = Connection::open(path)?;
        // WAL survives a crash at any point: a transaction is in the database whole or not at all. NORMAL syncs at
        // checkpoints rather than every commit, which in WAL mode can lose the last commits on a power cut but
        // never leaves the file inconsistent.
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::NoWal(mode));
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        configure(&conn)?;
        migrations::migrate(&mut conn)?;
        let (jobs, queue) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("excellent-store writer".to_owned())
            .spawn(move || {
                for job in queue {
                    job(&mut conn);
                }
            })?;
        Ok(Self {
            path: path.to_owned(),
            jobs,
        })
    }

    /// Makes a change on the writer thread: `f` runs in one transaction, committed when it returns `Ok` and rolled
    /// back when it returns `Err`. It returns at once; the [`Pending`] gives the result to a caller that waits for
    /// it, and may be dropped by one that doesn't.
    pub fn write<T, F>(&self, f: F) -> Pending<T>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T> + Send + 'static,
    {
        let (reply, answer) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let result = conn.transaction().map_err(Error::from).and_then(|tx| {
                let value = f(&tx)?;
                tx.commit()?;
                Ok(value)
            });
            // The caller may not be waiting.
            let _ = reply.send(result);
        });
        // A failed send drops the job and its reply, so the caller's wait reports the writer gone.
        let _ = self.jobs.send(job);
        Pending(answer)
    }

    /// Waits until every change asked for so far is in the database.
    ///
    /// # Errors
    ///
    /// The writer thread has stopped.
    pub fn flush(&self) -> Result<()> {
        self.write(|_| Ok(())).wait()
    }

    /// Reads with a connection of the caller's own, on the caller's thread. It sees every change committed before
    /// it started, and none of a change the writer is making meanwhile.
    ///
    /// shortcut: a new connection for each read (well under a millisecond); keep one per thread if reads get
    /// frequent.
    ///
    /// # Errors
    ///
    /// The database can't be opened, or `f` failed.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        configure(&conn)?;
        f(&conn)
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What every connection sets: foreign keys are checked, and a lock is waited for.
fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

/// The result of a [`Store::write`], once the writer has made it.
#[must_use = "dropping it is fine, but then nothing reports whether the write failed"]
#[derive(Debug)]
pub struct Pending<T>(mpsc::Receiver<Result<T>>);

impl<T> Pending<T> {
    /// Waits for the write and gives its result.
    ///
    /// # Errors
    ///
    /// The write failed (and changed nothing), or the writer thread has stopped.
    pub fn wait(self) -> Result<T> {
        self.0.recv().unwrap_or(Err(Error::WriterGone))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{self, Panes, SavedTab, Tabs, WorkspaceId};
    use std::process::{Command, Stdio};

    /// A database file of the test's own, with no WAL or index left from an earlier run.
    fn temp_db(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("excellent-store-{}-{name}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.clone().into_os_string();
            file.push(suffix);
            let _ = std::fs::remove_file(file);
        }
        path
    }

    fn count(store: &Store) -> usize {
        store
            .read(|conn| Ok(conn.query_row("SELECT count(*) FROM file", [], |row| row.get(0))?))
            .expect("count")
    }

    #[test]
    fn writes_are_made_in_order_and_seen_by_reads_on_other_threads() {
        let store = Store::open(&temp_db("order")).expect("open");
        for n in 0..10 {
            let _ = store.write(move |tx| {
                tx.execute("INSERT INTO file (path) VALUES (?1)", [format!("{n}")])?;
                Ok(())
            });
        }
        store.flush().expect("flush");
        let seen = thread::scope(|scope| {
            scope
                .spawn(|| {
                    store.read(|conn| {
                        let mut statement = conn.prepare("SELECT path FROM file ORDER BY id")?;
                        let paths = statement
                            .query_map([], |row| row.get::<_, String>(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        Ok(paths)
                    })
                })
                .join()
                .expect("reader")
        })
        .expect("read");
        let expected: Vec<String> = (0..10).map(|n| n.to_string()).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let store = Store::open(&temp_db("failed")).expect("open");
        let result = store
            .write(|tx| {
                tx.execute("INSERT INTO file (path) VALUES ('a')", [])?;
                // The second row breaks `path`'s uniqueness: the first must go too.
                tx.execute("INSERT INTO file (path) VALUES ('a')", [])?;
                Ok(())
            })
            .wait();
        assert!(matches!(result, Err(Error::Sqlite(_))));
        assert_eq!(count(&store), 0);
    }

    /// The environment variable that tells [`crash_child`] which database to crash on.
    const CRASH_DB: &str = "EXCELLENT_STORE_CRASH_DB";
    /// Enough rows (about 2.6 MB, past the 2 MB page cache) that SQLite spills the unfinished transaction into the WAL before the crash.
    const ROWS: usize = 6_000;

    fn generation(n: usize) -> Panes {
        let tabs = Tabs {
            open: (0..ROWS)
                .map(|row| SavedTab {
                    path: PathBuf::from(format!("C:/papers/{row}.pdf")),
                    view: format!("{{\"generation\":{n},\"padding\":\"{}\"}}", "x".repeat(400)),
                    pinned: false,
                    group: None,
                })
                .collect(),
            active: Some(n),
            ..Tabs::default()
        };
        Panes {
            layout: Some(format!("{{\"generation\":{n}}}")),
            panes: vec![tabs],
        }
    }

    /// Run by [`a_crash_during_a_write_leaves_the_database_consistent`] in a process of its own: five whole
    /// writes, then a crash halfway through the sixth. Without the variable it does nothing.
    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os(CRASH_DB) else {
            return;
        };
        let store = Store::open(Path::new(&path)).expect("open");
        for n in 0..5 {
            store
                .write(move |tx| workspace::save_panes(tx, WorkspaceId::DEFAULT, &generation(n)))
                .wait()
                .expect("write");
        }
        let _ = store
            .write(|tx| -> Result<()> {
                workspace::save_panes(tx, WorkspaceId::DEFAULT, &generation(5))?;
                std::process::abort()
            })
            .wait();
        unreachable!("the writer aborts the process");
    }

    #[test]
    fn a_crash_during_a_write_leaves_the_database_consistent() {
        let path = temp_db("crash");
        let status = Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", "store::tests::crash_child", "--nocapture"])
            .env(CRASH_DB, &path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run the child");
        assert!(!status.success(), "the child must have crashed");
        let store = Store::open(&path).expect("reopen");
        store
            .read(|conn| {
                let check: String =
                    conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
                assert_eq!(check, "ok");
                assert_eq!(
                    workspace::load_panes(conn, WorkspaceId::DEFAULT)?,
                    generation(4)
                );
                Ok(())
            })
            .expect("read");
    }

    #[test]
    fn writes_are_there_after_reopening() {
        let path = temp_db("drop");
        {
            let store = Store::open(&path).expect("open");
            let _ = store.write(|tx| {
                tx.execute("INSERT INTO file (path) VALUES ('kept')", [])?;
                Ok(())
            });
            store.flush().expect("flush");
        }
        assert_eq!(count(&Store::open(&path).expect("reopen")), 1);
    }
}
