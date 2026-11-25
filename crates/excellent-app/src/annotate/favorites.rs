//! Favourite pens (H.7), the app's side: the bar of saved pens lives in `settings.toml` under `[ink]` (so it is the
//! same in every pane and window, and travels with the settings, Y.2), the pen in hand goes to the page canvas as its
//! `pen` prop, and the bar leads the Write mode's toolbar: press a pen to take it, drag it along the bar to reorder,
//! right-click to edit or remove it, `+` to save the pen in hand as a new one. The bar exports to and imports from a
//! JSON file. The model is `excellent-features`' `ink::favorites`.

use excellent_features::annotate::colors::{Tool, hex};
use excellent_features::ink::favorites::{Favorite, Favorites, ImportError, Response};
use excellent_ink::pens::PenKind;
use nsh_base::{button, context_menu, theme, toggle};
use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::caption;
use crate::commands::{self, CommandId};
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::outline::menu_entry;
use crate::settings::{Live, Settings, write_atomically};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// The settings key the bar lives under.
const KEY: &str = "ink";

/// A pen's place on the bar, in points: a drag this far along moves it one place.
const SLOT: f32 = 34.0;

/// The widths the edit dialog offers, in points.
const SIZES: [f32; 7] = [0.5, 1.0, 1.6, 2.4, 4.0, 6.0, 10.0];

fn read(settings: &Settings) -> Favorites {
    settings
        .value(KEY)
        .and_then(|value| serde_json::to_value(value).ok())
        .map(|json| Favorites::from_json(&json))
        .unwrap_or_default()
}

fn write(settings: &mut Settings, favorites: &Favorites) {
    let value = (*favorites != Favorites::default())
        .then(|| toml::Value::try_from(favorites.to_json()).ok())
        .flatten();
    settings.set_value(KEY, value);
}

/// The bar as the settings hold it.
pub(crate) fn favorites(live: Live) -> Favorites {
    live.read(read)
}

/// Changes the bar by `f`, kept in the settings.
fn change(live: Live, f: impl FnOnce(&mut Favorites)) {
    live.change(|settings| {
        let mut bar = read(settings);
        f(&mut bar);
        write(settings, &bar);
    });
}

impl AnnotateState {
    /// The canvas's `pen` prop, the pen in hand: `{kind, size, color, pressure}` (H.2, H.7).
    pub(crate) fn pen_prop(self) -> JsonValue {
        let pen = favorites(self.live).in_hand();
        serde_json::json!({
            "kind": pen.kind.to_string(),
            "size": pen.size,
            "color": format!("#{:06x}", pen.color),
            "pressure": pen.response.exponent(),
        })
    }

    /// The canvas's `fingerDraws` prop (H.8): whether a finger that isn't a resting hand draws ink.
    pub(crate) fn finger_draws_prop(self) -> JsonValue {
        JsonValue::from(
            self.live
                .read(|s| s.flag(crate::modes::FINGER_DRAWS).unwrap_or(false)),
        )
    }

    /// The pen in hand's kind (H.2's toolbar shows it pressed).
    pub(super) fn kind_in_hand(self) -> PenKind {
        favorites(self.live).in_hand().kind
    }

    /// Takes the first saved pen of `kind` in hand, saving one as it comes when there is none (H.2's commands),
    /// putting any eraser or lasso away.
    pub(crate) fn choose_pen(self, kind: PenKind) {
        change(self.live, |bar| {
            bar.choose_kind(kind);
        });
        self.put_tools_away();
    }

    /// Takes saved pen `index` in hand.
    pub(super) fn choose_favorite(self, index: usize) {
        change(self.live, |bar| bar.choose(index));
        self.put_tools_away();
    }

    /// Saves the pen in hand again at the end of the bar, and opens it to edit.
    pub(crate) fn save_pen(self) {
        let mut added = None;
        change(self.live, |bar| added = bar.add(bar.in_hand()));
        match added {
            Some(index) => self.editing_pen.set(Some(index)),
            None => show_message(t!("favorites.full"), t!("favorites.full_detail")),
        }
    }

    /// Removes the pen in hand from the bar; the last one stays.
    pub(crate) fn remove_pen(self) {
        change(self.live, |bar| bar.remove(bar.chosen()));
    }

