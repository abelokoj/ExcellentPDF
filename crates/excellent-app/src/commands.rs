//! The command table (plan K.16): every action the app has, written once. The command palette's items and the
//! keyboard shortcuts are both built from it, so no command is in one and missing from the other. The keys a command
//! runs from are its defaults here with the user's own over them ([`crate::keymap`]).
//!
//! A command has keys for the whole window and keys for the pages: plain keys such as the arrows, Space and single
//! letters belong to whatever has the keyboard focus, so they reach the pages only while the pages have it (Enhanced's
//! rule for its canvas). A command that needs a document is left out of the palette while none is open.

use excellent_features::outline::Shift;
use nsh_base::CommandItem;
use nsh_core::prelude::*;

use crate::keymap::{self, Keymap, Scope};
use crate::outline::OutlineAction;
use crate::tabs::{Action as TabAction, Step};

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
    /// Escape: out of full screen, focus mode, the reading ruler and auto-scroll (R.19).
    LeaveModes,
    /// A page-canvas command, by its name in the canvas's protocol (`zoomIn`, `layoutBook`, `rotateClockwise`).
    Canvas(&'static str),
    /// Something done to the shown document's outline (B.1 to B.5).
    Outline(OutlineAction),
    /// Takes back the shown document's last edit (K.14).
    Undo,
    /// Makes the last undone edit again (K.14).
    Redo,
    /// Saves the edited documents now (K.15); they are saved on their own too.
    Save,
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
                    | Self::Undo
                    | Self::Redo
                    | Self::Save
            ) && !matches!(self, Self::Tab(action) if action != TabAction::ReopenClosed)
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
        id: CommandId::Outline(OutlineAction::Delete),
        name: "outline.delete",
        label: "command.outline_delete",
        group: "command.group.outline",
        // Delete while the outline has the keyboard.
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
