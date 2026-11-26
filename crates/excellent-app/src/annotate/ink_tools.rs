//! Phase 4 Ink & Drawing UI actions and dialogs:
//! - H.13 Handwriting Straighten
//! - H.15 Bulk Ink Restyle
//! - H.16 Fill & Gradient
//! - H.17 Eyedropper & Loupe
//! - H.18 Saved Color Palettes
//! - H.22 Ink Stroke Replay
//! - H.26 Convert Ink Shapes to Vector PDF Annotations

use std::time::Duration;

use excellent_core::annot::{Annotation, Boxed, Path, Shape};
use excellent_core::edit::{Change, Operation};
use excellent_features::annotate::colors::{self, Tool, hex, parse_hex};
use excellent_features::ink::eyedropper::{
    SampleArea, SampledColor, sample_page_point, sample_pixmap,
};
use excellent_features::ink::palettes::{ColorPalette, PaletteLibrary, PaletteOrigin, Swatch};
use excellent_features::ink::replay::{
    PageInkRecord, PlaybackSpeed, PlaybackState, ReplayPlayer, SPEED_PRESETS,
};
use excellent_features::ink::restyle::{
    ColorMatch, InkCategory, RestyleSpec, StrokeFilter, WidthChange, restyle_operation,
};
use excellent_features::ink::vectorize::convert_operation;
use excellent_ink::erase::Point as InkPt;
use excellent_ink::fill::{
    BorderStroke, ColorStop, ExtendMode, Fill, FillRule, LinearGradient, RadialGradient, SolidFill,
    build_appearance, is_closed_stroke,
};
use excellent_ink::pens::PenKind;
use excellent_ink::straighten::{LineSpacing, StraightenOptions, straighten_ink};
use excellent_ink::stroke::InkPoint;
use excellent_mupdf::{Background, DeviceRect, Matrix, PagePoint};
use nsh_base::{SliderValue, button, input, slider, theme};
use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{Live, write_atomically};
use crate::settings_window::{SLIDER_RANGE, SLIDER_THUMB, SLIDER_TRACK, segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// Settings key storing saved color palettes JSON (H.18).
pub(crate) const PALETTES_KEY: &str = "ink_palettes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestyleScope {
    Selection,
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillKind {
    Solid,
    Linear,
    Radial,
    Clear,
}

/// Reads the palette library from `settings.toml` (H.18).
pub(crate) fn read_palettes(live: Live) -> PaletteLibrary {
    live.read(|s| {
        s.value(PALETTES_KEY)
            .and_then(|v| serde_json::to_value(v).ok())
            .map(|json| PaletteLibrary::from_settings_json(&json))
            .unwrap_or_default()
    })
}

fn write_palettes(live: Live, lib: &PaletteLibrary) {
    let val = toml::Value::try_from(lib.to_settings_json()).ok();
    live.change(|s| s.set_value(PALETTES_KEY, val));
}

impl AnnotateState {
    /// Current 0-based page index for ink tools (visible page fallback to `current_page`).
    pub(crate) fn active_page(self) -> usize {
        untracked(|| {
            self.shown_pages
                .with(|pages| pages.first().copied())
                .unwrap_or_else(|| self.current_page.get())
        })
    }

    /// Straightens selected ink strokes (or current page ink if none selected) into level baselines (H.13).
    pub(crate) fn straighten_ink_selection(self) {
        let stack = self.layer_stack();
        let page = self.active_page();
        let candidates = {
            let sel = self.selection();
            if sel.is_empty() { self.page(page) } else { sel }
        };
        let changes: Vec<Change> = candidates
            .into_iter()
            .filter(|a| !stack.is_locked(a))
            .filter_map(straighten_annotation)
            .collect();
        if changes.is_empty() {
            show_message(
                t!("ink_tools.straighten.title"),
                t!("ink_tools.straighten.empty"),
            );
            return;
        }
        self.editing
            .apply(&Operation::new(t!("command.straighten_ink"), changes));
    }

    /// Opens the bulk ink restyle dialog (H.15).
    pub(crate) fn open_restyle(self) {
        self.restyle_open.set(true);
    }

    /// Opens the fill & gradient dialog (H.16).
    pub(crate) fn open_fill(self) {
        self.fill_open.set(true);
    }

    /// Toggles the page eyedropper color sampler (H.17).
    pub(crate) fn toggle_eyedropper(self) {
        let next = !untracked(|| self.eyedropper_active.get());
        self.eyedropper_active.set(next);
        if next {
            self.eraser.set(None);
            self.lasso.set(false);
            self.placing.set(None);
        }
    }

    /// Samples exact pre-dark-mode pixel color from the PDF display list at a page click (H.17).
    pub(super) fn sample_eyedropper(self, data: &JsonValue) {
        let Some((page, point)) = parse_press_point(data) else {
            return;
        };
        let Some(document) = self.editing.document() else {
            return;
        };
        let area = untracked(|| self.eyedropper_area.get());
        let sampled = {
            let Ok(shared) = document.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            sample_from_document(&mupdf, page, point, area)
        };
        let Some((color, loupe)) = sampled else {
            return;
        };
        self.eyedropper_sample.set(Some((color, loupe)));
        super::favorites::set_pen_color(self.live, color.to_hex());
        self.live.change(|s| {
            let mut c = colors::Colors::from_json(
                &s.value(super::colors::KEY)
                    .and_then(|v| serde_json::to_value(v).ok())
                    .unwrap_or_default(),
            );
            c.choose_color(Tool::Shape, color.to_hex());
            if let Ok(toml_val) = toml::Value::try_from(c.to_json()) {
                s.set_value(super::colors::KEY, Some(toml_val));
            }
        });
    }

    /// Opens the saved color palettes dialog (H.18).
    pub(crate) fn open_palettes(self) {
        self.palettes_open.set(true);
    }

    /// Opens the stroke replay transport dialog (H.22).
    pub(crate) fn open_replay(self) {
        self.replay_open.set(true);
    }

    /// Converts recognized shape strokes in selection (or current page) into vector PDF annotations (H.26).
    pub(crate) fn vectorize_shapes(self) {
        let stack = self.layer_stack();
        let page = self.active_page();
        let candidates = {
            let sel = self.selection();
            if sel.is_empty() { self.page(page) } else { sel }
        };
        let Some(result) = convert_operation(&candidates, Some(&stack)) else {
            show_message(
                t!("ink_tools.vectorize.title"),
                t!("ink_tools.vectorize.none"),
            );
            return;
        };
        let converted = result.created.len();
        if self.editing.apply(&result.operation) {
            let targets = result
                .created
                .into_iter()
                .map(|a| excellent_features::annotate::notes::Target {
                    page: a.page,
                    id: a.id,
                })
                .collect();
            self.selected.set(targets);
            show_message(
                t!("ink_tools.vectorize.title"),
                t!(
                    "ink_tools.vectorize.done",
                    converted = converted,
                    kept = result.unrecognized_strokes
                ),
            );
        }
    }
}

fn parse_press_point(data: &JsonValue) -> Option<(usize, PagePoint)> {
    excellent_features::annotate::notes::parse_press(data)
}

/// Samples the color and a 7×7 magnifier loupe from `mupdf` at `point` without dark-mode recoloring (H.17).
fn sample_from_document(
    mupdf: &excellent_mupdf::Document,
    page: usize,
    point: PagePoint,
    area: SampleArea,
) -> Option<(SampledColor, Vec<Option<SampledColor>>)> {
    let loaded = mupdf.load_page(page).ok()?;
    let list = loaded.display_list(None).ok()?;
    let scale = 2.0_f32;
    let picked = sample_page_point(&list, point, scale, area).ok()?;
    let ctm = Matrix::scale(scale, scale);
    let dev_x = (point.x * scale).floor() as i32;
    let dev_y = (point.y * scale).floor() as i32;
    let radius = 3_i32;
    let patch = DeviceRect {
        x0: dev_x - radius,
        y0: dev_y - radius,
        x1: dev_x + radius + 1,
        y1: dev_y + radius + 1,
    };
    let mut loupe = Vec::new();
    if let Ok(pixmap) = list.render_area(&ctm, patch, Background::White, None) {
        let side = (radius * 2 + 1) as u32;
        for py in 0..side {
            for px in 0..side {
                loupe.push(sample_pixmap(&pixmap, px, py, SampleArea::SinglePixel));
            }
        }
    }
    Some((picked, loupe))
}

/// Straightens an ink annotation using rigid Euclidean transforms while keeping pressure widths (H.13).
fn straighten_annotation(annotation: Annotation) -> Option<Change> {
    let Shape::Ink(ref raw_strokes) = annotation.shape else {
        return None;
    };
    if raw_strokes.is_empty() {
        return None;
    }
    let ink_strokes: Vec<Vec<InkPoint>> = raw_strokes
        .iter()
        .enumerate()
        .map(|(idx, pts)| {
            let widths = annotation.widths.get(idx);
            pts.iter()
                .enumerate()
                .map(|(k, p)| InkPoint {
                    x: p.x,
                    y: p.y,
                    width: widths
                        .and_then(|list| list.get(k).copied())
                        .unwrap_or(annotation.style.width),
                })
                .collect()
        })
        .collect();
    let (leveled, _) = straighten_ink(
        &ink_strokes,
        StraightenOptions {
            spacing: LineSpacing::KeepBaselines,
            ..StraightenOptions::new()
        },
    );
    let new_strokes: Vec<Vec<PagePoint>> = leveled
        .into_iter()
        .map(|stroke| {
            stroke
                .into_iter()
                .map(|pt| PagePoint { x: pt.x, y: pt.y })
                .collect()
        })
        .collect();
    if new_strokes == *raw_strokes {
        return None;
    }
    let mut after = annotation;
    after.shape = Shape::Ink(new_strokes);
    Some(Change::UpdateAnnotation(after))
}

fn color_u32_to_rgb(c: u32) -> [f32; 3] {
    [
        f32::from(((c >> 16) & 0xff) as u8) / 255.0,
        f32::from(((c >> 8) & 0xff) as u8) / 255.0,
        f32::from((c & 0xff) as u8) / 255.0,
    ]
}

/// Bulk Restyle dialog (H.15): restyles selected strokes or matching ink across the page in one undo step.
pub(crate) fn restyle_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.restyle_open.set(false);
    let scope = signal(if untracked(|| state.has_selection()) {
        RestyleScope::Selection
    } else {
        RestyleScope::Page
    });
    let filter_hex = signal(String::new());
    let category_filter = signal(None::<InkCategory>);
    let new_color_hex = signal(String::from("#1d4ed8"));
    let apply_color = signal(true);
    let width_mode = signal(0_u8); // 0=keep, 1=thinner, 2=thicker, 3=set 2pt
    let target_pen = signal(None::<PenKind>);
    let opacity_pct = signal(100.0_f32);
    let apply_opacity = signal(false);

    let apply = move || {
        let stack = state.layer_stack();
        let page = state.active_page();
        let annots = state.page(page);
        let mut filter = match untracked(|| scope.get()) {
            RestyleScope::Selection => {
                let ids = state
                    .selection()
                    .into_iter()
                    .map(|a| a.id)
                    .collect::<Vec<_>>();
                StrokeFilter::selection(ids)
            }
            RestyleScope::Page => StrokeFilter {
                page: Some(page),
                ..StrokeFilter::default()
            },
        };
        if let Some(c) = untracked(|| filter_hex.get())
            .trim()
            .strip_prefix('#')
            .and_then(|s| parse_hex(&format!("#{s}")))
            .or_else(|| parse_hex(untracked(|| filter_hex.get()).trim()))
        {
            filter.color = Some(ColorMatch::from_hex(c));
        }
        filter.category = untracked(|| category_filter.get());

        let mut spec = RestyleSpec::default();
        if untracked(|| apply_color.get())
            && let Some(c) = parse_hex(&untracked(|| new_color_hex.get()))
        {
            spec.color = Some(color_u32_to_rgb(c));
        }
        spec.pen_kind = untracked(|| target_pen.get());
        if untracked(|| apply_opacity.get()) {
            spec.opacity = Some(untracked(|| opacity_pct.get()) / 100.0);
        }
        spec.width = match untracked(|| width_mode.get()) {
            1 => Some(WidthChange::Scale(0.75)),
            2 => Some(WidthChange::Scale(1.35)),
            3 => Some(WidthChange::Set(2.0)),
            _ => None,
        };
        match restyle_operation(&annots, &filter, &spec, Some(&stack)) {
            Some(op) => {
                state.editing.apply(&op);
                close();
            }
            None => show_message(t!("ink_tools.restyle.title"), t!("ink_tools.restyle.none")),
        }
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("ink_tools.restyle.title"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("ink_tools.restyle.scope"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.restyle.selected"),
                    move || scope.get() == RestyleScope::Selection,
                    move || scope.set(RestyleScope::Selection),
                ))
                .child(segment_of(
                    move || t!("ink_tools.restyle.page"),
                    move || scope.get() == RestyleScope::Page,
                    move || scope.set(RestyleScope::Page),
                )),
        )
        .child(caption("ink_tools.restyle.filter_color"))
        .child(
            field(input().value(move || filter_hex.get()))
                .on_change(move |s: &str| filter_hex.set(s.to_owned()))
                .placeholder(t!("ink_tools.restyle.filter_placeholder"))
                .label(t!("ink_tools.restyle.filter_color")),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.restyle.cat_all"),
                    move || category_filter.get().is_none(),
                    move || category_filter.set(None),
                ))
                .child(segment_of(
                    move || t!("ink_tools.restyle.cat_pen"),
                    move || category_filter.get() == Some(InkCategory::PenOnly),
                    move || category_filter.set(Some(InkCategory::PenOnly)),
                ))
                .child(segment_of(
                    move || t!("ink_tools.restyle.cat_highlighter"),
                    move || category_filter.get() == Some(InkCategory::HighlighterOnly),
                    move || category_filter.set(Some(InkCategory::HighlighterOnly)),
                )),
        )
        .child(caption("ink_tools.restyle.new_color"))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(action_button(
                    "ink_tools.restyle.toggle_color",
                    Tone::Plain,
                    move || {
                        apply_color.update(|v| *v = !*v);
                    },
                ))
                .child(
                    view().flex_1().child(
                        field(input().value(move || new_color_hex.get()))
                            .on_change(move |s: &str| new_color_hex.set(s.to_owned()))
                            .label(t!("ink_tools.restyle.new_color")),
                    ),
                ),
        )
        .child(caption("ink_tools.restyle.width"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.keep"),
                    move || width_mode.get() == 0,
                    move || width_mode.set(0),
                ))
                .child(segment_of(
                    move || t!("command.thinner"),
                    move || width_mode.get() == 1,
                    move || width_mode.set(1),
                ))
                .child(segment_of(
                    move || t!("command.thicker"),
                    move || width_mode.get() == 2,
                    move || width_mode.set(2),
                ))
                .child(segment_of(
                    move || String::from("2 pt"),
                    move || width_mode.get() == 3,
                    move || width_mode.set(3),
                )),
        )
        .child(caption("ink_tools.restyle.pen"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.keep"),
                    move || target_pen.get().is_none(),
                    move || target_pen.set(None),
                ))
                .child(segment_of(
                    move || t!("command.pen_ballpoint"),
                    move || target_pen.get() == Some(PenKind::Ballpoint),
                    move || target_pen.set(Some(PenKind::Ballpoint)),
                ))
                .child(segment_of(
                    move || t!("command.pen_fountain"),
                    move || target_pen.get() == Some(PenKind::Fountain),
                    move || target_pen.set(Some(PenKind::Fountain)),
                ))
                .child(segment_of(
                    move || t!("command.pen_highlighter"),
                    move || target_pen.get() == Some(PenKind::Highlighter),
                    move || target_pen.set(Some(PenKind::Highlighter)),
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(action_button(
                    "ink_tools.restyle.toggle_opacity",
                    Tone::Plain,
                    move || apply_opacity.update(|v| *v = !*v),
                ))
                .child(
                    slider()
                        .min(10.0)
                        .max(100.0)
                        .step(5.0)
                        .value(move || SliderValue::Single(f64::from(opacity_pct.get())))
                        .on_change(move |v: &SliderValue| {
                            if let SliderValue::Single(pct) = v {
                                opacity_pct.set(*pct as f32);
                                apply_opacity.set(true);
                            }
                        })
                        .track_class(SLIDER_TRACK)
                        .range_class(SLIDER_RANGE)
                        .thumb_class(SLIDER_THUMB)
                        .flex_1(),
                )
                .child(
                    text(move || format!("{:.0}%", opacity_pct.get()))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("ink_tools.apply", apply)),
        );

    list_dialog(t!("ink_tools.restyle.title"), close, card(body))
}

