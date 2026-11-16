//! The window's shell (plan §10.1): a title bar of the app's own over a dock area, with the document in the center,
//! the sidebar on the left and the inspector on the right, and the command palette over it all. Every action here
//! is a command from [`crate::commands`], reachable from a button, its shortcut and the palette (plan §2.1, rule 3).
//!
//! The tab strip (R.12) takes the title bar's left; the workspace switcher and the icon rail of §10.1 come with
//! their features (W.1, R.13). The document area is the split view's grid of panes ([`crate::panes`]): the tab
//! strip, the page button, the sidebar's panels and every command are the active pane's (V.5).
//!
//! Every window of the app has this shell (W.8): [`shell`] is the main window's and does what the app does once (the
//! theme, the language, the windows of the last run); [`open_workspace_window`] opens another on a workspace of its
//! own. What they share is [`App`].
//!
//! The pages take the keyboard focus when a document opens and when the reader presses on them; keys scoped to the
//! pages ([`crate::keymap::Scope::Page`]) work while they have it. In full screen (R.19) the title bar and the docks
//! hide ([`FullScreen`]).

use excellent_store::workspace::WorkspaceId;
use excellent_store::workspaces::Workspace;
use nsh_base::dock::{
    DockAreaLayout, DockLayout, DockNode, DockPanel, DockPanelOptions, DockPlacement, DockRegion,
    DockTab,
};
use nsh_base::{CommandEntry, button, command, dock_area, theme};
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{OpenChange, current_window, open_window};
use nsh_core::element::{create_root, next_frame};
use nsh_core::geometry::Points;

use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::annotate::{Kind, Placing};
use crate::bookmarks::{self, BookmarkAction};
use crate::commands::{self, CommandId};
use crate::full_screen::FullScreen;
use crate::goto::{self, PageState};
use crate::links;
use crate::list_dialog::{list_dialog, style};
use crate::modes::{self, Modes};
use crate::onboarding::{self, Onboarding};
use crate::outline::{self, OutlineAction};
use crate::panes::{self, CELLS, FileDrag, Pane, PaneAction, Panes, Place, Shared};
use crate::print::{Printing, print_strip};
use crate::reading::Reading;
use crate::reading_theme_bar::reading_theme_bar;
use crate::recent::RecentState;
use crate::settings;
use crate::settings_window::settings_window;
use crate::tabs;
use crate::theme as themes;
use crate::thumbnails;
use crate::workspaces::{self, MAIN, Workspaces};

/// The title bar's height; Windows draws its caption buttons this tall.
const TITLE_BAR_HEIGHT: Points = px(40.0);

/// Below this window width the docks become drawers over the document (U.4): with the sidebar's 260 points and the
/// inspector's 300 both open, the page would get less than 340. A 1280-point screen at 150 % scaling is 853.
const COMPACT_BELOW: Points = px(900.0);

/// A workspace window's size before the platform keeps one of its own (W.8).
const WINDOW_SIZE: (f32, f32) = (1100.0, 800.0);

/// How often the system's high contrast is looked at (U.4).
const SYSTEM_CONTRAST_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// The panels the shell starts with, keyed by their dock-layout ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Document,
    /// The pages, as thumbnails (R.5).
    Pages,
    /// The document's table of contents (§11.14).
    Outline,
    /// The reader's own bookmarks in the document (B.6).
    Bookmarks,
    /// What the find bar found, with the words round each hit (F.1).
    Results,
    Inspector,
}

impl Panel {
    const ALL: [Self; 6] = [
        Self::Document,
        Self::Pages,
        Self::Outline,
        Self::Bookmarks,
        Self::Results,
        Self::Inspector,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Pages => "pages",
            Self::Outline => "outline",
            Self::Bookmarks => "bookmarks",
            Self::Results => "results",
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
            Self::Bookmarks => t!("panel.bookmarks"),
            Self::Results => t!("panel.results"),
            Self::Inspector => t!("panel.inspector"),
        }
    }

    /// The heading of its empty state.
    fn heading(self) -> String {
        match self {
            Self::Document => t!("document.empty.title"),
            Self::Pages | Self::Outline | Self::Bookmarks | Self::Results | Self::Inspector => {
                self.title()
            }
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
            Self::Bookmarks => t!("bookmark.empty"),
            Self::Results => t!("find.no_document"),
            Self::Inspector => t!("inspector.empty"),
        }
    }
}

