//! Folder watching and incremental scan reconciliation (plan F.9).
//!
//! Tracks disk snapshots `(path -> (size, modified_at))` against `library_item` rows in `library.db`,
//! adding new files, updating changed ones, and pruning removed entries.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use excellent_store::library::{self, LibraryFilter, WatchDepth};
use excellent_store::{Result as StoreResult, Store};

use super::metadata;

/// A change detected inside a watched folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A new document was discovered and added to the library.
    Added(PathBuf),
    /// An existing document changed size or modification time and was refreshed.
    Modified(PathBuf),
    /// A previously tracked document disappeared from disk and was removed from the library.
    Removed(PathBuf),
}

/// Diff between the current disk contents of a folder and what is stored in `library.db`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderDiff {
    /// Newly discovered files.
    pub added: Vec<PathBuf>,
    /// Files whose size or modification time changed.
    pub modified: Vec<PathBuf>,
    /// Files previously recorded under this folder that are no longer on disk.
    pub removed: Vec<PathBuf>,
    /// Files unchanged since last scan.
    pub unchanged: usize,
}

/// Summary of reconciling one or more watched folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Number of newly added library entries.
    pub added: usize,
    /// Number of updated library entries.
    pub modified: usize,
    /// Number of removed library entries.
    pub removed: usize,
    /// Number of unchanged files skipped.
    pub unchanged: usize,
    /// Detailed events emitted in order.
    pub events: Vec<WatchEvent>,
}

/// Computes the incremental [`FolderDiff`] between `folder` on disk and `library_item` rows in `store`.
///
/// # Errors
///
/// Returns [`excellent_store::Error`] on filesystem or SQLite failure.
pub fn diff_folder(store: &Store, folder: &Path, depth: WatchDepth) -> StoreResult<FolderDiff> {
    let mut disk = BTreeMap::new();
    scan_disk_tree(folder, depth, &mut disk).map_err(excellent_store::Error::Io)?;

    let prefix = normalized_prefix(folder);
    let existing = store.read(|conn| {
        let items = library::list_items(conn, &LibraryFilter::default())?;
        Ok(items
            .into_iter()
            .filter(|item| normalized_prefix(&item.path).starts_with(&prefix))
            .map(|item| (item.path, (item.file_size, item.modified_at)))
            .collect::<BTreeMap<_, _>>())
    })?;

    let mut diff = FolderDiff::default();
    let mut seen = BTreeSet::new();
    for (path, (size, mtime)) in &disk {
        seen.insert(path.clone());
        match existing.get(path) {
            None => diff.added.push(path.clone()),
            Some(&(prev_size, prev_mtime)) if prev_size != *size || prev_mtime != *mtime => {
                diff.modified.push(path.clone());
            }
            Some(_) => diff.unchanged += 1,
        }
    }
    for path in existing.keys() {
        if !seen.contains(path) {
            diff.removed.push(path.clone());
        }
    }
    Ok(diff)
}

/// Reconciles `folder` with `store`, adding new files, refreshing modified ones, and pruning removed ones.
///
/// # Errors
///
/// Returns [`excellent_store::Error`] on filesystem or SQLite failure.
pub fn reconcile_folder(
    store: &Store,
    folder: &Path,
    depth: WatchDepth,
) -> StoreResult<ReconcileReport> {
    let diff = diff_folder(store, folder, depth)?;
    let now = now_secs();
    let mut report = ReconcileReport {
        unchanged: diff.unchanged,
        ..ReconcileReport::default()
    };

    for path in diff.added {
        if let Ok(item) = metadata::build_library_item(&path, now) {
            let event_path = path.clone();
            store
                .write(move |tx| library::upsert_item(tx, &item))
                .wait()?;
            report.added += 1;
            report.events.push(WatchEvent::Added(event_path));
        }
    }

    for path in diff.modified {
        if let Ok(item) = metadata::build_library_item(&path, now) {
            let event_path = path.clone();
            store
                .write(move |tx| library::upsert_item(tx, &item))
                .wait()?;
            report.modified += 1;
            report.events.push(WatchEvent::Modified(event_path));
        }
    }

    for path in diff.removed {
        let event_path = path.clone();
        store
            .write(move |tx| {
                let _ = library::delete_item(tx, &path)?;
                Ok(())
            })
            .wait()?;
        report.removed += 1;
        report.events.push(WatchEvent::Removed(event_path));
    }

    let watched_path = folder.to_path_buf();
    store
        .write(move |tx| library::mark_folder_scanned(tx, &watched_path, now))
        .wait()?;

    Ok(report)
}

