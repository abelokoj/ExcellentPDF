//! Thumbnails (plan R.5), the view: the sidebar's list of pages, each drawn small with its label under it, the
//! reader's page ringed; a press goes there. Only the rows near the list's viewport exist and are rendered, on the
//! shared render workers below the tiles on screen; each rendered page is kept on disk, named by the file, page and
//! revision, so reopening the file shows them at once.
//!
//! shortcut: the cache folder is never trimmed; the library's housekeeping (K.20) bounds it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ecclesia_core::document::{Document, SharedMupdf};
use ecclesia_core::shared::{pool, registry};
use ecclesia_core::thumbnail;
use ecclesia_features::read::thumbnails::{self, RENDER_WIDTH};
use nsh_base::{button, theme};
use nsh_core::color::{transparent_black, white};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use crate::goto::PageState;
use crate::tabs::TabState;

/// The box a thumbnail's page frame fits in, in points: a Letter sheet's shape.
const BOX_WIDTH: f32 = 136.0;
const BOX_HEIGHT: f32 = 176.0;

/// The document thumbnails are made from, found once its pages are reported.
struct Source {
    document: Arc<Document>,
    mupdf: SharedMupdf,
    /// The file's identity, for the cache names.
    key: u64,
    folder: PathBuf,
}

/// The rendered thumbnails and the renders under way.
#[derive(Clone, Copy)]
pub(crate) struct Thumbnails {
    /// Each rendered page's file.
    ready: Signal<HashMap<usize, SharedString>>,
    /// The renders under way, by page: dropping one cancels it.
    running: Stored<HashMap<usize, Spawned<()>>>,
    source: Stored<Option<Source>>,
    /// Active document path being opened asynchronously so duplicate opens are coalesced.
    opening_for: Stored<Option<PathBuf>>,
    /// Async open task handle; replaced/cancelled immediately when switching away.
    opening_task: Stored<Option<Spawned<()>>>,
    /// Requested pages waiting while [`Self::opening_task`] opens the document off-thread.
    pending_pages: Stored<Vec<usize>>,
    tabs: TabState,
}

impl Thumbnails {
    pub(crate) fn new(tabs: TabState) -> Self {
        let thumbs = Self {
            ready: signal(HashMap::new()),
            running: stored(HashMap::new()),
            source: stored(None),
            opening_for: stored(None),
            opening_task: stored(None),
            pending_pages: stored(Vec::new()),
            tabs,
        };
        // Immediately hibernate the previous document's thumbnail source and cancel in-flight
        // thumbnail renders whenever the active tab changes or closes.
        effect(move || {
            let _shown = tabs.shown_path();
            untracked(|| thumbs.reset());
        });
        thumbs
    }

    fn reset(self) {
        self.running.with(HashMap::clear);
        self.opening_task.with(|task| *task = None);
        self.opening_for.with(|for_path| *for_path = None);
        self.pending_pages.with(Vec::clear);
        self.source.with(|source| *source = None);
        self.ready.set(HashMap::new());
    }

    /// Takes the canvas's `pages` event: a document (or none) was opened, so every thumbnail goes.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name == "pages" {
            self.reset();
        }
    }

    /// The list shows rows `shown` of `count`: render what is near them, cancel what isn't.
    fn request(self, shown: std::ops::RangeInclusive<usize>, count: usize) {
        let Some(wanted) = thumbnails::wanted(shown, count) else {
            return;
        };
        self.running
            .with(|running| running.retain(|page, _| wanted.contains(page)));
        self.pending_pages
            .with(|pending| pending.retain(|page| wanted.contains(page)));
        for page in wanted {
            let known = self.ready.with(|ready| ready.contains_key(&page))
                || self.running.with(|running| running.contains_key(&page));
            if !known {
                self.start(page);
            }
        }
    }

    /// Shows page `page`'s thumbnail from the disk cache, or starts rendering it without blocking
    /// the UI thread on document parsing.
    fn start(self, page: usize) {
        let Some(shown) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        if self.source.with(|source| source.is_none()) {
            self.pending_pages.with(|pending| {
                if !pending.contains(&page) {
                    pending.push(page);
                }
            });
            let already_opening = self
                .opening_for
                .with(|for_path| for_path.as_ref() == Some(&shown));
            if !already_opening {
                self.opening_for
                    .with(|for_path| *for_path = Some(shown.clone()));
                let open_path = shown.clone();
                let folder = data_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("thumbnails");
                let task = spawn(async move {
                    let opened = blocking(move || open_source(Some(open_path), folder)).await;
                    let Ok(Some(src)) = opened else {
                        self.opening_for.with(|for_path| *for_path = None);
                        return;
                    };
                    if untracked(|| self.tabs.shown_path()).as_ref() != Some(&shown) {
                        return;
                    }
                    self.source.with(|slot| *slot = Some(src));
                    self.opening_for.with(|for_path| *for_path = None);
                    let pages = self.pending_pages.with(std::mem::take);
                    for p in pages {
                        self.start(p);
                    }
                });
                self.opening_task.with(|slot| *slot = Some(task));
            }
            return;
        }
        let job = self.source.with(|source| {
            let source = source.as_mut()?;
            let name = thumbnails::file_name(
                source.key,
                page,
                source.document.revision(page),
                RENDER_WIDTH,
            );
            let path = source.folder.join(name);
            if path.is_file() {
                return Some(Err(path));
            }
            let mupdf = Arc::clone(&source.mupdf);
            let job = thumbnail::render(pool(), &source.document, mupdf, page, RENDER_WIDTH, path);
            Some(Ok(job))
        });
        let ready = self.ready;
        let show = move |path: &Path| {
            let path = SharedString::from(path.to_string_lossy().into_owned());
            ready.update(|ready| {
                ready.insert(page, path);
            });
        };
        match job {
            None => {}
            Some(Err(cached)) => show(&cached),
            Some(Ok(job)) => {
                let task = spawn(async move {
                    match job.await {
                        Ok(Ok(path)) => show(&path),
                        Ok(Err(err)) => {
                            log::warn!("thumbnail of page {page}: cannot save it: {err}")
                        }
                        Err(err) if !err.is_aborted() => {
                            log::warn!("thumbnail of page {page}: {err}");
                        }
                        Err(_) => {}
                    }
                });
                self.running.with(|running| {
                    running.insert(page, task);
                });
            }
        }
    }

    /// Page `page`'s file, once rendered.
    fn file(self, page: usize) -> Option<SharedString> {
        self.ready.with(|ready| ready.get(&page).cloned())
    }
}

