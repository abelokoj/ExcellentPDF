//! The command table (plan K.16): every action the app has, written once. The command palette's items and the
//! keyboard shortcuts are both built from it, so no command is in one and missing from the other. The keys a command
//! runs from are its defaults here with the user's own over them ([`crate::keymap`]).
//!
//! A command has keys for the whole window and keys for the pages: plain keys such as the arrows, Space and single
//! letters belong to whatever has the keyboard focus, so they reach the pages only while the pages have it (Enhanced's
//! rule for its canvas). A command that needs a document is left out of the palette while none is open.

use excellent_features::modes::Mode;
use excellent_features::outline::{Restyle, Shift, Swatch};
use nsh_base::CommandItem;
use nsh_core::prelude::*;

use excellent_features::split::Follow;

use crate::bookmarks::BookmarkAction;
use crate::find::FindAction;
use crate::keymap::{self, Keymap, Scope};
use crate::outline::OutlineAction;
use crate::panes::{PaneAction, Side};
use crate::tabs::{Action as TabAction, Step};
use crate::workspaces::{Start, Transfer, WorkspaceAction};

/// What a command does; [`crate::shell`] carries it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandId {
    OpenPalette,
    /// Opens the settings window (K.16).
    OpenSettings,
    ChooseTheme,
    ChooseAppearance,
    ToggleSidebar,
    ToggleInspector,
    /// Opens the go-to box (R.6).
    GoToPage,
    /// Full screen on or off (R.19); F11 does it too, done by the platform itself.
    ToggleFullScreen,
    /// The reading ruler tints its band, or dims the page around it (R.20).
    ToggleRulerInvert,
    /// The pages in the theme's colours, or in their own (R.14).
    ToggleDocumentColors,
    /// Opens the reading-theme bar (R.15).
    ChooseReadingTheme,
    /// Opens files in new tabs (R.12).
    OpenFile,
    /// The tab list (R.12): every tab, searchable.
    TabList,
    /// Offers the app for its document types and opens the system's default-apps settings (R.11).
    SetDefaultViewer,
    /// Something done to the shown tab (R.12).
    Tab(TabAction),
    /// Something done to the split view's panes (V.1 to V.6).
    Pane(PaneAction),
    /// Something done to the workspaces (W.1 to W.5).
    Workspace(WorkspaceAction),
    /// Escape: out of full screen, focus mode, the reading ruler and auto-scroll (R.19).
    LeaveModes,
    /// A page-canvas command, by its name in the canvas's protocol (`zoomIn`, `layoutBook`, `rotateClockwise`).
    Canvas(&'static str),
    /// Something done to the shown document's outline (B.1 to B.5).
    Outline(OutlineAction),
    /// Something done to the shown document's personal bookmarks (B.6).
    Bookmark(BookmarkAction),
    /// Find in the shown document (F.1).
    Find(FindAction),
    /// Opens the markup tools' colour dialog (A.3).
    AnnotationColors,
    /// Places a sticky note where the page is clicked next (A.5).
    StickyNote,
    /// Takes back the shown document's last edit (K.14).
    Undo,
    /// Makes the last undone edit again (K.14).
    Redo,
    /// Saves the edited documents now (K.15); they are saved on their own too.
    Save,
    /// Prints the shown document (O.22).
    Print,
    /// Shows the first-run guide again (U.9).
    ReplayGuide,
    /// Switches to a mode and its toolbar (K.18).
    Mode(Mode),
    /// Switches to the next offered mode, or with `true` the previous one (K.18).
    StepMode(bool),
    /// Opens the toolbar's customize dialog (K.18).
    CustomizeToolbar,
    /// Saves every mode's toolbar as a preset file (K.18).
    ExportToolbars,
    /// Reads toolbars from a preset file (K.18).
    ImportToolbars,
}

impl CommandId {
    /// Whether it does anything now: page commands need a document.
    pub(crate) fn is_available(self, has_document: bool) -> bool {
        has_document
            || !matches!(
                self,
                Self::Canvas(_)
                    | Self::GoToPage
                    | Self::Outline(_)
                    | Self::Bookmark(_)
                    | Self::Find(_)
                    | Self::StickyNote
                    | Self::Undo
                    | Self::Redo
                    | Self::Save
                    | Self::Print
            ) && !matches!(self, Self::Tab(action) if action != TabAction::ReopenClosed)
                && !matches!(
                    self,
                    Self::Pane(PaneAction::Split(_) | PaneAction::MoveTab(_))
                        | Self::Workspace(
                            WorkspaceAction::New(Start::AllTabs | Start::ShownTab)
                                | WorkspaceAction::SendTab(_)
                        )
                )
    }
}

