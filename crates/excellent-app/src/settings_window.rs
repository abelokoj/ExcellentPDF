//! The settings window (K.16 step 3, plan §9.2): every setting the app has, in groups, each beside the control that
//! changes it. A change takes effect and is kept at once, so there is no Save or Cancel; Escape closes it. Shortcuts
//! are listed as they are bound, with what could not be bound; they are edited in `settings.toml`, which it opens.
//!
//! Built on the quick dialogs' card ([`crate::list_dialog`]), wider, with rows of a label and its hint on the left and
//! the control on the right.

use excellent_core::formats::{DEFAULT_FONT_SIZE, FONT_SIZES};
use excellent_features::read::autoscroll::{DEFAULT_SPEED, SPEEDS};
use excellent_features::read::links::DEFAULT_PROTOCOLS;
use excellent_features::read::ruler::{self, MAX_HEIGHT, MIN_HEIGHT};
use nsh_base::{SliderValue, button, input, number_input, slider, switch, theme, toggle};
use nsh_core::prelude::*;

use crate::commands::{COMMANDS, CommandId};
use crate::home::{
    DEFAULT_HOME_MAX_RECENT, DEFAULT_HOME_THUMB_SIZE, HOME_DESIGN_STYLE, HOME_FIXED_HEADER,
    HOME_FULL_PAGE_THUMBNAILS, HOME_MAX_RECENT, HOME_SHOW_PROGRESS, HOME_SORT_BY_FREQUENCY,
    HOME_THUMBNAIL_PADDING, HOME_THUMBNAIL_SIZE, HOME_VIEW_MODE, HomeDesignStyle, HomeViewMode,
    MAX_HOME_MAX_RECENT, MAX_HOME_THUMB_SIZE, MIN_HOME_MAX_RECENT, MIN_HOME_THUMB_SIZE,
};
use crate::keymap::{self, Scope};
use crate::links::PROTOCOLS_KEY;
use crate::list_dialog::{card, list_dialog};
use crate::modes::{FINGER_DRAWS, PEN_TO_WRITE};
use crate::onboarding::SHOW_TIPS;
use crate::packs::{PACKS_INDEX_URL, PACKS_KEY_HEX, PacksState, format_size};
use crate::panes::{CLOSE_PANE_TABS, ClosingTabs, SYNC_SCROLL, SYNC_ZOOM};
use crate::print::{PRINT_ANNOTATIONS, PRINT_SCALING};
use crate::reading::{
    AUTO_SCROLL_SPEED, DEFAULT_PREVIEW_DELAY, DocumentColors, FOCUS_SCOPE, MAX_PREVIEW_DELAY,
    PAPER_TEXTURE, READING_INK, READING_PAPER, REF_PREVIEW, REF_PREVIEW_DELAY, REFLOW_FONT_SIZE,
    RULER_COLOR, RULER_HEIGHT, RULER_INVERT, RULER_OPACITY, Reading, hex, parse_hex,
};
use crate::reading_theme_bar;
use crate::settings::{self, Live};
use crate::tabs::{
    Cycle, DEFAULT_TAB_MIN_WIDTH_PX, MAX_TAB_MIN_WIDTH_PX, MIN_TAB_MIN_WIDTH_PX, TAB_CYCLE_ORDER,
    TAB_MIN_WIDTH_PX, TAB_STYLE, TabStyle,
};
use crate::theme::{self as themes, ThemeChoice};
use crate::workspaces::{Elsewhere, OPEN_ELSEWHERE};
use excellent_core::darkmode::{Palette, Rgb};
use excellent_core::print::Scaling;
use excellent_features::split::Follow;

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
pub(crate) const SLIDER_TRACK: &str = "h-1 rounded-full bg-muted";
pub(crate) const SLIDER_RANGE: &str = "h-1 rounded-full bg-primary";
pub(crate) const SLIDER_THUMB: &str =
    "size-4 rounded-full bg-background border-2 border-primary focus-visible:border-ring";
pub(crate) const SWITCH_TRACK: &str = "w-9 h-5 p-0.5 rounded-full bg-input checked:bg-primary \
                            focus-visible:ring-2 focus-visible:ring-ring";
pub(crate) const SWITCH_THUMB: &str = "size-4 rounded-full bg-background shadow-sm checked:ml-4";

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
            .child(appearance_section(live, theme_choice, hand_over))
            .child(reading_section(live, hand_over))
            .child(tabs_section(live))
            .child(home_section(live))
            .child(links_section(live))
            .child(print_section(live))
            .child(annotation_section(live, hand_over))
            .child(modes_section(live, hand_over))
            .child(packs_section(live))
            .child(phase5_section(live, hand_over))
            .child(phase6_7_section(live))
            .child(sandbox_section(live))
            .child(help_section(live, hand_over))
            .child(shortcuts_section(live)),
    );
    list_dialog(
        t!("settings.title"),
        close,
        card(view().child(header).child(body)).w(px(640.0)),
    )
}

fn appearance_section(
    live: Live,
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
        .child(row(
            "settings.language",
            "settings.language_hint",
            language_choice(live),
        ))
        .child(row(
            "settings.follow_system_contrast",
            "settings.follow_system_contrast_hint",
            on_off(
                "settings.follow_system_contrast",
                move || live.read(|s| s.flag(themes::FOLLOW_SYSTEM_CONTRAST).unwrap_or(true)),
                move |on| {
                    live.change(|s| {
                        s.set_flag(themes::FOLLOW_SYSTEM_CONTRAST, (!on).then_some(false));
                    });
                },
            ),
        ))
}

