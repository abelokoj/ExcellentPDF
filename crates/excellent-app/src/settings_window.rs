//! The settings window (K.16 step 3, plan §9.2): every setting the app has, in groups, each beside the control that
//! changes it. A change takes effect and is kept at once, so there is no Save or Cancel; Escape closes it. Shortcuts
//! are listed as they are bound, with what could not be bound; they are edited in `settings.toml`, which it opens.
//!
//! Built on the quick dialogs' card ([`crate::list_dialog`]), wider, with rows of a label and its hint on the left and
//! the control on the right.

use excellent_features::read::autoscroll::{DEFAULT_SPEED, SPEEDS};
use excellent_features::read::links::DEFAULT_PROTOCOLS;
use excellent_features::read::ruler::{self, MAX_HEIGHT, MIN_HEIGHT};
use nsh_base::{SliderValue, button, input, number_input, slider, switch, theme, toggle};
use nsh_core::prelude::*;

use crate::commands::{COMMANDS, CommandId};
use crate::keymap::{self, Scope};
use crate::links::PROTOCOLS_KEY;
use crate::list_dialog::{card, list_dialog};
use crate::reading::{
    AUTO_SCROLL_SPEED, DocumentColors, FOCUS_SCOPE, READING_INK, READING_PAPER, RULER_COLOR,
    RULER_HEIGHT, RULER_INVERT, RULER_OPACITY, Reading, hex, parse_hex,
};
use crate::reading_theme_bar;
use crate::settings::{self, Live};
use crate::tabs::{Cycle, TAB_CYCLE_ORDER};
use crate::theme::{self as themes, ThemeChoice};
use excellent_core::darkmode::{Palette, Rgb};

/// The ruler colours offered: light tints that keep black text readable (content colours, not the theme's).
const RULER_COLORS: [(&str, &str); 5] = [
    ("#ffe082", "settings.ruler_color.yellow"),
    ("#c5e1a5", "settings.ruler_color.green"),
    ("#90caf9", "settings.ruler_color.blue"),
    ("#f8bbd0", "settings.ruler_color.pink"),
    ("#cfd8dc", "settings.ruler_color.grey"),
];

/// The focus-mode scopes, as `focus_scope` names them.
const FOCUS_SCOPES: [(&str, &str); 2] = [
    ("paragraph", "settings.focus_scope.paragraph"),
    ("line", "settings.focus_scope.line"),
];

/// The parts of the slider and the switch, which have no look of their own.
const SLIDER_TRACK: &str = "h-1 rounded-full bg-muted";
const SLIDER_RANGE: &str = "h-1 rounded-full bg-primary";
const SLIDER_THUMB: &str =
    "size-4 rounded-full bg-background border-2 border-primary focus-visible:border-ring";
const SWITCH_TRACK: &str = "w-9 h-5 p-0.5 rounded-full bg-input checked:bg-primary \
                            focus-visible:ring-2 focus-visible:ring-ring";
const SWITCH_THUMB: &str = "size-4 rounded-full bg-background shadow-sm checked:ml-4";

/// The settings window. `choose` runs a command after it closes (the theme pickers are their own dialogs).
pub(crate) fn settings_window(
    open: Signal<bool>,
    live: Live,
    theme_choice: Signal<ThemeChoice>,
    choose: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || open.set(false);
    let hand_over = move |id: CommandId| {
        close();
        choose(id);
    };
    let header = view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(52.0))
        .px_5()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(move || t!("settings.title"))
                .flex_1()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(move || t!("settings.saved_as_you_go"))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        );
    let body = scroll().max_h(px(560.0)).child(
        view()
            .px_5()
            .pb_4()
            .child(appearance_section(theme_choice, hand_over))
            .child(reading_section(live, hand_over))
            .child(tabs_section(live))
            .child(links_section(live))
            .child(shortcuts_section(live)),
    );
    list_dialog(
        t!("settings.title"),
        close,
        card(view().child(header).child(body)).w(px(640.0)),
    )
}

