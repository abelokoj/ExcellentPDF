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

pub(crate) mod ai_ui;
pub(crate) mod app_menu;
pub(crate) mod control_channel;
pub(crate) mod ecosystem_ui;
pub(crate) mod forms_sign_ui;
pub(crate) mod library_ui;
pub(crate) mod pro_pdf_ui;
pub(crate) mod reading_extra;

use std::cell::Cell;

use ecclesia_store::workspace::WorkspaceId;
use ecclesia_store::workspaces::Workspace;
use nsh_base::dock::{DockAreaLayout, DockLayout};
use nsh_base::{CommandEntry, button, command, theme};
use nsh_core::desktop::{QuitMode, TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{
    OpenChange, current_window, on_last_window_closed, open_window, quit, set_quit_mode,
};
use nsh_core::element::{create_root, next_frame};

use nsh_core::JsonValue;
use nsh_core::prelude::*;

use self::app_menu::ChromeIcon;

use crate::annotate::guides::GuideKind;
use crate::annotate::{Kind, Placing};
use crate::bookmarks::{self, BookmarkAction};
use crate::commands::{self, CommandId};
use crate::full_screen::FullScreen;
use crate::goto::{self, PageState};
use crate::links;
use crate::list_dialog::{list_dialog, style};
use crate::modes::{self, Modes};
use crate::notebooks;
use crate::onboarding::{self, Onboarding};
use crate::outline::{self, OutlineAction};
use crate::panes::{self, FileDrag, Pane, PaneAction, Panes, Place, Shared};
use crate::present_ui;
use crate::print::{Printing, print_strip};
use crate::reading::Reading;
use crate::reading_theme_bar::reading_theme_bar;
use crate::recent::RecentState;
use crate::settings;
use crate::settings_window::SettingsWindow;
use crate::study_ui;
use crate::tabs;
use crate::theme as themes;
use crate::thumbnails;
use crate::workspaces::{self, MAIN, Workspaces};
use ecclesia_features::annotate::lasso;
use ecclesia_features::annotate::shapes::Kind as ShapeKind;
use ecclesia_features::modes::Mode;
use ecclesia_features::present::{ActivePointerTool, ScreenBlank, WhiteboardBackground};

/// Width of the Vivaldi-style thin vertical left icon rail.
const LEFT_RAIL_WIDTH: f32 = 38.0;

/// Default width of the expandable left sidebar drawer.
const LEFT_DRAWER_DEFAULT_WIDTH: f32 = 260.0;

/// Minimum user-resized width of the expandable left sidebar drawer.
const LEFT_DRAWER_MIN_WIDTH: f32 = 180.0;

/// Maximum user-resized width of the expandable left sidebar drawer.
const LEFT_DRAWER_MAX_WIDTH: f32 = 480.0;

/// Width of the docked right vertical sidebar.
const RIGHT_SIDEBAR_WIDTH: f32 = 290.0;

/// A workspace window's size before the platform keeps one of its own (W.8).
const WINDOW_SIZE: (f32, f32) = (1100.0, 800.0);

/// The main window's size the first time it opens, in points; after that it opens where it was left.
const MAIN_WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

/// Minimum right-side title-bar padding reserved on Windows for the native caption buttons (`−`, `□`, `×`).
const WINDOWS_CAPTION_MIN_RIGHT_PX: f32 = 142.0;

/// How often the system's high contrast is looked at (U.4).
const SYSTEM_CONTRAST_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Side panels shown in the left Vivaldi icon rail drawer or right inspector sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    /// The pages, as thumbnails (R.5).
    Pages,
    /// The document's table of contents (§11.14).
    Outline,
    /// The reader's own bookmarks in the document (B.6).
    Bookmarks,
    /// What the find bar found, with the words round each hit (F.1).
    Results,
    /// Active selection properties and comment editor.
    Inspector,
    /// Every annotation of the document, filtered and searched (A.14).
    Annotations,
}

impl Panel {
    /// Pinned panels on the thin vertical left icon rail, top to bottom.
    const LEFT_RAIL: [Self; 5] = [
        Self::Bookmarks,
        Self::Pages,
        Self::Outline,
        Self::Annotations,
        Self::Results,
    ];

    /// Tabs offered in the right vertical sidebar header.
    const RIGHT_TABS: [Self; 3] = [Self::Inspector, Self::Annotations, Self::Results];

    /// Its rail / header label.
    fn title(self) -> String {
        match self {
            Self::Pages => t!("panel.pages"),
            Self::Outline => t!("panel.outline"),
            Self::Bookmarks => t!("panel.bookmarks"),
            Self::Results => t!("panel.results"),
            Self::Inspector => t!("panel.inspector"),
            Self::Annotations => t!("panel.annotations"),
        }
    }

    /// The monochrome chrome icon representing this panel on the thin left rail.
    fn rail_icon(self) -> ChromeIcon {
        match self {
            Self::Bookmarks => ChromeIcon::RailBookmarks,
            Self::Pages => ChromeIcon::RailPages,
            Self::Outline => ChromeIcon::RailOutline,
            Self::Annotations | Self::Inspector => ChromeIcon::RailAnnotations,
            Self::Results => ChromeIcon::RailSearch,
        }
    }

    /// The heading of its empty state.
    fn heading(self) -> String {
        self.title()
    }

    /// The line under the heading: the one next step it teaches (plan §10.3).
    fn hint(self) -> String {
        match self {
            Self::Pages => t!("pages.empty"),
            Self::Outline => t!("outline.empty"),
            Self::Bookmarks => t!("bookmark.empty"),
            Self::Results => t!("find.no_document"),
            Self::Inspector => t!("inspector.empty"),
            Self::Annotations => t!("annotations.no_document"),
        }
    }
}