/// The layout a new window starts with: the document between a 260-point sidebar and a 300-point inspector.
fn default_layout() -> DockAreaLayout {
    let mut sidebar = DockRegion::new(
        DockLayout::tabs()
            .panel(Panel::Pages.key())
            .panel(Panel::Outline.key())
            .panel(Panel::Bookmarks.key()),
    );
    sidebar.size = Some(px(260.0));
    // The find results sit beside the inspector: a fourth tab would not fit the sidebar's width.
    let mut inspector = DockRegion::new(
        DockLayout::tabs()
            .panel(Panel::Inspector.key())
            .panel(Panel::Results.key()),
    );
    inspector.size = Some(px(300.0));
    DockAreaLayout::new(DockLayout::tabs().panel(Panel::Document.key()))
        .with_dock(DockPlacement::Left, sidebar)
        .with_dock(DockPlacement::Right, inspector)
}

/// Shows the panel `key` wherever the reader put it: its tab is chosen, and its dock opened.
fn show_panel(layout: &mut DockAreaLayout, key: &str) {
    fn choose(node: &mut DockNode, key: &str) -> bool {
        match node {
            DockNode::Tabs {
                panels,
                active_index,
                ..
            } => panels
                .iter()
                .position(|panel| panel.as_str() == key)
                .map(|index| *active_index = index)
                .is_some(),
            DockNode::Split { children, .. } => children.iter_mut().any(|child| choose(child, key)),
        }
    }
    if choose(&mut layout.center, key) {
        return;
    }
    for dock in [&mut layout.left, &mut layout.right, &mut layout.bottom]
        .into_iter()
        .flatten()
    {
        if choose(&mut dock.layout, key) {
            dock.open = Some(true);
            return;
        }
    }
}

/// Opens a closed dock and closes an open one.
fn toggle(dock: &mut Option<DockRegion>) {
    if let Some(dock) = dock {
        dock.open = Some(!dock.open.unwrap_or(true));
    }
}

/// What every window of the app shares (W.8): the settings, the recent files, the theme picked and the list of
/// workspaces.
#[derive(Clone, Copy)]
pub(crate) struct App {
    pub(crate) live: settings::Live,
    pub(crate) recent: RecentState,
    pub(crate) workspace_list: Signal<Vec<Workspace>>,
    /// The pickers keep their choice in the settings file; the root starts with it (`crate::root`).
    theme_choice: Signal<themes::ThemeChoice>,
}

/// Which window a shell is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The main window: later launches' files come to it, and the guide opens in it (U.9).
    Main,
    /// A workspace's window (W.8).
    Workspace,
}

/// The main window's shell, and what the app does once: following the theme, the language and the system's
/// contrast, and opening the workspace windows of the last run (W.8).
pub(crate) fn shell() -> impl IntoElement {
    let live = settings::Live::new();
    let app = App {
        live,
        recent: RecentState::new(),
        workspace_list: workspaces::shared_list(),
        theme_choice: signal(themes::saved()),
    };
    let theme_choice = app.theme_choice;
    // An edit of the settings file changes the theme too.
    effect(move || {
        let saved = live.read(themes::ThemeChoice::from_settings);
        let followed = live.read(themes::follow_system);
        if saved != untracked(|| theme_choice.get()) {
            theme_choice.set(saved);
            themes::apply(saved);
        } else if followed {
            themes::apply(saved);
        }
    });
    // The reader's language (U.7), from the settings file as it changes.
    effect(move || live.read(crate::l10n::apply));
    // The system's high contrast, turned on or off while the app runs (U.4).
    // shortcut: a look every two seconds; listen for WM_SETTINGCHANGE if NSH's host ever passes it on.
    set_interval(SYSTEM_CONTRAST_POLL, move || {
        if themes::follow_system(&settings::shared()) {
            themes::apply(untracked(|| theme_choice.get()));
        }
    });
    let (shown, restored) = workspaces::opening();
    let panes = Panes::new(live, app.recent, restored);
    let workspaces = Workspaces::new(app, panes, shown, MAIN);
    workspaces.set_window(current_window());
    // W.8: the windows open when the app last quit, once the main window is up.
    spawn(async move {
        next_frame().await;
        for id in workspaces::reopened(shown) {
            open_workspace_window(app, id);
        }
    });
    window_shell(app, panes, workspaces, Role::Main)
}

