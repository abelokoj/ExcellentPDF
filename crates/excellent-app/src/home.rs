//! The home page (plan F.17): what an empty pane shows. Its heading and "Open a document"; a search box over the
//! recent files; "Continue reading" with the file shown last, at its page; the workspaces to switch to; then the
//! pinned and recent files (R.10), each with its cover.
//!
//! Covers are the files' first pages, rendered on the shared render workers below the pages on screen and kept on
//! disk by file, size and date, so the page paints at once and each cover appears when it is ready.
//!
//! shortcut: the cover folder is never trimmed; the library's housekeeping (K.20) bounds it, as the thumbnails'.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use excellent_core::document::Document;
use excellent_core::shared::pool;
use excellent_core::thumbnail;
use excellent_features::home::{COVER_WIDTH, continue_reading, cover_name};
use excellent_store::recent::RecentFile;
use excellent_store::workspaces::Workspace;
use nsh_base::{button, input, theme};
use nsh_core::color::white;
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::panes::Shared;
use crate::recent::{self, RecentState};
use crate::shell::primary_button;
use crate::study_ui::{HomeStudySummary, home_study_summary};
use crate::tabs::TabState;
use crate::workspaces::{Workspaces, color_dot, display_name};

/// The files' covers, for the window.
#[derive(Clone, Copy)]
pub(crate) struct Covers {
    /// Each rendered cover's file.
    ready: Signal<HashMap<PathBuf, SharedString>>,
    /// The files whose cover was asked for, rendered or not.
    asked: Stored<HashSet<PathBuf>>,
    /// The renders under way: they live as long as the window.
    running: Stored<Vec<Spawned<()>>>,
}

impl Covers {
    pub(crate) fn new() -> Self {
        Self {
            ready: signal(HashMap::new()),
            asked: stored(HashSet::new()),
            running: stored(Vec::new()),
        }
    }

    /// The cover of the file at `path` once rendered (tracked); the first call asks for it.
    pub(crate) fn of(self, path: &Path) -> Option<SharedString> {
        if self.asked.with(|asked| asked.insert(path.to_owned())) {
            untracked(|| self.start(path.to_owned()));
        }
        self.ready.with(|ready| ready.get(path).cloned())
    }

    /// Shows the cached cover of `path`, or renders it.
    fn start(self, path: PathBuf) {
        let Ok(meta) = fs::metadata(&path) else {
            return;
        };
        if !meta.is_file() {
            return;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_secs());
        let out = data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("covers")
            .join(cover_name(&path, meta.len(), modified));
        let ready = self.ready;
        let show = move |path: PathBuf, file: &Path| {
            let file = SharedString::from(file.to_string_lossy().into_owned());
            ready.update(|ready| {
                ready.insert(path, file);
            });
        };
        if out.is_file() {
            show(path, &out);
            return;
        }
        let task = spawn(async move {
            // Its own document, not the registry's: parsing it never holds up a tab opening meanwhile.
            let opened = blocking({
                let path = path.clone();
                move || {
                    let document = Arc::new(Document::open(&path)?);
                    let mupdf = document.mupdf()?;
                    Ok::<_, excellent_core::document::Error>((document, mupdf))
                }
            })
            .await;
            let (document, mupdf) = match opened {
                Ok(Ok(opened)) => opened,
                Ok(Err(err)) => {
                    log::info!("home: no cover for {}: {err}", path.display());
                    return;
                }
                Err(err) => {
                    log::warn!("home: opening {} failed: {err}", path.display());
                    return;
                }
            };
            match thumbnail::render(pool(), &document, mupdf, 0, COVER_WIDTH, out).await {
                Ok(Ok(file)) => show(path, &file),
                Ok(Err(err)) => {
                    log::warn!("home: cannot save the cover of {}: {err}", path.display())
                }
                Err(err) if !err.is_aborted() => {
                    log::warn!("home: cannot draw the cover of {}: {err}", path.display());
                }
                Err(_) => {}
            }
        });
        self.running.with(|running| running.push(task));
    }
}

