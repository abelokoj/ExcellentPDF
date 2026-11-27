//! The settings window (K.16 step 3, plan §9.2) in EcclesiaBible's nsh.dev documentation style:
//! a left navigation sidebar with a 3 px primary accent bar on the active section and a live search
//! field over every setting, paired with a scrollable right content pane showing a section heading,
//! summary, and grouped cards of hairline-divided rows (label and description on the left, control
//! on the right).
//!
//! On desktop platforms Settings opens in its own movable native OS window (`WindowKind::Normal`,
//! with system Minimize, Maximize, and Close controls) so the document stays visible beside it;
//! where multi-window hosting is unavailable (such as headless tests) it falls back to a centred
//! in-window modal dialog. Every change takes effect and persists immediately, so there is no Save
//! or Cancel button; Escape or the window's close button closes Settings.

use ecclesia_core::darkmode::{Palette, Rgb};
use ecclesia_core::formats::{DEFAULT_FONT_SIZE, FONT_SIZES};
use ecclesia_core::print::Scaling;
use ecclesia_features::read::autoscroll::{DEFAULT_SPEED, SPEEDS};
use ecclesia_features::read::links::DEFAULT_PROTOCOLS;
use ecclesia_features::read::ruler::{self, MAX_HEIGHT, MIN_HEIGHT};
use ecclesia_features::split::Follow;
use nsh_base::{SliderValue, button, input, number_input, slider, switch, theme, toggle};
use nsh_core::desktop::WindowKind;
use nsh_core::element::desktop::{OpenChange, WindowRef, current_window, open_window};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::annotate::selection_popup::{ColorPickerIcon, PopupItem, selection_popup_settings_rows};
use crate::autoreload::{AUTO_RELOAD_MODE, AutoReloadMode};
use crate::commands::{COMMANDS, CommandId};
use crate::home::{
    DEFAULT_HOME_MAX_RECENT, DEFAULT_HOME_THUMB_SIZE, HOME_DESIGN_STYLE, HOME_FIXED_HEADER,
    HOME_FULL_PAGE_THUMBNAILS, HOME_MAX_RECENT, HOME_SHOW_PROGRESS, HOME_SORT_BY_FREQUENCY,
    HOME_THUMBNAIL_PADDING, HOME_THUMBNAIL_SIZE, HOME_VIEW_MODE, HomeDesignStyle, HomeViewMode,
    MAX_HOME_MAX_RECENT, MAX_HOME_THUMB_SIZE, MIN_HOME_MAX_RECENT, MIN_HOME_THUMB_SIZE,
};
use crate::keymap::{self, Scope};
use crate::links::PROTOCOLS_KEY;
use crate::list_dialog::{card, list_dialog};
use crate::modes::{FINGER_DRAWS, PEN_TO_WRITE};
use crate::onboarding::SHOW_TIPS;
use crate::packs::{self, PACKS_INDEX_URL, PacksState, format_size};
use crate::panes::{CLOSE_PANE_TABS, ClosingTabs, SYNC_SCROLL, SYNC_ZOOM};
use crate::print::{PRINT_ANNOTATIONS, PRINT_SCALING};
use crate::reading::{
    AUTO_SCROLL_SPEED, DEFAULT_PREVIEW_DELAY, DEFAULT_PREVIEW_MAX_WIDTH, DocumentColors,
    FOCUS_SCOPE, MAX_PREVIEW_DELAY, MAX_PREVIEW_MAX_WIDTH, MIN_PREVIEW_MAX_WIDTH, PAPER_TEXTURE,
    READING_INK, READING_PAPER, REF_PREVIEW, REF_PREVIEW_DELAY, REF_PREVIEW_MAX_WIDTH,
    REFLOW_FONT_FAMILY, REFLOW_FONT_SIZE, REFLOW_USE_DOCUMENT_CSS, REFLOW_USER_CSS_PATH,
    RULER_COLOR, RULER_HEIGHT, RULER_INVERT, RULER_OPACITY, Reading, hex, parse_hex,
};
use crate::reading_theme_bar;
use crate::settings::{self, Live};
use crate::shell::app_menu::{ChromeIcon, chrome_icon, dropdown_arrow};
use crate::tabs::{
    Cycle, DEFAULT_TAB_FONT_SIZE_PX, DEFAULT_TAB_HEIGHT_PX, DEFAULT_TAB_MIN_WIDTH_PX,
    MAX_TAB_FONT_SIZE_PX, MAX_TAB_HEIGHT_PX, MAX_TAB_MIN_WIDTH_PX, MIN_TAB_FONT_SIZE_PX,
    MIN_TAB_HEIGHT_PX, MIN_TAB_MIN_WIDTH_PX, TAB_CYCLE_ORDER, TAB_FONT_SIZE_PX, TAB_HEIGHT_PX,
    TAB_MIN_WIDTH_PX, TAB_STYLE, TabStyle,
};
use crate::theme::{self as themes, ThemeChoice};
use crate::workspaces::{Elsewhere, OPEN_ELSEWHERE};

/// Outer settings dialog width in logical pixels (fits cleanly inside a 1280 px desktop viewport).
const DIALOG_WIDTH: f32 = 960.0;

/// Outer settings dialog height in logical pixels (leaves clickable backdrop margins at 800 px).
const DIALOG_HEIGHT: f32 = 640.0;

/// Default width of the standalone settings OS window in logical pixels.
const STANDALONE_WIDTH: f32 = 980.0;

/// Default height of the standalone settings OS window in logical pixels.
const STANDALONE_HEIGHT: f32 = 680.0;

/// Left section navigation sidebar width in logical pixels.
const SIDEBAR_WIDTH: f32 = 248.0;

/// Height of each section navigation button in the sidebar.
const SIDEBAR_ROW_HEIGHT: f32 = 34.0;

/// Corner radius of grouped settings cards (matching EcclesiaBible's `size::CARD_RADIUS`).
const CARD_RADIUS: f32 = 10.0;

/// Corner radius of standard controls and sidebar items (matching EcclesiaBible's `size::RADIUS`).
const CONTROL_RADIUS: f32 = 6.0;

/// Width of the active section's vertical left accent bar.
const ACCENT_BAR_WIDTH: f32 = 3.0;

/// Vertical inset of the active section's left accent bar.
const ACCENT_BAR_INSET_Y: f32 = 7.0;

/// The ruler colours offered: light tints that keep black text readable (content colours, not the theme's).
const RULER_COLORS: [(&str, &str); 5] = [
    ("#ffe082", "settings.ruler_color.yellow"),
    ("#c5e1a5", "settings.ruler_color.green"),
    ("#90caf9", "settings.ruler_color.blue"),
    ("#f8bbd0", "settings.ruler_color.pink"),
    ("#cfd8dc", "settings.ruler_color.grey"),
];

/// The focus-mode scopes, as `focus_scope` names them.
const FOCUS_SCOPES: [(&str, &str); 2] = [
    ("paragraph", "settings.focus_scope.paragraph"),
    ("line", "settings.focus_scope.line"),
];

/// Tailwind classes for the slider track, range, and thumb, which have no default look.
pub(crate) const SLIDER_TRACK: &str = "h-1 rounded-full bg-muted";
/// Tailwind classes for the filled range portion of a slider.
pub(crate) const SLIDER_RANGE: &str = "h-1 rounded-full bg-primary";
/// Tailwind classes for the draggable thumb of a slider.
pub(crate) const SLIDER_THUMB: &str =
    "size-4 rounded-full bg-background border-2 border-primary focus-visible:border-ring";
/// Tailwind classes for the switch track.
pub(crate) const SWITCH_TRACK: &str = "w-9 h-5 p-0.5 rounded-full bg-input checked:bg-primary \
                            focus-visible:ring-2 focus-visible:ring-ring";
/// Tailwind classes for the switch thumb.
pub(crate) const SWITCH_THUMB: &str = "size-4 rounded-full bg-background shadow-sm checked:ml-4";

/// Top-level sections listed in the settings sidebar, split into everyday General sections
/// and the dedicated Advanced section (mirroring SumatraPDF Enhanced's Advanced Settings split).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsSection {
    /// Theme family, light/dark appearance, language, and system high contrast.
    #[default]
    Appearance,
    /// Document colours, paper grain, reading theme, reflow text size, ruler, focus, auto-scroll.
    Reading,
    /// Tab bar style, dimensions, cycling, split-pane closing, sync scroll/zoom, and workspaces.
    TabsAndPanes,
    /// Home page design style, layout, thumbnail size, sort order, and recent document limit.
    HomePage,
    /// Annotation palette, tool colours, margin notes, polygon/star counts, modes, and toolbars.
    Annotations,
    /// Page print scaling, print annotations, and citation/equation hover previews.
    PrintAndLinks,
    /// Active keyboard shortcuts and direct editing of `settings.toml`.
    Shortcuts,
    /// Contextual tips and replaying the getting-started guide.
    Help,
    /// Power-user settings: sandbox/security, link protocols, packs, AI/CAD/TSA, automation,
    /// and the live `settings.toml` override inspector.
    Advanced,
}

impl SettingsSection {
    /// Everyday user-facing sections shown in the top group of the sidebar.
    const GENERAL: [Self; 8] = [
        Self::Appearance,
        Self::Reading,
        Self::TabsAndPanes,
        Self::HomePage,
        Self::Annotations,
        Self::PrintAndLinks,
        Self::Shortcuts,
        Self::Help,
    ];

    /// Power-user sections shown beneath the hairline divider in the sidebar.
    const ADVANCED: [Self; 1] = [Self::Advanced];

    /// Every section in search traversal order.
    const ALL: [Self; 9] = [
        Self::Appearance,
        Self::Reading,
        Self::TabsAndPanes,
        Self::HomePage,
        Self::Annotations,
        Self::PrintAndLinks,
        Self::Shortcuts,
        Self::Help,
        Self::Advanced,
    ];