/// Opens workspace `id` in a window of its own (W.8), with every part of the main window. The caller checked that no
/// window shows it.
pub(crate) fn open_workspace_window(app: App, id: WorkspaceId) {
    let no = workspaces::next_window();
    // The window's panes and workspaces live until its content goes (the cleanup below), not with the window that
    // opened it; they are made out here so its closing can still save them.
    let (owner, (panes, workspaces)) = create_root(move || {
        let restored = crate::session::load(id).unwrap_or_default();
        let panes = Panes::new(app.live, app.recent, restored);
        (panes, Workspaces::new(app, panes, id, no))
    });
    let opened = open_window(
        nsh_core::element::desktop::window()
            .id(format!("workspace-{no}"))
            .title(move || {
                let name = workspaces
                    .shown()
                    .map(|w| workspaces::display_name(&w))
                    .unwrap_or_default();
                t!("workspace.window_name", name = name)
            })
            .default_size(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)))
            // However it closes, its tabs are saved and its workspace is free.
            .on_open_change(move |_: &OpenChange| workspaces.close())
            .content(move || {
                cleanup(move || owner.dispose());
                window_shell(app, panes, workspaces, Role::Workspace)
            }),
    );
    match opened {
        Some(window) => workspaces.set_window(window),
        // No windows of the app's own here.
        None => {
            workspaces.close();
            owner.dispose();
        }
    }
}

