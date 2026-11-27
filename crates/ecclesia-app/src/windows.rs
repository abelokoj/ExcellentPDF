//! Popped-out windows (V.11, plan §11.15): the shown tab moved into a window of its own, opened with NSH's
//! `open_window`. The window holds one pane of its own: a tab strip over the pages, with the same page canvas, so the
//! tab keeps its page and view, and a file open in the main window too shares its document (the registry of
//! `ecclesia_core::shared`). Each window's `WindowRef` stays in the registry ([`Windows`]) while it is open.
//!
//! "Move tab to its own window or back" docks a window again: its tabs go to the main window's active pane, as they
//! do when the window is closed, so closing one loses nothing. The session keeps a window's tabs with the active
//! pane's, where docking puts them.

use nsh_base::{button, theme};
use nsh_core::JsonValue;
use nsh_core::desktop::WindowMode;
#[cfg(target_os = "windows")]
use nsh_core::element::desktop::quit;
use nsh_core::element::desktop::{OpenChange, WindowRef, current_window, open_window};
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::panes::{Pane, PaneAction, Panes, Place, Shared, pages_view};
use crate::tabs::{self, Tabs};

/// A new window's size, before the platform keeps one of its own.
const DEFAULT_SIZE: (f32, f32) = (900.0, 720.0);

/// One popped-out window.
#[derive(Clone)]
struct Popped {
    /// The app's number for it, also in its NSH id.
    id: u64,
    window: WindowRef,
    pane: Pane,
}

impl PartialEq for Popped {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// The window registry: the popped-out windows open now.
#[derive(Clone, Copy)]
pub(crate) struct Windows {
    open: Signal<Vec<Popped>>,
    /// The last window's number.
    last: Stored<u64>,
}

impl Windows {
    /// No window popped out.
    pub(crate) fn new() -> Self {
        Self {
            open: signal(Vec::new()),
            last: stored(0),
        }
    }

    /// Every window's tabs as they are now (tracked): what the session keeps with the active pane's.
    pub(crate) fn tabs(self) -> Vec<Tabs> {
        self.open.with(|open| {
            open.iter()
                .map(|popped| popped.pane.tabs.current())
                .collect()
        })
    }

    /// Docks window `id`: its tabs go to `panes`' active pane, and it closes. Nothing when it is gone already.
    fn dock(self, id: u64, panes: Panes) {
        let Some(popped) = self.take(id) else {
            return;
        };
        panes.dock(popped.pane.tabs.take_all());
        popped.window.close();
    }

    /// Docks every window (W.2: a new workspace from every tab).
    pub(crate) fn dock_all(self, panes: Panes) {
        let ids: Vec<u64> = self
            .open
            .with_untracked(|open| open.iter().map(|popped| popped.id).collect());
        for id in ids {
            self.dock(id, panes);
        }
    }

    /// Closes every window without docking: another workspace takes the window's place (W.1), and the session
    /// saved their tabs with the workspace shown until now.
    pub(crate) fn close_all(self) {
        let mut all = Vec::new();
        self.open.update(|open| all = std::mem::take(open));
        for popped in all {
            // Taking the tabs out saves their edits, as closing a pane's does (K.15).
            popped.pane.tabs.take_all();
            popped.window.close();
        }
    }