    /// Localization key for the section title shown in the sidebar and page heading.
    const fn title_key(self) -> &'static str {
        match self {
            Self::Appearance => "settings.section.appearance",
            Self::Reading => "settings.section.reading",
            Self::TabsAndPanes => "settings.section.tabs_and_panes",
            Self::HomePage => "settings.section.home",
            Self::Annotations => "settings.section.annotations",
            Self::PrintAndLinks => "settings.section.print_and_links",
            Self::Shortcuts => "settings.section.shortcuts",
            Self::Help => "settings.section.help_and_guide",
            Self::Advanced => "settings.section.advanced_area",
        }
    }

    /// Localization key for the one-sentence subtitle beneath the page heading.
    const fn summary_key(self) -> &'static str {
        match self {
            Self::Appearance => "settings.summary.appearance",
            Self::Reading => "settings.summary.reading",
            Self::TabsAndPanes => "settings.summary.tabs_and_panes",
            Self::HomePage => "settings.summary.home",
            Self::Annotations => "settings.summary.annotations",
            Self::PrintAndLinks => "settings.summary.print_and_links",
            Self::Shortcuts => "settings.summary.shortcuts",
            Self::Help => "settings.summary.help",
            Self::Advanced => "settings.summary.advanced",
        }
    }

    /// Monochrome chrome icon representing this section in the sidebar.
    const fn icon(self) -> ChromeIcon {
        match self {
            Self::Appearance => ChromeIcon::RailSettings,
            Self::Reading => ChromeIcon::RailLibrary,
            Self::TabsAndPanes => ChromeIcon::SidebarLeft,
            Self::HomePage => ChromeIcon::RailPages,
            Self::Annotations => ChromeIcon::RailAnnotations,
            Self::PrintAndLinks => ChromeIcon::RailBookmarks,
            Self::Shortcuts => ChromeIcon::RailSearch,
            Self::Help => ChromeIcon::RailBookmarks,
            Self::Advanced => ChromeIcon::RailOutline,
        }
    }

    /// Localization keys `(label, hint)` of every setting row in this section, used for live search.
    const fn row_keys(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Appearance => &[
                ("settings.theme", "settings.theme_hint"),
                ("settings.appearance", "settings.appearance_hint"),
                ("settings.language", "settings.language_hint"),
                (
                    "settings.follow_system_contrast",
                    "settings.follow_system_contrast_hint",
                ),
            ],
            Self::Reading => &[
                ("settings.document_colors", "settings.document_colors_hint"),
                ("settings.paper_texture", "settings.paper_texture_hint"),
                ("settings.reading_theme", "settings.reading_theme_hint"),
                ("settings.reading_custom", "settings.reading_custom_hint"),
                (
                    "settings.reflow_font_size",
                    "settings.reflow_font_size_hint",
                ),
                (
                    "settings.reflow_font_family",
                    "settings.reflow_font_family_hint",
                ),
                (
                    "settings.reflow_use_document_css",
                    "settings.reflow_use_document_css_hint",
                ),
                ("settings.reflow_user_css", "settings.reflow_user_css_hint"),
                ("settings.ruler_color", "settings.ruler_color_hint"),
                ("settings.ruler_height", "settings.ruler_height_hint"),
                ("settings.ruler_opacity", "settings.ruler_opacity_hint"),
                ("settings.ruler_invert", "settings.ruler_invert_hint"),
                ("settings.focus_scope", "settings.focus_scope_hint"),
                (
                    "settings.auto_scroll_speed",
                    "settings.auto_scroll_speed_hint",
                ),
                ("settings.auto_reload", "settings.auto_reload_hint"),
            ],
            Self::TabsAndPanes => &[
                ("settings.tab_style", "settings.tab_style_hint"),
                ("settings.tab_min_width", "settings.tab_min_width_hint"),
                ("settings.tab_height", "settings.tab_height_hint"),
                ("settings.tab_font_size", "settings.tab_font_size_hint"),
                ("settings.tab_cycle", "settings.tab_cycle_hint"),
                ("settings.close_pane_tabs", "settings.close_pane_tabs_hint"),
                ("settings.sync_scroll", "settings.sync_scroll_hint"),
                ("settings.sync_zoom", "settings.sync_zoom_hint"),
                ("settings.open_elsewhere", "settings.open_elsewhere_hint"),
            ],
            Self::HomePage => &[
                (
                    "settings.home_design_style",
                    "settings.home_design_style_hint",
                ),
                ("settings.home_view_mode", "settings.home_view_mode_hint"),
                ("settings.home_thumb_size", "settings.home_thumb_size_hint"),
                (
                    "settings.home_sort_frequency",
                    "settings.home_sort_frequency_hint",
                ),
                (
                    "settings.home_thumb_padding",
                    "settings.home_thumb_padding_hint",
                ),
                (
                    "settings.home_show_progress",
                    "settings.home_show_progress_hint",
                ),
                ("settings.home_full_page", "settings.home_full_page_hint"),
                (
                    "settings.home_fixed_header",
                    "settings.home_fixed_header_hint",
                ),
                ("settings.home_max_recent", "settings.home_max_recent_hint"),
            ],
            Self::Annotations => &[
                (
                    "settings.annotation_palette",
                    "settings.annotation_palette_hint",
                ),
                (
                    "settings.annotation_colors",
                    "settings.annotation_colors_hint",
                ),
                ("settings.margin_notes", "settings.margin_notes_hint"),
                ("settings.polygon_sides", "settings.polygon_sides_hint"),
                ("settings.star_points", "settings.star_points_hint"),
                ("settings.pen_to_write", "settings.pen_to_write_hint"),
                ("settings.finger_draws", "settings.finger_draws_hint"),
                ("settings.toolbar", "settings.toolbar_hint"),
                (
                    "settings.section.selection_popup",
                    "annotate.popup.customize_hint",
                ),
                (
                    "annotate.popup.picker_icon",
                    "annotate.popup.picker_icon_hint",
                ),
                (
                    "annotate.popup.remember_color",
                    "annotate.popup.remember_color_hint",
                ),
            ],
            Self::PrintAndLinks => &[
                ("settings.print_scaling", "settings.print_scaling_hint"),
                (
                    "settings.print_annotations",
                    "settings.print_annotations_hint",
                ),
                ("settings.ref_preview", "settings.ref_preview_hint"),
                (
                    "settings.ref_preview_max_width",
                    "settings.ref_preview_max_width_hint",
                ),
            ],
            Self::Shortcuts => &[("settings.shortcuts", "settings.shortcuts_hint")],
            Self::Help => &[
                ("settings.show_tips", "settings.show_tips_hint"),
                ("settings.guide", "settings.guide_hint"),
            ],
            Self::Advanced => &[
                ("settings.sandbox.status", "settings.sandbox.status_hint"),
                ("settings.sandbox.strict", "settings.sandbox.strict_hint"),
                (
                    "settings.sandbox.max_pages",
                    "settings.sandbox.max_pages_hint",
                ),
                (
                    "settings.sandbox.memory_mb",
                    "settings.sandbox.memory_mb_hint",
                ),
                (
                    "settings.sandbox.timeout_s",
                    "settings.sandbox.timeout_s_hint",
                ),
                ("settings.link_protocols", "settings.link_protocols_hint"),
                (
                    "settings.ref_preview_delay",
                    "settings.ref_preview_delay_hint",
                ),
                (
                    "settings.ref_preview_max_width",
                    "settings.ref_preview_max_width_hint",
                ),
                ("settings.packs.index_url", "settings.packs.index_url_hint"),
                ("settings.packs.actions", "settings.packs.actions_hint"),
                ("settings.bionic", "settings.bionic_hint"),
                ("settings.synctex", "settings.synctex_hint"),
                ("settings.neural_voice", "settings.neural_voice_hint"),
                ("settings.watch", "settings.watch_hint"),
                ("settings.workspace_rules", "settings.workspace_rules_hint"),
                ("settings.control_channel", "settings.control_channel_hint"),
                ("settings.ai.consent", "settings.ai.consent_hint"),
                ("settings.ai.provider", "settings.ai.provider_hint"),
                ("settings.ai.endpoint", "settings.ai.endpoint_hint"),
                ("settings.ai.model", "settings.ai.model_hint"),
                ("settings.ai.api_key", "settings.ai.api_key_hint"),
                ("settings.ai.redact", "settings.ai.redact_hint"),
                ("settings.ai.budget", "settings.ai.budget_hint"),
                ("settings.lens.online", "settings.lens.online_hint"),
                ("settings.cad.mode", "settings.cad.mode_hint"),
                ("settings.cad.background", "settings.cad.background_hint"),
                ("settings.cad.weights", "settings.cad.weights_hint"),
                ("settings.tsa.consent", "settings.tsa.consent_hint"),
                ("settings.tsa.url", "settings.tsa.url_hint"),
                ("settings.toml_inspector", "settings.toml_inspector_hint"),
            ],
        }
    }

    /// Card titles belonging to this section, so searching a card heading matches its rows.
    const fn card_title_keys(self) -> &'static [&'static str] {
        match self {
            Self::Appearance => &["settings.section.appearance"],
            Self::Reading => &[
                "settings.group.reading_colors",
                "settings.group.reflow_books",
                "settings.group.reading_aids",
            ],
            Self::TabsAndPanes => &[
                "settings.section.tabs",
                "settings.group.panes_and_workspaces",
            ],
            Self::HomePage => &["settings.section.home"],
            Self::Annotations => &[
                "settings.section.annotation",
                "settings.section.selection_popup",
                "settings.section.modes",
            ],
            Self::PrintAndLinks => &["settings.section.print", "settings.section.links"],
            Self::Shortcuts => &["settings.section.shortcuts"],
            Self::Help => &["settings.section.help"],
            Self::Advanced => &[
                "settings.section.sandbox",
                "settings.section.packs",
                "settings.section.advanced",
                "settings.section.ai_pro",
                "settings.group.toml_inspector",
            ],
        }
    }

    /// Returns whether the section's localized title matches `needle`.
    fn title_matches(self, needle: &str) -> bool {
        matches_query(&t!(self.title_key()), needle)
    }

    /// Returns whether any title, card heading, row, or shortcut command in this section matches `needle`.
    fn matches_query(self, needle: &str) -> bool {
        if self.title_matches(needle) {
            return true;
        }
        if self
            .card_title_keys()
            .iter()
            .any(|title| matches_query(&t!(title), needle))
        {
            return true;
        }
        if self.row_keys().iter().any(|(label, hint)| {
            matches_query(&t!(label), needle) || matches_query(&t!(hint), needle)
        }) {
            return true;
        }
        if self == Self::Shortcuts {
            return COMMANDS
                .iter()
                .any(|cmd| matches_query(&t!(cmd.label), needle));
        }
        false
    }
}

/// Structural memoization key for the right-hand content pane.
///
/// Rebuilding the right pane only when `section` or trimmed `query` changes ensures that
/// unrelated `live` settings writes (dragging sliders, editing number inputs, typing hex colours)
/// never reconstruct the view tree or drop keyboard focus.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentKey {
    section: SettingsSection,
    query: String,
}

/// Whether a row is the first inside a card or separated from the previous row by a hairline border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowDivider {
    /// Topmost row in a card (no top hairline).
    First,
    /// Subsequent row with a 1 px hairline divider above it.
    Divided,
}

/// Visual tone for shortcut keycaps, matching `Scope::Window` vs `Scope::Page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeycapScope {
    /// Window-wide shortcut keycap.
    Window,
    /// Quieter page-canvas shortcut keycap.
    Page,
}

/// A single setting row definition before card assembly and optional search filtering.
struct RowItem {
    label: &'static str,
    hint: &'static str,
    control: AnyElement,
}

impl RowItem {
    fn new(label: &'static str, hint: &'static str, control: impl IntoElement) -> Self {
        Self {
            label,
            hint,
            control: control.into_element(),
        }
    }
}

/// Presentation container for the settings UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsHost {
    /// Standalone native OS window with system title bar (minimize, maximize, close).
    StandaloneWindow,
    /// Centered in-window modal fallback (used when no desktop multi-window host is available, e.g. headless tests).
    ModalFallback,
}

/// Handle managing the Settings window, opening a movable native OS window on desktop platforms
/// and falling back to an in-window modal dialog where multi-window hosting is unavailable.
#[derive(Clone, Copy)]
pub(crate) struct SettingsWindow {
    /// Whether the in-window modal fallback dialog is currently open.
    fallback_open: Signal<bool>,
    /// Live reference to the standalone OS Settings window while it is open.
    detached: Stored<Option<WindowRef>>,
    /// Hosting window that owns this handle, focused when handing over to a main-window dialog.
    host_window: Stored<WindowRef>,
}

impl SettingsWindow {
    /// Creates a closed settings handle bound to the current window.
    pub(crate) fn new() -> Self {
        Self {
            fallback_open: signal(false),
            detached: stored(None),
            host_window: stored(current_window()),
        }
    }

    /// Returns whether the in-window modal fallback dialog is currently open.
    ///
    /// When Settings is open in its own standalone OS window, this returns `false` so the main
    /// window's PDF canvas stays visible and interactive beside Settings.
    pub(crate) fn is_modal_open(self) -> bool {
        self.fallback_open.get()
    }

    /// Opens Settings in a standalone movable OS window with native Minimize, Maximize, and Close
    /// controls, raising the existing window if already open. Falls back to the in-window modal
    /// dialog when no desktop multi-window host is available.
    pub(crate) fn open(
        self,
        live: Live,
        theme_choice: Signal<ThemeChoice>,
        choose: impl Fn(CommandId) + Copy + 'static,
    ) {
        if let Some(existing) = self.detached.get() {
            existing.focus();
            return;
        }

        let win = nsh_core::element::desktop::window()
            .id("settings")
            .kind(WindowKind::Normal)
            .title(move || t!("settings.window_title"))
            .default_size(size(px(STANDALONE_WIDTH), px(STANDALONE_HEIGHT)))
            .restore_bounds(true)
            .on_open_change(move |event: &OpenChange| {
                if !event.open {
                    let _ = self.detached.try_with(|slot| *slot = None);
                }
            })
            .content(move || {
                let close = move || self.close();
                let hand_over = move |id: CommandId| {
                    self.close();
                    if let Some(host) = self.host_window.try_with(|w| w.clone()) {
                        host.focus();
                    }
                    choose(id);
                };
                let in_fields = KeyOptions {
                    when: When::Always,
                    ..KeyOptions::default()
                };
                on_key_with("escape", in_fields, move |_| close());
                let body = settings_body(
                    live,
                    theme_choice,
                    hand_over,
                    close,
                    SettingsHost::StandaloneWindow,
                );
                view()
                    .flex_1()
                    .direction(move || live.read(crate::l10n::direction))
                    .bg(move || theme().background)
                    .child(body)
            });

        match open_window(win) {
            Some(window_ref) => {
                let _ = self.detached.replace(Some(window_ref));
                self.fallback_open.set(false);
            }
            None => {
                self.fallback_open.set(true);
            }
        }
    }

    /// Closes Settings, whether presented as a standalone OS window or an in-window modal fallback.
    pub(crate) fn close(self) {
        let _ = self.fallback_open.try_set(false);
        if let Some(window) = self.detached.try_with(Option::take).flatten() {
            window.close();
        }
    }

    /// Renders the centered in-window modal fallback dialog when multi-window hosting is unavailable.
    pub(crate) fn fallback_dialog(
        self,
        live: Live,
        theme_choice: Signal<ThemeChoice>,
        choose: impl Fn(CommandId) + Copy + 'static,
    ) -> impl IntoElement {
        let close = move || self.close();
        let hand_over = move |id: CommandId| {
            close();
            choose(id);
        };
        let body = settings_body(
            live,
            theme_choice,
            hand_over,
            close,
            SettingsHost::ModalFallback,
        );
        list_dialog(
            t!("settings.title"),
            close,
            card(body).w(px(DIALOG_WIDTH)).h(px(DIALOG_HEIGHT)),
        )
    }
}

/// Shared two-pane settings layout (left navigation sidebar + scrollable right content pane).
fn settings_body(
    live: Live,
    theme_choice: Signal<ThemeChoice>,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    close: impl Fn() + Copy + 'static,
    host: SettingsHost,
) -> View {
    let active_section = signal(SettingsSection::default());
    let search_query = signal(String::new());
    let content_key = signal(ContentKey {
        section: SettingsSection::default(),
        query: String::new(),
    });

    effect(move || {
        let next = ContentKey {
            section: active_section.get(),
            query: search_query.with(|q| q.trim().to_owned()),
        };
        if content_key.with_untracked(|cur| *cur != next) {
            content_key.set(next);
        }
    });

    let right_pane = view()
        .flex_1()
        .min_w(px(0.0))
        .bg(move || theme().background)
        .child(
            scroll().flex_1().scrollbar(true).child(
                view()
                    .w_full()
                    .px(px(28.0))
                    .py(px(24.0))
                    .child(dynamic(move || {
                        let key = content_key.get();
                        untracked(move || {
                            content_view(&key, live, theme_choice, hand_over, close, host)
                        })
                        .into_element()
                    })),
            ),
        );

    view()
        .flex_1()
        .flex_row()
        .min_h(px(0.0))
        .child(sidebar(active_section, search_query))
        .child(right_pane)
}

