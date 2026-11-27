//! Vivaldi-style top-left application menu and crisp vector dropdown arrow.
//!
//! Renders the leftmost title-bar anchor showing the Nib & Leaf app icon paired with a solid downward triangle.
//! Clicking the anchor opens a cascading menu (`File`, `Edit`, `View`, `Bookmarks`, `Tools`, `Window`, `Help`,
//! `Settings...`, and `Exit`). Also exposes [`dropdown_arrow`] so every dropdown button across the chrome uses
//! the same solid triangle instead of `⌄` or rotated squares.

use std::sync::OnceLock;

use base64::Engine as _;
use nsh_base::{MenuItemElement, menu, menu_item, menu_separator, submenu, theme};
use nsh_core::prelude::*;

use crate::bookmarks::BookmarkAction;
use crate::commands::{self, CommandId};
use crate::find::FindAction;
use crate::panes::{PaneAction, Side};
use crate::tabs::{Action as TabAction, Step};
use crate::workspaces::WorkspaceAction;

const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/app-icon-64.png");
const TRIANGLE_DOWN_SVG: &[u8] = include_bytes!("../../assets/icons/triangle-down.svg");
const TRIANGLE_RIGHT_SVG: &[u8] = include_bytes!("../../assets/icons/triangle-right.svg");
const TRIANGLE_LEFT_SVG: &[u8] = include_bytes!("../../assets/icons/triangle-left.svg");
const CHEVRON_DOWN_SVG: &[u8] = include_bytes!("../../assets/icons/chevron-down.svg");
const SIDEBAR_LEFT_SVG: &[u8] = include_bytes!("../../assets/icons/sidebar-left.svg");
const SIDEBAR_RIGHT_SVG: &[u8] = include_bytes!("../../assets/icons/sidebar-right.svg");
const RAIL_PAGES_SVG: &[u8] = include_bytes!("../../assets/icons/rail-pages.svg");
const RAIL_OUTLINE_SVG: &[u8] = include_bytes!("../../assets/icons/rail-outline.svg");
const RAIL_BOOKMARKS_SVG: &[u8] = include_bytes!("../../assets/icons/rail-bookmarks.svg");
const RAIL_ANNOTATIONS_SVG: &[u8] = include_bytes!("../../assets/icons/rail-annotations.svg");
const SEARCH_SVG: &[u8] = include_bytes!("../../assets/icons/search.svg");
const BOOK_SVG: &[u8] = include_bytes!("../../assets/icons/book.svg");
const PALETTE_SVG: &[u8] = include_bytes!("../../assets/icons/palette.svg");
const CLOSE_X_SVG: &[u8] = include_bytes!("../../assets/icons/close-x.svg");
const TRASH_SVG: &[u8] = include_bytes!("../../assets/icons/trash.svg");

/// Monochrome white-on-transparent chrome icons tinted to the active theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChromeIcon {
    /// Top-left toolbar toggle for the left vertical sidebar.
    SidebarLeft,
    /// Top-right toolbar toggle for the right vertical sidebar.
    SidebarRight,
    /// Left rail icon for page thumbnails.
    RailPages,
    /// Left rail icon for the document outline.
    RailOutline,
    /// Left rail icon for user bookmarks.
    RailBookmarks,
    /// Left rail icon for annotations.
    RailAnnotations,
    /// Left rail icon for search results.
    RailSearch,
    /// Left rail quick icon for the library.
    RailLibrary,
    /// Left rail quick icon for settings.
    RailSettings,
    /// Drawer header close icon.
    CloseX,
    /// Trash-can delete icon for annotation and highlight removal.
    Trash,
}

impl ChromeIcon {
    fn uri(self) -> String {
        static CACHE: [OnceLock<String>; 11] = [
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
            OnceLock::new(),
        ];
        let (slot, bytes): (usize, &[u8]) = match self {
            Self::SidebarLeft => (0, SIDEBAR_LEFT_SVG),
            Self::SidebarRight => (1, SIDEBAR_RIGHT_SVG),
            Self::RailPages => (2, RAIL_PAGES_SVG),
            Self::RailOutline => (3, RAIL_OUTLINE_SVG),
            Self::RailBookmarks => (4, RAIL_BOOKMARKS_SVG),
            Self::RailAnnotations => (5, RAIL_ANNOTATIONS_SVG),
            Self::RailSearch => (6, SEARCH_SVG),
            Self::RailLibrary => (7, BOOK_SVG),
            Self::RailSettings => (8, PALETTE_SVG),
            Self::CloseX => (9, CLOSE_X_SVG),
            Self::Trash => (10, TRASH_SVG),
        };
        CACHE[slot]
            .get_or_init(|| encode_data_uri("data:image/svg+xml;base64,", bytes))
            .clone()
    }
}

