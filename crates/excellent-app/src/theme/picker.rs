//! The theme and appearance pickers (K.17). The highlighted row is applied to the whole window at once, so moving
//! through the list (arrow keys, Page Up and Down, the pointer, typing to filter) previews each choice live. Enter or
//! a click keeps it; Escape, or a press outside the picker, goes back to the choice in effect.

use nsh_base::{Command, CommandApi, CommandEntry, CommandItem, command, theme};
use nsh_core::element::{next_frame, spawn};
use nsh_core::platform::Appearance;
use nsh_core::prelude::*;

use super::{Contrast, FAMILIES, Family, ThemeChoice, apply, save, theme_of};
use crate::list_dialog::{list_dialog, style};

/// The theme picker: every family, then every family in high contrast.
pub(crate) fn theme_picker(open: Signal<bool>, current: Signal<ThemeChoice>) -> impl IntoElement {
    live_picker(
        open,
        current,
        Picker {
            title: "theme.picker.title",
            placeholder: "theme.picker.placeholder",
            items: theme_items,
            choice_of: theme_choice,
            id_of: theme_id,
            swatch: true,
        },
    )
}

/// The appearance picker: the system's, light or dark.
pub(crate) fn appearance_picker(
    open: Signal<bool>,
    current: Signal<ThemeChoice>,
) -> impl IntoElement {
    live_picker(
        open,
        current,
        Picker {
            title: "appearance.picker.title",
            placeholder: "appearance.picker.placeholder",
            items: appearance_items,
            choice_of: appearance_choice,
            id_of: appearance_id,
            swatch: false,
        },
    )
}

/// What tells one picker from the other.
#[derive(Clone, Copy)]
struct Picker {
    /// Localization keys.
    title: &'static str,
    placeholder: &'static str,
    /// Its rows, the one in effect checked.
    items: fn(ThemeChoice) -> Vec<CommandItem>,
    /// The choice a row's id makes of the one in effect.
    choice_of: fn(&str, ThemeChoice) -> Option<ThemeChoice>,
    /// The row of the choice in effect.
    id_of: fn(ThemeChoice) -> String,
    /// Rows show their theme's colours.
    swatch: bool,
}

fn live_picker(
    open: Signal<bool>,
    current: Signal<ThemeChoice>,
    picker: Picker,
) -> impl IntoElement {
    let list = component_ref::<Command>();
    // The list highlights its first row as it opens, before this picker hears of it; once the list is shown the
    // highlight moves to the choice in effect, and only from then on does a highlight preview its row. It also takes
    // the keyboard focus then: a dialog that just closed (the palette that opened this picker) gives the focus back
    // to what had it before, after this list's `auto_focus`.
    let placed = signal(false);
    spawn(async move {
        next_frame().await;
        if let Some(api) = list.api::<CommandApi>() {
            api.highlight((picker.id_of)(current.get_untracked()));
            api.focus();
        }
        placed.set(true);
    });
    let revert = move || {
        apply(current.get_untracked());
        open.set(false);
    };
    let preview = move |id: &Option<SharedString>| {
        if !placed.get_untracked() {
            return;
        }
        let now = current.get_untracked();
        let shown = id.as_deref().and_then(|id| (picker.choice_of)(id, now));
        apply(shown.unwrap_or(now));
    };
    let choose = move |id: &SharedString| {
        if let Some(choice) = (picker.choice_of)(id, current.get_untracked()) {
            current.set(choice);
            apply(choice);
            save(choice);
        }
        open.set(false);
    };
    list_dialog(
        t!(picker.title),
        revert,
        style(
            command()
                .component_ref(&list)
                .items(move || (picker.items)(current.get()))
                .placeholder(t!(picker.placeholder))
                .accessibility_label(t!(picker.title))
                .auto_focus(true)
                .on_highlight_change(preview)
                .on_select(choose)
                .on_cancel(revert)
                .render_item(move |item: &CommandEntry| row(item, picker.swatch)),
        ),
    )
}

/// A row: the theme's swatch (theme rows), its name, and a dot on the one in effect.
fn row(item: &CommandEntry, swatch: bool) -> impl IntoElement + use<> {
    let theme_shown = swatch
        .then(|| theme_choice(&item.id, ThemeChoice::default()))
        .flatten();
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap_3()
        .children(theme_shown.map(|choice| swatch_of(choice.family, choice.contrast)))
        .child(text(item.label.clone()).flex_1())
        .child(
            view()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(move || theme().primary)
                .opacity(if item.checked { 1.0 } else { 0.0 }),
        )
}