/// A.3: the palette the markup tools offer, and the colour dialog.
fn annotation_section(live: Live, hand_over: impl Fn(CommandId) + Copy + 'static) -> View {
    use crate::annotate::margin::MARGIN_NOTES;
    use crate::annotate::{POLYGON_SIDES, STAR_POINTS, colors};
    use excellent_features::annotate::polygons::{DEFAULT_SIDES, SIDES};
    let counts = (f64::from(*SIDES.start()), f64::from(*SIDES.end()));
    use excellent_features::annotate::colors::Palette;
    let palette = [
        (Palette::Standard, "settings.annotation_palette_standard"),
        (Palette::ColorBlindSafe, "settings.annotation_palette_safe"),
    ]
    .into_iter()
    .fold(segmented(), |row, (choice, label)| {
        row.child(segment(
            label,
            move || colors::palette(live) == choice,
            move || colors::set_palette(live, choice),
        ))
    });
    section("settings.section.annotation")
        .child(row(
            "settings.annotation_palette",
            "settings.annotation_palette_hint",
            palette,
        ))
        .child(row(
            "settings.annotation_colors",
            "settings.annotation_colors_hint",
            picker_button(
                move || t!("settings.annotation_colors_choose"),
                move || hand_over(CommandId::AnnotationColors),
            ),
        ))
        .child(row(
            "settings.margin_notes",
            "settings.margin_notes_hint",
            on_off(
                "settings.margin_notes",
                move || live.read(|s| s.flag(MARGIN_NOTES).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(MARGIN_NOTES, on.then_some(true))),
            ),
        ))
        .child(row(
            "settings.polygon_sides",
            "settings.polygon_sides_hint",
            number_field(
                live,
                "settings.polygon_sides",
                POLYGON_SIDES,
                f64::from(DEFAULT_SIDES),
                counts,
                1.0,
                "settings.unit.sides",
            ),
        ))
        .child(row(
            "settings.star_points",
            "settings.star_points_hint",
            number_field(
                live,
                "settings.star_points",
                STAR_POINTS,
                f64::from(DEFAULT_SIDES),
                counts,
                1.0,
                "settings.unit.star_points",
            ),
        ))
}

/// U.9: the tips on or off, and the guide again.
fn modes_section(live: Live, hand_over: impl Fn(CommandId) + Copy + 'static) -> View {
    section("settings.section.modes")
        .child(row(
            "settings.pen_to_write",
            "settings.pen_to_write_hint",
            on_off(
                "settings.pen_to_write",
                move || live.read(|s| s.flag(PEN_TO_WRITE).unwrap_or(true)),
                move |on| live.change(|s| s.set_flag(PEN_TO_WRITE, (!on).then_some(false))),
            ),
        ))
        .child(row(
            "settings.finger_draws",
            "settings.finger_draws_hint",
            on_off(
                "settings.finger_draws",
                move || live.read(|s| s.flag(FINGER_DRAWS).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(FINGER_DRAWS, on.then_some(true))),
            ),
        ))
        .child(row(
            "settings.toolbar",
            "settings.toolbar_hint",
            picker_button(
                move || t!("settings.toolbar_customize"),
                move || hand_over(CommandId::CustomizeToolbar),
            ),
        ))
}