/// Reactive state controlling the Vivaldi-style left icon rail, expandable drawer, and right vertical sidebar.
#[derive(Clone, Copy)]
struct SidebarState {
    /// Whether the left icon rail (and any expanded drawer) is visible.
    left_rail_visible: Signal<bool>,
    /// Currently expanded panel in the wide left drawer, or `None` for rail-only mode.
    left_expanded: Signal<Option<Panel>>,
    /// Current width of the expanded left drawer in points.
    left_drawer_width: Signal<f32>,
    /// Active drag state `(start_page_x, start_width)` while resizing the left drawer.
    left_drag_start: Signal<Option<(f32, f32)>>,
    /// Whether the right vertical sidebar is open.
    right_open: Signal<bool>,
    /// Active panel tab shown in the right vertical sidebar.
    right_panel: Signal<Panel>,
    /// Whether keyboard focus is currently inside the expanded left drawer (e.g. Outline/Bookmarks list).
    left_drawer_focused: Signal<bool>,
    /// Whether keyboard focus is currently inside the right vertical sidebar.
    right_sidebar_focused: Signal<bool>,
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
    /// Number of open shell windows (main + workspace windows); quitting runs when the last closes.
    open_windows: Stored<usize>,
    /// Standalone Settings window handle owned by `"main"`, closed when `"main"` closes.
    main_settings: Stored<Option<SettingsWindow>>,
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
    set_quit_mode(QuitMode::LastWindowClosed);
    on_last_window_closed(quit);
    let live = settings::Live::new();
    let app = App {
        live,
        recent: RecentState::new(),
        workspace_list: workspaces::shared_list(),
        theme_choice: signal(themes::saved()),
        open_windows: stored(1),
        main_settings: stored(None),
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
    // W.8: the windows open when the app last quit, once the main window is up.
    spawn(async move {
        next_frame().await;
        for id in workspaces::reopened(shown) {
            open_workspace_window(app, id);
        }
    });
    let main_closed = Cell::new(false);
    // NSH 0.7's window model: the app host never shows, so the main window is declared like the others, and opens
    // where the reader left it.
    nsh_core::element::desktop::window()
        .id("main")
        .title("Ecclesia PDF".to_owned())
        .default_size(size(px(MAIN_WINDOW_SIZE.0), px(MAIN_WINDOW_SIZE.1)))
        .restore_bounds(true)
        .on_open_change(move |event: &OpenChange| {
            if !event.open && !main_closed.replace(true) {
                panes.windows().close_all();
                if let Some(settings_win) = app.main_settings.try_with(Option::take).flatten() {
                    settings_win.close();
                }
                for pane in panes.slots() {
                    pane.editing.save_now();
                }
                let remaining = app
                    .open_windows
                    .try_with(|count| {
                        *count = count.saturating_sub(1);
                        *count
                    })
                    .unwrap_or(0);
                if remaining == 0 {
                    quit();
                }
            }
        })
        .content(move || {
            workspaces.set_window(current_window());
            window_shell(app, panes, workspaces, Role::Main)
        })
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
    let closed = Cell::new(false);
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
            .on_open_change(move |event: &OpenChange| {
                if !event.open && !closed.replace(true) {
                    workspaces.close();
                    let remaining = app
                        .open_windows
                        .try_with(|count| {
                            *count = count.saturating_sub(1);
                            *count
                        })
                        .unwrap_or(0);
                    if remaining == 0 {
                        quit();
                    }
                }
            })
            .content(move || {
                cleanup(move || owner.dispose());
                window_shell(app, panes, workspaces, Role::Workspace)
            }),
    );
    match opened {
        Some(window) => {
            let _ = app.open_windows.try_with(|count| *count += 1);
            workspaces.set_window(window);
        }
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
    let layout = signal(DockAreaLayout::new(DockLayout::tabs().panel("document")));
    let sidebars = SidebarState {
        left_rail_visible: signal(true),
        left_expanded: signal(None),
        left_drawer_width: signal(LEFT_DRAWER_DEFAULT_WIDTH),
        left_drag_start: signal(None),
        right_open: signal(false),
        right_panel: signal(Panel::Inspector),
        left_drawer_focused: signal(false),
        right_sidebar_focused: signal(false),
    };
    let palette_open = signal(false);
    let theme_open = signal(false);
    let appearance_open = signal(false);
    let goto_open = signal(false);
    let custom_zoom_open = signal(false);
    let rename_open = signal(false);
    let toolbar_visible = signal(true);
    let settings_win = SettingsWindow::new();
    if role == Role::Main {
        let _ = app
            .main_settings
            .try_with(|slot| *slot = Some(settings_win));
    }
    cleanup(move || settings_win.close());
    // Not a dialog: it lies in the title bar, so the pages stay in sight to preview each theme.
    let reading_theme_open = signal(false);
    let tab_list_open = signal(false);
    let app_menu_open = signal(false);
    let lib_ui = library_ui::LibraryUiState::new();
    let ai_ui = ai_ui::AiUiState::new();
    let forms_sign_ui = forms_sign_ui::FormsSignUiState::new();
    let pro_pdf_ui = pro_pdf_ui::ProPdfUiState::new();
    let ecosystem_ui = ecosystem_ui::EcosystemUiState::new();
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
            onboarding.moment(ecclesia_features::onboarding::Tip::TabList);
        } else if tabs.has_shown() {
            onboarding.moment(ecclesia_features::onboarding::Tip::Palette);
        }
    });
    // The page canvases are windows over the app's, so they hide while a dialog is open.
    // Box behind a `Stored<Rc<...>>` handle so every downstream `Copy` closure captures 8 bytes
    // instead of duplicating all 16 modal-state handles across the call stack.
    let dialog_open_fn: Stored<std::rc::Rc<dyn Fn() -> bool>> =
        stored(std::rc::Rc::new(move || {
            palette_open.get()
                || theme_open.get()
                || appearance_open.get()
                || goto_open.get()
                || custom_zoom_open.get()
                || rename_open.get()
                || settings_win.is_modal_open()
                || tab_list_open.get()
                || app_menu_open.get()
                || onboarding.is_open()
                || modes.is_customizing()
                || workspaces.dialog().is_some()
                || lib_ui.any_dialog_open()
                || ai_ui.any_dialog_open()
                || forms_sign_ui.any_dialog_open()
                || pro_pdf_ui.any_dialog_open()
                || ecosystem_ui.any_dialog_open()
                || panes.slots().into_iter().any(Pane::covers_pages)
                || panes.sync_menu_open()
        }));
    let dialog_open = move || {
        dialog_open_fn
            .try_with(|f| std::rc::Rc::clone(f))
            .is_some_and(|f| f())
    };
    // Clear stale sidebar focus flags whenever a drawer/sidebar hides or collapses, or when any
    // document pane takes keyboard focus back.
    effect(move || {
        if !sidebars.left_rail_visible.get()
            || sidebars.left_expanded.with(Option::is_none)
            || full_screen.is_on()
        {
            sidebars.left_drawer_focused.set(false);
        }
    });
    effect(move || {
        if !sidebars.right_open.get() || full_screen.is_on() {
            sidebars.right_sidebar_focused.set(false);
        }
    });
    for pane in panes.slots() {
        pane.focus.on_focus_in(move || {
            sidebars.left_drawer_focused.set(false);
            sidebars.right_sidebar_focused.set(false);
        });
    }
    let sidebar_focused =
        move || sidebars.left_drawer_focused.get() || sidebars.right_sidebar_focused.get();
    // Move keyboard focus onto the active pane's pages whenever a document tab is shown (switching
    // tabs, leaving Home, or closing a dialog/popover), deferred one tick so it lands after the
    // clicked chrome button or closing modal settles.
    effect(move || {
        let pane = panes.slots()[panes.active_cell()];
        let shown = pane.tabs.shown_path().is_some() && !pane.home_open.get();
        let blocked = dialog_open() || reading_theme_open.get() || pane.find.is_open();
        if shown && !blocked && !untracked(sidebar_focused) {
            let focus = pane.focus;
            drop(set_timeout(std::time::Duration::ZERO, move || {
                if !untracked(dialog_open)
                    && !untracked(|| reading_theme_open.get())
                    && !untracked(sidebar_focused)
                    && !untracked(|| pane.find.is_open())
                {
                    focus.focus();
                }
            }));
        }
    });
    // Reactive page-shortcut mode: mounts page bindings only while a document is shown outside
    // dialogs, popovers, and focused sidebar lists (or Escape-only while full screen on Home).
    let page_key_state = computed(move || {
        if dialog_open() || reading_theme_open.get() {
            return commands::PageKeyState::Inactive;
        }
        let pane = panes.slots()[panes.active_cell()];
        let showing_doc = pane.tabs.has_shown() && !pane.home_open.get();
        if showing_doc && !sidebar_focused() {
            commands::PageKeyState::Active
        } else if full_screen.is_on() {
            commands::PageKeyState::FullScreenEscapeOnly
        } else {
            commands::PageKeyState::Inactive
        }
    });
    let dispatch = move |id: CommandId| {
        let pane = panes.active();
        let to_canvas = move |name: &str| pane.canvas.command(name, JsonValue::Null);
        match id {
            CommandId::OpenPalette => palette_open.set(true),
            CommandId::OpenSettings => {}
            CommandId::ReplayGuide => onboarding.replay(),
            CommandId::Mode(mode) => {
                let prev = untracked(|| modes.current());
                modes.set(mode);
                if mode == Mode::Present && prev != Mode::Present {
                    pane.present.start(pane.pages.count(), pane.pages.current());
                } else if prev == Mode::Present && mode != Mode::Present {
                    pane.present.end();
                }
            }
            CommandId::StepMode(back) => {
                let prev = untracked(|| modes.current());
                modes.step(back);
                let next = untracked(|| modes.current());
                if next == Mode::Present && prev != Mode::Present {
                    pane.present.start(pane.pages.count(), pane.pages.current());
                } else if prev == Mode::Present && next != Mode::Present {
                    pane.present.end();
                }
            }
            CommandId::CustomizeToolbar => modes.customize(),
            CommandId::ExportToolbars => modes.export(),
            CommandId::ImportToolbars => modes.import(),
            CommandId::ChooseTheme => theme_open.set(true),
            CommandId::ChooseAppearance => appearance_open.set(true),
            CommandId::ToggleSidebar => {
                sidebars
                    .left_rail_visible
                    .update(|visible| *visible = !*visible);
            }
            CommandId::ToggleToolbar => {
                toolbar_visible.update(|visible| *visible = !*visible);
            }
            CommandId::ToggleMenuBar => {
                app_menu_open.update(|open| *open = !*open);
            }
            CommandId::ToggleOutline => {
                sidebars.left_rail_visible.set(true);
                sidebars.left_expanded.update(|current| {
                    if *current == Some(Panel::Outline) {
                        *current = None;
                    } else {
                        *current = Some(Panel::Outline);
                    }
                });
            }
            CommandId::ToggleInspector => {
                sidebars.right_open.update(|open| *open = !*open);
            }
            CommandId::GoToPage => {
                if untracked(|| pane.tabs.has_shown()) {
                    goto_open.set(true);
                }
            }
            CommandId::CustomZoom => {
                if untracked(|| pane.tabs.has_shown()) {
                    custom_zoom_open.set(true);
                }
            }
            CommandId::ToggleFullScreen => full_screen.toggle(),
            CommandId::ToggleRulerInvert => reading.toggle_invert(),
            CommandId::ToggleDocumentColors => reading.toggle_document_colors(),
            CommandId::ChooseReadingTheme => reading_theme_open.set(true),
            CommandId::OpenFile => tabs::pick_documents(move |paths| {
                workspaces.open_paths(paths);
                onboarding.make_way();
                panes.active().home_open.set(false);
            }),
            CommandId::OpenNextInFolder => {
                if let Some(path) = pane.tabs.adjacent_in_folder(crate::tabs::Step::Next) {
                    workspaces.open_paths(vec![path]);
                    panes.active().home_open.set(false);
                }
            }
            CommandId::OpenPrevInFolder => {
                if let Some(path) = pane.tabs.adjacent_in_folder(crate::tabs::Step::Previous) {
                    workspaces.open_paths(vec![path]);
                    panes.active().home_open.set(false);
                }
            }
            CommandId::RenameFile => {
                if untracked(|| pane.tabs.has_shown() && !pane.find.is_open()) {
                    rename_open.set(true);
                }
            }
            CommandId::SaveAs => pane.editing.save_as_copy(),
            CommandId::ReloadDocument => {
                if let Some(path) = untracked(|| pane.tabs.shown_path()) {
                    let resolved = crate::expdf::resolve_open_path(&path);
                    match ecclesia_core::shared::registry().reload(&resolved) {
                        Ok(_) => {
                            pane.editing.on_external_reload(&resolved);
                            to_canvas("reload");
                        }
                        Err(err) => {
                            log::warn!("reload: cannot reload {}: {err}", resolved.display());
                            to_canvas("reload");
                        }
                    }
                }
            }
            CommandId::DocumentProperties => {
                if untracked(|| pane.tabs.has_shown()) {
                    pane.tabs.open_properties_shown();
                }
            }
            CommandId::NewWindow => workspaces.new_empty_window(),
            CommandId::TabList => tab_list_open.set(true),
            CommandId::SetDefaultViewer => crate::open::set_as_default_viewer(),
            CommandId::Tab(action) => pane.tabs.run(action),
            // V.11: the window's pages need the shared state, which the shell holds.
            CommandId::Pane(PaneAction::PopOut) => crate::windows::pop_out(panes, shared),
            CommandId::Pane(action) => panes.run(action),
            CommandId::Workspace(action) => workspaces.run(action),
            CommandId::LeaveModes => {
                if untracked(|| pane.find.is_open()) {
                    pane.find.run(crate::find::FindAction::Close);
                    return;
                }
                pane.annotate.clear_selection();
                pane.annotate.clear_block_select();
                to_canvas("leaveModes");
                if untracked(|| modes.current()) == Mode::Present {
                    modes.set(Mode::Read);
                    pane.present.end();
                }
                full_screen.leave();
            }
            // A.13: with annotations selected, copy and the arrow keys are theirs. In Present mode,
            // navigation keys first step the curtain reveal or slide deck before scrolling the canvas.
            CommandId::Canvas(name) => {
                if untracked(|| modes.current()) == Mode::Present
                    && !untracked(dialog_open)
                    && let Some(key_name) = present_nav_key(name)
                    && pane.present.handle_key(
                        key_name,
                        move |page| pane.go_to(page),
                        move || {
                            modes.set(Mode::Read);
                            pane.present.end();
                        },
                    )
                {
                    return;
                }
                if !pane.annotate.selection_command(name) {
                    to_canvas(name);
                }
            }
            CommandId::CutAnnotations => pane.annotate.cut(),
            CommandId::PasteAnnotations => pane.annotate.paste(pane.pages.current()),
            CommandId::DeleteAnnotations => pane.annotate.delete_selected(),
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
            CommandId::Callout => pane.annotate.place(Placing::Text(Kind::Callout)),
            CommandId::Line => pane.annotate.place(Placing::Shape(ShapeKind::Line)),
            CommandId::Arrow => pane.annotate.place(Placing::Shape(ShapeKind::Arrow)),
            CommandId::Rectangle => pane.annotate.place(Placing::Shape(ShapeKind::Rectangle)),
            CommandId::Ellipse => pane.annotate.place(Placing::Shape(ShapeKind::Ellipse)),
            CommandId::Polygon => pane.annotate.place(Placing::Shape(ShapeKind::Polygon)),
            CommandId::PolyLine => pane.annotate.place(Placing::Shape(ShapeKind::PolyLine)),
            CommandId::Cloud => pane.annotate.place(Placing::Shape(ShapeKind::Cloud)),
            CommandId::RegularPolygon => pane
                .annotate
                .place(Placing::Shape(pane.annotate.regular_polygon())),
            CommandId::Star => pane.annotate.place(Placing::Shape(pane.annotate.star())),
            CommandId::Stamp => pane.annotate.place(Placing::Stamp),
            CommandId::Pen(kind) => pane.annotate.choose_pen(kind),
            CommandId::Eraser(eraser) => pane.annotate.choose_eraser(eraser),
            CommandId::Lasso => pane.annotate.toggle_lasso(),
            CommandId::BlockSelect => pane.annotate.toggle_block_select(),
            CommandId::Tape => pane.annotate.place(Placing::Tape),
            CommandId::ZoomPad => pane.annotate.toggle_zoom_pad(),
            CommandId::Ruler => pane.annotate.toggle_guide(GuideKind::Ruler),
            CommandId::Protractor => pane.annotate.toggle_guide(GuideKind::Protractor),
            CommandId::Compass => pane.annotate.toggle_guide(GuideKind::Compass),
            CommandId::RotateLeft => pane.annotate.rotate_selection(-90.0),
            CommandId::RotateRight => pane.annotate.rotate_selection(90.0),
            CommandId::Thinner => pane.annotate.scale_selection_width(lasso::THINNER),
            CommandId::Thicker => pane.annotate.scale_selection_width(lasso::THICKER),
            CommandId::DuplicateSelection => pane.annotate.duplicate_selection(),
            CommandId::Snapshot => pane.snapshot.toggle(),
            CommandId::Attachments => pane.attachments.show(),
            CommandId::SavePen => pane.annotate.save_pen(),
            CommandId::EditPen => pane.annotate.edit_pen(),
            CommandId::RemovePen => pane.annotate.remove_pen(),
            CommandId::ExportPens => pane.annotate.export_pens(),
            CommandId::ImportPens => pane.annotate.import_pens(),
            CommandId::MarginNotes => pane.annotate.toggle_margin(),
            CommandId::TransferAnnotations => pane.annotate.transfer(),
            CommandId::ExportAnnotations => pane.annotate.export_annotations(),
            CommandId::ImportAnnotations => pane.annotate.import_annotations(),
            CommandId::ExportFlattened => pane.annotate.export_flattened(),
            // The layers sit at the top of the annotation list (A.22).
            CommandId::AnnotationList | CommandId::Layers => {
                sidebars.left_rail_visible.set(true);
                sidebars.left_expanded.set(Some(Panel::Annotations));
            }
            CommandId::LayersByAuthor => pane.annotate.layers_by_author(),
            CommandId::AnnotationColors => pane
                .annotate
                .pick(ecclesia_features::annotate::colors::Tool::Highlight),
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
            CommandId::StraightenInk => pane.annotate.straighten_ink_selection(),
            CommandId::RestyleInk => pane.annotate.open_restyle(),
            CommandId::InkFill => pane.annotate.open_fill(),
            CommandId::Eyedropper => pane.annotate.toggle_eyedropper(),
            CommandId::ColorPalettes => pane.annotate.open_palettes(),
            CommandId::InkReplay => pane.annotate.open_replay(),
            CommandId::VectorizeInk => pane.annotate.vectorize_shapes(),
            CommandId::NewNotebook => pane.notebooks.open_new_notebook(),
            CommandId::InsertPage => pane.notebooks.open_insert_page(),
            CommandId::ExpandMargins => pane.notebooks.open_margins(),
            CommandId::ExportNotebookPdf => pane.notebooks.export_notebook_pdf(),
            CommandId::InfiniteCanvas => pane.notebooks.open_infinite_canvas(),
            CommandId::InsertTextBlock => pane.notebooks.open_text_block(),
            CommandId::InsertImage => pane.notebooks.insert_image_file(),
            CommandId::NotebookSections => pane.notebooks.open_sections(),
            CommandId::NotebookLinks => pane.notebooks.open_links_manager(),
            CommandId::QuickNote => {
                pane.notebooks.trigger_quick_note(move |path| {
                    workspaces.open_paths(vec![path]);
                });
            }
            CommandId::MarkdownNotes => pane.notebooks.open_markdown(),
            CommandId::LectureAudio => pane.notebooks.open_audio(),
            CommandId::CondenseSlides => pane.notebooks.open_condense_slides(),
            CommandId::PresentLaser => pane.present.select_pointer_tool(ActivePointerTool::Laser),
            CommandId::PresentSpotlight => {
                pane.present
                    .select_pointer_tool(ActivePointerTool::Spotlight);
            }
            CommandId::PresentMagnifier => {
                pane.present
                    .select_pointer_tool(ActivePointerTool::Magnifier);
            }
            CommandId::PresentBlankBlack => pane.present.toggle_blank(ScreenBlank::Black),
            CommandId::PresentBlankWhite => pane.present.toggle_blank(ScreenBlank::White),
            CommandId::PresentFadingInk => pane.present.toggle_ink_persistence(),
            CommandId::PresentWhiteboard => {
                pane.present.toggle_whiteboard(WhiteboardBackground::Blank);
            }
            CommandId::PresentCurtain => pane.present.toggle_curtain(),
            CommandId::PresenterView => pane.present.open_presenter_view(),
            CommandId::MakeFlashcard => pane.study.open_flashcard(),
            CommandId::ImageOcclusion => pane.study.open_occlusion(),
            CommandId::MakeCloze => pane.study.open_cloze(),
            CommandId::DailyReview => pane.study.open_review(),
            CommandId::Dictionary => pane.study.open_dictionary(),
            CommandId::Vocabulary => pane.study.open_vocabulary(),
            CommandId::StudyExport => pane.study.open_export(),
            CommandId::ExamMode => pane.study.toggle_exam_mode(),
            CommandId::ImportAnki => pane.study.import_apkg_dialog(),
            CommandId::ExportAnki => pane.study.export_apkg_dialog(),
            CommandId::StudyStatistics => pane.study.open_stats(),
            CommandId::ReadAloud => pane.study.toggle_read_aloud(),
            CommandId::OpenLibrary => lib_ui.open_library(),
            CommandId::FindDuplicates => lib_ui.open_duplicates(),
            CommandId::Citations => lib_ui.open_citations(),
            CommandId::LibraryNotes => lib_ui.open_notes(),
            CommandId::WorkspaceRules => lib_ui.open_rules(),
            CommandId::CliMcpDocs => lib_ui.open_cli_docs(),
            CommandId::ReflowMode => pane.reading_extra.toggle_reflow(),
            CommandId::BionicReading => pane.reading_extra.toggle_bionic(),
            CommandId::TwoColumnScroll => {
                pane.reading_extra.step_two_column_scroll(
                    pane.tabs.shown_path(),
                    pane.pages.current(),
                    move |p| pane.go_to(p),
                );
            }
            CommandId::SyncTex => pane.reading_extra.open_synctex(),
            CommandId::ExtraFormats => {
                if let Some(p) = pane.tabs.shown_path()
                    && reading_extra::is_extra_non_mupdf_container(&p)
                {
                    pane.reading_extra.open_extra_file(p);
                } else {
                    pane.reading_extra
                        .open_extra_file(pane.tabs.shown_path().unwrap_or_default());
                }
            }
            CommandId::OpenUrl => pane.reading_extra.open_remote(),
            CommandId::GenerateOutline => pane.outline_extra.open_generate(),
            CommandId::OutlineIo => pane.outline_extra.open_io(),
            CommandId::AnnotationReport => pane.annotate.open_report(),
            CommandId::AnnotationHistory => pane.annotate.open_history(),
            CommandId::BeautifyInk => pane.annotate.beautify_ink_selection(),
            CommandId::PatternBrush => pane.annotate.open_pattern_brush(),
            CommandId::Symmetry => pane.annotate.open_symmetry(),
            CommandId::HandwritingSearch => pane.annotate.open_hw_search(),
            CommandId::HandwritingToText => pane.annotate.open_hw_convert(),
            CommandId::OcrDocument => pane.annotate.open_ocr(),
            CommandId::Planner => pane.notebooks.open_planner(),
            CommandId::TemplatePacks => pane.notebooks.open_template_packs(),
            CommandId::NotebookVersions => pane.notebooks.open_versions(),
            CommandId::AudioTranscript => pane.notebooks.open_transcript(),
            CommandId::HighlightSummary => pane.study.open_summaries(),
            CommandId::Assignments => pane.study.open_assignments(),
            CommandId::LanguageLearning => pane.study.open_language(),
            CommandId::TranslateSelection => pane.study.open_translate(),
            CommandId::AiPanel => ai_ui.open_panel(ai_ui::AiPanelTab::Chat),
            CommandId::AiSummarize => ai_ui.open_panel(ai_ui::AiPanelTab::Actions),
            CommandId::AiStudyMaterials => ai_ui.open_panel(ai_ui::AiPanelTab::Study),
            CommandId::AiResearchExtract => ai_ui.open_panel(ai_ui::AiPanelTab::Research),
            CommandId::AiQuiz => ai_ui.open_panel(ai_ui::AiPanelTab::Quiz),
            CommandId::MathToLatex => ai_ui.open_math_to_latex(),
            CommandId::VisualSearch => ai_ui.open_visual_search(),
            CommandId::FormsPanel => forms_sign_ui.open_forms(panes),
            CommandId::FormsFlatten => forms_sign_ui.flatten_forms(panes),
            CommandId::SignaturesPanel => forms_sign_ui.open_signatures(panes),
            CommandId::PageOrganizer => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::Organizer, panes);
            }
            CommandId::EditTextAndImages
            | CommandId::TrueRedaction
            | CommandId::ExportAndConvert => {
                let tab = match id {
                    CommandId::TrueRedaction => pro_pdf_ui::EditExportTab::Redact,
                    CommandId::ExportAndConvert => pro_pdf_ui::EditExportTab::Convert,
                    _ => pro_pdf_ui::EditExportTab::Text,
                };
                pro_pdf_ui.open_edit_tab(tab, panes);
            }
            CommandId::CompressPdf | CommandId::SecurityPermissions => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::SecurityCompress, panes);
            }
            CommandId::BatesAndHeaders => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::BatesHeaders, panes);
            }
            CommandId::PrintProduction => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::PreflightCad, panes);
            }
            CommandId::CadView => pro_pdf_ui.toggle_cad_mode(panes, live),
            CommandId::OutlineActions => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::OutlineAction, panes);
            }
            CommandId::MeasureTool => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::Measure, panes);
            }
            CommandId::ComparePdfs | CommandId::ExtractTablesFigures => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::CompareExtract, panes);
            }
            CommandId::ScreencastRecord => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::Screencast, panes);
            }
            CommandId::GradingSuite => {
                pro_pdf_ui.open_dialog(pro_pdf_ui::ProPdfDialog::Grading, panes);
            }
            CommandId::CameraScanImport => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::CameraScan, panes);
            }
            CommandId::ExcerptBoard => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::ExcerptBoard, panes);
            }
            CommandId::KnowledgeGraph => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::KnowledgeGraph, panes);
            }
            CommandId::SyncAndShare => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::SyncAndShare, panes);
            }
            CommandId::LiveRelay => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::LiveRelay, panes);
            }
            CommandId::LiveClassroom => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::Classroom, panes);
            }
            CommandId::PluginsManager => {
                ecosystem_ui.open_dialog(ecosystem_ui::EcoDialog::Plugins, panes);
            }
        }
    };
    let run_fn: Stored<std::rc::Rc<dyn Fn(CommandId)>> =
        stored(std::rc::Rc::new(move |id: CommandId| {
            match id {
                CommandId::OpenSettings => settings_win.open(live, theme_choice, dispatch),
                other => dispatch(other),
            }
            // Return focus from clicked chrome/toolbar controls back to the active PDF pages after
            // direct commands when no dialog, popover, Find bar, or sidebar panel owns the keyboard.
            if !matches!(
                id,
                CommandId::OpenSettings
                    | CommandId::AnnotationList
                    | CommandId::Layers
                    | CommandId::Outline(_)
                    | CommandId::Bookmark(_)
                    | CommandId::Find(_)
            ) {
                let pane = panes.active();
                if untracked(|| pane.tabs.has_shown() && !pane.home_open.get())
                    && !untracked(dialog_open)
                    && !untracked(|| reading_theme_open.get())
                    && !untracked(sidebar_focused)
                    && !untracked(|| pane.find.is_open())
                {
                    let focus = pane.focus;
                    drop(set_timeout(std::time::Duration::ZERO, move || {
                        if !untracked(dialog_open)
                            && !untracked(|| reading_theme_open.get())
                            && !untracked(sidebar_focused)
                            && !untracked(|| pane.find.is_open())
                        {
                            focus.focus();
                        }
                    }));
                }
            }
        }));
    let run = move |id: CommandId| {
        if let Some(f) = run_fn.try_with(|f| std::rc::Rc::clone(f)) {
            f(id);
        }
    };
    effect(move || {
        let pane = panes.active();
        control_channel::update_state(pane.tabs.shown_path(), pane.pages.current());
    });
    spawn(async move {
        next_frame().await;
        control_channel::ensure_started();
        if role == Role::Main && control_channel::status_snapshot().running {
            set_interval(std::time::Duration::from_millis(250), move || {
                for action in control_channel::drain_pending() {
                    match action {
                        control_channel::PendingControlAction::OpenDocument(p) => {
                            workspaces.open_paths(vec![p]);
                        }
                        control_channel::PendingControlAction::JumpToPage(page) => {
                            panes.active().go_to(page);
                        }
                        control_channel::PendingControlAction::RunCommand(name) => {
                            if let Some(cmd) = commands::find(&name) {
                                run(cmd);
                            }
                        }
                    }
                }
            });
        }
    });
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
    // Window-scoped shortcuts rebind when settings change; page-scoped shortcuts mount reactively
    // only while the active document owns plain keys (so dialogs, Home, and sidebars stay clean).
    effect(move || {
        live.read(|_| ());
        commands::bind_window_keys(run);
    });
    effect(move || {
        live.read(|_| ());
        commands::bind_page_keys(page_key_state.get(), run);
    });
    // A note opened on the pages shows in the inspector (A.4).
    for pane in panes.slots() {
        effect(move || {
            if pane.annotate.reveals() > 0 {
                untracked(|| {
                    sidebars.right_open.set(true);
                    sidebars.right_panel.set(Panel::Inspector);
                });
            }
        });
    }
    // Each pane's own dialogs: the group being edited, the bookmark being written, the link being asked about, the
    // text of a proposed edit (A.2), the markup tools' colours (A.3).
    let pane_dialogs = per_pane_dialogs(panes);
    let chrome_dialogs = window_chrome_dialogs(
        ChromeDialogs {
            palette_open,
            tab_list_open,
            settings_win,
            theme_open,
            appearance_open,
            goto_open,
            custom_zoom_open,
            rename_open,
            live,
            theme_choice,
            recent,
            modes,
            panes,
            workspaces,
        },
        run,
    );
    let feature_overlays = feature_dialogs(
        FeatureDialogs {
            ai_ui,
            forms_sign_ui,
            pro_pdf_ui,
            ecosystem_ui,
            onboarding,
            panes,
            workspaces,
            live,
        },
        run,
    );

    let root = view()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        // A right-to-left language the reader chose mirrors the window; with none chosen, the system's decides (U.7).
        .direction(move || live.read(crate::l10n::direction))
        // Documents dropped on the window outside the panes open in the active pane (R.11), or where another
        // workspace has them (W.6); other files are left alone. Every drop target follows the drag, so the panes' edges take drops while it lasts (V.2).
        .on_file_drag_enter(move || file_drag.enter())
        .on_file_drag_leave(move || file_drag.leave())
        .on_file_drop(move |event: &FileDropEvent| {
            file_drag.drop_done();
            let documents = crate::open::dropped_documents(&event.uris);
            if !documents.is_empty() {
                workspaces.open_paths(documents);
                panes.active().home_open.set(false);
            }
        })
        .on_key_down(move |ev: &KeyEvent| {
            if untracked(|| modes.current()) == Mode::Present && !untracked(dialog_open) {
                let pane = panes.active();
                let _ = pane.present.handle_key(
                    &ev.key,
                    move |page| pane.go_to(page),
                    move || run(CommandId::Mode(Mode::Read)),
                );
            }
        })
        .child(window_bar(
            run,
            panes,
            workspaces,
            shared,
            reading_theme_open,
            app_menu_open,
        ))
        .child(when(
            move || toolbar_visible.get(),
            move || {
                modes::mode_bar(
                    modes,
                    move || panes.active().pages.count() > 0,
                    move || full_screen.is_on(),
                    (sidebars.left_rail_visible, sidebars.right_open),
                    move |id| {
                        let pane = panes.slots()[panes.active_cell()];
                        pane.annotate.is_on(id)
                            || pane.present.is_on(id)
                            || pane.study.is_on(id)
                            || (id == CommandId::ReflowMode && pane.reading_extra.is_reflow_open())
                            || (id == CommandId::BionicReading && pane.reading_extra.is_bionic_on())
                            || (id == CommandId::CadView && pro_pdf_ui.is_cad_active(live))
                            || (id == CommandId::MeasureTool && pro_pdf_ui.is_measure_active())
                            || (id == CommandId::ScreencastRecord
                                && pro_pdf_ui.is_screencast_active())
                            || id == CommandId::Snapshot && pane.snapshot.is_on()
                    },
                    run,
                    // H.7: the favourite pens lead the Write mode's toolbar.
                    move || {
                        crate::annotate::favorites::favorites_bar(
                            panes.slots()[panes.active_cell()].annotate,
                        )
                        .into_element()
                    },
                )
            },
        ))
        .child(workspace_body(sidebars, dialog_open, panes, shared, run))
        // O.22: a print job's progress, below the pages (they are a window over the app's).
        .child(print_strip(printing))
        // U.9: a tip, once, below the pages.
        .child(onboarding::tip_strip(onboarding))
        .child(chrome_dialogs)
        .child(pane_dialogs)
        .child(active_pane_dialogs(panes, workspaces))
        .child(library_dialogs(lib_ui, panes, workspaces))
        .child(feature_overlays);
    #[cfg(target_os = "windows")]
    let root = root.child(windows_bridge_element(
        move || page_key_state.get(),
        panes,
        workspaces,
        run,
    ));
    root
}

