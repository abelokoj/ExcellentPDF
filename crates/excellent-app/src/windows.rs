//! Popped-out windows (V.11, plan §11.15): the shown tab moved into a window of its own, opened with NSH's
//! `open_window`. The window holds one pane of its own: a tab strip over the pages, with the same page canvas, so the
//! tab keeps its page and view, and a file open in the main window too shares its document (the registry of
//! `excellent_core::shared`). Each window's `WindowRef` stays in the registry ([`Windows`]) while it is open.
//!
//! "Move tab to its own window or back" docks a window again: its tabs go to the main window's active pane, as they
//! do when the window is closed, so closing one loses nothing. The session keeps a window's tabs with the active
//! pane's, where docking puts them.

use nsh_base::{button, theme};
use nsh_core::JsonValue;
use nsh_core::element::desktop::{OpenChange, WindowRef, open_window};
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::panes::{Pane, PaneAction, Panes, Place, Shared, pages_view};
use crate::tabs::{self, Tabs};

/// A new window's size, before the platform keeps one of its own.
const DEFAULT_SIZE: (f32, f32) = (900.0, 720.0);

/// The height of the bar that holds the window's tabs.
const BAR_HEIGHT: f32 = 40.0;

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
    let on_pages = signal(false);
    pane.focus.on_focus_in(move || on_pages.set(true));
    pane.focus.on_focus_out(move || on_pages.set(false));
    let run = move |command: CommandId| match command {
        CommandId::Tab(action) => pane.tabs.run(action),
        CommandId::Canvas(name) => pane.canvas.command(name, JsonValue::Null),
        CommandId::LeaveModes => pane.canvas.command("leaveModes", JsonValue::Null),
        CommandId::OpenFile => tabs::pick_documents(move |paths| pane.tabs.open_paths(paths)),
        CommandId::Undo => pane.editing.undo(),
        CommandId::Redo => pane.editing.redo(),
        CommandId::Save => pane.editing.save_now(),
        CommandId::Pane(PaneAction::PopOut) => windows.dock(id, panes),
        // The rest are the main window's: its dialogs, its sidebar and its panes.
        _ => {}
    };
    // A window's keys fire only in it, so the main window's are bound here again, to this pane.
    commands::bind_keys(run, move || on_pages.get());
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
    view()
        .flex_1()
        .direction(move || panes.live().read(crate::l10n::direction))
        .bg(move || theme().background)
        .child(
            view()
                .h(px(BAR_HEIGHT))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .bg(move || theme().sidebar)
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(tabs::tab_strip(pane.tabs, run))
                .child(dock_button(move || windows.dock(id, panes))),
        )
        .child(view().flex_1().child(pages))
        .child(when(
            move || pane.tabs.editing().is_some(),
            move || tabs::group_dialog(pane.tabs),
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
        ))
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