/// Subtle muted group caption in the settings sidebar.
fn sidebar_group_label(key: &'static str) -> View {
    view().px(px(10.0)).pt(px(8.0)).pb(px(4.0)).child(
        text(move || t!(key))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().muted_foreground),
    )
}

/// Left sidebar with Settings title, live search bar, grouped section list (General vs. Advanced), and footer.
fn sidebar(active_section: Signal<SettingsSection>, search_query: Signal<String>) -> View {
    let mut nav_list = view()
        .gap(px(2.0))
        .px(px(10.0))
        .py(px(4.0))
        .child(sidebar_group_label("settings.nav.general"));
    for section in SettingsSection::GENERAL {
        nav_list = nav_list.child(sidebar_row(section, active_section, search_query));
    }
    nav_list = nav_list
        .child(
            view()
                .my(px(8.0))
                .mx(px(10.0))
                .h(px(1.0))
                .bg(move || theme().border),
        )
        .child(sidebar_group_label("settings.nav.advanced"));
    for section in SettingsSection::ADVANCED {
        nav_list = nav_list.child(sidebar_row(section, active_section, search_query));
    }

    let header = view()
        .px(px(16.0))
        .pt(px(18.0))
        .pb(px(12.0))
        .gap(px(10.0))
        .child(
            text(move || t!("settings.title"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().sidebar_foreground),
        )
        .child(search_field(search_query));

    let footer = view()
        .p(px(12.0))
        .gap(px(8.0))
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(
            text(move || t!("settings.saved_as_you_go"))
                .text_size(px(11.5))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            button()
                .accessibility_label(t!("settings.open_file"))
                .on_press(open_settings_file)
                .h(px(30.0))
                .px(px(10.0))
                .rounded(px(CONTROL_RADIUS))
                .items_center()
                .justify_center()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .focus_visible(|s| s.border_color(move || theme().ring))
                .child(
                    text(move || t!("settings.open_file"))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().sidebar_foreground),
                ),
        );

    view()
        .w(px(SIDEBAR_WIDTH))
        .flex_shrink_0()
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_r_color(move || theme().border)
        .child(header)
        .child(scroll().flex_1().child(nav_list))
        .child(footer)
}

/// Search field in the sidebar header that filters across all settings live.
fn search_field(search_query: Signal<String>) -> View {
    let has_query = move || search_query.with(|q| !q.is_empty());
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(8.0))
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(chrome_icon(ChromeIcon::RailSearch, 14.0, move || {
            theme().muted_foreground
        }))
        .child(
            input()
                .flex_1()
                .min_w(px(0.0))
                .placeholder(t!("settings.search_placeholder"))
                .label(t!("settings.search_placeholder"))
                .text_size(px(13.0))
                .text_color(move || theme().foreground)
                .value(move || search_query.get())
                .on_change(move |value: &str| search_query.set(value.to_owned())),
        )
        .child(when(has_query, move || {
            crate::shell::app_menu::close_x_button(
                t!("settings.search_clear"),
                18.0,
                10.0,
                9.0,
                move || search_query.set(String::new()),
            )
        }))
}