/// Reconciles every watched folder registered in `store`.
///
/// # Errors
///
/// SQLite or filesystem failed.
pub fn reconcile_all(store: &Store) -> StoreResult<ReconcileReport> {
    let folders = store.read(library::list_watched_folders)?;
    let mut total = ReconcileReport::default();
    for folder in folders {
        if !folder.path.exists() {
            continue;
        }
        let part = reconcile_folder(store, &folder.path, folder.depth)?;
        total.added += part.added;
        total.modified += part.modified;
        total.removed += part.removed;
        total.unchanged += part.unchanged;
        total.events.extend(part.events);
    }
    Ok(total)
}

/// Background polling watcher that reconciles registered folders on an interval and emits [`WatchEvent`]s.
/// Stops cleanly when dropped.
#[derive(Debug)]
pub struct FolderWatcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FolderWatcher {
    /// Spawns a background watcher thread polling `db_path` at `interval`.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the database cannot be opened or the thread fails to spawn.
    pub fn start<F>(db_path: PathBuf, interval: Duration, mut on_event: F) -> io::Result<Self>
    where
        F: FnMut(WatchEvent) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("excellent-folder-watcher".to_owned())
            .spawn(move || {
                let Ok(store) = Store::open(&db_path) else {
                    return;
                };
                while !stop_flag.load(Ordering::Acquire) {
                    if let Ok(report) = reconcile_all(&store) {
                        for event in report.events {
                            on_event(event);
                        }
                    }
                    let mut slept = Duration::ZERO;
                    let step = Duration::from_millis(50);
                    while slept < interval && !stop_flag.load(Ordering::Acquire) {
                        thread::sleep(step.min(interval.saturating_sub(slept)));
                        slept += step;
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Signals the watcher thread to stop and waits for it to exit.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn scan_disk_tree(
    dir: &Path,
    depth: WatchDepth,
    out: &mut BTreeMap<PathBuf, (u64, i64)>,
) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if depth.is_recursive() {
                scan_disk_tree(&path, depth, out)?;
            }
        } else if is_library_document(&path)
            && let Ok(meta) = entry.metadata()
        {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
            out.insert(path, (meta.len(), mtime));
        }
    }
    Ok(())
}

fn is_library_document(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "pdf" | "xps" | "epub" | "cbz" | "fb2" | "mobi"
            )
        })
}

fn normalized_prefix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("excellent-watch-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn incremental_reconcile_detects_added_modified_and_removed_files() {
        let dir = temp_dir("reconcile");
        let watched = dir.join("watched");
        fs::create_dir_all(&watched).expect("mkdir watched");
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let target = watched.join("paper.pdf");
        fs::copy(&corpus, &target).expect("copy");

        let store = Store::open(&dir.join("library.db")).expect("open");
        store
            .write({
                let w = watched.clone();
                move |tx| library::add_watched_folder(tx, &w, WatchDepth::Recursive).map(|_| ())
            })
            .wait()
            .expect("watch");

        let r1 = reconcile_all(&store).expect("reconcile 1");
        assert_eq!(r1.added, 1);
        assert_eq!(r1.events, [WatchEvent::Added(target.clone())]);

        let r2 = reconcile_all(&store).expect("reconcile 2");
        assert_eq!(r2.unchanged, 1);
        assert_eq!(r2.added, 0);

        // Append a PDF comment to alter file size -> detected as modified.
        let mut bytes = fs::read(&target).expect("read");
        bytes.extend_from_slice(b"\n% modified\n");
        fs::write(&target, &bytes).expect("write");
        let r3 = reconcile_all(&store).expect("reconcile 3");
        assert_eq!(r3.modified, 1);

        // Delete file -> detected as removed.
        fs::remove_file(&target).expect("remove");
        let r4 = reconcile_all(&store).expect("reconcile 4");
        assert_eq!(r4.removed, 1);
        assert_eq!(r4.events, [WatchEvent::Removed(target)]);

        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }
}
