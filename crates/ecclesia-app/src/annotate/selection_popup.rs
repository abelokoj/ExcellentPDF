//! Floating pill text-selection popup and per-item customization (plan A.1, A.3, A.4).
//!
//! When text is selected on the page canvas, the canvas reports logical-DIP anchor coordinates via
//! `selectionPopup`, and [`selection_popup_overlay`] renders a clean floating pill directly above the
//! selected text (flipping below near the top edge): four vibrant circular swatch dots on the left,
//! a vertical hairline divider, and customizable action buttons on the right. Right-clicking the pill
//! (or visiting Settings → Annotations & ink) lets the reader add or remove any optional action.

use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine as _;
use ecclesia_features::annotate::colors::{self as colors_model, Tool as ColorTool};
use nsh_base::{button, context_menu, menu_item, menu_separator, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{BookmarkAction, BookmarkState};
use crate::settings::{Live, Settings};
use crate::settings_window::on_off;
use crate::shell::app_menu::{ChromeIcon, chrome_icon};
use crate::study_ui::StudyState;

/// Settings key persisting the ordered list of action items enabled in the text-selection popup.
pub(crate) const SELECTION_POPUP_ITEMS: &str = "selection_popup_items";

/// Settings key persisting which custom color picker icon design shows as the 6th swatch button.
pub(crate) const COLOR_PICKER_ICON: &str = "selection_popup_color_picker_icon";

/// Original five pastel default highlighter swatch colors (`STANDARD_LIGHT`: yellow, green, blue,
/// pink, orange) designed to keep black PDF text sharp and legible underneath.
pub(crate) const SWATCHES: [(u32, &str); 5] = [
    (0xFD_E047, "annotate.popup.color.yellow"),
    (0x86_EFAC, "annotate.popup.color.green"),
    (0x93_C5FD, "annotate.popup.color.blue"),
    (0xF9_A8D4, "annotate.popup.color.pink"),
    (0xFD_BA74, "annotate.popup.color.orange"),
];

/// Four selectable designs for the custom color picker button (the 6th icon beside the 5 swatches).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ColorPickerIcon {
    /// Rounded frame containing a 2×2 grid of coral, yellow, green, and blue rounded tiles (default).
    #[default]
    Grid,
    /// Smooth 360° rainbow color wheel ring.
    Wheel,
    /// Artist's palette with four paint dabs and thumb cutout.
    Palette,
    /// Angled color dropper / pipette with vibrant spectrum droplet.
    Dropper,
}

impl ColorPickerIcon {
    /// Every selectable design in settings display order.
    pub(crate) const ALL: [Self; 4] = [Self::Grid, Self::Wheel, Self::Palette, Self::Dropper];

    /// Stable identifier stored in `settings.toml` under [`COLOR_PICKER_ICON`].
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Wheel => "wheel",
            Self::Palette => "palette",
            Self::Dropper => "dropper",
        }
    }

    /// Parses a settings identifier back into a [`ColorPickerIcon`].
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "grid" => Some(Self::Grid),
            "wheel" => Some(Self::Wheel),
            "palette" => Some(Self::Palette),
            "dropper" => Some(Self::Dropper),
            _ => None,
        }
    }

    /// Localization key for this design's human-readable label.
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            Self::Grid => "annotate.popup.picker_icon.grid",
            Self::Wheel => "annotate.popup.picker_icon.wheel",
            Self::Palette => "annotate.popup.picker_icon.palette",
            Self::Dropper => "annotate.popup.picker_icon.dropper",
        }
    }
}

/// Embedded 64×64 white-on-transparent PNG for the comment bubble action.
const POPUP_COMMENT_PNG: &[u8] = include_bytes!("../../assets/icons/popup-comment.png");
/// Embedded 64×64 white-on-transparent PNG for the copy overlapping-squares action.
const POPUP_COPY_PNG: &[u8] = include_bytes!("../../assets/icons/popup-copy.png");
/// Embedded 64×64 RGBA PNG for the rainbow color wheel picker icon.
const PICKER_WHEEL_PNG: &[u8] = include_bytes!("../../assets/icons/color-picker-wheel.png");
/// Embedded 64×64 RGBA PNG for the artist palette picker icon.
const PICKER_PALETTE_PNG: &[u8] = include_bytes!("../../assets/icons/color-picker-palette.png");
/// Embedded 64×64 RGBA PNG for the color dropper picker icon.
const PICKER_DROPPER_PNG: &[u8] = include_bytes!("../../assets/icons/color-picker-dropper.png");

/// Four vibrant micro-tile colors for the default [`ColorPickerIcon::Grid`] design (top-left red,
/// top-right yellow, bottom-left green, bottom-right blue).
const GRID_TILE_RED: u32 = 0xF1_5555;
const GRID_TILE_YELLOW: u32 = 0xFB_C736;
const GRID_TILE_GREEN: u32 = 0x34_C77B;
const GRID_TILE_BLUE: u32 = 0x53_9BFF;