/// A window's shell, showing `workspaces`' shown workspace in `panes`.
fn window_shell(app: App, panes: Panes, workspaces: Workspaces, role: Role) -> impl IntoElement {
    let App {
        live,
        recent,
        theme_choice,
        ..
    } = app;
    let layout = signal(default_layout());
    let palette_open = signal(false);
    let theme_open = signal(false);
    let appearance_open = signal(false);
    let goto_open = signal(false);
    let settings_open = signal(false);
    // Not a dialog: it lies in the title bar, so the pages stay in sight to preview each theme.
    let reading_theme_open = signal(false);
    let tab_list_open = signal(false);
    let printing = Printing::new();
    let shared = Shared {
        reading: Reading::new(live),
        full_screen: FullScreen::new(layout),
        recent,
        file_drag: FileDrag::new(),
        covers: crate::home::Covers::new(),
        workspaces,
    };
    let Shared {
        full_screen,
        reading,
        file_drag,
        ..
    } = shared;
    // U.9: the guide opens now when this start is its moment; tips come at theirs. A workspace window opens no guide.
    let onboarding = match role {
        Role::Main => Onboarding::new(live),
        Role::Workspace => Onboarding::quiet(live),
    };
    // K.18: the mode and its toolbar; the tab strip and the panes find it in the context (the accent, the pen).
    let modes = Modes::new(live);
    provide_context(modes);
    effect(move || {
        let tabs = panes.active().tabs;
        if tabs.count() >= 2 {
            onboarding.moment(excellent_features::onboarding::Tip::TabList);
        } else if tabs.has_shown() {
            onboarding.moment(excellent_features::onboarding::Tip::Palette);
        }
    });
    // The page canvases are windows over the app's, so they hide while a dialog is open.
    let dialog_open = move || {
        palette_open.get()
            || theme_open.get()
            || appearance_open.get()
            || goto_open.get()
            || settings_open.get()
            || tab_list_open.get()
            || onboarding.is_open()
            || modes.is_customizing()
            || workspaces.dialog().is_some()
            || panes.slots().into_iter().any(Pane::covers_pages)
            || panes.sync_menu_open()
    };
    // Which panes' pages have the keyboard focus. Nothing has it at first, and then the pages take it
    // (`crate::panes`); moving between panes reports one leaving and the other coming in, in either order.
    let on_pages: [Signal<bool>; CELLS] = std::array::from_fn(|cell| signal(cell == 0));
    for (pane, on) in panes.slots().into_iter().zip(on_pages) {
        pane.focus.on_focus_in(move || on.set(true));
        pane.focus.on_focus_out(move || on.set(false));
    }
    let on_page = move || on_pages.iter().any(|on| on.get());
    let run = move |id: CommandId| {
        let pane = panes.active();
        let to_canvas = move |name: &str| pane.canvas.command(name, JsonValue::Null);
        match id {
            CommandId::OpenPalette => palette_open.set(true),
            CommandId::OpenSettings => settings_open.set(true),
            CommandId::ReplayGuide => onboarding.replay(),
            CommandId::Mode(mode) => modes.set(mode),
            CommandId::StepMode(back) => modes.step(back),
            CommandId::CustomizeToolbar => modes.customize(),
            CommandId::ExportToolbars => modes.export(),
            CommandId::ImportToolbars => modes.import(),
            CommandId::ChooseTheme => theme_open.set(true),
            CommandId::ChooseAppearance => appearance_open.set(true),
            CommandId::ToggleSidebar => layout.update(|layout| toggle(&mut layout.left)),
            CommandId::ToggleInspector => layout.update(|layout| toggle(&mut layout.right)),
            CommandId::GoToPage => goto_open.set(true),
            CommandId::ToggleFullScreen => full_screen.toggle(),
            CommandId::ToggleRulerInvert => reading.toggle_invert(),
            CommandId::ToggleDocumentColors => reading.toggle_document_colors(),
            CommandId::ChooseReadingTheme => reading_theme_open.set(true),
            CommandId::OpenFile => tabs::pick_documents(move |paths| workspaces.open_paths(paths)),
            CommandId::TabList => tab_list_open.set(true),
            CommandId::SetDefaultViewer => crate::open::set_as_default_viewer(),
            CommandId::Tab(action) => pane.tabs.run(action),
            // V.11: the window's pages need the shared state, which the shell holds.
            CommandId::Pane(PaneAction::PopOut) => crate::windows::pop_out(panes, shared),
            CommandId::Pane(action) => panes.run(action),
            CommandId::Workspace(action) => workspaces.run(action),
            CommandId::LeaveModes => {
                to_canvas("leaveModes");
                full_screen.leave();
            }
            CommandId::Canvas(name) => to_canvas(name),
            // B.10: the outline and the bookmarks hand each other an entry.
            CommandId::Outline(OutlineAction::ToBookmark) => {
                if let Some((title, page, spot)) = pane.outline.selected_place() {
                    pane.bookmarks.add_from_outline(title, page, spot);
                }
            }
            // V.9: the selected outline entry's destination, in the other pane.
            CommandId::Outline(OutlineAction::OpenElsewhere) => {
                if let Some((_, page, spot)) = pane.outline.selected_place() {
                    panes.open_elsewhere(
                        pane.cell,
                        Place {
                            page,
                            spot: Some(spot),
                        },
                    );
                }
            }
            CommandId::Bookmark(BookmarkAction::ToOutline) => {
                if let Some((title, page, spot)) = pane.bookmarks.selected_place() {
                    pane.outline.add_from_bookmark(&title, page, spot);
                }
            }
            CommandId::Outline(action) => pane.outline.run(action, pane.canvas),
            CommandId::Bookmark(action) => pane.bookmarks.run(action, pane.canvas),
            CommandId::Find(action) => pane.find.run(action),
            CommandId::StickyNote => pane.annotate.place(Placing::Sticky),
            CommandId::TextBox => pane.annotate.place(Placing::Text(Kind::Box)),
            CommandId::Typewriter => pane.annotate.place(Placing::Text(Kind::Typewriter)),
            CommandId::AnnotationColors => pane
                .annotate
                .pick(excellent_features::annotate::colors::Tool::Highlight),
            CommandId::Undo => pane.editing.undo(),
            CommandId::Redo => pane.editing.redo(),
            // Every pane's edits.
            CommandId::Save => panes
                .slots()
                .into_iter()
                .for_each(|pane| pane.editing.save_now()),
            CommandId::Print => {
                if let Some(path) = pane.tabs.shown_path() {
                    printing.start(path, pane.pages.count(), pane.pages.current(), live);
                }
            }
        }
    };
    // Later launches' files (R.11), until the window closes; the main window takes them, or hands them to the window
    // that shows their workspace (W.6, W.8).
    if role == Role::Main {
        spawn(async move {
            loop {
                let launch = crate::open::next().await;
                current_window().focus();
                // The files are what the reader wants now (U.9).
                onboarding.make_way();
                workspaces.open(&launch);
            }
        });
    }
    // Bound again when the settings change: the user's keys may have.
    effect(move || {
        live.read(|_| ());
        commands::bind_keys(run, move || {
            on_page() && !dialog_open() && !reading_theme_open.get()
        });
    });
    // A note opened on the pages shows in the inspector (A.4).
    for pane in panes.slots() {
        effect(move || {
            if pane.annotate.reveals() > 0 {
                untracked(|| {
                    layout.update(|layout| show_panel(layout, Panel::Inspector.key()));
                });
            }
        });
    }
    // Each pane's own dialogs: the group being edited, the bookmark being written, the link being asked about, the
    // text of a proposed edit (A.2), the markup tools' colours (A.3).
    let pane_dialogs = panes.slots().into_iter().fold(view(), |dialogs, pane| {
        dialogs
            .child(when(
                move || pane.tabs.editing().is_some(),
                move || tabs::group_dialog(pane.tabs),
            ))
            .child(when(
                move || pane.bookmarks.is_editing(),
                move || bookmarks::bookmark_dialog(pane.bookmarks),
            ))
            .child(when(
                move || pane.link_state.is_asking(),
                move || links::link_dialog(pane.link_state),
            ))
            .child(when(
                move || pane.annotate.is_asking(),
                move || crate::annotate::text_edit_dialog(pane.annotate),
            ))
            .child(when(
                move || pane.annotate.is_picking(),
                move || crate::annotate::color_dialog(pane.annotate),
            ))
    });

    view()
        .flex_1()
        // A right-to-left language the reader chose mirrors the window; with none chosen, the system's decides (U.7).
        .direction(move || live.read(crate::l10n::direction))
        // Documents dropped on the window outside the panes open in the active pane (R.11), or where another
        // workspace has them (W.6); other files are left alone. Every drop target follows the drag, so the panes' edges take drops while it lasts (V.2).
        .on_file_drag_enter(move || file_drag.enter())
        .on_file_drag_leave(move || file_drag.leave())
        .on_file_drop(move |event: &FileDropEvent| {
            file_drag.drop_done();
            workspaces.open_paths(crate::open::dropped_documents(&event.uris));
        })
        .child(window_bar(
            run,
            panes,
            workspaces,
            shared,
            reading_theme_open,
        ))
        .child(modes::mode_bar(
            modes,
            move || panes.active().pages.count() > 0,
            move || full_screen.is_on(),
            run,
        ))
        .child(
            view()
                .flex_1()
                .child(docks(layout, dialog_open, panes, shared, run)),
        )
        // O.22: a print job's progress, below the pages (they are a window over the app's).
        .child(print_strip(printing))
        // U.9: a tip, once, below the pages.
        .child(onboarding::tip_strip(onboarding))
        .child(when(palette_open, move || {
            palette(palette_open, panes.active().pages, run)
        }))
        .child(when(tab_list_open, move || {
            tabs::tab_list(tab_list_open, panes.active().tabs, recent, workspaces)
        }))
        .child(pane_dialogs)
        .child(workspaces::workspace_dialogs(workspaces, run))
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
            let pane = panes.active();
            goto::goto_dialog(goto_open, pane.pages, move |index| pane.go_to(index))
        }))
        .child(modes::customize_dialog(modes))
        .child(onboarding::guide_dialog(onboarding, run))
}