/// One row of the table.
#[derive(Debug)]
pub(crate) struct Command {
    pub(crate) id: CommandId,
    /// Its stable name: the palette's item id, and later its key in `settings.toml`.
    pub(crate) name: &'static str,
    /// The localization key of its label.
    pub(crate) label: &'static str,
    /// The localization key of its palette section.
    group: &'static str,
    /// Its default shortcuts anywhere in the window, as NSH's keymap writes them; the first is the one hints show.
    pub(crate) keys: &'static [&'static str],
    /// Its default shortcuts while the pages have the keyboard focus; hints show the first when `keys` is empty.
    pub(crate) page_keys: &'static [&'static str],
}

/// Every command. Defaults follow SumatraPDF Enhanced where it has the command (`sumatra_pdf_enhanced_plan.md`).
pub(crate) const COMMANDS: &[Command] = &[
    Command {
        id: CommandId::OpenPalette,
        name: "palette.open",
        label: "command.open_palette",
        group: "command.group.general",
        // Enhanced's CmdCommandPalette.
        keys: &["mod+k", "mod+shift+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenFile,
        name: "file.open",
        label: "command.open_file",
        group: "command.group.file",
        // Enhanced's CmdOpenFile.
        keys: &["mod+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Save,
        name: "file.save",
        label: "command.save",
        group: "command.group.file",
        // Enhanced's CmdSave.
        keys: &["mod+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Print,
        name: "file.print",
        label: "command.print",
        group: "command.group.file",
        // Enhanced's CmdPrint.
        keys: &["mod+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::SetDefaultViewer,
        name: "app.set_default_viewer",
        label: "command.set_default_viewer",
        group: "command.group.file",
        // Enhanced's "Make default PDF reader" button (its options dialog).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Close),
        name: "tab.close",
        label: "command.close_tab",
        group: "command.group.tabs",
        // Enhanced's CmdClose.
        keys: &["mod+w", "mod+f4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::ReopenClosed),
        name: "tab.reopen_closed",
        label: "command.reopen_closed_tab",
        group: "command.group.tabs",
        // The browsers' key; Enhanced has none.
        keys: &["mod+shift+t"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Cycle(Step::Next)),
        name: "tab.next",
        label: "command.next_tab",
        group: "command.group.tabs",
        // Enhanced's CmdNextTab.
        keys: &["ctrl+tab", "ctrl+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Cycle(Step::Previous)),
        name: "tab.previous",
        label: "command.previous_tab",
        group: "command.group.tabs",
        // Enhanced's CmdPrevTab.
        keys: &["ctrl+shift+tab", "ctrl+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::TabList,
        name: "tab.list",
        label: "command.tab_list",
        group: "command.group.tabs",
        // Vivaldi's and Edge's tab search.
        keys: &["mod+shift+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Move(Step::Previous)),
        name: "tab.move_left",
        label: "command.move_tab_left",
        group: "command.group.tabs",
        // Enhanced's CmdMoveTabLeft.
        keys: &["ctrl+shift+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Move(Step::Next)),
        name: "tab.move_right",
        label: "command.move_tab_right",
        group: "command.group.tabs",
        // Enhanced's CmdMoveTabRight.
        keys: &["ctrl+shift+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::TogglePin),
        name: "tab.pin",
        label: "command.pin_tab",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseOthers),
        name: "tab.close_others",
        label: "command.close_other_tabs",
        group: "command.group.tabs",
        // Enhanced's CmdCloseOtherTabs.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseToTheRight),
        name: "tab.close_right",
        label: "command.close_tabs_right",
        group: "command.group.tabs",
        // Enhanced's CmdCloseTabsToTheRight.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseDuplicates),
        name: "tab.close_duplicates",
        label: "command.close_duplicate_tabs",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::NewGroup),
        name: "tab.group.new",
        label: "command.new_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::EditGroup),
        name: "tab.group.edit",
        label: "command.edit_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::LeaveGroup),
        name: "tab.group.leave",
        label: "command.leave_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::ToggleGroup),
        name: "tab.group.toggle",
        label: "command.toggle_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenSettings,
        name: "app.settings",
        label: "command.settings",
        group: "command.group.general",
        // The common desktop key for an app's settings (Enhanced opens its file from a menu).
        keys: &["mod+,"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ReplayGuide,
        name: "help.guide",
        label: "command.replay_guide",
        group: "command.group.general",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ChooseTheme,
        name: "view.theme",
        label: "command.choose_theme",
        group: "command.group.view",
        // No default shortcut (Enhanced has none); K.16 lets the user set one.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ChooseAppearance,
        name: "view.appearance",
        label: "command.choose_appearance",
        group: "command.group.view",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleSidebar,
        name: "view.sidebar",
        label: "command.toggle_sidebar",
        group: "command.group.view",
        // Enhanced's CmdToggleBookmarks: its sidebar.
        keys: &["f12"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleInspector,
        name: "view.inspector",
        label: "command.toggle_inspector",
        group: "command.group.view",
        // Enhanced has no inspector; VS Code's secondary side bar.
        keys: &["mod+alt+b"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Undo,
        name: "edit.undo",
        label: "command.undo",
        group: "command.group.edit",
        // Enhanced's undo. A text field that has the focus keeps its own.
        keys: &["mod+z"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Redo,
        name: "edit.redo",
        label: "command.redo",
        group: "command.group.edit",
        // Not Ctrl+Y: Enhanced gives it to zoom.
        keys: &["mod+shift+z"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("highlight"),
        name: "markup.highlight",
        label: "command.highlight",
        group: "command.group.annotate",
        keys: &[],
        // Enhanced's CmdCreateAnnotHighlight is "a".
        page_keys: &["a"],
    },
    Command {
        id: CommandId::Canvas("underline"),
        name: "markup.underline",
        label: "command.underline",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["u"],
    },
    Command {
        id: CommandId::Canvas("squiggly"),
        name: "markup.squiggly",
        label: "command.squiggly",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+u"],
    },
    Command {
        id: CommandId::Canvas("strikeOut"),
        name: "markup.strikeout",
        label: "command.strikeout",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["x"],
    },
    Command {
        id: CommandId::Canvas("replaceText"),
        name: "markup.replace",
        label: "command.replace_text",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["e"],
    },
    Command {
        id: CommandId::Canvas("insertText"),
        name: "markup.insert",
        label: "command.insert_text",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["insert"],
    },
    Command {
        id: CommandId::Canvas("noteSelection"),
        name: "annotate.note",
        label: "command.note",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+n"],
    },
    Command {
        id: CommandId::StickyNote,
        name: "annotate.sticky",
        label: "command.sticky_note",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+s"],
    },
    Command {
        id: CommandId::AnnotationColors,
        name: "annotate.colors",
        label: "command.annotation_colors",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+c"],
    },
    Command {
        id: CommandId::Canvas("copy"),
        name: "edit.copy",
        label: "command.copy",
        group: "command.group.edit",
        // Enhanced's CmdCopySelection. A text field that has the focus keeps its own copy.
        keys: &["mod+c", "mod+insert"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("selectAll"),
        name: "edit.select_all",
        label: "command.select_all",
        group: "command.group.edit",
        // Enhanced's CmdSelectAll, for the page shown until the text of the whole document is indexed.
        keys: &["mod+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("zoomIn"),
        name: "view.zoom_in",
        label: "command.zoom_in",
        group: "command.group.zoom",
        // Enhanced's CmdZoomIn.
        keys: &["mod+plus"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("zoomOut"),
        name: "view.zoom_out",
        label: "command.zoom_out",
        group: "command.group.zoom",
        // Enhanced's CmdZoomOut.
        keys: &["mod+minus"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("actualSize"),
        name: "view.actual_size",
        label: "command.actual_size",
        group: "command.group.zoom",
        // Enhanced's CmdZoomActualSize.
        keys: &["mod+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("fitWidth"),
        name: "view.fit_width",
        label: "command.fit_width",
        group: "command.group.zoom",
        // Enhanced's CmdZoomFitWidth.
        keys: &["mod+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("fitPage"),
        name: "view.fit_page",
        label: "command.fit_page",
        group: "command.group.zoom",
        // Enhanced's CmdZoomFitPage.
        keys: &["mod+0"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("fitHeight"),
        name: "view.fit_height",
        label: "command.fit_height",
        group: "command.group.zoom",
        // Enhanced has no key for it.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutSingle"),
        name: "view.layout_single",
        label: "command.layout_single",
        group: "command.group.layout",
        // Enhanced's CmdSinglePageView.
        keys: &["mod+6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutFacing"),
        name: "view.layout_facing",
        label: "command.layout_facing",
        group: "command.group.layout",
        // Enhanced's CmdFacingView.
        keys: &["mod+7"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutBook"),
        name: "view.layout_book",
        label: "command.layout_book",
        group: "command.group.layout",
        // Enhanced's CmdBookView.
        keys: &["mod+8"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutHorizontal"),
        name: "view.layout_horizontal",
        label: "command.layout_horizontal",
        group: "command.group.layout",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutGrid"),
        name: "view.layout_grid",
        label: "command.layout_grid",
        group: "command.group.layout",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("toggleContinuous"),
        name: "view.toggle_continuous",
        label: "command.toggle_continuous",
        group: "command.group.layout",
        // Enhanced's `c`.
        keys: &[],
        page_keys: &["c"],
    },
    Command {
        id: CommandId::Canvas("toggleDirection"),
        name: "view.toggle_direction",
        label: "command.toggle_direction",
        group: "command.group.layout",
        // Enhanced's manga mode has no key.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("rotateClockwise"),
        name: "view.rotate_clockwise",
        label: "command.rotate_clockwise",
        group: "command.group.page",
        // Enhanced's CmdRotateRight.
        keys: &["mod+shift+plus"],
        page_keys: &["]"],
    },
    Command {
        id: CommandId::Canvas("rotateAnticlockwise"),
        name: "view.rotate_anticlockwise",
        label: "command.rotate_anticlockwise",
        group: "command.group.page",
        // Enhanced's CmdRotateLeft.
        keys: &["mod+shift+minus"],
        page_keys: &["["],
    },
    Command {
        id: CommandId::Canvas("boxMedia"),
        name: "view.box_media",
        label: "command.box_media",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxCrop"),
        name: "view.box_crop",
        label: "command.box_crop",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxBleed"),
        name: "view.box_bleed",
        label: "command.box_bleed",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxTrim"),
        name: "view.box_trim",
        label: "command.box_trim",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxArt"),
        name: "view.box_art",
        label: "command.box_art",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::GoToPage,
        name: "go.to_page",
        label: "command.go_to_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToPage.
        keys: &["mod+g"],
        page_keys: &["g"],
    },
    Command {
        id: CommandId::Canvas("firstPage"),
        name: "go.first_page",
        label: "command.first_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToFirstPage.
        keys: &["mod+home"],
        page_keys: &["home"],
    },
    Command {
        id: CommandId::Canvas("lastPage"),
        name: "go.last_page",
        label: "command.last_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToLastPage.
        keys: &["mod+end"],
        page_keys: &["end"],
    },
    Command {
        id: CommandId::Canvas("historyBack"),
        name: "go.back",
        label: "command.go_back",
        group: "command.group.navigate",
        // Enhanced's CmdNavigateBack.
        keys: &["alt+left"],
        page_keys: &["backspace"],
    },
    Command {
        id: CommandId::Canvas("historyForward"),
        name: "go.forward",
        label: "command.go_forward",
        group: "command.group.navigate",
        // Enhanced's CmdNavigateForward.
        keys: &["alt+right"],
        page_keys: &["shift+backspace"],
    },
    Command {
        id: CommandId::Canvas("screenDown"),
        name: "go.screen_down",
        label: "command.screen_down",
        group: "command.group.navigate",
        keys: &["pagedown"],
        page_keys: &["enter"],
    },
    Command {
        id: CommandId::Canvas("screenUp"),
        name: "go.screen_up",
        label: "command.screen_up",
        group: "command.group.navigate",
        keys: &["pageup"],
        page_keys: &["shift+space", "shift+enter"],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Add),
        name: "outline.add",
        label: "command.outline_add",
        group: "command.group.outline",
        // Enhanced's CmdAddBookmark.
        keys: &["mod+b"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Rename),
        name: "outline.rename",
        label: "command.outline_rename",
        group: "command.group.outline",
        // F2 while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::SetDestination),
        name: "outline.set_destination",
        label: "command.outline_set_destination",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Bold)),
        name: "outline.bold",
        label: "command.outline_bold",
        group: "command.group.outline",
        // Ctrl+Shift+B while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Italic)),
        name: "outline.italic",
        label: "command.outline_italic",
        group: "command.group.outline",
        // Ctrl+Shift+I while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(None))),
        name: "outline.color_none",
        label: "command.outline_color_none",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Red)))),
        name: "outline.color_red",
        label: "command.outline_color_red",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Orange)))),
        name: "outline.color_orange",
        label: "command.outline_color_orange",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Green)))),
        name: "outline.color_green",
        label: "command.outline_color_green",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Blue)))),
        name: "outline.color_blue",
        label: "command.outline_color_blue",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Purple)))),
        name: "outline.color_purple",
        label: "command.outline_color_purple",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Up)),
        name: "outline.move_up",
        label: "command.outline_move_up",
        group: "command.group.outline",
        // Alt+Up while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Down)),
        name: "outline.move_down",
        label: "command.outline_move_down",
        group: "command.group.outline",
        // Alt+Down while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::In)),
        name: "outline.indent",
        label: "command.outline_indent",
        group: "command.group.outline",
        // Tab while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Out)),
        name: "outline.outdent",
        label: "command.outline_outdent",
        group: "command.group.outline",
        // Shift+Tab while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Sort),
        name: "outline.sort",
        label: "command.outline_sort",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::OpenElsewhere),
        name: "outline.open_in_other_pane",
        label: "command.outline_open_elsewhere",
        group: "command.group.outline",
        // Ctrl+Enter while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::ToBookmark),
        name: "outline.to_bookmark",
        label: "command.outline_to_bookmark",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Check),
        name: "outline.check",
        label: "command.outline_check",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Fix),
        name: "outline.fix",
        label: "command.outline_fix",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Delete),
        name: "outline.delete",
        label: "command.outline_delete",
        group: "command.group.outline",
        // Delete while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Add),
        name: "bookmark.add",
        label: "command.bookmark_add",
        group: "command.group.bookmarks",
        // Plan B.6; Enhanced has no key for CmdFavoriteAdd.
        keys: &["mod+d"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Edit),
        name: "bookmark.edit",
        label: "command.bookmark_edit",
        group: "command.group.bookmarks",
        // F2 while the bookmarks have the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::ToOutline),
        name: "bookmark.to_outline",
        label: "command.bookmark_to_outline",
        group: "command.group.bookmarks",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Delete),
        name: "bookmark.delete",
        label: "command.bookmark_delete",
        group: "command.group.bookmarks",
        // Delete while the bookmarks have the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Open),
        name: "find.open",
        label: "command.find_open",
        group: "command.group.find",
        // Enhanced's CmdFindFirst.
        keys: &["mod+f"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Next),
        name: "find.next",
        label: "command.find_next",
        group: "command.group.find",
        // Enhanced's CmdFindNext; anywhere, so it works while the find bar has the keyboard.
        keys: &["f3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Previous),
        name: "find.previous",
        label: "command.find_previous",
        group: "command.group.find",
        // Enhanced's CmdFindPrev.
        keys: &["shift+f3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Close),
        name: "find.close",
        label: "command.find_close",
        group: "command.group.find",
        // Escape while the find bar has the keyboard (the bar binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleFullScreen,
        name: "view.full_screen",
        label: "command.full_screen",
        group: "command.group.read",
        // Enhanced's CmdToggleFullscreen; its F11 the platform takes itself.
        keys: &["mod+shift+l"],
        page_keys: &["f"],
    },
    Command {
        id: CommandId::Canvas("toggleFocus"),
        name: "view.focus_mode",
        label: "command.focus_mode",
        group: "command.group.read",
        // Enhanced has no focus mode.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("toggleRuler"),
        name: "view.reading_ruler",
        label: "command.reading_ruler",
        group: "command.group.read",
        // Enhanced's reading bar has no key.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleRulerInvert,
        name: "view.reading_ruler_invert",
        label: "command.reading_ruler_invert",
        group: "command.group.read",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleDocumentColors,
        name: "view.document_colors",
        label: "command.document_colors",
        group: "command.group.read",
        // Enhanced's Shift+I inverts the colours for the session; this one keeps the choice.
        keys: &[],
        page_keys: &["shift+i"],
    },
    Command {
        id: CommandId::ChooseReadingTheme,
        name: "view.reading_theme",
        label: "command.reading_theme",
        group: "command.group.read",
        // No default shortcut (Enhanced has no reading themes); K.16 lets the user set one.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("lineDown"),
        name: "go.line_down",
        label: "command.line_down",
        group: "command.group.read",
        // Enhanced's Down: a line down, or the reading bar's next line.
        keys: &[],
        page_keys: &["down", "j"],
    },
    Command {
        id: CommandId::Canvas("lineUp"),
        name: "go.line_up",
        label: "command.line_up",
        group: "command.group.read",
        keys: &[],
        page_keys: &["up", "k"],
    },
    Command {
        id: CommandId::Canvas("stepLeft"),
        name: "go.left",
        label: "command.step_left",
        group: "command.group.navigate",
        // Enhanced's Left and H; the row before or after follows the reading direction (R.3).
        keys: &[],
        page_keys: &["left", "h"],
    },
    Command {
        id: CommandId::Canvas("stepRight"),
        name: "go.right",
        label: "command.step_right",
        group: "command.group.navigate",
        keys: &[],
        page_keys: &["right", "l"],
    },
    Command {
        id: CommandId::Canvas("toggleAutoScroll"),
        name: "view.auto_scroll",
        label: "command.auto_scroll",
        group: "command.group.read",
        // Enhanced's auto-scroll toggle.
        keys: &["mod+shift+h"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("autoScrollPause"),
        name: "view.auto_scroll_pause",
        label: "command.auto_scroll_pause",
        group: "command.group.read",
        // Enhanced's Space: pauses auto-scroll, else a screen down.
        keys: &[],
        page_keys: &["space"],
    },
    Command {
        id: CommandId::Canvas("autoScrollFaster"),
        name: "view.auto_scroll_faster",
        label: "command.auto_scroll_faster",
        group: "command.group.read",
        // Only while auto-scroll runs; with Ctrl the same keys zoom.
        keys: &[],
        page_keys: &["plus"],
    },
    Command {
        id: CommandId::Canvas("autoScrollSlower"),
        name: "view.auto_scroll_slower",
        label: "command.auto_scroll_slower",
        group: "command.group.read",
        keys: &[],
        page_keys: &["minus"],
    },
    Command {
        id: CommandId::LeaveModes,
        name: "view.leave_modes",
        label: "command.leave_modes",
        group: "command.group.read",
        // Enhanced's Escape leaves full screen.
        keys: &[],
        page_keys: &["escape"],
    },
    Command {
        id: CommandId::Pane(PaneAction::Split(Side::Right)),
        name: "pane.split_right",
        label: "command.split_right",
        group: "command.group.panes",
        // VS Code's split key.
        keys: &["mod+\\"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Split(Side::Down)),
        name: "pane.split_down",
        label: "command.split_down",
        group: "command.group.panes",
        keys: &["mod+alt+\\"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::OpenInSplit),
        name: "pane.open_in_split",
        label: "command.open_in_split",
        group: "command.group.panes",
        keys: &["mod+shift+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::MoveTab(Step::Next)),
        name: "pane.move_tab_next",
        label: "command.move_tab_next_pane",
        group: "command.group.panes",
        keys: &["mod+alt+right"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::MoveTab(Step::Previous)),
        name: "pane.move_tab_previous",
        label: "command.move_tab_previous_pane",
        group: "command.group.panes",
        keys: &["mod+alt+left"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(0)),
        name: "pane.focus_1",
        label: "command.focus_pane_1",
        group: "command.group.panes",
        // The plan's Ctrl+1 to 4 are the tabs' (R.12); Alt+1 to 4 waits for the maintainer's word (dev_decisions).
        keys: &["alt+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(1)),
        name: "pane.focus_2",
        label: "command.focus_pane_2",
        group: "command.group.panes",
        keys: &["alt+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(2)),
        name: "pane.focus_3",
        label: "command.focus_pane_3",
        group: "command.group.panes",
        keys: &["alt+3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(3)),
        name: "pane.focus_4",
        label: "command.focus_pane_4",
        group: "command.group.panes",
        keys: &["alt+4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Cycle(Step::Next)),
        name: "pane.next",
        label: "command.next_pane",
        group: "command.group.panes",
        // The common key for the next pane.
        keys: &["f6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Cycle(Step::Previous)),
        name: "pane.previous",
        label: "command.previous_pane",
        group: "command.group.panes",
        keys: &["shift+f6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Close),
        name: "pane.close",
        label: "command.close_pane",
        group: "command.group.panes",
        keys: &["mod+alt+w"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Sync),
        name: "pane.sync",
        label: "command.sync_scroll",
        group: "command.group.panes",
        keys: &["mod+alt+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::SamePage)),
        name: "pane.sync_page",
        label: "command.sync_same_page",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::SamePosition)),
        name: "pane.sync_position",
        label: "command.sync_same_position",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::Pairs)),
        name: "pane.sync_pairs",
        label: "command.sync_pairs",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncZoom),
        name: "pane.sync_zoom",
        label: "command.sync_zoom",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::PairPages),
        name: "pane.pair_pages",
        label: "command.pair_pages",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::ClearPagePairs),
        name: "pane.clear_page_pairs",
        label: "command.clear_page_pairs",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::PopOut),
        name: "pane.pop_out",
        label: "command.pop_out",
        group: "command.group.panes",
        keys: &["mod+alt+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Swap),
        name: "pane.swap",
        label: "command.swap_panes",
        group: "command.group.panes",
        keys: &["mod+alt+x"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Flip),
        name: "pane.flip",
        label: "command.flip_panes",
        group: "command.group.panes",
        keys: &["mod+alt+f"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Maximize),
        name: "pane.maximize",
        label: "command.maximize_pane",
        group: "command.group.panes",
        keys: &["mod+alt+enter"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Equalize),
        name: "pane.equalize",
        label: "command.equalize_panes",
        group: "command.group.panes",
        keys: &["mod+alt+0"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Switcher),
        name: "workspace.switch",
        label: "command.switch_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::Empty)),
        name: "workspace.new",
        label: "command.new_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+n"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::AllTabs)),
        name: "workspace.new_from_tabs",
        label: "command.new_workspace_from_tabs",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::ShownTab)),
        name: "workspace.new_from_tab",
        label: "command.new_workspace_from_tab",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Move)),
        name: "workspace.move_tab",
        label: "command.move_tab_to_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Copy)),
        name: "workspace.copy_tab",
        label: "command.copy_tab_to_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Edit),
        name: "workspace.edit",
        label: "command.edit_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::OpenWindow),
        name: "workspace.open_window",
        label: "command.open_workspace_window",
        group: "command.group.workspaces",
        keys: &["mod+alt+shift+n"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Export),
        name: "workspace.export",
        label: "command.export_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Import),
        name: "workspace.import",
        label: "command.import_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Shift(Step::Previous)),
        name: "workspace.move_earlier",
        label: "command.move_workspace_earlier",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Shift(Step::Next)),
        name: "workspace.move_later",
        label: "command.move_workspace_later",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(0)),
        name: "workspace.1",
        label: "command.workspace_1",
        group: "command.group.workspaces",
        // The plan's keys (W.5).
        keys: &["mod+alt+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(1)),
        name: "workspace.2",
        label: "command.workspace_2",
        group: "command.group.workspaces",
        keys: &["mod+alt+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(2)),
        name: "workspace.3",
        label: "command.workspace_3",
        group: "command.group.workspaces",
        keys: &["mod+alt+3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(3)),
        name: "workspace.4",
        label: "command.workspace_4",
        group: "command.group.workspaces",
        keys: &["mod+alt+4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(4)),
        name: "workspace.5",
        label: "command.workspace_5",
        group: "command.group.workspaces",
        keys: &["mod+alt+5"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(5)),
        name: "workspace.6",
        label: "command.workspace_6",
        group: "command.group.workspaces",
        keys: &["mod+alt+6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(6)),
        name: "workspace.7",
        label: "command.workspace_7",
        group: "command.group.workspaces",
        keys: &["mod+alt+7"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(7)),
        name: "workspace.8",
        label: "command.workspace_8",
        group: "command.group.workspaces",
        keys: &["mod+alt+8"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(8)),
        name: "workspace.9",
        label: "command.workspace_9",
        group: "command.group.workspaces",
        keys: &["mod+alt+9"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Cycle(Step::Next)),
        name: "workspace.next",
        label: "command.next_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Cycle(Step::Previous)),
        name: "workspace.previous",
        label: "command.previous_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Read),
        name: "mode.read",
        label: "command.mode.read",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Annotate),
        name: "mode.annotate",
        label: "command.mode.annotate",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Write),
        name: "mode.write",
        label: "command.mode.write",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Study),
        name: "mode.study",
        label: "command.mode.study",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Present),
        name: "mode.present",
        label: "command.mode.present",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::StepMode(false),
        name: "mode.next",
        label: "command.mode.next",
        group: "command.group.mode",
        // Enhanced has no modes; Ctrl+M is the window's minimize on macOS, so Shift joins it.
        keys: &["mod+shift+m"],
        page_keys: &[],
    },
    Command {
        id: CommandId::StepMode(true),
        name: "mode.previous",
        label: "command.mode.previous",
        group: "command.group.mode",
        keys: &["mod+alt+shift+m"],
        page_keys: &[],
    },
    Command {
        id: CommandId::CustomizeToolbar,
        name: "toolbar.customize",
        label: "command.toolbar.customize",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportToolbars,
        name: "toolbar.export",
        label: "command.toolbar.export",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImportToolbars,
        name: "toolbar.import",
        label: "command.toolbar.import",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
];

