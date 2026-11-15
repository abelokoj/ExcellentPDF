//! The split view's grid (V.4, V.5): the open panes in two resizable rows of two resizable cells. Each pane is its
//! document panel (the page canvas, or what to do without one) with, while the window is split, a header naming its
//! file over it and a ring in the accent around the active one. A double-click on a divider shares the room alike
//! again. While files are dragged over the window, a band along each edge of the grid opens them in a new pane on
//! that side (V.2), and a drop on a pane opens them there.
//!
//! The page canvas is a window of its own over the app's, so nothing of the app's can lie on it: the edge bands take
//! room from the grid while a drag lasts instead of lying over the pages.

use std::time::{Duration, Instant};

use nsh_base::{
    ResizeHandleContext, ResizeHandleState, button, h_resizable, resizable_panel, theme,
    v_resizable,
};
use nsh_core::JsonValue;
use nsh_core::geometry::Points;
use nsh_core::prelude::*;

use super::{Pane, Panes, Shared, Side};
use crate::commands::CommandId;
use crate::shell::{empty_message, primary_button};
use crate::tabs::TabState;
use crate::theme as themes;

/// How thick the edge bands a file drag shows are.
const EDGE: f32 = 40.0;

/// The smallest a pane gets along a divider's axis.
const MIN_PANE: f32 = 160.0;

/// Two presses on a divider closer together than this, with no drag between, are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// The pane header's height.
const HEADER_HEIGHT: f32 = 28.0;

/// The divider line's class: the border's colour, the accent while it is pointed at or dragged.
const DIVIDER: &str = "bg-border group-hover:bg-ring group-active:bg-ring";

/// The panes, laid out as [`Panes`]' layout says.
pub(crate) fn pane_grid(
    panes: Panes,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let dragging = move || shared.file_drag.is_on();
    let inset = move || px(if dragging() { EDGE } else { 0.0 });
    let rows = v_resizable()
        .handle_class(DIVIDER)
        .handle_accessibility_label(t!("pane.resize"))
        .render_handle(divider(panes))
        .on_resize(move |sizes: &Vec<Points>| panes.resize_rows(sizes))
        .child(grid_row(panes, 0, shared, dialog_open, run))
        .child(grid_row(panes, 1, shared, dialog_open, run));
    view()
        .flex_1()
        .pt(inset)
        .pr(inset)
        .pb(inset)
        .pl(inset)
        .child(rows)
        .child(when(dragging, move || edge_bands(panes, shared)))
}

/// Row `row` of the grid: its two cells side by side.
fn grid_row(
    panes: Panes,
    row: usize,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    resizable_panel()
        .visible(move || panes.row_open(row))
        .size(move || panes.row_size(row))
        .min_size(px(MIN_PANE))
        .child(
            h_resizable()
                .handle_class(DIVIDER)
                .handle_accessibility_label(t!("pane.resize"))
                .render_handle(divider(panes))
                .on_resize(move |sizes: &Vec<Points>| panes.resize_columns(row, sizes))
                .child(grid_cell(panes, row * 2, shared, dialog_open, run))
                .child(grid_cell(panes, row * 2 + 1, shared, dialog_open, run)),
        )
}

/// The cell `cell`: its pane while it has one.
fn grid_cell(
    panes: Panes,
    cell: usize,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    resizable_panel()
        .visible(move || panes.is_open(cell))
        .size(move || panes.column_size(cell))
        .min_size(px(MIN_PANE))
        .child(pane_view(
            panes,
            panes.slots()[cell],
            shared,
            dialog_open,
            run,
        ))
}