/// Fill & Gradient dialog (H.16): generates ISO 32000-1 fill appearances for closed ink/shape annotations.
pub(crate) fn fill_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.fill_open.set(false);
    let kind = signal(FillKind::Linear);
    let start_hex = signal(String::from("#fef08a"));
    let end_hex = signal(String::from("#f59e0b"));
    let even_odd = signal(false);
    let extend_both = signal(true);
    let opacity_pct = signal(65.0_f32);
    let preview_stream = signal(Option::<String>::None);

    let apply = move || {
        let sel = state.selection();
        if sel.is_empty() {
            show_message(
                t!("ink_tools.fill.title"),
                t!("ink_tools.fill.no_selection"),
            );
            return;
        }
        let chosen_kind = untracked(|| kind.get());
        if chosen_kind == FillKind::Clear {
            let changes: Vec<Change> = sel
                .into_iter()
                .map(|mut a| {
                    a.fill = None;
                    Change::UpdateAnnotation(a)
                })
                .collect();
            state
                .editing
                .apply(&Operation::new(t!("ink_tools.fill.clear"), changes));
            close();
            return;
        }
        let c0 = parse_hex(&untracked(|| start_hex.get())).unwrap_or(0xfe_f0_8a);
        let c1 = parse_hex(&untracked(|| end_hex.get())).unwrap_or(0xf5_9e_0b);
        let rgb0 = color_u32_to_rgb(c0);
        let rgb1 = color_u32_to_rgb(c1);
        let alpha = (untracked(|| opacity_pct.get()) / 100.0).clamp(0.05, 1.0);
        let rule = if untracked(|| even_odd.get()) {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        };
        let extend = if untracked(|| extend_both.get()) {
            ExtendMode::Both
        } else {
            ExtendMode::None
        };

        let mut changes = Vec::new();
        let mut last_xobj = None;
        for a in sel {
            let pts: Vec<InkPt> = match &a.shape {
                Shape::Ink(strokes) => {
                    let first = strokes.first().cloned().unwrap_or_default();
                    let ink_pts: Vec<InkPt> = first
                        .into_iter()
                        .map(|p| InkPt { x: p.x, y: p.y })
                        .collect();
                    if !is_closed_stroke(&ink_pts) {
                        continue;
                    }
                    ink_pts
                }
                Shape::Boxed(Boxed::Square | Boxed::Circle, r) => vec![
                    InkPt { x: r.x0, y: r.y0 },
                    InkPt { x: r.x1, y: r.y0 },
                    InkPt { x: r.x1, y: r.y1 },
                    InkPt { x: r.x0, y: r.y1 },
                ],
                Shape::Path(Path::Polygon, verts) => {
                    verts.iter().map(|p| InkPt { x: p.x, y: p.y }).collect()
                }
                _ => continue,
            };
            if pts.len() < 3 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            );
            for &p in &pts {
                x0 = x0.min(p.x);
                y0 = y0.min(p.y);
                x1 = x1.max(p.x);
                y1 = y1.max(p.y);
            }
            let fill = match chosen_kind {
                FillKind::Solid => Fill::Solid(SolidFill::new(rgb0, alpha)),
                FillKind::Linear => Fill::Linear(LinearGradient {
                    start: InkPt { x: x0, y: y0 },
                    end: InkPt {
                        x: x1.max(x0 + 1.0),
                        y: y1,
                    },
                    stops: vec![ColorStop::new(0.0, rgb0), ColorStop::new(1.0, rgb1)],
                    extend,
                    opacity: alpha,
                }),
                FillKind::Radial => {
                    let cx = (x0 + x1) * 0.5;
                    let cy = (y0 + y1) * 0.5;
                    let radius = ((x1 - x0).hypot(y1 - y0) * 0.5).max(4.0);
                    Fill::Radial(RadialGradient {
                        start_center: InkPt { x: cx, y: cy },
                        start_radius: 0.0,
                        end_center: InkPt { x: cx, y: cy },
                        end_radius: radius,
                        stops: vec![ColorStop::new(0.0, rgb0), ColorStop::new(1.0, rgb1)],
                        extend,
                        opacity: alpha,
                    })
                }
                FillKind::Clear => unreachable!(),
            };
            let border = a.style.color.map(|color| BorderStroke {
                color,
                width: a.style.width,
                opacity: a.style.opacity,
            });
            if let Ok(stream) = build_appearance(&pts, &fill, rule, border, 792.0) {
                last_xobj = Some(stream.to_form_xobject());
                let mut updated = a;
                updated.fill = Some(rgb0);
                changes.push(Change::UpdateAnnotation(updated));
            }
        }
        if changes.is_empty() {
            show_message(t!("ink_tools.fill.title"), t!("ink_tools.fill.not_closed"));
            return;
        }
        if let Some(xobj) = last_xobj {
            preview_stream.set(Some(xobj));
        }
        state
            .editing
            .apply(&Operation::new(t!("command.ink_fill"), changes));
    };

    let body = view()
        .w(px(440.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("ink_tools.fill.title"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("ink_tools.fill.kind"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.fill.solid"),
                    move || kind.get() == FillKind::Solid,
                    move || kind.set(FillKind::Solid),
                ))
                .child(segment_of(
                    move || t!("ink_tools.fill.linear"),
                    move || kind.get() == FillKind::Linear,
                    move || kind.set(FillKind::Linear),
                ))
                .child(segment_of(
                    move || t!("ink_tools.fill.radial"),
                    move || kind.get() == FillKind::Radial,
                    move || kind.set(FillKind::Radial),
                ))
                .child(segment_of(
                    move || t!("ink_tools.fill.clear"),
                    move || kind.get() == FillKind::Clear,
                    move || kind.set(FillKind::Clear),
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(caption("ink_tools.fill.start"))
                        .child(
                            field(input().value(move || start_hex.get()))
                                .on_change(move |s: &str| start_hex.set(s.to_owned()))
                                .label(t!("ink_tools.fill.start")),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(caption("ink_tools.fill.end"))
                        .child(
                            field(input().value(move || end_hex.get()))
                                .on_change(move |s: &str| end_hex.set(s.to_owned()))
                                .label(t!("ink_tools.fill.end")),
                        ),
                ),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.fill.nonzero"),
                    move || !even_odd.get(),
                    move || even_odd.set(false),
                ))
                .child(segment_of(
                    move || t!("ink_tools.fill.evenodd"),
                    move || even_odd.get(),
                    move || even_odd.set(true),
                ))
                .child(segment_of(
                    move || t!("ink_tools.fill.extend"),
                    move || extend_both.get(),
                    move || extend_both.update(|v| *v = !*v),
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    slider()
                        .min(10.0)
                        .max(100.0)
                        .step(5.0)
                        .value(move || SliderValue::Single(f64::from(opacity_pct.get())))
                        .on_change(move |v: &SliderValue| {
                            if let SliderValue::Single(pct) = v {
                                opacity_pct.set(*pct as f32);
                            }
                        })
                        .track_class(SLIDER_TRACK)
                        .range_class(SLIDER_RANGE)
                        .thumb_class(SLIDER_THUMB)
                        .flex_1(),
                )
                .child(
                    text(move || format!("{:.0}%", opacity_pct.get()))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(dynamic(move || match preview_stream.get() {
            Some(stream) => view()
                .p_2()
                .rounded(px(6.0))
                .bg(move || theme().muted)
                .child(
                    text(stream.lines().take(4).collect::<Vec<_>>().join(" "))
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
            None => view(),
        }))
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("ink_tools.apply", apply)),
        );

    list_dialog(t!("ink_tools.fill.title"), close, card(body))
}

/// Compact floating bar shown when the Eyedropper is active (H.17).
pub(crate) fn eyedropper_bar(state: AnnotateState) -> impl IntoElement {
    dynamic(move || {
        if !state.eyedropper_active.get() {
            return view();
        }
        let sample = state.eyedropper_sample.get();
        let area = state.eyedropper_area.get();
        let loupe_view = match &sample {
            Some((color, loupe)) => {
                let swatch = view()
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .bg(Hsla::from(rgb(color.to_hex())));
                let grid = if loupe.len() == 49 {
                    (0..7).fold(view().gap_0(), |col, r| {
                        let row = (0..7).fold(view().flex_row().gap_0(), |row, c| {
                            let cell = loupe[r * 7 + c].unwrap_or(*color);
                            let mut px_cell = view()
                                .w(px(6.0))
                                .h(px(6.0))
                                .bg(Hsla::from(rgb(cell.to_hex())));
                            if r == 3 && c == 3 {
                                px_cell = px_cell.border_1().border_color(move || theme().ring);
                            }
                            row.child(px_cell)
                        });
                        col.child(row)
                    })
                } else {
                    view()
                };
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(swatch)
                    .child(
                        text(color.to_hex_string())
                            .text_size(px(12.0))
                            .tabular_nums()
                            .text_color(move || theme().foreground),
                    )
                    .child(grid)
            }
            None => view().child(
                text(move || t!("ink_tools.eyedropper.hint"))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            ),
        };
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py(px(6.0))
            .border_b_1()
            .border_b_color(move || theme().border)
            .bg(move || theme().muted)
            .child(
                text(move || t!("command.eyedropper"))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
            .child(
                segmented()
                    .child(segment_of(
                        move || String::from("1×1"),
                        move || area == SampleArea::SinglePixel,
                        move || state.eyedropper_area.set(SampleArea::SinglePixel),
                    ))
                    .child(segment_of(
                        move || String::from("3×3"),
                        move || area == SampleArea::Average3x3,
                        move || state.eyedropper_area.set(SampleArea::Average3x3),
                    )),
            )
            .child(loupe_view)
            .child(view().flex_1())
            .child(action_button("ink_tools.done", Tone::Plain, move || {
                state.eyedropper_active.set(false);
            }))
    })
}

/// Saved Color Palettes dialog (H.18): built-in & user palettes, swatch click applies to pen, duplicate/create/import/export.
pub(crate) fn palettes_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.palettes_open.set(false);
    let new_name = signal(String::new());
    let new_hexes = signal(String::from("#2563eb, #16a34a, #dc2626, #d97706"));
    let version = signal(0_u64);

    let create_user = move || {
        let name = untracked(|| new_name.get());
        let swatches: Vec<Swatch> = untracked(|| new_hexes.get())
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(parse_hex)
            .map(|c| Swatch::new(hex(c), c))
            .collect();
        let Ok(palette) = ColorPalette::new_user("", name, swatches) else {
            show_message(
                t!("ink_tools.palettes.title"),
                t!("ink_tools.palettes.invalid"),
            );
            return;
        };
        let mut lib = read_palettes(state.live);
        let id = palette.id.clone();
        if let Err(err) = lib.add_user(palette) {
            show_message(t!("ink_tools.palettes.title"), err.to_string());
            return;
        }
        let _ = lib.choose(&id);
        write_palettes(state.live, &lib);
        new_name.set(String::new());
        version.update(|v| *v += 1);
    };

    let import_json = move || {
        let dialog = OpenDialog::files()
            .title(t!("ink_tools.palettes.import"))
            .filter(FileFilter::new(t!("favorites.filter"), ["json"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                return;
            };
            let mut lib = read_palettes(state.live);
            match lib.import_bundle(&text) {
                Ok(_) => {
                    write_palettes(state.live, &lib);
                    version.update(|v| *v += 1);
                }
                Err(err) => show_message(t!("ink_tools.palettes.title"), err.to_string()),
            }
        });
    };

    let export_json = move || {
        let text = read_palettes(state.live).export_user_bundle();
        let dialog = SaveDialog::new()
            .file_name("ExcellentPDF palettes.json".to_owned())
            .filter(FileFilter::new(t!("favorites.filter"), ["json"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, text) {
                show_message(t!("ink_tools.palettes.title"), err.to_string());
            }
        });
    };

    let list_view = dynamic(move || {
        let _ = version.get();
        let lib = read_palettes(state.live);
        let active_id = lib.active().id.clone();
        lib.all().cloned().fold(view().gap_2(), |acc, palette| {
            let pid = palette.id.clone();
            let pid_dup = pid.clone();
            let pid_del = pid.clone();
            let is_active = pid == active_id;
            let is_user = palette.origin == PaletteOrigin::User;
            let swatch_row =
                palette
                    .swatches
                    .iter()
                    .fold(view().flex_row().flex_wrap().gap_1p5(), |row, sw| {
                        let color = sw.color;
                        row.child(
                            button()
                                .accessibility_label(sw.name.clone())
                                .on_press(move || {
                                    super::favorites::set_pen_color(state.live, color);
                                })
                                .w(px(26.0))
                                .h(px(26.0))
                                .rounded_full()
                                .bg(Hsla::from(rgb(color)))
                                .border_1()
                                .border_color(move || theme().border),
                        )
                    });
            let mut row = view()
                .p_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || {
                    if is_active {
                        theme().primary
                    } else {
                        theme().border
                    }
                })
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(palette.name.clone())
                                .flex_1()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().popover_foreground),
                        )
                        .child(action_button(
                            "ink_tools.palettes.use",
                            Tone::Plain,
                            move || {
                                let mut lib = read_palettes(state.live);
                                if lib.choose(&pid).is_ok() {
                                    write_palettes(state.live, &lib);
                                    version.update(|v| *v += 1);
                                }
                            },
                        ))
                        .child(action_button(
                            "ink_tools.palettes.copy",
                            Tone::Plain,
                            move || {
                                let mut lib = read_palettes(state.live);
                                let name = format!("{} copy", palette.name);
                                if lib.duplicate_as_user(&pid_dup, &name).is_ok() {
                                    write_palettes(state.live, &lib);
                                    version.update(|v| *v += 1);
                                }
                            },
                        )),
                )
                .child(swatch_row);
            if is_user {
                row = row.child(action_button(
                    "ink_tools.palettes.delete",
                    Tone::Plain,
                    move || {
                        let mut lib = read_palettes(state.live);
                        if lib.remove_user(&pid_del).is_ok() {
                            write_palettes(state.live, &lib);
                            version.update(|v| *v += 1);
                        }
                    },
                ));
            }
            acc.child(row)
        })
    });

    let body = view()
        .w(px(520.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("ink_tools.palettes.title"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button(
                    "ink_tools.palettes.import",
                    Tone::Plain,
                    import_json,
                ))
                .child(action_button(
                    "ink_tools.palettes.export",
                    Tone::Plain,
                    export_json,
                )),
        )
        .child(scroll().max_h(px(320.0)).child(list_view))
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(caption("ink_tools.palettes.new"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().w(px(140.0)).child(
                        field(input().value(move || new_name.get()))
                            .on_change(move |s: &str| new_name.set(s.to_owned()))
                            .placeholder(t!("ink_tools.palettes.name_placeholder"))
                            .label(t!("ink_tools.palettes.new")),
                    ),
                )
                .child(
                    view().flex_1().child(
                        field(input().value(move || new_hexes.get()))
                            .on_change(move |s: &str| new_hexes.set(s.to_owned()))
                            .label(t!("ink_tools.palettes.swatches")),
                    ),
                )
                .child(action_button(
                    "ink_tools.palettes.add",
                    Tone::Plain,
                    create_user,
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );

    list_dialog(t!("ink_tools.palettes.title"), close, card(body))
}

/// Ink Stroke Replay dialog (H.22): replays a page's strokes in chronological order with scrubbing and speed presets.
pub(crate) fn replay_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.replay_open.set(false);
    let page = state.active_page();
    let annots = state.page(page);
    let records: Vec<PageInkRecord<'_>> = annots
        .iter()
        .filter(|a| matches!(a.shape, Shape::Ink(_)))
        .map(|annotation| PageInkRecord {
            annotation,
            raw_samples: None,
        })
        .collect();
    let player = stored(ReplayPlayer::from_page_records(&records, None));
    let frame = signal(player.with(|p| p.seek_frame_at_current()));
    let playing = signal(false);
    let speed = signal(1.0_f32);

    let sync_frame = move || {
        player.with(|p| {
            let snapshot = p.timeline().frame_at(p.position_ms());
            frame.set(snapshot);
            playing.set(p.state() == PlaybackState::Playing);
        });
    };

    let toggle_play = move || {
        let should_run = player.with(|p| {
            p.toggle_play_pause();
            p.state() == PlaybackState::Playing
        });
        sync_frame();
        if should_run {
            spawn(async move {
                loop {
                    let _ = blocking(|| std::thread::sleep(Duration::from_millis(40))).await;
                    if !untracked(|| state.replay_open.get()) {
                        break;
                    }
                    let keep_going = player.with(|p| {
                        let f = p.advance(40.0);
                        frame.set(f);
                        let is_playing = p.state() == PlaybackState::Playing;
                        playing.set(is_playing);
                        is_playing
                    });
                    if !keep_going {
                        break;
                    }
                }
            });
        }
    };

    let preview = dynamic(move || {
        let f = frame.get();
        let canvas_w = 480.0_f32;
        let canvas_h = 260.0_f32;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for s in &f.strokes {
            for pt in &s.points {
                min_x = min_x.min(pt.x);
                min_y = min_y.min(pt.y);
                max_x = max_x.max(pt.x);
                max_y = max_y.max(pt.y);
            }
        }
        let scale = if min_x.is_finite() {
            let w = (max_x - min_x).max(40.0);
            let h = (max_y - min_y).max(40.0);
            ((canvas_w - 32.0) / w).min((canvas_h - 32.0) / h).min(2.0)
        } else {
            1.0
        };
        let mut stage = view()
            .relative()
            .w(px(canvas_w))
            .h(px(canvas_h))
            .rounded(px(8.0))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().background);
        if f.strokes.is_empty() {
            stage = stage.items_center().justify_center().child(
                text(move || t!("ink_tools.replay.empty"))
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
            );
            return stage;
        }
        for s in &f.strokes {
            let col = Hsla::from(rgb(s.color));
            let step = (s.points.len() / 60).max(1);
            for pt in s.points.iter().step_by(step) {
                let x = 16.0 + (pt.x - min_x) * scale;
                let y = 16.0 + (pt.y - min_y) * scale;
                let d = (pt.width * scale).clamp(2.0, 10.0);
                stage = stage.child(
                    view()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(d))
                        .h(px(d))
                        .rounded_full()
                        .bg(col),
                );
            }
        }
        stage
    });

    let speeds = SPEED_PRESETS
        .into_iter()
        .fold(segmented(), |group, preset| {
            group.child(segment_of(
                move || format!("{preset}×"),
                move || (speed.get() - preset).abs() < 0.01,
                move || {
                    speed.set(preset);
                    player.with(|p| p.set_speed(PlaybackSpeed::new(preset)));
                },
            ))
        });

    let body = view()
        .w(px(520.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("ink_tools.replay.title", page = page + 1))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(preview)
        .child(
            slider()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .value(move || SliderValue::Single(f64::from(frame.get().progress) * 100.0))
                .on_change(move |v: &SliderValue| {
                    if let SliderValue::Single(pct) = v {
                        player.with(|p| p.seek_progress((*pct as f32) / 100.0));
                        sync_frame();
                    }
                })
                .track_class(SLIDER_TRACK)
                .range_class(SLIDER_RANGE)
                .thumb_class(SLIDER_THUMB),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(action_button(
                    "ink_tools.replay.prev",
                    Tone::Plain,
                    move || {
                        player.with(ReplayPlayer::step_stroke_backward);
                        sync_frame();
                    },
                ))
                .child(action_button(
                    "ink_tools.replay.play_pause",
                    Tone::Plain,
                    toggle_play,
                ))
                .child(action_button(
                    "ink_tools.replay.stop",
                    Tone::Plain,
                    move || {
                        player.with(ReplayPlayer::stop);
                        sync_frame();
                    },
                ))
                .child(action_button(
                    "ink_tools.replay.next",
                    Tone::Plain,
                    move || {
                        player.with(ReplayPlayer::step_stroke_forward);
                        sync_frame();
                    },
                ))
                .child(view().flex_1())
                .child(
                    text(move || {
                        let f = frame.get();
                        format!(
                            "{:.1}s / {:.1}s",
                            f.elapsed_ms / 1000.0,
                            f.total_ms / 1000.0
                        )
                    })
                    .text_size(px(12.0))
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(speeds)
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );

    list_dialog(t!("command.ink_replay"), close, card(body))
}

trait ReplayFrameExt {
    fn seek_frame_at_current(&self) -> excellent_ink::replay::ReplayFrame;
}

impl ReplayFrameExt for ReplayPlayer {
    fn seek_frame_at_current(&self) -> excellent_ink::replay::ReplayFrame {
        self.timeline().frame_at(self.position_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::annot::Style;

    #[test]
    fn straightens_tilted_ink_annotation() {
        let strokes = vec![
            vec![
                PagePoint { x: 20.0, y: 86.0 },
                PagePoint { x: 20.0, y: 100.0 },
            ],
            vec![
                PagePoint { x: 50.0, y: 94.0 },
                PagePoint { x: 50.0, y: 108.0 },
            ],
            vec![
                PagePoint { x: 80.0, y: 102.0 },
                PagePoint { x: 80.0, y: 116.0 },
            ],
            vec![
                PagePoint { x: 110.0, y: 110.0 },
                PagePoint { x: 110.0, y: 124.0 },
            ],
        ];
        let annot = Annotation::new(0, Shape::Ink(strokes), Style::default(), "tester");
        let Some(Change::UpdateAnnotation(after)) = straighten_annotation(annot) else {
            panic!("expected straightened update");
        };
        let Shape::Ink(leveled) = after.shape else {
            panic!("expected ink shape");
        };
        assert!((leveled[3][1].y - leveled[0][1].y).abs() < 4.0);
    }

    fn rgb_to_u32(rgb: [f32; 3]) -> u32 {
        let r = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u32;
        let g = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u32;
        let b = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u32;
        (r << 16) | (g << 8) | b
    }

    #[test]
    fn round_trips_color_channel_conversions() {
        let hex_color = 0x3b82f6;
        let rgb = color_u32_to_rgb(hex_color);
        assert_eq!(rgb_to_u32(rgb), hex_color);
    }
}
