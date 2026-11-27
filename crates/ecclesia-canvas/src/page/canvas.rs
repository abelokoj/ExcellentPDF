//! One canvas's state on its view's thread: its document, layout, viewport and tiles, and drawing a frame
//! (plan §7.2, §7.4), on any platform through its [`Host`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use crate::layout::{Axis, Direction, Layout, PageSize};
use crate::page::gestures::Verdict;
use crate::tiles::{self, TileCache, TileKey};
use crate::view::history::Place;
use crate::view::{Effect, View, ViewCommand};
use crate::viewport::{self, Side, Viewport, ZoomMode};
use ecclesia_core::darkmode::{PageColors, PageImages};
use ecclesia_core::document::{self, Document};
use ecclesia_core::formats;
use ecclesia_core::render::{Job, Note, Priority, Recolor};
use ecclesia_core::shared::{pool, registry};
use ecclesia_core::transform::Transform;
use ecclesia_ink::pens::{Blend, Pen};
use ecclesia_ink::predict::predict;
use ecclesia_ink::sample::{Sample, Tilt, Tool};
use ecclesia_ink::session::Session;
use ecclesia_ink::shapes;
use ecclesia_ink::stroke::InkPoint;
use ecclesia_ink::stroke::Stroke;
use ecclesia_mupdf::{
    DeviceRect, DisplayList, Link, PageBox, PagePoint, PageRect, Pixmap, Reflow, ReflowStyle,
    Target, TextLayout,
};
use ecclesia_text::{Granularity, PageText, Selection, TextPos};
use nsh_core::JsonValue;

use super::host::{Host, InkPixel, Quad as ScreenQuad, Wake, Wash};
use crate::page::clicks::{ClickLimits, Clicks};
use crate::page::near::Near;
use crate::page::placement::Placement;
use crate::page::saved::SavedView;

mod access;
mod block_select;
mod draw;
mod erase;
mod find;
mod guides;
mod margin;
mod markbar;
mod minimap;
mod reading;
mod refhover;
mod select;
mod zoompad;

pub(super) use markbar::NoteMode;
use refhover::PopoverPress;
pub(super) use refhover::{
    DEFAULT_DELAY as DEFAULT_HOVER_DELAY, DEFAULT_MAX_WIDTH_PCT as DEFAULT_HOVER_MAX_WIDTH_PCT,
};

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
/// How far ahead of the pen the preview draws (plan §8.2, step 3), in milliseconds: about one frame at 60 Hz.
const PREDICT_MS: f64 = 16.0;
/// The furthest the preview runs ahead of the pen, in pixels, so a sudden stop never overshoots far.
const PREDICT_MAX_PX: f32 = 24.0;
/// How many of the newest samples prediction looks at.
const RECENT_SAMPLES: usize = 8;
/// How far the pen may wander, in pixels, and still be held still (H.10): a hand resting the pen trembles.
const STILL_PX: f32 = 3.0;
/// How closely a finished stroke's curve is followed, in page points (plan §8.2, step 4); the pen says how much it is
/// steadied.
const SMOOTH_TOLERANCE: f32 = 0.1;
/// Set to a folder to record every finished stroke there as a pen session file (plan §16): sessions recorded on a
/// real device become the replay tests' fixtures.
const RECORD_INK: &str = "ECCLESIAPDF_RECORD_INK";
/// Selected text's highlight: a light blue that keeps black text black under it (drawn darkest-wins), or laid over
/// dark paper translucent ([`Wash`]).
const SELECTION: u32 = 0xb4d5fe;
/// Minimum custom zoom percentage supported by [`PageCanvas::set_zoom_percent`], matching SumatraPDF.
const MIN_ZOOM_PERCENT: f32 = 10.0;
/// Maximum custom zoom percentage supported by [`PageCanvas::set_zoom_percent`], matching SumatraPDF.
const MAX_ZOOM_PERCENT: f32 = 6400.0;
/// Share of the viewport width and height filled by [`PageCanvas::zoom_to_selection`].
const ZOOM_SELECTION_FILL: f32 = 0.85;

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
    /// A drawing tool is on (A.8): the crosshair.
    Cross,
    /// Horizontal edge resize (`ew-resize`).
    ResizeEw,
    /// Vertical edge resize (`ns-resize`).
    ResizeNs,
    /// Top-left / bottom-right corner resize (`nwse-resize`).
    ResizeNwse,
    /// Top-right / bottom-left corner resize (`nesw-resize`).
    ResizeNesw,
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
    /// How the pen leans; `None` when the pen doesn't say.
    pub tilt: Option<Tilt>,
    /// When the sample was taken, in milliseconds on the platform's input clock; `None` when the platform doesn't
    /// say, and the canvas times it as it arrives.
    pub time: Option<f64>,
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
    /// The style (document CSS + user CSS/font) applied to a reflowable book.
    reflow_style: ReflowStyle,
}

/// A book laid out again (R.28): its new pages, and the page that starts where the reader was.
struct Reflowed {
    reflow: Reflow,
    style: ReflowStyle,
    bounds: Vec<PageRect>,
    kept: Option<usize>,
}

/// An externally reloaded document's refreshed page metrics and the reader's view to restore.
struct Reloaded {
    bounds: Vec<PageRect>,
    labels: Option<Vec<String>>,
    reflow: Option<Reflow>,
    reflow_style: ReflowStyle,
    saved: Option<SavedView>,
}

/// A page's display list.
enum List {
    /// Being recorded from the page at this revision.
    Building(Job<DisplayList>, u64),
    /// Recorded from the page at this revision, with its pictures for smart dark mode (listed when a tile first
    /// needs them).
    Ready(Arc<DisplayList>, Arc<PageImages>, u64),
    Failed,
}

/// What a canvas shows and everything it needs to show it, drawn through its platform's `H`.
pub struct PageCanvas<H: Host> {
    host: H,
    waker: Arc<Waker>,
    path: Option<PathBuf>,
    opening: Option<Receiver<document::Result<Opened>>>,
    reloading: Option<Receiver<document::Result<Reloaded>>>,
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
    /// The erasers (H.5): the chosen one and a pass under way.
    erasers: erase::Erasers,
    /// Which touches count (H.8): none while the pen is about, none too wide for a finger.
    palms: crate::page::gestures::Palms,
    /// How strongly the paper's grain shows over the pages (R.23), 0 for none.
    paper_texture: f32,
    /// The grain, uploaded once it is wanted; dropped with the device.
    grain: Option<H::Image>,
    /// The ruler, protractor or compass on the page (H.11).
    guides: guides::Guides,
    /// The zoom writing pad, while it is on (H.12).
    pad: Option<zoompad::Pad>,
    /// The pen the next stroke draws with (H.2).
    ink_pen: Pen,
    /// Tokens of the points handed to the host's ink trail that it still draws, oldest first.
    trail: Vec<u32>,
    /// The page last reported as the reader's.
    reported_page: Option<usize>,
    /// The view last reported, for the app to save with the tab (R.9).
    reported_view: Option<SavedView>,
    /// The view moved to follow another pane (V.8) since it was last reported: the report says so, so the app
    /// doesn't send it back.
    following: bool,
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
    /// While the app shows its margin column (A.11): the pages last reported to it.
    margin: Option<margin::Margin>,
    /// A copy is waiting for the text of selected pages that aren't read yet.
    copy_pending: bool,
    /// The link under the mouse, as last reported.
    hovered: Option<Link>,
    /// The link the left button went down on: releasing over it follows it.
    pressed: Option<Link>,
    /// Whether the left button went down on the pages themselves, not on the bar, a popover, the minimap or a link:
    /// releasing it is a press on the page (A.4).
    on_page: bool,
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
    /// Desired styling (document CSS toggle + user CSS/font) for reflowable books.
    reflow_style: ReflowStyle,
    /// Styling last applied to the currently open reflowable book.
    applied_reflow_style: ReflowStyle,
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
    /// The markup bar under selected text (A.1).
    markbar: markbar::MarkBar,
    /// Rectangular block selection mode, active drag, and finalized regions.
    block_select: block_select::BlockSelectState,
    /// The drawing tool and the shape being drawn (A.8).
    drawing: draw::Drawing,
    /// The annotations the app has selected (A.13), and a drag on them.
    selected: select::Selected,
    /// The document's edit count the pages were drawn at: an edit draws the pages it changed again (A.1).
    edits: u64,
}

