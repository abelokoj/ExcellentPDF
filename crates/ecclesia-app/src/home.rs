//! SumatraPDF Enhanced-style Home Page (plan F.17 + Enhanced parity):
//! centered brand hero (`Ecclesia PDF`), flanking Command Palette / Shortcuts circle
//! buttons, pill document search, two-row brand/surface CTAs, expandable Enhanced Features drawer,
//! Recent / Frequently Read header bar with inline zoom (`75%–250%`), Grid / List view switch,
//! fixed-header pin, whole-page preview toggle, numbered cover cards with progress badges,
//! Completed Documents section, and interactive tip footer.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use ecclesia_core::document::Document;
use ecclesia_core::shared::pool;
use ecclesia_core::thumbnail;
use ecclesia_features::home::{self, COVER_WIDTH, cover_name};
use ecclesia_store::recent::RecentFile;
use ecclesia_store::workspaces::Workspace;
use nsh_base::{button, input, number_input, theme};
use nsh_core::color::{rgb, rgba, white};
use nsh_core::prelude::*;

use crate::commands::CommandId;
use crate::panes::Shared;
use crate::recent::RecentState;
use crate::settings::{Live, Settings};
use crate::study_ui::{HomeStudySummary, home_study_summary};
use crate::tabs::{TabState, pin_mark};
use crate::workspaces::{Workspaces, color_dot, display_name};

/// `settings.toml` key for Home design style: `"enhanced_warm"` (default), `"modern_green"`, or `"classic_clean"`.
pub(crate) const HOME_DESIGN_STYLE: &str = "home_design_style";
/// `settings.toml` key for Home layout mode: `"thumbnails"` (default) or `"list"`.
pub(crate) const HOME_VIEW_MODE: &str = "home_view_mode";
/// `settings.toml` key for thumbnail scale percent (`75..=250`, default `100`).
pub(crate) const HOME_THUMBNAIL_SIZE: &str = "home_thumbnail_size";
/// `settings.toml` key keeping header fixed while scrolling (`false` default).
pub(crate) const HOME_FIXED_HEADER: &str = "home_fixed_header";
/// `settings.toml` key for uncropped whole-page previews (`false` default).
pub(crate) const HOME_FULL_PAGE_THUMBNAILS: &str = "home_full_page_thumbnails";
/// `settings.toml` key for framed thumbnail cards (`true` default).
pub(crate) const HOME_THUMBNAIL_PADDING: &str = "home_thumbnail_padding";
/// `settings.toml` key sorting Home library by open count (`false` default).
pub(crate) const HOME_SORT_BY_FREQUENCY: &str = "home_sort_by_frequency";
/// `settings.toml` key displaying progress badges (`true` default).
pub(crate) const HOME_SHOW_PROGRESS: &str = "home_show_progress";
/// `settings.toml` key bounding visible Home documents (`1..=200`, default `100`).
pub(crate) const HOME_MAX_RECENT: &str = "home_max_recent";

pub(crate) const DEFAULT_HOME_THUMB_SIZE: u32 = 100;
pub(crate) const MIN_HOME_THUMB_SIZE: u32 = 75;
pub(crate) const MAX_HOME_THUMB_SIZE: u32 = 250;
const HOME_THUMB_STEP: i32 = 25;

pub(crate) const DEFAULT_HOME_MAX_RECENT: u32 = 100;
pub(crate) const MIN_HOME_MAX_RECENT: u32 = 1;
pub(crate) const MAX_HOME_MAX_RECENT: u32 = 200;

const BASE_CARD_W: f32 = 158.0;
const BASE_COVER_CROPPED_H: f32 = 192.0;
const BASE_COVER_FULL_H: f32 = 224.0;

const TIP_KEYS: [&str; 6] = [
    "home.tip.palette",
    "home.tip.tabs",
    "home.tip.hibernate",
    "home.tip.dictionary",
    "home.tip.annotate",
    "home.tip.research",
];

const ICON_SEARCH: &[u8] = include_bytes!("../assets/icons/search.svg");
const ICON_PALETTE: &[u8] = include_bytes!("../assets/icons/palette.svg");
const ICON_KEYBOARD: &[u8] = include_bytes!("../assets/icons/keyboard.svg");
const ICON_FOLDER: &[u8] = include_bytes!("../assets/icons/folder.svg");
const ICON_CLOCK: &[u8] = include_bytes!("../assets/icons/clock.svg");
const ICON_GRID: &[u8] = include_bytes!("../assets/icons/grid.svg");
const ICON_LIST: &[u8] = include_bytes!("../assets/icons/list.svg");
const ICON_ZOOM_OUT: &[u8] = include_bytes!("../assets/icons/zoom-out.svg");
const ICON_ZOOM_IN: &[u8] = include_bytes!("../assets/icons/zoom-in.svg");
const ICON_BOOK: &[u8] = include_bytes!("../assets/icons/book.svg");

#[derive(Clone, Copy)]
struct HomeStyleProps {
    recent: RecentState,
    covers: Covers,
    live: Live,
    style: fn(&Settings) -> HomeDesignStyle,
}

impl HomeStyleProps {
    fn palette(self) -> HomePalette {
        HomePalette::for_style(self.live.read(self.style))
    }

    fn view_mode(self) -> HomeViewMode {
        self.live
            .read(|s| HomeViewMode::parse(s.text(HOME_VIEW_MODE)))
    }

    fn thumb_size(self) -> u32 {
        self.live.read(read_thumb_size)
    }

    fn fixed_header(self) -> bool {
        self.live
            .read(|s| s.flag(HOME_FIXED_HEADER).unwrap_or(false))
    }

    fn full_page(self) -> bool {
        self.live
            .read(|s| s.flag(HOME_FULL_PAGE_THUMBNAILS).unwrap_or(false))
    }

    fn thumb_padding(self) -> bool {
        self.live
            .read(|s| s.flag(HOME_THUMBNAIL_PADDING).unwrap_or(true))
    }

    fn sort_freq(self) -> bool {
        self.live
            .read(|s| s.flag(HOME_SORT_BY_FREQUENCY).unwrap_or(false))
    }

    fn show_progress(self) -> bool {
        self.live
            .read(|s| s.flag(HOME_SHOW_PROGRESS).unwrap_or(true))
    }
}