    /// Opens the pen in hand to edit.
    pub(crate) fn edit_pen(self) {
        self.editing_pen.set(Some(favorites(self.live).chosen()));
    }

    /// Asks where to save the bar, and writes it there as JSON.
    pub(crate) fn export_pens(self) {
        let text = favorites(self.live).export();
        let dialog = SaveDialog::new()
            .file_name("ExcellentPDF pens.json".to_owned())
            .filter(FileFilter::new(t!("favorites.filter"), ["json"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, text) {
                log::warn!("favorites: export: {err}");
                show_message(t!("favorites.export_failed"), err.to_string());
            }
        });
    }

    /// Asks for a file of pens and replaces the bar with them.
    pub(crate) fn import_pens(self) {
        let live = self.live;
        let dialog = OpenDialog::files()
            .title(t!("favorites.pick"))
            .filter(FileFilter::new(t!("favorites.filter"), ["json"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Some(path) = answer
                .await
                .ok()
                .flatten()
                .and_then(|p| p.into_iter().next())
            else {
                return;
            };
            let read = std::fs::read_to_string(&path)
                .map_err(|err| err.to_string())
                .and_then(|text| Favorites::import(&text).map_err(|err| import_problem(&err)));
            match read {
                Ok(bar) => live.change(|settings| write(settings, &bar)),
                Err(problem) => show_message(t!("favorites.import_failed"), problem),
            }
        });
    }

    fn put_tools_away(self) {
        self.eraser.set(None);
        self.lasso.set(false);
    }
}

/// Why an import failed, in the reader's words.
fn import_problem(err: &ImportError) -> String {
    match err {
        ImportError::NotJson | ImportError::NotPens => t!("favorites.not_pens"),
        ImportError::Newer(_) => t!("favorites.newer"),
        ImportError::Empty => t!("favorites.empty"),
    }
}

/// A pen's name for screen readers and its menu: its kind, colour and width.
fn describe(pen: Favorite) -> String {
    t!(
        "favorites.pen",
        kind = commands::label(CommandId::Pen(pen.kind)),
        color = hex(pen.color),
        size = format!("{:.1}", pen.size)
    )
}

/// The bar: each saved pen, the one in hand pressed, then `+`. Shown at the start of the Write mode's toolbar.
pub(crate) fn favorites_bar(state: AnnotateState) -> impl IntoElement {
    dynamic(move || {
        let bar = favorites(state.live);
        let chosen = bar.chosen();
        let on_pen = state.eraser.get().is_none() && !state.lasso.get();
        untracked(move || {
            bar.pens()
                .iter()
                .enumerate()
                .fold(
                    view().flex_row().items_center().gap_1(),
                    |row, (index, pen)| {
                        row.child(pen_button(state, index, *pen, on_pen && index == chosen))
                    },
                )
                .child(
                    button()
                        .accessibility_label(t!("command.save_pen"))
                        .on_press(move || state.save_pen())
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(6.0))
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(move || theme().accent))
                        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                        .child(
                            text("+")
                                .text_size(px(16.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .into_element()
        })
    })
}

/// One saved pen: a dot in its colour, as wide as it writes (within reason); pressed when in hand. A drag along the
/// bar moves it; the context menu edits or removes it.
fn pen_button(
    state: AnnotateState,
    index: usize,
    pen: Favorite,
    in_hand: bool,
) -> impl IntoElement {
    let offset = animated_value(0.0);
    let dot = (pen.size * 2.0).clamp(6.0, 18.0);
    let content = toggle()
        .pressed(move || in_hand)
        .accessibility_label(describe(pen))
        .on_change(move |_: &bool| state.choose_favorite(index))
        .w(px(30.0))
        .h(px(30.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .pressed_style(|s| {
            s.bg(move || theme().muted)
                .border_2()
                .border_color(move || theme().primary)
        })
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            view()
                .w(px(dot))
                .h(px(dot))
                .rounded_full()
                .bg(Hsla::from(rgb(pen.color)))
                .border_1()
                .border_color(move || theme().border),
        );
    // The drag follows the pointer natively; where it ends, the pen moves that many places.
    let content = view()
        .pan_values(Some(&offset), None)
        .bind_translate_x(&offset)
        .on_pan_end(move |event: &PanEvent| {
            let places = (event.translation.x.0 / SLOT).round() as isize;
            offset.set(0.0);
            if places != 0 {
                let to = index.saturating_add_signed(places);
                change(state.live, |bar| bar.move_to(index, to));
            }
        })
        .child(content);
    context_menu()
        .trigger(content)
        .popup_class("w-48 p-1 rounded-lg border border-border bg-popover shadow-lg")
        .child(menu_entry(CommandId::EditPen, move || {
            state.editing_pen.set(Some(index))
        }))
        .child(menu_entry(CommandId::RemovePen, move || {
            change(state.live, |bar| bar.remove(index));
        }))
}

/// The dialog that edits saved pen `index`: its kind, colour, width and pressure response. Each choice shows on the
/// bar at once; Escape or Done closes it.
pub(crate) fn pen_dialog(state: AnnotateState) -> impl IntoElement {
    when(
        move || state.editing_pen.get().is_some(),
        move || {
            let index = untracked(|| state.editing_pen.get()).unwrap_or_default();
            let close = move || state.editing_pen.set(None);
            let pen = move || favorites(state.live).pens().get(index).copied();
            let set = move |edit: &dyn Fn(&mut Favorite)| {
                change(state.live, |bar| {
                    if let Some(mut pen) = bar.pens().get(index).copied() {
                        edit(&mut pen);
                        bar.set(index, pen);
                        bar.choose(index);
                    }
                });
            };
            let kinds = PenKind::ALL.into_iter().fold(segmented(), |group, kind| {
                group.child(segment_of(
                    move || commands::label(CommandId::Pen(kind)),
                    move || pen().is_some_and(|p| p.kind == kind),
                    move || set(&|p| *p = Favorite { kind, ..*p }),
                ))
            });
            let colors = super::colors::palette(state.live)
                .swatches(Tool::Shape)
                .into_iter()
                .chain(super::colors::palette(state.live).swatches(Tool::Highlight))
                .fold(view().flex_row().flex_wrap().gap_2(), |row, color| {
                    row.child(
                        toggle()
                            .pressed(move || pen().is_some_and(|p| p.color == color))
                            .accessibility_label(t!("inspector.swatch", color = hex(color)))
                            .on_change(move |_: &bool| set(&|p| p.color = color))
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded_full()
                            .bg(Hsla::from(rgb(color)))
                            .border_2()
                            .border_color(move || theme().border)
                            .pressed_style(|s| s.border_color(move || theme().foreground))
                            .focus_visible(|s| s.border_color(move || theme().ring)),
                    )
                });
            let sizes = SIZES.into_iter().fold(segmented(), |group, size| {
                group.child(segment_of(
                    move || format!("{size}"),
                    move || pen().is_some_and(|p| (p.size - size).abs() < 0.01),
                    move || set(&|p| p.size = size),
                ))
            });
            let responses = Response::ALL
                .into_iter()
                .fold(segmented(), |group, response| {
                    group.child(segment_of(
                        move || {
                            t!(match response {
                                Response::Light => "favorites.light",
                                Response::Normal => "favorites.normal",
                                Response::Firm => "favorites.firm",
                            })
                        },
                        move || pen().is_some_and(|p| p.response == response),
                        move || set(&|p| p.response = response),
                    ))
                });
            let content = view()
                .gap_3()
                .p_4()
                .w(px(460.0))
                .child(
                    text(move || t!("favorites.title"))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(caption("favorites.kind"))
                .child(kinds)
                .child(caption("inspector.color"))
                .child(colors)
                .child(caption("favorites.size"))
                .child(sizes)
                .child(caption("favorites.pressure"))
                .child(responses)
                .child(
                    view()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(action_button(
                            "favorites.remove",
                            Tone::Destructive,
                            move || {
                                change(state.live, |bar| bar.remove(index));
                                close();
                            },
                        ))
                        .child(action_button("favorites.done", Tone::Plain, close)),
                );
            list_dialog(t!("favorites.title"), close, card(content))
        },
    )
}
