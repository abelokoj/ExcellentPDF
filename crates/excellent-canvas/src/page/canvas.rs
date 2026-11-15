//! One canvas's state on its view's thread: its document, layout, viewport and tiles, and drawing a frame
//! (plan §7.2, §7.4), on any platform through its [`Host`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use crate::layout::{Axis, Direction, Layout, PageSize};
use crate::tiles::{self, TileCache, TileKey};
use crate::view::history::Place;
use crate::view::{Effect, View, ViewCommand};
use crate::viewport::{self, Side, Viewport, ZoomMode};
use excellent_core::darkmode::{PageColors, PageImages};
use excellent_core::document::{self, Document};
use excellent_core::formats;
use excellent_core::render::{Job, Note, Priority, Recolor};
use excellent_core::shared::{pool, registry};
use excellent_core::transform::Transform;
use excellent_ink::stroke::{InkPoint, Stroke, WidthCurve};
use excellent_mupdf::{
    DeviceRect, DisplayList, Link, PageBox, PagePoint, PageRect, Pixmap, Reflow, Target, TextLayout,
};
use excellent_text::{Granularity, PageText, Selection, TextPos};
use nsh_core::JsonValue;

use super::host::{Host, InkPixel, Wake, Wash};
use crate::page::clicks::{ClickLimits, Clicks};
use crate::page::near::Near;
use crate::page::placement::Placement;
use crate::page::saved::SavedView;

mod access;
mod find;
mod minimap;
mod reading;
mod refhover;

pub(super) use refhover::DEFAULT_DELAY as DEFAULT_HOVER_DELAY;

/// Behind the pages until the app sets its theme's `page-area` colour.
const BACKGROUND: u32 = 0x2b2d31;
/// A page whose tiles haven't rendered yet, in the document's own colours.
const PAPER: u32 = 0xffffff;
/// Bytes of tile bitmaps kept that are no longer on screen.
const TILE_BUDGET: usize = 64 << 20;
/// Display lists kept for pages this far from the screen.
const LIST_REACH: usize = 8;
/// Of the screen's width, what the Left and Right arrows scroll while the pages are wider than it (R.3).
const SIDE_STEP: f32 = 0.1;
/// How much of a screen "screen down" moves; the rest stays on screen to keep the reader's place.
const SCREEN_STEP: f32 = 0.9;
/// Ink colour.
// shortcut: one pen in one colour until H.2's pen types and the toolbar's colour picker.
const INK: u32 = 0x1a3d8f;
/// Selected text's highlight: a light blue that keeps black text black under it (drawn darkest-wins), or laid over
/// dark paper translucent ([`Wash`]).
const SELECTION: u32 = 0xb4d5fe;

/// The pointer's shape over the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// The default arrow.
    Arrow,
    /// Over a link: the hand.
    Hand,
    /// Over text, or selecting it: the I-beam.
    Text,
    /// Middle-click scrolling (R.21): the four-way arrows.
    Pan,
}

/// Wakes the canvas's thread from a worker thread, once per batch of finished jobs.
struct Waker {
    posted: AtomicBool,
    wake: Wake,
}

impl Waker {
    fn wake(&self) {
        if !self.posted.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
}

/// A pen sample, in the view's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenSample {
    /// The pointer it belongs to: a stroke follows one pointer.
    pub id: u32,
    /// Where, in the view's pixels.
    pub x: f32,
    /// Where, in the view's pixels.
    pub y: f32,
    /// 0 to 1; `None` when the pen doesn't say.
    pub pressure: Option<f32>,
    /// Touching the screen.
    pub contact: bool,
    /// The pen's eraser end, or the pen turned over.
    pub eraser: bool,
    /// When the input happened, on the host's clock (0: unknown), for the latency log ([`Host::since_input_ms`]).
    pub input: u64,
}

/// An open document, its pages' sizes and its reading direction.
struct Opened {
    document: Arc<Document>,
    bounds: Vec<PageRect>,
    right_to_left: bool,
    /// Each page's label, or `None` when the pages go by number.
    labels: Option<Vec<String>>,
    /// The pages a reflowable book was laid out on (R.28); `None` for fixed pages.
    reflow: Option<Reflow>,
}

/// A book laid out again (R.28): its new pages, and the page that starts where the reader was.
struct Reflowed {
    reflow: Reflow,
    bounds: Vec<PageRect>,
    kept: Option<usize>,
}

/// A page's display list.
enum List {
    /// Being recorded from the page at this revision.
    Building(Job<DisplayList>, u64),
    /// Recorded, with its pictures for smart dark mode (listed when a tile first needs them).
    Ready(Arc<DisplayList>, Arc<PageImages>),
    Failed,
}

/// What a canvas shows and everything it needs to show it, drawn through its platform's `H`.
pub struct PageCanvas<H: Host> {
    host: H,
    waker: Arc<Waker>,
    path: Option<PathBuf>,
    opening: Option<Receiver<document::Result<Opened>>>,
    document: Option<Arc<Document>>,
    /// Every page's box (the view's [`PageBox`]) in page points.
    bounds: Vec<PageRect>,
    /// The pages' boxes being read again after the reader chose another box.
    measuring: Option<Receiver<document::Result<Vec<PageRect>>>>,
    view: View,
    layout: Layout,
    viewport: Viewport,
    lists: HashMap<usize, List>,
    tiles: TileCache<Job<Pixmap>, H::Image>,
    /// Finished strokes, each on its page, in page points.
    // shortcut: kept in memory until the document closes; H.1 commits them to the document as one operation each.
    ink: Vec<PageStroke>,
    /// The stroke the pen is drawing.
    pen: Option<PenStroke>,
    /// Tokens of the points handed to the host's ink trail that it still draws, oldest first.
    trail: Vec<u32>,
    /// The page last reported as the reader's.
    reported_page: Option<usize>,
    /// The view last reported, for the app to save with the tab (R.9).
    reported_view: Option<SavedView>,
    /// The links of the pages near the screen (R.7).
    links: Near<Vec<Link>>,
    /// The notes of the pages near the screen, for screen readers (U.2).
    notes: Near<Vec<Note>>,
    /// The text of the pages near the screen and of the selected pages (K.13).
    texts: Near<TextLayout, PageText>,
    /// The selected text.
    selection: Option<Selection>,
    /// The left button is down and dragging the selection's focus.
    selecting: bool,
    /// The mouse is over text, not a link.
    over_text: bool,
    /// The left button's presses, for double and triple clicks.
    clicks: Clicks,
    /// A copy is waiting for the text of selected pages that aren't read yet.
    copy_pending: bool,
    /// The link under the mouse, as last reported.
    hovered: Option<Link>,
    /// The link the left button went down on: releasing over it follows it.
    pressed: Option<Link>,
    /// When the document was asked for, until its first page is drawn complete (logged for the open budget, §13.1).
    opened_at: Option<Instant>,
    /// Behind the pages, `0xRRGGBB`.
    background: u32,
    /// The colours the pages are drawn in (R.14); `None`: the document's own.
    page_colors: Option<PageColors>,
    /// The reading ruler, focus mode and auto-scroll (R.19–R.21).
    reading: reading::Reading,
    /// A reflowable book's text size, in points (R.28).
    font_size: f32,
    /// The pages the book shown is laid out on; `None` for fixed pages.
    reflow: Option<Reflow>,
    /// The book being laid out again for a new window size or text size.
    reflowing: Option<Receiver<document::Result<Reflowed>>>,
    /// The size or the text changed again while the book was being laid out.
    reflow_again: bool,
    /// Find in document (F.1).
    finder: find::Finder,
    /// The mini-map (R.17).
    minimap: minimap::MiniMap,
    /// Citation and equation hover (R.25).
    refhover: refhover::RefHover<H::Image>,
}