fn appearance_section(
    theme_choice: Signal<ThemeChoice>,
    hand_over: impl Fn(CommandId) + Copy + 'static,
) -> View {
    let theme_name = move || {
        let choice = theme_choice.get();
        match choice.contrast {
            themes::Contrast::High => t!("settings.theme_high", name = choice.family.name()),
            themes::Contrast::Standard => choice.family.name(),
        }
    };
    section("settings.section.appearance")
        .child(row(
            "settings.theme",
            "settings.theme_hint",
            picker_button(theme_name, move || hand_over(CommandId::ChooseTheme)),
        ))
        .child(row(
            "settings.appearance",
            "settings.appearance_hint",
            picker_button(
                move || themes::appearance_name(theme_choice.get()),
                move || hand_over(CommandId::ChooseAppearance),
            ),
        ))
}

fn reading_section(live: Live, hand_over: impl Fn(CommandId) + Copy + 'static) -> View {
    let default = ruler::Style::default();
    let default_color = format!("#{:06x}", default.color);
    let color = move || {
        live.read(|s| s.text(RULER_COLOR).map(str::to_ascii_lowercase))
            .unwrap_or_else(|| default_color.clone())
    };
    let swatches = RULER_COLORS
        .iter()
        .fold(view().flex_row().gap_2(), |swatches, &(hex, name)| {
            let color = color.clone();
            swatches.child(
                toggle()
                    .pressed(move || color() == hex)
                    .accessibility_label(t!(name))
                    .on_change(move |_: &bool| {
                        live.change(|s| {
                            let is_default = hex == format!("#{:06x}", default.color);
                            s.set_text(RULER_COLOR, (!is_default).then_some(hex));
                        });
                    })
                    .size(px(26.0))
                    .rounded_full()
                    .bg(Hsla::parse_hex(hex).expect("a built-in colour"))
                    .border_1()
                    .border_color(move || theme().border)
                    .pressed_style(|s| s.border_2().border_color(move || theme().foreground))
                    .focus_visible(|s| s.border_2().border_color(move || theme().ring)),
            )
        });
    let opacity_shown = signal(None::<f64>);
    let opacity = move || {
        opacity_shown.get().unwrap_or_else(|| {
            live.read(|s| s.number(RULER_OPACITY))
                .unwrap_or(f64::from(default.opacity))
        })
    };
    let scope = move || {
        live.read(|s| s.text(FOCUS_SCOPE).map(str::to_owned))
            .unwrap_or_else(|| FOCUS_SCOPES[0].0.to_owned())
    };
    let scopes = FOCUS_SCOPES.iter().fold(segmented(), |group, &(id, name)| {
        group.child(segment(
            name,
            move || scope() == id,
            move || {
                live.change(|s| s.set_text(FOCUS_SCOPE, (id != FOCUS_SCOPES[0].0).then_some(id)));
            },
        ))
    });
    let reading = Reading::new(live);
    let document_colors = DocumentColors::ALL
        .into_iter()
        .fold(segmented(), |group, colors| {
            group.child(segment(
                colors.label(),
                move || reading.document_colors() == colors,
                move || reading.set_document_colors(colors),
            ))
        });
    let contrast = move || {
        let palette = reading.custom_palette();
        let ratio = format!("{:.1}", palette.text_contrast());
        if palette.passes_aa() {
            t!("settings.reading_custom_contrast", ratio = ratio)
        } else {
            t!("settings.reading_custom_contrast_low", ratio = ratio)
        }
    };
    section("settings.section.reading")
        .child(row(
            "settings.document_colors",
            "settings.document_colors_hint",
            document_colors,
        ))
        .child(row(
            "settings.reading_theme",
            "settings.reading_theme_hint",
            picker_button(
                move || t!(reading_theme_bar::label(reading.reading_theme())),
                move || hand_over(CommandId::ChooseReadingTheme),
            ),
        ))
        .child(row(
            "settings.reading_custom",
            "settings.reading_custom_hint",
            view()
                .items_end()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(color_field(
                            reading,
                            READING_PAPER,
                            "settings.reading_paper",
                            |p| p.paper,
                        ))
                        .child(color_field(
                            reading,
                            READING_INK,
                            "settings.reading_ink",
                            |p| p.ink,
                        )),
                )
                // Said in words as well as colour.
                .child(text(contrast).text_size(px(12.0)).text_color(move || {
                    if reading.custom_palette().passes_aa() {
                        theme().muted_foreground
                    } else {
                        theme().destructive
                    }
                })),
        ))
        .child(row(
            "settings.ruler_color",
            "settings.ruler_color_hint",
            swatches,
        ))
        .child(row(
            "settings.ruler_height",
            "settings.ruler_height_hint",
            number_field(
                live,
                "settings.ruler_height",
                RULER_HEIGHT,
                f64::from(default.height),
                (f64::from(MIN_HEIGHT), f64::from(MAX_HEIGHT)),
                4.0,
                "settings.unit.pixels",
            ),
        ))
        .child(row(
            "settings.ruler_opacity",
            "settings.ruler_opacity_hint",
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    slider()
                        .value(move || Some(SliderValue::Single(opacity())))
                        .min(0.1)
                        .max(1.0)
                        .step(0.05)
                        .accessibility_label(t!("settings.ruler_opacity"))
                        .on_change(move |value: &SliderValue| {
                            opacity_shown.set(Some(value.start()));
                        })
                        .on_change_end(move |value: &SliderValue| {
                            let value = (value.start() * 100.0).round() / 100.0;
                            opacity_shown.set(None);
                            live.change(|s| {
                                let is_default = (value - f64::from(default.opacity)).abs() < 0.005;
                                s.set_number(RULER_OPACITY, (!is_default).then_some(value));
                            });
                        })
                        .w(px(160.0))
                        .h(px(20.0))
                        .justify_center()
                        .track_class(SLIDER_TRACK)
                        .range_class(SLIDER_RANGE)
                        .thumb_class(SLIDER_THUMB),
                )
                .child(
                    text(move || format!("{:.0} %", opacity() * 100.0))
                        .w(px(44.0))
                        .text_size(px(13.0))
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        ))
        .child(row(
            "settings.ruler_invert",
            "settings.ruler_invert_hint",
            on_off(
                "settings.ruler_invert",
                move || live.read(|s| s.flag(RULER_INVERT)).unwrap_or(false),
                move |on| live.change(|s| s.set_flag(RULER_INVERT, on.then_some(true))),
            ),
        ))
        .child(row(
            "settings.focus_scope",
            "settings.focus_scope_hint",
            scopes,
        ))
        .child(row(
            "settings.auto_scroll_speed",
            "settings.auto_scroll_speed_hint",
            number_field(
                live,
                "settings.auto_scroll_speed",
                AUTO_SCROLL_SPEED,
                f64::from(DEFAULT_SPEED),
                (f64::from(SPEEDS[0]), f64::from(SPEEDS[SPEEDS.len() - 1])),
                4.0,
                "settings.unit.pixels_a_second",
            ),
        ))
}