/// Per-pane modal dialogs (groups, properties, bookmarks, links, annotation editors), split out
/// of [`window_shell`] so debug-mode stack frames stay bounded.
#[inline(never)]
fn per_pane_dialogs(panes: Panes) -> View {
    panes.slots().into_iter().fold(view(), |mut dialogs, pane| {
        dialogs = dialogs.child(when(
            move || pane.tabs.editing().is_some(),
            move || tabs::group_dialog(pane.tabs),
        ));
        dialogs = dialogs.child(when(
            move || pane.tabs.properties_tab().is_some(),
            move || tabs::properties_dialog(pane.tabs),
        ));
        dialogs = dialogs.child(when(
            move || pane.bookmarks.is_editing(),
            move || bookmarks::bookmark_dialog(pane.bookmarks),
        ));
        dialogs = dialogs.child(when(
            move || pane.link_state.is_asking(),
            move || links::link_dialog(pane.link_state),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_asking(),
            move || crate::annotate::text_edit_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_picking(),
            move || crate::annotate::color_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_restyling(),
            move || crate::annotate::ink_tools::restyle_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_filling(),
            move || crate::annotate::ink_tools::fill_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_palettes(),
            move || crate::annotate::ink_tools::palettes_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_replaying(),
            move || crate::annotate::ink_tools::replay_dialog(pane.annotate),
        ));
        dialogs
    })
}

