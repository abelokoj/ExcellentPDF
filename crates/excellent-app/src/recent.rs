//! Recent and pinned files (plan R.10): every file shown, with the page it was left at, kept in the library (K.20).
//! The home page (F.17) lists the pinned files and the latest ones, each with its cover, "Pin" and "Remove", and
//! "Clear", and its search box finds any of them; the tab list (Ctrl+Shift+A) searches all of them too.
//!
//! The window keeps the list in a signal and changes it at once, and the library's writer saves each change behind
//! it, so a click never waits on the disk. The list is read from the library the first time a view shows it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use excellent_store::recent::{self, RECENT_KEPT, RecentFile};
use nsh_core::element::desktop::on_app_quit;
use nsh_core::prelude::*;

use crate::tabs::TabState;

/// How long the shown page must stay put before it is saved: the reader is turning pages meanwhile.
const SAVE_DELAY: Duration = Duration::from_secs(1);

/// The window's recent list.
#[derive(Clone, Copy)]
pub(crate) struct RecentState {
    /// The list, once read.
    files: Signal<Option<Vec<RecentFile>>>,
}

impl std::fmt::Debug for RecentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecentState").finish_non_exhaustive()
    }
}

impl RecentState {
    /// The list, not read yet.
    pub(crate) fn new() -> Self {
        Self {
            files: signal(None),
        }
    }

    /// Reads the list from the library, the first time it is needed: a view calls it as it is made.
    pub(crate) fn load(self) {
        if untracked(|| self.files.with(Option::is_some)) {
            return;
        }
        let files = crate::library::store()
            .and_then(|store| match store.read(recent::load) {
                Ok(files) => Some(files),
                Err(err) => {
                    log::warn!("recent: can't read the list: {err}");
                    None
                }
            })
            .unwrap_or_default();
        self.files.set(Some(files));
    }

    /// The list: pinned files first, then the most recently shown.
    pub(crate) fn files(self) -> Vec<RecentFile> {
        self.files.get().unwrap_or_default()
    }

    /// Pins or unpins `path`.
    pub(crate) fn set_pinned(self, path: PathBuf, pinned: bool) {
        self.change(|files| pin(files, &path, pinned));
        save(move |tx| recent::set_pinned(tx, &path, pinned));
    }

    /// Marks `path` completed or active on the home library.
    pub(crate) fn set_completed(self, path: PathBuf, completed: bool) {
        self.change(|files| {
            for file in files.iter_mut().filter(|file| file.path == path) {
                file.completed = completed;
            }
        });
        save(move |tx| recent::set_completed(tx, &path, completed));
    }

    /// Takes `path` off the list.
    pub(crate) fn remove(self, path: PathBuf) {
        self.change(|files| files.retain(|file| file.path != path));
        save(move |tx| recent::remove(tx, &path));
    }

    /// Takes every unpinned file off the list.
    pub(crate) fn clear(self) {
        self.change(|files| files.retain(|file| file.pinned));
        save(recent::clear);
    }

    /// Notes that `path` is shown at `page` of `total_pages`.
    fn visit(self, path: PathBuf, page: usize, total_pages: usize) {
        let seen = now();
        self.change(|files| visit(files, &path, page, total_pages, seen));
        save(move |tx| recent::visit_with_pages(tx, &path, page, total_pages, seen));
    }

    /// Changes the list in the window, if it has been read (otherwise the library has the change when it is).
    fn change(self, f: impl FnOnce(&mut Vec<RecentFile>)) {
        self.files.update(|files| {
            if let Some(files) = files {
                f(files);
            }
        });
    }
}

/// Keeps the list as tabs are shown: the shown file and its page are noted a second after they last change, and as
/// the app quits.
pub(crate) fn keep(state: RecentState, tabs: TabState) {
    let pending = stored(None::<Spawned<()>>);
    let latest = stored(None::<(PathBuf, usize, usize)>);
    effect(move || {
        let Some((path, page, total)) = tabs.shown_place_with_total() else {
            return;
        };
        latest.replace(Some((path.clone(), page, total)));
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        pending.replace(Some(set_timeout(SAVE_DELAY, move || {
            latest.replace(None);
            state.visit(path, page, total);
        })));
    });
    on_app_quit(move || {
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        if let Some((path, page, total)) = latest.replace(None) {
            let seen = now();
            save(move |tx| recent::visit_with_pages(tx, &path, page, total, seen));
        }
        if let Some(Err(err)) = crate::library::store().map(excellent_store::Store::flush) {
            log::warn!("recent: the list wasn't saved: {err}");
        }
        async {}
    });
}