fn design_style_of(settings: &Settings) -> HomeDesignStyle {
    HomeDesignStyle::parse(settings.text(HOME_DESIGN_STYLE))
}

#[derive(Clone, Copy)]
enum HomeSvg {
    Search,
    Palette,
    Keyboard,
    Folder,
    Clock,
    Grid,
    List,
    ZoomOut,
    ZoomIn,
    Book,
}

impl HomeSvg {
    fn uri(self) -> String {
        static CACHE: [OnceLock<String>; 10] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        let (slot, bytes): (usize, &[u8]) = match self {
            Self::Search => (0, ICON_SEARCH),
            Self::Palette => (1, ICON_PALETTE),
            Self::Keyboard => (2, ICON_KEYBOARD),
            Self::Folder => (3, ICON_FOLDER),
            Self::Clock => (4, ICON_CLOCK),
            Self::Grid => (5, ICON_GRID),
            Self::List => (6, ICON_LIST),
            Self::ZoomOut => (7, ICON_ZOOM_OUT),
            Self::ZoomIn => (8, ICON_ZOOM_IN),
            Self::Book => (9, ICON_BOOK),
        };
        CACHE[slot]
            .get_or_init(|| {
                format!(
                    "data:image/svg+xml;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            })
            .clone()
    }
}

fn svg_icon(
    icon: HomeSvg,
    size: f32,
    tint: impl Fn() -> AdaptiveColor + Copy + 'static,
) -> impl IntoElement {
    image(icon.uri())
        .w(px(size))
        .h(px(size))
        .object_fit(ResizeMode::Contain)
        .tint_color(tint)
}

/// Home page visual design style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HomeDesignStyle {
    /// Warm SumatraPDF Enhanced palette: amber/gold CTAs, green brand accent, ivory/charcoal backdrop.
    #[default]
    EnhancedWarm,
    /// Emerald-accented palette paired with subtle mint/forest surfaces.
    ModernGreen,
    /// Native theme tokens without custom home wash.
    ClassicClean,
}

impl HomeDesignStyle {
    /// Parses a persisted style string, defaulting to [`HomeDesignStyle::EnhancedWarm`].
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("modern_green") => Self::ModernGreen,
            Some("classic_clean") => Self::ClassicClean,
            _ => Self::EnhancedWarm,
        }
    }

    /// Stable settings identifier.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::EnhancedWarm => "enhanced_warm",
            Self::ModernGreen => "modern_green",
            Self::ClassicClean => "classic_clean",
        }
    }
}

/// Home document presentation layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HomeViewMode {
    /// Numbered thumbnail cover grid.
    #[default]
    Thumbnails,
    /// Compact horizontal rows with folder path & progress.
    List,
}

impl HomeViewMode {
    /// Parses a persisted layout string, defaulting to [`HomeViewMode::Thumbnails`].
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("list") => Self::List,
            _ => Self::Thumbnails,
        }
    }
}

/// Resolved adaptive colors for the selected [`HomeDesignStyle`].
#[derive(Clone, Copy)]
struct HomePalette {
    page_bg: AdaptiveColor,
    card_bg: AdaptiveColor,
    brand_cta_bg: AdaptiveColor,
    brand_cta_fg: AdaptiveColor,
    selection_outline: AdaptiveColor,
    badge_bg: AdaptiveColor,
}

impl HomePalette {
    fn for_style(style: HomeDesignStyle) -> Self {
        let base = theme();
        let selection_outline = AdaptiveColor::new(rgb(0x4ca6ff), rgb(0x4ca6ff));
        let badge_bg = AdaptiveColor::new(rgba(0x14161ad4), rgba(0x0c0d10dc));
        match style {
            HomeDesignStyle::EnhancedWarm => Self {
                page_bg: AdaptiveColor::new(rgb(0xfaf7ef), rgb(0x14161a)),
                card_bg: AdaptiveColor::new(rgb(0xffffff), rgb(0x1e2128)),
                brand_cta_bg: AdaptiveColor::new(rgb(0xf59e0b), rgb(0xd97706)),
                brand_cta_fg: AdaptiveColor::new(rgb(0x1d1d1f), rgb(0xffffff)),
                selection_outline,
                badge_bg,
            },
            HomeDesignStyle::ModernGreen => Self {
                page_bg: AdaptiveColor::new(rgb(0xf2f7f4), rgb(0x102219)),
                card_bg: AdaptiveColor::new(rgb(0xffffff), rgb(0x173124)),
                brand_cta_bg: AdaptiveColor::new(rgb(0x22c55e), rgb(0x16a34a)),
                brand_cta_fg: AdaptiveColor::new(rgb(0x0f172a), rgb(0xffffff)),
                selection_outline,
                badge_bg,
            },
            HomeDesignStyle::ClassicClean => Self {
                page_bg: base.background,
                card_bg: base.popover,
                brand_cta_bg: base.primary,
                brand_cta_fg: base.primary_foreground,
                selection_outline,
                badge_bg,
            },
        }
    }
}

/// Reads and clamps preview scale percent (`75..=250`).
pub(crate) fn read_thumb_size(settings: &Settings) -> u32 {
    settings
        .number(HOME_THUMBNAIL_SIZE)
        .map_or(DEFAULT_HOME_THUMB_SIZE, |v| {
            (v.round() as i64).clamp(
                i64::from(MIN_HOME_THUMB_SIZE),
                i64::from(MAX_HOME_THUMB_SIZE),
            ) as u32
        })
}

/// Reads and clamps maximum recent items (`1..=200`).
pub(crate) fn read_max_recent(settings: &Settings) -> usize {
    settings
        .number(HOME_MAX_RECENT)
        .map_or(DEFAULT_HOME_MAX_RECENT as usize, |v| {
            (v.round() as i64).clamp(
                i64::from(MIN_HOME_MAX_RECENT),
                i64::from(MAX_HOME_MAX_RECENT),
            ) as usize
        })
}

/// Formats reading progress (`41/675` when total pages known, or `p. 41`).
pub(crate) fn format_progress(page_zero_based: usize, total_pages: usize) -> String {
    let current = page_zero_based.saturating_add(1);
    if total_pages > 0 {
        format!("{current}/{total_pages}")
    } else {
        format!("p. {current}")
    }
}