fn help_section(live: Live, hand_over: impl Fn(CommandId) + Copy + 'static) -> View {
    section("settings.section.help")
        .child(row(
            "settings.show_tips",
            "settings.show_tips_hint",
            on_off(
                "settings.show_tips",
                move || live.read(|s| s.flag(SHOW_TIPS).unwrap_or(true)),
                move |on| live.change(|s| s.set_flag(SHOW_TIPS, (!on).then_some(false))),
            ),
        ))
        .child(row(
            "settings.guide",
            "settings.guide_hint",
            picker_button(
                move || t!("settings.guide_show"),
                move || hand_over(CommandId::ReplayGuide),
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
            "settings.paper_texture",
            "settings.paper_texture_hint",
            on_off(
                "settings.paper_texture",
                move || live.read(|s| s.flag(PAPER_TEXTURE).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(PAPER_TEXTURE, on.then_some(true))),
            ),
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
            "settings.reflow_font_size",
            "settings.reflow_font_size_hint",
            number_field(
                live,
                "settings.reflow_font_size",
                REFLOW_FONT_SIZE,
                f64::from(DEFAULT_FONT_SIZE),
                (f64::from(*FONT_SIZES.start()), f64::from(*FONT_SIZES.end())),
                1.0,
                "settings.unit.points",
            ),
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
/// SumatraPDF Enhanced Home Page design style, layout, and card controls.
fn home_section(live: Live) -> View {
    let design_style = move || live.read(|s| HomeDesignStyle::parse(s.text(HOME_DESIGN_STYLE)));
    let choose_design = move |style: HomeDesignStyle| {
        live.change(|s| {
            s.set_text(
                HOME_DESIGN_STYLE,
                (style != HomeDesignStyle::default()).then_some(style.name()),
            );
        });
    };
    let view_mode = move || live.read(|s| HomeViewMode::parse(s.text(HOME_VIEW_MODE)));
    let choose_view = move |mode: HomeViewMode| {
        live.change(|s| {
            s.set_text(
                HOME_VIEW_MODE,
                (mode == HomeViewMode::List).then_some("list"),
            );
        });
    };
    let freq_sort = move || live.read(|s| s.flag(HOME_SORT_BY_FREQUENCY).unwrap_or(false));
    section("settings.section.home")
        .child(row(
            "settings.home_design_style",
            "settings.home_design_style_hint",
            segmented()
                .child(segment(
                    "settings.home_design_style.enhanced_warm",
                    move || design_style() == HomeDesignStyle::EnhancedWarm,
                    move || choose_design(HomeDesignStyle::EnhancedWarm),
                ))
                .child(segment(
                    "settings.home_design_style.modern_green",
                    move || design_style() == HomeDesignStyle::ModernGreen,
                    move || choose_design(HomeDesignStyle::ModernGreen),
                ))
                .child(segment(
                    "settings.home_design_style.classic_clean",
                    move || design_style() == HomeDesignStyle::ClassicClean,
                    move || choose_design(HomeDesignStyle::ClassicClean),
                )),
        ))
        .child(row(
            "settings.home_view_mode",
            "settings.home_view_mode_hint",
            segmented()
                .child(segment(
                    "settings.home_view_mode.thumbnails",
                    move || view_mode() == HomeViewMode::Thumbnails,
                    move || choose_view(HomeViewMode::Thumbnails),
                ))
                .child(segment(
                    "settings.home_view_mode.list",
                    move || view_mode() == HomeViewMode::List,
                    move || choose_view(HomeViewMode::List),
                )),
        ))
        .child(row(
            "settings.home_thumb_size",
            "settings.home_thumb_size_hint",
            number_field(
                live,
                "settings.home_thumb_size",
                HOME_THUMBNAIL_SIZE,
                f64::from(DEFAULT_HOME_THUMB_SIZE),
                (
                    f64::from(MIN_HOME_THUMB_SIZE),
                    f64::from(MAX_HOME_THUMB_SIZE),
                ),
                25.0,
                "settings.unit.percent",
            ),
        ))
        .child(row(
            "settings.home_sort_frequency",
            "settings.home_sort_frequency_hint",
            segmented()
                .child(segment(
                    "settings.home_sort.recent",
                    move || !freq_sort(),
                    move || live.change(|s| s.set_flag(HOME_SORT_BY_FREQUENCY, None)),
                ))
                .child(segment(
                    "settings.home_sort.frequent",
                    freq_sort,
                    move || live.change(|s| s.set_flag(HOME_SORT_BY_FREQUENCY, Some(true))),
                )),
        ))
        .child(row(
            "settings.home_thumb_padding",
            "settings.home_thumb_padding_hint",
            on_off(
                "settings.home_thumb_padding",
                move || live.read(|s| s.flag(HOME_THUMBNAIL_PADDING).unwrap_or(true)),
                move |on| {
                    live.change(|s| s.set_flag(HOME_THUMBNAIL_PADDING, (!on).then_some(false)));
                },
            ),
        ))
        .child(row(
            "settings.home_show_progress",
            "settings.home_show_progress_hint",
            on_off(
                "settings.home_show_progress",
                move || live.read(|s| s.flag(HOME_SHOW_PROGRESS).unwrap_or(true)),
                move |on| live.change(|s| s.set_flag(HOME_SHOW_PROGRESS, (!on).then_some(false))),
            ),
        ))
        .child(row(
            "settings.home_full_page",
            "settings.home_full_page_hint",
            on_off(
                "settings.home_full_page",
                move || live.read(|s| s.flag(HOME_FULL_PAGE_THUMBNAILS).unwrap_or(false)),
                move |on| {
                    live.change(|s| s.set_flag(HOME_FULL_PAGE_THUMBNAILS, on.then_some(true)))
                },
            ),
        ))
        .child(row(
            "settings.home_fixed_header",
            "settings.home_fixed_header_hint",
            on_off(
                "settings.home_fixed_header",
                move || live.read(|s| s.flag(HOME_FIXED_HEADER).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(HOME_FIXED_HEADER, on.then_some(true))),
            ),
        ))
        .child(row(
            "settings.home_max_recent",
            "settings.home_max_recent_hint",
            number_field(
                live,
                "settings.home_max_recent",
                HOME_MAX_RECENT,
                f64::from(DEFAULT_HOME_MAX_RECENT),
                (
                    f64::from(MIN_HOME_MAX_RECENT),
                    f64::from(MAX_HOME_MAX_RECENT),
                ),
                10.0,
                "settings.unit.docs",
            ),
        ))
}

fn tabs_section(live: Live) -> View {
    let tab_style = move || live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let choose_style = move |style: TabStyle| {
        live.change(|s| {
            s.set_text(TAB_STYLE, (style == TabStyle::Chrome).then_some("chrome"));
        });
    };
    let order = move || live.read(|s| Cycle::parse(s.text(TAB_CYCLE_ORDER)));
    let choose = move |cycle: Cycle| {
        live.change(|s| {
            s.set_text(TAB_CYCLE_ORDER, (cycle == Cycle::Strip).then_some("strip"));
        });
    };
    let closing = move || live.read(|s| ClosingTabs::parse(s.text(CLOSE_PANE_TABS)));
    let choose_closing = move |choice: ClosingTabs| {
        live.change(|s| {
            s.set_text(
                CLOSE_PANE_TABS,
                (choice == ClosingTabs::Close).then_some("close"),
            );
        });
    };
    let follow = move || live.read(|s| Follow::parse(s.text(SYNC_SCROLL)));
    let choose_follow = move |choice: Follow| {
        live.change(|s| {
            s.set_text(
                SYNC_SCROLL,
                (choice != Follow::default()).then_some(choice.name()),
            );
        });
    };
    let elsewhere = move || live.read(|s| Elsewhere::parse(s.text(OPEN_ELSEWHERE)));
    let choose_elsewhere = move |choice: Elsewhere| {
        live.change(|s| {
            s.set_text(
                OPEN_ELSEWHERE,
                (choice == Elsewhere::Here).then_some("here"),
            );
        });
    };
    section("settings.section.tabs")
        .child(row(
            "settings.tab_style",
            "settings.tab_style_hint",
            segmented()
                .child(segment(
                    "settings.tab_style.vivaldi",
                    move || tab_style() == TabStyle::Vivaldi,
                    move || choose_style(TabStyle::Vivaldi),
                ))
                .child(segment(
                    "settings.tab_style.chrome",
                    move || tab_style() == TabStyle::Chrome,
                    move || choose_style(TabStyle::Chrome),
                )),
        ))
        .child(row(
            "settings.tab_min_width",
            "settings.tab_min_width_hint",
            number_field(
                live,
                "settings.tab_min_width",
                TAB_MIN_WIDTH_PX,
                DEFAULT_TAB_MIN_WIDTH_PX,
                (MIN_TAB_MIN_WIDTH_PX, MAX_TAB_MIN_WIDTH_PX),
                10.0,
                "settings.unit.px",
            ),
        ))
        .child(row(
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
        // V.6: what closing a pane does with its tabs.
        .child(row(
            "settings.close_pane_tabs",
            "settings.close_pane_tabs_hint",
            segmented()
                .child(segment(
                    "settings.close_pane_tabs.move",
                    move || closing() == ClosingTabs::Move,
                    move || choose_closing(ClosingTabs::Move),
                ))
                .child(segment(
                    "settings.close_pane_tabs.close",
                    move || closing() == ClosingTabs::Close,
                    move || choose_closing(ClosingTabs::Close),
                )),
        ))
        // V.8: how synced panes follow, and whether the zoom does.
        .child(row(
            "settings.sync_scroll",
            "settings.sync_scroll_hint",
            segmented()
                .child(segment(
                    "settings.sync_scroll.page",
                    move || follow() == Follow::SamePage,
                    move || choose_follow(Follow::SamePage),
                ))
                .child(segment(
                    "settings.sync_scroll.position",
                    move || follow() == Follow::SamePosition,
                    move || choose_follow(Follow::SamePosition),
                ))
                .child(segment(
                    "settings.sync_scroll.pairs",
                    move || follow() == Follow::Pairs,
                    move || choose_follow(Follow::Pairs),
                )),
        ))
        .child(row(
            "settings.sync_zoom",
            "settings.sync_zoom_hint",
            on_off(
                "settings.sync_zoom",
                move || live.read(|s| s.flag(SYNC_ZOOM).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(SYNC_ZOOM, on.then_some(true))),
            ),
        ))
        // W.6: a file another workspace has open.
        .child(row(
            "settings.open_elsewhere",
            "settings.open_elsewhere_hint",
            segmented()
                .child(segment(
                    "settings.open_elsewhere.switch",
                    move || elsewhere() == Elsewhere::Switch,
                    move || choose_elsewhere(Elsewhere::Switch),
                ))
                .child(segment(
                    "settings.open_elsewhere.here",
                    move || elsewhere() == Elsewhere::Here,
                    move || choose_elsewhere(Elsewhere::Here),
                )),
        ))
}

/// Printing (O.22): how pages are sized on the paper, and whether annotations print. The system print dialog has
/// the rest.
fn print_section(live: Live) -> View {
    let scaling = move || live.read(|s| Scaling::parse(s.text(PRINT_SCALING)));
    let choose = move |choice: Scaling| {
        live.change(|s| {
            s.set_text(
                PRINT_SCALING,
                (choice != Scaling::default()).then_some(choice.name()),
            );
        });
    };
    let segments = Scaling::ALL.into_iter().fold(segmented(), |group, choice| {
        let label = match choice {
            Scaling::Shrink => "settings.print_scaling.shrink",
            Scaling::Fit => "settings.print_scaling.fit",
            Scaling::Actual => "settings.print_scaling.actual",
        };
        group.child(segment(
            label,
            move || scaling() == choice,
            move || choose(choice),
        ))
    });
    let annotations = move || live.read(|s| s.flag(PRINT_ANNOTATIONS).unwrap_or(true));
    section("settings.section.print")
        .child(row(
            "settings.print_scaling",
            "settings.print_scaling_hint",
            segments,
        ))
        .child(row(
            "settings.print_annotations",
            "settings.print_annotations_hint",
            on_off("settings.print_annotations", annotations, move |on| {
                live.change(|s| s.set_flag(PRINT_ANNOTATIONS, (!on).then_some(false)));
            }),
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
    let previews = move || live.read(|s| s.flag(REF_PREVIEW)).unwrap_or(true);
    section("settings.section.links")
        .child(row(
            "settings.ref_preview",
            "settings.ref_preview_hint",
            on_off("settings.ref_preview", previews, move |on| {
                live.change(|s| s.set_flag(REF_PREVIEW, (!on).then_some(false)));
            }),
        ))
        .child(row(
            "settings.ref_preview_delay",
            "settings.ref_preview_delay_hint",
            number_field(
                live,
                "settings.ref_preview_delay",
                REF_PREVIEW_DELAY,
                DEFAULT_PREVIEW_DELAY,
                (0.0, MAX_PREVIEW_DELAY),
                50.0,
                "settings.unit.milliseconds",
            ),
        ))
        .child(row(
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

/// Phase 5 settings: Folder watch (F.9), Workspace rules (W.12), SyncTeX preset (R.27),
/// Bionic reading (R.22), Neural offline voice (U.6), and UI control channel (E.4).
fn phase5_section(live: Live, hand_over: impl Fn(CommandId) + Copy + 'static) -> View {
    use crate::shell::control_channel::CONTROL_CHANNEL_ENABLED;
    use crate::shell::reading_extra::{BIONIC_ENABLED, SYNCTEX_PRESET};
    use crate::study_ui::{StudyState, phase5::NEURAL_VOICE_ID};
    use excellent_features::library::{Library, WatchDepth};

    let watch_status = signal(String::new());
    let add_watched_folder = move || {
        let dialog = nsh_core::file_dialog::OpenDialog::folders().title(t!("settings.watch.add"));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(dir) = paths.into_iter().next()
                && let Some(store) = crate::library::store()
            {
                let lib = Library::new(store);
                let _ = lib.watch_folder(&dir, WatchDepth::Recursive);
                if let Ok(report) = lib.reconcile_watched() {
                    watch_status.set(t!(
                        "library.folder_watched",
                        added = report.added,
                        updated = report.modified
                    ));
                }
            }
        });
    };

    let current_preset =
        move || live.read(|s| s.text(SYNCTEX_PRESET).unwrap_or("vscode").to_owned());
    let set_preset = move |id: &str| {
        let owned = id.to_owned();
        live.change(|s| s.set_text(SYNCTEX_PRESET, Some(owned.as_str())));
    };
    let presets = [
        ("vscode", "settings.synctex.vscode"),
        ("vim", "settings.synctex.vim"),
        ("emacs", "settings.synctex.emacs"),
        ("texstudio", "settings.synctex.texstudio"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || current_preset() == id,
            move || set_preset(id),
        ))
    });

    let neural_voices = StudyState::installed_neural_voices();
    let current_voice = move || live.read(|s| s.text(NEURAL_VOICE_ID).map(str::to_owned));
    let voice_picker = if neural_voices.is_empty() {
        segmented().child(segment(
            "settings.neural_voice.system",
            move || current_voice().is_none(),
            move || live.change(|s| s.set_text(NEURAL_VOICE_ID, None)),
        ))
    } else {
        let mut bar = segmented().child(segment(
            "settings.neural_voice.system",
            move || current_voice().is_none(),
            move || live.change(|s| s.set_text(NEURAL_VOICE_ID, None)),
        ));
        for (vid, vlabel) in neural_voices {
            let vid_sig = signal(vid);
            bar = bar.child(segment_owned(
                vlabel,
                move || vid_sig.with(|v| current_voice().as_deref() == Some(v.as_str())),
                move || {
                    vid_sig.with(|v| live.change(|s| s.set_text(NEURAL_VOICE_ID, Some(v.as_str()))))
                },
            ));
        }
        bar
    };

    section("settings.section.advanced")
        .child(row(
            "settings.bionic",
            "settings.bionic_hint",
            on_off(
                "settings.bionic",
                move || live.read(|s| s.flag(BIONIC_ENABLED).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(BIONIC_ENABLED, on.then_some(true))),
            ),
        ))
        .child(row("settings.synctex", "settings.synctex_hint", presets))
        .child(row(
            "settings.neural_voice",
            "settings.neural_voice_hint",
            voice_picker,
        ))
        .child(row(
            "settings.watch",
            "settings.watch_hint",
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(picker_button(
                    move || {
                        let s = watch_status.get();
                        if s.is_empty() {
                            t!("settings.watch.add")
                        } else {
                            s
                        }
                    },
                    add_watched_folder,
                )),
        ))
        .child(row(
            "settings.workspace_rules",
            "settings.workspace_rules_hint",
            picker_button(
                move || t!("command.workspace_rules"),
                move || hand_over(CommandId::WorkspaceRules),
            ),
        ))
        .child(row(
            "settings.control_channel",
            "settings.control_channel_hint",
            on_off(
                "settings.control_channel",
                move || live.read(|s| s.flag(CONTROL_CHANNEL_ENABLED).unwrap_or(true)),
                move |on| live.change(|s| s.set_flag(CONTROL_CHANNEL_ENABLED, Some(on))),
            ),
        ))
}

/// Phase 6 & 7 settings: AI consent & providers (I.1), PII redaction & budget, Google Lens
/// online toggle (R.31), CAD drawing view defaults (R.26), and TSA remote consent (O.14/O.20).
fn phase6_7_section(live: Live) -> View {
    use crate::shell::ai_ui::{
        AI_API_KEY, AI_CONSENT_MODE, AI_ENDPOINT_URL, AI_MODEL_NAME, AI_PROVIDER_KIND,
        AI_REDACTION_MODE, AI_SESSION_BUDGET, LENS_ONLINE_ENABLED,
    };
    use crate::shell::forms_sign_ui::{TSA_REMOTE_CONSENT, TSA_REMOTE_URL};
    use crate::shell::pro_pdf_ui::{CAD_BACKGROUND_MODE, CAD_ENHANCE_MODE, CAD_LINE_WEIGHTS};

    let consent_picker = [
        ("disabled", "settings.ai.consent_off"),
        ("local", "settings.ai.consent_local"),
        ("prompt", "settings.ai.consent_prompt"),
        ("cloud", "settings.ai.consent_cloud"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(AI_CONSENT_MODE).unwrap_or("local") == id),
            move || live.change(|s| s.set_text(AI_CONSENT_MODE, Some(id))),
        ))
    });

    let provider_picker = [
        ("ollama", "settings.ai.provider_ollama"),
        ("llama_server", "settings.ai.provider_llama"),
        ("openai", "settings.ai.provider_openai"),
        ("anthropic", "settings.ai.provider_anthropic"),
        ("gemini", "settings.ai.provider_gemini"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(AI_PROVIDER_KIND).unwrap_or("ollama") == id),
            move || live.change(|s| s.set_text(AI_PROVIDER_KIND, Some(id))),
        ))
    });

    let cad_picker = [
        ("auto", "settings.cad.auto"),
        ("on", "settings.cad.on"),
        ("off", "settings.cad.off"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(CAD_ENHANCE_MODE).unwrap_or("auto") == id),
            move || live.change(|s| s.set_text(CAD_ENHANCE_MODE, Some(id))),
        ))
    });

    let cad_bg_picker = [
        ("light", "settings.cad.light"),
        ("blueprint", "settings.cad.blueprint"),
    ]
    .into_iter()
    .fold(segmented(), |row, (id, label)| {
        row.child(segment(
            label,
            move || live.read(|s| s.text(CAD_BACKGROUND_MODE).unwrap_or("light") == id),
            move || live.change(|s| s.set_text(CAD_BACKGROUND_MODE, Some(id))),
        ))
    });

    let text_setting = |key: &'static str, label: &'static str, placeholder: &'static str| {
        let draft = signal(None::<String>);
        let commit = move || {
            let Some(typed) = draft.get_untracked() else {
                return;
            };
            draft.set(None);
            let trimmed = typed.trim();
            live.change(|s| s.set_text(key, (!trimmed.is_empty()).then_some(trimmed)));
        };
        input()
            .value(move || {
                draft
                    .get()
                    .unwrap_or_else(|| live.read(|s| s.text(key).unwrap_or_default().to_owned()))
            })
            .on_change(move |text: &str| draft.set(Some(text.to_owned())))
            .on_submit(move |_: &str| commit())
            .on_blur(commit)
            .placeholder(t!(placeholder))
            .label(t!(label))
            .w(px(230.0))
            .h(px(32.0))
            .px_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || theme().input)
            .text_size(px(12.0))
    };

    section("settings.section.ai_pro")
        .child(row(
            "settings.ai.consent",
            "settings.ai.consent_hint",
            consent_picker,
        ))
        .child(row(
            "settings.ai.provider",
            "settings.ai.provider_hint",
            provider_picker,
        ))
        .child(row(
            "settings.ai.endpoint",
            "settings.ai.endpoint_hint",
            text_setting(
                AI_ENDPOINT_URL,
                "settings.ai.endpoint",
                "settings.ai.endpoint_placeholder",
            ),
        ))
        .child(row(
            "settings.ai.model",
            "settings.ai.model_hint",
            text_setting(
                AI_MODEL_NAME,
                "settings.ai.model",
                "settings.ai.model_placeholder",
            ),
        ))
        .child(row(
            "settings.ai.api_key",
            "settings.ai.api_key_hint",
            text_setting(
                AI_API_KEY,
                "settings.ai.api_key",
                "settings.ai.api_key_placeholder",
            ),
        ))
        .child(row(
            "settings.ai.redact",
            "settings.ai.redact_hint",
            on_off(
                "settings.ai.redact",
                move || live.read(|s| s.text(AI_REDACTION_MODE).unwrap_or("redact") == "redact"),
                move |on| {
                    live.change(|s| {
                        s.set_text(AI_REDACTION_MODE, Some(if on { "redact" } else { "raw" }))
                    });
                },
            ),
        ))
        .child(row(
            "settings.ai.budget",
            "settings.ai.budget_hint",
            number_field(
                live,
                "settings.ai.budget",
                AI_SESSION_BUDGET,
                0.0,
                (0.0, 2_000_000.0),
                5_000.0,
                "settings.ai.budget_unit",
            ),
        ))
        .child(row(
            "settings.lens.online",
            "settings.lens.online_hint",
            on_off(
                "settings.lens.online",
                move || live.read(|s| s.flag(LENS_ONLINE_ENABLED).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(LENS_ONLINE_ENABLED, Some(on))),
            ),
        ))
        .child(row(
            "settings.cad.mode",
            "settings.cad.mode_hint",
            cad_picker,
        ))
        .child(row(
            "settings.cad.background",
            "settings.cad.background_hint",
            cad_bg_picker,
        ))
        .child(row(
            "settings.cad.weights",
            "settings.cad.weights_hint",
            on_off(
                "settings.cad.weights",
                move || live.read(|s| s.text(CAD_LINE_WEIGHTS).unwrap_or("respect") == "uniform"),
                move |on| {
                    live.change(|s| {
                        s.set_text(
                            CAD_LINE_WEIGHTS,
                            Some(if on { "uniform" } else { "respect" }),
                        )
                    });
                },
            ),
        ))
        .child(row(
            "settings.tsa.consent",
            "settings.tsa.consent_hint",
            on_off(
                "settings.tsa.consent",
                move || live.read(|s| s.flag(TSA_REMOTE_CONSENT).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(TSA_REMOTE_CONSENT, Some(on))),
            ),
        ))
        .child(row(
            "settings.tsa.url",
            "settings.tsa.url_hint",
            text_setting(
                TSA_REMOTE_URL,
                "settings.tsa.url",
                "settings.tsa.url_placeholder",
            ),
        ))
}