/// State bundle for [`window_chrome_dialogs`].
struct ChromeDialogs {
    palette_open: Signal<bool>,
    tab_list_open: Signal<bool>,
    settings_win: SettingsWindow,
    theme_open: Signal<bool>,
    appearance_open: Signal<bool>,
    goto_open: Signal<bool>,
    custom_zoom_open: Signal<bool>,
    rename_open: Signal<bool>,
    live: settings::Live,
    theme_choice: Signal<themes::ThemeChoice>,
    recent: RecentState,
    modes: Modes,
    panes: Panes,
    workspaces: Workspaces,
}

/// Window-chrome modal dialogs (palette, tab list, workspaces, settings fallback, pickers, go-to,
/// custom zoom, rename file, customize toolbar), split out of [`window_shell`] to keep debug-mode
/// stack frames bounded.
#[inline(never)]
fn window_chrome_dialogs(args: ChromeDialogs, run: impl Fn(CommandId) + Copy + 'static) -> View {
    let ChromeDialogs {
        palette_open,
        tab_list_open,
        settings_win,
        theme_open,
        appearance_open,
        goto_open,
        custom_zoom_open,
        rename_open,
        live,
        theme_choice,
        recent,
        modes,
        panes,
        workspaces,
    } = args;
    let mut dialogs = view();
    dialogs = dialogs.child(when(palette_open, move || {
        palette(palette_open, panes.active().pages, run)
    }));
    dialogs = dialogs.child(when(tab_list_open, move || {
        tabs::tab_list(tab_list_open, panes.active().tabs, recent, workspaces)
    }));
    dialogs = dialogs.child(workspaces::workspace_dialogs(workspaces, run));
    dialogs = dialogs.child(when(
        move || settings_win.is_modal_open(),
        move || settings_win.fallback_dialog(live, theme_choice, run),
    ));
    dialogs = dialogs.child(when(theme_open, move || {
        themes::theme_picker(theme_open, theme_choice)
    }));
    dialogs = dialogs.child(when(appearance_open, move || {
        themes::appearance_picker(appearance_open, theme_choice)
    }));
    dialogs = dialogs.child(when(goto_open, move || {
        let pane = panes.active();
        goto::goto_dialog(goto_open, pane.pages, move |index| pane.go_to(index))
    }));
    dialogs = dialogs.child(when(custom_zoom_open, move || {
        let pane = panes.active();
        goto::custom_zoom_dialog(custom_zoom_open, move |pct| {
            pane.canvas
                .command("setZoom", serde_json::json!({ "percent": pct }));
        })
    }));
    dialogs = dialogs.child(when(rename_open, move || {
        let pane = panes.active();
        goto::rename_dialog(rename_open, pane.tabs)
    }));
    dialogs = dialogs.child(modes::customize_dialog(modes));
    dialogs
}