/// Floating pill height in logical DIPs.
const PILL_HEIGHT: f32 = 44.0;
/// Vertical clearance between the selected text and the floating pill in logical DIPs.
const GAP: f32 = 10.0;
/// Minimum edge inset inside the canvas viewport in logical DIPs.
const MARGIN: f32 = 8.0;
/// Width of each circular color swatch button in logical DIPs.
const SWATCH_SIZE: f32 = 24.0;
/// Width and height of each action icon button inside the pill in logical DIPs.
const ACTION_SIZE: f32 = 30.0;
/// Default vibrant yellow tint for the Highlight `A` button band.
const HIGHLIGHT_BAND_YELLOW: u32 = 0xFD_E047;
/// Dark ink color for the `A` glyph rendered over the yellow highlight band (legible in light and dark themes).
const HIGHLIGHT_GLYPH_INK: u32 = 0x1F_2937;
/// Red accent bar color beneath the Underline `A` button.
const UNDERLINE_RED: u32 = 0xDC_2626;
/// Warm orange accent wave color beneath the Squiggly `A` button.
const SQUIGGLY_ORANGE: u32 = 0xEA_580C;

/// Which text markup tool the four circular swatch buttons apply when clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ActiveMarkupTool {
    /// Highlight the selected text (default, matching the reference design).
    #[default]
    Highlight,
    /// Underline the selected text.
    Underline,
    /// Draw a squiggly underline beneath the selected text.
    Squiggly,
    /// Strike through the selected text.
    StrikeOut,
}

impl ActiveMarkupTool {
    /// Canvas `markWithColor` command `"kind"` string for this tool.
    pub(crate) const fn command_kind(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::Squiggly => "squiggly",
            Self::StrikeOut => "strikeOut",
        }
    }

    /// Corresponding palette tool for opening the custom color picker.
    pub(crate) const fn color_tool(self) -> ColorTool {
        match self {
            Self::Highlight => ColorTool::Highlight,
            Self::Underline => ColorTool::Underline,
            Self::Squiggly => ColorTool::Squiggly,
            Self::StrikeOut => ColorTool::StrikeOut,
        }
    }
}

/// One customizable action shown in the floating text-selection pill popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PopupItem {
    /// Highlight the selected text.
    Highlight,
    /// Underline the selected text.
    Underline,
    /// Highlight the selected text and open a comment note.
    Comment,
    /// Copy the selected text to the clipboard.
    Copy,
    /// Draw a squiggly underline under the selected text.
    Squiggly,
    /// Strike through the selected text.
    StrikeOut,
    /// Propose replacing the selected text.
    Replace,
    /// Propose inserting text after the selection.
    Insert,
    /// Look up the selection in the offline dictionary.
    Dictionary,
    /// Open the selection translation dialog.
    Translate,
    /// Toggle Read Aloud from the current page.
    ReadAloud,
    /// Bookmark the selected passage.
    Bookmark,
    /// Open the custom annotation color picker.
    CustomColor,
}

impl PopupItem {
    /// Default four items shown in the floating pill (matching the reference screenshot).
    pub(crate) const DEFAULT: [Self; 4] =
        [Self::Highlight, Self::Underline, Self::Comment, Self::Copy];

    /// Every customizable item in canonical display order.
    pub(crate) const ALL: [Self; 13] = [
        Self::Highlight,
        Self::Underline,
        Self::Comment,
        Self::Copy,
        Self::Squiggly,
        Self::StrikeOut,
        Self::Replace,
        Self::Insert,
        Self::Dictionary,
        Self::Translate,
        Self::ReadAloud,
        Self::Bookmark,
        Self::CustomColor,
    ];

    /// Stable identifier stored in `settings.toml` under [`SELECTION_POPUP_ITEMS`].
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::Comment => "comment",
            Self::Copy => "copy",
            Self::Squiggly => "squiggly",
            Self::StrikeOut => "strike_out",
            Self::Replace => "replace",
            Self::Insert => "insert",
            Self::Dictionary => "dictionary",
            Self::Translate => "translate",
            Self::ReadAloud => "read_aloud",
            Self::Bookmark => "bookmark",
            Self::CustomColor => "custom_color",
        }
    }

    /// Parses a settings identifier back into a [`PopupItem`].
    pub(crate) fn parse(id: &str) -> Option<Self> {
        match id.trim() {
            "highlight" => Some(Self::Highlight),
            "underline" => Some(Self::Underline),
            "comment" => Some(Self::Comment),
            "copy" => Some(Self::Copy),
            "squiggly" => Some(Self::Squiggly),
            "strike_out" => Some(Self::StrikeOut),
            "replace" => Some(Self::Replace),
            "insert" => Some(Self::Insert),
            "dictionary" => Some(Self::Dictionary),
            "translate" => Some(Self::Translate),
            "read_aloud" => Some(Self::ReadAloud),
            "bookmark" => Some(Self::Bookmark),
            "custom_color" => Some(Self::CustomColor),
            _ => None,
        }
    }

    /// Localization key for this item's human-readable label.
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            Self::Highlight => "annotate.popup.item.highlight",
            Self::Underline => "annotate.popup.item.underline",
            Self::Comment => "annotate.popup.item.comment",
            Self::Copy => "annotate.popup.item.copy",
            Self::Squiggly => "annotate.popup.item.squiggly",
            Self::StrikeOut => "annotate.popup.item.strike_out",
            Self::Replace => "annotate.popup.item.replace",
            Self::Insert => "annotate.popup.item.insert",
            Self::Dictionary => "annotate.popup.item.dictionary",
            Self::Translate => "annotate.popup.item.translate",
            Self::ReadAloud => "annotate.popup.item.read_aloud",
            Self::Bookmark => "annotate.popup.item.bookmark",
            Self::CustomColor => "annotate.popup.item.custom_color",
        }
    }

    /// Whether this item modifies PDF annotations and should only appear on editable documents.
    pub(crate) const fn requires_editable(self) -> bool {
        matches!(
            self,
            Self::Highlight
                | Self::Underline
                | Self::Comment
                | Self::Squiggly
                | Self::StrikeOut
                | Self::Replace
                | Self::Insert
                | Self::CustomColor
        )
    }

    /// Whether this item applies to rectangular block selections (SumatraPDF Enhanced supports
    /// highlights, comments, copy, bookmark, and custom color on block selections; other markup
    /// and text-study tools require a text selection).
    pub(crate) const fn supports_block(self) -> bool {
        matches!(
            self,
            Self::Highlight | Self::Comment | Self::Copy | Self::Bookmark | Self::CustomColor
        )
    }
}

