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
use crate::panes::{self, CELLS, FileDrag, Pane, PaneAction, Panes, Place, Shared};
use crate::present_ui;
use crate::print::{Printing, print_strip};
use crate::reading::Reading;
use crate::reading_theme_bar::reading_theme_bar;
use crate::recent::RecentState;
use crate::settings;
use crate::settings_window::settings_window;
use crate::study_ui;
use crate::tabs;
use crate::theme as themes;
use crate::thumbnails;
use crate::workspaces::{self, MAIN, Workspaces};
use excellent_features::annotate::lasso;
use excellent_features::annotate::shapes::Kind as ShapeKind;
use excellent_features::modes::Mode;
use excellent_features::present::{ActivePointerTool, ScreenBlank, WhiteboardBackground};

/// The title bar's height; Windows draws its caption buttons this tall.
const TITLE_BAR_HEIGHT: Points = px(40.0);

/// Below this window width the docks become drawers over the document (U.4): with the sidebar's 260 points and the
/// inspector's 300 both open, the page would get less than 340. A 1280-point screen at 150 % scaling is 853.
const COMPACT_BELOW: Points = px(900.0);

/// A workspace window's size before the platform keeps one of its own (W.8).
const WINDOW_SIZE: (f32, f32) = (1100.0, 800.0);

/// The main window's size the first time it opens, in points; after that it opens where it was left.
const MAIN_WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

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
    /// Every annotation of the document, filtered and searched (A.14).
    Annotations,
}