/// The tabs (R.12): the order Ctrl+Tab goes through them in.
fn tabs_section(live: Live) -> View {
    let order = move || live.read(|s| Cycle::parse(s.text(TAB_CYCLE_ORDER)));
    let choose = move |cycle: Cycle| {
        live.change(|s| {
            s.set_text(TAB_CYCLE_ORDER, (cycle == Cycle::Strip).then_some("strip"));
        });
    };
    section("settings.section.tabs").child(row(
        "settings.tab_cycle",
        "settings.tab_cycle_hint",
        segmented()
            .child(segment(
                "settings.tab_cycle.recent",
                move || order() == Cycle::Recent,
                move || choose(Cycle::Recent),
            ))
            .child(segment(
                "settings.tab_cycle.strip",
                move || order() == Cycle::Strip,
                move || choose(Cycle::Strip),
            )),
    ))
}

fn links_section(live: Live) -> View {
    let saved = move || {
        live.read(|s| s.text(PROTOCOLS_KEY).map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_PROTOCOLS.to_owned())
    };
    let typed = signal(None::<String>);
    let keep = move || {
        let Some(text) = typed.get_untracked() else {
            return;
        };
        typed.set(None);
        let tidy = tidy_protocols(&text);
        live.change(|s| {
            s.set_text(
                PROTOCOLS_KEY,
                (tidy != DEFAULT_PROTOCOLS).then_some(tidy.as_str()),
            );
        });
    };
    section("settings.section.links").child(row(
        "settings.link_protocols",
        "settings.link_protocols_hint",
        input()
            .value(move || typed.get().unwrap_or_else(saved))
            .on_change(move |text: &str| typed.set(Some(text.to_owned())))
            .on_submit(move |_: &str| keep())
            .on_blur(keep)
            .label(t!("settings.link_protocols"))
            .w(px(200.0))
            .h(px(32.0))
            .px_3()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || theme().input)
            .focus(|s| s.border_color(move || theme().ring))
            .text_size(px(13.0))
            .text_color(move || theme().popover_foreground),
    ))
}