/// Whether the active selection anchored by the floating pill is text or a rectangular block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    Text,
    Block,
}

/// Logical-DIP anchor geometry reported by the page canvas's `selectionPopup` event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SelectionPopupAnchor {
    /// Horizontal center of the first selected line or topmost block region in logical DIPs.
    pub(crate) x: f32,
    /// Top edge of the first selected line or topmost block region in logical DIPs.
    pub(crate) top_y: f32,
    /// Bottom edge of the last selected line or bottommost block region in logical DIPs.
    pub(crate) bottom_y: f32,
    /// Canvas viewport width in logical DIPs.
    pub(crate) view_w: f32,
    /// Canvas viewport height in logical DIPs.
    pub(crate) view_h: f32,
    /// Whether the active document can be annotated.
    pub(crate) editable: bool,
    /// Whether the anchor comes from a rectangular block selection rather than text selection.
    pub(crate) block: bool,
}

impl SelectionPopupAnchor {
    /// Parses a `selectionPopup` native event payload; returns `None` when hidden or malformed.
    pub(crate) fn parse(data: &JsonValue) -> Option<Self> {
        if data.get("visible").and_then(JsonValue::as_bool) != Some(true) {
            return None;
        }
        let number = |key: &str| {
            data.get(key)
                .and_then(JsonValue::as_f64)
                .filter(|value| value.is_finite())
                .map(|value| value as f32)
        };
        Some(Self {
            x: number("x")?,
            top_y: number("topY")?,
            bottom_y: number("bottomY")?,
            view_w: number("viewW")?,
            view_h: number("viewH")?,
            editable: data
                .get("editable")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            block: data
                .get("block")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
        })
    }

    const fn selection_kind(self) -> SelectionKind {
        if self.block {
            SelectionKind::Block
        } else {
            SelectionKind::Text
        }
    }
}

/// Returns the ordered list of enabled selection-popup items from `settings`, falling back to
/// [`PopupItem::DEFAULT`] when unset or empty.
pub(crate) fn enabled_items(settings: &Settings) -> Vec<PopupItem> {
    let Some(raw) = settings.text(SELECTION_POPUP_ITEMS) else {
        return PopupItem::DEFAULT.to_vec();
    };
    let mut items = Vec::new();
    for token in raw.split(',') {
        if let Some(item) = PopupItem::parse(token)
            && !items.contains(&item)
        {
            items.push(item);
        }
    }
    if items.is_empty() {
        PopupItem::DEFAULT.to_vec()
    } else {
        items
    }
}

/// Persists `items` into `settings`, clearing the key when it equals [`PopupItem::DEFAULT`].
fn write_items(settings: &mut Settings, items: &[PopupItem]) {
    if items == PopupItem::DEFAULT {
        settings.set_text(SELECTION_POPUP_ITEMS, None);
    } else {
        let joined = items
            .iter()
            .map(|item| item.id())
            .collect::<Vec<_>>()
            .join(",");
        settings.set_text(SELECTION_POPUP_ITEMS, Some(&joined));
    }
}

/// Pure helper toggling `item` inside `settings` while preserving at least one enabled item.
fn toggle_item_in(settings: &mut Settings, item: PopupItem) {
    let mut items = enabled_items(settings);
    if let Some(pos) = items.iter().position(|&existing| existing == item) {
        if items.len() > 1 {
            items.remove(pos);
        }
    } else {
        items.push(item);
        items.sort_by_key(|candidate| {
            PopupItem::ALL
                .iter()
                .position(|&all| all == *candidate)
                .unwrap_or(usize::MAX)
        });
    }
    write_items(settings, &items);
}

/// Toggles `item` in `live`'s selection-popup configuration and saves `settings.toml`.
pub(crate) fn toggle_item(live: Live, item: PopupItem) {
    live.change(|settings| toggle_item_in(settings, item));
}