/// The document the canvas opened (`path`, the shown tab's), from the shared registry: already parsed, so this reads
/// Prepares `path`'s MuPDF handle and its thumbnail folder; it reads nothing until a thumbnail is rendered, so
/// missing a single-file check never fails on an unopened document and avoids holding the document open for
/// only its identity.
fn open_source(path: Option<PathBuf>, folder: PathBuf) -> Option<Source> {
    let path = path?;
    let resolved = crate::expdf::resolve_open_path(&path);
    let opened = registry()
        .open(&resolved)
        .and_then(|document| Ok((document.mupdf()?, document)));
    let (mupdf, document) = match opened {
        Ok(opened) => opened,
        Err(err) => {
            log::warn!("thumbnails: cannot open {}: {err}", path.display());
            return None;
        }
    };
    Some(Source {
        key: document.id().key(),
        document,
        mupdf,
        folder,
    })
}

/// The sidebar's list of pages. `go` gets the index of a page pressed.
pub(crate) fn thumbnail_list(
    thumbs: Thumbnails,
    pages: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    virtual_list(
        move || pages.count(),
        move |at: Position| row(at, thumbs, pages, go),
    )
    .on_viewable_items_change(move |items: &ViewableItems| {
        thumbs.request(items.indices(), pages.count());
    })
    .label(t!("thumbnails.title"))
    .py_2()
    .flex_1()
}

/// One page: its frame (white until rendered), then its label.
fn row(
    at: Position,
    thumbs: Thumbnails,
    pages: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let index = move || at.index();
    let current = move || pages.current() == index();
    let size = move || {
        let (width, height) = pages.size(index());
        thumbnails::frame(width, height, BOX_WIDTH, BOX_HEIGHT)
    };
    let label = move || {
        let page = pages.describe(index());
        let name = if current() {
            t!("thumbnails.page_current", page = page)
        } else {
            t!("thumbnails.page", page = page)
        };
        Some(SharedString::from(name))
    };
    let picture = view()
        .w(move || px(size().0))
        .h(move || px(size().1))
        // Paper: what the rendered page is drawn on, so nothing flashes when it arrives.
        .bg(white())
        .border_1()
        .border_color(move || theme().border)
        .shadow_sm()
        .overflow_hidden()
        .child(when(
            move || thumbs.file(index()).is_some(),
            move || {
                image(move || thumbs.file(index()).unwrap_or_default())
                    .object_fit(ResizeMode::Contain)
                    .size_full()
            },
        ));
    // The ring sits outside the frame and is always there, clear when the page isn't the reader's: nothing moves.
    let ring = view()
        .p(px(2.0))
        .rounded(px(4.0))
        .border_2()
        .border_color(move || {
            if current() {
                theme().ring
            } else {
                transparent_black().into()
            }
        })
        .child(picture);
    let frame_box = view()
        .w(px(BOX_WIDTH + 8.0))
        .h(px(BOX_HEIGHT + 8.0))
        .items_center()
        .justify_center()
        .child(ring);
    let caption = text(move || pages.label(index()))
        .text_size(px(12.0))
        .tabular_nums()
        .truncate()
        .max_w(px(BOX_WIDTH))
        .style_when(current, |s| s.font_weight(FontWeight::SEMIBOLD))
        .text_color(move || {
            if current() {
                theme().sidebar_foreground
            } else {
                theme().muted_foreground
            }
        });
    view().px_2().child(
        button()
            .accessibility_label(label)
            .on_press(move || go(index()))
            .w_full()
            .py_2()
            .gap_1()
            .rounded(px(8.0))
            .items_center()
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.8))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(frame_box)
            .child(caption),
    )
}