/// The protocol list as the setting keeps it: lower case, comma-separated, without blanks or repeats.
fn tidy_protocols(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for protocol in text
        .split([',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let protocol = protocol.trim_end_matches(':').to_ascii_lowercase();
        if !kept.contains(&protocol) {
            kept.push(protocol);
        }
    }
    kept.join(",")
}

fn shortcuts_section(live: Live) -> View {
    let keymap = move || {
        live.read(|_| ());
        keymap::current()
    };
    let problems = move || {
        keymap()
            .problems()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let edit = button()
        .accessibility_label(t!("settings.shortcuts_edit"))
        .on_press(|| {
            if let Some(path) = settings::file_to_edit() {
                spawn(async move {
                    if let Err(err) = open_path(path).await {
                        log::warn!("settings: can't open the settings file: {err}");
                    }
                });
            }
        })
        .h(px(30.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || t!("settings.shortcuts_edit"))
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        );
    let list = COMMANDS.iter().fold(view(), |list, command| {
        let id = command.id;
        let keys = move |scope: Scope| {
            keymap()
                .bindings()
                .iter()
                .filter(|b| b.id == id && b.scope == scope)
                .map(|b| key_label(&b.keys))
                .collect::<Vec<_>>()
        };
        list.child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .min_h(px(34.0))
                .child(
                    text(move || t!(command.label))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().popover_foreground),
                )
                .child(keycaps(move || keys(Scope::Window), false))
                .child(keycaps(move || keys(Scope::Page), true)),
        )
    });
    section("settings.section.shortcuts")
        .child(row("settings.shortcuts", "settings.shortcuts_hint", edit))
        .child(each(problems, String::clone, |problem, _| {
            text(move || problem.get())
                .py_1()
                .text_size(px(12.0))
                .text_color(move || theme().destructive)
        }))
        .child(list)
}

/// The keys of one scope as keycaps; page keys are drawn quieter (the section's hint says why).
fn keycaps(keys: impl Fn() -> Vec<String> + 'static, page: bool) -> impl IntoElement {
    let caps = each(keys, String::clone, move |label, _| {
        text(move || label.get())
            .px_2()
            .h(px(22.0))
            .rounded(px(5.0))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || if page { theme().popover } else { theme().muted })
            .text_size(px(12.0))
            .tabular_nums()
            .text_color(move || {
                if page {
                    theme().muted_foreground
                } else {
                    theme().popover_foreground
                }
            })
    });
    view().flex_row().gap_1().child(caps)
}

/// A group of rows under its heading, with a rule above it.
fn section(title: &'static str) -> View {
    view().pt_5().mt_1().child(
        text(move || t!(title))
            .pb_1()
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().popover_foreground),
    )
}

/// A setting: its name and a line on what it does on the left, its control on the right.
fn row(label: &'static str, hint: &'static str, control: impl IntoElement) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_4()
        .py_3()
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .gap_1()
                .child(
                    text(move || t!(label))
                        .text_size(px(14.0))
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    text(move || t!(hint))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(control)
}