    /// Takes window `id` out of the registry.
    fn take(self, id: u64) -> Option<Popped> {
        let at = self
            .open
            .with_untracked(|open| open.iter().position(|popped| popped.id == id))?;
        let mut taken = None;
        self.open.update(|open| taken = Some(open.remove(at)));
        taken
    }
}

/// Moves the active pane's shown tab into a window of its own, at its page and in its view.
pub(crate) fn pop_out(panes: Panes, shared: Shared) {
    let Some(tab) = panes.active().tabs.take_shown() else {
        return;
    };
    let windows = panes.windows();
    let pane = panes.new_pane();
    pane.tabs.adopt(vec![tab]);
    let id = windows.last.with(|last| {
        *last += 1;
        *last
    });
    let opened = open_window(
        nsh_core::element::desktop::window()
            .id(format!("popped-{id}"))
            .title(move || pane.tabs.shown_name().unwrap_or_else(|| t!("app.name")))
            .default_size(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)))
            // However it closes, its tabs go back.
            .on_open_change(move |_: &OpenChange| windows.dock(id, panes))
            .content(move || popped_view(panes, pane, shared, id)),
    );
    match opened {
        Some(window) => windows
            .open
            .update(|open| open.push(Popped { id, window, pane })),
        // No windows of the app's own here: the tab goes back where it was.
        None => panes.dock(pane.tabs.take_all()),
    }
}