/// Reads the selected custom color picker icon style from `settings`, defaulting to [`ColorPickerIcon::Grid`].
pub(crate) fn selected_picker_icon(settings: &Settings) -> ColorPickerIcon {
    settings
        .text(COLOR_PICKER_ICON)
        .and_then(ColorPickerIcon::parse)
        .unwrap_or_default()
}

/// Persists `icon` into `settings`, clearing the key when it equals the default [`ColorPickerIcon::Grid`].
fn write_picker_icon(settings: &mut Settings, icon: ColorPickerIcon) {
    if icon == ColorPickerIcon::default() {
        settings.set_text(COLOR_PICKER_ICON, None);
    } else {
        settings.set_text(COLOR_PICKER_ICON, Some(icon.id()));
    }
}

/// Persists `icon` into `live`'s settings and saves `settings.toml`.
pub(crate) fn set_picker_icon(live: Live, icon: ColorPickerIcon) {
    live.change(|settings| write_picker_icon(settings, icon));
}

/// Resets the selection-popup items and color picker icon style back to their defaults.
pub(crate) fn reset_defaults(live: Live) {
    live.change(|settings| {
        settings.set_text(SELECTION_POPUP_ITEMS, None);
        settings.set_text(COLOR_PICKER_ICON, None);
    });
}

/// Estimates the floating pill's width in logical DIPs for clamping within the viewport.
fn estimate_pill_width(editable: bool, item_count: usize) -> f32 {
    let padding = 28.0;
    // Five circular swatch dots plus the 6th custom color picker icon button.
    let swatch_slots = SWATCHES.len() + 1;
    let swatches = if editable {
        swatch_slots as f32 * SWATCH_SIZE + (swatch_slots.saturating_sub(1) as f32) * 8.0 + 4.0
    } else {
        0.0
    };
    let divider = if editable && item_count > 0 {
        21.0
    } else {
        0.0
    };
    let actions = if item_count > 0 {
        item_count as f32 * ACTION_SIZE + (item_count.saturating_sub(1) as f32) * 6.0
    } else {
        0.0
    };
    padding + swatches + divider + actions
}

/// Computes `(left, top)` in logical DIPs for a floating pill of width `est_width`, preferring
/// placement above the selection and flipping below when too close to the top edge.
pub(crate) fn popup_position(anchor: SelectionPopupAnchor, est_width: f32) -> (f32, f32) {
    let top = if anchor.top_y >= PILL_HEIGHT + GAP + MARGIN {
        anchor.top_y - PILL_HEIGHT - GAP
    } else {
        (anchor.bottom_y + GAP).min((anchor.view_h - PILL_HEIGHT - MARGIN).max(MARGIN))
    };
    let max_left = (anchor.view_w - est_width - MARGIN).max(MARGIN);
    let left = (anchor.x - est_width / 2.0).clamp(MARGIN, max_left);
    (left, top)
}

fn png_data_uri(bytes: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn comment_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| png_data_uri(POPUP_COMMENT_PNG)).clone()
}

fn copy_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| png_data_uri(POPUP_COPY_PNG)).clone()
}

fn picker_wheel_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| png_data_uri(PICKER_WHEEL_PNG)).clone()
}

fn picker_palette_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| png_data_uri(PICKER_PALETTE_PNG)).clone()
}

fn picker_dropper_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| png_data_uri(PICKER_DROPPER_PNG)).clone()
}

/// Renders one of the four selectable custom color picker icons at `size` logical DIPs.
pub(crate) fn picker_icon_glyph(icon: ColorPickerIcon, size: f32) -> AnyElement {
    match icon {
        ColorPickerIcon::Grid => {
            let tile = ((size - 8.0) / 2.0).max(4.0);
            let corner = (size * 0.28).clamp(4.0, 8.0);
            let tile_corner = (tile * 0.28).clamp(1.5, 3.0);
            let tile_box = |color: u32| {
                view()
                    .w(px(tile))
                    .h(px(tile))
                    .rounded(px(tile_corner))
                    .bg(Hsla::from(rgb(color)))
            };
            view()
                .w(px(size))
                .h(px(size))
                .rounded(px(corner))
                .border_2()
                .border_color(move || theme().popover_foreground)
                .items_center()
                .justify_center()
                .gap(px(1.5))
                .child(
                    view()
                        .flex_row()
                        .gap(px(1.5))
                        .child(tile_box(GRID_TILE_RED))
                        .child(tile_box(GRID_TILE_YELLOW)),
                )
                .child(
                    view()
                        .flex_row()
                        .gap(px(1.5))
                        .child(tile_box(GRID_TILE_GREEN))
                        .child(tile_box(GRID_TILE_BLUE)),
                )
                .into_element()
        }
        ColorPickerIcon::Wheel => image(picker_wheel_icon_src())
            .w(px(size))
            .h(px(size))
            .object_fit(ResizeMode::Contain)
            .into_element(),
        ColorPickerIcon::Palette => image(picker_palette_icon_src())
            .w(px(size))
            .h(px(size))
            .object_fit(ResizeMode::Contain)
            .into_element(),
        ColorPickerIcon::Dropper => image(picker_dropper_icon_src())
            .w(px(size))
            .h(px(size))
            .object_fit(ResizeMode::Contain)
            .into_element(),
    }
}

