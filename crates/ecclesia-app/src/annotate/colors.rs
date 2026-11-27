//! Annotation colours (A.3), the app's side: every tool's colour and opacity live in `settings.toml` under
//! `[annotation]` (the palette, each tool given a colour of its own, the custom colours used lately), go to the page
//! canvas as its `annotationColors` prop, and are chosen on the markup bar or in the colour dialog, which has NSH's
//! colour picker with an opacity slider. The model is `ecclesia-features`' `annotate::colors`.

use ecclesia_features::annotate::colors::{self as model, Colors, Palette, Tool, ToolStyle};
use nsh_base::{ColorChannel, ColorPickerEvent, ColorSpace};
use nsh_base::{color_area, color_field, color_picker, color_slider, color_swatch, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::bookmarks::caption;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{Live, Settings};
use crate::settings_window::{segment, segmented};
use crate::tabs::{Tone, action_button};

/// The settings key the colours live under.
pub(crate) const KEY: &str = "annotation";

/// Settings key persisting whether picking a highlight colour updates the default highlight tool colour.
pub(crate) const REMEMBER_HIGHLIGHT_COLOR: &str = "remember_highlight_color";

/// Default pastel yellow highlight colour (`#fde047`) used when highlight colour memory is disabled.
pub(crate) const DEFAULT_HIGHLIGHT_HEX: u32 = 0xFD_E047;

/// Returns whether choosing a highlight colour updates the remembered highlight tool colour (`true` by default).
pub(crate) fn remember_highlight_color(settings: &Settings) -> bool {
    settings.flag(REMEMBER_HIGHLIGHT_COLOR).unwrap_or(true)
}

/// Persists whether highlight colour choices are remembered for future highlights (`true` clears the key).
pub(crate) fn set_remember_highlight_color(live: Live, remember: bool) {
    live.change(|s| s.set_flag(REMEMBER_HIGHLIGHT_COLOR, (!remember).then_some(false)));
}

/// Remembers `color` as `Tool::Highlight`'s saved colour when highlight colour memory is enabled.
pub(crate) fn remember_highlight_choice(live: Live, color: u32) {
    if !untracked(|| live.read(remember_highlight_color)) {
        return;
    }
    live.change(|s| {
        let mut colors = read(s);
        colors.choose_color(Tool::Highlight, color);
        write(s, &colors);
    });
}

/// The colours the settings hold.
pub(crate) fn read(settings: &Settings) -> Colors {
    settings
        .value(KEY)
        .and_then(|value| serde_json::to_value(value).ok())
        .map(|json| Colors::from_json(&json))
        .unwrap_or_default()
}

/// Keeps `colors` in the settings; the defaults remove the key.
pub(crate) fn write(settings: &mut Settings, colors: &Colors) {
    let value = (*colors != Colors::default())
        .then(|| toml::Value::try_from(colors.to_json()).ok())
        .flatten();
    settings.set_value(KEY, value);
}

/// The palette the tools offer.
pub(crate) fn palette(live: Live) -> Palette {
    live.read(read).palette
}

/// Offers `palette`'s colours; the tools given one of the old palette's colours keep it.
pub(crate) fn set_palette(live: Live, palette: Palette) {
    live.change(|s| {
        let mut colors = read(s);
        colors.palette = palette;
        write(s, &colors);
    });
}

/// Effective colours exposed to the page canvas, keeping the default yellow highlight when memory is off.
fn effective_for_canvas(settings: &Settings) -> Colors {
    let mut colors = read(settings);
    if !remember_highlight_color(settings) {
        let opacity = colors.style(Tool::Highlight).opacity;
        let style = ToolStyle::new(DEFAULT_HIGHLIGHT_HEX, opacity);
        match colors
            .chosen
            .iter_mut()
            .find(|(tool, _)| *tool == Tool::Highlight)
        {
            Some((_, kept)) => *kept = style,
            None => colors.chosen.push((Tool::Highlight, style)),
        }
    }
    colors
}

/// The `annotationColors` prop: the colours as the canvas reads them.
pub(crate) fn prop(live: Live) -> JsonValue {
    live.read(|s| effective_for_canvas(s).to_json())
}

/// Gives `tool` `color` (the canvas's `toolColor`: `{tool, color}`), keeping its opacity.
pub(crate) fn on_tool_color(live: Live, data: &JsonValue) {
    let tool = data
        .get("tool")
        .and_then(JsonValue::as_str)
        .and_then(Tool::parse);
    let color = data
        .get("color")
        .and_then(JsonValue::as_str)
        .and_then(model::parse_hex);
    let (Some(tool), Some(color)) = (tool, color) else {
        log::warn!("toolColor: a request that doesn't parse: {data}");
        return;
    };
    if tool == Tool::Highlight && !untracked(|| live.read(remember_highlight_color)) {
        return;
    }
    live.change(|s| {
        let mut colors = read(s);
        colors.choose_color(tool, color);
        write(s, &colors);
    });
}

/// The colour dialog (A.3): which tool, then its colour and opacity on NSH's picker, the palette's and the recent
/// colours as swatches. Done keeps them (and notifies `on_pick`); Escape, Cancel or a press outside drops them and
/// notifies `on_cancel`.
pub(crate) fn color_dialog(
    live: Live,
    picking: Signal<Option<Tool>>,
    initial_color: Option<u32>,
    has_target: bool,
    on_pick: impl Fn(Tool, ToolStyle) + Copy + 'static,
    on_cancel: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let cancel = move || {
        picking.set(None);
        on_cancel();
    };
    let finish = move || {
        picking.set(None);
    };
    let content = match untracked(|| picking.get()) {
        Some(tool) => form(
            live,
            tool,
            initial_color,
            has_target,
            on_pick,
            finish,
            cancel,
        ),
        None => view(),
    };
    list_dialog(t!("annotation_colors.dialog"), cancel, card(content))
}

fn form(
    live: Live,
    first: Tool,
    initial_color: Option<u32>,
    has_target: bool,
    on_pick: impl Fn(Tool, ToolStyle) + Copy + 'static,
    finish: impl Fn() + Copy + 'static,
    cancel: impl Fn() + Copy + 'static,
) -> View {
    let colors = untracked(|| live.read(read));
    let tool = signal(first);
    let index = move |tool: Tool| Tool::ALL.iter().position(|&t| t == tool).unwrap_or(0);
    // What each tool is given in the dialog, kept only on Done.
    let mut initial_styles = Tool::ALL.map(|tool| colors.style(tool));
    if let Some(rgb_hex) = initial_color {
        let at = index(first);
        initial_styles[at] = ToolStyle::new(rgb_hex, initial_styles[at].opacity);
    }
    let styles = signal(initial_styles);
    let current = move || styles.with(|styles| styles[index(tool.get())]);
    let recent = colors.recent.clone();
    let opacity = move || current().opacity;
    let set = move |style: ToolStyle| {
        let at = index(untracked(|| tool.get()));
        styles.update(|styles| styles[at] = style);
    };
    let done = move || {
        let active_tool = untracked(|| tool.get());
        let chosen = untracked(|| styles.get());
        let active_style = chosen[index(active_tool)];
        live.change(|s| {
            let remember = remember_highlight_color(s);
            let mut colors = read(s);
            for (tool, style) in Tool::ALL.into_iter().zip(chosen) {
                if has_target && tool == Tool::Highlight && !remember {
                    continue;
                }
                if style != colors.style(tool) {
                    colors.choose(tool, style);
                }
            }
            write(s, &colors);
        });
        on_pick(active_tool, active_style);
        finish();
    };
    let tabs = Tool::ALL.into_iter().fold(segmented(), |row, choice| {
        row.child(segment(
            tool_label(choice),
            move || tool.get() == choice,
            move || tool.set(choice),
        ))
    });
    let picker = color_picker()
        .value(move || Some(hsla(current())))
        .on_change(move |event: &ColorPickerEvent| {
            if let ColorPickerEvent::Change(value, _) = event {
                set(style_of(*value));
            }
        })
        .child(
            view()
                .gap_3()
                .child(
                    color_area()
                        .accessibility_label(t!("annotation_colors.area"))
                        .w_full()
                        .h(px(140.0)),
                )
                .child(slider(ColorChannel::Hue, "annotation_colors.hue"))
                .child(slider(ColorChannel::Alpha, "annotation_colors.opacity"))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            color_field()
                                .accessibility_label(t!("annotation_colors.hex"))
                                .h(px(32.0))
                                .px_2()
                                .flex_1()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(move || theme().input)
                                .text_size(px(13.0)),
                        )
                        .child(
                            text(move || {
                                let percent = (opacity() * 100.0).round();
                                t!("annotation_colors.percent", value = percent.to_string())
                            })
                            .text_size(px(13.0))
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(caption("annotation_colors.palette"))
                .child(swatch_row(
                    move || colors.palette.swatches(tool.get()).to_vec(),
                    opacity,
                ))
                .child(caption("annotation_colors.recent"))
                .child(swatch_row(move || recent.clone(), opacity)),
        );
    view()
        .w(px(400.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("annotation_colors.dialog"))
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(tabs)
        .child(picker)
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .gap_2()
                .justify_end()
                .child(action_button(
                    "annotation_colors.cancel",
                    Tone::Plain,
                    cancel,
                ))
                .child(crate::shell::primary_button("annotation_colors.done", done)),
        )
}

/// A slider of one of HSB's channels, or the opacity.
fn slider(channel: ColorChannel, label: &'static str) -> impl IntoElement {
    color_slider()
        .color_space(ColorSpace::Hsb)
        .channel(channel)
        .accessibility_label(t!(label))
        .w_full()
        .h(px(20.0))
        .justify_center()
}

/// Swatches that pick their colour into the picker at the tool's `opacity`, so a press keeps it; "none yet" when
/// there are none.
fn swatch_row(
    colors: impl Fn() -> Vec<u32> + 'static,
    opacity: impl Fn() -> f32 + Copy + 'static,
) -> impl IntoElement {
    dynamic(move || {
        let colors = colors();
        if colors.is_empty() {
            return text(move || t!("annotation_colors.none"))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground)
                .into_element();
        }
        colors
            .into_iter()
            .fold(view().flex_row().gap_2(), |row, color| {
                row.child(
                    color_swatch()
                        .color(move || Some(hsla(ToolStyle::new(color, opacity()))))
                        .accessibility_label(model::hex(color))
                        .size(px(24.0))
                        .rounded_full()
                        .border_1()
                        .border_color(move || theme().border)
                        .selected_style(|s| s.border_2().border_color(move || theme().foreground)),
                )
            })
            .into_element()
    })
}

