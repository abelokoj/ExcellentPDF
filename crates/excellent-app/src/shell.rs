//! The window's shell (plan §10.1): a title bar of the app's own over a dock area, with the document in the center,
//! the sidebar on the left and the inspector on the right, and the command palette over it all. Every action here
//! is a command from [`crate::commands`], reachable from a button, its shortcut and the palette (plan §2.1, rule 3).
//!
//! The tab strip (R.12) takes the title bar's left; the workspace switcher and the icon rail of §10.1 come with
//! their features (W.1, R.13). Every tab shows on the one page canvas: switching gives it the other file.
//!
//! The pages take the keyboard focus when a document opens and when the reader presses on them; keys scoped to the
//! pages ([`crate::keymap::Scope::Page`]) work while they have it. In full screen (R.19) the title bar and the docks
//! hide ([`FullScreen`]).

use nsh_base::dock::{
    DockAreaLayout, DockLayout, DockPanel, DockPanelOptions, DockPlacement, DockRegion, DockTab,
};
use nsh_base::{CommandEntry, button, command, dock_area, theme};
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{current_window, title_bar};
use nsh_core::geometry::Points;

use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::full_screen::FullScreen;
use crate::goto::{self, PageState};
use crate::links::{self, LinkState};
use crate::list_dialog::{list_dialog, style};
use crate::outline::{self, OutlineState};
use crate::reading::Reading;
use crate::reading_theme_bar::reading_theme_bar;
use crate::recent::RecentState;
use crate::settings;
use crate::settings_window::settings_window;
use crate::tabs::{self, TabState};
use crate::theme as themes;
use crate::thumbnails::{self, Thumbnails};

/// The title bar's height; Windows draws its caption buttons this tall.
const TITLE_BAR_HEIGHT: Points = px(40.0);

/// The panels the shell starts with, keyed by their dock-layout ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Document,
    /// The pages, as thumbnails (R.5).
    Pages,
    /// The document's table of contents (§11.14); bookmarks (B.6) join these as a tab.
    Outline,
    Inspector,
}

impl Panel {
    const ALL: [Self; 4] = [Self::Document, Self::Pages, Self::Outline, Self::Inspector];

    fn key(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Pages => "pages",
            Self::Outline => "outline",
            Self::Inspector => "inspector",
        }
    }

    fn of(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.key() == key)
    }

    /// Its tab's label.
    fn title(self) -> String {
        match self {
            Self::Document => t!("panel.document"),
            Self::Pages => t!("panel.pages"),
            Self::Outline => t!("panel.outline"),
            Self::Inspector => t!("panel.inspector"),
        }
    }

    /// The heading of its empty state.
    fn heading(self) -> String {
        match self {
            Self::Document => t!("document.empty.title"),
            Self::Pages | Self::Outline | Self::Inspector => self.title(),
        }
    }

    /// The line under the heading: the one next step it teaches (plan §10.3).
    fn hint(self) -> String {
        match self {
            Self::Document => t!(
                "document.empty.hint",
                open = commands::shortcut_label(CommandId::OpenFile),
                keys = commands::shortcut_label(CommandId::OpenPalette)
            ),
            Self::Pages => t!("pages.empty"),
            Self::Outline => t!("outline.empty"),
            Self::Inspector => t!("inspector.empty"),
        }
    }
}

/// The layout a new window starts with: the document between a 260-point sidebar and a 300-point inspector.
fn default_layout() -> DockAreaLayout {
    let mut sidebar = DockRegion::new(
        DockLayout::tabs()
            .panel(Panel::Pages.key())
            .panel(Panel::Outline.key()),
    );
    sidebar.size = Some(px(260.0));
    let mut inspector = DockRegion::new(DockLayout::tabs().panel(Panel::Inspector.key()));
    inspector.size = Some(px(300.0));
    DockAreaLayout::new(DockLayout::tabs().panel(Panel::Document.key()))
        .with_dock(DockPlacement::Left, sidebar)
        .with_dock(DockPlacement::Right, inspector)
}

/// What the document panel works with: the canvas and the state its events feed.
#[derive(Clone, Copy)]
struct Document {
    canvas: ElementRef,
    /// The view around the canvas that holds the keyboard focus for it: the canvas is a window of its own.
    focus: ElementRef,
    pages: PageState,
    link_state: LinkState,
    thumbs: Thumbnails,
    reading: Reading,
    full_screen: FullScreen,
    tabs: TabState,
    recent: RecentState,
}