/// Absolute pass-through overlay hosting the floating text-selection pill above the page canvas.
pub(crate) fn selection_popup_overlay(
    annotate: AnnotateState,
    canvas: ElementRef,
    study: StudyState,
    bookmarks: BookmarkState,
) -> impl IntoElement {
    let live = annotate.live_settings();
    view()
        .absolute()
        .inset_0()
        .pointer_events(PointerEvents::BoxNone)
        .child(dynamic(move || {
            let Some(anchor) = annotate.selection_popup_anchor() else {
                return view().into_element();
            };
            let visible_items: Vec<PopupItem> = live
                .read(enabled_items)
                .into_iter()
                .filter(|item| {
                    (anchor.editable || !item.requires_editable())
                        && (!anchor.block || item.supports_block())
                })
                .collect();
            if !anchor.editable && visible_items.is_empty() {
                return view().into_element();
            }
            let picker_icon = live.read(selected_picker_icon);
            let est_width = estimate_pill_width(anchor.editable, visible_items.len());
            let (left, top) = popup_position(anchor, est_width);
            let pill = build_pill(
                annotate,
                canvas,
                study,
                bookmarks,
                anchor,
                picker_icon,
                &visible_items,
            );
            let with_menu = attach_customization_menu(live, pill);
            view()
                .absolute()
                .left(px(left))
                .top(px(top))
                .pointer_events(PointerEvents::Auto)
                .child(with_menu)
                .into_element()
        }))
}

/// Builds the floating pill container with five pastel color swatches, the 6th custom color picker
/// button, a hairline divider, and customizable action buttons.
fn build_pill(
    annotate: AnnotateState,
    canvas: ElementRef,
    study: StudyState,
    bookmarks: BookmarkState,
    anchor: SelectionPopupAnchor,
    picker_icon: ColorPickerIcon,
    items: &[PopupItem],
) -> View {
    let editable = anchor.editable;
    let selection_kind = anchor.selection_kind();
    let mut pill = view()
        .pointer_events(PointerEvents::Auto)
        .flex_row()
        .items_center()
        .h(px(PILL_HEIGHT))
        .px(px(14.0))
        .gap(px(10.0))
        .rounded_full()
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg();

    if editable {
        let swatches = SWATCHES
            .into_iter()
            .fold(
                view().flex_row().items_center().gap(px(8.0)),
                |row, (color, label_key)| {
                    row.child(swatch_button(
                        annotate,
                        canvas,
                        selection_kind,
                        color,
                        label_key,
                    ))
                },
            )
            .child(custom_color_picker_button(
                annotate,
                selection_kind,
                picker_icon,
            ));
        pill = pill.child(swatches);
    }

    if editable && !items.is_empty() {
        pill = pill.child(view().w(px(1.0)).h(px(22.0)).bg(move || theme().border));
    }

    if !items.is_empty() {
        let actions = items.iter().copied().fold(
            view().flex_row().items_center().gap(px(6.0)),
            |row, item| {
                row.child(action_button(
                    annotate,
                    canvas,
                    study,
                    bookmarks,
                    selection_kind,
                    picker_icon,
                    item,
                ))
            },
        );
        pill = pill.child(actions);
    }

    pill
}

/// One pastel circular swatch button on the left side of the floating pill.
fn swatch_button(
    annotate: AnnotateState,
    canvas: ElementRef,
    selection_kind: SelectionKind,
    color: u32,
    label_key: &'static str,
) -> impl IntoElement {
    button()
        .accessibility_label(t!(label_key))
        .on_press(move || {
            let kind = match selection_kind {
                SelectionKind::Block => "highlight",
                SelectionKind::Text => untracked(|| annotate.active_markup_tool()).command_kind(),
            };
            canvas.command(
                "markWithColor",
                serde_json::json!({
                    "kind": kind,
                    "color": colors_model::hex(color),
                    "note": false,
                }),
            );
            drop(set_timeout(Duration::ZERO, move || {
                annotate.dismiss_selection_popup();
            }));
        })
        .w(px(SWATCH_SIZE))
        .h(px(SWATCH_SIZE))
        .rounded_full()
        .bg(Hsla::from(rgb(color)))
        .border_1()
        .border_color(Hsla::from(rgba(0x0000_0026)))
        .shadow_sm()
        .hover(|s| s.border_2().border_color(move || theme().ring))
        .active(|s| s.opacity(0.85))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
}

/// The 6th button to the right of the five default color swatches: opens the full custom color
/// picker for the active markup tool using the user's chosen [`ColorPickerIcon`] design.
fn custom_color_picker_button(
    annotate: AnnotateState,
    selection_kind: SelectionKind,
    picker_icon: ColorPickerIcon,
) -> impl IntoElement {
    button()
        .accessibility_label(t!("annotate.popup.item.custom_color"))
        .on_press(move || {
            let tool = match selection_kind {
                SelectionKind::Block => ColorTool::Highlight,
                SelectionKind::Text => untracked(|| annotate.active_markup_tool()).color_tool(),
            };
            annotate.pick(tool);
            drop(set_timeout(Duration::ZERO, move || {
                annotate.dismiss_selection_popup();
            }));
        })
        .w(px(SWATCH_SIZE + 4.0))
        .h(px(SWATCH_SIZE + 4.0))
        .rounded(px(7.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.80))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(picker_icon_glyph(picker_icon, 21.0))
}