/// What a popped-out window shows: its tabs in a bar, the pages, and the dialogs its pages ask for.
fn popped_view(panes: Panes, pane: Pane, shared: Shared, id: u64) -> impl IntoElement {
    let windows = panes.windows();
    let window = stored(current_window());
    let is_full_screen = move || {
        window
            .with(|w| w.state())
            .is_some_and(|state| state.mode == WindowMode::FullScreen)
    };
    let toggle_full_screen = move || {
        let next = if untracked(is_full_screen) {
            WindowMode::Normal
        } else {
            WindowMode::FullScreen
        };
        window.with(|w| w.set_mode(next));
    };
    let run = move |command: CommandId| match command {
        CommandId::Tab(action) => pane.tabs.run(action),
        CommandId::Canvas(name) => {
            if !pane.annotate.selection_command(name) {
                pane.canvas.command(name, JsonValue::Null);
            }
        }
        CommandId::ToggleFullScreen => toggle_full_screen(),
        CommandId::BlockSelect => pane.annotate.toggle_block_select(),
        CommandId::LeaveModes => {
            if untracked(|| pane.find.is_open()) {
                pane.find.run(crate::find::FindAction::Close);
                return;
            }
            pane.annotate.clear_selection();
            pane.annotate.clear_block_select();
            pane.canvas.command("leaveModes", JsonValue::Null);
            if untracked(is_full_screen) {
                window.with(|w| w.set_mode(WindowMode::Normal));
            }
        }
        CommandId::CutAnnotations => pane.annotate.cut(),
        CommandId::PasteAnnotations => pane.annotate.paste(pane.pages.current()),
        CommandId::DeleteAnnotations => pane.annotate.delete_selected(),
        CommandId::OpenFile => tabs::pick_documents(move |paths| pane.tabs.open_paths(paths)),
        CommandId::Undo => pane.editing.undo(),
        CommandId::Redo => pane.editing.redo(),
        CommandId::Save => pane.editing.save_now(),
        CommandId::Pane(PaneAction::PopOut) => windows.dock(id, panes),
        // The rest are the main window's: its dialogs, its sidebar and its panes.
        _ => {}
    };
    let page_key_state = computed(move || {
        if pane.covers_pages() {
            return commands::PageKeyState::Inactive;
        }
        if pane.tabs.has_shown() && !pane.home_open.get() {
            commands::PageKeyState::Active
        } else if is_full_screen() {
            commands::PageKeyState::FullScreenEscapeOnly
        } else {
            commands::PageKeyState::Inactive
        }
    });
    // A window's keys fire only in it, so the main window's are bound here again, to this pane.
    let live = panes.live();
    effect(move || {
        live.read(|_| ());
        commands::bind_window_keys(run);
    });
    effect(move || {
        live.read(|_| ());
        commands::bind_page_keys(page_key_state.get(), run);
    });
    // The window closes with its last tab.
    effect(move || {
        if !pane.tabs.has_tabs() {
            untracked(|| windows.dock(id, panes));
        }
    });
    let pages = pages_view(
        pane,
        shared,
        move || pane.covers_pages(),
        move |event: &NativeEvent| {
            // V.9's Ctrl+click goes where the link goes here: this window has no other pane.
            if &*event.name == "openElsewhere"
                && let Some(place) = Place::from_json(&event.data)
            {
                let (name, data) = place.command();
                pane.canvas.command(name, data);
            }
        },
    );
    let root = view()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .direction(move || panes.live().read(crate::l10n::direction))
        .bg(move || theme().background)
        .child(
            view()
                .w_full()
                .min_w(px(0.0))
                .h(move || px(panes.live().read(tabs::title_bar_height_px)))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .overflow_hidden()
                .bg(move || theme().sidebar)
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .flex_row()
                        .items_center()
                        .overflow_hidden()
                        .child(tabs::tab_strip(
                            pane.tabs,
                            panes,
                            pane.home_open,
                            pane.editing,
                            shared.workspaces,
                            run,
                        )),
                )
                .child(dock_button(move || windows.dock(id, panes))),
        )
        .child(
            view()
                .flex_1()
                .w_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden()
                .child(pages),
        )
        .child(when(
            move || pane.tabs.editing().is_some(),
            move || tabs::group_dialog(pane.tabs),
        ))
        .child(when(
            move || pane.tabs.properties_tab().is_some(),
            move || tabs::properties_dialog(pane.tabs),
        ))
        .child(when(
            move || pane.link_state.is_asking(),
            move || crate::links::link_dialog(pane.link_state),
        ))
        .child(when(
            move || pane.annotate.is_asking(),
            move || crate::annotate::text_edit_dialog(pane.annotate),
        ))
        .child(when(
            move || pane.annotate.is_picking(),
            move || crate::annotate::color_dialog(pane.annotate),
        ));
    #[cfg(target_os = "windows")]
    let root = root.child(
        native(crate::WINDOW_BRIDGE)
            .w(px(0.0))
            .h(px(0.0))
            .pointer_events(PointerEvents::None)
            .prop("pageKeysActive", move || {
                let active =
                    page_key_state.get() == commands::PageKeyState::Active && !pane.find.is_open();
                JsonValue::from(active)
            })
            .on_native_event(move |event: &NativeEvent| match &*event.name {
                "osFileDrop" => {
                    let paths = crate::open::paths_from_event(&event.data);
                    let documents = crate::open::dropped_paths(&paths);
                    if !documents.is_empty() {
                        pane.tabs.open_paths(documents);
                        pane.home_open.set(false);
                    }
                }
                "osShortcut" => {
                    if let Some(shortcut) = commands::OsShortcut::from_event(&event.data) {
                        match shortcut {
                            commands::OsShortcut::Quit => {
                                pane.editing.save_now();
                                for slot in panes.slots() {
                                    slot.editing.save_now();
                                }
                                quit();
                            }
                            other if other.is_page_scoped() => {
                                if untracked(|| page_key_state.get())
                                    == commands::PageKeyState::Active
                                    && !untracked(|| pane.find.is_open())
                                    && let Some(cmd) = other.command()
                                {
                                    run(cmd);
                                }
                            }
                            other => {
                                if let Some(cmd) = other.command() {
                                    run(cmd);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }),
    );
    root
}

/// The button that docks the window, named with its shortcut.
fn dock_button(dock: impl Fn() + 'static) -> impl IntoElement {
    let command = CommandId::Pane(PaneAction::PopOut);
    button()
        .accessibility_label(move || {
            let keys = commands::shortcut_label(command);
            Some(SharedString::from(if keys.is_empty() {
                t!("window.dock")
            } else {
                t!("find.with_keys", name = t!("window.dock"), keys = keys)
            }))
        })
        .on_press(dock)
        .h(px(28.0))
        .px_2()
        .flex_shrink_0()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .gap_1()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text("⇱")
                .text_size(px(14.0))
                .text_color(move || theme().sidebar_foreground),
        )
        .child(
            text(move || t!("window.dock_short"))
                .text_size(px(12.0))
                .text_color(move || theme().sidebar_foreground),
        )
}