fn encode_data_uri(prefix: &str, bytes: &[u8]) -> String {
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Returns the cached `data:image/png;base64,...` URI for the Nib & Leaf application icon.
pub(crate) fn app_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| encode_data_uri("data:image/png;base64,", APP_ICON_PNG))
        .clone()
}

fn dropdown_arrow_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| encode_data_uri("data:image/svg+xml;base64,", TRIANGLE_DOWN_SVG))
        .clone()
}

fn triangle_right_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| encode_data_uri("data:image/svg+xml;base64,", TRIANGLE_RIGHT_SVG))
        .clone()
}

fn triangle_left_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| encode_data_uri("data:image/svg+xml;base64,", TRIANGLE_LEFT_SVG))
        .clone()
}

fn tab_list_chevron_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| encode_data_uri("data:image/svg+xml;base64,", CHEVRON_DOWN_SVG))
        .clone()
}

/// Renders the softly rounded downward triangle used on the app-menu dropdown trigger.
pub(crate) fn dropdown_arrow() -> impl IntoElement {
    image(dropdown_arrow_src())
        .w(px(10.0))
        .h(px(7.5))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().muted_foreground)
}

/// Renders the softly rounded downward triangle used on the workspace switcher button in the title bar.
pub(crate) fn workspace_dropdown_arrow() -> impl IntoElement {
    image(dropdown_arrow_src())
        .w(px(14.0))
        .h(px(10.5))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().sidebar_foreground)
}

/// Renders the softly rounded right-pointing triangle matching the top-left app triangle for submenu triggers.
pub(crate) fn submenu_arrow_right() -> impl IntoElement {
    image(triangle_right_src())
        .w(px(7.35))
        .h(px(9.8))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().popover_foreground)
}

/// Renders the softly rounded left-pointing triangle matching the top-left app triangle for submenu back rows.
pub(crate) fn submenu_arrow_left() -> impl IntoElement {
    image(triangle_left_src())
        .w(px(7.35))
        .h(px(9.8))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().popover_foreground)
}

/// Renders the Vivaldi-style downward chevron for the top-right tab-list dropdown button.
pub(crate) fn tab_list_chevron() -> impl IntoElement {
    image(tab_list_chevron_src())
        .w(px(14.0))
        .h(px(14.0))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().foreground)
}

/// Renders a monochrome SVG chrome icon tinted with `tint` at `size` points square.
pub(crate) fn chrome_icon(
    icon: ChromeIcon,
    size: f32,
    tint: impl Fn() -> AdaptiveColor + Copy + 'static,
) -> impl IntoElement {
    image(icon.uri())
        .w(px(size))
        .h(px(size))
        .object_fit(ResizeMode::Contain)
        .tint_color(tint)
}

/// Windows-style red hover background (`#E81123`) used by every close (`×`) button across
/// windows, tabs, panes, drawers, bars, and popups.
pub(crate) fn close_hover_red() -> Hsla {
    Hsla::parse_hex("#e81123").expect("valid hex color")
}

/// Crisp white icon colour used over [`close_hover_red`] while hovering a close (`×`) button.
pub(crate) fn close_hover_fg() -> AdaptiveColor {
    Hsla::parse_hex("#ffffff").expect("valid hex color").into()
}

/// Renders a crisp SVG close (`×`) button whose background turns red (`#E81123`) with a white
/// glyph on hover so users can immediately recognize close/cancel targets across any theme.
pub(crate) fn close_x_button(
    label: impl Into<SharedString>,
    box_size: f32,
    icon_size: f32,
    radius: f32,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let hovered = signal(false);
    nsh_base::button()
        .accessibility_label(label.into())
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press(on_press)
        .cursor(Cursor::Pointer)
        .w(px(box_size))
        .h(px(box_size))
        .flex_shrink(0.0)
        .rounded(px(radius))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(close_hover_red))
        .active(|s| s.opacity(0.82))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(chrome_icon(ChromeIcon::CloseX, icon_size, move || {
            if hovered.get() {
                close_hover_fg()
            } else {
                theme().muted_foreground
            }
        }))
}