/// What a divider shows (its line is the group's `handle_class`), and its double-click: two presses close together
/// with no drag between share the room alike again. The handle draws it again at each change of its state, so the
/// presses are counted here.
fn divider(panes: Panes) -> impl Fn(&ResizeHandleContext) -> View + 'static {
    let pressed = stored(None::<Instant>);
    move |handle: &ResizeHandleContext| {
        match handle.state {
            ResizeHandleState::Pressed => {
                let now = Instant::now();
                let again = pressed
                    .replace(Some(now))
                    .is_some_and(|then| now.duration_since(then) < DOUBLE_CLICK);
                if again {
                    pressed.replace(None);
                    // Not while the handle is drawn: once it is.
                    drop(set_timeout(Duration::ZERO, move || {
                        panes.run(super::PaneAction::Equalize);
                    }));
                }
            }
            ResizeHandleState::Dragging => {
                pressed.replace(None);
            }
            ResizeHandleState::Idle | ResizeHandleState::Hovered => {}
        }
        view()
    }
}

/// A pane: while the window is split, a header over its document panel and the accent's ring round the active one.
/// Files dropped on it open in it.
fn pane_view(
    panes: Panes,
    pane: Pane,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let cell = pane.cell;
    let active = move || panes.active_cell() == cell;
    let split = move || panes.is_split();
    pane.focus.on_focus_in(move || panes.focus(cell));
    view()
        .flex_1()
        .style_when(split, |s| s.border_2())
        .border_color(move || {
            if active() {
                theme().primary
            } else {
                theme().border
            }
        })
        .on_file_drag_enter(move || shared.file_drag.enter())
        .on_file_drag_leave(move || shared.file_drag.leave())
        .on_file_drop(move |event: &FileDropEvent| {
            shared.file_drag.drop_done();
            let documents = crate::open::dropped_documents(&event.uris);
            if !documents.is_empty() {
                panes.focus(cell);
                pane.tabs.open_paths(documents);
            }
        })
        .child(when(split, move || pane_header(panes, pane, active)))
        .child(document_panel(panes, pane, shared, dialog_open, run))
}