/// One action icon button on the right side of the floating pill.
fn action_button(
    annotate: AnnotateState,
    canvas: ElementRef,
    study: StudyState,
    bookmarks: BookmarkState,
    selection_kind: SelectionKind,
    picker_icon: ColorPickerIcon,
    item: PopupItem,
) -> impl IntoElement {
    let active_mode = match item {
        PopupItem::Highlight => Some(ActiveMarkupTool::Highlight),
        PopupItem::Underline => Some(ActiveMarkupTool::Underline),
        PopupItem::Squiggly => Some(ActiveMarkupTool::Squiggly),
        PopupItem::StrikeOut => Some(ActiveMarkupTool::StrikeOut),
        _ => None,
    };
    let is_active = move || match selection_kind {
        SelectionKind::Block => active_mode == Some(ActiveMarkupTool::Highlight),
        SelectionKind::Text => {
            active_mode.is_some_and(|mode| annotate.active_markup_tool() == mode)
        }
    };
    button()
        .accessibility_label(t!(item.label_key()))
        .on_press(move || run_item(annotate, canvas, study, bookmarks, selection_kind, item))
        .w(px(ACTION_SIZE))
        .h(px(ACTION_SIZE))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .style_when(is_active, |s| s.bg(move || theme().muted))
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(item_glyph(item, picker_icon))
}

/// Executes the selected popup item action against the canvas or pane controller.
fn run_item(
    annotate: AnnotateState,
    canvas: ElementRef,
    study: StudyState,
    bookmarks: BookmarkState,
    selection_kind: SelectionKind,
    item: PopupItem,
) {
    match item {
        PopupItem::Highlight => {
            annotate.set_active_markup_tool(ActiveMarkupTool::Highlight);
            canvas.command(
                "markWithColor",
                serde_json::json!({ "kind": "highlight", "note": false }),
            );
        }
        PopupItem::Underline => {
            annotate.set_active_markup_tool(ActiveMarkupTool::Underline);
            canvas.command(
                "markWithColor",
                serde_json::json!({ "kind": "underline", "note": false }),
            );
        }
        PopupItem::Squiggly => {
            annotate.set_active_markup_tool(ActiveMarkupTool::Squiggly);
            canvas.command(
                "markWithColor",
                serde_json::json!({ "kind": "squiggly", "note": false }),
            );
        }
        PopupItem::StrikeOut => {
            annotate.set_active_markup_tool(ActiveMarkupTool::StrikeOut);
            canvas.command(
                "markWithColor",
                serde_json::json!({ "kind": "strikeOut", "note": false }),
            );
        }
        PopupItem::Comment => {
            canvas.command(
                "markWithColor",
                serde_json::json!({ "kind": "note", "note": true }),
            );
        }
        PopupItem::Copy => {
            canvas.command("copy", JsonValue::Null);
        }
        PopupItem::Replace => {
            canvas.command("replaceText", JsonValue::Null);
        }
        PopupItem::Insert => {
            canvas.command("insertText", JsonValue::Null);
        }
        PopupItem::Dictionary => {
            study.open_dictionary();
        }
        PopupItem::Translate => {
            study.open_translate();
        }
        PopupItem::ReadAloud => {
            study.toggle_read_aloud();
        }
        PopupItem::Bookmark => {
            bookmarks.run(BookmarkAction::Add, canvas);
        }
        PopupItem::CustomColor => {
            let tool = match selection_kind {
                SelectionKind::Block => ColorTool::Highlight,
                SelectionKind::Text => untracked(|| annotate.active_markup_tool()).color_tool(),
            };
            annotate.pick(tool);
        }
    }
    drop(set_timeout(Duration::ZERO, move || {
        annotate.dismiss_selection_popup();
    }));
}