/// State bundle for [`feature_dialogs`].
struct FeatureDialogs {
    ai_ui: ai_ui::AiUiState,
    forms_sign_ui: forms_sign_ui::FormsSignUiState,
    pro_pdf_ui: pro_pdf_ui::ProPdfUiState,
    ecosystem_ui: ecosystem_ui::EcosystemUiState,
    onboarding: Onboarding,
    panes: Panes,
    workspaces: Workspaces,
    live: settings::Live,
}

/// Feature dialogs (AI, forms/signatures, pro PDF, ecosystem, onboarding guide), split out of
/// [`window_shell`] to keep debug-mode stack frames bounded.
#[inline(never)]
fn feature_dialogs(args: FeatureDialogs, run: impl Fn(CommandId) + Copy + 'static) -> View {
    let FeatureDialogs {
        ai_ui,
        forms_sign_ui,
        pro_pdf_ui,
        ecosystem_ui,
        onboarding,
        panes,
        workspaces,
        live,
    } = args;
    let mut dialogs = view();
    dialogs = dialogs.child(ai_ui::ai_dialogs(ai_ui, panes, live));
    dialogs = dialogs.child(forms_sign_ui::forms_sign_dialogs(
        forms_sign_ui,
        panes,
        workspaces,
        live,
    ));
    dialogs = dialogs.child(pro_pdf_ui::pro_pdf_dialogs(
        pro_pdf_ui, panes, workspaces, live,
    ));
    dialogs = dialogs.child(ecosystem_ui::ecosystem_dialogs(
        ecosystem_ui,
        panes,
        workspaces,
        live,
    ));
    dialogs = dialogs.child(onboarding::guide_dialog(onboarding, run));
    dialogs
}