/// A split pane's header: its shown file's name, and a button that closes the pane. A press on it makes the pane
/// active.
fn pane_header(
    panes: Panes,
    pane: Pane,
    active: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let name = move || pane.tabs.shown_name().unwrap_or_else(|| t!("pane.no_file"));
    let tone = move || {
        if active() {
            theme().foreground
        } else {
            theme().muted_foreground
        }
    };
    view()
        .h(px(HEADER_HEIGHT))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .pl_3()
        .pr_1()
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_b_color(move || theme().border)
        .on_press(move || panes.focus_pane(pane.cell))
        .child(
            text(name)
                .flex_1()
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tone),
        )
        .child(
            button()
                .accessibility_label(t!("pane.close"))
                .on_press(move || {
                    panes.focus(pane.cell);
                    panes.run(super::PaneAction::Close);
                })
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .active(|s| s.opacity(0.7))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    text("×")
                        .text_size(px(14.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

/// The bands along the grid's edges while files are dragged over the window: a drop on one opens them in a new pane
/// on its side.
fn edge_bands(panes: Panes, shared: Shared) -> impl IntoElement {
    let hot = signal(None::<Side>);
    view()
        .absolute()
        .inset_0()
        .child(edge_band(panes, shared, hot, Side::Up))
        .child(edge_band(panes, shared, hot, Side::Down))
        .child(edge_band(panes, shared, hot, Side::Left))
        .child(edge_band(panes, shared, hot, Side::Right))
}

/// The band on `side`: a line in its middle, in the accent while the drag is over it.
fn edge_band(
    panes: Panes,
    shared: Shared,
    hot: Signal<Option<Side>>,
    side: Side,
) -> impl IntoElement {
    let is_hot = move || hot.get() == Some(side);
    let edge = px(EDGE);
    let band = view().absolute();
    let band = match side {
        Side::Up => band.top_0().left(edge).right(edge).h(edge),
        Side::Down => band.bottom_0().left(edge).right(edge).h(edge),
        Side::Left => band.left_0().top_0().bottom_0().w(edge),
        Side::Right => band.right_0().top_0().bottom_0().w(edge),
    };
    let line = view().rounded(px(2.0)).bg(move || {
        if is_hot() {
            theme().primary
        } else {
            theme().border
        }
    });
    let line = match side {
        Side::Up | Side::Down => line.h(px(4.0)).w(px(96.0)),
        Side::Left | Side::Right => line.w(px(4.0)).h(px(96.0)),
    };
    band.items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(move || {
            if is_hot() {
                theme().accent
            } else {
                theme().background
            }
        })
        .label(SharedString::from(t!(side_label(side))))
        .on_file_drag_enter(move || {
            shared.file_drag.enter();
            hot.set(Some(side));
        })
        .on_file_drag_leave(move || {
            shared.file_drag.leave();
            hot.set(None);
        })
        .on_file_drop(move |event: &FileDropEvent| {
            shared.file_drag.drop_done();
            panes.open_in_split(side, crate::open::dropped_documents(&event.uris));
        })
        .child(line)
}

/// The localization key naming the edge band on `side`.
fn side_label(side: Side) -> &'static str {
    match side {
        Side::Up => "pane.drop_up",
        Side::Down => "pane.drop_down",
        Side::Left => "pane.drop_left",
        Side::Right => "pane.drop_right",
    }
}

/// A pane's document panel: the page canvas once a tab has been shown in it (and from then on, so switching tabs
/// only gives it another file), and while no tab is open the way to open one.
fn document_panel(
    panes: Panes,
    pane: Pane,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let tabs = pane.tabs;
    // No canvas (and no swap chain) until there is something to show: cold start without a file stays light, and
    // so do the cells without a pane.
    let mounted = signal(false);
    effect(move || {
        if tabs.has_shown() && !untracked(|| mounted.get()) {
            mounted.set(true);
        }
    });
    // The find bar (F.1) is a row above the pages, not over them: the canvas is a window of its own. Using it makes
    // its pane the active one, as a press on the pages does.
    let bar = element_ref();
    bar.on_focus_in(move || panes.focus(pane.cell));
    let full_screen = shared.full_screen;
    view()
        .flex_1()
        .child(when(
            move || pane.find.is_open() && tabs.has_shown() && !full_screen.is_on(),
            move || {
                view()
                    .element_ref(&bar)
                    .child(crate::find::find_bar(pane.find))
            },
        ))
        .child(when(mounted, move || {
            page_canvas(panes, pane, shared, dialog_open)
        }))
        .child(when(
            move || tabs.shown_missing().is_some(),
            move || document_missing(tabs),
        ))
        .child(when(
            move || !tabs.has_tabs(),
            move || crate::home::home_page(shared, tabs, run),
        ))
}

/// A document panel while its shown tab's file has gone (R.9): where it was, and "Locate…".
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

/// A pane's pages: the platform's page canvas (plan D5), showing the pane's shown tab's file. It is a window of its
/// own, drawn over the app, so it hides while a dialog is open. It draws the theme's page area behind the pages
/// itself. The view around it holds the keyboard focus for it: a Tab stop, focused when a press lands on the pages
/// and when the active pane's document opens.
fn page_canvas(
    panes: Panes,
    pane: Pane,
    shared: Shared,
    dialog_open: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let Pane {
        cell,
        canvas,
        focus,
        pages,
        link_state,
        thumbs,
        tabs,
        outline,
        bookmarks,
        find,
        ..
    } = pane;
    let Shared {
        reading,
        full_screen,
        ..
    } = shared;
    let canvas_ref = canvas;
    let label = move || {
        let name = tabs.shown_name().unwrap_or_default();
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
            match &*event.name {
                "press" => focus.focus(),
                "penContact" => {
                    if let Some(modes) = use_context::<crate::modes::Modes>() {
                        modes.pen_contact();
                    }
                }
                // A file opening in a pane beside the active one leaves the keyboard where it is.
                "pages" if untracked(|| panes.active_cell()) == cell => focus.focus(),
                _ => {}
            }
            pages.on_canvas_event(event);
            tabs.on_canvas_event(event, canvas_ref);
            link_state.on_canvas_event(event);
            thumbs.on_canvas_event(event);
            reading.on_canvas_event(event);
            full_screen.on_canvas_event(event);
            outline.on_canvas_event(event);
            bookmarks.on_canvas_event(event);
            find.on_canvas_event(event);
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