/// Opens a closed dock and closes an open one.
fn toggle(dock: &mut Option<DockRegion>) {
    if let Some(dock) = dock {
        dock.open = Some(!dock.open.unwrap_or(true));
    }
}

pub(crate) fn shell() -> impl IntoElement {
    let layout = signal(default_layout());
    let palette_open = signal(false);
    let theme_open = signal(false);
    let appearance_open = signal(false);
    let goto_open = signal(false);
    let settings_open = signal(false);
    // Not a dialog: it lies in the title bar, so the pages stay in sight to preview each theme.
    let reading_theme_open = signal(false);
    let tab_list_open = signal(false);
    let live = settings::Live::new();
    let pages = PageState::new();
    let tabs = TabState::new(pages, live);
    crate::session::keep(tabs);
    let recent = RecentState::new();
    crate::recent::keep(recent, tabs);
    let outline_state = OutlineState::new(tabs);
    let document = Document {
        canvas: element_ref(),
        focus: element_ref(),
        pages,
        link_state: LinkState::new(tabs),
        thumbs: Thumbnails::new(tabs),
        reading: Reading::new(live),
        full_screen: FullScreen::new(layout),
        tabs,
        recent,
    };
    let Document {
        canvas,
        link_state,
        full_screen,
        reading,
        ..
    } = document;
    // The pickers keep their choice in the settings file; the root starts with it (`crate::root`).
    let theme_choice = signal(themes::saved());
    // An edit of the settings file changes the theme too.
    effect(move || {
        let saved = live.read(themes::ThemeChoice::from_settings);
        if saved != untracked(|| theme_choice.get()) {
            theme_choice.set(saved);
            themes::apply(saved);
        }
    });
    // The page canvas is a window over the app's, so it hides while a dialog is open.
    let dialog_open = move || {
        palette_open.get()
            || theme_open.get()
            || appearance_open.get()
            || goto_open.get()
            || settings_open.get()
            || tab_list_open.get()
            || tabs.editing().is_some()
            || link_state.is_asking()
    };
    // Nothing has the focus at first, and then the pages take it (`page_canvas`).
    let on_page = signal(true);
    document.focus.on_focus_in(move || on_page.set(true));
    document.focus.on_focus_out(move || on_page.set(false));
    let to_canvas = move |name: &str| canvas.command(name, JsonValue::Null);
    let run = move |id: CommandId| match id {
        CommandId::OpenPalette => palette_open.set(true),
        CommandId::OpenSettings => settings_open.set(true),
        CommandId::ChooseTheme => theme_open.set(true),
        CommandId::ChooseAppearance => appearance_open.set(true),
        CommandId::ToggleSidebar => layout.update(|layout| toggle(&mut layout.left)),
        CommandId::ToggleInspector => layout.update(|layout| toggle(&mut layout.right)),
        CommandId::GoToPage => goto_open.set(true),
        CommandId::ToggleFullScreen => full_screen.toggle(),
        CommandId::ToggleRulerInvert => reading.toggle_invert(),
        CommandId::ToggleDocumentColors => reading.toggle_document_colors(),
        CommandId::ChooseReadingTheme => reading_theme_open.set(true),
        CommandId::OpenFile => tabs.open_dialog(),
        CommandId::TabList => tab_list_open.set(true),
        CommandId::SetDefaultViewer => crate::open::set_as_default_viewer(),
        CommandId::Tab(action) => tabs.run(action),
        CommandId::LeaveModes => {
            to_canvas("leaveModes");
            full_screen.leave();
        }
        CommandId::Canvas(name) => to_canvas(name),
    };
    // Later launches' files (R.11), until the window closes.
    spawn(async move {
        loop {
            let launch = crate::open::next().await;
            current_window().focus();
            if tabs.launch(&launch)
                && let Some(page) = launch.page
            {
                canvas.command("goToPage", serde_json::json!({ "index": page }));
            }
        }
    });
    // Bound again when the settings change: the user's keys may have.
    effect(move || {
        live.read(|_| ());
        commands::bind_keys(run, move || {
            on_page.get() && !dialog_open() && !reading_theme_open.get()
        });
    });

    view()
        .flex_1()
        // Documents dropped anywhere on the window open in new tabs (R.11); other files are left alone.
        .on_file_drop(move |event: &FileDropEvent| {
            tabs.open_paths(crate::open::dropped_documents(&event.uris));
        })
        .child(window_bar(run, document, reading_theme_open))
        .child(
            view()
                .flex_1()
                .child(docks(layout, dialog_open, document, outline_state, run)),
        )
        .child(when(palette_open, move || {
            palette(palette_open, pages, run)
        }))
        .child(when(tab_list_open, move || {
            tabs::tab_list(tab_list_open, tabs, recent)
        }))
        .child(when(
            move || tabs.editing().is_some(),
            move || tabs::group_dialog(tabs),
        ))
        .child(when(settings_open, move || {
            settings_window(settings_open, live, theme_choice, run)
        }))
        .child(when(theme_open, move || {
            themes::theme_picker(theme_open, theme_choice)
        }))
        .child(when(appearance_open, move || {
            themes::appearance_picker(appearance_open, theme_choice)
        }))
        .child(when(goto_open, move || {
            goto::goto_dialog(goto_open, pages, move |index| {
                canvas.command("goToPage", serde_json::json!({ "index": index }));
            })
        }))
        .child(when(
            move || link_state.is_asking(),
            move || links::link_dialog(link_state),
        ))
}