/// One section row in the left sidebar with EcclesiaBible's 3 px left primary accent bar.
fn sidebar_row(
    section: SettingsSection,
    active_section: Signal<SettingsSection>,
    search_query: Signal<String>,
) -> View {
    let active =
        move || search_query.with(|q| q.trim().is_empty()) && active_section.get() == section;
    view()
        .relative()
        .h(px(SIDEBAR_ROW_HEIGHT))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(10.0))
        .rounded(px(CONTROL_RADIUS))
        .cursor_pointer()
        .role(AccessibilityRole::Button)
        .label(t!(section.title_key()))
        .accessibility_state(move || AccessibilityState {
            selected: active(),
            ..AccessibilityState::default()
        })
        .style_when(active, |s| s.bg(move || theme().accent))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .on_press(move || {
            active_section.set(section);
            search_query.set(String::new());
        })
        .child(
            view()
                .absolute()
                .left_0()
                .top(px(ACCENT_BAR_INSET_Y))
                .bottom(px(ACCENT_BAR_INSET_Y))
                .w(px(ACCENT_BAR_WIDTH))
                .rounded(px(1.5))
                .bg(move || theme().primary)
                .opacity(move || if active() { 1.0_f32 } else { 0.0 }),
        )
        .child(chrome_icon(section.icon(), 15.0, move || {
            if active() {
                theme().primary
            } else {
                theme().muted_foreground
            }
        }))
        .child(
            text(move || t!(section.title_key()))
                .flex_1()
                .number_of_lines(1u32)
                .text_size(px(13.0))
                .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || {
                    if active() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

/// Renders either the active section's cards or live search results across every section.
fn content_view(
    key: &ContentKey,
    live: Live,
    theme_choice: Signal<ThemeChoice>,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    close: impl Fn() + Copy + 'static,
    host: SettingsHost,
) -> View {
    let mut page = view().w_full().gap(px(22.0));
    if key.query.is_empty() {
        page = page.child(page_heading(
            key.section.title_key(),
            key.section.summary_key(),
            host,
            close,
        ));
        for card_view in section_cards(key.section, None, live, theme_choice, hand_over) {
            page = page.child(card_view);
        }
        return page;
    }

    let needle = key.query.to_lowercase();
    page = page.child(page_heading(
        "settings.search_results",
        "settings.search_results_hint",
        host,
        close,
    ));

    let mut matched_cards = 0usize;
    for section in SettingsSection::ALL {
        if !section.matches_query(&needle) {
            continue;
        }
        let row_filter = if section.title_matches(&needle) {
            None
        } else {
            Some(needle.as_str())
        };
        for card_view in section_cards(section, row_filter, live, theme_choice, hand_over) {
            matched_cards += 1;
            page = page.child(card_view);
        }
    }

    if matched_cards == 0 {
        let display_query = key.query.clone();
        page = page.child(
            view()
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().popover)
                .px(px(20.0))
                .py(px(24.0))
                .child(
                    text(move || t!("settings.search_no_results", query = display_query.as_str()))
                        .text_size(px(13.5))
                        .text_color(move || theme().muted_foreground),
                ),
        );
    }

    page
}

/// Page heading row: section title and summary on the left, plus an inline close button in modal fallback mode.
fn page_heading(
    title_key: &'static str,
    summary_key: &'static str,
    host: SettingsHost,
    close: impl Fn() + Copy + 'static,
) -> View {
    let heading = view()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(px(16.0))
        .pb(px(4.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(6.0))
                .child(
                    text(move || t!(title_key))
                        .text_size(px(24.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .letter_spacing(px(-0.4))
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || t!(summary_key))
                        .text_size(px(13.5))
                        .text_color(move || theme().muted_foreground),
                ),
        );
    match host {
        SettingsHost::StandaloneWindow => heading,
        SettingsHost::ModalFallback => heading.child(close_button(close)),
    }
}

/// Top-right close icon button for the settings pane.
fn close_button(close: impl Fn() + 'static) -> impl IntoElement {
    crate::shell::app_menu::close_x_button(
        t!("tabs.properties.close"),
        28.0,
        13.0,
        CONTROL_RADIUS,
        close,
    )
}

/// Builds the grouped cards for one section, optionally filtering rows by a lowercase search query.
fn section_cards(
    section: SettingsSection,
    query: Option<&str>,
    live: Live,
    theme_choice: Signal<ThemeChoice>,
    hand_over: impl Fn(CommandId) + Copy + 'static,
) -> Vec<View> {
    match section {
        SettingsSection::Appearance => appearance_cards(live, theme_choice, hand_over, query),
        SettingsSection::Reading => reading_cards(live, hand_over, query),
        SettingsSection::TabsAndPanes => tabs_and_panes_cards(live, query),
        SettingsSection::HomePage => home_cards(live, query),
        SettingsSection::Annotations => annotations_cards(live, hand_over, query),
        SettingsSection::PrintAndLinks => print_and_links_cards(live, query),
        SettingsSection::Shortcuts => shortcuts_cards(live, query),
        SettingsSection::Help => help_cards(live, hand_over, query),
        SettingsSection::Advanced => advanced_cards(live, hand_over, query),
    }
}

/// Case-insensitive substring match helper for live settings search.
fn matches_query(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// Opens `settings.toml` in the system's default editor.
fn open_settings_file() {
    if let Some(path) = settings::file_to_edit() {
        spawn(async move {
            if let Err(err) = open_path(path).await {
                log::warn!("settings: can't open the settings file: {err}");
            }
        });
    }
}

/// Builds a grouped card from `items`, filtering rows when `query` is `Some` and does not match the card title.
fn card_from_rows(
    title_key: &'static str,
    query: Option<&str>,
    items: Vec<RowItem>,
) -> Option<View> {
    let row_query = query.filter(|needle| !matches_query(&t!(title_key), needle));
    let filtered: Vec<RowItem> = match row_query {
        None => items,
        Some(needle) => items
            .into_iter()
            .filter(|item| {
                matches_query(&t!(item.label), needle) || matches_query(&t!(item.hint), needle)
            })
            .collect(),
    };
    if filtered.is_empty() {
        return None;
    }
    let rows = filtered
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let divider = if idx == 0 {
                RowDivider::First
            } else {
                RowDivider::Divided
            };
            card_row(divider, item.label, item.hint, item.control)
        })
        .collect();
    Some(settings_card(title_key, rows))
}

/// A titled group of hairline-divided rows inside a rounded popover card.
fn settings_card(title_key: &'static str, rows: Vec<View>) -> View {
    let mut body = view()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .overflow_hidden();
    for row in rows {
        body = body.child(row);
    }
    view()
        .gap(px(8.0))
        .child(
            text(move || t!(title_key))
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(body)
}

/// A setting row inside a card: label and description on the left, control on the right.
fn card_row(
    divider: RowDivider,
    label: &'static str,
    hint: &'static str,
    control: impl IntoElement,
) -> View {
    let base = view()
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .px(px(16.0))
        .py(px(12.0));
    let styled = match divider {
        RowDivider::First => base,
        RowDivider::Divided => base.border_t_1().border_t_color(move || theme().border),
    };
    styled
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(
                    text(move || t!(label))
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || t!(hint))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(view().flex_shrink_0().child(control))
}

// ---------------------------------------------------------------------------
// Section 1: Appearance
// ---------------------------------------------------------------------------

fn appearance_cards(
    live: Live,
    theme_choice: Signal<ThemeChoice>,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Vec<View> {
    let theme_name = move || {
        let choice = theme_choice.get();
        match choice.contrast {
            themes::Contrast::High => t!("settings.theme_high", name = choice.family.name()),
            themes::Contrast::Standard => choice.family.name(),
        }
    };
    let rows = vec![
        RowItem::new(
            "settings.theme",
            "settings.theme_hint",
            picker_button(theme_name, move || hand_over(CommandId::ChooseTheme)),
        ),
        RowItem::new(
            "settings.appearance",
            "settings.appearance_hint",
            picker_button(
                move || themes::appearance_name(theme_choice.get()),
                move || hand_over(CommandId::ChooseAppearance),
            ),
        ),
        RowItem::new(
            "settings.language",
            "settings.language_hint",
            language_choice(live),
        ),
        RowItem::new(
            "settings.follow_system_contrast",
            "settings.follow_system_contrast_hint",
            on_off(
                "settings.follow_system_contrast",
                move || live.read(|s| s.flag(themes::FOLLOW_SYSTEM_CONTRAST).unwrap_or(true)),
                move |on| {
                    live.change(|s| {
                        s.set_flag(themes::FOLLOW_SYSTEM_CONTRAST, (!on).then_some(false));
                    });
                },
            ),
        ),
    ];
    card_from_rows("settings.section.appearance", query, rows)
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// Section 2: Reading
// ---------------------------------------------------------------------------

fn reading_cards(
    live: Live,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Vec<View> {
    let default = ruler::Style::default();
    let default_color = format!("#{:06x}", default.color);
    let color = move || {
        live.read(|s| s.text(RULER_COLOR).map(str::to_ascii_lowercase))
            .unwrap_or_else(|| default_color.clone())
    };
    let swatches = RULER_COLORS
        .iter()
        .fold(view().flex_row().gap_2(), |swatches, &(hex, name)| {
            let color = color.clone();
            swatches.child(
                toggle()
                    .pressed(move || color() == hex)
                    .accessibility_label(t!(name))
                    .on_change(move |_: &bool| {
                        live.change(|s| {
                            let is_default = hex == format!("#{:06x}", default.color);
                            s.set_text(RULER_COLOR, (!is_default).then_some(hex));
                        });
                    })
                    .size(px(26.0))
                    .rounded_full()
                    .bg(Hsla::parse_hex(hex).expect("a built-in colour"))
                    .border_1()
                    .border_color(move || theme().border)
                    .pressed_style(|s| s.border_2().border_color(move || theme().foreground))
                    .focus_visible(|s| s.border_2().border_color(move || theme().ring)),
            )
        });
    let opacity_shown = signal(None::<f64>);
    let opacity = move || {
        opacity_shown.get().unwrap_or_else(|| {
            live.read(|s| s.number(RULER_OPACITY))
                .unwrap_or(f64::from(default.opacity))
        })
    };
    let scope = move || {
        live.read(|s| s.text(FOCUS_SCOPE).map(str::to_owned))
            .unwrap_or_else(|| FOCUS_SCOPES[0].0.to_owned())
    };
    let scopes = FOCUS_SCOPES.iter().fold(segmented(), |group, &(id, name)| {
        group.child(segment(
            name,
            move || scope() == id,
            move || {
                live.change(|s| s.set_text(FOCUS_SCOPE, (id != FOCUS_SCOPES[0].0).then_some(id)));
            },
        ))
    });
    let reading = Reading::new(live);
    let document_colors = DocumentColors::ALL
        .into_iter()
        .fold(segmented(), |group, colors| {
            group.child(segment(
                colors.label(),
                move || reading.document_colors() == colors,
                move || reading.set_document_colors(colors),
            ))
        });
    let contrast = move || {
        let palette = reading.custom_palette();
        let ratio = format!("{:.1}", palette.text_contrast());
        if palette.passes_aa() {
            t!("settings.reading_custom_contrast", ratio = ratio)
        } else {
            t!("settings.reading_custom_contrast_low", ratio = ratio)
        }
    };

    let colors_card = card_from_rows(
        "settings.group.reading_colors",
        query,
        vec![
            RowItem::new(
                "settings.document_colors",
                "settings.document_colors_hint",
                document_colors,
            ),
            RowItem::new(
                "settings.paper_texture",
                "settings.paper_texture_hint",
                on_off(
                    "settings.paper_texture",
                    move || live.read(|s| s.flag(PAPER_TEXTURE).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(PAPER_TEXTURE, on.then_some(true))),
                ),
            ),
            RowItem::new(
                "settings.reading_theme",
                "settings.reading_theme_hint",
                picker_button(
                    move || t!(reading_theme_bar::label(reading.reading_theme())),
                    move || hand_over(CommandId::ChooseReadingTheme),
                ),
            ),
            RowItem::new(
                "settings.reading_custom",
                "settings.reading_custom_hint",
                view()
                    .items_end()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(color_field(
                                reading,
                                READING_PAPER,
                                "settings.reading_paper",
                                |p| p.paper,
                            ))
                            .child(color_field(
                                reading,
                                READING_INK,
                                "settings.reading_ink",
                                |p| p.ink,
                            )),
                    )
                    // Said in words as well as colour.
                    .child(text(contrast).text_size(px(12.0)).text_color(move || {
                        if reading.custom_palette().passes_aa() {
                            theme().muted_foreground
                        } else {
                            theme().destructive
                        }
                    })),
            ),
        ],
    );

    let reflow_card = card_from_rows(
        "settings.group.reflow_books",
        query,
        vec![
            RowItem::new(
                "settings.reflow_font_size",
                "settings.reflow_font_size_hint",
                number_field(
                    live,
                    "settings.reflow_font_size",
                    REFLOW_FONT_SIZE,
                    f64::from(DEFAULT_FONT_SIZE),
                    (f64::from(*FONT_SIZES.start()), f64::from(*FONT_SIZES.end())),
                    1.0,
                    "settings.unit.points",
                ),
            ),
            RowItem::new(
                "settings.reflow_font_family",
                "settings.reflow_font_family_hint",
                reflow_font_picker(live),
            ),
            RowItem::new(
                "settings.reflow_use_document_css",
                "settings.reflow_use_document_css_hint",
                on_off(
                    "settings.reflow_use_document_css",
                    move || live.read(|s| s.flag(REFLOW_USE_DOCUMENT_CSS).unwrap_or(true)),
                    move |on| {
                        live.change(|s| {
                            s.set_flag(REFLOW_USE_DOCUMENT_CSS, (!on).then_some(false));
                        });
                    },
                ),
            ),
            RowItem::new(
                "settings.reflow_user_css",
                "settings.reflow_user_css_hint",
                reflow_css_picker(live),
            ),
        ],
    );

    let aids_card = card_from_rows(
        "settings.group.reading_aids",
        query,
        vec![
            RowItem::new(
                "settings.ruler_color",
                "settings.ruler_color_hint",
                swatches,
            ),
            RowItem::new(
                "settings.ruler_height",
                "settings.ruler_height_hint",
                number_field(
                    live,
                    "settings.ruler_height",
                    RULER_HEIGHT,
                    f64::from(default.height),
                    (f64::from(MIN_HEIGHT), f64::from(MAX_HEIGHT)),
                    4.0,
                    "settings.unit.pixels",
                ),
            ),
            RowItem::new(
                "settings.ruler_opacity",
                "settings.ruler_opacity_hint",
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        slider()
                            .value(move || Some(SliderValue::Single(opacity())))
                            .min(0.1)
                            .max(1.0)
                            .step(0.05)
                            .accessibility_label(t!("settings.ruler_opacity"))
                            .on_change(move |value: &SliderValue| {
                                opacity_shown.set(Some(value.start()));
                            })
                            .on_change_end(move |value: &SliderValue| {
                                let value = (value.start() * 100.0).round() / 100.0;
                                opacity_shown.set(None);
                                live.change(|s| {
                                    let is_default =
                                        (value - f64::from(default.opacity)).abs() < 0.005;
                                    s.set_number(RULER_OPACITY, (!is_default).then_some(value));
                                });
                            })
                            .w(px(160.0))
                            .h(px(20.0))
                            .justify_center()
                            .track_class(SLIDER_TRACK)
                            .range_class(SLIDER_RANGE)
                            .thumb_class(SLIDER_THUMB),
                    )
                    .child(
                        text(move || format!("{:.0} %", opacity() * 100.0))
                            .w(px(44.0))
                            .text_size(px(13.0))
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                    ),
            ),
            RowItem::new(
                "settings.ruler_invert",
                "settings.ruler_invert_hint",
                on_off(
                    "settings.ruler_invert",
                    move || live.read(|s| s.flag(RULER_INVERT)).unwrap_or(false),
                    move |on| live.change(|s| s.set_flag(RULER_INVERT, on.then_some(true))),
                ),
            ),
            RowItem::new("settings.focus_scope", "settings.focus_scope_hint", scopes),
            RowItem::new(
                "settings.auto_scroll_speed",
                "settings.auto_scroll_speed_hint",
                number_field(
                    live,
                    "settings.auto_scroll_speed",
                    AUTO_SCROLL_SPEED,
                    f64::from(DEFAULT_SPEED),
                    (f64::from(SPEEDS[0]), f64::from(SPEEDS[SPEEDS.len() - 1])),
                    4.0,
                    "settings.unit.pixels_a_second",
                ),
            ),
            RowItem::new(
                "settings.auto_reload",
                "settings.auto_reload_hint",
                auto_reload_control(live),
            ),
        ],
    );

    [colors_card, reflow_card, aids_card]
        .into_iter()
        .flatten()
        .collect()
}

/// Searchable system-font picker for reflowable books (EPUB, FB2, MOBI, HTML, Markdown).
fn reflow_font_picker(live: Live) -> View {
    let open = signal(false);
    let filter = signal(String::new());
    let chosen = move || {
        live.read(|s| {
            s.text(REFLOW_FONT_FAMILY)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    };
    let button_label = move || chosen().unwrap_or_else(|| t!("settings.reflow_font_default"));

    let trigger = button()
        .accessibility_label(t!("settings.reflow_font_family"))
        .on_press(move || {
            let next = !open.get();
            if next {
                filter.set(String::new());
            }
            open.set(next);
        })
        .h(px(32.0))
        .min_w(px(210.0))
        .max_w(px(260.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(button_label)
                .text_size(px(13.0))
                .number_of_lines(1)
                .text_color(move || theme().popover_foreground),
        )
        .child(dropdown_arrow());

    let picker_panel = when(
        move || open.get(),
        move || {
            let search = input()
                .value(move || filter.get())
                .placeholder(t!("settings.reflow_font_search"))
                .label(t!("settings.reflow_font_search"))
                .on_change(move |next: &str| filter.set(next.to_owned()))
                .h(px(30.0))
                .px_2p5()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .text_size(px(12.5))
                .text_color(move || theme().foreground)
                .focus_visible(|s| s.border_color(move || theme().ring));

            let list = dynamic(move || {
                let needle = filter.with(|q| q.trim().to_ascii_lowercase());
                let active = chosen();
                let mut rows: Vec<AnyElement> = Vec::new();
                if needle.is_empty()
                    || t!("settings.reflow_font_default")
                        .to_ascii_lowercase()
                        .contains(&needle)
                {
                    let is_active = active.is_none();
                    rows.push(
                        font_option_row(t!("settings.reflow_font_default"), is_active, move || {
                            live.change(|s| s.set_text(REFLOW_FONT_FAMILY, None));
                            open.set(false);
                        })
                        .into_element(),
                    );
                }
                for family in ecclesia_core::formats::system_font_families() {
                    if !needle.is_empty() && !family.to_ascii_lowercase().contains(&needle) {
                        continue;
                    }
                    let is_active = active.as_deref() == Some(family.as_str());
                    let family_pick = family.clone();
                    rows.push(
                        font_option_row(family.clone(), is_active, move || {
                            live.change(|s| {
                                s.set_text(REFLOW_FONT_FAMILY, Some(family_pick.as_str()));
                            });
                            open.set(false);
                        })
                        .into_element(),
                    );
                }
                if rows.is_empty() {
                    return view()
                        .px_2p5()
                        .py_2()
                        .child(
                            text(t!("settings.reflow_font_none"))
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        )
                        .into_element();
                }
                let mut col = view().gap(px(2.0));
                for row in rows {
                    col = col.child(row);
                }
                col.into_element()
            });

            view()
                .w(px(260.0))
                .p(px(8.0))
                .gap(px(6.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .child(search)
                .child(scroll().max_h(px(210.0)).child(list))
        },
    );

    view()
        .items_end()
        .gap(px(6.0))
        .child(trigger)
        .child(picker_panel)
}

fn font_option_row(label: String, active: bool, on_pick: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(label.clone())
        .on_press(on_pick)
        .w_full()
        .h(px(28.0))
        .px_2p5()
        .flex_row()
        .items_center()
        .justify_between()
        .rounded(px(5.0))
        .bg(move || {
            if active {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().accent))
        .child(
            text(label)
                .text_size(px(12.5))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
}

/// Custom `.css` stylesheet file picker (Browse + Clear) for reflowable documents.
fn reflow_css_picker(live: Live) -> View {
    let css_path = move || {
        live.read(|s| {
            s.text(REFLOW_USER_CSS_PATH)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    };
    let display_name = move || match css_path() {
        Some(path) => std::path::Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .map_or(path.clone(), str::to_owned),
        None => t!("settings.reflow_user_css_none"),
    };
    let browse = move || {
        let dialog = nsh_core::file_dialog::OpenDialog::files()
            .title(t!("settings.reflow_user_css_pick"))
            .filter(nsh_core::file_dialog::FileFilter::new(
                t!("settings.reflow_user_css_filter"),
                ["css"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(path) = paths.into_iter().next()
                && let Some(utf8) = path.to_str()
            {
                let owned = utf8.to_owned();
                live.change(|s| s.set_text(REFLOW_USER_CSS_PATH, Some(owned.as_str())));
            }
        });
    };

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            text(display_name)
                .max_w(px(150.0))
                .number_of_lines(1)
                .text_size(px(12.5))
                .text_color(move || {
                    if css_path().is_some() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
        .child(picker_button(
            move || t!("settings.reflow_user_css_browse"),
            browse,
        ))
        .child(when(
            move || css_path().is_some(),
            move || {
                picker_button(
                    move || t!("settings.reflow_user_css_clear"),
                    move || live.change(|s| s.set_text(REFLOW_USER_CSS_PATH, None)),
                )
            },
        ))
}

/// Segmented selector for [`AutoReloadMode`] (`Silent` / `Ask` / `Off`).
fn auto_reload_control(live: Live) -> impl IntoElement {
    let mode = move || live.read(AutoReloadMode::from_settings);
    let choose = move |choice: AutoReloadMode| {
        live.change(|s| {
            s.set_text(
                AUTO_RELOAD_MODE,
                (choice != AutoReloadMode::default()).then_some(choice.as_str()),
            );
        });
    };
    segmented()
        .child(segment(
            "settings.auto_reload.silent",
            move || mode() == AutoReloadMode::Silent,
            move || choose(AutoReloadMode::Silent),
        ))
        .child(segment(
            "settings.auto_reload.ask",
            move || mode() == AutoReloadMode::Ask,
            move || choose(AutoReloadMode::Ask),
        ))
        .child(segment(
            "settings.auto_reload.off",
            move || mode() == AutoReloadMode::Off,
            move || choose(AutoReloadMode::Off),
        ))
}

// ---------------------------------------------------------------------------
// Section 3: Tabs & Panes
// ---------------------------------------------------------------------------

fn tabs_and_panes_cards(live: Live, query: Option<&str>) -> Vec<View> {
    let tab_style = move || live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let choose_style = move |style: TabStyle| {
        live.change(|s| {
            s.set_text(TAB_STYLE, (style == TabStyle::Chrome).then_some("chrome"));
        });
    };
    let order = move || live.read(|s| Cycle::parse(s.text(TAB_CYCLE_ORDER)));
    let choose_cycle = move |cycle: Cycle| {
        live.change(|s| {
            s.set_text(TAB_CYCLE_ORDER, (cycle == Cycle::Strip).then_some("strip"));
        });
    };
    let closing = move || live.read(|s| ClosingTabs::parse(s.text(CLOSE_PANE_TABS)));
    let choose_closing = move |choice: ClosingTabs| {
        live.change(|s| {
            s.set_text(
                CLOSE_PANE_TABS,
                (choice == ClosingTabs::Close).then_some("close"),
            );
        });
    };
    let follow = move || live.read(|s| Follow::parse(s.text(SYNC_SCROLL)));
    let choose_follow = move |choice: Follow| {
        live.change(|s| {
            s.set_text(
                SYNC_SCROLL,
                (choice != Follow::default()).then_some(choice.name()),
            );
        });
    };
    let elsewhere = move || live.read(|s| Elsewhere::parse(s.text(OPEN_ELSEWHERE)));
    let choose_elsewhere = move |choice: Elsewhere| {
        live.change(|s| {
            s.set_text(
                OPEN_ELSEWHERE,
                (choice == Elsewhere::Here).then_some("here"),
            );
        });
    };

    let tabs_card = card_from_rows(
        "settings.section.tabs",
        query,
        vec![
            RowItem::new(
                "settings.tab_style",
                "settings.tab_style_hint",
                segmented()
                    .child(segment(
                        "settings.tab_style.vivaldi",
                        move || tab_style() == TabStyle::Vivaldi,
                        move || choose_style(TabStyle::Vivaldi),
                    ))
                    .child(segment(
                        "settings.tab_style.chrome",
                        move || tab_style() == TabStyle::Chrome,
                        move || choose_style(TabStyle::Chrome),
                    )),
            ),
            RowItem::new(
                "settings.tab_min_width",
                "settings.tab_min_width_hint",
                number_field(
                    live,
                    "settings.tab_min_width",
                    TAB_MIN_WIDTH_PX,
                    DEFAULT_TAB_MIN_WIDTH_PX,
                    (MIN_TAB_MIN_WIDTH_PX, MAX_TAB_MIN_WIDTH_PX),
                    10.0,
                    "settings.unit.px",
                ),
            ),
            RowItem::new(
                "settings.tab_height",
                "settings.tab_height_hint",
                number_field(
                    live,
                    "settings.tab_height",
                    TAB_HEIGHT_PX,
                    DEFAULT_TAB_HEIGHT_PX,
                    (MIN_TAB_HEIGHT_PX, MAX_TAB_HEIGHT_PX),
                    2.0,
                    "settings.unit.px",
                ),
            ),
            RowItem::new(
                "settings.tab_font_size",
                "settings.tab_font_size_hint",
                number_field(
                    live,
                    "settings.tab_font_size",
                    TAB_FONT_SIZE_PX,
                    DEFAULT_TAB_FONT_SIZE_PX,
                    (MIN_TAB_FONT_SIZE_PX, MAX_TAB_FONT_SIZE_PX),
                    1.0,
                    "settings.unit.px",
                ),
            ),
            RowItem::new(
                "settings.tab_cycle",
                "settings.tab_cycle_hint",
                segmented()
                    .child(segment(
                        "settings.tab_cycle.recent",
                        move || order() == Cycle::Recent,
                        move || choose_cycle(Cycle::Recent),
                    ))
                    .child(segment(
                        "settings.tab_cycle.strip",
                        move || order() == Cycle::Strip,
                        move || choose_cycle(Cycle::Strip),
                    )),
            ),
        ],
    );

    let panes_card = card_from_rows(
        "settings.group.panes_and_workspaces",
        query,
        vec![
            RowItem::new(
                "settings.close_pane_tabs",
                "settings.close_pane_tabs_hint",
                segmented()
                    .child(segment(
                        "settings.close_pane_tabs.move",
                        move || closing() == ClosingTabs::Move,
                        move || choose_closing(ClosingTabs::Move),
                    ))
                    .child(segment(
                        "settings.close_pane_tabs.close",
                        move || closing() == ClosingTabs::Close,
                        move || choose_closing(ClosingTabs::Close),
                    )),
            ),
            RowItem::new(
                "settings.sync_scroll",
                "settings.sync_scroll_hint",
                segmented()
                    .child(segment(
                        "settings.sync_scroll.page",
                        move || follow() == Follow::SamePage,
                        move || choose_follow(Follow::SamePage),
                    ))
                    .child(segment(
                        "settings.sync_scroll.position",
                        move || follow() == Follow::SamePosition,
                        move || choose_follow(Follow::SamePosition),
                    ))
                    .child(segment(
                        "settings.sync_scroll.pairs",
                        move || follow() == Follow::Pairs,
                        move || choose_follow(Follow::Pairs),
                    )),
            ),
            RowItem::new(
                "settings.sync_zoom",
                "settings.sync_zoom_hint",
                on_off(
                    "settings.sync_zoom",
                    move || live.read(|s| s.flag(SYNC_ZOOM).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(SYNC_ZOOM, on.then_some(true))),
                ),
            ),
            RowItem::new(
                "settings.open_elsewhere",
                "settings.open_elsewhere_hint",
                segmented()
                    .child(segment(
                        "settings.open_elsewhere.switch",
                        move || elsewhere() == Elsewhere::Switch,
                        move || choose_elsewhere(Elsewhere::Switch),
                    ))
                    .child(segment(
                        "settings.open_elsewhere.here",
                        move || elsewhere() == Elsewhere::Here,
                        move || choose_elsewhere(Elsewhere::Here),
                    )),
            ),
        ],
    );

    [tabs_card, panes_card].into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// Section 4: Home Page
// ---------------------------------------------------------------------------

fn home_cards(live: Live, query: Option<&str>) -> Vec<View> {
    let design_style = move || live.read(|s| HomeDesignStyle::parse(s.text(HOME_DESIGN_STYLE)));
    let choose_design = move |style: HomeDesignStyle| {
        live.change(|s| {
            s.set_text(
                HOME_DESIGN_STYLE,
                (style != HomeDesignStyle::default()).then_some(style.name()),
            );
        });
    };
    let view_mode = move || live.read(|s| HomeViewMode::parse(s.text(HOME_VIEW_MODE)));
    let choose_view = move |mode: HomeViewMode| {
        live.change(|s| {
            s.set_text(
                HOME_VIEW_MODE,
                (mode == HomeViewMode::List).then_some("list"),
            );
        });
    };
    let freq_sort = move || live.read(|s| s.flag(HOME_SORT_BY_FREQUENCY).unwrap_or(false));

    card_from_rows(
        "settings.section.home",
        query,
        vec![
            RowItem::new(
                "settings.home_design_style",
                "settings.home_design_style_hint",
                segmented()
                    .child(segment(
                        "settings.home_design_style.enhanced_warm",
                        move || design_style() == HomeDesignStyle::EnhancedWarm,
                        move || choose_design(HomeDesignStyle::EnhancedWarm),
                    ))
                    .child(segment(
                        "settings.home_design_style.modern_green",
                        move || design_style() == HomeDesignStyle::ModernGreen,
                        move || choose_design(HomeDesignStyle::ModernGreen),
                    ))
                    .child(segment(
                        "settings.home_design_style.classic_clean",
                        move || design_style() == HomeDesignStyle::ClassicClean,
                        move || choose_design(HomeDesignStyle::ClassicClean),
                    )),
            ),
            RowItem::new(
                "settings.home_view_mode",
                "settings.home_view_mode_hint",
                segmented()
                    .child(segment(
                        "settings.home_view_mode.thumbnails",
                        move || view_mode() == HomeViewMode::Thumbnails,
                        move || choose_view(HomeViewMode::Thumbnails),
                    ))
                    .child(segment(
                        "settings.home_view_mode.list",
                        move || view_mode() == HomeViewMode::List,
                        move || choose_view(HomeViewMode::List),
                    )),
            ),
            RowItem::new(
                "settings.home_thumb_size",
                "settings.home_thumb_size_hint",
                number_field(
                    live,
                    "settings.home_thumb_size",
                    HOME_THUMBNAIL_SIZE,
                    f64::from(DEFAULT_HOME_THUMB_SIZE),
                    (
                        f64::from(MIN_HOME_THUMB_SIZE),
                        f64::from(MAX_HOME_THUMB_SIZE),
                    ),
                    25.0,
                    "settings.unit.percent",
                ),
            ),
            RowItem::new(
                "settings.home_sort_frequency",
                "settings.home_sort_frequency_hint",
                segmented()
                    .child(segment(
                        "settings.home_sort.recent",
                        move || !freq_sort(),
                        move || live.change(|s| s.set_flag(HOME_SORT_BY_FREQUENCY, None)),
                    ))
                    .child(segment(
                        "settings.home_sort.frequent",
                        freq_sort,
                        move || live.change(|s| s.set_flag(HOME_SORT_BY_FREQUENCY, Some(true))),
                    )),
            ),
            RowItem::new(
                "settings.home_thumb_padding",
                "settings.home_thumb_padding_hint",
                on_off(
                    "settings.home_thumb_padding",
                    move || live.read(|s| s.flag(HOME_THUMBNAIL_PADDING).unwrap_or(true)),
                    move |on| {
                        live.change(|s| s.set_flag(HOME_THUMBNAIL_PADDING, (!on).then_some(false)));
                    },
                ),
            ),
            RowItem::new(
                "settings.home_show_progress",
                "settings.home_show_progress_hint",
                on_off(
                    "settings.home_show_progress",
                    move || live.read(|s| s.flag(HOME_SHOW_PROGRESS).unwrap_or(true)),
                    move |on| {
                        live.change(|s| s.set_flag(HOME_SHOW_PROGRESS, (!on).then_some(false)));
                    },
                ),
            ),
            RowItem::new(
                "settings.home_full_page",
                "settings.home_full_page_hint",
                on_off(
                    "settings.home_full_page",
                    move || live.read(|s| s.flag(HOME_FULL_PAGE_THUMBNAILS).unwrap_or(false)),
                    move |on| {
                        live.change(|s| s.set_flag(HOME_FULL_PAGE_THUMBNAILS, on.then_some(true)));
                    },
                ),
            ),
            RowItem::new(
                "settings.home_fixed_header",
                "settings.home_fixed_header_hint",
                on_off(
                    "settings.home_fixed_header",
                    move || live.read(|s| s.flag(HOME_FIXED_HEADER).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(HOME_FIXED_HEADER, on.then_some(true))),
                ),
            ),
            RowItem::new(
                "settings.home_max_recent",
                "settings.home_max_recent_hint",
                number_field(
                    live,
                    "settings.home_max_recent",
                    HOME_MAX_RECENT,
                    f64::from(DEFAULT_HOME_MAX_RECENT),
                    (
                        f64::from(MIN_HOME_MAX_RECENT),
                        f64::from(MAX_HOME_MAX_RECENT),
                    ),
                    10.0,
                    "settings.unit.docs",
                ),
            ),
        ],
    )
    .into_iter()
    .collect()
}

// ---------------------------------------------------------------------------
// Section 5: Annotations & Modes
// ---------------------------------------------------------------------------

fn annotations_cards(
    live: Live,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Vec<View> {
    use crate::annotate::margin::MARGIN_NOTES;
    use crate::annotate::{POLYGON_SIDES, STAR_POINTS, colors};
    use ecclesia_features::annotate::colors::Palette as AnnotPalette;
    use ecclesia_features::annotate::polygons::{DEFAULT_SIDES, SIDES};

    let counts = (f64::from(*SIDES.start()), f64::from(*SIDES.end()));
    let palette = [
        (
            AnnotPalette::Standard,
            "settings.annotation_palette_standard",
        ),
        (
            AnnotPalette::ColorBlindSafe,
            "settings.annotation_palette_safe",
        ),
    ]
    .into_iter()
    .fold(segmented(), |row, (choice, label)| {
        row.child(segment(
            label,
            move || colors::palette(live) == choice,
            move || colors::set_palette(live, choice),
        ))
    });

    let annotation_card = card_from_rows(
        "settings.section.annotation",
        query,
        vec![
            RowItem::new(
                "settings.annotation_palette",
                "settings.annotation_palette_hint",
                palette,
            ),
            RowItem::new(
                "settings.annotation_colors",
                "settings.annotation_colors_hint",
                picker_button(
                    move || t!("settings.annotation_colors_choose"),
                    move || hand_over(CommandId::AnnotationColors),
                ),
            ),
            RowItem::new(
                "settings.margin_notes",
                "settings.margin_notes_hint",
                on_off(
                    "settings.margin_notes",
                    move || live.read(|s| s.flag(MARGIN_NOTES).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(MARGIN_NOTES, on.then_some(true))),
                ),
            ),
            RowItem::new(
                "settings.polygon_sides",
                "settings.polygon_sides_hint",
                number_field(
                    live,
                    "settings.polygon_sides",
                    POLYGON_SIDES,
                    f64::from(DEFAULT_SIDES),
                    counts,
                    1.0,
                    "settings.unit.sides",
                ),
            ),
            RowItem::new(
                "settings.star_points",
                "settings.star_points_hint",
                number_field(
                    live,
                    "settings.star_points",
                    STAR_POINTS,
                    f64::from(DEFAULT_SIDES),
                    counts,
                    1.0,
                    "settings.unit.star_points",
                ),
            ),
        ],
    );

    let modes_card = card_from_rows(
        "settings.section.modes",
        query,
        vec![
            RowItem::new(
                "settings.pen_to_write",
                "settings.pen_to_write_hint",
                on_off(
                    "settings.pen_to_write",
                    move || live.read(|s| s.flag(PEN_TO_WRITE).unwrap_or(true)),
                    move |on| live.change(|s| s.set_flag(PEN_TO_WRITE, (!on).then_some(false))),
                ),
            ),
            RowItem::new(
                "settings.finger_draws",
                "settings.finger_draws_hint",
                on_off(
                    "settings.finger_draws",
                    move || live.read(|s| s.flag(FINGER_DRAWS).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(FINGER_DRAWS, on.then_some(true))),
                ),
            ),
            RowItem::new(
                "settings.toolbar",
                "settings.toolbar_hint",
                picker_button(
                    move || t!("settings.toolbar_customize"),
                    move || hand_over(CommandId::CustomizeToolbar),
                ),
            ),
        ],
    );

    let popup_matches = query.is_none_or(|needle| {
        matches_query(&t!("settings.section.selection_popup"), needle)
            || matches_query(&t!("annotate.popup.customize_hint"), needle)
            || matches_query(&t!("annotate.popup.picker_icon"), needle)
            || matches_query(&t!("annotate.popup.picker_icon_hint"), needle)
            || matches_query(&t!("annotate.popup.remember_color"), needle)
            || matches_query(&t!("annotate.popup.remember_color_hint"), needle)
            || ColorPickerIcon::ALL
                .iter()
                .any(|icon| matches_query(&t!(icon.label_key()), needle))
            || PopupItem::ALL
                .iter()
                .any(|item| matches_query(&t!(item.label_key()), needle))
    });
    let popup_card = popup_matches.then(|| {
        settings_card(
            "settings.section.selection_popup",
            selection_popup_settings_rows(live),
        )
    });

    [annotation_card, popup_card, modes_card]
        .into_iter()
        .flatten()
        .collect()
}

// ---------------------------------------------------------------------------
// Section 6: Print & Links
// ---------------------------------------------------------------------------

fn print_and_links_cards(live: Live, query: Option<&str>) -> Vec<View> {
    let scaling = move || live.read(|s| Scaling::parse(s.text(PRINT_SCALING)));
    let choose_scaling = move |choice: Scaling| {
        live.change(|s| {
            s.set_text(
                PRINT_SCALING,
                (choice != Scaling::default()).then_some(choice.name()),
            );
        });
    };
    let segments = Scaling::ALL.into_iter().fold(segmented(), |group, choice| {
        let label = match choice {
            Scaling::Shrink => "settings.print_scaling.shrink",
            Scaling::Fit => "settings.print_scaling.fit",
            Scaling::Actual => "settings.print_scaling.actual",
        };
        group.child(segment(
            label,
            move || scaling() == choice,
            move || choose_scaling(choice),
        ))
    });
    let annotations = move || live.read(|s| s.flag(PRINT_ANNOTATIONS).unwrap_or(true));
    let print_query = query.filter(|q| !matches_query(&t!("settings.section.print"), q));
    let print_card = card_from_rows(
        "settings.section.print",
        print_query,
        vec![
            RowItem::new(
                "settings.print_scaling",
                "settings.print_scaling_hint",
                segments,
            ),
            RowItem::new(
                "settings.print_annotations",
                "settings.print_annotations_hint",
                on_off("settings.print_annotations", annotations, move |on| {
                    live.change(|s| s.set_flag(PRINT_ANNOTATIONS, (!on).then_some(false)));
                }),
            ),
        ],
    );

    let previews = move || live.read(|s| s.flag(REF_PREVIEW)).unwrap_or(true);
    let links_query = query.filter(|q| !matches_query(&t!("settings.section.links"), q));
    let links_card = card_from_rows(
        "settings.section.links",
        links_query,
        vec![
            RowItem::new(
                "settings.ref_preview",
                "settings.ref_preview_hint",
                on_off("settings.ref_preview", previews, move |on| {
                    live.change(|s| s.set_flag(REF_PREVIEW, (!on).then_some(false)));
                }),
            ),
            RowItem::new(
                "settings.ref_preview_max_width",
                "settings.ref_preview_max_width_hint",
                number_field(
                    live,
                    "settings.ref_preview_max_width",
                    REF_PREVIEW_MAX_WIDTH,
                    DEFAULT_PREVIEW_MAX_WIDTH,
                    (MIN_PREVIEW_MAX_WIDTH, MAX_PREVIEW_MAX_WIDTH),
                    5.0,
                    "settings.unit.percent",
                ),
            ),
        ],
    );

    [print_card, links_card].into_iter().flatten().collect()
}

/// The protocol list as the setting keeps it: lower case, comma-separated, without blanks or repeats.
fn tidy_protocols(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for protocol in text
        .split([',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let protocol = protocol.trim_end_matches(':').to_ascii_lowercase();
        if !kept.contains(&protocol) {
            kept.push(protocol);
        }
    }
    kept.join(",")
}

// ---------------------------------------------------------------------------
// Advanced Settings (Security & Sandbox, Packs & Services, settings.toml Inspector)
// ---------------------------------------------------------------------------

fn advanced_cards(
    live: Live,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Vec<View> {
    let mut cards = Vec::new();
    if let Some(card) = security_card(live, query) {
        cards.push(card);
    }
    if let Some(card) = packs_card(live, query) {
        cards.push(card);
    }
    if let Some(card) = phase5_card(live, hand_over, query) {
        cards.push(card);
    }
    if let Some(card) = phase6_7_card(live, query) {
        cards.push(card);
    }
    if let Some(card) = toml_inspector_card(live, query) {
        cards.push(card);
    }
    cards
}

/// Optional packs card (K.21): signed index URL, offline `.expack` import, and installed/available lists.
fn packs_card(live: Live, query: Option<&str>) -> Option<View> {
    let packs_matches = match query {
        None => true,
        Some(needle) => {
            matches_query(&t!("settings.section.packs"), needle)
                || matches_query(&t!("settings.packs.index_url"), needle)
                || matches_query(&t!("settings.packs.index_url_hint"), needle)
                || matches_query(&t!("settings.packs.actions"), needle)
                || matches_query(&t!("settings.packs.actions_hint"), needle)
        }
    };
    if !packs_matches {
        return None;
    }

    let state = PacksState::new(live);
    let url_draft = signal(None::<String>);
    let commit_url = move || {
        let Some(typed) = url_draft.get_untracked() else {
            return;
        };
        url_draft.set(None);
        let trimmed = typed.trim();
        live.change(|s| s.set_text(PACKS_INDEX_URL, (!trimmed.is_empty()).then_some(trimmed)));
    };
    let url_field = input()
        .value(move || {
            url_draft.get().unwrap_or_else(|| {
                live.read(|s| s.text(PACKS_INDEX_URL).unwrap_or_default().to_owned())
            })
        })
        .on_change(move |text: &str| url_draft.set(Some(text.to_owned())))
        .on_submit(move |_: &str| commit_url())
        .on_blur(commit_url)
        .placeholder(t!("settings.packs.index_placeholder"))
        .label(t!("settings.packs.index_url"))
        .w(px(230.0))
        .h(px(32.0))
        .px_2()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(12.0))
        .text_color(move || theme().popover_foreground);

    let actions = view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(crate::tabs::action_button(
            "settings.packs.install_file",
            crate::tabs::Tone::Plain,
            move || state.install_from_file(),
        ))
        .child(crate::tabs::action_button(
            "settings.packs.refresh_index",
            crate::tabs::Tone::Plain,
            move || state.refresh_index(),
        ))
        .child(dynamic(move || {
            if state.is_busy() {
                view().child(crate::tabs::action_button(
                    "settings.packs.cancel",
                    crate::tabs::Tone::Plain,
                    move || state.cancel_download(),
                ))
            } else {
                view()
            }
        }));

    let unavailable_notice = if packs::packs_available() {
        view()
    } else {
        view().pt_1().child(
            text(move || t!("packs.unavailable"))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
    };
    let status_bar = dynamic(move || match state.status() {
        Some(message) => view().pt_1().child(
            text(message)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        ),
        None => view(),
    });
    let installed_list = dynamic(move || {
        let items = state.installed();
        if items.is_empty() {
            return view().py_1().child(
                text(move || t!("settings.packs.none_installed"))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            );
        }
        items.into_iter().fold(view().gap_1().pt_1(), |list, item| {
            let id = item.manifest.id.clone();
            let title = item.manifest.title.clone();
            let meta = format!("v{} - {}", item.manifest.version, item.manifest.id);
            list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(
                        view()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                text(title)
                                    .text_size(px(13.0))
                                    .text_color(move || theme().popover_foreground),
                            )
                            .child(
                                text(meta)
                                    .text_size(px(11.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(crate::tabs::action_button(
                        "settings.packs.remove",
                        crate::tabs::Tone::Plain,
                        move || state.remove(id.clone()),
                    )),
            )
        })
    });
    let available_list = dynamic(move || {
        let entries = state.available();
        if entries.is_empty() {
            return view();
        }
        entries.into_iter().fold(
            view().gap_1().pt_2().child(
                text(move || t!("settings.packs.available_title"))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().muted_foreground),
            ),
            |list, entry| {
                let title = entry.title.clone();
                let meta = format!("v{} - {}", entry.version, format_size(entry.size));
                list.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .child(
                            view()
                                .flex_1()
                                .gap_0p5()
                                .child(
                                    text(title)
                                        .text_size(px(13.0))
                                        .text_color(move || theme().popover_foreground),
                                )
                                .child(
                                    text(meta)
                                        .text_size(px(11.0))
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(crate::tabs::action_button(
                            "settings.packs.download",
                            crate::tabs::Tone::Plain,
                            move || state.download(entry.clone()),
                        )),
                )
            },
        )
    });

    let details = view()
        .px(px(16.0))
        .py(px(10.0))
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(unavailable_notice)
        .child(status_bar)
        .child(installed_list)
        .child(available_list);

    Some(settings_card(
        "settings.section.packs",
        vec![
            card_row(
                RowDivider::First,
                "settings.packs.index_url",
                "settings.packs.index_url_hint",
                url_field,
            ),
            card_row(
                RowDivider::Divided,
                "settings.packs.actions",
                "settings.packs.actions_hint",
                actions,
            ),
            details,
        ],
    ))
}

/// Phase 5 settings: Folder watch (F.9), Workspace rules (W.12), SyncTeX preset (R.27),
/// Bionic reading (R.22), Neural offline voice (U.6), and UI control channel (E.4).
fn phase5_card(
    live: Live,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Option<View> {
    use crate::shell::control_channel::CONTROL_CHANNEL_ENABLED;
    use crate::shell::reading_extra::{BIONIC_ENABLED, SYNCTEX_PRESET};
    use crate::study_ui::{StudyState, phase5::NEURAL_VOICE_ID};
    use ecclesia_features::library::{Library, WatchDepth};

    let watch_status = signal(String::new());
    let add_watched_folder = move || {
        let dialog = nsh_core::file_dialog::OpenDialog::folders().title(t!("settings.watch.add"));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(dir) = paths.into_iter().next()
                && let Some(store) = crate::library::store()
            {
                let lib = Library::new(store);
                let _ = lib.watch_folder(&dir, WatchDepth::Recursive);
                if let Ok(report) = lib.reconcile_watched() {
                    watch_status.set(t!(
                        "library.folder_watched",
                        added = report.added,
                        updated = report.modified
                    ));
                }
            }
        });
    };

    let current_preset =
        move || live.read(|s| s.text(SYNCTEX_PRESET).unwrap_or("vscode").to_owned());
    let set_preset = move |id: &str| {
        let owned = id.to_owned();
        live.change(|s| s.set_text(SYNCTEX_PRESET, Some(owned.as_str())));
    };
    let presets = [
        ("vscode", "settings.synctex.vscode"),
        ("vim", "settings.synctex.vim"),
        ("emacs", "settings.synctex.emacs"),
        ("texstudio", "settings.synctex.texstudio"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || current_preset() == id,
            move || set_preset(id),
        ))
    });

    let neural_voices = StudyState::installed_neural_voices();
    let current_voice = move || live.read(|s| s.text(NEURAL_VOICE_ID).map(str::to_owned));
    let voice_picker = if neural_voices.is_empty() {
        segmented().child(segment(
            "settings.neural_voice.system",
            move || current_voice().is_none(),
            move || live.change(|s| s.set_text(NEURAL_VOICE_ID, None)),
        ))
    } else {
        let mut bar = segmented().child(segment(
            "settings.neural_voice.system",
            move || current_voice().is_none(),
            move || live.change(|s| s.set_text(NEURAL_VOICE_ID, None)),
        ));
        for (vid, vlabel) in neural_voices {
            let vid_sig = signal(vid);
            bar = bar.child(segment_owned(
                vlabel,
                move || vid_sig.with(|v| current_voice().as_deref() == Some(v.as_str())),
                move || {
                    vid_sig
                        .with(|v| live.change(|s| s.set_text(NEURAL_VOICE_ID, Some(v.as_str()))));
                },
            ));
        }
        bar
    };

    let effective_query = query.filter(|q| !matches_query(&t!("settings.section.advanced"), q));
    card_from_rows(
        "settings.section.advanced",
        effective_query,
        vec![
            RowItem::new(
                "settings.bionic",
                "settings.bionic_hint",
                on_off(
                    "settings.bionic",
                    move || live.read(|s| s.flag(BIONIC_ENABLED).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(BIONIC_ENABLED, on.then_some(true))),
                ),
            ),
            RowItem::new("settings.synctex", "settings.synctex_hint", presets),
            RowItem::new(
                "settings.neural_voice",
                "settings.neural_voice_hint",
                voice_picker,
            ),
            RowItem::new(
                "settings.watch",
                "settings.watch_hint",
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(picker_button(
                        move || {
                            let s = watch_status.get();
                            if s.is_empty() {
                                t!("settings.watch.add")
                            } else {
                                s
                            }
                        },
                        add_watched_folder,
                    )),
            ),
            RowItem::new(
                "settings.workspace_rules",
                "settings.workspace_rules_hint",
                picker_button(
                    move || t!("command.workspace_rules"),
                    move || hand_over(CommandId::WorkspaceRules),
                ),
            ),
            RowItem::new(
                "settings.control_channel",
                "settings.control_channel_hint",
                on_off(
                    "settings.control_channel",
                    move || live.read(|s| s.flag(CONTROL_CHANNEL_ENABLED).unwrap_or(true)),
                    move |on| live.change(|s| s.set_flag(CONTROL_CHANNEL_ENABLED, Some(on))),
                ),
            ),
        ],
    )
}

/// Phase 6 & 7 settings: AI consent & providers (I.1), PII redaction & budget, Google Lens
/// online toggle (R.31), CAD drawing view defaults (R.26), and TSA remote consent (O.14/O.20).
fn phase6_7_card(live: Live, query: Option<&str>) -> Option<View> {
    use crate::shell::ai_ui::{
        AI_API_KEY, AI_CONSENT_MODE, AI_ENDPOINT_URL, AI_MODEL_NAME, AI_PROVIDER_KIND,
        AI_REDACTION_MODE, AI_SESSION_BUDGET, LENS_ONLINE_ENABLED,
    };
    use crate::shell::forms_sign_ui::{TSA_REMOTE_CONSENT, TSA_REMOTE_URL};
    use crate::shell::pro_pdf_ui::{CAD_BACKGROUND_MODE, CAD_ENHANCE_MODE, CAD_LINE_WEIGHTS};

    let consent_picker = [
        ("disabled", "settings.ai.consent_off"),
        ("local", "settings.ai.consent_local"),
        ("prompt", "settings.ai.consent_prompt"),
        ("cloud", "settings.ai.consent_cloud"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(AI_CONSENT_MODE).unwrap_or("local") == id),
            move || live.change(|s| s.set_text(AI_CONSENT_MODE, Some(id))),
        ))
    });

    let provider_picker = [
        ("ollama", "settings.ai.provider_ollama"),
        ("llama_server", "settings.ai.provider_llama"),
        ("openai", "settings.ai.provider_openai"),
        ("anthropic", "settings.ai.provider_anthropic"),
        ("gemini", "settings.ai.provider_gemini"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(AI_PROVIDER_KIND).unwrap_or("ollama") == id),
            move || live.change(|s| s.set_text(AI_PROVIDER_KIND, Some(id))),
        ))
    });

    let cad_picker = [
        ("auto", "settings.cad.auto"),
        ("on", "settings.cad.on"),
        ("off", "settings.cad.off"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(CAD_ENHANCE_MODE).unwrap_or("auto") == id),
            move || live.change(|s| s.set_text(CAD_ENHANCE_MODE, Some(id))),
        ))
    });

    let cad_bg_picker = [
        ("light", "settings.cad.light"),
        ("blueprint", "settings.cad.blueprint"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(CAD_BACKGROUND_MODE).unwrap_or("light") == id),
            move || live.change(|s| s.set_text(CAD_BACKGROUND_MODE, Some(id))),
        ))
    });

    let text_setting = |key: &'static str, label: &'static str, placeholder: &'static str| {
        let draft = signal(None::<String>);
        let commit = move || {
            let Some(typed) = draft.get_untracked() else {
                return;
            };
            draft.set(None);
            let trimmed = typed.trim();
            live.change(|s| s.set_text(key, (!trimmed.is_empty()).then_some(trimmed)));
        };
        input()
            .value(move || {
                draft
                    .get()
                    .unwrap_or_else(|| live.read(|s| s.text(key).unwrap_or_default().to_owned()))
            })
            .on_change(move |text: &str| draft.set(Some(text.to_owned())))
            .on_submit(move |_: &str| commit())
            .on_blur(commit)
            .placeholder(t!(placeholder))
            .label(t!(label))
            .w(px(230.0))
            .h(px(32.0))
            .px_2()
            .rounded(px(CONTROL_RADIUS))
            .border_1()
            .border_color(move || theme().input)
            .focus(|s| s.border_color(move || theme().ring))
            .text_size(px(12.0))
            .text_color(move || theme().popover_foreground)
    };

    let effective_query = query.filter(|q| !matches_query(&t!("settings.section.ai_pro"), q));
    card_from_rows(
        "settings.section.ai_pro",
        effective_query,
        vec![
            RowItem::new(
                "settings.ai.consent",
                "settings.ai.consent_hint",
                consent_picker,
            ),
            RowItem::new(
                "settings.ai.provider",
                "settings.ai.provider_hint",
                provider_picker,
            ),
            RowItem::new(
                "settings.ai.endpoint",
                "settings.ai.endpoint_hint",
                text_setting(
                    AI_ENDPOINT_URL,
                    "settings.ai.endpoint",
                    "settings.ai.endpoint_placeholder",
                ),
            ),
            RowItem::new(
                "settings.ai.model",
                "settings.ai.model_hint",
                text_setting(
                    AI_MODEL_NAME,
                    "settings.ai.model",
                    "settings.ai.model_placeholder",
                ),
            ),
            RowItem::new(
                "settings.ai.api_key",
                "settings.ai.api_key_hint",
                text_setting(
                    AI_API_KEY,
                    "settings.ai.api_key",
                    "settings.ai.api_key_placeholder",
                ),
            ),
            RowItem::new(
                "settings.ai.redact",
                "settings.ai.redact_hint",
                on_off(
                    "settings.ai.redact",
                    move || {
                        live.read(|s| s.text(AI_REDACTION_MODE).unwrap_or("redact") == "redact")
                    },
                    move |on| {
                        live.change(|s| {
                            s.set_text(AI_REDACTION_MODE, Some(if on { "redact" } else { "raw" }));
                        });
                    },
                ),
            ),
            RowItem::new(
                "settings.ai.budget",
                "settings.ai.budget_hint",
                number_field(
                    live,
                    "settings.ai.budget",
                    AI_SESSION_BUDGET,
                    0.0,
                    (0.0, 2_000_000.0),
                    5_000.0,
                    "settings.ai.budget_unit",
                ),
            ),
            RowItem::new(
                "settings.lens.online",
                "settings.lens.online_hint",
                on_off(
                    "settings.lens.online",
                    move || live.read(|s| s.flag(LENS_ONLINE_ENABLED).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(LENS_ONLINE_ENABLED, Some(on))),
                ),
            ),
            RowItem::new("settings.cad.mode", "settings.cad.mode_hint", cad_picker),
            RowItem::new(
                "settings.cad.background",
                "settings.cad.background_hint",
                cad_bg_picker,
            ),
            RowItem::new(
                "settings.cad.weights",
                "settings.cad.weights_hint",
                on_off(
                    "settings.cad.weights",
                    move || {
                        live.read(|s| s.text(CAD_LINE_WEIGHTS).unwrap_or("respect") == "uniform")
                    },
                    move |on| {
                        live.change(|s| {
                            s.set_text(
                                CAD_LINE_WEIGHTS,
                                Some(if on { "uniform" } else { "respect" }),
                            );
                        });
                    },
                ),
            ),
            RowItem::new(
                "settings.tsa.consent",
                "settings.tsa.consent_hint",
                on_off(
                    "settings.tsa.consent",
                    move || live.read(|s| s.flag(TSA_REMOTE_CONSENT).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(TSA_REMOTE_CONSENT, Some(on))),
                ),
            ),
            RowItem::new(
                "settings.tsa.url",
                "settings.tsa.url_hint",
                text_setting(
                    TSA_REMOTE_URL,
                    "settings.tsa.url",
                    "settings.tsa.url_placeholder",
                ),
            ),
        ],
    )
}

// ---------------------------------------------------------------------------
// Advanced Card 1: Security, Sandbox & Low-Level Link Controls
// ---------------------------------------------------------------------------

/// Phase 8/9 sandbox worker status, `SecurityLimits` preferences (`K.22`, `K.23`),
/// allowed external link protocols, and reference hover preview delay.
fn security_card(live: Live, query: Option<&str>) -> Option<View> {
    use crate::shell::ecosystem_ui::{
        SANDBOX_MAX_PAGES, SANDBOX_MEMORY_CAP_MB, SANDBOX_STRICT_MODE, SANDBOX_TIMEOUT_SECS,
        sandbox_status_summary,
    };

    let saved_protocols = move || {
        live.read(|s| s.text(PROTOCOLS_KEY).map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_PROTOCOLS.to_owned())
    };
    let typed_protocols = signal(None::<String>);
    let keep_protocols = move || {
        let Some(text) = typed_protocols.get_untracked() else {
            return;
        };
        typed_protocols.set(None);
        let tidy = tidy_protocols(&text);
        live.change(|s| {
            s.set_text(
                PROTOCOLS_KEY,
                (tidy != DEFAULT_PROTOCOLS).then_some(tidy.as_str()),
            );
        });
    };

    let effective_query = query.filter(|q| !matches_query(&t!("settings.section.sandbox"), q));
    card_from_rows(
        "settings.section.sandbox",
        effective_query,
        vec![
            RowItem::new(
                "settings.sandbox.status",
                "settings.sandbox.status_hint",
                text(move || sandbox_status_summary(live))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            ),
            RowItem::new(
                "settings.sandbox.strict",
                "settings.sandbox.strict_hint",
                on_off(
                    "settings.sandbox.strict",
                    move || live.read(|s| s.flag(SANDBOX_STRICT_MODE).unwrap_or(false)),
                    move |on| live.change(|s| s.set_flag(SANDBOX_STRICT_MODE, Some(on))),
                ),
            ),
            RowItem::new(
                "settings.sandbox.max_pages",
                "settings.sandbox.max_pages_hint",
                number_field(
                    live,
                    "settings.sandbox.max_pages",
                    SANDBOX_MAX_PAGES,
                    50_000.0,
                    (100.0, 200_000.0),
                    1_000.0,
                    "settings.sandbox.pages_unit",
                ),
            ),
            RowItem::new(
                "settings.sandbox.memory_mb",
                "settings.sandbox.memory_mb_hint",
                number_field(
                    live,
                    "settings.sandbox.memory_mb",
                    SANDBOX_MEMORY_CAP_MB,
                    1024.0,
                    (128.0, 8192.0),
                    128.0,
                    "settings.sandbox.mb_unit",
                ),
            ),
            RowItem::new(
                "settings.sandbox.timeout_s",
                "settings.sandbox.timeout_s_hint",
                number_field(
                    live,
                    "settings.sandbox.timeout_s",
                    SANDBOX_TIMEOUT_SECS,
                    30.0,
                    (5.0, 300.0),
                    5.0,
                    "settings.sandbox.secs_unit",
                ),
            ),
            RowItem::new(
                "settings.link_protocols",
                "settings.link_protocols_hint",
                input()
                    .value(move || typed_protocols.get().unwrap_or_else(saved_protocols))
                    .on_change(move |text: &str| typed_protocols.set(Some(text.to_owned())))
                    .on_submit(move |_: &str| keep_protocols())
                    .on_blur(keep_protocols)
                    .label(t!("settings.link_protocols"))
                    .w(px(200.0))
                    .h(px(32.0))
                    .px_3()
                    .rounded(px(CONTROL_RADIUS))
                    .border_1()
                    .border_color(move || theme().input)
                    .focus(|s| s.border_color(move || theme().ring))
                    .text_size(px(13.0))
                    .text_color(move || theme().popover_foreground),
            ),
            RowItem::new(
                "settings.ref_preview_delay",
                "settings.ref_preview_delay_hint",
                number_field(
                    live,
                    "settings.ref_preview_delay",
                    REF_PREVIEW_DELAY,
                    DEFAULT_PREVIEW_DELAY,
                    (0.0, MAX_PREVIEW_DELAY),
                    50.0,
                    "settings.unit.milliseconds",
                ),
            ),
            RowItem::new(
                "settings.ref_preview_max_width",
                "settings.ref_preview_max_width_hint",
                number_field(
                    live,
                    "settings.ref_preview_max_width",
                    REF_PREVIEW_MAX_WIDTH,
                    DEFAULT_PREVIEW_MAX_WIDTH,
                    (MIN_PREVIEW_MAX_WIDTH, MAX_PREVIEW_MAX_WIDTH),
                    5.0,
                    "settings.unit.percent",
                ),
            ),
        ],
    )
}

// ---------------------------------------------------------------------------
// Advanced Card 3: settings.toml Override Inspector (SumatraPDF Enhanced style)
// ---------------------------------------------------------------------------

/// Live inspector listing every non-default key currently stored in `settings.toml` with a
/// one-click Reset button and direct access to open `settings.toml`.
fn toml_inspector_card(live: Live, query: Option<&str>) -> Option<View> {
    let matches = match query {
        None => true,
        Some(needle) => {
            matches_query(&t!("settings.group.toml_inspector"), needle)
                || matches_query(&t!("settings.toml_inspector"), needle)
                || matches_query(&t!("settings.toml_inspector_hint"), needle)
        }
    };
    if !matches {
        return None;
    }

    let open_btn = button()
        .accessibility_label(t!("settings.open_file"))
        .on_press(open_settings_file)
        .h(px(30.0))
        .px_3()
        .rounded(px(CONTROL_RADIUS))
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || t!("settings.open_file"))
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        );

    let header_row = card_row(
        RowDivider::First,
        "settings.toml_inspector",
        "settings.toml_inspector_hint",
        open_btn,
    );

    let overrides_list = dynamic(move || {
        let entries = live.read(settings::Settings::entries);
        if entries.is_empty() {
            return view()
                .px(px(16.0))
                .py(px(14.0))
                .border_t_1()
                .border_t_color(move || theme().border)
                .child(
                    text(move || t!("settings.toml_inspector_empty"))
                        .text_size(px(12.5))
                        .text_color(move || theme().muted_foreground),
                )
                .into_element();
        }
        entries
            .into_iter()
            .fold(view(), |list, (key, value)| {
                let reset_key = key.clone();
                let reset_label = t!("settings.toml_inspector_reset_key", key = key.as_str());
                list.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .px(px(16.0))
                        .py(px(10.0))
                        .border_t_1()
                        .border_t_color(move || theme().border)
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(2.0))
                                .child(
                                    text(key)
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(value)
                                        .number_of_lines(1u32)
                                        .text_size(px(12.0))
                                        .tabular_nums()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            button()
                                .accessibility_label(reset_label)
                                .on_press(move || {
                                    live.change(|s| s.set_value(&reset_key, None));
                                })
                                .h(px(28.0))
                                .px(px(10.0))
                                .flex_shrink_0()
                                .rounded(px(CONTROL_RADIUS))
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(move || theme().border)
                                .hover(|s| s.bg(move || theme().accent))
                                .focus_visible(|s| s.border_color(move || theme().ring))
                                .child(
                                    text(move || t!("settings.toml_inspector_reset"))
                                        .text_size(px(12.0))
                                        .text_color(move || theme().popover_foreground),
                                ),
                        ),
                )
            })
            .into_element()
    });

    Some(settings_card(
        "settings.group.toml_inspector",
        vec![header_row, view().child(overrides_list)],
    ))
}

// ---------------------------------------------------------------------------
// Section 9: Shortcuts
// ---------------------------------------------------------------------------

fn shortcuts_cards(live: Live, query: Option<&str>) -> Vec<View> {
    let keymap = move || {
        live.read(|_| ());
        keymap::current()
    };
    let problems = move || {
        keymap()
            .problems()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let edit = button()
        .accessibility_label(t!("settings.shortcuts_edit"))
        .on_press(open_settings_file)
        .h(px(30.0))
        .px_3()
        .rounded(px(CONTROL_RADIUS))
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || t!("settings.shortcuts_edit"))
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        );

    let header_matches = match query {
        None => true,
        Some(needle) => {
            matches_query(&t!("settings.shortcuts"), needle)
                || matches_query(&t!("settings.shortcuts_hint"), needle)
        }
    };

    let filtered_commands: Vec<_> = COMMANDS
        .iter()
        .filter(|command| match query {
            None => true,
            Some(_) if header_matches => true,
            Some(needle) => matches_query(&t!(command.label), needle),
        })
        .collect();

    if !header_matches && filtered_commands.is_empty() {
        return Vec::new();
    }

    let mut rows = vec![card_row(
        RowDivider::First,
        "settings.shortcuts",
        "settings.shortcuts_hint",
        edit,
    )];

    rows.push(
        view()
            .px(px(16.0))
            .child(each(problems, String::clone, |problem, _| {
                text(move || problem.get())
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(move || theme().destructive)
            })),
    );

    let list = filtered_commands.into_iter().fold(view(), |list, command| {
        let id = command.id;
        let keys = move |scope: Scope| {
            keymap()
                .bindings()
                .iter()
                .filter(|b| b.id == id && b.scope == scope)
                .map(|b| key_label(&b.keys))
                .collect::<Vec<_>>()
        };
        list.child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .px(px(16.0))
                .min_h(px(36.0))
                .border_t_1()
                .border_t_color(move || theme().border)
                .child(
                    text(move || t!(command.label))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().popover_foreground),
                )
                .child(keycaps(move || keys(Scope::Window), KeycapScope::Window))
                .child(keycaps(move || keys(Scope::Page), KeycapScope::Page)),
        )
    });
    rows.push(list);

    vec![settings_card("settings.section.shortcuts", rows)]
}