/// A stroke on a page, in that page's points: it stays on the page through any layout, zoom or rotation.
struct PageStroke {
    page: usize,
    stroke: Stroke,
}

/// A stroke being drawn, the pointer drawing it, and how long its samples took to reach the screen.
struct PenStroke {
    id: u32,
    page: usize,
    stroke: Stroke,
    latency: Latency,
}

/// Input-to-present times of one stroke's frames, in milliseconds: from the newest sample's time stamp to the frame
/// being handed to the display. The frame shows on the next refresh after that, which this doesn't see.
#[derive(Debug, Default)]
struct Latency {
    frames: u32,
    total: f64,
    worst: f64,
}

impl Latency {
    /// Adds a frame whose newest sample happened `ms` milliseconds ago (`None`: unknown, not counted).
    fn add(&mut self, ms: Option<f64>) {
        let Some(ms) = ms else {
            return;
        };
        self.frames += 1;
        self.total += ms;
        self.worst = self.worst.max(ms);
    }
}

impl<H: Host> PageCanvas<H> {
    /// A canvas drawing through `host`, on a screen of `pixels_per_point` (device pixels to a page point at 100 %).
    pub fn new(host: H, pixels_per_point: f32) -> Self {
        Self {
            waker: Arc::new(Waker {
                posted: AtomicBool::new(false),
                wake: host.waker(),
            }),
            host,
            path: None,
            opening: None,
            document: None,
            bounds: Vec::new(),
            measuring: None,
            view: View::default(),
            layout: Layout::default(),
            viewport: Viewport::new(pixels_per_point),
            lists: HashMap::new(),
            tiles: TileCache::new(TILE_BUDGET),
            ink: Vec::new(),
            pen: None,
            trail: Vec::new(),
            reported_page: None,
            reported_view: None,
            links: Near::new("links"),
            notes: Near::new("notes"),
            texts: Near::new("text"),
            selection: None,
            selecting: false,
            over_text: false,
            clicks: Clicks::default(),
            copy_pending: false,
            hovered: None,
            pressed: None,
            opened_at: None,
            background: BACKGROUND,
            page_colors: None,
            reading: reading::Reading::default(),
            font_size: formats::DEFAULT_FONT_SIZE,
            reflow: None,
            reflowing: None,
            reflow_again: false,
            finder: find::Finder::default(),
            minimap: minimap::MiniMap::default(),
            refhover: refhover::RefHover::default(),
        }
    }

    /// Shows the document at `path` (`None`: nothing). It opens on a background thread.
    pub fn open(&mut self, path: Option<&Path>) {
        if self.path.as_deref() == path {
            return;
        }
        self.path = path.map(Path::to_path_buf);
        self.close();
        let Some(path) = self.path.clone() else {
            self.invalidate();
            return;
        };
        self.opened_at = Some(Instant::now());
        let page_box = self.view.page_box;
        let reflow = self.reflow_size();
        self.opening = self.in_background("open-document", move || {
            open_document(&path, page_box, reflow)
        });
    }

    /// The pages a reflowable book is laid out on in this window, at the reader's text size (R.28).
    fn reflow_size(&self) -> Reflow {
        let (width, height) = self.viewport_size();
        let ppp = self.viewport.pixels_per_point();
        // The layout keeps a gap round the pages; the book's pages fill what is left.
        let gaps = 2.0 * crate::layout::GAP;
        formats::reflow_pages(width / ppp - gaps, height / ppp - gaps, self.font_size)
    }

    /// The reader's text size for books, in points (`None`: the default).
    pub fn set_reflow_font_size(&mut self, points: Option<f32>) {
        let range = formats::FONT_SIZES;
        let points = points.unwrap_or(formats::DEFAULT_FONT_SIZE);
        self.font_size = points.clamp(*range.start(), *range.end());
        self.reflow();
    }

    /// Lays the book shown out again for the window and the text size, on a background thread, keeping the
    /// reader's place. Fixed pages, and a book already laid out so, are left alone; a change while a layout runs
    /// waits for it.
    fn reflow(&mut self) {
        let (Some(document), Some(current)) = (self.document.clone(), self.reflow) else {
            return;
        };
        if self.reflowing.is_some() {
            self.reflow_again = true;
            return;
        }
        let target = self.reflow_size();
        if target == current {
            return;
        }
        let keep = self.viewport.current_page(&self.layout);
        let page_box = self.view.page_box;
        self.reflowing = self.in_background("reflow", move || {
            let kept = document.reflow(target, keep)?;
            let mupdf = document.mupdf()?;
            let bounds = page_boxes(&mupdf.lock().expect("document lock poisoned"), page_box)?;
            Ok(Reflowed {
                reflow: target,
                bounds,
                kept,
            })
        });
    }