/// The title bar: the system's is hidden, so this one drags the window, and Windows draws its caption buttons over
/// the reserved space at the right. In full screen it hides until the pointer reaches the top edge, and pushes the
/// pages down while it shows: the canvas is a window of its own, so nothing of the app's can lie over it.
fn window_bar(
    run: impl Fn(CommandId) + Copy + 'static,
    document: Document,
    reading_theme_open: Signal<bool>,
) -> impl IntoElement {
    let Document {
        pages,
        link_state,
        full_screen,
        reading,
        tabs,
        ..
    } = document;
    let mut bar = TitleBarOptions::default();
    bar.style = TitleBarStyle::Hidden;
    bar.height = Some(TITLE_BAR_HEIGHT);
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(bar);
    current_window().set(attributes);

    view()
        .h(TITLE_BAR_HEIGHT)
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .window_drag(true)
        .pl(move || title_bar().left + px(8.0))
        .pr(move || title_bar().right + px(8.0))
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_b_color(move || theme().border)
        .style_when(move || full_screen.hides_bar(), |s| s.hidden())
        .on_mouse_leave(move |_: &MouseEvent| full_screen.bar_left())
        .child(bar_button(
            "command.toggle_sidebar",
            CommandId::ToggleSidebar,
            run,
            dock_glyph(DockPlacement::Left),
        ))
        // The reading-theme bar needs the room while it is open.
        .child(when(
            move || !reading_theme_open.get(),
            move || tabs::tab_strip(tabs, run),
        ))
        .child(
            view()
                .flex_shrink(0.0)
                .flex_row()
                .justify_center()
                .child(when(
                    move || !reading_theme_open.get(),
                    move || palette_button(run),
                ))
                .child(when(reading_theme_open, move || {
                    reading_theme_bar(reading_theme_open, reading)
                })),
        )
        // shortcut: the link hint (R.7) and the reader's page sit in the title bar until the status bar of plan R.6
        // exists.
        .child(links::link_hint(link_state, pages))
        .child(goto::page_button(pages, move || run(CommandId::GoToPage)))
        .child(bar_button(
            "command.toggle_inspector",
            CommandId::ToggleInspector,
            run,
            dock_glyph(DockPlacement::Right),
        ))
}

/// A small button in the title bar that runs `id`; `label` is its localization key, for screen readers.
fn bar_button(
    label: &'static str,
    id: CommandId,
    run: impl Fn(CommandId) + Copy + 'static,
    glyph: impl IntoElement,
) -> impl IntoElement {
    // Presses on the button must not drag the window.
    view().window_drag(false).child(
        button()
            .accessibility_label(t!(label))
            .on_press(move || run(id))
            .w(px(30.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.7))
            .child(glyph),
    )
}