/// A file's cover in a `width` × `height` frame: white paper until it is rendered, or the file's type on it when it
/// can't be.
pub(crate) fn cover(covers: Covers, path: PathBuf, width: f32, height: f32) -> impl IntoElement {
    let kind = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let path = stored(path);
    let file = move || path.with(|path| covers.of(path));
    view()
        .w(px(width))
        .h(px(height))
        .flex_shrink(0.0)
        .rounded(px(3.0))
        .bg(white())
        .border_1()
        .border_color(move || theme().border)
        .shadow_sm()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(
            when(
                move || file().is_some(),
                move || {
                    image(move || file().unwrap_or_default())
                        .object_fit(ResizeMode::Cover)
                        .size_full()
                },
            )
            .otherwise(move || {
                // Ink on the white paper, in either theme.
                text(kind.clone())
                    .text_size(px((width / 4.0).clamp(7.0, 12.0)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(Hsla::parse_hex("#8a8f98").expect("a colour"))
            }),
        )
}

/// The home page of a pane with no tab open.
pub(crate) fn home_page(
    shared: Shared,
    tabs: TabState,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let Shared {
        recent,
        covers,
        workspaces,
        ..
    } = shared;
    recent.load();
    let query = signal(String::new());
    let searching = move || !query.with(|q| q.trim().is_empty());
    let has_files = move || !recent.files().is_empty();
    let many_workspaces = move || workspaces.list().len() > 1;
    scroll().flex_1().child(
        view().w_full().items_center().px_6().py(px(40.0)).child(
            view()
                .w_full()
                .max_w(px(640.0))
                .gap_6()
                .child(header(run))
                .child(when(has_files, move || search_field(query)))
                .child(when(
                    move || !searching() && last_read(recent).is_some(),
                    move || continue_card(recent, covers, tabs),
                ))
                .child(when(move || !searching(), move || study_card(run)))
                .child(when(
                    move || !searching() && many_workspaces(),
                    move || workspace_row(workspaces),
                ))
                .child(recent::recent_panel(recent, tabs, covers, query)),
        ),
    )
}

/// The heading, the one line on how to start, and "Open a document".
fn header(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view()
                .flex_1()
                .gap_1()
                .child(
                    text(move || t!("home.title"))
                        .text_size(px(22.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || {
                        t!(
                            "document.empty.hint",
                            open = commands::shortcut_label(CommandId::OpenFile),
                            keys = commands::shortcut_label(CommandId::OpenPalette)
                        )
                    })
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(primary_button("document.open", move || {
            run(CommandId::OpenFile);
        }))
}

/// The search box over the recent files.
fn search_field(query: Signal<String>) -> impl IntoElement {
    input()
        .value(move || query.get())
        .on_change(move |text: &str| query.set(text.to_owned()))
        .placeholder(t!("home.search"))
        .label(t!("home.search"))
        .h(px(36.0))
        .px_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .bg(move || theme().background)
        .text_size(px(14.0))
        .text_color(move || theme().foreground)
}

/// The file to continue reading, if any (tracked).
fn last_read(recent: RecentState) -> Option<RecentFile> {
    let files = recent.files();
    let path = continue_reading(
        files.iter().map(|file| (file.path.as_path(), file.seen)),
        Path::is_file,
    )?
    .to_path_buf();
    files.into_iter().find(|file| file.path == path)
}

/// "Continue reading": the file shown last, with its cover, name, folder and page; pressing it opens it there.
fn study_card(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    let HomeStudySummary {
        due_cards,
        streak_days,
    } = home_study_summary();
    let count_text = t!("home.study.summary", due = due_cards, streak = streak_days);
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .p(px(14.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(
            view()
                .flex_1()
                .gap_0p5()
                .child(
                    text(move || t!("home.study.title"))
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(count_text)
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(primary_button("command.daily_review", move || {
            run(CommandId::DailyReview);
        }))
        .child(
            button()
                .accessibility_label(t!("command.new_notebook"))
                .on_press(move || run(CommandId::NewNotebook))
                .h(px(32.0))
                .px_3()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || theme().border)
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .child(
                    text(move || t!("command.new_notebook"))
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                ),
        )
}

/// "Continue reading": the file shown last, with its cover, name, folder and page; pressing it opens it there.
fn continue_card(recent: RecentState, covers: Covers, tabs: TabState) -> impl IntoElement {
    // Made again when the file to offer changes: its cover and name are fixed per card.
    dynamic(move || {
        let Some(file) = last_read(recent) else {
            return view().into_element();
        };
        let name = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.path.display().to_string());
        let folder = file
            .path
            .parent()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        let page = file.page + 1;
        let label = t!("home.continue_label", name = name.clone(), page = page);
        let path = file.path.clone();
        untracked(move || {
            button()
                .accessibility_label(label)
                .on_press(move || tabs.open_at(path.clone(), file.page))
                .flex_row()
                .items_center()
                .gap_4()
                .p_4()
                .rounded(px(12.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .shadow_sm()
                .hover(|s| s.bg(move || theme().muted))
                .active(|s| s.opacity(0.8))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(cover(covers, file.path.clone(), 72.0, 96.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            text(move || t!("home.continue"))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().primary),
                        )
                        .child(
                            text(name)
                                .truncate()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(folder)
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    text(t!("recent.page", page = page))
                        .flex_shrink(0.0)
                        .tabular_nums()
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
                .into_element()
        })
    })
}

/// The workspaces, each a chip with its colour, icon, name and tab count; pressing one switches to it.
fn workspace_row(workspaces: Workspaces) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            text(move || t!("home.workspaces"))
                .h(px(28.0))
                .px_3()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(view().flex_row().flex_wrap().gap_2().child(each(
            move || workspaces.list(),
            |workspace: &Workspace| workspace.id,
            move |workspace, _| workspace_chip(workspaces, workspace),
        )))
}

/// One workspace; the shown one is ringed.
fn workspace_chip(workspaces: Workspaces, workspace: ReadSignal<Workspace>) -> impl IntoElement {
    let id = workspace.with_untracked(|workspace| workspace.id);
    let shown = move || workspaces.shown_id() == id;
    let name = move || workspace.with(display_name);
    let icon = move || workspace.with(|workspace| workspace.icon.clone());
    let count = move || {
        let tabs = if shown() {
            workspaces.shown_tab_count()
        } else {
            workspace.with(|workspace| workspace.tabs)
        };
        t!("home.tab_count", n = tabs)
    };
    button()
        .accessibility_label(move || {
            Some(SharedString::from(t!(
                if shown() {
                    "home.workspace_shown"
                } else {
                    "home.workspace"
                },
                name = name(),
                tabs = count()
            )))
        })
        .on_press(move || workspaces.switch(id))
        .h(px(36.0))
        .max_w(px(240.0))
        .px_3()
        .gap_2()
        .flex_row()
        .items_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || {
            if shown() {
                theme().primary
            } else {
                theme().border
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(color_dot(move || workspace.with(|w| w.color.clone())))
        .child(when(
            move || icon().is_some(),
            move || text(move || icon().unwrap_or_default()).text_size(px(13.0)),
        ))
        .child(
            text(name)
                .truncate()
                .min_w(px(0.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(
            text(count)
                .flex_shrink(0.0)
                .tabular_nums()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}