fn command_item(
    id: CommandId,
    run: impl Fn(CommandId) + Copy + 'static,
    has_document: impl Fn() -> bool + Copy + 'static,
) -> MenuItemElement {
    let shortcut = commands::shortcut_label(id);
    menu_item()
        .on_select(move || run(id))
        .disabled(move || !id.is_available(has_document()))
        .accessibility_label(commands::label_with_shortcut(id))
        .flex_row()
        .items_center()
        .gap_4()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(commands::label(id))
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(shortcut)
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
}

fn action_item(label: String, select: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(label.clone())
        .flex_row()
        .items_center()
        .gap_4()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(
            text(label)
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

/// Direction of the softly rounded submenu arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmenuArrow {
    Right,
    Left,
}

fn menu_row(label: String, arrow: SubmenuArrow) -> impl IntoElement {
    let icon = match arrow {
        SubmenuArrow::Right => submenu_arrow_right().into_element(),
        SubmenuArrow::Left => submenu_arrow_left().into_element(),
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(
            text(label)
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(icon)
}

fn divider() -> impl IntoElement {
    menu_separator()
        .h(px(1.0))
        .my_1()
        .mx_1()
        .bg(move || theme().border)
}

fn build_submenu(
    label: String,
    items: Vec<Option<CommandId>>,
    run: impl Fn(CommandId) + Copy + 'static,
    has_document: impl Fn() -> bool + Copy + 'static,
) -> nsh_base::SubmenuElement {
    let mut sub = submenu()
        .offset(px(2.0))
        .trigger(menu_row(label.clone(), SubmenuArrow::Right))
        .back(menu_row(label, SubmenuArrow::Left));
    for entry in items {
        sub = match entry {
            Some(id) => sub.child(command_item(id, run, has_document)),
            None => sub.child(divider()),
        };
    }
    sub
}

/// Top-left Vivaldi-style app icon and cascading dropdown menu anchored before workspaces and tabs.
pub(crate) fn app_menu_button(
    open: Signal<bool>,
    run: impl Fn(CommandId) + Copy + 'static,
    has_document: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let file_items = vec![
        Some(CommandId::OpenFile),
        Some(CommandId::OpenUrl),
        Some(CommandId::Pane(PaneAction::OpenInSplit)),
        Some(CommandId::NewNotebook),
        None,
        Some(CommandId::Save),
        Some(CommandId::ExportAndConvert),
        Some(CommandId::Print),
        None,
        Some(CommandId::Tab(TabAction::Close)),
        Some(CommandId::Tab(TabAction::ReopenClosed)),
    ];
    let edit_items = vec![
        Some(CommandId::Undo),
        Some(CommandId::Redo),
        None,
        Some(CommandId::Find(FindAction::Open)),
        Some(CommandId::Find(FindAction::Next)),
        Some(CommandId::Find(FindAction::Previous)),
        None,
        Some(CommandId::CutAnnotations),
        Some(CommandId::PasteAnnotations),
        Some(CommandId::DeleteAnnotations),
    ];
    let view_items = vec![
        Some(CommandId::ToggleSidebar),
        Some(CommandId::ToggleInspector),
        Some(CommandId::ToggleFullScreen),
        None,
        Some(CommandId::GoToPage),
        Some(CommandId::ReflowMode),
        Some(CommandId::BionicReading),
        None,
        Some(CommandId::ChooseTheme),
        Some(CommandId::ChooseAppearance),
        Some(CommandId::ChooseReadingTheme),
    ];
    let bookmark_items = vec![
        Some(CommandId::Bookmark(BookmarkAction::Add)),
        Some(CommandId::Bookmark(BookmarkAction::Edit)),
        Some(CommandId::Bookmark(BookmarkAction::Delete)),
        Some(CommandId::Bookmark(BookmarkAction::ToOutline)),
    ];
    let tool_items = vec![
        Some(CommandId::OpenPalette),
        Some(CommandId::OpenLibrary),
        Some(CommandId::HighlightSummary),
        Some(CommandId::PageOrganizer),
        Some(CommandId::OcrDocument),
        Some(CommandId::ComparePdfs),
        Some(CommandId::AiPanel),
        Some(CommandId::PluginsManager),
    ];
    let window_items = vec![
        Some(CommandId::Pane(PaneAction::Split(Side::Right))),
        Some(CommandId::Pane(PaneAction::Split(Side::Down))),
        Some(CommandId::Pane(PaneAction::Close)),
        None,
        Some(CommandId::TabList),
        Some(CommandId::Tab(TabAction::Cycle(Step::Next))),
        Some(CommandId::Tab(TabAction::Cycle(Step::Previous))),
        None,
        Some(CommandId::Workspace(WorkspaceAction::Switcher)),
        Some(CommandId::Workspace(WorkspaceAction::OpenWindow)),
    ];
    let help_items = vec![
        Some(CommandId::ReplayGuide),
        Some(CommandId::CliMcpDocs),
        Some(CommandId::SetDefaultViewer),
    ];

    menu()
        .align(Align::Start)
        .offset(px(4.0))
        .on_open(move || open.set(true))
        .on_close(move || open.set(false))
        .popup_class("w-60 p-1 rounded-lg border border-border bg-popover shadow-lg")
        .trigger(
            view()
                .label(t!("app_menu.label"))
                .h(px(26.0))
                .px_1p5()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .rounded(px(6.0))
                .style_when(move || open.get(), |s| s.bg(move || theme().sidebar_accent))
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .active(|s| s.opacity(0.75))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    image(app_icon_src())
                        .w(px(18.0))
                        .h(px(18.0))
                        .rounded(px(4.0))
                        .object_fit(ResizeMode::Contain),
                )
                .child(dropdown_arrow()),
        )
        .child(build_submenu(
            t!("app_menu.file"),
            file_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.edit"),
            edit_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.view"),
            view_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.bookmarks"),
            bookmark_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.tools"),
            tool_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.window"),
            window_items,
            run,
            has_document,
        ))
        .child(build_submenu(
            t!("app_menu.help"),
            help_items,
            run,
            has_document,
        ))
        .child(divider())
        .child(command_item(CommandId::OpenSettings, run, has_document))
        .child(divider())
        .child(action_item(t!("app_menu.exit"), || {
            nsh_core::element::desktop::quit();
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_chrome_icons_use_valid_svg_data_uris() {
        let uris = [
            ChromeIcon::SidebarLeft.uri(),
            ChromeIcon::SidebarRight.uri(),
            ChromeIcon::RailPages.uri(),
            ChromeIcon::RailOutline.uri(),
            ChromeIcon::RailBookmarks.uri(),
            ChromeIcon::RailAnnotations.uri(),
            ChromeIcon::RailSearch.uri(),
            ChromeIcon::RailLibrary.uri(),
            ChromeIcon::RailSettings.uri(),
            ChromeIcon::CloseX.uri(),
            ChromeIcon::Trash.uri(),
            dropdown_arrow_src(),
            triangle_right_src(),
            triangle_left_src(),
            tab_list_chevron_src(),
        ];
        for uri in uris {
            let encoded = uri
                .strip_prefix("data:image/svg+xml;base64,")
                .expect("chrome icon must be an SVG data URI");
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("valid base64");
            let xml = std::str::from_utf8(&decoded).expect("UTF-8 SVG");
            assert!(
                xml.contains("<svg") && xml.contains("</svg>"),
                "valid SVG: {xml}"
            );
            assert!(
                !xml.contains("currentColor"),
                "standalone SVG must not use currentColor"
            );
            let raster = nsh_core::svg::rasterize(&decoded, 48)
                .expect("NSH svg feature must be enabled and rasterize every chrome icon");
            assert!(raster.width > 0 && raster.height > 0);
            assert!(
                raster.pixels.iter().skip(3).step_by(4).any(|&a| a > 128),
                "rasterized chrome SVG must have visible opaque pixels"
            );
        }

        // Also rasterize every SVG file in assets/icons/ so no icon can ever ship blank.
        let icons_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let mut count = 0usize;
        for entry in std::fs::read_dir(&icons_dir).expect("read assets/icons") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("svg") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read svg");
            let raster = nsh_core::svg::rasterize(&bytes, 48)
                .unwrap_or_else(|err| panic!("failed to rasterize {}: {err}", path.display()));
            assert!(
                raster.width > 0
                    && raster.height > 0
                    && raster.pixels.iter().skip(3).step_by(4).any(|&a| a > 128),
                "SVG {} produced blank output",
                path.display()
            );
            count += 1;
        }
        assert!(
            count >= 43,
            "all embedded SVG UI, workspace, and tab-list icons must rasterize cleanly (found {count})"
        );
    }
}