// ---------------------------------------------------------------------------
// Section 10: Help / Getting Started
// ---------------------------------------------------------------------------

fn help_cards(
    live: Live,
    hand_over: impl Fn(CommandId) + Copy + 'static,
    query: Option<&str>,
) -> Vec<View> {
    card_from_rows(
        "settings.section.help",
        query,
        vec![
            RowItem::new(
                "settings.show_tips",
                "settings.show_tips_hint",
                on_off(
                    "settings.show_tips",
                    move || live.read(|s| s.flag(SHOW_TIPS).unwrap_or(true)),
                    move |on| live.change(|s| s.set_flag(SHOW_TIPS, (!on).then_some(false))),
                ),
            ),
            RowItem::new(
                "settings.guide",
                "settings.guide_hint",
                picker_button(
                    move || t!("settings.guide_show"),
                    move || hand_over(CommandId::ReplayGuide),
                ),
            ),
        ],
    )
    .into_iter()
    .collect()
}

// ---------------------------------------------------------------------------
// Reusable Controls & Public Exports
// ---------------------------------------------------------------------------

/// The keys of one scope as keycaps; page keys are drawn quieter (the section's hint says why).
fn keycaps(keys: impl Fn() -> Vec<String> + 'static, scope: KeycapScope) -> impl IntoElement {
    let caps = each(keys, String::clone, move |label, _| {
        text(move || label.get())
            .px_2()
            .h(px(22.0))
            .rounded(px(5.0))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || match scope {
                KeycapScope::Page => theme().popover,
                KeycapScope::Window => theme().muted,
            })
            .text_size(px(12.0))
            .tabular_nums()
            .text_color(move || match scope {
                KeycapScope::Page => theme().muted_foreground,
                KeycapScope::Window => theme().popover_foreground,
            })
    });
    view().flex_row().gap_1().child(caps)
}