/// A window outline with a filled strip at the dock's edge, drawn from views: the icon of a dock toggle.
fn dock_glyph(placement: DockPlacement) -> impl IntoElement {
    let outline = view()
        .w(px(16.0))
        .h(px(13.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(move || theme().sidebar_foreground)
        .flex_row();
    let outline = match placement {
        DockPlacement::Right => outline.justify_end(),
        _ => outline,
    };
    outline.child(
        view()
            .w(px(5.0))
            .h_full()
            .bg(move || theme().sidebar_foreground),
    )
}

/// The command palette's place in the title bar, where a browser has its address bar.
fn palette_button(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    view().window_drag(false).w(px(280.0)).child(
        button()
            .accessibility_label(t!("palette.title"))
            .on_press(move || run(CommandId::OpenPalette))
            .h(px(28.0))
            .px_3()
            .flex_row()
            .items_center()
            .gap_2()
            .rounded(px(8.0))
            .bg(move || theme().background)
            .border_1()
            .border_color(move || theme().border)
            .hover(|s| s.border_color(move || theme().ring))
            .child(
                text(move || t!("palette.placeholder"))
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                text(move || commands::shortcut_label(CommandId::OpenPalette))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

/// The dock area: its layout is ours (controlled), so commands can open and close the docks.
fn docks(
    layout: Signal<DockAreaLayout>,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    document: Document,
    outline_state: OutlineState,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let Document {
        canvas,
        pages,
        thumbs,
        full_screen,
        tabs,
        ..
    } = document;
    let go = move |index: usize| {
        canvas.command("goToPage", serde_json::json!({ "index": index }));
    };
    let panels: Vec<DockPanelOptions> = Panel::ALL
        .into_iter()
        .map(|panel| {
            let mut options = DockPanelOptions::new(panel.key());
            options.closable = Some(false);
            options
        })
        .collect();
    dock_area()
        .layout(move || layout.get())
        .on_layout_change(move |next: &DockAreaLayout| layout.set(next.clone()))
        .panels(panels)
        .render_tab(|tab: &DockTab| {
            let panel = Panel::of(tab.panel.as_str());
            text(move || panel.map(Panel::title).unwrap_or_default())
        })
        .render_panel(move |panel: &DockPanel| {
            let panel = Panel::of(panel.id.as_str());
            let shown = move || tabs.has_shown();
            match panel {
                Some(Panel::Document) => document_panel(dialog_open, document, run).into_element(),
                Some(Panel::Pages) => {
                    when(shown, move || thumbnails::thumbnail_list(thumbs, pages, go))
                        .otherwise(move || empty_state(panel))
                        .into_element()
                }
                Some(Panel::Outline) => when(shown, move || {
                    outline::outline_panel(outline_state, pages, go)
                })
                .otherwise(move || empty_state(panel))
                .into_element(),
                _ => empty_state(panel).into_element(),
            }
        })
        .handle_accessibility_label(t!("dock.resize"))
        .group_class("bg-background")
        .dock_class("bg-sidebar")
        // Full screen shows the pages alone: no tab bar over them.
        .tab_bar_class(move || {
            SharedString::from(if full_screen.is_on() {
                "hidden"
            } else {
                "flex-row items-center gap-1 px-2 h-9 border-b border-border"
            })
        })
        .tab_class(
            "px-3 py-1 rounded-md text-sm text-muted-foreground hover:bg-accent \
             selected:bg-accent selected:text-accent-foreground",
        )
        .content_class("flex-1")
        .handle_class("bg-border group-hover:bg-ring group-active:bg-ring")
        .drop_indicator_class("bg-ring opacity-20 rounded-md")
        .size_full()
}

/// The document panel: the page canvas once a tab has been shown (and from then on, so switching tabs only gives it
/// another file), and while no tab is open the way to open one.
fn document_panel(
    dialog_open: impl Fn() -> bool + Copy + 'static,
    document: Document,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let tabs = document.tabs;
    // No canvas (and no swap chain) until there is something to show: cold start without a file stays light.
    let mounted = signal(false);
    effect(move || {
        if tabs.has_shown() && !untracked(|| mounted.get()) {
            mounted.set(true);
        }
    });
    view()
        .flex_1()
        .child(when(mounted, move || page_canvas(dialog_open, document)))
        .child(when(
            move || tabs.shown_missing().is_some(),
            move || document_missing(tabs),
        ))
        .child(when(
            move || !tabs.has_tabs(),
            move || document_empty(run, document.tabs, document.recent),
        ))
}

/// The document panel with no tab open: what to do, and a button that does it.
fn document_empty(
    run: impl Fn(CommandId) + Copy + 'static,
    tabs: TabState,
    recent: RecentState,
) -> impl IntoElement {
    empty_message(
        move || Panel::Document.heading(),
        move || Panel::Document.hint(),
    )
    .child(primary_button("document.open", move || {
        run(CommandId::OpenFile);
    }))
    .child(crate::recent::recent_panel(recent, tabs))
}

/// The document panel while the shown tab's file has gone (R.9): where it was, and "Locate…".
fn document_missing(tabs: TabState) -> impl IntoElement {
    let missing = move || tabs.shown_missing();
    let name = move || {
        missing()
            .and_then(|(_, path)| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default()
    };
    let place = move || {
        missing()
            .and_then(|(_, path)| path.parent().map(|dir| dir.display().to_string()))
            .unwrap_or_default()
    };
    empty_message(
        move || t!("document.missing.title", name = name()),
        move || t!("document.missing.hint", place = place()),
    )
    .child(primary_button("document.locate", move || {
        if let Some((id, _)) = untracked(missing) {
            tabs.locate(id);
        }
    }))
}

/// The one button of a panel's message, in the primary colour, labelled by the translation `key`.
fn primary_button(key: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(t!(key))
        .on_press(on_press)
        .h(px(32.0))
        .px_4()
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .bg(move || theme().primary)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(move || t!(key))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().primary_foreground),
        )
}

/// The document's pages: the platform's page canvas (plan D5), showing the shown tab's file. It is a window of its
/// own, drawn over the app, so it hides while a dialog is open. It draws the theme's page area behind the pages
/// itself. The view around it holds the keyboard focus for it: a Tab stop, and focused when the document opens and
/// when the reader presses on it.
fn page_canvas(
    dialog_open: impl Fn() -> bool + Copy + 'static,
    document: Document,
) -> impl IntoElement {
    let Document {
        canvas,
        focus,
        pages,
        link_state,
        thumbs,
        reading,
        full_screen,
        tabs,
        recent: _,
    } = document;
    let canvas_ref = canvas;
    let label = move || {
        let name = tabs
            .shown_path()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        t!("document.canvas", name = name)
    };
    let canvas = native(crate::PAGE_CANVAS)
        .element_ref(&canvas)
        // Before `path`, so a book opens laid out at the reader's size, not laid out twice.
        .prop("reflowFontSize", move || reading.reflow_font_size_prop())
        .prop("path", move || tabs.path_prop())
        .prop("label", move || JsonValue::from(label()))
        .prop("background", move || {
            JsonValue::from(themes::page_area_hex())
        })
        .prop("pageColors", move || reading.page_colors_prop())
        .prop("readingRuler", move || reading.ruler_prop())
        .prop("focusScope", move || reading.focus_scope_prop())
        .prop("autoScrollSpeed", move || reading.speed_prop())
        .label(move || SharedString::from(label()))
        .on_native_event(move |event: &NativeEvent| {
            if matches!(&*event.name, "pages" | "press") {
                focus.focus();
            }
            pages.on_canvas_event(event);
            tabs.on_canvas_event(event, canvas_ref);
            link_state.on_canvas_event(event);
            thumbs.on_canvas_event(event);
            reading.on_canvas_event(event);
            full_screen.on_canvas_event(event);
            copy_selection(event);
        })
        .flex_1()
        .style_when(
            move || dialog_open() || !tabs.has_shown(),
            |s| s.invisible(),
        );
    view()
        .flex_1()
        .style_when(move || !tabs.has_shown(), |s| s.hidden())
        .focusable(true)
        .element_ref(&focus)
        .label(move || SharedString::from(label()))
        .child(canvas)
}

/// The canvas copied the selected text (K.13): it goes on the clipboard.
fn copy_selection(event: &NativeEvent) {
    if &*event.name == "copy"
        && let Some(text) = event.data.get("text").and_then(JsonValue::as_str)
    {
        set_clipboard(text);
    }
}

/// What a panel shows until its feature fills it: a heading and the next step.
fn empty_state(panel: Option<Panel>) -> impl IntoElement {
    empty_message(
        move || panel.map(Panel::heading).unwrap_or_default(),
        move || panel.map(Panel::hint).unwrap_or_default(),
    )
}

/// A panel with nothing to show: a heading and one line under it.
pub(crate) fn empty_message(
    heading: impl Fn() -> String + 'static,
    hint: impl Fn() -> String + 'static,
) -> View {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(
            text(heading)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(hint)
                .text_size(px(13.0))
                .text_center()
                .text_color(move || theme().muted_foreground),
        )
}

/// The command palette (K.16): every command available now, searchable, with its shortcut; Escape or a choice
/// closes it.
fn palette(
    open: Signal<bool>,
    pages: PageState,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || open.set(false);
    list_dialog(
        t!("palette.title"),
        close,
        style(
            command()
                .items(move || commands::palette_items(pages.count() > 0))
                .placeholder(t!("palette.placeholder"))
                .accessibility_label(t!("palette.title"))
                .auto_focus(true)
                .on_select(move |name: &SharedString| {
                    close();
                    if let Some(id) = commands::find(name) {
                        run(id);
                    }
                })
                .on_cancel(close)
                .render_item(|item: &CommandEntry| {
                    view()
                        .flex_1()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(text(item.label.clone()).flex_1())
                        .child(
                            text(item.shortcut_label.clone().unwrap_or_default())
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        )
                }),
        ),
    )
}
