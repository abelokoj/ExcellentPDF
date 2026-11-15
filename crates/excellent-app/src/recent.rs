//! Recent and pinned files (plan R.10): every file shown, with the page it was left at, kept in the library (K.20).
//! The empty document panel lists the pinned files and the latest ones, each with "Pin" and "Remove", and "Clear";
//! the tab list (Ctrl+Shift+A) searches all of them.
//!
//! The window keeps the list in a signal and changes it at once, and the library's writer saves each change behind
//! it, so a click never waits on the disk. The list is read from the library the first time a view shows it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use excellent_store::recent::{self, RECENT_KEPT, RecentFile};
use nsh_base::{button, theme, toggle};
use nsh_core::element::desktop::on_app_quit;
use nsh_core::prelude::*;

use crate::tabs::{TabState, pin_mark};

/// How long the shown page must stay put before it is saved: the reader is turning pages meanwhile.
const SAVE_DELAY: Duration = Duration::from_secs(1);

/// How many files of each kind the empty panel lists; the tab list has the rest.
const PANEL_ROWS: usize = 8;

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

    /// Notes that `path` is shown at `page`.
    fn visit(self, path: PathBuf, page: usize) {
        let seen = now();
        self.change(|files| visit(files, &path, page, seen));
        save(move |tx| recent::visit(tx, &path, page, seen));
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
    let latest = stored(None::<(PathBuf, usize)>);
    effect(move || {
        let Some((path, page)) = tabs.shown_place() else {
            return;
        };
        latest.replace(Some((path.clone(), page)));
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        pending.replace(Some(set_timeout(SAVE_DELAY, move || {
            latest.replace(None);
            state.visit(path, page);
        })));
    });
    on_app_quit(move || {
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        if let Some((path, page)) = latest.replace(None) {
            let seen = now();
            save(move |tx| recent::visit(tx, &path, page, seen));
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

/// `files` with `path` shown at `seen` and left at `page`: the library's [`recent::visit`], in the window.
fn visit(files: &mut Vec<RecentFile>, path: &Path, page: usize, seen: i64) {
    let pinned = files.iter().any(|file| file.path == path && file.pinned);
    files.retain(|file| file.path != path);
    files.push(RecentFile {
        path: path.to_path_buf(),
        page,
        seen,
        pinned,
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

/// The empty document panel's lists: "Pinned" and "Recent", each a few rows; nothing while both are empty.
pub(crate) fn recent_panel(state: RecentState, tabs: TabState) -> impl IntoElement {
    state.load();
    let section = move |pinned: bool| {
        move || {
            state
                .files()
                .into_iter()
                .filter(|file| file.pinned == pinned)
                .take(PANEL_ROWS)
                .collect::<Vec<_>>()
        }
    };
    let has = move |pinned: bool| {
        move || {
            state
                .files
                .with(|files| files.iter().flatten().any(|file| file.pinned == pinned))
        }
    };
    view()
        .w_full()
        .max_w(px(560.0))
        .gap_4()
        .pt_4()
        .child(when(has(true), move || {
            view()
                .gap_1()
                .child(heading("recent.pinned", None))
                .child(rows(state, tabs, section(true)))
        }))
        .child(when(has(false), move || {
            view()
                .gap_1()
                .child(heading("recent.title", Some(state)))
                .child(rows(state, tabs, section(false)))
        }))
}

/// A list's heading; the recent list's has "Clear" at its end.
fn heading(key: &'static str, clear: Option<RecentState>) -> impl IntoElement {
    let row = view().flex_row().items_center().h(px(28.0)).px_3().child(
        text(move || t!(key))
            .flex_1()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().muted_foreground),
    );
    match clear {
        Some(state) => row.child(
            button()
                .accessibility_label(t!("recent.clear_label"))
                .on_press(move || state.clear())
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .active(|s| s.opacity(0.7))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    text(move || t!("recent.clear"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        ),
        None => row,
    }
}

/// The rows of one list.
fn rows(
    state: RecentState,
    tabs: TabState,
    files: impl Fn() -> Vec<RecentFile> + 'static,
) -> impl IntoElement {
    view().gap(px(2.0)).child(each(
        files,
        |file: &RecentFile| file.path.clone(),
        move |file, _| file_row(state, tabs, file),
    ))
}

/// One file: its name, folder and page (pressing it opens the file there), then "Pin" and "Remove". A file that
/// isn't there any more is greyed; opening it offers "Locate…" (R.9).
fn file_row(state: RecentState, tabs: TabState, file: ReadSignal<RecentFile>) -> impl IntoElement {
    let path = file.with_untracked(|file| file.path.clone());
    let missing = !path.is_file();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let folder = path
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let pinned = move || file.with(|file| file.pinned);
    let page = move || file.with(|file| file.page) + 1;
    let open = {
        let (path, label_name) = (path.clone(), name.clone());
        button()
            .accessibility_label(move || {
                Some(SharedString::from(t!(
                    "recent.open",
                    name = label_name,
                    page = page()
                )))
            })
            .on_press(move || tabs.open_at(path.clone(), file.with(|file| file.page)))
            .flex_1()
            .min_w(px(0.0))
            .h(px(40.0))
            .px_3()
            .gap_3()
            .flex_row()
            .items_center()
            .rounded(px(8.0))
            .hover(|s| s.bg(move || theme().muted))
            .active(|s| s.opacity(0.7))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(
                text(name.clone())
                    .flex_shrink(0.0)
                    .max_w(px(240.0))
                    .truncate()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if missing {
                            theme().muted_foreground
                        } else {
                            theme().foreground
                        }
                    }),
            )
            .child(
                text(folder)
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                text(move || {
                    if missing {
                        t!("tabs.missing")
                    } else {
                        t!("recent.page", page = page())
                    }
                })
                .flex_shrink(0.0)
                .tabular_nums()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
            )
    };
    let pin_button = {
        let (path, name) = (path.clone(), name.clone());
        toggle()
            .pressed(pinned)
            .accessibility_label(t!("recent.pin", name = name))
            .on_change(move |pressed: &bool| state.set_pinned(path.clone(), *pressed))
            .size(px(32.0))
            .rounded(px(6.0))
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().muted))
            .pressed_style(|s| s.bg(move || theme().muted))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(pin_mark(move || theme().muted_foreground))
    };
    let remove_button = button()
        .accessibility_label(t!("recent.remove", name = name))
        .on_press(move || state.remove(path.clone()))
        .size(px(32.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text("×")
                .text_size(px(16.0))
                .text_color(move || theme().muted_foreground),
        );
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(open)
        .child(pin_button)
        .child(remove_button)
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
        visit(&mut files, Path::new("a.pdf"), 9, 10);
        assert_eq!(names(&files), "p a b");
        assert_eq!(files[1].page, 9);
        // A pinned file shown again stays pinned.
        visit(&mut files, Path::new("p.pdf"), 2, 11);
        assert!(files[0].pinned);
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
                i64::try_from(n).expect("small"),
            );
        }
        assert_eq!(files.len(), RECENT_KEPT);
        assert_eq!(files[0].path, PathBuf::from((RECENT_KEPT + 1).to_string()));
    }
}