/// Hidden Windows-only native bridge hosting Explorer file drops and OS-level shortcut forwarding.
#[cfg(target_os = "windows")]
#[inline(never)]
fn windows_bridge_element(
    page_key_state: impl Fn() -> commands::PageKeyState + Copy + 'static,
    panes: Panes,
    workspaces: Workspaces,
    run: impl Fn(CommandId) + Copy + 'static,
) -> AnyElement {
    native(crate::WINDOW_BRIDGE)
        .w(px(0.0))
        .h(px(0.0))
        .pointer_events(PointerEvents::None)
        .prop("pageKeysActive", move || {
            let active = page_key_state() == commands::PageKeyState::Active
                && !panes.slots()[panes.active_cell()].find.is_open();
            JsonValue::from(active)
        })
        .on_native_event(move |event: &NativeEvent| match &*event.name {
            "osFileDrop" => {
                let paths = crate::open::paths_from_event(&event.data);
                let documents = crate::open::dropped_paths(&paths);
                if !documents.is_empty() {
                    workspaces.open_paths(documents);
                    panes.active().home_open.set(false);
                }
            }
            "osShortcut" => {
                if let Some(shortcut) = commands::OsShortcut::from_event(&event.data) {
                    match shortcut {
                        commands::OsShortcut::Quit => {
                            for pane in panes.slots() {
                                pane.editing.save_now();
                            }
                            quit();
                        }
                        other if other.is_page_scoped() => {
                            if untracked(page_key_state) == commands::PageKeyState::Active
                                && !untracked(|| panes.active().find.is_open())
                                && let Some(id) = other.command()
                            {
                                run(id);
                            }
                        }
                        other => {
                            if let Some(id) = other.command() {
                                run(id);
                            }
                        }
                    }
                }
            }
            _ => {}
        })
        .into_element()
}