/// A small window in `family`'s colours, light or dark as the window is: its chrome strip, its page and its accent.
fn swatch_of(family: &'static Family, contrast: Contrast) -> impl IntoElement {
    let colours = theme_of(family, contrast);
    view()
        .w(px(32.0))
        .h(px(22.0))
        .flex_shrink_0()
        .flex_row()
        .rounded(px(5.0))
        .overflow_hidden()
        .border_1()
        .border_color(colours.border)
        .bg(colours.background)
        .child(view().w(px(10.0)).h_full().bg(colours.sidebar))
        .child(
            view().flex_1().items_center().justify_center().child(
                view()
                    .w(px(9.0))
                    .h(px(9.0))
                    .rounded(px(4.5))
                    .bg(colours.primary),
            ),
        )
}

/// A theme row's id: the family's, with `:high` for high contrast.
pub(super) fn theme_id(choice: ThemeChoice) -> String {
    match choice.contrast {
        Contrast::Standard => choice.family.id.to_owned(),
        Contrast::High => format!("{}:high", choice.family.id),
    }
}

pub(super) fn theme_choice(id: &str, now: ThemeChoice) -> Option<ThemeChoice> {
    let (family, contrast) = match id.split_once(':') {
        Some((family, "high")) => (family, Contrast::High),
        None => (id, Contrast::Standard),
        Some(_) => return None,
    };
    Some(ThemeChoice {
        family: Family::find(family)?,
        contrast,
        ..now
    })
}

fn theme_items(now: ThemeChoice) -> Vec<CommandItem> {
    [
        (Contrast::Standard, "theme.picker.group.standard"),
        (Contrast::High, "theme.picker.group.contrast"),
    ]
    .into_iter()
    .flat_map(|(contrast, group)| {
        FAMILIES.iter().map(move |family| {
            let choice = ThemeChoice {
                family,
                contrast,
                ..now
            };
            CommandItem::new(theme_id(choice), family.name())
                .keywords([family.id])
                .group(t!(group))
                .checked(choice == now)
        })
    })
    .collect()
}

/// The appearances, in the order the picker lists them, with their ids and localization keys.
const APPEARANCES: [(Appearance, &str, &str); 3] = [
    (Appearance::System, "system", "appearance.system"),
    (Appearance::Light, "light", "appearance.light"),
    (Appearance::Dark, "dark", "appearance.dark"),
];

pub(super) fn appearance_id(choice: ThemeChoice) -> String {
    APPEARANCES
        .iter()
        .find(|(appearance, ..)| *appearance == choice.appearance)
        .map(|(_, id, _)| (*id).to_owned())
        .unwrap_or_default()
}

/// The appearance `choice` uses, in the app's language.
pub(crate) fn appearance_name(choice: ThemeChoice) -> String {
    APPEARANCES
        .iter()
        .find(|(appearance, ..)| *appearance == choice.appearance)
        .map(|&(.., label)| t!(label))
        .unwrap_or_default()
}

pub(super) fn appearance_choice(id: &str, now: ThemeChoice) -> Option<ThemeChoice> {
    let (appearance, ..) = APPEARANCES.iter().find(|(_, row, _)| *row == id)?;
    Some(ThemeChoice {
        appearance: *appearance,
        ..now
    })
}

fn appearance_items(now: ThemeChoice) -> Vec<CommandItem> {
    APPEARANCES
        .iter()
        .map(|&(appearance, id, label)| {
            CommandItem::new(id, t!(label)).checked(appearance == now.appearance)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_round_trip() {
        // The rows' labels need the UI's translations; their ids don't.
        let now = ThemeChoice {
            appearance: Appearance::Dark,
            ..ThemeChoice::default()
        };
        for family in &FAMILIES {
            for contrast in [Contrast::Standard, Contrast::High] {
                let choice = ThemeChoice {
                    family,
                    contrast,
                    ..now
                };
                assert_eq!(theme_choice(&theme_id(choice), now), Some(choice));
            }
        }
        assert!(theme_choice("lagoon:sepia", now).is_none());
        assert!(theme_choice("plaid", now).is_none());
    }

    #[test]
    fn appearance_ids_round_trip() {
        let now = ThemeChoice {
            contrast: Contrast::High,
            ..ThemeChoice::default()
        };
        for (appearance, id, _) in APPEARANCES {
            let choice = appearance_choice(id, now).expect("every row is a choice");
            assert_eq!(choice.appearance, appearance);
            assert_eq!(appearance_id(choice), id);
            assert_eq!((choice.family, choice.contrast), (now.family, now.contrast));
        }
        assert!(appearance_choice("sepia", now).is_none());
    }
}