/// A button showing a choice made in a picker; pressing it opens the picker.
fn picker_button(
    shown: impl Fn() -> String + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let shown = std::rc::Rc::new(shown);
    let label = shown.clone();
    button()
        .accessibility_label(move || Some(SharedString::from(label())))
        .on_press(press)
        .h(px(32.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || shown())
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

/// The language segments: the system's, then each language the app has, named in its own language (U.7).
fn language_choice(live: Live) -> View {
    use crate::l10n::{LANGUAGE_KEY, LANGUAGES};
    let chosen = move || live.read(|s| s.text(LANGUAGE_KEY).map(str::to_owned));
    let system = segment(
        "settings.language.system",
        move || chosen().is_none(),
        move || live.change(|s| s.set_text(LANGUAGE_KEY, None)),
    );
    LANGUAGES
        .iter()
        .fold(segmented().child(system), |group, &(tag, name)| {
            group.child(named_segment(
                name,
                move || chosen().as_deref() == Some(tag),
                move || live.change(|s| s.set_text(LANGUAGE_KEY, Some(tag))),
            ))
        })
}

/// A yes-or-no setting's switch.
pub(crate) fn on_off(
    label: &'static str,
    on: impl Fn() -> bool + Copy + 'static,
    set: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .checked(on)
        .accessibility_label(t!(label))
        .on_change(move |checked: &bool| set(*checked))
        .track_class(SWITCH_TRACK)
        .thumb_class(SWITCH_THUMB)
}

/// The frame of a row of segments, one of which is chosen; they wrap when they don't fit.
pub(crate) fn segmented() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(move || theme().muted)
}

/// One segment: `chosen` says whether it is the one; pressing it chooses it.
pub(crate) fn segment(
    label: &'static str,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    segment_of(move || t!(label), chosen, choose)
}

/// A segment whose label is already in the reader's words (a language's own name, U.7).
fn named_segment(
    name: &'static str,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    segment_of(move || name.to_owned(), chosen, choose)
}

/// One segment with a dynamic label closure; pressing it selects it.
pub(crate) fn segment_of(
    label: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(label())
        .on_change(move |_: &bool| choose())
        .h(px(28.0))
        .px_3()
        .rounded(px(CONTROL_RADIUS))
        .items_center()
        .justify_center()
        .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(text(label).text_size(px(13.0)).text_color(move || {
            if chosen() {
                theme().popover_foreground
            } else {
                theme().muted_foreground
            }
        }))
}

/// Segment control for a dynamic `String` label that cannot be `Copy`-captured.
pub(crate) fn segment_owned(
    label: String,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(label.clone())
        .on_change(move |_: &bool| choose())
        .h(px(28.0))
        .px_3()
        .rounded(px(CONTROL_RADIUS))
        .items_center()
        .justify_center()
        .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(text(label).text_size(px(13.0)).text_color(move || {
            if chosen() {
                theme().popover_foreground
            } else {
                theme().muted_foreground
            }
        }))
}

/// A custom reading-theme colour (`key`): a swatch of it and its `#rrggbb`. What is typed is kept on Return or when
/// the field loses the focus; text that is no colour is dropped and the kept colour shows again.
fn color_field(
    reading: Reading,
    key: &'static str,
    label: &'static str,
    of: fn(Palette) -> Rgb,
) -> impl IntoElement {
    let kept = move || of(reading.custom_palette());
    let typed = signal(None::<String>);
    let keep = move || {
        let Some(text) = typed.get_untracked() else {
            return;
        };
        typed.set(None);
        if let Some(color) = parse_hex(&text) {
            reading.set_custom_color(key, Some(color));
        }
    };
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || Hsla::parse_hex(&hex(kept())).expect("hex() writes #rrggbb")),
        )
        .child(
            input()
                .value(move || typed.get().unwrap_or_else(|| hex(kept())))
                .on_change(move |text: &str| typed.set(Some(text.to_owned())))
                .on_submit(move |_: &str| keep())
                .on_blur(keep)
                .label(t!(label))
                .w(px(92.0))
                .h(px(32.0))
                .px_2()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(move || theme().input)
                .focus(|s| s.border_color(move || theme().ring))
                .text_size(px(13.0))
                .tabular_nums()
                .text_color(move || theme().popover_foreground),
        )
}