/// Renders the visual icon or typographic glyph for `item`.
fn item_glyph(item: PopupItem, picker_icon: ColorPickerIcon) -> AnyElement {
    match item {
        PopupItem::Highlight => view()
            .relative()
            .w(px(24.0))
            .h(px(22.0))
            .items_center()
            .justify_center()
            .child(
                view()
                    .absolute()
                    .w(px(22.0))
                    .h(px(10.0))
                    .rounded(px(2.0))
                    .bg(Hsla::from(rgb(HIGHLIGHT_BAND_YELLOW))),
            )
            .child(
                text("A")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(Hsla::from(rgb(HIGHLIGHT_GLYPH_INK))),
            )
            .into_element(),
        PopupItem::Underline => view()
            .items_center()
            .justify_center()
            .child(
                text("A")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().popover_foreground),
            )
            .child(
                view()
                    .w(px(16.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(Hsla::from(rgb(UNDERLINE_RED))),
            )
            .into_element(),
        PopupItem::Comment => image(comment_icon_src())
            .w(px(18.0))
            .h(px(18.0))
            .object_fit(ResizeMode::Contain)
            .tint_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Copy => image(copy_icon_src())
            .w(px(18.0))
            .h(px(18.0))
            .object_fit(ResizeMode::Contain)
            .tint_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Squiggly => view()
            .items_center()
            .justify_center()
            .child(
                text("A")
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().popover_foreground),
            )
            .child(
                text("~")
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(Hsla::from(rgb(SQUIGGLY_ORANGE))),
            )
            .into_element(),
        PopupItem::StrikeOut => text("S")
            .text_size(px(15.0))
            .font_weight(FontWeight::MEDIUM)
            .line_through()
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Replace => text("⇄")
            .text_size(px(15.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Insert => text("^")
            .text_size(px(16.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Dictionary => text("Aa")
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Translate => text("文")
            .text_size(px(14.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::ReadAloud => text("▶")
            .text_size(px(12.0))
            .text_color(move || theme().popover_foreground)
            .into_element(),
        PopupItem::Bookmark => chrome_icon(ChromeIcon::RailBookmarks, 16.0, move || {
            theme().popover_foreground
        })
        .into_element(),
        PopupItem::CustomColor => picker_icon_glyph(picker_icon, 18.0),
    }
}

/// Wraps `pill` in a right-click context menu allowing the user to toggle any [`PopupItem`] or reset defaults.
fn attach_customization_menu(live: Live, pill: View) -> impl IntoElement {
    let mut menu = context_menu()
        .trigger(pill)
        .popup_class("w-60 p-1 rounded-lg border border-border bg-popover shadow-lg");
    for item in PopupItem::ALL {
        let checked = move || live.read(enabled_items).contains(&item);
        menu = menu.child(
            menu_item()
                .checked(move || Some(checked()))
                .accessibility_label(t!(item.label_key()))
                .on_select(move || toggle_item(live, item))
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(30.0))
                .px_2()
                .rounded(px(6.0))
                .hover(|s| s.bg(move || theme().accent))
                .focus(|s| s.bg(move || theme().accent))
                .child(
                    text(move || if checked() { "✓" } else { "" })
                        .w(px(14.0))
                        .text_size(px(13.0))
                        .text_color(move || theme().primary),
                )
                .child(
                    text(move || t!(item.label_key()))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().popover_foreground),
                ),
        );
    }
    menu.child(
        menu_separator()
            .h(px(1.0))
            .my(px(4.0))
            .bg(move || theme().border),
    )
    .child(
        menu_item()
            .accessibility_label(t!("annotate.popup.reset"))
            .on_select(move || reset_defaults(live))
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(30.0))
            .px_2()
            .rounded(px(6.0))
            .hover(|s| s.bg(move || theme().accent))
            .focus(|s| s.bg(move || theme().accent))
            .child(view().w(px(14.0)))
            .child(
                text(move || t!("annotate.popup.reset"))
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

/// Settings rows allowing the user to pick the custom color picker icon style, enable/disable
/// each selection-popup item, and reset to defaults inside Settings → Annotations & ink.
pub(crate) fn selection_popup_settings_rows(live: Live) -> Vec<View> {
    let mut rows = Vec::with_capacity(PopupItem::ALL.len() + 2);
    let chosen_icon = move || live.read(selected_picker_icon);
    let icon_choices = ColorPickerIcon::ALL.into_iter().fold(
        view().flex_row().items_center().gap(px(8.0)),
        |row, icon| {
            let active = move || chosen_icon() == icon;
            row.child(
                button()
                    .accessibility_label(t!(icon.label_key()))
                    .on_press(move || set_picker_icon(live, icon))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .flex_row()
                    .items_center()
                    .gap(px(7.0))
                    .border_1()
                    .border_color(move || {
                        if active() {
                            theme().primary
                        } else {
                            theme().border
                        }
                    })
                    .bg(move || {
                        if active() {
                            theme().accent
                        } else {
                            theme().background
                        }
                    })
                    .hover(|s| s.bg(move || theme().accent))
                    .focus_visible(|s| s.border_color(move || theme().ring))
                    .child(picker_icon_glyph(icon, 20.0))
                    .child(
                        text(move || t!(icon.label_key()))
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    ),
            )
        },
    );
    rows.push(
        view()
            .px(px(16.0))
            .py(px(12.0))
            .gap(px(8.0))
            .child(
                view()
                    .gap(px(2.0))
                    .child(
                        text(move || t!("annotate.popup.picker_icon"))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(move || t!("annotate.popup.picker_icon_hint"))
                            .text_size(px(12.0))
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(icon_choices),
    );
    for item in PopupItem::ALL {
        let enabled = move || live.read(enabled_items).contains(&item);
        let row = view()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(10.0))
            .border_t_1()
            .border_t_color(move || theme().border);
        rows.push(
            row.child(
                text(move || t!(item.label_key()))
                    .flex_1()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().foreground),
            )
            .child(on_off(item.label_key(), enabled, move |want| {
                let current = untracked(enabled);
                if want != current {
                    toggle_item(live, item);
                }
            })),
        );
    }
    rows.push(
        view()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(10.0))
            .border_t_1()
            .border_t_color(move || theme().border)
            .child(
                text(move || t!("annotate.popup.customize_hint"))
                    .flex_1()
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                button()
                    .accessibility_label(t!("annotate.popup.reset"))
                    .on_press(move || reset_defaults(live))
                    .h(px(28.0))
                    .px_3()
                    .rounded(px(6.0))
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(move || theme().border)
                    .hover(|s| s.bg(move || theme().accent))
                    .focus_visible(|s| s.border_color(move || theme().ring))
                    .child(
                        text(move || t!("annotate.popup.reset"))
                            .text_size(px(12.5))
                            .text_color(move || theme().popover_foreground),
                    ),
            ),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_items_match_reference_pill() {
        let settings = Settings::default();
        assert_eq!(
            enabled_items(&settings),
            vec![
                PopupItem::Highlight,
                PopupItem::Underline,
                PopupItem::Comment,
                PopupItem::Copy,
            ]
        );
    }

    #[test]
    fn toggling_items_round_trips_through_settings() {
        let mut settings = Settings::default();
        toggle_item_in(&mut settings, PopupItem::Translate);
        assert!(enabled_items(&settings).contains(&PopupItem::Translate));
        assert!(settings.text(SELECTION_POPUP_ITEMS).is_some());

        toggle_item_in(&mut settings, PopupItem::Dictionary);
        assert!(enabled_items(&settings).contains(&PopupItem::Dictionary));

        toggle_item_in(&mut settings, PopupItem::Translate);
        toggle_item_in(&mut settings, PopupItem::Dictionary);
        assert_eq!(enabled_items(&settings), PopupItem::DEFAULT.to_vec());
        assert!(
            settings.text(SELECTION_POPUP_ITEMS).is_none(),
            "returning to defaults removes the settings key"
        );
    }

    #[test]
    fn positioning_prefers_above_selection_and_flips_below_near_top_edge() {
        let width = estimate_pill_width(true, 4);
        let centered = SelectionPopupAnchor {
            x: 400.0,
            top_y: 220.0,
            bottom_y: 240.0,
            view_w: 800.0,
            view_h: 600.0,
            editable: true,
            block: false,
        };
        let (left, top) = popup_position(centered, width);
        assert!((top - (220.0 - PILL_HEIGHT - GAP)).abs() < 0.01);
        assert!((left - (400.0 - width / 2.0)).abs() < 0.01);

        let near_top = SelectionPopupAnchor {
            x: 40.0,
            top_y: 24.0,
            bottom_y: 44.0,
            view_w: 800.0,
            view_h: 600.0,
            editable: true,
            block: false,
        };
        let (clamped_left, flipped_top) = popup_position(near_top, width);
        assert!((flipped_top - (44.0 + GAP)).abs() < 0.01);
        assert!((clamped_left - MARGIN).abs() < 0.01);
    }

    #[test]
    fn parses_block_anchor_and_filters_block_compatible_items() {
        let anchor = SelectionPopupAnchor::parse(&serde_json::json!({
            "visible": true,
            "x": 250.0,
            "topY": 120.0,
            "bottomY": 240.0,
            "viewW": 1000.0,
            "viewH": 800.0,
            "editable": true,
            "block": true,
        }))
        .expect("block anchor parses");
        assert!(anchor.block);
        assert!(anchor.editable);
        assert_eq!(anchor.selection_kind(), SelectionKind::Block);

        let plain = SelectionPopupAnchor::parse(&serde_json::json!({
            "visible": true,
            "x": 250.0,
            "topY": 120.0,
            "bottomY": 140.0,
            "viewW": 1000.0,
            "viewH": 800.0,
            "editable": true,
        }))
        .expect("plain anchor parses");
        assert!(!plain.block);
        assert_eq!(plain.selection_kind(), SelectionKind::Text);

        let block_items: Vec<PopupItem> = PopupItem::ALL
            .into_iter()
            .filter(|item| item.supports_block())
            .collect();
        assert_eq!(
            block_items,
            vec![
                PopupItem::Highlight,
                PopupItem::Comment,
                PopupItem::Copy,
                PopupItem::Bookmark,
                PopupItem::CustomColor,
            ]
        );
    }

    #[test]
    fn swatches_restore_five_original_pastel_defaults_and_picker_icons_round_trip() {
        let expected = colors_model::Palette::Standard.swatches(ColorTool::Highlight);
        let actual: Vec<u32> = SWATCHES.iter().map(|(color, _)| *color).collect();
        assert_eq!(actual, expected.to_vec());

        let mut settings = Settings::default();
        assert_eq!(selected_picker_icon(&settings), ColorPickerIcon::Grid);
        for icon in ColorPickerIcon::ALL {
            write_picker_icon(&mut settings, icon);
            assert_eq!(selected_picker_icon(&settings), icon);
            if icon == ColorPickerIcon::Grid {
                assert!(settings.text(COLOR_PICKER_ICON).is_none());
            } else {
                assert_eq!(settings.text(COLOR_PICKER_ICON), Some(icon.id()));
            }
        }
    }
}