/// Filters by search query, sorts by recency or frequency (keeping pinned first), bounds active rows,
/// and partitions into `(active_documents, completed_documents)`.
pub(crate) fn filter_and_partition(
    files: &[RecentFile],
    query: &str,
    sort_by_frequency: bool,
    max_recent: usize,
) -> (Vec<RecentFile>, Vec<RecentFile>) {
    let trimmed = query.trim();
    let has_query = !trimmed.is_empty();
    let mut matching: Vec<RecentFile> = files
        .iter()
        .filter(|file| home::matches(trimmed, &file.path))
        .cloned()
        .collect();
    matching.sort_by(|a, b| {
        if sort_by_frequency {
            b.pinned
                .cmp(&a.pinned)
                .then(b.open_count.cmp(&a.open_count))
                .then(b.seen.cmp(&a.seen))
        } else {
            b.pinned.cmp(&a.pinned).then(b.seen.cmp(&a.seen))
        }
    });
    let mut active = Vec::new();
    let mut completed = Vec::new();
    for file in matching {
        let limit = if has_query {
            MAX_HOME_MAX_RECENT as usize
        } else {
            max_recent
        };
        if file.completed {
            if completed.len() < limit {
                completed.push(file);
            }
        } else if active.len() < limit {
            active.push(file);
        }
    }
    (active, completed)
}