/// Phase 8/9 sandbox worker status and `SecurityLimits` preferences (`K.22`, `K.23`).
fn sandbox_section(live: Live) -> View {
    use crate::shell::ecosystem_ui::{
        SANDBOX_MAX_PAGES, SANDBOX_MEMORY_CAP_MB, SANDBOX_STRICT_MODE, SANDBOX_TIMEOUT_SECS,
        sandbox_status_summary,
    };

    section("settings.section.sandbox")
        .child(row(
            "settings.sandbox.status",
            "settings.sandbox.status_hint",
            text(move || sandbox_status_summary(live))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        ))
        .child(row(
            "settings.sandbox.strict",
            "settings.sandbox.strict_hint",
            on_off(
                "settings.sandbox.strict",
                move || live.read(|s| s.flag(SANDBOX_STRICT_MODE).unwrap_or(false)),
                move |on| live.change(|s| s.set_flag(SANDBOX_STRICT_MODE, Some(on))),
            ),
        ))
        .child(row(
            "settings.sandbox.max_pages",
            "settings.sandbox.max_pages_hint",
            number_field(
                live,
                "settings.sandbox.max_pages",
                SANDBOX_MAX_PAGES,
                50_000.0,
                (100.0, 200_000.0),
                1_000.0,
                "settings.sandbox.pages_unit",
            ),
        ))
        .child(row(
            "settings.sandbox.memory_mb",
            "settings.sandbox.memory_mb_hint",
            number_field(
                live,
                "settings.sandbox.memory_mb",
                SANDBOX_MEMORY_CAP_MB,
                1024.0,
                (128.0, 8192.0),
                128.0,
                "settings.sandbox.mb_unit",
            ),
        ))
        .child(row(
            "settings.sandbox.timeout_s",
            "settings.sandbox.timeout_s_hint",
            number_field(
                live,
                "settings.sandbox.timeout_s",
                SANDBOX_TIMEOUT_SECS,
                30.0,
                (5.0, 300.0),
                5.0,
                "settings.sandbox.secs_unit",
            ),
        ))
}