/// Hands a change to the library's writer.
fn save(
    change: impl FnOnce(&excellent_store::rusqlite::Transaction<'_>) -> excellent_store::Result<()>
    + Send
    + 'static,
) {
    let Some(store) = crate::library::store() else {
        return;
    };
    let _ = store.write(move |tx| {
        let result = change(tx);
        if let Err(err) = &result {
            log::warn!("recent: can't save the list: {err}");
        }
        result
    });
}

/// Seconds since 1970.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// `files` with `path` shown at `seen` and left at `page` of `total_pages`: the library's
/// [`recent::visit_with_pages`], in the window.
fn visit(files: &mut Vec<RecentFile>, path: &Path, page: usize, total_pages: usize, seen: i64) {
    let existing = files.iter().find(|file| file.path == path);
    let pinned = existing.is_some_and(|file| file.pinned);
    let prev_total = existing.map_or(0, |file| file.total_pages);
    let open_count = existing.map_or(1, |file| file.open_count.saturating_add(1));
    let completed = existing.is_some_and(|file| file.completed);
    let total_pages = if total_pages > 0 {
        total_pages
    } else {
        prev_total
    };
    files.retain(|file| file.path != path);
    files.push(RecentFile {
        path: path.to_path_buf(),
        page,
        seen,
        pinned,
        total_pages,
        open_count,
        completed,
    });
    sort(files);
    let mut unpinned = 0;
    files.retain(|file| {
        unpinned += usize::from(!file.pinned);
        file.pinned || unpinned <= RECENT_KEPT
    });
}

/// `files` with `path` pinned or not.
fn pin(files: &mut [RecentFile], path: &Path, pinned: bool) {
    for file in files.iter_mut().filter(|file| file.path == path) {
        file.pinned = pinned;
    }
    sort(files);
}

/// The library's order: pinned first, then the newest.
fn sort(files: &mut [RecentFile]) {
    files.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.seen.cmp(&a.seen)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, seen: i64, pinned: bool) -> RecentFile {
        RecentFile {
            path: PathBuf::from(format!("{name}.pdf")),
            page: 0,
            seen,
            pinned,
            total_pages: 0,
            open_count: 1,
            completed: false,
        }
    }

    fn names(files: &[RecentFile]) -> String {
        files
            .iter()
            .map(|file| {
                file.path
                    .to_string_lossy()
                    .trim_end_matches(".pdf")
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_window_keeps_the_list_in_the_library_order() {
        let mut files = vec![file("p", 1, true), file("b", 5, false), file("a", 3, false)];
        visit(&mut files, Path::new("a.pdf"), 9, 0, 10);
        assert_eq!(names(&files), "p a b");
        assert_eq!(files[1].page, 9);
        // A pinned file shown again stays pinned.
        visit(&mut files, Path::new("p.pdf"), 2, 120, 11);
        assert!(files[0].pinned);
        assert_eq!(files[0].total_pages, 120);
        pin(&mut files, Path::new("b.pdf"), true);
        assert_eq!(names(&files), "p b a");
        pin(&mut files, Path::new("p.pdf"), false);
        assert_eq!(names(&files), "b p a");
    }

    #[test]
    fn the_window_keeps_as_many_files_as_the_library() {
        let mut files = Vec::new();
        for n in 0..RECENT_KEPT + 2 {
            visit(
                &mut files,
                Path::new(&n.to_string()),
                0,
                0,
                i64::try_from(n).expect("small"),
            );
        }
        assert_eq!(files.len(), RECENT_KEPT);
        assert_eq!(files[0].path, PathBuf::from((RECENT_KEPT + 1).to_string()));
    }
}