impl Command {
    /// Its row in the command palette, in the app's language, with the first of its keys.
    fn palette_item(&self, keymap: &Keymap) -> CommandItem {
        let item = CommandItem::new(self.name, t!(self.label)).group(t!(self.group));
        match keymap.first_keys(self.id) {
            Some(keys) => item.shortcut(keys.to_owned()),
            None => item,
        }
    }
}

/// The command named `name`, as the palette reports it and settings name it.
pub(crate) fn find(name: &str) -> Option<CommandId> {
    COMMANDS.iter().find(|c| c.name == name).map(|c| c.id)
}

/// The command palette's items: every command available now, in table order.
pub(crate) fn palette_items(has_document: bool) -> Vec<CommandItem> {
    let keymap = keymap::current();
    COMMANDS
        .iter()
        .filter(|c| c.id.is_available(has_document))
        .map(|c| c.palette_item(&keymap))
        .collect()
}

/// The label of `id`, in the app's language.
pub(crate) fn label(id: CommandId) -> String {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .map(|c| t!(c.label))
        .unwrap_or_default()
}

/// What screen readers say for a button that runs `id`: its label, and its first shortcut if it has one.
pub(crate) fn label_with_shortcut(id: CommandId) -> String {
    let label = label(id);
    match shortcut_label(id) {
        keys if keys.is_empty() => label,
        keys => t!("command.with_shortcut", label = label, keys = keys),
    }
}

