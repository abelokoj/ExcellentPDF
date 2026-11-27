//! The split view's grid (V.4, V.5): the open panes in two resizable rows of two resizable cells. Each pane is its
//! document panel (the page canvas, or what to do without one) with, while the window is split, a header naming its
//! file over it and a ring in the accent around the active one. A double-click on a divider shares the room alike
//! again. While files are dragged over the window, a band along each edge of the grid opens them in a new pane on
//! that side (V.2), and a drop on a pane opens them there.
//!
//! The page canvas is a window of its own over the app's, so nothing of the app's can lie on it: the edge bands take
//! room from the grid while a drag lasts instead of lying over the pages.

use std::time::{Duration, Instant};

use ecclesia_features::split::Follow;
use nsh_base::{
    MenuItemElement, ResizeHandleContext, ResizeHandleState, button, h_resizable, menu, menu_item,
    menu_separator, resizable_panel, theme, v_resizable,
};
use nsh_core::JsonValue;
use nsh_core::geometry::Points;
use nsh_core::prelude::*;

use super::{Pane, PaneAction, Panes, Shared, Side};
use crate::commands::{self, CommandId};
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
    _dialog_open: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let dragging = move || shared.file_drag.is_on();
    let inset = move || px(if dragging() { EDGE } else { 0.0 });
    let rows = v_resizable()
        .handle_class(DIVIDER)
        .handle_accessibility_label(t!("pane.resize"))
        .render_handle(divider(panes))
        .on_resize(move |sizes: &Vec<Points>| panes.resize_rows(sizes))
        .child(grid_row(panes, 0, shared, run))
        .child(grid_row(panes, 1, shared, run));
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
                .child(grid_cell(panes, row * 2, shared, run))
                .child(grid_cell(panes, row * 2 + 1, shared, run)),
        )
}

/// The cell `cell`: its pane while it has one.
fn grid_cell(
    panes: Panes,
    cell: usize,
    shared: Shared,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    resizable_panel()
        .visible(move || panes.is_open(cell))
        .size(move || panes.column_size(cell))
        .min_size(px(MIN_PANE))
        .child(pane_view(panes, panes.slots()[cell], shared, run))
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
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let cell = pane.cell;
    let active = move || panes.active_cell() == cell;
    let split = move || panes.is_split();
    // A maximized pane keeps its header, with the button that puts the panes back (V.10).
    let header = move || split() || panes.is_maximized();
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
        .child(when(header, move || pane_header(panes, pane, active)))
        .child(document_panel(panes, pane, shared, run))
}

/// A split pane's header: its shown file's name, the synchronized scrolling menu (V.8), the button that maximizes
/// the pane or puts the panes back (V.10), and a button that closes the pane. A press on it makes the pane active; a
/// double-click maximizes it or puts the panes back.
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
        .on_press(move |event: &PressEvent| {
            panes.focus_pane(pane.cell);
            if event.clicks == 2 {
                panes.run(super::PaneAction::Maximize);
            }
        })
        .child(
            text(name)
                .flex_1()
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tone),
        )
        .child(sync_menu(panes))
        .child(header_button(
            move || {
                if panes.is_maximized() {
                    t!("pane.restore")
                } else {
                    t!("pane.maximize")
                }
            },
            move || if panes.is_maximized() { "❐" } else { "□" },
            move || {
                panes.focus(pane.cell);
                panes.run(super::PaneAction::Maximize);
            },
        ))
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

/// A small button of the pane header: `glyph` drawn, `label` read out.
fn header_button(
    label: impl Fn() -> String + 'static,
    glyph: impl Fn() -> &'static str + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(move || Some(SharedString::from(label())))
        .on_press(on_press)
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(5.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(glyph)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// The synchronized scrolling menu (V.8): "Sync" in the header, in the primary colour while it is on; its items turn
/// it on and off, choose how panes follow, sync the zoom and pair pages. The pages hide while it is open, as for
/// every menu over them.
fn sync_menu(panes: Panes) -> impl IntoElement {
    let on = move || panes.sync_on();
    let item = move |action: PaneAction, checked: Option<Box<dyn Fn() -> bool>>| {
        sync_entry(CommandId::Pane(action), checked, move || {
            panes.run(action);
        })
    };
    let follows = |follow: Follow| -> Option<Box<dyn Fn() -> bool>> {
        Some(Box::new(move || panes.sync_follow() == follow))
    };
    let separator = || {
        menu_separator()
            .h(px(1.0))
            .my(px(4.0))
            .bg(move || theme().border)
    };
    menu()
        .align(Align::End)
        .offset(px(4.0))
        .on_open(move || panes.set_sync_menu_open(true))
        .on_close(move || panes.set_sync_menu_open(false))
        .popup_class("w-64 p-1 rounded-lg border border-border bg-popover shadow-lg")
        .trigger(
            view()
                .label(t!("pane.sync_menu"))
                .h(px(22.0))
                .px_2()
                .rounded(px(5.0))
                .items_center()
                .justify_center()
                .style_when(on, |s| s.bg(move || theme().sidebar_accent))
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .active(|s| s.opacity(0.7))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    text(move || t!("pane.sync"))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if on() {
                                theme().primary
                            } else {
                                theme().muted_foreground
                            }
                        }),
                ),
        )
        .child(item(PaneAction::Sync, Some(Box::new(on))))
        .child(separator())
        .child(item(
            PaneAction::SyncFollow(Follow::SamePage),
            follows(Follow::SamePage),
        ))
        .child(item(
            PaneAction::SyncFollow(Follow::SamePosition),
            follows(Follow::SamePosition),
        ))
        .child(item(
            PaneAction::SyncFollow(Follow::Pairs),
            follows(Follow::Pairs),
        ))
        .child(item(
            PaneAction::SyncZoom,
            Some(Box::new(move || panes.sync_zoom())),
        ))
        .child(separator())
        .child(item(PaneAction::PairPages, None))
        .child(item(PaneAction::ClearPagePairs, None).disabled(move || !panes.has_page_pairs()))
}

