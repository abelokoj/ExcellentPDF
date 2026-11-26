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
const TRIANGLE_DOWN_PNG: &[u8] = include_bytes!("../../assets/icons/triangle-down.png");

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
    SRC.get_or_init(|| encode_data_uri("data:image/png;base64,", TRIANGLE_DOWN_PNG))
        .clone()
}

/// Renders the Vivaldi-style solid downward triangle used on dropdown buttons across the title bar.
pub(crate) fn dropdown_arrow() -> impl IntoElement {
    image(dropdown_arrow_src())
        .w(px(8.0))
        .h(px(6.0))
        .object_fit(ResizeMode::Contain)
        .tint_color(move || theme().muted_foreground)
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

fn menu_row(label: String, trailing: &str) -> impl IntoElement {
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
        .child(
            text(trailing.to_string())
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
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
        .trigger(menu_row(label.clone(), "›"))
        .back(menu_row(label, "‹"));
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