/// Active-pane modal dialogs (notebooks, study, outline, reading extras, and phase-5 annotation tools),
/// split out of [`window_shell`] so debug-mode stack frames stay small.
#[inline(never)]
fn active_pane_dialogs(panes: Panes, workspaces: Workspaces) -> View {
    let mut group = view();
    group = group.child(of_active(panes, |pane| {
        crate::annotate::favorites::pen_dialog(pane.annotate)
    }));
    group = group.child(of_active(panes, |pane| {
        crate::snapshot::snapshot_dialog(pane.snapshot)
    }));
    group = group.child(of_active(panes, |pane| {
        crate::attachments::attachments_dialog(pane.attachments)
    }));
    group = group.child(of_active(panes, move |pane| {
        let mut dialogs = view();
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_new_open(),
            move || {
                notebooks::new_notebook_dialog(pane.notebooks, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_insert_page_open(),
            move || {
                notebooks::insert_page_dialog(pane.notebooks, move |path| {
                    let page = pane.pages.current();
                    let _ = pane.tabs.take_shown();
                    pane.tabs.open_at(path, page);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_margins_open(),
            move || {
                notebooks::margins_dialog(pane.notebooks, move |path| {
                    let page = pane.pages.current();
                    let _ = pane.tabs.take_shown();
                    pane.tabs.open_at(path, page);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_infinite_open(),
            move || notebooks::infinite_canvas_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_text_block_open(),
            move || notebooks::text_block_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_sections_open(),
            move || {
                notebooks::sections_dialog(pane.notebooks, move |path| {
                    let page = pane.pages.current();
                    let _ = pane.tabs.take_shown();
                    pane.tabs.open_at(path, page);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_links_open(),
            move || {
                notebooks::links_manager_dialog(pane.notebooks, move |path| {
                    let page = pane.pages.current();
                    let _ = pane.tabs.take_shown();
                    pane.tabs.open_at(path, page);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_markdown_open(),
            move || notebooks::markdown_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_audio_open(),
            move || notebooks::audio_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_condense_open(),
            move || {
                notebooks::condense_slides_dialog(pane.notebooks, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.present.is_keep_prompt_open(),
            move || present_ui::keep_ink_dialog(pane.present),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_flashcard_open(),
            move || study_ui::flashcard_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_occlusion_open(),
            move || study_ui::occlusion_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_cloze_open(),
            move || study_ui::cloze_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_review_open(),
            move || {
                study_ui::daily_review_dialog(pane.study, move |path, page| {
                    pane.tabs.open_at(path, page);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_dict_open(),
            move || study_ui::dictionary_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_vocab_open(),
            move || study_ui::vocabulary_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_export_open(),
            move || study_ui::study_export_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_stats_open(),
            move || study_ui::statistics_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_summaries_open(),
            move || study_ui::phase5::highlight_summaries_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_assignments_open(),
            move || study_ui::phase5::assignments_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_language_open(),
            move || study_ui::phase5::language_learning_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.study.is_translate_open(),
            move || study_ui::phase5::translate_dialog(pane.study),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_planner_open(),
            move || {
                notebooks::phase5::planner_dialog(pane.notebooks, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_packs_open(),
            move || notebooks::phase5::template_packs_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_versions_open(),
            move || {
                notebooks::phase5::version_history_dialog(pane.notebooks, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.notebooks.is_transcript_open(),
            move || notebooks::phase5::audio_transcript_dialog(pane.notebooks),
        ));
        dialogs = dialogs.child(when(
            move || pane.outline_extra.is_generate_open(),
            move || {
                outline::extra::generate_outline_dialog(pane.outline_extra, pane.tabs.shown_path())
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.outline_extra.is_io_open(),
            move || outline::extra::outline_io_dialog(pane.outline_extra, pane.tabs.shown_path()),
        ));
        dialogs = dialogs.child(when(
            move || pane.reading_extra.is_reflow_open(),
            move || {
                reading_extra::reflow_dialog(
                    pane.reading_extra,
                    pane.tabs.shown_path(),
                    pane.pages.current(),
                )
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.reading_extra.is_synctex_open(),
            move || {
                reading_extra::synctex_dialog(
                    pane.reading_extra,
                    pane.tabs.shown_path(),
                    pane.pages.current(),
                    move |pg| pane.go_to(pg),
                )
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.reading_extra.is_remote_open(),
            move || {
                reading_extra::open_url_dialog(pane.reading_extra, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.reading_extra.is_extra_open(),
            move || reading_extra::extra_format_dialog(pane.reading_extra),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_reporting(),
            move || crate::annotate::phase5::report_export_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_history(),
            move || crate::annotate::phase5::history_timeline_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_pattern(),
            move || crate::annotate::phase5::pattern_brush_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_symmetry(),
            move || crate::annotate::phase5::symmetry_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_hw_searching(),
            move || {
                crate::annotate::phase5::handwriting_search_dialog(pane.annotate, move |pg| {
                    pane.go_to(pg)
                })
            },
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_hw_converting(),
            move || crate::annotate::phase5::handwriting_convert_dialog(pane.annotate),
        ));
        dialogs = dialogs.child(when(
            move || pane.annotate.is_ocr(),
            move || {
                crate::annotate::phase5::ocr_dialog(pane.annotate, move |path| {
                    workspaces.open_paths(vec![path]);
                })
            },
        ));
        dialogs
    }));
    group
}

/// Library, citation, notes, workspace-rules, and CLI/MCP dialogs, split out of [`window_shell`]
/// to keep debug-mode stack frames bounded.
#[inline(never)]
fn library_dialogs(
    lib_ui: library_ui::LibraryUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> View {
    let mut dialogs = view();
    dialogs = dialogs.child(when(
        move || lib_ui.is_library_open(),
        move || {
            let pane = panes.active();
            library_ui::library_dialog(
                lib_ui,
                pane.tabs.shown_path(),
                workspaces.shown_id(),
                move |path, page| {
                    workspaces.open(&crate::open::Launch {
                        files: vec![path],
                        page: Some(page),
                        split: None,
                    });
                },
                move |ws_id| workspaces.switch(ws_id),
            )
        },
    ));
    dialogs = dialogs.child(when(
        move || lib_ui.is_duplicates_open(),
        move || {
            library_ui::duplicates_dialog(lib_ui, move |path, page| {
                workspaces.open(&crate::open::Launch {
                    files: vec![path],
                    page: Some(page),
                    split: None,
                });
            })
        },
    ));
    dialogs = dialogs.child(when(
        move || lib_ui.is_citations_open(),
        move || library_ui::citations_dialog(lib_ui),
    ));
    dialogs = dialogs.child(when(
        move || lib_ui.is_notes_open(),
        move || {
            let pane = panes.active();
            library_ui::notes_backlinks_dialog(lib_ui, pane.tabs.shown_path(), pane.pages.current())
        },
    ));
    dialogs = dialogs.child(when(
        move || lib_ui.is_rules_open(),
        move || {
            library_ui::workspace_rules_dialog(lib_ui, workspaces.shown_id(), move |ws_id| {
                workspaces.switch(ws_id);
            })
        },
    ));
    dialogs = dialogs.child(when(
        move || lib_ui.is_cli_docs_open(),
        move || library_ui::cli_mcp_docs_dialog(lib_ui),
    ));
    dialogs
}

/// Maps page-canvas navigation commands to the DOM key names [`crate::present_ui::PresentState::handle_key`]
/// expects while Presentation mode is active.
fn present_nav_key(command: &str) -> Option<&'static str> {
    match command {
        "lineDown" => Some("ArrowDown"),
        "lineUp" => Some("ArrowUp"),
        "stepLeft" => Some("ArrowLeft"),
        "stepRight" => Some("ArrowRight"),
        "screenDown" => Some("PageDown"),
        "screenUp" => Some("PageUp"),
        _ => None,
    }
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
    app_menu_open: Signal<bool>,
) -> impl IntoElement {
    let Shared {
        full_screen,
        reading,
        ..
    } = shared;
    let live = panes.live();
    let mut bar = TitleBarOptions::default();
    bar.style = TitleBarStyle::Hidden;
    bar.height = Some(px(untracked(|| live.read(tabs::title_bar_height_px))));
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(bar);
    // This window's, the main one or a workspace's (W.8).
    let window = current_window();
    window.set(attributes);
    let resize_window = window.clone();
    let first_height = stored(true);
    effect(move || {
        let height = px(live.read(tabs::title_bar_height_px));
        if first_height.replace(false) {
            return;
        }
        let mut bar = TitleBarOptions::default();
        bar.style = TitleBarStyle::Hidden;
        bar.height = Some(height);
        let mut attributes = WindowAttributes::default();
        attributes.title_bar = Some(bar);
        resize_window.set(attributes);
    });
    let left = window.clone();

    view()
        .w_full()
        .min_w(px(0.0))
        .h(move || px(live.read(tabs::title_bar_height_px)))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .overflow_hidden()
        .window_drag(true)
        .pl(move || left.title_bar().left + px(8.0))
        .pr(move || {
            let min_right = if cfg!(target_os = "windows") {
                WINDOWS_CAPTION_MIN_RIGHT_PX
            } else {
                0.0
            };
            px(window.title_bar().right.0.max(min_right) + 8.0)
        })
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_b_color(move || theme().border)
        .style_when(move || full_screen.hides_bar(), |s| s.hidden())
        .on_mouse_leave(move |_: &MouseEvent| full_screen.bar_left())
        .child(
            view()
                .window_drag(false)
                .flex_shrink(0.0)
                .child(app_menu::app_menu_button(app_menu_open, run, move || {
                    panes.active().tabs.has_shown()
                })),
        )
        .child(workspaces::switcher_button(workspaces, run))
        // The reading-theme bar needs the room while it is open.
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex_row()
                .items_center()
                .overflow_hidden()
                .child(when(
                    move || !reading_theme_open.get(),
                    move || {
                        of_active(panes, move |pane| {
                            tabs::tab_strip(
                                pane.tabs,
                                pane.home_open,
                                pane.editing,
                                workspaces,
                                run,
                            )
                        })
                    },
                )),
        )
        .child(
            view()
                .flex_shrink(1.0)
                .min_w(px(120.0))
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
        .child(of_active(panes, |pane| {
            crate::annotate::guides::reading_line(pane.annotate)
        }))
        .child(view().flex_shrink(0.0).child(of_active(panes, move |pane| {
            goto::page_button(pane.pages, move || run(CommandId::GoToPage))
        })))
        // Compact draggable grip area to the left of the top-right tab-list dropdown.
        .child(view().w(px(12.0)).flex_shrink(1.0).h_full())
        .child(tab_list_button(run))
}

/// Vivaldi-style top-right framed dropdown trigger opening the searchable workspace-grouped tab list.
fn tab_list_button(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    view().window_drag(false).flex_shrink(0.0).child(
        button()
            .accessibility_label(move || {
                Some(SharedString::from(commands::label_with_shortcut(
                    CommandId::TabList,
                )))
            })
            .on_press(move || run(CommandId::TabList))
            .w(px(32.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(move || theme().muted_foreground)
            .bg(move || theme().secondary)
            .items_center()
            .justify_center()
            .hover(|s| {
                s.bg(move || theme().sidebar_accent)
                    .border_color(move || theme().foreground)
            })
            .active(|s| s.opacity(0.7))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(app_menu::tab_list_chevron()),
    )
}

/// The command palette's place in the title bar, where a browser has its address bar.
fn palette_button(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    view()
        .window_drag(false)
        .flex_shrink(1.0)
        .w(px(220.0))
        .min_w(px(120.0))
        .max_w(px(260.0))
        .child(
            button()
                .accessibility_label(t!("palette.title"))
                .on_press(move || run(CommandId::OpenPalette))
                .w_full()
                .min_w(px(0.0))
                .h(px(28.0))
                .px_3()
                .flex_row()
                .items_center()
                .gap_2()
                .overflow_hidden()
                .rounded(px(8.0))
                .bg(move || theme().background)
                .border_1()
                .border_color(move || theme().border)
                .hover(|s| s.border_color(move || theme().ring))
                .child(
                    text(move || t!("palette.placeholder"))
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
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

/// Main workspace layout: Vivaldi thin left icon rail, optional expandable left drawer, center pane grid, and
/// optional right vertical sidebar.
fn workspace_body(
    state: SidebarState,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    panes: Panes,
    shared: Shared,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let full_screen = shared.full_screen;
    view()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .flex_row()
        .on_mouse_move(move |event: &MouseEvent| {
            let Some((start_x, start_w)) = state.left_drag_start.get_untracked() else {
                return;
            };
            if event.pressed_button.is_none() {
                state.left_drag_start.set(None);
                return;
            }
            let next = (start_w + (event.page_position.x.0 - start_x))
                .clamp(LEFT_DRAWER_MIN_WIDTH, LEFT_DRAWER_MAX_WIDTH);
            state.left_drawer_width.set(next);
        })
        .on_mouse_up(move |_: &MouseEvent| {
            if state.left_drag_start.get_untracked().is_some() {
                state.left_drag_start.set(None);
            }
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            if state.left_drag_start.get_untracked().is_some() {
                state.left_drag_start.set(None);
            }
        })
        .child(when(
            move || state.left_rail_visible.get() && !full_screen.is_on(),
            move || left_icon_rail(state.left_expanded, run),
        ))
        .child(when(
            move || {
                state.left_rail_visible.get()
                    && !full_screen.is_on()
                    && state.left_expanded.with(Option::is_some)
            },
            move || left_drawer(state, panes, run),
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden()
                .child(panes::pane_grid(panes, shared, dialog_open, run)),
        )
        .child(when(
            move || state.right_open.get() && !full_screen.is_on(),
            move || right_sidebar(state, panes, run),
        ))
}

/// Vivaldi-style thin vertical left icon rail (`38px`) with pinned panel toggles at top and quick shortcuts below.
fn left_icon_rail(
    expanded: Signal<Option<Panel>>,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let mut rail = view()
        .w(px(LEFT_RAIL_WIDTH))
        .flex_shrink(0.0)
        .py_1p5()
        .gap_1()
        .items_center()
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_r_color(move || theme().border);

    for panel in Panel::LEFT_RAIL {
        let active = move || expanded.get() == Some(panel);
        rail = rail.child(
            view()
                .w_full()
                .h(px(32.0))
                .flex_row()
                .items_center()
                .pl(px(1.0))
                .gap(px(2.0))
                .child(view().w(px(2.0)).h(px(18.0)).rounded_full().bg(move || {
                    if active() {
                        theme().primary
                    } else {
                        theme().sidebar
                    }
                }))
                .child(
                    button()
                        .accessibility_label(panel.title())
                        .on_press(move || {
                            expanded.update(|current| {
                                if *current == Some(panel) {
                                    *current = None;
                                } else {
                                    *current = Some(panel);
                                }
                            });
                        })
                        .w(px(30.0))
                        .h(px(30.0))
                        .rounded(px(6.0))
                        .items_center()
                        .justify_center()
                        .style_when(active, |s| s.bg(move || theme().sidebar_accent))
                        .hover(|s| s.bg(move || theme().sidebar_accent))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                        .child(app_menu::chrome_icon(panel.rail_icon(), 18.0, move || {
                            if active() {
                                theme().sidebar_foreground
                            } else {
                                theme().muted_foreground
                            }
                        })),
                ),
        );
    }

    rail.child(view().flex_1())
        .child(rail_quick_button(
            ChromeIcon::RailLibrary,
            CommandId::OpenLibrary,
            run,
        ))
        .child(rail_quick_button(
            ChromeIcon::RailSettings,
            CommandId::OpenSettings,
            run,
        ))
}

/// Bottom quick-action icon button on the left icon rail (Library, Settings).
fn rail_quick_button(
    icon: ChromeIcon,
    command_id: CommandId,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(commands::label(command_id))
        .on_press(move || run(command_id))
        .w(px(30.0))
        .h(px(30.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(app_menu::chrome_icon(icon, 18.0, move || {
            theme().muted_foreground
        }))
}

/// Expandable wide left sidebar drawer rendered beside the thin icon rail with header close button and drag handle.
fn left_drawer(
    state: SidebarState,
    panes: Panes,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let header = view()
        .h(px(36.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(move || {
                state
                    .left_expanded
                    .get()
                    .map(Panel::title)
                    .unwrap_or_default()
            })
            .flex_1()
            .truncate()
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().sidebar_foreground),
        )
        .child(drawer_close_button(move || state.left_expanded.set(None)));

    let body = view().flex_1().min_h(px(0.0)).child(dynamic(move || {
        let Some(panel) = state.left_expanded.get() else {
            return view().into_element();
        };
        untracked(move || of_active(panes, move |pane| side_panel(panel, pane, panes, run)))
    }));

    let resize_edge = view()
        .w(px(5.0))
        .flex_shrink(0.0)
        .cursor(Cursor::ColResize)
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_r_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .style_when(
            move || state.left_drag_start.with(Option::is_some),
            |s| s.bg(move || theme().accent),
        )
        .on_press_down(move |event: &PressEvent| {
            state.left_drag_start.set(Some((
                event.page_position.x.0,
                state.left_drawer_width.get_untracked(),
            )));
        })
        .on_press_up(move |_: &PressEvent| state.left_drag_start.set(None));

    let drawer_ref = element_ref();
    drawer_ref.on_focus_in(move || state.left_drawer_focused.set(true));
    drawer_ref.on_focus_out(move || state.left_drawer_focused.set(false));
    cleanup(move || state.left_drawer_focused.set(false));

    view()
        .element_ref(&drawer_ref)
        .w(move || px(state.left_drawer_width.get()))
        .flex_shrink(0.0)
        .flex_row()
        .bg(move || theme().sidebar)
        .child(view().flex_1().min_w(px(0.0)).child(header).child(body))
        .child(resize_edge)
}

/// Docked right vertical sidebar with header tabs (`Inspector`, `Annotations`, `Results`) and a close button.
fn right_sidebar(
    state: SidebarState,
    panes: Panes,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let right_open = state.right_open;
    let right_panel = state.right_panel;
    let sidebar_ref = element_ref();
    sidebar_ref.on_focus_in(move || state.right_sidebar_focused.set(true));
    sidebar_ref.on_focus_out(move || state.right_sidebar_focused.set(false));
    cleanup(move || state.right_sidebar_focused.set(false));
    let mut header = view()
        .h(px(36.0))
        .px_2()
        .flex_row()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_b_color(move || theme().border);

    for panel in Panel::RIGHT_TABS {
        let active = move || right_panel.get() == panel;
        header = header.child(
            button()
                .accessibility_label(panel.title())
                .on_press(move || right_panel.set(panel))
                .h(px(26.0))
                .px_2()
                .rounded(px(5.0))
                .items_center()
                .justify_center()
                .style_when(active, |s| s.bg(move || theme().sidebar_accent))
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .active(|s| s.opacity(0.75))
                .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                .child(
                    text(move || panel.title())
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active() {
                                theme().sidebar_foreground
                            } else {
                                theme().muted_foreground
                            }
                        }),
                ),
        );
    }

    header = header
        .child(view().flex_1())
        .child(drawer_close_button(move || right_open.set(false)));

    let body = view().flex_1().min_h(px(0.0)).child(dynamic(move || {
        let panel = right_panel.get();
        untracked(move || of_active(panes, move |pane| side_panel(panel, pane, panes, run)))
    }));

    view()
        .element_ref(&sidebar_ref)
        .w(px(RIGHT_SIDEBAR_WIDTH))
        .flex_shrink(0.0)
        .bg(move || theme().sidebar)
        .border_l_1()
        .border_l_color(move || theme().border)
        .child(header)
        .child(body)
}

/// Compact `×` close button in the left drawer and right sidebar headers.
fn drawer_close_button(on_close: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(t!("sidebar.close_panel"))
        .on_press(on_close)
        .w(px(24.0))
        .h(px(24.0))
        .flex_shrink(0.0)
        .rounded(px(5.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(app_menu::chrome_icon(ChromeIcon::CloseX, 12.0, move || {
            theme().muted_foreground
        }))
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
        Panel::Annotations => {
            crate::annotate::annotation_list(pane.annotate, pane.pages, go).into_element()
        }
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