/// A sync menu item: a check mark when `checked` says so (screen readers hear it checked), its label and shortcut.
fn sync_entry(
    id: CommandId,
    checked: Option<Box<dyn Fn() -> bool>>,
    select: impl Fn() + 'static,
) -> MenuItemElement {
    let checked: Option<std::rc::Rc<dyn Fn() -> bool>> = checked.map(std::rc::Rc::from);
    let mark = checked.clone();
    let item = menu_item()
        .on_select(select)
        .accessibility_label(commands::label_with_shortcut(id))
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(32.0))
        .px_2()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .disabled_style(|s| s.opacity(0.5))
        .child(
            text(move || match &mark {
                Some(checked) if checked() => "✓",
                _ => "",
            })
            .w(px(14.0))
            .text_size(px(13.0))
            .text_color(move || theme().primary),
        )
        .child(
            text(commands::label(id))
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(commands::shortcut_label(id))
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        );
    match checked {
        Some(checked) => item.checked(move || Some(checked())),
        None => item,
    }
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
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let tabs = pane.tabs;
    // No canvas until there is something to show: cold start without a file stays light, and
    // so do the cells without a pane.
    let mounted = signal(false);
    effect(move || {
        if tabs.has_shown() && !untracked(|| mounted.get()) {
            mounted.set(true);
        }
    });
    // The find bar (F.1) sits above the pages; using it makes its pane the active one, as a press on the pages does.
    let bar = element_ref();
    bar.on_focus_in(move || panes.focus(pane.cell));
    let full_screen = shared.full_screen;
    let showing_home = move || !tabs.has_tabs() || pane.home_open.get();
    view()
        .flex_1()
        .child(when(
            move || pane.editing.recovered().is_some() && !full_screen.is_on() && !showing_home(),
            move || crate::recovery_bar::recovery_bar(pane.editing),
        ))
        .child(when(
            move || pane.autoreload.has_prompt() && !full_screen.is_on() && !showing_home(),
            move || crate::autoreload::autoreload_bar(pane.autoreload, pane.canvas),
        ))
        .child(when(
            move || {
                pane.find.is_open() && tabs.has_shown() && !full_screen.is_on() && !showing_home()
            },
            move || {
                view()
                    .element_ref(&bar)
                    .child(crate::find::find_bar(pane.find))
            },
        ))
        .child(when(mounted, move || {
            page_canvas(panes, pane, shared, showing_home)
        }))
        .child(when(
            move || !showing_home() && tabs.shown_missing().is_some(),
            move || document_missing(tabs),
        ))
        .child(when(showing_home, move || {
            crate::home::home_page(shared, tabs, pane.home_open, run)
        }))
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

/// A grid pane's pages ([`pages_view`]), with what the grid hears of them: a file opening in the active pane takes
/// the keyboard, the view leads synced panes (V.8), a link opens in the other pane (V.9), and full screen (R.19).
fn page_canvas(
    panes: Panes,
    pane: Pane,
    shared: Shared,
    showing_home: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let cell = pane.cell;
    let full_screen = shared.full_screen;
    let hidden = move || showing_home() || !panes.is_open(cell);
    pages_view(pane, shared, hidden, move |event: &NativeEvent| {
        match &*event.name {
            // A file opening in a pane beside the active one leaves the keyboard where it is.
            // Defer one tick so focus lands after the opening click's button/canvas blur settles.
            "pages" if untracked(|| panes.active_cell()) == cell => {
                focus_pages_deferred(pane.focus);
            }
            // V.8: while synced, the pane the reader moves leads the others.
            "view" => panes.on_view(cell, &event.data),
            // V.9: Ctrl+click or middle-click on a link shows where it goes in the other pane.
            "openElsewhere" => {
                if let Some(place) = super::Place::from_json(&event.data) {
                    panes.open_elsewhere(cell, place);
                }
            }
            _ => {}
        }
        full_screen.on_canvas_event(event);
    })
}