/// A stroke on a page, in that page's points: it stays on the page through any layout, zoom or rotation.
struct PageStroke {
    page: usize,
    stroke: Stroke,
    with: Pen,
}

/// A stroke being drawn, the pointer drawing it, and how long its samples took to reach the screen.
struct PenStroke {
    id: u32,
    page: usize,
    stroke: Stroke,
    /// The pen it draws with, as chosen when it started.
    with: Pen,
    latency: Latency,
    /// When it started, to time samples the platform doesn't.
    started: std::time::Instant,
    /// The newest samples in the view's pixels, for prediction.
    recent: Vec<Sample>,
    /// When the pen last moved (a little more than a hand's tremor): held still long enough before it lifts, the
    /// stroke becomes the shape it was meant to be (H.10).
    last_moved: std::time::Instant,
    /// Written in the zoom pad (H.12): its samples land in the pad's box on the page.
    in_pad: bool,
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
            reloading: None,
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
            ink_pen: Pen::default(),
            erasers: erase::Erasers::default(),
            palms: crate::page::gestures::Palms::default(),
            paper_texture: 0.0,
            grain: None,
            guides: guides::Guides::default(),
            pad: None,
            trail: Vec::new(),
            reported_page: None,
            reported_view: None,
            following: false,
            links: Near::new("links"),
            notes: Near::new("notes"),
            texts: Near::new("text"),
            selection: None,
            selecting: false,
            over_text: false,
            clicks: Clicks::default(),
            margin: None,
            copy_pending: false,
            hovered: None,
            pressed: None,
            on_page: false,
            opened_at: None,
            background: BACKGROUND,
            page_colors: None,
            reading: reading::Reading::default(),
            font_size: formats::DEFAULT_FONT_SIZE,
            reflow_style: ReflowStyle::default(),
            applied_reflow_style: ReflowStyle::default(),
            reflow: None,
            reflowing: None,
            reflow_again: false,
            finder: find::Finder::default(),
            minimap: minimap::MiniMap::default(),
            refhover: refhover::RefHover::default(),
            markbar: markbar::MarkBar::default(),
            block_select: block_select::BlockSelectState::default(),
            drawing: draw::Drawing::default(),
            selected: select::Selected::default(),
            edits: 0,
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
        let reflow_style = self.reflow_style.clone();
        self.opening = self.in_background("open-document", move || {
            open_document(&path, page_box, reflow, reflow_style)
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

    /// The reader's custom CSS and document-CSS preference for reflowable books (EPUB, FB2, MOBI, HTML, Markdown).
    pub fn set_reflow_style(&mut self, style: ReflowStyle) {
        if self.reflow_style == style {
            return;
        }
        self.reflow_style = style;
        self.reflow();
    }

    /// Lays the book shown out again for the window, text size, and stylesheet, on a background thread, keeping the
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
        let target_style = self.reflow_style.clone();
        if target == current && target_style == self.applied_reflow_style {
            return;
        }
        let keep = self.viewport.current_page(&self.layout);
        let page_box = self.view.page_box;
        self.reflowing = self.in_background("reflow", move || {
            let kept = document.reflow_with_style(target, &target_style, keep)?;
            let mupdf = document.mupdf()?;
            let bounds = page_boxes(&mupdf.lock().expect("document lock poisoned"), page_box)?;
            Ok(Reflowed {
                reflow: target,
                style: target_style,
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
            let mut json = view.to_json();
            json["follow"] = self.following.into();
            self.emit("view", json);
        }
        self.following = false;
    }

    /// Shows `anchor` to follow another pane (V.8), at `zoom` when there is one: no jump, so nothing goes on the
    /// history, and the next view report says it followed.
    pub fn follow_pane(&mut self, anchor: viewport::Anchor, zoom: Option<(ZoomMode, f32)>) {
        if anchor.page >= self.layout.len() {
            return;
        }
        if self.reflow.is_some() {
            self.viewport.scroll_to_page(anchor.page, &self.layout);
        } else {
            match zoom {
                Some((ZoomMode::Custom, zoom)) => self.viewport.set_zoom(zoom, &self.layout),
                Some((mode, _)) => self.viewport.fit(mode, &self.layout),
                None => {}
            }
            self.viewport.relayout(Some(anchor), &self.layout);
        }
        self.following = true;
        self.invalidate();
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

    /// A Ctrl+click (Command on a Mac) or middle-click at client pixel (`x`, `y`) (V.9). Over a link to a place in
    /// this document, the app hears where it goes (`openElsewhere`: `{page, x, y}`, the point in page points when the
    /// link names one) to show it in another pane, and this pane stays where it is; over any other link, it is
    /// followed as a click would. Returns whether a link was there: then the press does nothing else.
    pub fn open_elsewhere(&mut self, x: i32, y: i32) -> bool {
        let Some(link) = self.link_at(x, y).cloned() else {
            return false;
        };
        match link.target {
            Some(target) => {
                let mut data = serde_json::json!({ "page": target.page });
                if let Some(point) = target.point {
                    data["x"] = point.x.into();
                    data["y"] = point.y.into();
                }
                self.emit("openElsewhere", data);
            }
            None => self.follow(&link),
        }
        true
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
        } else if let Some(cursor) = self.popover_cursor() {
            cursor
        } else if self.is_drawing_tool() || self.is_block_selecting() {
            Cursor::Cross
        } else if self.over_selected() {
            Cursor::Pan
        } else if self.selecting || (self.over_text && self.hovered.is_none()) {
            Cursor::Text
        } else if self.hovered.is_some() {
            Cursor::Hand
        } else {
            Cursor::Arrow
        }
    }

    /// The pointer's shape at client pixel (`x`, `y`), checking live popover resize edges first.
    pub fn cursor_at(&self, x: i32, y: i32) -> Cursor {
        if self.reading.is_panning() {
            Cursor::Pan
        } else if let Some(cursor) = self.popover_cursor_at(x, y) {
            cursor
        } else {
            self.cursor()
        }
    }

    /// The mouse moved to client pixel (`x`, `y`), or left the canvas (`None`). While selecting, the selection
    /// follows it. Otherwise tells the app when the link under it changes (`linkHover`: `{uri, page}`, page `null`
    /// for a link out of the document; `null` for no link).
    pub fn mouse_moved(&mut self, at: Option<(i32, i32)>) {
        self.reading_mouse(at);
        if self.popover_moved(at)
            || self.guide_moved(at)
            || self.draw_moved(at)
            || self.select_moved(at)
            || self.block_select_moved(at)
        {
            return;
        }
        self.markbar_hover(at);
        if let Some((x, y)) = at
            && (self.minimap_drag(y)
                || self.over_minimap(x, y)
                || self.over_popover(x, y)
                || self.over_markbar(x, y))
        {
            // Over the mini-map (R.17), a hover preview (R.25) or the markup bar (A.1): no text or link under the
            // pointer.
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
        self.on_page = false;
        if self.markbar_down(x, y) {
            return false;
        }
        match self.popover_down(x, y) {
            PopoverPress::Ignored => {}
            PopoverPress::Navigated => return false,
            PopoverPress::StartedResize => return true,
        }
        if self.minimap_down(x, y)
            || self.guide_down(x, y)
            || self.draw_down(x, y, time, limits)
            || self.select_down(x, y)
        {
            return true;
        }
        let clicks = self.clicks.press(time, x, y, limits);
        self.pressed = self.link_at(x, y).cloned();
        if self.pressed.is_some() {
            return false;
        }
        if self.block_select_down(x, y, clicks) {
            return true;
        }
        let _ = self.clear_block_selection();
        self.on_page = true;
        let granularity = Granularity::from_clicks(clicks);
        let start = (clicks > 1 || self.is_over_text(x, y))
            .then(|| self.text_pos(x, y, granularity))
            .flatten();
        let had = self.selection.is_some();
        if start.is_none() && clicks == 1 && self.marquee_down(x, y) {
            // Away from text, a drag draws a marquee round annotations (A.13); a short one is still a click.
            self.on_page = false;
            self.markbar.caret = None;
            self.selection = None;
            if had {
                self.invalidate();
            }
            return true;
        }
        self.markbar.caret = None;
        self.selection = start.map(|at| Selection::new(at, granularity));
        self.selecting = start.is_some();
        if had || self.selecting {
            self.invalidate();
        }
        self.selecting
    }

    /// The left button came up at client pixel (`x`, `y`): a selection ends there; over the link it went down on,
    /// the link is followed. A link inside the document jumps; any other goes to the app (`openLink`: `{uri}`),
    /// which decides whether to open it. A plain click that did neither goes to the app as `pagePress` (A.4).
    /// Returns whether a selection ended, when the window lets the mouse go.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> bool {
        if self.popover_up()
            || self.minimap_up()
            || self.guide_up()
            || self.draw_up(x, y)
            || self.select_up(x, y)
            || self.block_select_up(x, y)
        {
            return true;
        }
        if self.selecting {
            self.selecting = false;
            if let Some(click) = self.selection.filter(|s| s.is_collapsed()) {
                self.selection = None;
                self.markbar_click(click);
                if std::mem::take(&mut self.on_page) {
                    self.page_press(x, y);
                }
            }
            if self.selection.is_some() {
                self.invalidate();
            }
            self.host.selection_changed();
            return true;
        }
        let Some(pressed) = self.pressed.take() else {
            if std::mem::take(&mut self.on_page) {
                self.page_press(x, y);
            }
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
        self.popover_cancel();
        self.minimap_up();
        self.draw_cancel();
        self.select_cancel();
        self.block_select_cancel();
        self.selecting = false;
        self.stop_pan();
    }

    /// Copies the selected text: the canvas sends it to the app (`copy`: `{text}`), which puts it on the clipboard.
    /// Pages of the selection whose text isn't read yet are waited for.
    pub fn copy(&mut self) {
        if self.selection.is_none() && self.has_block_selection() {
            let text = self.block_selection_text();
            if !text.is_empty() {
                self.emit("copy", serde_json::json!({ "text": text }));
            }
            return;
        }
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
        let len = {
            let Some(text) = self.texts.get(page).filter(|text| !text.is_empty()) else {
                return;
            };
            text.len()
        };
        self.clear_block_selection();
        self.selection = Some(Selection {
            anchor: TextPos { page, index: 0 },
            focus: TextPos { page, index: len },
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
        let Some((page, point, title)) = selected
            .or_else(|| {
                let first = self
                    .has_block_selection()
                    .then(|| self.block_regions().first().copied())
                    .flatten()?;
                let text = self.block_selection_text();
                let title = (!text.trim().is_empty()).then_some(text);
                Some((
                    first.page,
                    PagePoint {
                        x: first.rect.x0,
                        y: first.rect.y0,
                    },
                    title,
                ))
            })
            .or_else(|| {
                let (page, point) = self.view_top_left()?;
                Some((page, point, None))
            })
        else {
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
        self.reloading = None;
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
        self.markbar.pending = None;
        self.markbar.caret = None;
        self.clear_block_selection();
        self.clear_external_popup();
        self.hovered = None;
        self.pressed = None;
        self.on_page = false;
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

    /// Sets a custom zoom percentage (clamped to `10 %..=6400 %`, matching SumatraPDF's custom
    /// zoom bounds) about the viewport center. Non-finite or non-positive values are ignored.
    pub fn set_zoom_percent(&mut self, percent: f32) {
        if !percent.is_finite() || percent <= 0.0 {
            return;
        }
        let zoom = (percent.clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT) / 100.0)
            .clamp(viewport::MIN_ZOOM, viewport::MAX_ZOOM);
        self.viewport.set_zoom(zoom, &self.layout);
        self.invalidate();
    }

    /// Cycles fit modes (`FitPage` -> `FitWidth` -> `FitHeight` -> `FitPage`, and `Custom` ->
    /// `FitPage`), matching SumatraPDF's `z` (`CmdToggleZoom`).
    pub fn toggle_zoom(&mut self) {
        let next = match self.viewport.zoom_mode() {
            ZoomMode::FitPage => ZoomMode::FitWidth,
            ZoomMode::FitWidth => ZoomMode::FitHeight,
            ZoomMode::FitHeight | ZoomMode::Custom => ZoomMode::FitPage,
        };
        self.view_command(ViewCommand::Fit(next));
    }

    /// Zooms to fit and center the active text or rectangular block selection (SumatraPDF's
    /// `Ctrl+4`, `CmdZoomToSelection`), falling back to [`ZoomMode::FitWidth`] when no
    /// non-empty selection is available.
    pub fn zoom_to_selection(&mut self) {
        let Some((page, rect)) = self.selection_page_rect() else {
            self.view_command(ViewCommand::Fit(ZoomMode::FitWidth));
            return;
        };
        let (view_w, view_h) = (self.viewport.width(), self.viewport.height());
        let ppp = self.viewport.pixels_per_point();
        let (Some(placement), Some(page_rect)) = (
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page),
            self.layout.page(page),
        ) else {
            self.view_command(ViewCommand::Fit(ZoomMode::FitWidth));
            return;
        };
        if !view_w.is_finite()
            || !view_h.is_finite()
            || !ppp.is_finite()
            || view_w <= 0.0
            || view_h <= 0.0
            || ppp <= 0.0
        {
            self.view_command(ViewCommand::Fit(ZoomMode::FitWidth));
            return;
        }
        let corners = [
            PagePoint {
                x: rect.x0,
                y: rect.y0,
            },
            PagePoint {
                x: rect.x1,
                y: rect.y0,
            },
            PagePoint {
                x: rect.x0,
                y: rect.y1,
            },
            PagePoint {
                x: rect.x1,
                y: rect.y1,
            },
        ];
        let mut lx0 = f32::INFINITY;
        let mut ly0 = f32::INFINITY;
        let mut lx1 = f32::NEG_INFINITY;
        let mut ly1 = f32::NEG_INFINITY;
        for corner in corners {
            let (x, y) = placement.to_layout(corner);
            lx0 = lx0.min(x);
            ly0 = ly0.min(y);
            lx1 = lx1.max(x);
            ly1 = ly1.max(y);
        }
        let sel_w = lx1 - lx0;
        let sel_h = ly1 - ly0;
        if !sel_w.is_finite() || !sel_h.is_finite() || sel_w <= 0.0 || sel_h <= 0.0 {
            self.view_command(ViewCommand::Fit(ZoomMode::FitWidth));
            return;
        }
        let zoom_x = (view_w * ZOOM_SELECTION_FILL) / (sel_w * ppp);
        let zoom_y = (view_h * ZOOM_SELECTION_FILL) / (sel_h * ppp);
        let target_zoom = zoom_x
            .min(zoom_y)
            .clamp(viewport::MIN_ZOOM, viewport::MAX_ZOOM);
        self.viewport.set_zoom(target_zoom, &self.layout);
        let scale = self.viewport.scale();
        let shown_w = view_w / scale;
        let shown_h = view_h / scale;
        let target_x = (lx0 + lx1) / 2.0 - shown_w / 2.0;
        let target_y = (ly0 + ly1) / 2.0 - shown_h / 2.0;
        let across = if page_rect.width() > 0.0 {
            (target_x - page_rect.x0) / page_rect.width()
        } else {
            0.0
        };
        let down = if page_rect.height() > 0.0 {
            (target_y - page_rect.y0) / page_rect.height()
        } else {
            0.0
        };
        self.viewport
            .relayout(Some(viewport::Anchor { page, across, down }), &self.layout);
        self.invalidate();
    }

    /// Returns the union bounding box (in page points) on the first selected page from either an
    /// active non-collapsed text selection or an active block selection.
    fn selection_page_rect(&self) -> Option<(usize, PageRect)> {
        let text_rect = self
            .selection
            .filter(|selection| !selection.is_collapsed())
            .and_then(|selection| {
                selection.pages().find_map(|page| {
                    let text = self.texts.get(page)?;
                    let range = selection.range_on(page, text)?;
                    let rect = union_quad_bounds(&text.highlight(range))?;
                    (rect.x1 > rect.x0 && rect.y1 > rect.y0).then_some((page, rect))
                })
            });
        text_rect.or_else(|| {
            let first = self
                .has_block_selection()
                .then(|| self.block_regions().first().copied())
                .flatten()?;
            (first.rect.x1 > first.rect.x0 && first.rect.y1 > first.rect.y0)
                .then_some((first.page, first.rect))
        })
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

    /// The pen touched down. Returns whether the canvas takes it: only over a document's page. With the eraser end,
    /// or an eraser chosen, it erases (H.5).
    pub fn pen_down(&mut self, sample: &PenSample) -> bool {
        if self.document.is_none() {
            return false;
        }
        if sample.eraser || self.erasing_chosen() {
            return self.erase_down(sample);
        }
        // The lasso (H.6) is drawn the way the mouse draws it.
        if self.lasso_on() {
            return false;
        }
        // Ink belongs to a page: a stroke starts only on one, or in the zoom pad, which writes on its page (H.12).
        let in_pad = self.pad_page(sample);
        let (x, y) = self.viewport.to_layout_point(sample.x, sample.y);
        let Some(page) = in_pad.or_else(|| self.layout.page_at(x, y)) else {
            return false;
        };
        self.pen = Some(PenStroke {
            id: sample.id,
            page,
            stroke: Stroke::new(),
            with: self.ink_pen,
            latency: Latency::default(),
            started: std::time::Instant::now(),
            recent: Vec::with_capacity(RECENT_SAMPLES + 1),
            last_moved: std::time::Instant::now(),
            in_pad: in_pad.is_some(),
        });
        self.trail.clear();
        // The system's trail draws opaque: a highlighter's stroke is drawn by our frames alone, so text shows through.
        if self.ink_pen.blend == Blend::Normal {
            self.host.trail_start(self.ink_pen.color);
        }
        self.pen_moved(std::slice::from_ref(sample));
        true
    }

    /// The pen moved through `samples`, oldest first. The host's ink trail, if it has one, gets them at once; our
    /// frame with them is drawn now if the host takes one without waiting, else as soon as it does.
    pub fn pen_moved(&mut self, samples: &[PenSample]) {
        if self.erase_moved(samples) {
            return;
        }
        let Some(page) = self.pen.as_ref().map(|pen| pen.page) else {
            return;
        };
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return;
        };
        // In the zoom pad, samples land in its box (H.12); worked out before the stroke is taken to write.
        let in_pad = self.pen.as_ref().is_some_and(|pen| pen.in_pad);
        let pad_points: Vec<Option<PagePoint>> = samples
            .iter()
            .map(|s| in_pad.then(|| self.pad_point(s.x, s.y)).flatten())
            .collect();
        let Some(pen) = &mut self.pen else {
            return;
        };
        let scale = self.viewport.scale();
        let mut on_screen = Vec::with_capacity(samples.len());
        let mut newest = None;
        for (sample, pad_point) in samples
            .iter()
            .zip(pad_points)
            .filter(|(s, _)| s.id == pen.id && s.contact)
        {
            let (sx, sy) = (sample.x, sample.y);
            let (x, y) = self.viewport.to_layout_point(sx, sy);
            let on_page = pad_point.unwrap_or_else(|| placement.to_page(x, y));
            let time = sample
                .time
                .unwrap_or_else(|| pen.started.elapsed().as_secs_f64() * 1000.0);
            let ink = Sample {
                x: on_page.x,
                y: on_page.y,
                pressure: sample.pressure,
                tilt: sample.tilt,
                time,
                tool: Tool::Pen,
            };
            pen.with.push(&mut pen.stroke, ink);
            let width = pen.stroke.points().last().map_or(0.0, |point| point.width);
            on_screen.push((sx, sy, width * scale));
            if pen
                .recent
                .last()
                .is_none_or(|last| (last.x - sx).hypot(last.y - sy) >= STILL_PX)
            {
                pen.last_moved = std::time::Instant::now();
            }
            pen.recent.push(Sample {
                x: sx,
                y: sy,
                ..ink
            });
            newest = Some(sample.input);
        }
        let stale = pen.recent.len().saturating_sub(RECENT_SAMPLES);
        pen.recent.drain(..stale);
        let Some(input) = newest else {
            return;
        };
        let trailed = self
            .pen
            .as_ref()
            .is_some_and(|pen| pen.with.blend == Blend::Normal);
        if let Some(token) = trailed.then(|| self.host.trail_add(&on_screen)).flatten() {
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
        if self.erase_up() {
            return;
        }
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
            let held = pen.last_moved.elapsed().as_secs_f64() * 1000.0 >= shapes::HOLD_MS;
            let finished = finished_points(&pen, held);
            // Along a guide, the stroke is drawn true on it (H.11).
            let finished = self.guided(pen.page, &finished).unwrap_or(finished);
            // From the zoom pad, it comes down to the page's scale and the box steps on (H.12).
            let finished = if pen.in_pad {
                self.pad_done(finished, pen.with.color)
            } else {
                finished
            };
            self.commit_ink(&pen, &finished);
            // Shown until the annotation it becomes is drawn with its page.
            let mut shown = Stroke::new();
            for point in finished {
                shown.push(point);
            }
            self.ink.push(PageStroke {
                page: pen.page,
                stroke: shown,
                with: pen.with,
            });
        }
        self.invalidate();
    }

    /// Sends the app the finished stroke (`ink`: `{page, points: [[x, y, width]], color, opacity}`, its points
    /// smoothed as its pen does, in page points), to commit as one step of undo (plan §8.2, step 6), and records it
    /// when asked to.
    fn commit_ink(&self, pen: &PenStroke, finished: &[InkPoint]) {
        let PenStroke {
            page, stroke, with, ..
        } = pen;
        let points: Vec<[f32; 3]> = finished
            .iter()
            .map(|point| [point.x, point.y, point.width])
            .collect();
        let channel = |shift: u32| f32::from(((with.color >> shift) & 0xff) as u8) / 255.0;
        self.emit(
            "ink",
            serde_json::json!({
                "page": page,
                "points": points,
                "color": [channel(16), channel(8), channel(0)],
                "opacity": with.opacity,
                "blend": match with.blend {
                    Blend::Normal => "normal",
                    Blend::Multiply => "multiply",
                },
            }),
        );
        if let Some(dir) = std::env::var_os(RECORD_INK) {
            record_ink(std::path::PathBuf::from(dir), stroke.samples().to_vec());
        }
    }

    /// The pen is near the screen or on it, at `time` (the platform's input clock, milliseconds): touches are a
    /// resting hand until it has been away a while (H.8).
    pub fn pen_near(&mut self, time: f64) {
        self.palms.pen_near(time);
    }

    /// The pen left the screen's range at `time` (H.8).
    pub fn pen_gone(&mut self, time: f64) {
        self.palms.pen_gone(time);
    }

    /// Touch `id` came down at `time`, `width` millimetres across: whether it is ignored, passes to the platform, or
    /// draws (H.8). With no document every touch passes.
    pub fn touch_down(&mut self, id: u32, time: f64, width: Option<f32>) -> Verdict {
        if self.document.is_none() {
            return Verdict::Pass;
        }
        self.palms.touch_down(id, time, width)
    }

    /// Touch `id`'s verdict while it is down.
    pub fn touch_verdict(&self, id: u32) -> Verdict {
        self.palms.verdict(id)
    }

    /// Touch `id` lifted.
    pub fn touch_up(&mut self, id: u32) {
        self.palms.touch_up(id);
    }

    /// Whether a finger that counts draws ink ("finger draws", the `fingerDraws` prop) or pans and zooms (H.8).
    pub fn set_finger_draws(&mut self, on: bool) {
        self.palms.set_finger_draws(on);
    }

    /// The pen the next stroke draws with (H.2); a stroke under way keeps its own.
    pub fn set_pen(&mut self, pen: Pen) {
        self.ink_pen = pen;
    }

    /// Whether pointer `id` is drawing a stroke.
    pub fn is_drawing(&self, id: u32) -> bool {
        self.pen.as_ref().is_some_and(|pen| pen.id == id) || self.is_erasing(id)
    }

    /// A worker finished something: take what is done.
    pub fn jobs_done(&mut self) {
        self.waker.posted.store(false, Ordering::Release);
        let mut changed = self.take_opened();
        changed |= self.take_reloaded();
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
            if let Some(tool) = self.markbar.pending {
                self.use_tool(tool);
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
                    *list = List::Ready(ready, Arc::default(), *revision);
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
        changed |= self.upload_ready_tiles();
        if changed {
            self.invalidate();
        }
    }

    /// Uploads every finished tile job while the host can upload. Returns whether any tile slot changed.
    fn upload_ready_tiles(&mut self) -> bool {
        if !self.host.can_upload() {
            return false;
        }
        let host = &mut self.host;
        self.tiles.poll(|job| {
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
        })
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
                self.applied_reflow_style = opened.reflow_style;
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
                self.applied_reflow_style = reflowed.style;
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
        let could_upload = self.host.can_upload();
        if let Err(err) = self.host.begin(width, height) {
            // No frame until something asks for one again: a device that can't be made now won't be in a loop.
            log::error!("cannot draw the page canvas: {err}");
            self.lose_device();
            return;
        }
        let moving = self.advance_motion();
        self.notice_edits();
        // Defer full-document annotation scanning until the first page has drawn so visible pages
        // never wait behind a background scan on huge PDFs.
        if self.opened_at.is_none() {
            self.update_marks();
        }
        self.upload_grain();
        let needed = tiles::visible_tiles(&self.layout, &self.viewport);
        self.request(&needed);
        self.tiles.begin_frame();
        // The mini-map (R.17) over everything, before the frame ends.
        let drawn = self
            .draw(&needed)
            .and_then(|()| self.draw_block_selection())
            .and_then(|()| self.draw_shape_preview())
            .and_then(|()| self.draw_selected())
            .and_then(|()| self.draw_minimap())
            .and_then(|()| self.draw_markbar())
            .and_then(|()| self.draw_popover())
            .and_then(|()| self.host.end());
        if let Err(err) = drawn {
            log::warn!("the page canvas lost its device ({err}); starting over");
            self.lose_device();
            self.invalidate();
        } else if !could_upload && self.upload_ready_tiles() {
            // Tiles that finished before the host created its graphics device were skipped in `jobs_done`; upload
            // them now that the device exists and draw them on the next frame.
            self.invalidate();
        }
        self.tiles.evict();
        self.report_page();
        // Every frame, moving or not: the cards follow the pages as they scroll.
        self.report_margin();
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
        // shortcut: lists are kept by distance from the screen, not by size; K.7 budgets them.
        self.lists
            .retain(|page, _| *page + LIST_REACH >= first && *page <= last + LIST_REACH);
        let wanted: HashSet<TileKey> = needed.iter().copied().collect();
        self.tiles.cancel_unneeded(&wanted);
        // Schedule visible display lists and tiles FIRST so nothing locks MuPDF ahead of Page 0.
        let mut visible_ready = true;
        for key in needed {
            if self.tiles.contains(key) {
                continue;
            }
            let list = self.lists.entry(key.page).or_insert_with(|| {
                // Another view of the document may have recorded it.
                if let Some(list) = document.display_list(key.page) {
                    return List::Ready(list, Arc::default(), document.revision(key.page));
                }
                let revision = document.revision(key.page);
                let job = pool().build_display_list(
                    &mupdf,
                    key.page,
                    document.hidden_annotations(),
                    Priority::Visible,
                );
                let waker = Arc::clone(waker);
                job.when_done(move || waker.wake());
                List::Building(job, revision)
            });
            let List::Ready(list, images, _) = list else {
                visible_ready = false;
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
            let waker = Arc::clone(waker);
            job.when_done(move || waker.wake());
            self.tiles.request(*key, job);
        }
        // Only queue auxiliary per-page MuPDF metadata reads (links, notes, text) once visible
        // display lists are built so they never contend with first-page display list recording.
        if !visible_ready {
            return;
        }
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
    }

    /// Draws the pages: paper first, then any older zoom's tiles while this zoom's render, then this zoom's; then the
    /// selection, find's hits, the reading aids and the ink. The frame is begun and ended by [`Self::paint`].
    fn draw(&mut self, needed: &[TileKey]) -> Result<(), H::Error> {
        let aids = self.aids();
        let found = self.found_on_screen();
        let (paper, wash) = (self.paper(), self.wash());
        let swath = self.eraser_swath();
        let guide = self.guide_shapes();
        let pad = self.pad_shapes();
        let selected = self.selection_quads();
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
            // The paper's grain over the page, one picture stretched with it, never in its tiles (R.23).
            if let Some(grain) = self.grain.as_ref().filter(|_| self.paper_texture > 0.0) {
                host.faint_image(grain, &on_screen, self.paper_texture);
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
        if !selected.is_empty() {
            host.highlight(&selected, SELECTION, wash)?;
        }
        found.draw(host, wash)?;
        aids.draw_under(host)?;
        let scale = self.viewport.scale();
        // The pen's stroke runs on to where the pen is about to be; the next frame replaces that with what it did.
        let ahead = self
            .pen
            .as_ref()
            .and_then(|pen| predict(&pen.recent, PREDICT_MS, PREDICT_MAX_PX));
        let strokes = self
            .ink
            .iter()
            .map(|ink| (ink.page, &ink.stroke, ink.with, None))
            .chain(
                self.pen
                    .as_ref()
                    .map(|pen| (pen.page, &pen.stroke, pen.with, ahead)),
            );
        for (page, stroke, with, ahead) in strokes {
            let Some(placement) =
                Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
            else {
                continue;
            };
            let mut points: Vec<InkPixel> = stroke
                .points()
                .iter()
                .map(|p| {
                    let (x, y) = placement.to_layout(PagePoint { x: p.x, y: p.y });
                    let (x, y) = self.viewport.to_screen_point(x, y);
                    (x, y, p.width * scale)
                })
                .collect();
            if let (Some((x, y)), Some(&(_, _, width))) = (ahead, points.last()) {
                points.push((x, y, width));
            }
            match with.blend {
                Blend::Normal => host.polyline(&points, with.color)?,
                // Darkest wins, as a highlight does: text under the stroke stays readable while it is drawn.
                Blend::Multiply => host.highlight(&segment_quads(&points), with.color, wash)?,
            }
        }
        if let Some((quads, color)) = &swath {
            host.highlight(quads, *color, wash)?;
        }
        Self::draw_guide(host, guide)?;
        Self::draw_pad(host, pad)?;
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

    /// The selected text's lines on the pages on the screen, one quad per line, in client pixels.
    fn selection_quads(&self) -> Vec<ScreenQuad> {
        let Some(selection) = self.selection else {
            return Vec::new();
        };
        let mut quads = Vec::new();
        for (index, _) in self.layout.pages_in(&self.viewport.visible()) {
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
        quads
    }

    /// Draws again the pages an edit, an undo or a redo changed (A.1): their display lists are
    /// rebuilt and their existing tiles are marked stale so previous pixels stay visible on screen
    /// until the updated tiles arrive (zero blank-page blink).
    fn notice_edits(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let edits = document.edits();
        if edits == self.edits {
            return;
        }
        self.edits = edits;
        let stale: Vec<usize> = self
            .lists
            .iter()
            .filter(|(page, list)| match list {
                List::Building(_, revision) | List::Ready(_, _, revision) => {
                    *revision != document.revision(**page)
                }
                List::Failed => true,
            })
            .map(|(page, _)| *page)
            .collect();
        // A finished stroke on a changed page is now its annotation, drawn with the page.
        self.ink.retain(|ink| !stale.contains(&ink.page));
        for page in stale {
            self.lists.remove(&page);
            self.tiles.mark_page_stale(page);
        }
    }

    /// The document was edited (the app's `edits` prop): the pages it changed are drawn again.
    pub fn edited(&mut self) {
        self.invalidate();
    }

    /// Re-reads page bounds and labels from the already-reloaded document in place, clears stale
    /// tiles and cached page metadata, and preserves the reader's current page, scroll offset,
    /// zoom, layout, and rotation (clamping the page index if the reloaded file has fewer pages).
    pub fn reload(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let saved = self.place().map(|place| SavedView {
            place,
            spec: self.view.spec,
            rotation: self.view.rotation,
        });
        let page_box = self.view.page_box;
        let reflow = self.reflow_size();
        let reflow_style = self.reflow_style.clone();
        self.reloading = self.in_background("reload-document", move || {
            document.reflow_with_style(reflow, &reflow_style, None)?;
            let mupdf = document.mupdf()?;
            let mupdf = mupdf.lock().expect("document lock poisoned");
            let reflow = mupdf.is_reflowable()?.then_some(reflow);
            let bounds = page_boxes(&mupdf, page_box)?;
            let labels = mupdf.page_labels().unwrap_or_else(|err| {
                log::warn!("cannot read the page labels on reload: {err}");
                None
            });
            Ok(Reloaded {
                bounds,
                labels,
                reflow,
                reflow_style,
                saved,
            })
        });
    }

    /// Applies refreshed page bounds/labels from an external reload while preserving view state.
    fn take_reloaded(&mut self) -> bool {
        let Some(result) = take_sent(&mut self.reloading, "reload-document") else {
            return false;
        };
        match result {
            Ok(reloaded) => {
                self.bounds = reloaded.bounds;
                self.reflow = reloaded.reflow;
                self.applied_reflow_style = reloaded.reflow_style;
                self.forget_pages();
                if let Some(document) = &self.document {
                    self.edits = document.edits();
                }
                let (w, h) = self.viewport_size();
                self.view.resized(w);
                self.layout = Layout::new(&self.shown_sizes(), self.view.spec);
                self.viewport.resize(w, h, &self.layout);
                self.report_pages(reloaded.labels);
                if !self.layout.is_empty() {
                    match reloaded.saved {
                        Some(mut saved) => {
                            let max_page = self.layout.len().saturating_sub(1);
                            saved.place.anchor.page = saved.place.anchor.page.min(max_page);
                            self.set_view(saved);
                        }
                        None => {
                            self.viewport.relayout(None, &self.layout);
                        }
                    }
                }
            }
            Err(err) => log::warn!("cannot reload {:?}: {err}", self.path),
        }
        true
    }

    /// The device is gone: its images go with it; the next frame makes a new one.
    fn lose_device(&mut self) {
        self.host.lose_device();
        self.grain = None;
        self.tiles.clear();
        self.refhover.forget();
    }

    /// How strongly the paper's grain shows over the pages (R.23): 0 for none, up to 1 (the `paperTexture` prop).
    pub fn set_paper_texture(&mut self, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        if strength != self.paper_texture {
            self.paper_texture = strength;
            self.invalidate();
        }
    }

    /// Uploads the grain when it is wanted and isn't there yet (first use, or a new device).
    fn upload_grain(&mut self) {
        if self.paper_texture <= 0.0 || self.grain.is_some() || !self.host.can_upload() {
            return;
        }
        match ecclesia_core::darkmode::paper::grain_pixmap() {
            Ok(pixmap) => self.grain = self.host.upload(&pixmap).map(|(image, _)| image),
            Err(err) => log::warn!("paper texture: {err}"),
        }
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

/// Union bounding box (in page points) covering all `quads`, or `None` when empty.
fn union_quad_bounds(quads: &[ecclesia_mupdf::Quad]) -> Option<PageRect> {
    let mut points = quads.iter().flat_map(|q| [q.ul, q.ur, q.ll, q.lr]);
    let first = points.next()?;
    let mut rect = PageRect {
        x0: first.x,
        y0: first.y,
        x1: first.x,
        y1: first.y,
    };
    for p in points {
        rect.x0 = rect.x0.min(p.x);
        rect.y0 = rect.y0.min(p.y);
        rect.x1 = rect.x1.max(p.x);
        rect.y1 = rect.y1.max(p.y);
    }
    Some(rect)
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
    document: &ecclesia_mupdf::Document,
    page_box: PageBox,
) -> ecclesia_mupdf::Result<Vec<PageRect>> {
    (0..document.page_count()?)
        .map(|index| document.page_box(index, page_box))
        .collect()
}

/// Opens `path`, or takes it from the registry if it is open, lays a reflowable book out on `reflow`'s pages (R.28),
/// and reads every page's size and the reading direction (on a background thread).
// shortcut: two views of one book share its layout, so the last to lay it out wins; per-view layouts would need a
// MuPDF document each (V.1's split views).
fn open_document(
    path: &Path,
    page_box: PageBox,
    reflow: Reflow,
    reflow_style: ReflowStyle,
) -> document::Result<Opened> {
    let document = registry().open(path)?;
    document.reflow_with_style(reflow, &reflow_style, None)?;
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
        reflow_style,
    })
}

/// Writes `samples`, a finished stroke's, to a new pen session file in `dir`, off the UI thread (plan §16).
fn record_ink(dir: std::path::PathBuf, samples: Vec<Sample>) {
    std::thread::spawn(move || {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let path = dir.join(format!("ink-{stamp}.session"));
        let session = Session {
            strokes: vec![samples],
        };
        if let Err(err) = std::fs::write(&path, session.to_text()) {
            log::warn!("ink: cannot record {}: {err}", path.display());
        }
    });
}

/// The points `pen`'s stroke is saved with: smoothed as its pen does, or, when the pen was `held` still at its end
/// and the stroke is a shape drawn by hand, that shape drawn true at the stroke's mean width (H.10).
fn finished_points(pen: &PenStroke, held: bool) -> Vec<InkPoint> {
    let smoothed = pen.with.finish(&pen.stroke, SMOOTH_TOLERANCE);
    if !held {
        return smoothed;
    }
    let outline: Vec<ecclesia_ink::erase::Point> = smoothed
        .iter()
        .map(|p| ecclesia_ink::erase::Point { x: p.x, y: p.y })
        .collect();
    let Some(shape) = shapes::recognize(&outline) else {
        return smoothed;
    };
    let width = smoothed.iter().map(|p| p.width).sum::<f32>() / smoothed.len().max(1) as f32;
    log::debug!("ink: held still, drawn as a {:?}", shape.kind());
    shape
        .path()
        .into_iter()
        .map(|p| InkPoint {
            x: p.x,
            y: p.y,
            width,
        })
        .collect()
}

/// A stroke through `points` as quads, one per segment, each as wide as its first point (a lone point is a square): a
/// highlighter's chisel, drawn darkest-wins so overlapping quads don't darken twice.
fn segment_quads(points: &[InkPixel]) -> Vec<ScreenQuad> {
    let quad = |(x0, y0, w): InkPixel, (x1, y1): (f32, f32)| {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let length = dx.hypot(dy);
        let half = w / 2.0;
        let (ux, uy) = if length > 0.0 {
            (dx / length * half, dy / length * half)
        } else {
            (half, 0.0)
        };
        // Across the segment, and half a width past each end, so joins have no gaps.
        let (nx, ny) = (-uy, ux);
        [
            (x0 - ux + nx, y0 - uy + ny),
            (x1 + ux + nx, y1 + uy + ny),
            (x1 + ux - nx, y1 + uy - ny),
            (x0 - ux - nx, y0 - uy - ny),
        ]
    };
    match points {
        [] => Vec::new(),
        [only] => vec![quad(*only, (only.0, only.1))],
        _ => points
            .windows(2)
            .map(|pair| quad(pair[0], (pair[1].0, pair[1].1)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use super::{PageCanvas, ZoomChange, segment_quads, union_quad_bounds};
    use crate::Rect;
    use crate::layout::{Layout, PageSize, Spec};
    use crate::page::clicks::ClickLimits;
    use crate::page::host::{Host, InkPixel, Quad, Wake, Wash};
    use crate::page::placement::Placement;
    use crate::viewport::{MAX_ZOOM, MIN_ZOOM, ZoomMode};
    use ecclesia_core::document::Document;
    use ecclesia_core::render::Priority;
    use ecclesia_core::shared::pool;
    use ecclesia_mupdf::{PageBox, PagePoint, PageRect, Pixmap, Quad as PageQuad};
    use ecclesia_text::{Granularity, Selection, TextPos};
    use nsh_core::JsonValue;

    /// A test host that records uploaded tile pixels and every draw call, and starts without a
    /// graphics device until `begin()` is called (matching `WinHost`).
    struct RecordingHost {
        size: (f32, f32),
        device_ready: bool,
        wake_tx: mpsc::Sender<()>,
        draws: Vec<(Rect, usize)>,
    }

    struct TileImage {
        non_white_pixels: usize,
    }

    impl RecordingHost {
        fn new(width: f32, height: f32) -> (Self, mpsc::Receiver<()>) {
            let (wake_tx, wake_rx) = mpsc::channel();
            (
                Self {
                    size: (width, height),
                    device_ready: false,
                    wake_tx,
                    draws: Vec::new(),
                },
                wake_rx,
            )
        }
    }

    impl Host for RecordingHost {
        type Image = TileImage;
        type Error = &'static str;

        fn size(&self) -> (f32, f32) {
            self.size
        }

        fn invalidate(&self) {}

        fn emit(&self, _name: &str, _data: JsonValue) {}

        fn waker(&self) -> Wake {
            let tx = self.wake_tx.clone();
            Box::new(move || {
                let _ = tx.send(());
            })
        }

        fn can_upload(&self) -> bool {
            self.device_ready
        }

        fn upload(&mut self, pixmap: &Pixmap) -> Option<(Self::Image, usize)> {
            let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
            let n = pixmap.components();
            let stride = pixmap.stride();
            let mut non_white = 0usize;
            for row in pixmap.samples().chunks_exact(stride).take(h) {
                for px in row[..w * n].chunks_exact(n) {
                    if px[0] < 245 || px[1] < 245 || px[2] < 245 {
                        non_white += 1;
                    }
                }
            }
            Some((
                TileImage {
                    non_white_pixels: non_white,
                },
                w * h * 4,
            ))
        }

        fn begin(&mut self, _width: f32, _height: f32) -> Result<(), Self::Error> {
            self.device_ready = true;
            self.draws.clear();
            Ok(())
        }

        fn end(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn resize(&mut self, width: u32, height: u32) -> Result<(), Self::Error> {
            self.size = (width as f32, height as f32);
            Ok(())
        }

        fn lose_device(&mut self) {
            self.device_ready = false;
        }

        fn clear(&mut self, _rgb: u32) {}

        fn fill(&mut self, _rect: &Rect, _rgb: u32) -> Result<(), Self::Error> {
            Ok(())
        }

        fn image(&mut self, image: &Self::Image, dest: &Rect) {
            self.draws.push((*dest, image.non_white_pixels));
        }

        fn faint_image(&mut self, _image: &Self::Image, _dest: &Rect, _opacity: f32) {}

        fn highlight(
            &mut self,
            _quads: &[Quad],
            _rgb: u32,
            _wash: Wash,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn band(
            &mut self,
            _rect: &Rect,
            _radius: f32,
            _rgb: u32,
            _wash: Wash,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn dim_outside(
            &mut self,
            _screen: &Rect,
            _hole: &Rect,
            _radius: f32,
            _rgb: u32,
            _alpha: f32,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn polyline(&mut self, _points: &[InkPixel], _rgb: u32) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn pump_until_drawn(
        canvas: &mut PageCanvas<RecordingHost>,
        wake_rx: &mpsc::Receiver<()>,
        timeout: Duration,
    ) -> (usize, usize) {
        let deadline = Instant::now() + timeout;
        canvas.paint();
        while Instant::now() < deadline {
            let _ = wake_rx.recv_timeout(Duration::from_millis(25));
            while wake_rx.try_recv().is_ok() {}
            canvas.jobs_done();
            canvas.paint();
            let needed = crate::tiles::visible_tiles(&canvas.layout, &canvas.viewport);
            let all_ready =
                !needed.is_empty() && needed.iter().all(|k| canvas.tiles.ready(k).is_some());
            let tiles = canvas.host().draws.len();
            let ink_pixels: usize = canvas.host().draws.iter().map(|(_, n)| *n).sum();
            if all_ready && tiles > 0 && ink_pixels > 0 {
                return (tiles, ink_pixels);
            }
        }
        let tiles = canvas.host().draws.len();
        let ink_pixels: usize = canvas.host().draws.iter().map(|(_, n)| *n).sum();
        (tiles, ink_pixels)
    }

    fn synthetic_canvas(
        width: f32,
        height: f32,
    ) -> (PageCanvas<RecordingHost>, mpsc::Receiver<()>) {
        let (host, wake_rx) = RecordingHost::new(width, height);
        let mut canvas = PageCanvas::new(host, 1.0);
        let page = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 600.0,
            y1: 800.0,
        };
        canvas.bounds = vec![page, page];
        canvas.layout = Layout::new(
            &[
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
            ],
            Spec::default(),
        );
        canvas.viewport.resize(width, height, &canvas.layout);
        canvas.viewport.set_zoom(1.0, &canvas.layout);
        (canvas, wake_rx)
    }

    #[test]
    fn toggle_zoom_cycles_fit_modes_and_from_custom() {
        let (mut canvas, _wake_rx) = synthetic_canvas(800.0, 1000.0);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);

        canvas.toggle_zoom();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitPage);

        canvas.toggle_zoom();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitWidth);

        canvas.toggle_zoom();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitHeight);

        canvas.toggle_zoom();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitPage);

        canvas.command("toggleZoom", &JsonValue::Null);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitWidth);
    }

    #[test]
    fn set_zoom_percent_clamps_applies_and_ignores_invalid_values() {
        let (mut canvas, _wake_rx) = synthetic_canvas(800.0, 1000.0);

        canvas.set_zoom_percent(150.0);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);
        assert!((canvas.viewport.zoom() - 1.5).abs() < 1e-4);

        canvas.set_zoom_percent(2.0);
        assert!((canvas.viewport.zoom() - 0.1).abs() < 1e-4);

        canvas.set_zoom_percent(10_000.0);
        assert!((canvas.viewport.zoom() - MAX_ZOOM).abs() < 1e-4);

        for invalid in [0.0, -50.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            canvas.set_zoom_percent(invalid);
            assert!(
                (canvas.viewport.zoom() - MAX_ZOOM).abs() < 1e-4,
                "invalid zoom {invalid} must be ignored"
            );
        }

        canvas.command("setZoom", &serde_json::json!({ "percent": 225.0 }));
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);
        assert!((canvas.viewport.zoom() - 2.25).abs() < 1e-4);

        // Missing or non-numeric `percent` leaves zoom unchanged.
        canvas.command("setZoom", &serde_json::json!({}));
        assert!((canvas.viewport.zoom() - 2.25).abs() < 1e-4);
    }

    #[test]
    fn union_quad_bounds_merges_quads_and_handles_empty() {
        assert_eq!(union_quad_bounds(&[]), None);
        let q1 = PageQuad {
            ul: PagePoint { x: 20.0, y: 30.0 },
            ur: PagePoint { x: 100.0, y: 30.0 },
            ll: PagePoint { x: 20.0, y: 42.0 },
            lr: PagePoint { x: 100.0, y: 42.0 },
        };
        let q2 = PageQuad {
            ul: PagePoint { x: 10.0, y: 45.0 },
            ur: PagePoint { x: 140.0, y: 45.0 },
            ll: PagePoint { x: 10.0, y: 60.0 },
            lr: PagePoint { x: 140.0, y: 60.0 },
        };
        assert_eq!(
            union_quad_bounds(&[q1, q2]),
            Some(PageRect {
                x0: 10.0,
                y0: 30.0,
                x1: 140.0,
                y1: 60.0,
            })
        );
    }

    #[test]
    fn zoom_to_selection_falls_back_to_fit_width_when_empty() {
        let (mut canvas, _wake_rx) = synthetic_canvas(800.0, 1000.0);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);

        canvas.zoom_to_selection();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitWidth);

        // Collapsed text selection also falls back to FitWidth.
        canvas.set_zoom_percent(100.0);
        canvas.selection = Some(Selection {
            anchor: TextPos { page: 0, index: 5 },
            focus: TextPos { page: 0, index: 5 },
            granularity: Granularity::Char,
        });
        canvas.command("zoomToSelection", &JsonValue::Null);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::FitWidth);
    }

    #[test]
    fn zoom_to_selection_zooms_and_centers_block_selection() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let doc = Arc::new(Document::open(&path).expect("corpus pdf opens"));
        let (mut canvas, _wake_rx) = synthetic_canvas(800.0, 1000.0);
        canvas.document = Some(doc);
        canvas.set_block_select(true);

        let page0 = canvas
            .viewport
            .to_screen(&canvas.layout.page(0).expect("page 0"));
        let limits = ClickLimits {
            time: 500,
            width: 4,
            height: 4,
        };
        let sx0 = (page0.x0 + 200.0) as i32;
        let sy0 = (page0.y0 + 300.0) as i32;
        let sx1 = (page0.x0 + 400.0) as i32;
        let sy1 = (page0.y0 + 400.0) as i32;
        assert!(canvas.mouse_down(sx0, sy0, 100, limits));
        canvas.mouse_moved(Some((sx1, sy1)));
        assert!(canvas.mouse_up(sx1, sy1));
        assert!(canvas.has_block_selection());

        let region = canvas.block_regions()[0];
        let sel_w = region.rect.x1 - region.rect.x0;
        let sel_h = region.rect.y1 - region.rect.y0;
        let expected_zoom = ((800.0 * 0.85) / sel_w)
            .min((1000.0 * 0.85) / sel_h)
            .clamp(MIN_ZOOM, MAX_ZOOM);

        canvas.command("zoomToSelection", &JsonValue::Null);
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);
        assert!(
            (canvas.viewport.zoom() - expected_zoom).abs() < 1e-3,
            "expected zoom {expected_zoom}, got {}",
            canvas.viewport.zoom()
        );

        let placement = Placement::of_page(
            &canvas.layout,
            &canvas.bounds,
            canvas.view.rotation,
            region.page,
        )
        .expect("page placement");
        let (cx, cy) = placement.to_layout(PagePoint {
            x: (region.rect.x0 + region.rect.x1) / 2.0,
            y: (region.rect.y0 + region.rect.y1) / 2.0,
        });
        let (screen_x, screen_y) = canvas.viewport.to_screen_point(cx, cy);
        assert!(
            (screen_x - 400.0).abs() < 1.0 && (screen_y - 500.0).abs() < 1.0,
            "expected selection center at (400, 500), got ({screen_x}, {screen_y})"
        );
    }

    #[test]
    fn zoom_to_selection_zooms_text_selection() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let doc = Arc::new(Document::open(&path).expect("corpus pdf opens"));
        let mupdf = doc.mupdf().expect("mupdf");
        let page_box = {
            let guard = mupdf.lock().expect("document lock");
            guard.page_box(0, PageBox::Crop).expect("page 0 box")
        };
        let (host, wake_rx) = RecordingHost::new(800.0, 1000.0);
        let mut canvas = PageCanvas::new(host, 1.0);
        canvas.document = Some(doc);
        canvas.bounds = vec![page_box];
        canvas.layout = Layout::new(
            &[PageSize {
                width: page_box.x1 - page_box.x0,
                height: page_box.y1 - page_box.y0,
            }],
            Spec::default(),
        );
        canvas.viewport.resize(800.0, 1000.0, &canvas.layout);
        canvas.viewport.set_zoom(1.0, &canvas.layout);

        let waker = Arc::clone(&canvas.waker);
        canvas.texts.request(0..=0, |page| {
            let job = pool().read_text(&mupdf, page, Priority::PageText);
            let waker = Arc::clone(&waker);
            job.when_done(move || waker.wake());
            job
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while canvas.texts.get(0).is_none() && Instant::now() < deadline {
            let _ = wake_rx.recv_timeout(Duration::from_millis(25));
            canvas.texts.take_done();
        }
        let text = canvas.texts.get(0).expect("page 0 text loaded");
        assert!(!text.is_empty(), "expected text on page 0");
        let selection = Selection {
            anchor: TextPos { page: 0, index: 0 },
            focus: TextPos {
                page: 0,
                index: text.len().min(20),
            },
            granularity: Granularity::Char,
        };
        let range = selection.range_on(0, text).expect("selection range");
        let rect = union_quad_bounds(&text.highlight(range)).expect("selection quads");
        let expected_zoom = ((800.0 * 0.85) / (rect.x1 - rect.x0))
            .min((1000.0 * 0.85) / (rect.y1 - rect.y0))
            .clamp(MIN_ZOOM, MAX_ZOOM);

        canvas.selection = Some(selection);
        canvas.zoom_to_selection();
        assert_eq!(canvas.viewport.zoom_mode(), ZoomMode::Custom);
        assert!(
            (canvas.viewport.zoom() - expected_zoom).abs() < 1e-3,
            "expected zoom {expected_zoom}, got {}",
            canvas.viewport.zoom()
        );
    }

    #[test]
    fn a_highlighter_preview_covers_each_segment() {
        let quads = segment_quads(&[(0.0, 0.0, 4.0), (10.0, 0.0, 4.0)]);
        assert_eq!(
            quads,
            vec![[(-2.0, 2.0), (12.0, 2.0), (12.0, -2.0), (-2.0, -2.0)]]
        );
        assert_eq!(segment_quads(&[(5.0, 5.0, 2.0)]).len(), 1);
        assert!(segment_quads(&[]).is_empty());
    }

    #[test]
    fn corpus_pdfs_render_non_blank_tiles_through_resize_scroll_and_zoom() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
        for name in ["zlib-manual.pdf", "base14-outline-annot.pdf"] {
            let path = corpus.join(name);
            let (host, wake_rx) = RecordingHost::new(1600.0, 1200.0);
            let mut canvas = PageCanvas::new(host, 144.0 / 72.0);
            canvas.open(Some(&path));
            let (tiles, ink_pixels) =
                pump_until_drawn(&mut canvas, &wake_rx, Duration::from_secs(15));
            assert!(tiles > 0, "{name}: expected visible tiles to be drawn");
            assert!(
                ink_pixels > 100,
                "{name}: expected non-blank PDF text/ink pixels, got {ink_pixels}"
            );
            // Resize larger (e.g. maximized 4K pane), scroll, and zoom in.
            canvas.resize(3000, 1986, 144.0 / 72.0);
            let (resized_tiles, resized_ink) =
                pump_until_drawn(&mut canvas, &wake_rx, Duration::from_secs(15));
            assert!(
                resized_tiles >= tiles && resized_ink > 100,
                "{name}: resized 3000x1986 viewport should draw full non-blank tiles ({resized_tiles} tiles, {resized_ink} ink px)"
            );
            let max_x1 = canvas
                .host()
                .draws
                .iter()
                .map(|(r, _)| r.x1)
                .fold(0.0_f32, f32::max);
            assert!(
                max_x1 > 2000.0,
                "{name}: tiles must span past 2000px on a 3000px viewport, got max_x1={max_x1} across {} draws",
                canvas.host().draws.len()
            );
            canvas.scroll_by(0.0, 300.0);
            canvas.zoom_step(ZoomChange::In);
            let (zoomed_tiles, zoomed_ink) =
                pump_until_drawn(&mut canvas, &wake_rx, Duration::from_secs(15));
            assert!(
                zoomed_tiles > 0 && zoomed_ink > 0,
                "{name}: scrolled/zoomed view should render non-blank tiles"
            );
            // Simulate device loss while tiles are in flight: next paint must recover and upload.
            canvas.host_mut().lose_device();
            assert!(!canvas.host().can_upload());
            let (recovered_tiles, recovered_ink) =
                pump_until_drawn(&mut canvas, &wake_rx, Duration::from_secs(15));
            assert!(
                recovered_tiles > 0 && recovered_ink > 0,
                "{name}: device-loss recovery should re-upload and draw non-blank tiles"
            );
        }
    }
}