/// Asynchronous cover thumbnail cache for recent documents.
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
    /// Creates an empty per-window cover cache.
    pub(crate) fn new() -> Self {
        Self {
            ready: signal(HashMap::new()),
            asked: stored(HashSet::new()),
            running: stored(Vec::new()),
        }
    }

    /// The cover of the file at `path` once rendered (tracked); the first call schedules it off the UI thread.
    pub(crate) fn of(self, path: &Path) -> Option<SharedString> {
        if self.asked.with(|asked| asked.insert(path.to_owned())) {
            untracked(|| self.start(path.to_owned()));
        }
        self.ready.with(|ready| ready.get(path).cloned())
    }

    /// Resolves cached cover or renders page 0 on a background thread.
    fn start(self, path: PathBuf) {
        let ready = self.ready;
        let show = move |path: PathBuf, file: &Path| {
            let file = SharedString::from(file.to_string_lossy().into_owned());
            ready.update(|ready| {
                ready.insert(path, file);
            });
        };
        let covers_dir = data_dir().unwrap_or_else(std::env::temp_dir).join("covers");
        let task = spawn(async move {
            let meta_out = blocking({
                let path = path.clone();
                move || {
                    let meta = fs::metadata(&path).ok()?;
                    if !meta.is_file() {
                        return None;
                    }
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |since| since.as_secs());
                    let out = covers_dir.join(cover_name(&path, meta.len(), modified));
                    let cached = out.is_file();
                    Some((out, cached))
                }
            })
            .await;
            let Ok(Some((out, cached))) = meta_out else {
                return;
            };
            if cached {
                show(path, &out);
                return;
            }
            let opened = blocking({
                let path = path.clone();
                move || {
                    let document = Arc::new(Document::open(&path)?);
                    let mupdf = document.mupdf()?;
                    Ok::<_, ecclesia_core::document::Error>((document, mupdf))
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

/// A file's cover in a `width` × `height` frame: white paper until it is rendered, or its extension when unavailable.
pub(crate) fn cover(covers: Covers, path: PathBuf, width: f32, height: f32) -> impl IntoElement {
    let kind = extension_label(&path);
    let path = stored(path);
    let file = move || path.with(|path| covers.of(path));
    view()
        .w(px(width))
        .h(px(height))
        .flex_shrink(0.0)
        .rounded(px(4.0))
        .bg(white())
        .border_1()
        .border_color(move || theme().border)
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
                text(kind.clone())
                    .text_size(px((width / 4.0).clamp(8.0, 14.0)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0x8a8f98))
            }),
        )
}

fn extension_label(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_uppercase())
        .filter(|ext| !ext.is_empty())
        .unwrap_or_else(|| "DOC".to_owned())
}

fn extension_badge(path: &Path) -> impl IntoElement {
    let ext = extension_label(path);
    let color = match ext.as_str() {
        "PDF" => rgb(0xdc2626),
        "EPUB" | "MOBI" => rgb(0x2563eb),
        "MD" | "TXT" => rgb(0x16a34a),
        _ => rgb(0x64748b),
    };
    view()
        .px(px(4.0))
        .py(px(1.5))
        .rounded(px(3.0))
        .flex_shrink(0.0)
        .bg(color)
        .child(
            text(ext)
                .text_size(px(8.5))
                .font_weight(FontWeight::BOLD)
                .text_color(white()),
        )
}

/// Renders the SumatraPDF Enhanced-style Home Page inside a pane.
pub(crate) fn home_page(
    shared: Shared,
    tabs: TabState,
    home_open: Signal<bool>,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let Shared {
        recent,
        covers,
        workspaces,
        ..
    } = shared;
    let live = tabs.live;
    recent.load();
    let query = signal(String::new());
    let features_open = signal(false);
    let tip_index = signal(0usize);

    let props = HomeStyleProps {
        recent,
        covers,
        live,
        style: design_style_of,
    };
    let palette = move || props.palette();
    let fixed_header = move || props.fixed_header();
    let sort_freq = move || props.sort_freq();
    let max_recent = move || live.read(read_max_recent);

    let partitioned = computed(move || {
        let files = recent.files();
        query.with(|q| filter_and_partition(&files, q, sort_freq(), max_recent()))
    });
    let open_first_match = move || {
        let first = partitioned.with_untracked(|(active, completed)| {
            active.first().or_else(|| completed.first()).cloned()
        });
        if let Some(file) = first {
            home_open.set(false);
            tabs.open_at(file.path, file.page);
        }
    };
    let open_recent_file = move |path: PathBuf, page: usize| {
        home_open.set(false);
        tabs.open_at(path, page);
    };

    let header_block = move || {
        view()
            .w_full()
            .items_center()
            .gap(px(14.0))
            .child(hero_brand())
            .child(search_bar_row(query, palette, run, open_first_match))
            .child(action_rows(
                recent,
                features_open,
                palette,
                run,
                open_recent_file,
            ))
            .child(when(features_open, move || {
                features_drawer(palette, run, move || features_open.set(false))
            }))
            .child(when(
                move || workspaces.list().len() > 1,
                move || workspace_row(workspaces),
            ))
            .child(library_toolbar(props))
    };

    let library_body = move || {
        let active_items = move || partitioned.with(|(active, _)| active.clone());
        let completed_items = move || partitioned.with(|(_, completed)| completed.clone());
        view()
            .w_full()
            .gap(px(24.0))
            .child(when(
                move || !active_items().is_empty(),
                move || document_collection(active_items, props, open_recent_file),
            ))
            .child(when(
                move || active_items().is_empty() && completed_items().is_empty(),
                move || {
                    view().w_full().py(px(24.0)).items_center().child(
                        text(move || {
                            if query.with(|q| q.trim().is_empty()) {
                                t!("recent.empty")
                            } else {
                                t!(
                                    "home.no_match",
                                    keys = crate::commands::shortcut_label(CommandId::TabList)
                                )
                            }
                        })
                        .text_size(px(14.0))
                        .text_color(move || theme().muted_foreground),
                    )
                },
            ))
            .child(completed_section(completed_items, props, open_recent_file))
    };

    let tip_footer = move || {
        view()
            .w_full()
            .border_t_1()
            .border_t_color(move || theme().border)
            .bg(move || palette().card_bg)
            .px(px(20.0))
            .py(px(8.0))
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .child(
                text(move || {
                    let idx = tip_index.get() % TIP_KEYS.len();
                    format!("{}{}", t!("home.tip_prefix"), t!(TIP_KEYS[idx]))
                })
                .text_size(px(12.5))
                .text_color(move || theme().muted_foreground),
            )
            .child(
                button()
                    .accessibility_label(t!("home.tip_next"))
                    .on_press(move || tip_index.update(|idx| *idx = (*idx + 1) % TIP_KEYS.len()))
                    .px(px(8.0))
                    .h(px(22.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(move || theme().muted))
                    .child(
                        text(move || t!("home.tip_next"))
                            .text_size(px(11.0))
                            .text_color(move || theme().foreground),
                    ),
            )
    };

    view()
        .flex_1()
        .bg(move || palette().page_bg)
        .child(when(fixed_header, move || {
            view()
                .flex_1()
                .child(
                    view()
                        .w_full()
                        .px(px(32.0))
                        .pt(px(20.0))
                        .child(header_block()),
                )
                .child(
                    scroll().flex_1().child(
                        view()
                            .w_full()
                            .px(px(32.0))
                            .py(px(16.0))
                            .child(library_body()),
                    ),
                )
        }))
        .child(when(
            move || !fixed_header(),
            move || {
                scroll().flex_1().child(
                    view()
                        .w_full()
                        .px(px(32.0))
                        .pt(px(24.0))
                        .pb(px(24.0))
                        .gap(px(16.0))
                        .child(header_block())
                        .child(library_body()),
                )
            },
        ))
        .child(tip_footer())
}

fn hero_brand() -> impl IntoElement {
    view()
        .w_full()
        .items_center()
        .gap(px(8.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(12.0))
                .child(
                    image(crate::shell::app_menu::app_icon_src())
                        .w(px(48.0))
                        .h(px(48.0))
                        .rounded(px(10.0))
                        .object_fit(ResizeMode::Contain),
                )
                .child(
                    text(move || t!("app.name"))
                        .text_size(px(38.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .items_center()
                .gap(px(2.0))
                .child(
                    text(move || {
                        t!("home.subtitle")
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .text_size(px(15.0))
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        t!("home.subtitle")
                            .lines()
                            .nth(1)
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .text_size(px(15.0))
                    .text_color(move || theme().muted_foreground),
                ),
        )
}

fn search_bar_row(
    query: Signal<String>,
    palette: impl Fn() -> HomePalette + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
    open_first: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let circle_btn = move |label_key: &'static str, icon: HomeSvg, cmd: CommandId| {
        button()
            .accessibility_label(move || Some(SharedString::from(t!(label_key))))
            .on_press(move || run(cmd))
            .w(px(44.0))
            .h(px(44.0))
            .flex_shrink(0.0)
            .rounded_full()
            .border_1()
            .border_b(px(2.0))
            .border_color(move || theme().border)
            .bg(move || palette().card_bg)
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().muted))
            .active(|s| s.opacity(0.8))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(svg_icon(icon, 20.0, || theme().foreground))
    };

    let in_fields = KeyOptions {
        when: nsh_core::keymap::When::Always,
        ..KeyOptions::default()
    };
    on_key_with("escape", in_fields, move |_| {
        if !query.with_untracked(String::is_empty) {
            query.set(String::new());
        }
    });

    view()
        .w_full()
        .max_w(px(740.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(circle_btn(
            "command.open_palette",
            HomeSvg::Palette,
            CommandId::OpenPalette,
        ))
        .child(
            view()
                .flex_1()
                .h(px(46.0))
                .px(px(18.0))
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .rounded_full()
                .border_1()
                .border_b(px(2.0))
                .border_color(move || theme().border)
                .bg(move || palette().card_bg)
                .child(svg_icon(HomeSvg::Search, 18.0, || theme().muted_foreground))
                .child(
                    input()
                        .value(move || query.get())
                        .on_change(move |text: &str| query.set(text.to_owned()))
                        .on_submit(move |_: &str| open_first())
                        .placeholder(t!("home.search_placeholder"))
                        .label(t!("home.search_placeholder"))
                        .flex_1()
                        .text_size(px(16.0))
                        .text_color(move || theme().foreground),
                ),
        )
        .child(circle_btn(
            "command.open_settings",
            HomeSvg::Keyboard,
            CommandId::OpenSettings,
        ))
}

fn action_rows(
    recent: RecentState,
    features_open: Signal<bool>,
    palette: impl Fn() -> HomePalette + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
    open_recent: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let newest = move || recent.files().into_iter().find(|file| !file.completed);
    let summary = move || {
        let HomeStudySummary { due_cards, .. } = home_study_summary();
        t!("home.learning_hub", due = due_cards)
    };

    let brand_btn = move |label: String, icon: Option<HomeSvg>, press: Box<dyn Fn()>| {
        let icon_el = icon.map(|kind| svg_icon(kind, 16.0, move || palette().brand_cta_fg));
        let btn = button()
            .accessibility_label(label.clone())
            .on_press(press)
            .h(px(34.0))
            .px(px(14.0))
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .rounded(px(9.0))
            .border_1()
            .border_b(px(3.0))
            .border_color(move || theme().border)
            .bg(move || palette().brand_cta_bg)
            .hover(|s| s.opacity(0.92))
            .active(|s| s.opacity(0.8))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring));
        let btn = match icon_el {
            Some(el) => btn.child(el),
            None => btn,
        };
        btn.child(
            text(label)
                .text_size(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette().brand_cta_fg),
        )
    };

    let surface_btn = move |label: String, press: Box<dyn Fn()>| {
        button()
            .accessibility_label(label.clone())
            .on_press(press)
            .h(px(34.0))
            .px(px(14.0))
            .flex_row()
            .items_center()
            .justify_center()
            .rounded(px(9.0))
            .border_1()
            .border_b(px(3.0))
            .border_color(move || theme().border)
            .bg(move || palette().card_bg)
            .hover(|s| s.bg(move || theme().muted))
            .active(|s| s.opacity(0.8))
            .disabled_style(|s| s.opacity(0.45))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(
                text(label)
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().foreground),
            )
    };

    view()
        .w_full()
        .items_center()
        .gap(px(10.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .justify_center()
                .gap(px(10.0))
                .child(brand_btn(
                    t!("home.explore_files"),
                    Some(HomeSvg::Folder),
                    Box::new(move || run(CommandId::OpenFile)),
                ))
                .child(
                    surface_btn(
                        t!("home.resume_last"),
                        Box::new(move || {
                            if let Some(file) = newest() {
                                open_recent(file.path, file.page);
                            }
                        }),
                    )
                    .disabled(move || newest().is_none()),
                )
                .child(surface_btn(
                    t!("home.word_of_day"),
                    Box::new(move || run(CommandId::Dictionary)),
                )),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .justify_center()
                .gap(px(10.0))
                .child(brand_btn(
                    summary(),
                    None,
                    Box::new(move || run(CommandId::DailyReview)),
                ))
                .child(surface_btn(
                    t!("home.dictionary"),
                    Box::new(move || run(CommandId::Dictionary)),
                ))
                .child(
                    button()
                        .accessibility_label(move || {
                            Some(SharedString::from(if features_open.get() {
                                t!("home.features_hide")
                            } else {
                                t!("home.features_show")
                            }))
                        })
                        .on_press(move || features_open.update(|open| *open = !*open))
                        .h(px(34.0))
                        .px(px(14.0))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .border_1()
                        .border_b(px(3.0))
                        .border_color(move || theme().border)
                        .bg(move || palette().card_bg)
                        .hover(|s| s.bg(move || theme().muted))
                        .active(|s| s.opacity(0.8))
                        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                        .child(
                            text(move || {
                                if features_open.get() {
                                    t!("home.features_hide")
                                } else {
                                    t!("home.features_show")
                                }
                            })
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                        ),
                ),
        )
}

fn features_drawer(
    palette: impl Fn() -> HomePalette + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
    close: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let entries: [(HomeSvg, &'static str); 6] = [
        (HomeSvg::Grid, "home.feature.library"),
        (HomeSvg::Palette, "home.feature.styles"),
        (HomeSvg::Keyboard, "home.feature.dictionary"),
        (HomeSvg::Book, "home.feature.ink"),
        (HomeSvg::Clock, "home.feature.hibernation"),
        (HomeSvg::Folder, "home.feature.research"),
    ];
    let quick_actions: [(&'static str, CommandId); 6] = [
        ("home.feature_btn.dictionary", CommandId::Dictionary),
        ("home.feature_btn.review", CommandId::DailyReview),
        ("home.feature_btn.settings", CommandId::OpenSettings),
        ("home.feature_btn.notebook", CommandId::NewNotebook),
        ("home.feature_btn.library", CommandId::OpenLibrary),
        ("home.feature_btn.ai", CommandId::AiPanel),
    ];

    let mut cards = view().w_full().gap(px(8.0));
    for (icon, key) in entries {
        cards = cards.child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .p(px(10.0))
                .rounded(px(9.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || palette().card_bg)
                .child(svg_icon(icon, 18.0, || theme().foreground))
                .child(
                    text(move || t!(key))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                ),
        );
    }

    let mut actions_wrap = view().flex_row().flex_wrap().justify_center().gap(px(8.0));
    for (key, cmd) in quick_actions {
        actions_wrap = actions_wrap.child(
            button()
                .accessibility_label(move || Some(SharedString::from(t!(key))))
                .on_press(move || {
                    close();
                    run(cmd);
                })
                .h(px(30.0))
                .px(px(12.0))
                .rounded(px(8.0))
                .border_1()
                .border_b(px(2.0))
                .border_color(move || theme().border)
                .bg(move || palette().card_bg)
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .child(
                    text(move || t!(key))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                ),
        );
    }

    view()
        .w_full()
        .max_w(px(760.0))
        .p(px(14.0))
        .gap(px(12.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || palette().card_bg)
        .child(cards)
        .child(actions_wrap)
}

fn library_toolbar(props: HomeStyleProps) -> impl IntoElement {
    let zoom_editing = signal(false);
    let recent = props.recent;
    let live = props.live;
    let palette = move || props.palette();
    let view_mode = move || props.view_mode();
    let thumb_size = move || props.thumb_size();
    let fixed_header = move || props.fixed_header();
    let full_page = move || props.full_page();
    let sort_freq = move || props.sort_freq();

    let icon_btn = move |label_key: &'static str,
                         active: Box<dyn Fn() -> bool>,
                         on_click: Box<dyn Fn()>,
                         icon: HomeSvg| {
        let active_rc: std::rc::Rc<dyn Fn() -> bool> = std::rc::Rc::from(active);
        let active_bg = active_rc.clone();
        let active_border = active_rc.clone();
        button()
            .accessibility_label(move || Some(SharedString::from(t!(label_key))))
            .on_press(on_click)
            .w(px(30.0))
            .h(px(28.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(move || {
                if active_border() {
                    theme().ring
                } else {
                    theme().border
                }
            })
            .bg(move || {
                if active_bg() {
                    theme().sidebar_accent
                } else {
                    palette().card_bg
                }
            })
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().muted))
            .child(svg_icon(icon, 16.0, || theme().foreground))
    };

    let step_zoom = move |delta: i32| {
        live.change(|s| {
            let cur = i32::try_from(read_thumb_size(s)).unwrap_or(100);
            let next =
                (cur + delta).clamp(MIN_HOME_THUMB_SIZE as i32, MAX_HOME_THUMB_SIZE as i32) as u32;
            s.set_number(
                HOME_THUMBNAIL_SIZE,
                (next != DEFAULT_HOME_THUMB_SIZE).then_some(f64::from(next)),
            );
        });
    };

    let has_unpinned = move || {
        recent
            .files()
            .iter()
            .any(|file| !file.pinned && !file.completed)
    };

    view()
        .w_full()
        .gap(px(8.0))
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(svg_icon(HomeSvg::Clock, 18.0, || theme().foreground))
                .child(
                    button()
                        .accessibility_label(move || {
                            Some(SharedString::from(if sort_freq() {
                                t!("home.frequently_read")
                            } else {
                                t!("home.recent_documents")
                            }))
                        })
                        .on_press(move || {
                            live.change(|s| {
                                let next = !s.flag(HOME_SORT_BY_FREQUENCY).unwrap_or(false);
                                s.set_flag(HOME_SORT_BY_FREQUENCY, next.then_some(true));
                            });
                        })
                        .rounded(px(6.0))
                        .px(px(6.0))
                        .py(px(2.0))
                        .hover(|s| s.bg(move || theme().muted))
                        .child(
                            text(move || {
                                if sort_freq() {
                                    t!("home.frequently_read")
                                } else {
                                    t!("home.recent_documents")
                                }
                            })
                            .text_size(px(16.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                        ),
                )
                .child(when(has_unpinned, move || {
                    button()
                        .accessibility_label(t!("recent.clear_label"))
                        .on_press(move || recent.clear())
                        .h(px(26.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(move || theme().muted))
                        .child(
                            text(move || t!("recent.clear"))
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        )
                }))
                .child(view().flex_1())
                .child(when(
                    move || view_mode() == HomeViewMode::Thumbnails,
                    move || {
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(icon_btn(
                                "home.zoom_out",
                                Box::new(|| false),
                                Box::new(move || step_zoom(-HOME_THUMB_STEP)),
                                HomeSvg::ZoomOut,
                            ))
                            .child(when(
                                move || !zoom_editing.get(),
                                move || {
                                    button()
                                        .accessibility_label(t!("home.zoom_edit"))
                                        .on_press(move || zoom_editing.set(true))
                                        .h(px(28.0))
                                        .min_w(px(54.0))
                                        .px(px(8.0))
                                        .rounded(px(7.0))
                                        .border_1()
                                        .border_color(move || theme().border)
                                        .bg(move || palette().card_bg)
                                        .items_center()
                                        .justify_center()
                                        .hover(|s| s.bg(move || theme().muted))
                                        .child(
                                            text(move || format!("{}%", thumb_size()))
                                                .tabular_nums()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().foreground),
                                        )
                                },
                            ))
                            .child(when(zoom_editing, move || {
                                number_input()
                                    .value(move || Some(f64::from(thumb_size())))
                                    .min(Some(f64::from(MIN_HOME_THUMB_SIZE)))
                                    .max(Some(f64::from(MAX_HOME_THUMB_SIZE)))
                                    .step(25.0)
                                    .accessibility_label(t!("home.zoom_edit"))
                                    .on_change(move |value: &Option<f64>| {
                                        if let Some(v) = value {
                                            let clamped = v.round().clamp(
                                                f64::from(MIN_HOME_THUMB_SIZE),
                                                f64::from(MAX_HOME_THUMB_SIZE),
                                            );
                                            live.change(|s| {
                                                s.set_number(
                                                    HOME_THUMBNAIL_SIZE,
                                                    (clamped as u32 != DEFAULT_HOME_THUMB_SIZE)
                                                        .then_some(clamped),
                                                );
                                            });
                                        }
                                    })
                                    .w(px(78.0))
                                    .h(px(28.0))
                                    .rounded(px(7.0))
                                    .border_1()
                                    .border_color(move || theme().ring)
                                    .bg(move || palette().card_bg)
                                    .suffix(
                                        button()
                                            .on_press(move || zoom_editing.set(false))
                                            .px(px(4.0))
                                            .child(
                                                text("✓")
                                                    .text_size(px(11.0))
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                            }))
                            .child(icon_btn(
                                "home.zoom_in",
                                Box::new(|| false),
                                Box::new(move || step_zoom(HOME_THUMB_STEP)),
                                HomeSvg::ZoomIn,
                            ))
                    },
                ))
                .child(icon_btn(
                    "home.view_grid",
                    Box::new(move || view_mode() == HomeViewMode::Thumbnails),
                    Box::new(move || {
                        live.change(|s| s.set_text(HOME_VIEW_MODE, None));
                    }),
                    HomeSvg::Grid,
                ))
                .child(icon_btn(
                    "home.view_list",
                    Box::new(move || view_mode() == HomeViewMode::List),
                    Box::new(move || {
                        live.change(|s| s.set_text(HOME_VIEW_MODE, Some("list")));
                    }),
                    HomeSvg::List,
                ))
                .child(
                    button()
                        .accessibility_label(t!("home.pin_header"))
                        .on_press(move || {
                            live.change(|s| {
                                let next = !s.flag(HOME_FIXED_HEADER).unwrap_or(false);
                                s.set_flag(HOME_FIXED_HEADER, next.then_some(true));
                            });
                        })
                        .w(px(30.0))
                        .h(px(28.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if fixed_header() {
                                theme().ring
                            } else {
                                theme().border
                            }
                        })
                        .bg(move || {
                            if fixed_header() {
                                theme().sidebar_accent
                            } else {
                                palette().card_bg
                            }
                        })
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(move || theme().muted))
                        .child(pin_mark(move || theme().foreground)),
                )
                .child(icon_btn(
                    "home.full_page_previews",
                    Box::new(full_page),
                    Box::new(move || {
                        live.change(|s| {
                            let next = !s.flag(HOME_FULL_PAGE_THUMBNAILS).unwrap_or(false);
                            s.set_flag(HOME_FULL_PAGE_THUMBNAILS, next.then_some(true));
                        });
                    }),
                    HomeSvg::Book,
                )),
        )
        .child(view().w_full().h(px(1.0)).bg(move || theme().border))
}

fn document_collection(
    items: impl Fn() -> Vec<RecentFile> + Copy + 'static,
    props: HomeStyleProps,
    open_file: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let numbered = move || {
        items()
            .into_iter()
            .enumerate()
            .map(|(i, file)| (i + 1, file))
            .collect::<Vec<_>>()
    };
    view()
        .w_full()
        .child(when(
            move || props.view_mode() == HomeViewMode::Thumbnails,
            move || {
                view()
                    .w_full()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(16.0))
                    .child(each(
                        numbered,
                        |(idx, file): &(usize, RecentFile)| {
                            (file.path.clone(), *idx, file.completed)
                        },
                        move |entry, _| {
                            let (number, file) = entry.get();
                            thumbnail_card(number, file, props, open_file)
                        },
                    ))
            },
        ))
        .child(when(
            move || props.view_mode() == HomeViewMode::List,
            move || {
                view().w_full().gap(px(6.0)).child(each(
                    numbered,
                    |(idx, file): &(usize, RecentFile)| (file.path.clone(), *idx, file.completed),
                    move |entry, _| {
                        let (number, file) = entry.get();
                        list_row(number, file, props, open_file)
                    },
                ))
            },
        ))
}

fn completed_section(
    completed_items: impl Fn() -> Vec<RecentFile> + Copy + 'static,
    props: HomeStyleProps,
    open_file: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    view()
        .w_full()
        .gap(px(12.0))
        .pt(px(8.0))
        .child(view().w_full().h(px(1.0)).bg(move || theme().border))
        .child(
            text(move || t!("home.completed_documents"))
                .text_size(px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(when(
            move || !completed_items().is_empty(),
            move || document_collection(completed_items, props, open_file),
        ))
        .child(when(
            move || completed_items().is_empty(),
            move || {
                text(move || t!("home.no_completed"))
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground)
            },
        ))
}

fn thumbnail_card(
    number: usize,
    file: RecentFile,
    props: HomeStyleProps,
    open_file: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let recent = props.recent;
    let covers = props.covers;
    let palette = move || props.palette();
    let thumb_size = move || props.thumb_size();
    let full_page = move || props.full_page();
    let thumb_padding = move || props.thumb_padding();
    let show_progress = move || props.show_progress();
    let path = file.path.clone();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let progress = format_progress(file.page, file.total_pages);
    let scale = move || f32::from(thumb_size() as u16) / 100.0;
    let card_w = move || BASE_CARD_W * scale();
    let cover_h = move || {
        (if full_page() {
            BASE_COVER_FULL_H
        } else {
            BASE_COVER_CROPPED_H
        }) * scale()
    };

    let path_cover = stored(path.clone());
    let cover_file = move || path_cover.with(|p| covers.of(p));
    let kind = extension_label(&path);
    let path_open = path.clone();
    let path_pin = path.clone();
    let path_done = path.clone();
    let path_remove = path.clone();
    let name_done = name.clone();
    let name_remove = name.clone();
    let name_pin = name.clone();

    let cover_box = view()
        .w_full()
        .h(move || px(cover_h()))
        .rounded(px(6.0))
        .bg(white())
        .border_1()
        .border_color(move || theme().border)
        .overflow_hidden()
        .child(
            view().size_full().items_center().justify_center().child(
                when(
                    move || cover_file().is_some(),
                    move || {
                        image(move || cover_file().unwrap_or_default())
                            .object_fit(if full_page() {
                                ResizeMode::Contain
                            } else {
                                ResizeMode::Cover
                            })
                            .size_full()
                    },
                )
                .otherwise(move || {
                    text(kind.clone())
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0x8a8f98))
                }),
            ),
        )
        .child(
            view()
                .absolute()
                .top(px(6.0))
                .left(px(6.0))
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(move || palette().badge_bg)
                .child(
                    text(number.to_string())
                        .tabular_nums()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(white()),
                ),
        )
        .child(
            button()
                .accessibility_label(t!("recent.pin", name = name_pin))
                .on_press(move || recent.set_pinned(path_pin.clone(), !file.pinned))
                .absolute()
                .top(px(6.0))
                .right(px(6.0))
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(4.0))
                .bg(move || {
                    if file.pinned {
                        palette().brand_cta_bg
                    } else {
                        palette().badge_bg
                    }
                })
                .items_center()
                .justify_center()
                .hover(|s| s.opacity(0.9))
                .child(pin_mark(move || {
                    if file.pinned {
                        palette().brand_cta_fg
                    } else {
                        white().into()
                    }
                })),
        )
        .child(when(show_progress, move || {
            view()
                .absolute()
                .bottom(px(6.0))
                .right(px(6.0))
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(move || palette().badge_bg)
                .child(
                    text(progress.clone())
                        .tabular_nums()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(white()),
                )
        }));

    let footer = view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .child(extension_badge(&path))
        .child(
            text(name.clone())
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(
            button()
                .accessibility_label(t!(
                    if file.completed {
                        "home.mark_active"
                    } else {
                        "home.mark_completed"
                    },
                    name = name_done
                ))
                .on_press(move || recent.set_completed(path_done.clone(), !file.completed))
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(4.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .child(
                    text(if file.completed { "↺" } else { "✓" })
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(crate::shell::app_menu::close_x_button(
            t!("recent.remove", name = name_remove),
            20.0,
            10.0,
            10.0,
            move || recent.remove(path_remove.clone()),
        ));

    view()
        .w(move || px(card_w() + if thumb_padding() { 18.0 } else { 0.0 }))
        .flex_shrink(0.0)
        .rounded(px(12.0))
        .style_when(thumb_padding, |s| {
            s.p(px(9.0))
                .border_1()
                .border_b(px(2.0))
                .border_color(move || theme().border)
                .bg(move || palette().card_bg)
        })
        .hover(|s| s.border_color(move || palette().selection_outline))
        .gap(px(6.0))
        .child(
            button()
                .accessibility_label(t!("recent.open", name = name, page = file.page + 1))
                .on_press(move || open_file(path_open.clone(), file.page))
                .w_full()
                .child(cover_box),
        )
        .child(footer)
}

fn list_row(
    number: usize,
    file: RecentFile,
    props: HomeStyleProps,
    open_file: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let recent = props.recent;
    let covers = props.covers;
    let palette = move || props.palette();
    let show_progress = move || props.show_progress();
    let path = file.path.clone();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let folder = path
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let progress = format_progress(file.page, file.total_pages);
    let path_open = path.clone();
    let path_pin = path.clone();
    let path_done = path.clone();
    let path_remove = path.clone();

    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || palette().card_bg)
        .hover(|s| s.border_color(move || palette().selection_outline))
        .child(
            button()
                .accessibility_label(t!("recent.open", name = name.clone(), page = file.page + 1))
                .on_press(move || open_file(path_open.clone(), file.page))
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    view()
                        .min_w(px(24.0))
                        .px(px(5.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(move || palette().badge_bg)
                        .items_center()
                        .justify_center()
                        .child(
                            text(number.to_string())
                                .tabular_nums()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white()),
                        ),
                )
                .child(cover(covers, path.clone(), 28.0, 38.0))
                .child(extension_badge(&path))
                .child(
                    text(name.clone())
                        .max_w(px(280.0))
                        .truncate()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(folder)
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(when(show_progress, move || {
                    text(progress.clone())
                        .tabular_nums()
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground)
                })),
        )
        .child(
            button()
                .accessibility_label(t!("recent.pin", name = name.clone()))
                .on_press(move || recent.set_pinned(path_pin.clone(), !file.pinned))
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(6.0))
                .bg(move || {
                    if file.pinned {
                        palette().brand_cta_bg
                    } else {
                        Hsla::transparent_black().into()
                    }
                })
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .child(pin_mark(move || {
                    if file.pinned {
                        palette().brand_cta_fg
                    } else {
                        theme().muted_foreground
                    }
                })),
        )
        .child(
            button()
                .accessibility_label(t!(
                    if file.completed {
                        "home.mark_active"
                    } else {
                        "home.mark_completed"
                    },
                    name = name.clone()
                ))
                .on_press(move || recent.set_completed(path_done.clone(), !file.completed))
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(6.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().muted))
                .child(
                    text(if file.completed { "↺" } else { "✓" })
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(crate::shell::app_menu::close_x_button(
            t!("recent.remove", name = name),
            26.0,
            12.0,
            13.0,
            move || recent.remove(path_remove.clone()),
        ))
}

/// The workspaces, each a chip with its colour, icon, name and tab count; pressing one switches to it.
fn workspace_row(workspaces: Workspaces) -> impl IntoElement {
    view().w_full().items_center().gap(px(4.0)).child(
        view()
            .flex_row()
            .flex_wrap()
            .justify_center()
            .gap_2()
            .child(each(
                move || workspaces.list(),
                |workspace: &Workspace| workspace.id,
                move |workspace, _| workspace_chip(workspaces, workspace),
            )),
    )
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
        .h(px(32.0))
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
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(
            text(count)
                .flex_shrink(0.0)
                .tabular_nums()
                .text_size(px(11.5))
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, seen: i64, open_count: usize, pinned: bool, completed: bool) -> RecentFile {
        RecentFile {
            path: PathBuf::from(format!("C:/Docs/{name}.pdf")),
            page: 4,
            seen,
            pinned,
            total_pages: 42,
            open_count,
            completed,
        }
    }

    #[test]
    fn home_style_and_view_mode_default_to_enhanced_warm_and_thumbnails() {
        assert_eq!(HomeDesignStyle::default(), HomeDesignStyle::EnhancedWarm);
        assert_eq!(HomeDesignStyle::parse(None), HomeDesignStyle::EnhancedWarm);
        assert_eq!(
            HomeDesignStyle::parse(Some("modern_green")),
            HomeDesignStyle::ModernGreen
        );
        assert_eq!(
            HomeDesignStyle::parse(Some("classic_clean")),
            HomeDesignStyle::ClassicClean
        );
        assert_eq!(HomeViewMode::default(), HomeViewMode::Thumbnails);
        assert_eq!(HomeViewMode::parse(None), HomeViewMode::Thumbnails);
        assert_eq!(HomeViewMode::parse(Some("list")), HomeViewMode::List);
    }

    #[test]
    fn settings_clamp_thumbnail_size_and_format_progress() {
        let mut settings = Settings::default();
        assert_eq!(read_thumb_size(&settings), 100);
        settings.set_number(HOME_THUMBNAIL_SIZE, Some(10.0));
        assert_eq!(read_thumb_size(&settings), MIN_HOME_THUMB_SIZE);
        settings.set_number(HOME_THUMBNAIL_SIZE, Some(999.0));
        assert_eq!(read_thumb_size(&settings), MAX_HOME_THUMB_SIZE);

        assert_eq!(format_progress(40, 675), "41/675");
        assert_eq!(format_progress(0, 0), "p. 1");
    }

    #[test]
    fn filter_and_partition_separates_completed_and_respects_frequency_and_pins() {
        let files = vec![
            file("alpha", 10, 2, false, false),
            file("beta", 5, 9, false, false),
            file("pinned", 1, 1, true, false),
            file("finished", 20, 4, false, true),
        ];
        let (active_rec, completed) = filter_and_partition(&files, "", false, 100);
        assert_eq!(
            active_rec
                .iter()
                .map(|f| f
                    .path
                    .file_stem()
                    .expect("stem")
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>(),
            vec!["pinned", "alpha", "beta"]
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].path, PathBuf::from("C:/Docs/finished.pdf"));

        let (active_freq, _) = filter_and_partition(&files, "", true, 100);
        assert_eq!(
            active_freq
                .iter()
                .map(|f| f
                    .path
                    .file_stem()
                    .expect("stem")
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>(),
            vec!["pinned", "beta", "alpha"]
        );
    }
}