/// A pane's pages: the platform's page canvas (plan D5), showing the pane's shown tab's file, in the grid or in a
/// popped-out window (V.11). It is a window of its own, drawn over the app, so it hides while `hidden` says (a dialog
/// is open). It draws the theme's page area behind the pages itself. The view around it holds the keyboard focus for
/// it: a Tab stop, focused when a press lands on the pages. `on_event` hears each of the canvas's events before the
/// pane's parts do.
pub(crate) fn pages_view(
    pane: Pane,
    shared: Shared,
    hidden: impl Fn() -> bool + Copy + 'static,
    on_event: impl Fn(&NativeEvent) + 'static,
) -> impl IntoElement {
    let Pane {
        canvas,
        focus,
        pages,
        link_state,
        thumbs,
        tabs,
        outline,
        bookmarks,
        find,
        editing,
        annotate,
        present,
        study,
        snapshot,
        ..
    } = pane;
    let reading = shared.reading;
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
        .prop("hoverDelay", move || reading.hover_delay_prop())
        .prop("paperTexture", move || reading.paper_texture_prop())
        .prop("bookmarks", move || bookmarks.marks_prop())
        .prop("edits", move || JsonValue::from(editing.revision()))
        .prop("annotationColors", move || annotate.colors_prop())
        .prop("drawing", move || annotate.drawing_prop())
        .prop("pen", move || annotate.pen_prop())
        .prop("eraser", move || annotate.eraser_prop())
        .prop("lasso", move || annotate.lasso_prop())
        .prop("guide", move || annotate.guide_prop())
        .prop("zoomPad", move || annotate.zoom_pad_prop())
        .prop("snapshot", move || snapshot.prop())
        .prop("fingerDraws", move || annotate.finger_draws_prop())
        .prop("selected", move || annotate.selected_prop())
        .prop("margin", move || annotate.margin_prop())
        .prop("externalMarkbar", || JsonValue::from(true))
        .prop("visible", move || {
            JsonValue::from(!hidden() && tabs.has_shown())
        })
        .label(move || SharedString::from(label()))
        .on_native_event(move |event: &NativeEvent| {
            match &*event.name {
                "press" => {
                    // Defer past NSH's `Item::Hosted` pointer-down `blur_window` so clicking the
                    // PDF canvas reliably focuses `pages` and clears any stale toolbar button focus.
                    focus_pages_deferred(focus);
                    if let Some((_, pt)) =
                        ecclesia_features::annotate::notes::parse_press(&event.data)
                    {
                        present.update_pointer(pt.x, pt.y);
                    }
                }
                "hover" | "pointerMove" => {
                    if let Some((_, pt)) =
                        ecclesia_features::annotate::notes::parse_press(&event.data)
                    {
                        present.update_pointer(pt.x, pt.y);
                    }
                }
                "penContact" => {
                    if let Some(modes) = use_context::<crate::modes::Modes>() {
                        modes.pen_contact();
                    }
                }
                "snapshot" => snapshot.taken(&event.data),
                _ => {}
            }
            on_event(event);
            pages.on_canvas_event(event);
            tabs.on_canvas_event(event, canvas_ref);
            link_state.on_canvas_event(event);
            thumbs.on_canvas_event(event);
            reading.on_canvas_event(event);
            outline.on_canvas_event(event);
            bookmarks.on_canvas_event(event);
            find.on_canvas_event(event);
            annotate.on_canvas_event(event);
            copy_selection(event);
        })
        .flex_1()
        .style_when(move || hidden() || !tabs.has_shown(), |s| s.invisible());
    let canvas_host = view().flex_1().relative().child(canvas).child(
        crate::annotate::selection_popup::selection_popup_overlay(
            annotate, canvas_ref, study, bookmarks,
        ),
    );
    let pages = view()
        .flex_1()
        .focusable(true)
        .element_ref(&focus)
        .label(move || SharedString::from(label()))
        .child(crate::annotate::ink_tools::eyedropper_bar(annotate))
        .child(crate::study_ui::study_bars(study))
        .child(crate::present_ui::present_overlays(present))
        .child(canvas_host);
    // The notes beside the pages (A.11), outside the focus stop so a click on a card leaves the pages' keys alone.
    view()
        .flex_1()
        .flex_row()
        .style_when(move || hidden() || !tabs.has_shown(), |s| s.hidden())
        .child(pages)
        .child(crate::annotate::margin::margin_column(annotate))
}

/// Moves keyboard focus to a pane's pages container on the next event-loop turn, after NSH's
/// pointer-down focus/blur handling for hosted native items and chrome buttons has settled.
fn focus_pages_deferred(focus: ElementRef) {
    drop(set_timeout(Duration::ZERO, move || focus.focus()));
}

/// The canvas copied the selected text (K.13): it goes on the clipboard.
fn copy_selection(event: &NativeEvent) {
    if &*event.name == "copy"
        && let Some(text) = event.data.get("text").and_then(JsonValue::as_str)
    {
        drop(set_clipboard(text)); // the write happens whether or not the answer is awaited
    }
}