impl Panel {
    const ALL: [Self; 7] = [
        Self::Document,
        Self::Pages,
        Self::Outline,
        Self::Bookmarks,
        Self::Results,
        Self::Inspector,
        Self::Annotations,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Pages => "pages",
            Self::Outline => "outline",
            Self::Bookmarks => "bookmarks",
            Self::Results => "results",
            Self::Inspector => "inspector",
            Self::Annotations => "annotations",
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
            Self::Annotations => t!("panel.annotations"),
        }
    }

    /// The heading of its empty state.
    fn heading(self) -> String {
        match self {
            Self::Document => t!("document.empty.title"),
            Self::Pages
            | Self::Outline
            | Self::Bookmarks
            | Self::Results
            | Self::Inspector
            | Self::Annotations => self.title(),
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
            Self::Annotations => t!("annotations.no_document"),
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
    // The find results and the annotation list (A.14) sit beside the inspector: a fourth tab would not fit the
    // sidebar's width, and a row chosen in the list opens in the inspector next to it.
    let mut inspector = DockRegion::new(
        DockLayout::tabs()
            .panel(Panel::Inspector.key())
            .panel(Panel::Annotations.key())
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
    // W.8: the windows open when the app last quit, once the main window is up.
    spawn(async move {
        next_frame().await;
        for id in workspaces::reopened(shown) {
            open_workspace_window(app, id);
        }
    });
    // NSH 0.7's window model: the app host never shows, so the main window is declared like the others, and opens
    // where the reader left it.
    nsh_core::element::desktop::window()
        .id("main")
        .title("ExcellentPDF".to_owned())
        .default_size(size(px(MAIN_WINDOW_SIZE.0), px(MAIN_WINDOW_SIZE.1)))
        .restore_bounds(true)
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
                pane.annotate.clear_selection();
                to_canvas("leaveModes");
                full_screen.leave();
            }
            // A.13: with annotations selected, copy and the arrow keys are theirs.
            CommandId::Canvas(name) => {
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
                layout.update(|layout| show_panel(layout, Panel::Annotations.key()));
            }
            CommandId::LayersByAuthor => pane.annotate.layers_by_author(),
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
            .child(when(
                move || pane.annotate.is_restyling(),
                move || crate::annotate::ink_tools::restyle_dialog(pane.annotate),
            ))
            .child(when(
                move || pane.annotate.is_filling(),
                move || crate::annotate::ink_tools::fill_dialog(pane.annotate),
            ))
            .child(when(
                move || pane.annotate.is_palettes(),
                move || crate::annotate::ink_tools::palettes_dialog(pane.annotate),
            ))
            .child(when(
                move || pane.annotate.is_replaying(),
                move || crate::annotate::ink_tools::replay_dialog(pane.annotate),
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
            app_menu_open,
        ))
        .child(modes::mode_bar(
            modes,
            move || panes.active().pages.count() > 0,
            move || full_screen.is_on(),
            move |id| {
                let pane = panes.slots()[panes.active_cell()];
                pane.annotate.is_on(id)
                    || pane.present.is_on(id)
                    || pane.study.is_on(id)
                    || (id == CommandId::ReflowMode && pane.reading_extra.is_reflow_open())
                    || (id == CommandId::BionicReading && pane.reading_extra.is_bionic_on())
                    || (id == CommandId::CadView && pro_pdf_ui.is_cad_active(live))
                    || (id == CommandId::MeasureTool && pro_pdf_ui.is_measure_active())
                    || (id == CommandId::ScreencastRecord && pro_pdf_ui.is_screencast_active())
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
        .child(of_active(panes, |pane| {
            crate::annotate::favorites::pen_dialog(pane.annotate)
        }))
        .child(of_active(panes, |pane| {
            crate::snapshot::snapshot_dialog(pane.snapshot)
        }))
        .child(of_active(panes, |pane| {
            crate::attachments::attachments_dialog(pane.attachments)
        }))
        .child(of_active(panes, move |pane| {
            view()
                .child(when(
                    move || pane.notebooks.is_new_open(),
                    move || {
                        notebooks::new_notebook_dialog(pane.notebooks, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_insert_page_open(),
                    move || {
                        notebooks::insert_page_dialog(pane.notebooks, move |path| {
                            let page = pane.pages.current();
                            let _ = pane.tabs.take_shown();
                            pane.tabs.open_at(path, page);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_margins_open(),
                    move || {
                        notebooks::margins_dialog(pane.notebooks, move |path| {
                            let page = pane.pages.current();
                            let _ = pane.tabs.take_shown();
                            pane.tabs.open_at(path, page);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_infinite_open(),
                    move || notebooks::infinite_canvas_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.notebooks.is_text_block_open(),
                    move || notebooks::text_block_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.notebooks.is_sections_open(),
                    move || {
                        notebooks::sections_dialog(pane.notebooks, move |path| {
                            let page = pane.pages.current();
                            let _ = pane.tabs.take_shown();
                            pane.tabs.open_at(path, page);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_links_open(),
                    move || {
                        notebooks::links_manager_dialog(pane.notebooks, move |path| {
                            let page = pane.pages.current();
                            let _ = pane.tabs.take_shown();
                            pane.tabs.open_at(path, page);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_markdown_open(),
                    move || notebooks::markdown_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.notebooks.is_audio_open(),
                    move || notebooks::audio_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.notebooks.is_condense_open(),
                    move || {
                        notebooks::condense_slides_dialog(pane.notebooks, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
                .child(when(
                    move || pane.present.is_keep_prompt_open(),
                    move || present_ui::keep_ink_dialog(pane.present),
                ))
                .child(when(
                    move || pane.study.is_flashcard_open(),
                    move || study_ui::flashcard_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_occlusion_open(),
                    move || study_ui::occlusion_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_cloze_open(),
                    move || study_ui::cloze_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_review_open(),
                    move || {
                        study_ui::daily_review_dialog(pane.study, move |path, page| {
                            pane.tabs.open_at(path, page);
                        })
                    },
                ))
                .child(when(
                    move || pane.study.is_dict_open(),
                    move || study_ui::dictionary_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_vocab_open(),
                    move || study_ui::vocabulary_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_export_open(),
                    move || study_ui::study_export_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_stats_open(),
                    move || study_ui::statistics_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_summaries_open(),
                    move || study_ui::phase5::highlight_summaries_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_assignments_open(),
                    move || study_ui::phase5::assignments_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_language_open(),
                    move || study_ui::phase5::language_learning_dialog(pane.study),
                ))
                .child(when(
                    move || pane.study.is_translate_open(),
                    move || study_ui::phase5::translate_dialog(pane.study),
                ))
                .child(when(
                    move || pane.notebooks.is_planner_open(),
                    move || {
                        notebooks::phase5::planner_dialog(pane.notebooks, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_packs_open(),
                    move || notebooks::phase5::template_packs_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.notebooks.is_versions_open(),
                    move || {
                        notebooks::phase5::version_history_dialog(pane.notebooks, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
                .child(when(
                    move || pane.notebooks.is_transcript_open(),
                    move || notebooks::phase5::audio_transcript_dialog(pane.notebooks),
                ))
                .child(when(
                    move || pane.outline_extra.is_generate_open(),
                    move || {
                        outline::extra::generate_outline_dialog(
                            pane.outline_extra,
                            pane.tabs.shown_path(),
                        )
                    },
                ))
                .child(when(
                    move || pane.outline_extra.is_io_open(),
                    move || {
                        outline::extra::outline_io_dialog(
                            pane.outline_extra,
                            pane.tabs.shown_path(),
                        )
                    },
                ))
                .child(when(
                    move || pane.reading_extra.is_reflow_open(),
                    move || {
                        reading_extra::reflow_dialog(
                            pane.reading_extra,
                            pane.tabs.shown_path(),
                            pane.pages.current(),
                        )
                    },
                ))
                .child(when(
                    move || pane.reading_extra.is_synctex_open(),
                    move || {
                        reading_extra::synctex_dialog(
                            pane.reading_extra,
                            pane.tabs.shown_path(),
                            pane.pages.current(),
                            move |pg| pane.go_to(pg),
                        )
                    },
                ))
                .child(when(
                    move || pane.reading_extra.is_remote_open(),
                    move || {
                        reading_extra::open_url_dialog(pane.reading_extra, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
                .child(when(
                    move || pane.reading_extra.is_extra_open(),
                    move || reading_extra::extra_format_dialog(pane.reading_extra),
                ))
                .child(when(
                    move || pane.annotate.is_reporting(),
                    move || crate::annotate::phase5::report_export_dialog(pane.annotate),
                ))
                .child(when(
                    move || pane.annotate.is_history(),
                    move || crate::annotate::phase5::history_timeline_dialog(pane.annotate),
                ))
                .child(when(
                    move || pane.annotate.is_pattern(),
                    move || crate::annotate::phase5::pattern_brush_dialog(pane.annotate),
                ))
                .child(when(
                    move || pane.annotate.is_symmetry(),
                    move || crate::annotate::phase5::symmetry_dialog(pane.annotate),
                ))
                .child(when(
                    move || pane.annotate.is_hw_searching(),
                    move || {
                        crate::annotate::phase5::handwriting_search_dialog(
                            pane.annotate,
                            move |pg| pane.go_to(pg),
                        )
                    },
                ))
                .child(when(
                    move || pane.annotate.is_hw_converting(),
                    move || crate::annotate::phase5::handwriting_convert_dialog(pane.annotate),
                ))
                .child(when(
                    move || pane.annotate.is_ocr(),
                    move || {
                        crate::annotate::phase5::ocr_dialog(pane.annotate, move |path| {
                            workspaces.open_paths(vec![path]);
                        })
                    },
                ))
        }))
        .child(when(
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
        ))
        .child(when(
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
        ))
        .child(when(
            move || lib_ui.is_citations_open(),
            move || library_ui::citations_dialog(lib_ui),
        ))
        .child(when(
            move || lib_ui.is_notes_open(),
            move || {
                let pane = panes.active();
                library_ui::notes_backlinks_dialog(
                    lib_ui,
                    pane.tabs.shown_path(),
                    pane.pages.current(),
                )
            },
        ))
        .child(when(
            move || lib_ui.is_rules_open(),
            move || {
                library_ui::workspace_rules_dialog(lib_ui, workspaces.shown_id(), move |ws_id| {
                    workspaces.switch(ws_id);
                })
            },
        ))
        .child(when(
            move || lib_ui.is_cli_docs_open(),
            move || library_ui::cli_mcp_docs_dialog(lib_ui),
        ))
        .child(ai_ui::ai_dialogs(ai_ui, panes, live))
        .child(forms_sign_ui::forms_sign_dialogs(
            forms_sign_ui,
            panes,
            workspaces,
            live,
        ))
        .child(pro_pdf_ui::pro_pdf_dialogs(
            pro_pdf_ui, panes, workspaces, live,
        ))
        .child(ecosystem_ui::ecosystem_dialogs(
            ecosystem_ui,
            panes,
            workspaces,
            live,
        ))
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
    app_menu_open: Signal<bool>,
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
        .child(view().window_drag(false).child(app_menu::app_menu_button(
            app_menu_open,
            run,
            move || panes.active().tabs.has_shown(),
        )))
        .child(workspaces::switcher_button(workspaces, run))
        .child(bar_button(
            "command.toggle_sidebar",
            CommandId::ToggleSidebar,
            run,
            dock_glyph(DockPlacement::Left),
        ))
        // The reading-theme bar needs the room while it is open.
        .child(when(
            move || !reading_theme_open.get(),
            move || {
                of_active(panes, move |pane| {
                    tabs::tab_strip(pane.tabs, pane.home_open, run)
                })
            },
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
        .child(of_active(panes, |pane| {
            crate::annotate::guides::reading_line(pane.annotate)
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
        // Dedicated draggable grip area to the left of the top-right tab-list dropdown.
        .child(view().min_w(px(72.0)).h_full())
        .child(tab_list_button(run))
}

/// Vivaldi-style top-right framed dropdown trigger opening the searchable workspace-grouped tab list.
fn tab_list_button(run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    view().window_drag(false).child(
        button()
            .accessibility_label(move || {
                Some(SharedString::from(commands::label_with_shortcut(
                    CommandId::TabList,
                )))
            })
            .on_press(move || run(CommandId::TabList))
            .w(px(28.0))
            .h(px(24.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(move || theme().border)
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.7))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(app_menu::dropdown_arrow()),
    )
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
                    | Panel::Inspector
                    | Panel::Annotations),
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
        Panel::Annotations => {
            crate::annotate::annotation_list(pane.annotate, pane.pages, go).into_element()
        }
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