/// Optional packs management (K.21): installed packs, signed index settings, `.expack` import, and removal.
fn packs_section(live: Live) -> View {
    let state = PacksState::new(live);
    let url_draft = signal(None::<String>);
    let key_draft = signal(None::<String>);
    let commit_url = move || {
        let Some(typed) = url_draft.get_untracked() else {
            return;
        };
        url_draft.set(None);
        let trimmed = typed.trim();
        live.change(|s| s.set_text(PACKS_INDEX_URL, (!trimmed.is_empty()).then_some(trimmed)));
    };
    let commit_key = move || {
        let Some(typed) = key_draft.get_untracked() else {
            return;
        };
        key_draft.set(None);
        let trimmed = typed.trim();
        live.change(|s| s.set_text(PACKS_KEY_HEX, (!trimmed.is_empty()).then_some(trimmed)));
    };
    let url_field = input()
        .value(move || {
            url_draft.get().unwrap_or_else(|| {
                live.read(|s| s.text(PACKS_INDEX_URL).unwrap_or_default().to_owned())
            })
        })
        .on_change(move |text: &str| url_draft.set(Some(text.to_owned())))
        .on_submit(move |_: &str| commit_url())
        .on_blur(commit_url)
        .placeholder(t!("settings.packs.index_placeholder"))
        .label(t!("settings.packs.index_url"))
        .w(px(230.0))
        .h(px(32.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(12.0))
        .text_color(move || theme().popover_foreground);
    let key_field = input()
        .value(move || {
            key_draft.get().unwrap_or_else(|| {
                live.read(|s| s.text(PACKS_KEY_HEX).unwrap_or_default().to_owned())
            })
        })
        .on_change(move |text: &str| key_draft.set(Some(text.to_owned())))
        .on_submit(move |_: &str| commit_key())
        .on_blur(commit_key)
        .placeholder(t!("settings.packs.key_placeholder"))
        .label(t!("settings.packs.verifying_key"))
        .w(px(230.0))
        .h(px(32.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(12.0))
        .text_color(move || theme().popover_foreground);
    let actions = view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(crate::tabs::action_button(
            "settings.packs.install_file",
            crate::tabs::Tone::Plain,
            move || state.install_from_file(),
        ))
        .child(crate::tabs::action_button(
            "settings.packs.refresh_index",
            crate::tabs::Tone::Plain,
            move || state.refresh_index(),
        ))
        .child(dynamic(move || {
            if state.is_busy() {
                view().child(crate::tabs::action_button(
                    "settings.packs.cancel",
                    crate::tabs::Tone::Plain,
                    move || state.cancel_download(),
                ))
            } else {
                view()
            }
        }));
    let status_bar = dynamic(move || match state.status() {
        Some(message) => view().pt_1().child(
            text(message)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        ),
        None => view(),
    });
    let installed_list = dynamic(move || {
        let items = state.installed();
        if items.is_empty() {
            return view().py_1().child(
                text(move || t!("settings.packs.none_installed"))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            );
        }
        items.into_iter().fold(view().gap_1().pt_1(), |list, item| {
            let id = item.manifest.id.clone();
            let title = item.manifest.title.clone();
            let meta = format!("v{} - {}", item.manifest.version, item.manifest.id);
            list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(
                        view()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                text(title)
                                    .text_size(px(13.0))
                                    .text_color(move || theme().popover_foreground),
                            )
                            .child(
                                text(meta)
                                    .text_size(px(11.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(crate::tabs::action_button(
                        "settings.packs.remove",
                        crate::tabs::Tone::Plain,
                        move || state.remove(id.clone()),
                    )),
            )
        })
    });
    let available_list = dynamic(move || {
        let entries = state.available();
        if entries.is_empty() {
            return view();
        }
        entries.into_iter().fold(
            view().gap_1().pt_2().child(
                text(move || t!("settings.packs.available_title"))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().muted_foreground),
            ),
            |list, entry| {
                let title = entry.title.clone();
                let meta = format!("v{} - {}", entry.version, format_size(entry.size));
                list.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .child(
                            view()
                                .flex_1()
                                .gap_0p5()
                                .child(
                                    text(title)
                                        .text_size(px(13.0))
                                        .text_color(move || theme().popover_foreground),
                                )
                                .child(
                                    text(meta)
                                        .text_size(px(11.0))
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(crate::tabs::action_button(
                            "settings.packs.download",
                            crate::tabs::Tone::Plain,
                            move || state.download(entry.clone()),
                        )),
                )
            },
        )
    });
    section("settings.section.packs")
        .child(row(
            "settings.packs.index_url",
            "settings.packs.index_url_hint",
            url_field,
        ))
        .child(row(
            "settings.packs.verifying_key",
            "settings.packs.verifying_key_hint",
            key_field,
        ))
        .child(row(
            "settings.packs.actions",
            "settings.packs.actions_hint",
            actions,
        ))
        .child(status_bar)
        .child(installed_list)
        .child(available_list)
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

/// The language segments: the system's, then each language the app has, named in its own language (U.7).
fn language_choice(live: Live) -> View {
    use crate::l10n::{LANGUAGE_KEY, LANGUAGES};
    let chosen = move || live.read(|s| s.text(LANGUAGE_KEY).map(str::to_owned));
    let system = segment(
        "settings.language.system",
        move || chosen().is_none(),
        move || live.change(|s| s.set_text(LANGUAGE_KEY, None)),
    );
    LANGUAGES
        .iter()
        .fold(segmented().child(system), |group, &(tag, name)| {
            group.child(named_segment(
                name,
                move || chosen().as_deref() == Some(tag),
                move || live.change(|s| s.set_text(LANGUAGE_KEY, Some(tag))),
            ))
        })
}

/// A yes-or-no setting's switch.
pub(crate) fn on_off(
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

/// The frame of a row of segments, one of which is chosen; they wrap when they don't fit.
pub(crate) fn segmented() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(move || theme().muted)
}

/// One segment: `chosen` says whether it is the one; pressing it chooses it.
pub(crate) fn segment(
    label: &'static str,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    segment_of(move || t!(label), chosen, choose)
}

/// A segment whose label is already in the reader's words (a language's own name, U.7).
fn named_segment(
    name: &'static str,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    segment_of(move || name.to_owned(), chosen, choose)
}

pub(crate) fn segment_of(
    label: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(label())
        .on_change(move |_: &bool| choose())
        .h(px(28.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(text(label).text_size(px(13.0)).text_color(move || {
            if chosen() {
                theme().popover_foreground
            } else {
                theme().muted_foreground
            }
        }))
}

/// Segment control for a dynamic `String` label that cannot be `Copy`-captured.
pub(crate) fn segment_owned(
    label: String,
    chosen: impl Fn() -> bool + Copy + 'static,
    choose: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(label.clone())
        .on_change(move |_: &bool| choose())
        .h(px(28.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(text(label).text_size(px(13.0)).text_color(move || {
            if chosen() {
                theme().popover_foreground
            } else {
                theme().muted_foreground
            }
        }))
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