/// A button showing a choice made in a picker; pressing it opens the picker.
fn picker_button(
    shown: impl Fn() -> String + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let shown = std::rc::Rc::new(shown);
    let label = shown.clone();
    button()
        .accessibility_label(move || Some(SharedString::from(label())))
        .on_press(press)
        .h(px(32.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || shown())
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

/// A yes-or-no setting's switch.
fn on_off(
    label: &'static str,
    on: impl Fn() -> bool + Copy + 'static,
    set: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .checked(on)
        .accessibility_label(t!(label))
        .on_change(move |checked: &bool| set(*checked))
        .track_class(SWITCH_TRACK)
        .thumb_class(SWITCH_THUMB)
}

/// The frame of a row of segments, one of which is chosen.
fn segmented() -> View {
    view()
        .flex_row()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(move || theme().muted)
}

/// One segment: `chosen` says whether it is the one; pressing it chooses it.
fn segment(
    label: &'static str,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(t!(label))
        .on_change(move |_: &bool| choose())
        .h(px(28.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(move || t!(label))
                .text_size(px(13.0))
                .text_color(move || {
                    if chosen() {
                        theme().popover_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

/// A custom reading-theme colour (`key`): a swatch of it and its `#rrggbb`. What is typed is kept on Return or when
/// the field loses the focus; text that is no colour is dropped and the kept colour shows again.
fn color_field(
    reading: Reading,
    key: &'static str,
    label: &'static str,
    of: fn(Palette) -> Rgb,
) -> impl IntoElement {
    let kept = move || of(reading.custom_palette());
    let typed = signal(None::<String>);
    let keep = move || {
        let Some(text) = typed.get_untracked() else {
            return;
        };
        typed.set(None);
        if let Some(color) = parse_hex(&text) {
            reading.set_custom_color(key, Some(color));
        }
    };
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || Hsla::parse_hex(&hex(kept())).expect("hex() writes #rrggbb")),
        )
        .child(
            input()
                .value(move || typed.get().unwrap_or_else(|| hex(kept())))
                .on_change(move |text: &str| typed.set(Some(text.to_owned())))
                .on_submit(move |_: &str| keep())
                .on_blur(keep)
                .label(t!(label))
                .w(px(92.0))
                .h(px(32.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || theme().input)
                .focus(|s| s.border_color(move || theme().ring))
                .text_size(px(13.0))
                .tabular_nums()
                .text_color(move || theme().popover_foreground),
        )
}

/// A number setting's field, `default` when unset; a value equal to the default is kept as unset.
fn number_field(
    live: Live,
    label: &'static str,
    key: &'static str,
    default: f64,
    (min, max): (f64, f64),
    step: f64,
    unit: &'static str,
) -> impl IntoElement {
    number_input()
        .value(move || Some(live.read(|s| s.number(key)).unwrap_or(default)))
        .min(Some(min))
        .max(Some(max))
        .step(step)
        .accessibility_label(t!(label))
        .on_change(move |value: &Option<f64>| {
            let value = value.map(|v| v.clamp(min, max));
            live.change(|s| s.set_number(key, value.filter(|v| *v != default)));
        })
        .suffix(
            text(move || t!(unit))
                .pr_2()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .w(px(132.0))
        .h(px(32.0))
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().input)
        .input_style(|s| s.flex_1().px_3().text_size(px(13.0)).tabular_nums())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_of_the_controls_is_known() {
        let mut theme = nsh_core::tailwind::Theme::default();
        theme.set_tokens(&crate::theme::tokens(crate::theme::ThemeChoice::default()));
        for classes in [
            SLIDER_TRACK,
            SLIDER_RANGE,
            SLIDER_THUMB,
            SWITCH_TRACK,
            SWITCH_THUMB,
        ] {
            let parsed = nsh_core::tailwind::parse(&theme, classes);
            assert!(
                parsed.unknown().is_empty(),
                "{classes:?}: {:?}",
                parsed.unknown()
            );
        }
    }

    #[test]
    fn protocols_are_kept_tidy() {
        assert_eq!(
            tidy_protocols(" HTTPS, http,,mailto: https "),
            "https,http,mailto"
        );
        assert_eq!(tidy_protocols(""), "");
    }
}