/// A number setting's field, `default` when unset; a value equal to the default is kept as unset.
fn number_field(
    live: Live,
    label: &'static str,
    key: &'static str,
    default: f64,
    (min, max): (f64, f64),
    step: f64,
    unit: &'static str,
) -> impl IntoElement {
    number_input()
        .value(move || Some(live.read(|s| s.number(key)).unwrap_or(default)))
        .min(Some(min))
        .max(Some(max))
        .step(step)
        .accessibility_label(t!(label))
        .on_change(move |value: &Option<f64>| {
            let value = value.map(|v| v.clamp(min, max));
            live.change(|s| s.set_number(key, value.filter(|v| *v != default)));
        })
        .suffix(
            text(move || t!(unit))
                .pr_2()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .w(px(132.0))
        .h(px(32.0))
        .flex_row()
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(move || theme().input)
        .input_style(|s| s.flex_1().px_3().text_size(px(13.0)).tabular_nums())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_of_the_controls_is_known() {
        let mut theme = nsh_core::tailwind::Theme::default();
        theme.set_tokens(&crate::theme::tokens(crate::theme::ThemeChoice::default()));
        for classes in [
            SLIDER_TRACK,
            SLIDER_RANGE,
            SLIDER_THUMB,
            SWITCH_TRACK,
            SWITCH_THUMB,
        ] {
            let parsed = nsh_core::tailwind::parse(&theme, classes);
            assert!(
                parsed.unknown().is_empty(),
                "{classes:?}: {:?}",
                parsed.unknown()
            );
        }
    }

    #[test]
    fn protocols_are_kept_tidy() {
        assert_eq!(
            tidy_protocols(" HTTPS, http,,mailto: https "),
            "https,http,mailto"
        );
        assert_eq!(tidy_protocols(""), "");
    }
}