/// Elements of the active pane, made again when another pane becomes active (V.5).
fn of_active<E: IntoElement>(panes: Panes, make: impl Fn(Pane) -> E + 'static) -> AnyElement {
    dynamic(move || {
        let cell = panes.active_cell();
        untracked(|| make(panes.slots()[cell]).into_element())
    })
}

/// The title bar: the system's is hidden, so this one drags the window, and Windows draws its caption buttons over
/// the reserved space at the right. In full screen it hides until the pointer reaches the top edge, and pushes the
/// pages down while it shows: the canvas is a window of its own, so nothing of the app's can lie over it.
fn window_bar(
    run: impl Fn(CommandId) + Copy + 'static,
    panes: Panes,
    workspaces: Workspaces,
    shared: Shared,
    reading_theme_open: Signal<bool>,
) -> impl IntoElement {
    let Shared {
        full_screen,
        reading,
        ..
    } = shared;
    let mut bar = TitleBarOptions::default();
    bar.style = TitleBarStyle::Hidden;
    bar.height = Some(TITLE_BAR_HEIGHT);
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(bar);
    // This window's, the main one or a workspace's (W.8).
    let window = current_window();
    window.set(attributes);
    let left = window.clone();

    view()
        .h(TITLE_BAR_HEIGHT)
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .window_drag(true)
        .pl(move || left.title_bar().left + px(8.0))
        .pr(move || window.title_bar().right + px(8.0))
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
        .child(workspaces::switcher_button(workspaces, run))
        // The reading-theme bar needs the room while it is open.
        .child(when(
            move || !reading_theme_open.get(),
            move || of_active(panes, move |pane| tabs::tab_strip(pane.tabs, run)),
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
        .child(of_active(panes, |pane| {
            links::link_hint(pane.link_state, pane.pages)
        }))
        .child(of_active(panes, move |pane| {
            goto::page_button(pane.pages, move || run(CommandId::GoToPage))
        }))
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
    panes: Panes,
    shared: Shared,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let full_screen = shared.full_screen;
    let panels: Vec<DockPanelOptions> = Panel::ALL
        .into_iter()
        .map(|panel| {
            let mut options = DockPanelOptions::new(panel.key());
            options.closable = Some(false);
            options
        })
        .collect();
    dock_area()
        // None leaves NSH's own rule (below 600 points of the area's width) for narrower panes.
        .compact(move || (nsh_core::element::window().width < COMPACT_BELOW).then_some(true))
        .layout(move || layout.get())
        .on_layout_change(move |next: &DockAreaLayout| layout.set(next.clone()))
        .panels(panels)
        .render_tab(|tab: &DockTab| {
            let panel = Panel::of(tab.panel.as_str());
            text(move || panel.map(Panel::title).unwrap_or_default())
        })
        .render_panel(move |panel: &DockPanel| {
            let panel = Panel::of(panel.id.as_str());
            match panel {
                Some(Panel::Document) => {
                    panes::pane_grid(panes, shared, dialog_open, run).into_element()
                }
                Some(
                    side @ (Panel::Pages
                    | Panel::Outline
                    | Panel::Bookmarks
                    | Panel::Results
                    | Panel::Inspector),
                ) => of_active(panes, move |pane| side_panel(side, pane, panes, run)),
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

/// The one button of a panel's message, in the primary colour, labelled by the translation `key`.
pub(crate) fn primary_button(key: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
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

/// A sidebar panel (R.5, §11.14, B.6) for the active pane: its content while the pane shows a document, else what
/// it will show.
fn side_panel(
    panel: Panel,
    pane: Pane,
    panes: Panes,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let shown = move || pane.tabs.has_shown();
    let go = move |index: usize| pane.go_to(index);
    let elsewhere = move |place: Place| panes.open_elsewhere(pane.cell, place);
    when(shown, move || match panel {
        Panel::Pages => thumbnails::thumbnail_list(pane.thumbs, pane.pages, go).into_element(),
        Panel::Outline => {
            outline::outline_panel(pane.outline, go, run, elsewhere, pane.focus).into_element()
        }
        Panel::Bookmarks => {
            bookmarks::bookmark_panel(pane.bookmarks, pane.canvas, run, pane.focus).into_element()
        }
        Panel::Results => {
            crate::find::results_panel(pane.find, pane.pages, elsewhere).into_element()
        }
        Panel::Inspector => crate::annotate::note_panel(pane.annotate).into_element(),
        Panel::Document => empty_state(Some(panel)).into_element(),
    })
    .otherwise(move || empty_state(Some(panel)))
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