    /// Runs `work` on a thread of its own and wakes the canvas when it is done; `None` (logged) if the system can't
    /// start the thread.
    fn in_background<T: Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<Receiver<T>> {
        let (sender, receiver) = mpsc::channel();
        let waker = Arc::clone(&self.waker);
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // Fails only if the canvas closed, and then no one wants the result.
                let _ = sender.send(work());
                waker.wake();
            });
        match spawned {
            Ok(_) => Some(receiver),
            Err(err) => {
                log::error!("cannot start the {name} thread: {err}");
                None
            }
        }
    }

    /// Tells the app the reader pressed on the pages (`press`), so the keyboard goes to them: a window of its own,
    /// the canvas never takes the app's keyboard focus.
    pub fn pressed(&self) {
        self.emit("press", JsonValue::Null);
    }

    /// A pen touched the canvas (`penContact`), wherever it landed.
    pub fn pen_contact(&self) {
        self.emit("penContact", JsonValue::Null);
    }

    /// Tells the app something happened, through the host.
    fn emit(&self, name: &str, data: JsonValue) {
        self.host.emit(name, data);
    }

    /// The platform's side of the canvas.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// The platform's side of the canvas, to change.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Reports the document's pages (`pages`: `{count, labels, sizes}`, labels `null` when the pages go by number,
    /// sizes `[width, height]` in points as laid out), and that no page is reported yet.
    fn report_pages(&mut self, labels: Option<Vec<String>>) {
        self.reported_page = None;
        let labels = labels.map_or(JsonValue::Null, |labels| labels.into());
        let sizes: Vec<[f32; 2]> = self
            .layout
            .pages()
            .iter()
            .map(|page| [page.x1 - page.x0, page.y1 - page.y0])
            .collect();
        self.emit(
            "pages",
            serde_json::json!({ "count": self.layout.len(), "labels": labels, "sizes": sizes }),
        );
    }

    /// Reports the view (the place, zoom, layout and rotation) when it changed, for the app to save with the tab.
    fn report_view(&mut self) {
        let Some(place) = self.place() else {
            return;
        };
        let view = SavedView {
            place,
            spec: self.view.spec,
            rotation: self.view.rotation,
        };
        if self.reported_view != Some(view) {
            self.reported_view = Some(view);
            self.emit("view", view.to_json());
        }
    }

    /// Shows a saved view (R.9): its layout and rotation, then its zoom and place. A book keeps the zoom it is laid
    /// out for and goes to the page.
    // shortcut: a book's page number points at other text when the window or the text size differ from when it was
    // saved; save a MuPDF bookmark instead when readers notice.
    pub(crate) fn set_view(&mut self, saved: SavedView) {
        let page = saved.place.anchor.page;
        if page >= self.layout.len() {
            log::warn!("page canvas: no page {page} to show");
            return;
        }
        if self.reflow.is_some() {
            self.viewport.scroll_to_page(page, &self.layout);
            self.invalidate();
            return;
        }
        if self.view.rotation != saved.rotation {
            self.tiles.clear();
        }
        self.view.spec = saved.spec;
        self.view.rotation = saved.rotation;
        self.view.resized(self.viewport.width());
        self.layout = Layout::new(&self.shown_sizes(), self.view.spec);
        self.go_to_place(saved.place);
    }

    /// Reports the reader's page when it changed.
    fn report_page(&mut self) {
        let page = self.viewport.current_page(&self.layout);
        if page != self.reported_page {
            self.reported_page = page;
            if let Some(index) = page {
                self.emit("page", serde_json::json!({ "index": index }));
            }
        }
    }

    /// Jumps to `point` on page `page` (B.6), at `zoom` when there is one (1 is 100 %), else in the reader's zoom,
    /// remembering where the reader was (R.8).
    pub fn go_to_point(&mut self, page: usize, point: PagePoint, zoom: Option<f32>) {
        if let Some(zoom) = zoom {
            self.viewport.set_zoom(zoom, &self.layout);
        }
        self.jump(Target {
            page,
            point: Some(point),
        });
    }

    /// Jumps to page `index` (R.6), remembering where the reader was (R.8).
    pub fn go_to_page(&mut self, index: usize) {
        self.jump(Target {
            page: index,
            point: None,
        });
    }

    /// Jumps to `target` (go to page, a link), remembering where the reader was (R.8).
    fn jump(&mut self, target: Target) {
        if target.page >= self.layout.len() {
            log::warn!("page canvas: no page {}", target.page);
            return;
        }
        if let Some(here) = self.place() {
            self.view.history.jumped_from(here);
        }
        let placement =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, target.page);
        match target.point.zip(placement) {
            Some((point, placement)) => {
                let (x, y) = placement.to_layout(point);
                self.viewport
                    .scroll_to_point(target.page, x, y, &self.layout);
            }
            None => self.viewport.scroll_to_page(target.page, &self.layout),
        }
        self.invalidate();
    }

    /// The page under client pixel (`x`, `y`) and the point on it there, in page points.
    fn page_point(&self, x: i32, y: i32) -> Option<(usize, PagePoint)> {
        let (x, y) = self.viewport.to_layout_point(x as f32, y as f32);
        let page = self.layout.page_at(x, y)?;
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        Some((page, placement.to_page(x, y)))
    }

    /// The link at client pixel (`x`, `y`), if there is one there and its page's links are read: the last one MuPDF
    /// read, which is drawn on top.
    fn link_at(&self, x: i32, y: i32) -> Option<&Link> {
        let (page, point) = self.page_point(x, y)?;
        self.links.get(page)?.iter().rev().find(|link| {
            let r = &link.rect;
            (r.x0..=r.x1).contains(&point.x) && (r.y0..=r.y1).contains(&point.y)
        })
    }

    /// Where in the text client pixel (`x`, `y`) is, for a selection by `granularity`: a caret when selecting
    /// characters, otherwise the character there. `None` off the pages or before the page's text is read.
    fn text_pos(&self, x: i32, y: i32, granularity: Granularity) -> Option<TextPos> {
        let (page, point) = self.page_point(x, y)?;
        let text = self.texts.get(page)?;
        let index = match granularity {
            Granularity::Char => text.caret(point)?,
            _ => text.nearest_char(point)?,
        };
        Some(TextPos { page, index })
    }

    /// Whether client pixel (`x`, `y`) is over a line of text.
    fn is_over_text(&self, x: i32, y: i32) -> bool {
        self.page_point(x, y).is_some_and(|(page, point)| {
            self.texts
                .get(page)
                .is_some_and(|text| text.is_over_text(point))
        })
    }

    /// The pointer's shape where the mouse is.
    pub fn cursor(&self) -> Cursor {
        if self.reading.is_panning() {
            Cursor::Pan
        } else if self.selecting || (self.over_text && self.hovered.is_none()) {
            Cursor::Text
        } else if self.hovered.is_some() {
            Cursor::Hand
        } else {
            Cursor::Arrow
        }
    }

    /// The mouse moved to client pixel (`x`, `y`), or left the canvas (`None`). While selecting, the selection
    /// follows it. Otherwise tells the app when the link under it changes (`linkHover`: `{uri, page}`, page `null`
    /// for a link out of the document; `null` for no link).
    pub fn mouse_moved(&mut self, at: Option<(i32, i32)>) {
        self.reading_mouse(at);
        if let Some((x, y)) = at
            && (self.minimap_drag(y) || self.over_minimap(x, y) || self.over_popover(x, y))
        {
            // Over the mini-map (R.17) or a hover preview (R.25): no text or link under the pointer.
            self.hover_moved(at, None);
            self.over_text = false;
            if self.hovered.take().is_some() {
                self.emit("linkHover", JsonValue::Null);
            }
            return;
        }
        if self.selecting {
            // shortcut: the selection stops at the screen's edge; dragging past it doesn't scroll yet.
            let Some(selection) = self.selection else {
                return;
            };
            let focus = at.and_then(|(x, y)| self.text_pos(x, y, selection.granularity));
            if let Some(focus) = focus.filter(|&focus| focus != selection.focus) {
                self.selection = Some(Selection { focus, ..selection });
                self.invalidate();
            }
            return;
        }
        self.over_text = at.is_some_and(|(x, y)| self.is_over_text(x, y));
        let link = at.and_then(|(x, y)| self.link_at(x, y)).cloned();
        self.hover_moved(at, link.as_ref());
        if link == self.hovered {
            return;
        }
        let data = link.as_ref().map_or(JsonValue::Null, |link| {
            let page = link.target.map(|target| target.page);
            serde_json::json!({ "uri": link.uri, "page": page })
        });
        self.hovered = link;
        self.emit("linkHover", data);
    }

    /// The left button went down at client pixel (`x`, `y`), at message time `time`. On a link it waits for the
    /// button to come up. On text it starts a selection: by characters, or by words, lines or paragraphs for a
    /// second, third or fourth click. Anywhere else it clears the selection. Returns whether a selection started,
    /// when the window should capture the mouse until the button comes up.
    pub fn mouse_down(&mut self, x: i32, y: i32, time: u32, limits: ClickLimits) -> bool {
        if self.popover_down(x, y) {
            return false;
        }
        if self.minimap_down(x, y) {
            return true;
        }
        let clicks = self.clicks.press(time, x, y, limits);
        self.pressed = self.link_at(x, y).cloned();
        if self.pressed.is_some() {
            return false;
        }
        let granularity = Granularity::from_clicks(clicks);
        let start = (clicks > 1 || self.is_over_text(x, y))
            .then(|| self.text_pos(x, y, granularity))
            .flatten();
        let had = self.selection.is_some();
        self.selection = start.map(|at| Selection::new(at, granularity));
        self.selecting = start.is_some();
        if had || self.selecting {
            self.invalidate();
        }
        self.selecting
    }

    /// The left button came up at client pixel (`x`, `y`): a selection ends there; over the link it went down on,
    /// the link is followed. A link inside the document jumps; any other goes to the app (`openLink`: `{uri}`),
    /// which decides whether to open it. Returns whether a selection ended, when the window lets the mouse go.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> bool {
        if self.minimap_up() {
            return true;
        }
        if self.selecting {
            self.selecting = false;
            if self.selection.is_some_and(|s| s.is_collapsed()) {
                self.selection = None;
            }
            self.host.selection_changed();
            return true;
        }
        let Some(pressed) = self.pressed.take() else {
            return false;
        };
        if self.link_at(x, y) != Some(&pressed) {
            return false;
        }
        self.follow(&pressed);
        false
    }

    /// Something else took the mouse while a selection was being dragged: the selection stays as it is.
    pub fn capture_lost(&mut self) {
        self.minimap_up();
        self.selecting = false;
        self.stop_pan();
    }

    /// Copies the selected text: the canvas sends it to the app (`copy`: `{text}`), which puts it on the clipboard.
    /// Pages of the selection whose text isn't read yet are waited for.
    pub fn copy(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        if selection.pages().any(|page| self.texts.get(page).is_none()) {
            // Their reads were started with the last frame, which asked for every selected page.
            self.copy_pending = true;
            return;
        }
        self.copy_pending = false;
        let text = selection.text(|page| self.texts.get(page));
        if !text.is_empty() {
            self.emit("copy", serde_json::json!({ "text": text }));
        }
    }

    /// Selects all the text of the reader's page.
    // shortcut: the reader's page, not the whole document, until K.13's document-wide text index (search, R.18).
    pub fn select_page(&mut self) {
        let Some(page) = self.viewport.current_page(&self.layout) else {
            return;
        };
        let Some(text) = self.texts.get(page).filter(|text| !text.is_empty()) else {
            return;
        };
        self.selection = Some(Selection {
            anchor: TextPos { page, index: 0 },
            focus: TextPos {
                page,
                index: text.len(),
            },
            granularity: Granularity::Char,
        });
        self.invalidate();
        self.host.selection_changed();
    }

    /// Tells the app where the reader is, for a new outline entry or bookmark (B.1, B.4, B.6), as `readerPlace`:
    /// `{page, x, y, zoom, title}`, the place in page points. With selected text it is the top of the selection's
    /// first line and `title` is the text; otherwise the top left of what shows of the first page in view, and
    /// `title` is `null`. `zoom` is the reader's (1 is 100 %), `null` while a fit mode sets it.
    pub fn reader_place(&self) {
        let selected = self
            .selection
            .filter(|selection| !selection.is_collapsed())
            .and_then(|selection| {
                let (page, top) = selection.pages().find_map(|page| {
                    let text = self.texts.get(page)?;
                    let range = selection.range_on(page, text)?;
                    Some((page, text.highlight(range).first()?.ul))
                })?;
                Some((page, top, Some(selection.text(|page| self.texts.get(page)))))
            });
        let Some((page, point, title)) = selected.or_else(|| {
            let (page, point) = self.view_top_left()?;
            Some((page, point, None))
        }) else {
            return;
        };
        let zoom =
            matches!(self.viewport.zoom_mode(), ZoomMode::Custom).then(|| self.viewport.zoom());
        self.emit(
            "readerPlace",
            serde_json::json!({ "page": page, "x": point.x, "y": point.y, "zoom": zoom, "title": title }),
        );
    }

    /// The first page in view and the top left of what shows of it, in page points.
    fn view_top_left(&self) -> Option<(usize, PagePoint)> {
        let page = self.viewport.anchor(&self.layout)?.page;
        let (x, y) = self.viewport.to_layout_point(0.0, 0.0);
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        let point = placement.to_page(x, y);
        // The view's corner can be in the gap beside or above the page: the nearest point on it.
        let bounds = self.bounds.get(page)?;
        Some((
            page,
            PagePoint {
                x: point.x.max(bounds.x0).min(bounds.x1),
                y: point.y.max(bounds.y0).min(bounds.y1),
            },
        ))
    }

    /// Jumps to the first page.
    pub fn first_page(&mut self) {
        self.go_to_page(0);
    }

    /// Jumps to the last page.
    pub fn last_page(&mut self) {
        if let Some(last) = self.layout.len().checked_sub(1) {
            self.go_to_page(last);
        }
    }

    /// Forgets the document, its tiles and lists; pending jobs are cancelled.
    fn close(&mut self) {
        self.opening = None;
        self.measuring = None;
        self.reflowing = None;
        self.reflow_again = false;
        self.reflow = None;
        self.document = None;
        self.reported_view = None;
        self.bounds.clear();
        self.layout = Layout::default();
        self.forget_pages();
        self.finder.forget(false);
        self.pen = None;
        self.trail.clear();
        self.report_pages(None);
        self.opened_at = None;
        self.reading.reset();
    }

    /// Forgets what was read from and drawn of the pages: tiles, lists, links, text, the selection and the ink on
    /// them. For a closed document, and a book whose pages were laid out again.
    fn forget_pages(&mut self) {
        self.lists.clear();
        self.tiles.clear();
        self.ink.clear();
        self.links.clear();
        self.notes.clear();
        self.host.content_changed();
        self.texts.clear();
        self.selection = None;
        self.selecting = false;
        self.over_text = false;
        self.copy_pending = false;
        self.hovered = None;
        self.pressed = None;
        self.finder.forget(true);
        self.minimap.forget();
        self.refhover.forget();
    }

    /// The view is `width` × `height` pixels on a screen of `ppp` pixels per point.
    pub fn resize(&mut self, width: u32, height: u32, ppp: f32) {
        log::debug!("page canvas: {width} x {height} pixels at {ppp} pixels a point");
        if ppp != self.viewport.pixels_per_point() {
            self.viewport.set_pixels_per_point(ppp, &self.layout);
        }
        self.viewport
            .resize(width as f32, height as f32, &self.layout);
        if self.view.resized(width as f32) {
            self.relayout();
        }
        self.reflow();
        if let Err(err) = self.host.resize(width, height) {
            log::warn!("the page canvas can't follow the new size ({err}); starting over");
            self.lose_device();
        }
        self.invalidate();
    }

    /// Scrolls by device pixels.
    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        self.viewport.scroll_by(dx, dy, &self.layout);
        self.invalidate();
    }

    /// Scrolls by a mouse wheel's device pixels: in the horizontal layout the plain wheel moves along the strip.
    pub fn scroll_wheel(&mut self, dx: f32, dy: f32) {
        if dx == 0.0 && self.layout.axis() == Axis::Horizontal {
            self.viewport.scroll_forward(dy, &self.layout);
            self.invalidate();
        } else {
            self.scroll_by(dx, dy);
        }
    }

    /// The Left or Right arrow (R.3): a row of pages back or on by the reading direction, or sideways when the pages
    /// are wider than the screen.
    pub fn step_side(&mut self, side: Side) {
        let pixels = self.viewport.width() * SIDE_STEP;
        self.viewport.step_side(side, pixels, &self.layout);
        self.invalidate();
    }

    /// Scrolls one screen toward the later pages (`direction` 1) or the earlier ones (-1), along the layout.
    pub fn scroll_screen(&mut self, direction: f32) {
        let length = self.viewport.screen_length(&self.layout) * SCREEN_STEP;
        self.viewport
            .scroll_forward(direction * length, &self.layout);
        self.invalidate();
    }

    /// Multiplies the zoom by `factor` about device pixel (`x`, `y`).
    pub fn zoom_about(&mut self, factor: f32, x: f32, y: f32) {
        self.viewport.zoom_about(factor, x, y, &self.layout);
        self.invalidate();
    }

    /// Zooms to the next step in (`ZoomChange::In`) or out.
    pub fn zoom_step(&mut self, change: ZoomChange) {
        let zoom = self.viewport.zoom();
        let next = match change {
            ZoomChange::In => viewport::step_in(zoom),
            ZoomChange::Out => viewport::step_out(zoom),
        };
        self.viewport.set_zoom(next, &self.layout);
        self.invalidate();
    }

    /// Shows the page at its printed size.
    pub fn actual_size(&mut self) {
        self.viewport.set_zoom(1.0, &self.layout);
        self.invalidate();
    }

    /// Changes how the pages are shown (layout, rotation, box, fit), keeping the reader's place.
    pub fn view_command(&mut self, command: ViewCommand) {
        match self.view.apply(command, self.viewport.width()) {
            Effect::None => {}
            Effect::Relayout => self.relayout(),
            Effect::Rerender => {
                self.tiles.clear();
                self.relayout();
            }
            Effect::Remeasure => self.measure(),
            Effect::Fit(mode) => {
                self.viewport.fit(mode, &self.layout);
                self.invalidate();
            }
        }
    }

    /// Where the reader is, for the history; `None` with no pages.
    fn place(&self) -> Option<Place> {
        Some(Place {
            anchor: self.viewport.anchor(&self.layout)?,
            mode: self.viewport.zoom_mode(),
            zoom: self.viewport.zoom(),
        })
    }

    /// Shows `place`: its zoom, then its page and offset.
    fn go_to_place(&mut self, place: Place) {
        match place.mode {
            ZoomMode::Custom => self.viewport.set_zoom(place.zoom, &self.layout),
            mode => self.viewport.fit(mode, &self.layout),
        }
        self.viewport.relayout(Some(place.anchor), &self.layout);
        self.invalidate();
    }

    /// Back to where the reader was before the last jump (R.8).
    pub fn history_back(&mut self) {
        let Some(here) = self.place() else { return };
        if let Some(place) = self.view.history.back(here) {
            self.go_to_place(place);
        }
    }

    /// Forward again to where back came from (R.8).
    pub fn history_forward(&mut self) {
        let Some(here) = self.place() else { return };
        if let Some(place) = self.view.history.forward(here) {
            self.go_to_place(place);
        }
    }

    /// Every page's size as shown, after the view rotation.
    fn shown_sizes(&self) -> Vec<PageSize> {
        let transform = Transform {
            zoom: 1.0,
            rotation: self.view.rotation,
        };
        self.bounds
            .iter()
            .map(|bounds| {
                let (width, height) = transform.shown_size(bounds);
                PageSize { width, height }
            })
            .collect()
    }

    /// Lays the pages out again for the view, from the same reading position.
    fn relayout(&mut self) {
        let anchor = self.viewport.anchor(&self.layout);
        self.layout = Layout::new(&self.shown_sizes(), self.view.spec);
        self.viewport.relayout(anchor, &self.layout);
        self.invalidate();
    }

    /// Reads every page's box again, for the box the view now shows, on a background thread.
    fn measure(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let page_box = self.view.page_box;
        self.measuring = self.in_background("measure-pages", move || {
            let mupdf = document.mupdf()?;
            let boxes = page_boxes(&mupdf.lock().expect("document lock poisoned"), page_box)?;
            Ok(boxes)
        });
    }

    /// The pen touched down. Returns whether the canvas takes it: not the eraser end, and only over a document.
    pub fn pen_down(&mut self, sample: &PenSample) -> bool {
        // shortcut: the eraser end does nothing until H.5's erasers.
        if sample.eraser || self.document.is_none() {
            return false;
        }
        // Ink belongs to a page: a stroke starts only on one.
        let (x, y) = self.viewport.to_layout_point(sample.x, sample.y);
        let Some(page) = self.layout.page_at(x, y) else {
            return false;
        };
        self.pen = Some(PenStroke {
            id: sample.id,
            page,
            stroke: Stroke::new(),
            latency: Latency::default(),
        });
        self.trail.clear();
        self.host.trail_start(INK);
        self.pen_moved(std::slice::from_ref(sample));
        true
    }

    /// The pen moved through `samples`, oldest first. The host's ink trail, if it has one, gets them at once; our
    /// frame with them is drawn now if the host takes one without waiting, else as soon as it does.
    pub fn pen_moved(&mut self, samples: &[PenSample]) {
        let Some(page) = self.pen.as_ref().map(|pen| pen.page) else {
            return;
        };
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return;
        };
        let Some(pen) = &mut self.pen else {
            return;
        };
        let curve = WidthCurve::default();
        let scale = self.viewport.scale();
        let mut on_screen = Vec::with_capacity(samples.len());
        let mut newest = None;
        for sample in samples.iter().filter(|s| s.id == pen.id && s.contact) {
            let (sx, sy) = (sample.x, sample.y);
            let (x, y) = self.viewport.to_layout_point(sx, sy);
            let on_page = placement.to_page(x, y);
            let width = curve.width(sample.pressure);
            pen.stroke.push(InkPoint {
                x: on_page.x,
                y: on_page.y,
                width,
            });
            on_screen.push((sx, sy, width * scale));
            newest = Some(sample.input);
        }
        let Some(input) = newest else {
            return;
        };
        if let Some(token) = self.host.trail_add(&on_screen) {
            self.trail.push(token);
        }
        let ready = self.host.frame_ready();
        if !ready {
            self.invalidate();
            return;
        }
        self.paint();
        let ms = (input != 0)
            .then(|| self.host.since_input_ms(input))
            .flatten();
        if let Some(pen) = &mut self.pen {
            pen.latency.add(ms);
        }
    }

    /// The pen lifted (or the system took the pointer away): the stroke is finished.
    pub fn pen_up(&mut self) {
        let Some(pen) = self.pen.take() else {
            return;
        };
        let Latency {
            frames,
            total,
            worst,
        } = pen.latency;
        if frames > 0 {
            log::debug!(
                "ink: {} points, {frames} frames, input to present {:.1} ms on average, {worst:.1} ms at worst",
                pen.stroke.len(),
                total / f64::from(frames),
            );
        }
        if !pen.stroke.is_empty() {
            self.ink.push(PageStroke {
                page: pen.page,
                stroke: pen.stroke,
            });
        }
        self.invalidate();
    }

    /// Whether pointer `id` is drawing a stroke.
    pub fn is_drawing(&self, id: u32) -> bool {
        self.pen.as_ref().is_some_and(|pen| pen.id == id)
    }

    /// A worker finished something: take what is done.
    pub fn jobs_done(&mut self) {
        self.waker.posted.store(false, Ordering::Release);
        let mut changed = self.take_opened();
        changed |= self.take_measured();
        changed |= self.take_reflowed();
        self.find_again();
        changed |= self.take_found();
        changed |= self.take_marks();
        changed |= self.take_hover();
        let mut read = self.links.take_done();
        read |= self.notes.take_done();
        if self.texts.take_done() {
            read = true;
            changed |= self.selection.is_some();
            changed |= self.text_arrived();
            if self.copy_pending {
                self.copy();
            }
        }
        if read {
            self.host.content_changed();
        }
        for (&page, list) in &mut self.lists {
            let List::Building(job, revision) = list else {
                continue;
            };
            match job.try_take() {
                Some(Ok(ready)) => {
                    let ready = Arc::new(ready);
                    if let Some(document) = &self.document {
                        document.keep_display_list(page, *revision, Arc::clone(&ready));
                    }
                    *list = List::Ready(ready, Arc::default());
                }
                Some(Err(err)) => {
                    if !err.is_aborted() {
                        log::warn!("cannot record a page: {err}");
                    }
                    *list = List::Failed;
                }
                None => continue,
            }
            changed = true;
        }
        if self.host.can_upload() {
            let host = &mut self.host;
            changed |= self.tiles.poll(|job| {
                let result = job.try_take()?;
                Some(match result {
                    Ok(pixmap) => host.upload(&pixmap),
                    Err(err) => {
                        if !err.is_aborted() {
                            log::warn!("cannot render a tile: {err}");
                        }
                        None
                    }
                })
            });
        }
        if changed {
            self.invalidate();
        }
    }

    /// The document finished opening, if it did.
    fn take_opened(&mut self) -> bool {
        let Some(result) = take_sent(&mut self.opening, "open-document") else {
            return false;
        };
        match result {
            Ok(opened) => {
                self.bounds = opened.bounds;
                self.reflow = opened.reflow;
                self.document = Some(opened.document);
                self.view.history = Default::default();
                // R.3: a document that reads right to left opens that way, without a setting.
                self.view.spec.direction = if opened.right_to_left {
                    Direction::RightToLeft
                } else {
                    Direction::LeftToRight
                };
                let (w, h) = self.viewport_size();
                self.view.resized(w);
                self.layout = Layout::new(&self.shown_sizes(), self.view.spec);
                // A new document fits the width, and keeps fitting it as the window changes. A book's pages are
                // the window's size already, so it shows them at their size: its text at the reader's.
                self.viewport = Viewport::new(self.viewport.pixels_per_point());
                self.viewport.resize(w, h, &self.layout);
                if self.reflow.is_some() {
                    self.viewport.set_zoom(1.0, &self.layout);
                } else {
                    self.viewport.fit(ZoomMode::FitWidth, &self.layout);
                }
                self.viewport.scroll_to_page(0, &self.layout);
                self.report_pages(opened.labels);
                // The window or the text size may have changed while it opened.
                self.reflow();
            }
            // shortcut: the failure is only logged; the error view comes with R.11's open flow.
            Err(err) => log::error!("cannot open {:?}: {err}", self.path),
        }
        true
    }

    /// The book was laid out again, if it was: its new pages, at the reader's place.
    fn take_reflowed(&mut self) -> bool {
        let Some(result) = take_sent(&mut self.reflowing, "reflow") else {
            return false;
        };
        match result {
            Ok(reflowed) => {
                self.reflow = Some(reflowed.reflow);
                self.bounds = reflowed.bounds;
                self.forget_pages();
                // Page numbers before the layout mean other text now.
                self.view.history = Default::default();
                self.layout = Layout::new(&self.shown_sizes(), self.view.spec);
                self.viewport.relayout(None, &self.layout);
                self.viewport
                    .scroll_to_page(reflowed.kept.unwrap_or(0), &self.layout);
                self.report_pages(None);
            }
            Err(err) => log::warn!("cannot lay the book out again: {err}"),
        }
        if std::mem::take(&mut self.reflow_again) {
            self.reflow();
        }
        true
    }

    /// The pages' new boxes were read, if they were.
    fn take_measured(&mut self) -> bool {
        let Some(result) = take_sent(&mut self.measuring, "measure-pages") else {
            return false;
        };
        match result {
            Ok(bounds) if bounds.len() == self.bounds.len() => {
                self.bounds = bounds;
                self.tiles.clear();
                self.relayout();
            }
            Ok(_) => log::warn!("the page count changed while the pages were measured"),
            Err(err) => log::warn!(
                "cannot read the pages' {:?} boxes: {err}",
                self.view.page_box
            ),
        }
        true
    }

    /// The view's size in pixels.
    fn viewport_size(&self) -> (f32, f32) {
        self.host.size()
    }

    /// Draws a frame and asks for what it is missing.
    pub fn paint(&mut self) {
        let (width, height) = self.viewport_size();
        if width < 1.0 || height < 1.0 {
            return;
        }
        if let Err(err) = self.host.begin(width, height) {
            // No frame until something asks for one again: a device that can't be made now won't be in a loop.
            log::error!("cannot draw the page canvas: {err}");
            self.lose_device();
            return;
        }
        let moving = self.advance_motion();
        self.update_marks();
        let needed = tiles::visible_tiles(&self.layout, &self.viewport);
        self.request(&needed);
        self.tiles.begin_frame();
        // The mini-map (R.17) over everything, before the frame ends.
        let drawn = self
            .draw(&needed)
            .and_then(|()| self.draw_minimap())
            .and_then(|()| self.draw_popover())
            .and_then(|()| self.host.end());
        if let Err(err) = drawn {
            log::warn!("the page canvas lost its device ({err}); starting over");
            self.lose_device();
            self.invalidate();
        }
        self.tiles.evict();
        self.report_page();
        if !moving {
            self.report_view();
        }
        // The next frame waits for the display's refresh (`Host::begin`), so moving pages go at its rate.
        if moving {
            self.invalidate();
        }
    }

    /// Starts the display lists and tiles the screen needs, and cancels those it no longer does.
    fn request(&mut self, needed: &[TileKey]) {
        let Some(document) = &self.document else {
            return;
        };
        let mupdf = match document.mupdf() {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::error!("cannot open {:?} again: {err}", document.path());
                return;
            }
        };
        // In a right-to-left strip the screen's pages run backwards: take the lowest and highest.
        let Some(first) = needed.iter().map(|key| key.page).min() else {
            return;
        };
        let last = needed.iter().map(|key| key.page).max().unwrap_or(first);
        let waker = &self.waker;
        self.links.request(first..=last, |page| {
            let job = pool().load_links(&mupdf, page, Priority::Nearby);
            let waker = Arc::clone(waker);
            job.when_done(move || waker.wake());
            job
        });
        // shortcut: read whether or not a screen reader listens; asking UIA (`UiaClientsAreListening`) would save
        // the reads when none does.
        self.notes.request(first..=last, |page| {
            let job = pool().load_notes(&mupdf, page, Priority::Nearby);
            let waker = Arc::clone(waker);
            job.when_done(move || waker.wake());
            job
        });
        // Text for the pages on screen, one more each side while the reading line may step onto them, and every
        // selected page so a copy has all of it.
        let (near_first, near_last) = if self.reading.reads_ahead() {
            (
                first.saturating_sub(1),
                (last + 1).min(self.layout.len().saturating_sub(1)),
            )
        } else {
            (first, last)
        };
        let selected = self.selection.map(|s| s.pages());
        let text_first = selected
            .as_ref()
            .map_or(near_first, |pages| pages.start.min(near_first));
        let text_last = selected.map_or(near_last, |pages| (pages.end - 1).max(near_last));
        self.texts.request(text_first..=text_last, |page| {
            let job = pool().read_text(&mupdf, page, Priority::PageText);
            let waker = Arc::clone(waker);
            job.when_done(move || waker.wake());
            job
        });
        // shortcut: lists are kept by distance from the screen, not by size; K.7 budgets them.
        self.lists
            .retain(|page, _| *page + LIST_REACH >= first && *page <= last + LIST_REACH);
        let wanted: HashSet<TileKey> = needed.iter().copied().collect();
        self.tiles.cancel_unneeded(&wanted);
        for key in needed {
            if self.tiles.contains(key) {
                continue;
            }
            let list = self.lists.entry(key.page).or_insert_with(|| {
                // Another view of the document may have recorded it.
                if let Some(list) = document.display_list(key.page) {
                    return List::Ready(list, Arc::default());
                }
                let revision = document.revision(key.page);
                let job = pool().build_display_list(&mupdf, key.page, Priority::Visible);
                let waker = Arc::clone(&self.waker);
                job.when_done(move || waker.wake());
                List::Building(job, revision)
            });
            let List::Ready(list, images) = list else {
                continue;
            };
            let (Some(bounds), Some(page)) =
                (self.bounds.get(key.page), self.layout.page(key.page))
            else {
                continue;
            };
            let area = key.area(&page);
            let ctm = Transform {
                zoom: tiles::bucket_scale(key.bucket),
                rotation: self.view.rotation,
            }
            .matrix(bounds);
            let area = DeviceRect {
                x0: area.x0,
                y0: area.y0,
                x1: area.x1,
                y1: area.y1,
            };
            let recolor = self.page_colors.map(|colors| Recolor {
                colors,
                images: Arc::clone(images),
            });
            let job = pool().render_tile(list, ctm, area, Priority::Visible, recolor);
            let waker = Arc::clone(&self.waker);
            job.when_done(move || waker.wake());
            self.tiles.request(*key, job);
        }
    }

    /// Draws the pages: paper first, then any older zoom's tiles while this zoom's render, then this zoom's; then the
    /// selection, find's hits, the reading aids and the ink. The frame is begun and ended by [`Self::paint`].
    fn draw(&mut self, needed: &[TileKey]) -> Result<(), H::Error> {
        let aids = self.aids();
        let found = self.found_on_screen();
        let (paper, wash) = (self.paper(), self.wash());
        let host = &mut self.host;
        host.clear(self.background);
        let screen = crate::Rect::new(0.0, 0.0, self.viewport.width(), self.viewport.height());
        let visible = self.viewport.visible();
        let mut first_complete = None;
        for (index, page) in self.layout.pages_in(&visible) {
            let on_screen = self.viewport.to_screen(&page);
            host.fill(&on_screen, paper)?;
            let page_tiles = needed.iter().filter(|k| k.page == index);
            let complete = page_tiles.clone().all(|k| self.tiles.ready(k).is_some());
            first_complete.get_or_insert(complete);
            if !complete {
                for key in self
                    .tiles
                    .others_of_page(index, tiles::bucket(self.viewport.scale()))
                {
                    let Some(dest) = key.on_screen(&self.layout, &self.viewport) else {
                        continue;
                    };
                    if dest.intersect(&screen).is_none() {
                        continue;
                    }
                    if let Some(image) = self.tiles.ready(&key) {
                        host.image(image, &dest);
                    }
                }
            }
            for key in page_tiles {
                let Some(dest) = key.on_screen(&self.layout, &self.viewport) else {
                    continue;
                };
                if let Some(image) = self.tiles.ready(key) {
                    host.image(image, &dest);
                }
            }
        }
        if first_complete == Some(true)
            && let Some(start) = self.opened_at.take()
        {
            // The budget script (tests/budgets.ps1) reads this line.
            log::info!(
                "first page drawn {:.1} ms after the document was asked for",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(selection) = self.selection {
            let mut quads = Vec::new();
            for (index, _) in self.layout.pages_in(&visible) {
                let (Some(text), Some(placement)) = (
                    self.texts.get(index),
                    Placement::of_page(&self.layout, &self.bounds, self.view.rotation, index),
                ) else {
                    continue;
                };
                let Some(range) = selection.range_on(index, text) else {
                    continue;
                };
                for quad in text.highlight(range) {
                    quads.push([quad.ul, quad.ur, quad.lr, quad.ll].map(|p| {
                        let (x, y) = placement.to_layout(p);
                        self.viewport.to_screen_point(x, y)
                    }));
                }
            }
            host.highlight(&quads, SELECTION, wash)?;
        }
        found.draw(host, wash)?;
        aids.draw_under(host)?;
        let scale = self.viewport.scale();
        let strokes = self
            .ink
            .iter()
            .map(|ink| (ink.page, &ink.stroke))
            .chain(self.pen.as_ref().map(|pen| (pen.page, &pen.stroke)));
        for (page, stroke) in strokes {
            let Some(placement) =
                Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
            else {
                continue;
            };
            let points: Vec<InkPixel> = stroke
                .points()
                .iter()
                .map(|p| {
                    let (x, y) = placement.to_layout(PagePoint { x: p.x, y: p.y });
                    let (x, y) = self.viewport.to_screen_point(x, y);
                    (x, y, p.width * scale)
                })
                .collect();
            host.polyline(&points, INK)?;
        }
        aids.draw_over(host)?;
        // This frame shows every point the trail has. While the pen draws, the trail keeps the newest batch, so its
        // next points join on to it; once the stroke is done, the trail empties.
        let keep = usize::from(self.pen.is_some());
        if self.trail.len() > keep {
            let last = self.trail.len() - 1 - keep;
            host.trail_remove(self.trail[last]);
            self.trail.drain(..=last);
        }
        Ok(())
    }

    /// The device is gone: its images go with it; the next frame makes a new one.
    fn lose_device(&mut self) {
        self.host.lose_device();
        self.tiles.clear();
        self.refhover.forget();
    }

    /// Draws `rgb` (`0xRRGGBB`) behind the pages: the theme's page area (K.17).
    pub fn set_background(&mut self, rgb: u32) {
        if self.background != rgb {
            self.background = rgb;
            self.invalidate();
        }
    }

    /// Draws the pages in `colors` (`None`: their own), rendering them again (R.14).
    pub fn set_page_colors(&mut self, colors: Option<PageColors>) {
        if self.page_colors != colors {
            self.page_colors = colors;
            self.tiles.clear();
            self.invalidate();
        }
    }

    /// A page's colour before its tiles arrive, `0xRRGGBB`.
    fn paper(&self) -> u32 {
        self.page_colors.map_or(PAPER, |colors| {
            let [r, g, b] = colors.paper.map(u32::from);
            r << 16 | g << 8 | b
        })
    }

    /// How highlights lie over the pages: translucent on dark paper, where darkest-wins would not show.
    fn wash(&self) -> Wash {
        match self.page_colors {
            Some(colors) if is_dark(colors.paper) => Wash::Translucent,
            _ => Wash::Darkest,
        }
    }

    /// Asks for a frame.
    pub fn invalidate(&self) {
        self.host.invalidate();
    }
}

/// Whether `[r, g, b]` is a dark colour: its luminance is below the middle.
fn is_dark([r, g, b]: [u8; 3]) -> bool {
    0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) < 128.0
}