/// The first shortcut of `id`, as hints show it on this platform (`Ctrl+K`, `⌘K`).
pub(crate) fn shortcut_label(id: CommandId) -> String {
    keymap::current()
        .first_keys(id)
        .map(key_label)
        .unwrap_or_default()
}

/// Binds every command's shortcuts to `run` while the current component lives. Keys for the pages run only while
/// `on_page` says the pages have the keyboard; they go on to what has it either way, and repeat while held.
pub(crate) fn bind_keys(
    run: impl Fn(CommandId) + Copy + 'static,
    on_page: impl Fn() -> bool + Copy + 'static,
) {
    let page = KeyOptions {
        repeat: true,
        pass_through: true,
        ..KeyOptions::default()
    };
    for binding in keymap::current().bindings() {
        let id = binding.id;
        match binding.scope {
            Scope::Window => on_key(&binding.keys, move |_| run(id)),
            Scope::Page => on_key_with(&binding.keys, page, move |_| {
                if on_page() {
                    run(id);
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_found() {
        for command in COMMANDS {
            assert_eq!(find(command.name), Some(command.id), "{}", command.name);
        }
        assert_eq!(find("no.such.command"), None);
    }

    #[test]
    fn only_window_commands_are_offered_without_a_document() {
        assert!(CommandId::OpenPalette.is_available(false));
        assert!(CommandId::ToggleFullScreen.is_available(false));
        assert!(!CommandId::Canvas("zoomIn").is_available(false));
        assert!(!CommandId::GoToPage.is_available(false));
        assert!(CommandId::Canvas("zoomIn").is_available(true));
        assert!(!CommandId::Tab(TabAction::Close).is_available(false));
        assert!(CommandId::Tab(TabAction::ReopenClosed).is_available(false));
        assert!(CommandId::OpenFile.is_available(false));
        assert!(!CommandId::Pane(PaneAction::Split(Side::Right)).is_available(false));
        assert!(CommandId::Pane(PaneAction::OpenInSplit).is_available(false));
        assert!(CommandId::Workspace(WorkspaceAction::New(Start::Empty)).is_available(false));
        assert!(
            !CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Move)).is_available(false)
        );
        assert!(!CommandId::Undo.is_available(false));
        assert!(!CommandId::Outline(OutlineAction::Add).is_available(false));
    }

    #[test]
    fn every_command_has_english_text() {
        for command in COMMANDS {
            for key in [command.label, command.group] {
                assert!(
                    crate::l10n::EN.contains(&format!("\"{key}\":")),
                    "{key} is missing from app/locales/en.json"
                );
            }
        }
    }
}