/// A tool's name in the dialog.
fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Highlight => "markup.highlight",
        Tool::Underline => "markup.underline",
        Tool::Squiggly => "markup.squiggly",
        Tool::StrikeOut => "markup.strikeout",
        Tool::Proposal => "annotation_colors.proposal",
        Tool::Shape => "annotation_colors.shape",
    }
}

/// `style` as the picker holds it: its colour, its opacity the alpha.
fn hsla(style: ToolStyle) -> Hsla {
    let mut color = Hsla::from(rgb(style.color));
    color.a = style.opacity;
    color
}

/// What the picker holds, as a tool's colour and opacity.
fn style_of(color: Hsla) -> ToolStyle {
    let rgb = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    ToolStyle::new(
        channel(rgb.r) << 16 | channel(rgb.g) << 8 | channel(rgb.b),
        color.a,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_kept_in_the_settings_and_read_back() {
        let mut settings = Settings::default();
        assert_eq!(read(&settings), Colors::default());
        let mut colors = Colors {
            palette: Palette::ColorBlindSafe,
            ..Colors::default()
        };
        colors.choose(Tool::Underline, ToolStyle::new(0x123456, 0.4));
        write(&mut settings, &colors);
        assert_eq!(read(&settings), colors);
        write(&mut settings, &Colors::default());
        assert!(settings.value(KEY).is_none(), "the defaults leave no key");
    }

    #[test]
    fn the_picker_s_alpha_is_the_opacity() {
        let style = ToolStyle::new(0x3b82f6, 0.5);
        let back = style_of(hsla(style));
        assert!((back.opacity - 0.5).abs() < 0.01);
        let [r, g, b] = [16, 8, 0].map(|shift| ((back.color >> shift) & 0xff) as i32);
        assert!((r - 0x3b).abs() <= 1 && (g - 0x82).abs() <= 1 && (b - 0xf6).abs() <= 1);
    }

    #[test]
    fn remember_highlight_color_defaults_true_and_enforces_yellow_when_off() {
        let mut settings = Settings::default();
        assert!(remember_highlight_color(&settings));

        let mut custom = Colors::default();
        custom.choose_color(Tool::Highlight, 0x93_C5FD);
        write(&mut settings, &custom);
        assert_eq!(
            effective_for_canvas(&settings).style(Tool::Highlight).color,
            0x93_C5FD
        );

        settings.set_flag(REMEMBER_HIGHLIGHT_COLOR, Some(false));
        assert!(!remember_highlight_color(&settings));
        assert_eq!(
            effective_for_canvas(&settings).style(Tool::Highlight).color,
            DEFAULT_HIGHLIGHT_HEX
        );
    }
}