/// Which way to zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomChange {
    /// A step closer.
    In,
    /// A step back.
    Out,
}

/// What a background thread sent on `slot`, once it has; the slot empties then, or when the thread died.
fn take_sent<T>(slot: &mut Option<Receiver<T>>, thread: &str) -> Option<T> {
    let received = slot.as_ref()?.try_recv();
    match received {
        Ok(value) => {
            *slot = None;
            Some(value)
        }
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            log::error!("the {thread} thread stopped");
            *slot = None;
            None
        }
    }
}

/// Every page's `page_box`, read without loading the pages where the format allows (K.6, item 1).
fn page_boxes(
    document: &excellent_mupdf::Document,
    page_box: PageBox,
) -> excellent_mupdf::Result<Vec<PageRect>> {
    (0..document.page_count()?)
        .map(|index| document.page_box(index, page_box))
        .collect()
}

/// Opens `path`, or takes it from the registry if it is open, lays a reflowable book out on `reflow`'s pages (R.28),
/// and reads every page's size and the reading direction (on a background thread).
// shortcut: two views of one book share its layout, so the last to lay it out wins; per-view layouts would need a
// MuPDF document each (V.1's split views).
fn open_document(path: &Path, page_box: PageBox, reflow: Reflow) -> document::Result<Opened> {
    let document = registry().open(path)?;
    document.reflow(reflow, None)?;
    let (bounds, right_to_left, labels, reflow) = {
        let mupdf = document.mupdf()?;
        let mupdf = mupdf.lock().expect("document lock poisoned");
        let reflow = mupdf.is_reflowable()?.then_some(reflow);
        let bounds = page_boxes(&mupdf, page_box)?;
        let right_to_left = mupdf.reads_right_to_left().unwrap_or_else(|err| {
            log::warn!("cannot read the reading direction: {err}");
            false
        });
        let labels = mupdf.page_labels().unwrap_or_else(|err| {
            log::warn!("cannot read the page labels: {err}");
            None
        });
        (bounds, right_to_left, labels, reflow)
    };
    Ok(Opened {
        document,
        bounds,
        right_to_left,
        labels,
        reflow,
    })
}
