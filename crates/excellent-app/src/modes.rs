//! Modes and toolbars (plan K.18, §10.2), the view: the mode bar under the title bar (the mode switcher, then the
//! mode's toolbar), the customize dialog (reorder by dragging or with the arrow buttons, add, remove, reset), and
//! presets saved to and read from JSON files. The model is `excellent_features::modes`.
//!
//! The mode and any toolbar the reader changed live in `settings.toml` (`mode`, `toolbar_<mode>`); a toolbar left as it
//! came is not written, so a later version's defaults reach it. A mode is offered in the switcher once its toolbar has
//! a tool: Write and Study come with their tags' tools (H.*, S.*), or when the reader puts commands on them.

use std::path::Path;

use excellent_features::modes::{Item, Mode, Preset, PresetError, Toolbar, after_pen_contact};
use nsh_base::{button, input, theme, toggle, toggle_group, toolbar, toolbar_separator};
use nsh_core::element::AdaptiveColor;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use crate::commands::{self, COMMANDS, CommandId};
use crate::links::{Tone, dialog_button};
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, Live};

/// `mode` in `settings.toml`: the mode the window is in (absent: Read).
pub(crate) const MODE_KEY: &str = "mode";
/// `pen_switches_to_write`: a pen touching a page switches to Write mode (default on).
pub(crate) const PEN_TO_WRITE: &str = "pen_switches_to_write";
/// The height of a row in the customize dialog's list: dragging a row moves it one place per row height.
const ROW_HEIGHT: f32 = 36.0;

fn toolbar_key(mode: Mode) -> String {
    format!("toolbar_{}", mode.id())
}

/// Each mode's toolbar as it comes: only that mode's tools, and only the ones built so far.
fn default_words(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Read => &[
            "view.zoom_out",
            "view.zoom_in",
            "view.fit_width",
            "view.fit_page",
            "|",
            "view.layout_single",
            "view.layout_facing",
            "view.toggle_continuous",
            "|",
            "find.open",
            "view.reading_theme",
            "view.full_screen",
        ],
        Mode::Annotate => &[
            "markup.highlight",
            "markup.underline",
            "markup.squiggly",
            "markup.strikeout",
            "markup.replace",
            "markup.insert",
            "annotate.note",
            "annotate.sticky",
            "annotate.text_box",
            "annotate.typewriter",
            "annotate.callout",
            "annotate.line",
            "annotate.arrow",
            "annotate.rectangle",
            "annotate.ellipse",
            "annotate.polygon",
            "annotate.polyline",
            "annotate.cloud",
            "annotate.regular_polygon",
            "annotate.star",
            "annotate.colors",
            "annotate.margin_notes",
            "annotate.list",
            "annotate.transfer",
            "|",
            "annotate.cut",
            "annotate.paste",
            "annotate.delete",
            "|",
            "bookmark.add",
            "outline.add",
            "|",
            "edit.undo",
            "edit.redo",
        ],
        // shortcut: pens and erasers (H.1 to H.9) and the study tools (S.*) put their commands here with their tags.
        Mode::Write | Mode::Study => &[],
        Mode::Present => &[
            "view.full_screen",
            "|",
            "go.first_page",
            "go.screen_up",
            "go.screen_down",
            "go.last_page",
            "|",
            "view.reading_ruler",
            "view.auto_scroll",
        ],
    }
}

/// Whether a command named `name` exists, so it may stand on a toolbar.
fn known(name: &str) -> bool {
    commands::find(name).is_some()
}

/// `mode`'s toolbar as it comes.
pub(crate) fn default_toolbar(mode: Mode) -> Toolbar {
    Toolbar::from_words(default_words(mode).iter().copied(), known)
}

/// `mode`'s name in the reader's language.
pub(crate) fn mode_name(mode: Mode) -> String {
    t!(match mode {
        Mode::Read => "mode.read",
        Mode::Annotate => "mode.annotate",
        Mode::Write => "mode.write",
        Mode::Study => "mode.study",
        Mode::Present => "mode.present",
    })
}

/// `mode`'s accent (`mode-<id>` in the theme, plan §10.2).
pub(crate) fn accent(mode: Mode) -> AdaptiveColor {
    let theme = theme();
    theme
        .color(&format!("mode-{}", mode.id()))
        .unwrap_or(theme.primary)
}

/// Text and marks drawn on `mode`'s accent.
fn on_accent(mode: Mode) -> AdaptiveColor {
    let theme = theme();
    theme
        .color(&format!("mode-{}-foreground", mode.id()))
        .unwrap_or(theme.primary_foreground)
}

/// A row being dragged in the customize dialog: where it was, where the pointer went down, where it would land.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    from: usize,
    start_y: f32,
    to: usize,
}

/// The window's mode and toolbars, and the customize dialog's state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Modes {
    live: Live,
    /// The mode the customize dialog edits, while it is open.
    editing: Signal<Option<Mode>>,
    /// The dialog's last word on an export or import.
    notice: Signal<Option<String>>,
    drag: Signal<Option<Drag>>,
}

impl Modes {
    pub(crate) fn new(live: Live) -> Self {
        Self {
            live,
            editing: signal(None),
            notice: signal(None),
            drag: signal(None),
        }
    }

    /// The mode the window is in.
    pub(crate) fn current(self) -> Mode {
        self.live
            .read(|s| s.text(MODE_KEY).and_then(Mode::from_id))
            .unwrap_or_default()
    }

    /// `mode`'s toolbar: the reader's, else as it comes.
    pub(crate) fn toolbar(self, mode: Mode) -> Toolbar {
        self.live
            .read(|s| {
                s.text(&toolbar_key(mode))
                    .map(|text| Toolbar::parse(text, known))
            })
            .unwrap_or_else(|| default_toolbar(mode))
    }

    fn set_toolbar(self, mode: Mode, toolbar: &Toolbar) {
        let text = (*toolbar != default_toolbar(mode)).then(|| toolbar.to_text());
        self.live
            .change(|s| s.set_text(&toolbar_key(mode), text.as_deref()));
    }

    fn edit(self, mode: Mode, change: impl FnOnce(&mut Toolbar)) {
        let mut toolbar = untracked(|| self.toolbar(mode));
        change(&mut toolbar);
        self.set_toolbar(mode, &toolbar);
    }

    /// Whether the switcher offers `mode`: Read always, the current mode, and any whose toolbar has a tool.
    pub(crate) fn offered(self, mode: Mode) -> bool {
        mode == Mode::Read || mode == self.current() || !self.toolbar(mode).is_empty()
    }

    /// Switches to `mode`.
    pub(crate) fn set(self, mode: Mode) {
        self.live
            .change(|s| s.set_text(MODE_KEY, (mode != Mode::Read).then_some(mode.id())));
    }

    /// Switches to the next offered mode (`back`: the previous one).
    pub(crate) fn step(self, back: bool) {
        untracked(|| {
            let next = self.current().step(back, |mode| self.offered(mode));
            self.set(next);
        });
    }

    /// A pen touched a page: Write mode, when the setting allows and Write has tools.
    pub(crate) fn pen_contact(self) {
        untracked(|| {
            let allowed = self.live.read(|s| s.flag(PEN_TO_WRITE).unwrap_or(true));
            let current = self.current();
            let next = after_pen_contact(current, allowed, self.offered(Mode::Write));
            if next != current {
                self.set(next);
            }
        });
    }

    /// Opens the customize dialog on the current mode's toolbar.
    pub(crate) fn customize(self) {
        self.notice.set(None);
        self.editing.set(Some(untracked(|| self.current())));
    }

    /// Whether the customize dialog is open.
    pub(crate) fn is_customizing(self) -> bool {
        self.editing.with(Option::is_some)
    }

    fn close(self) {
        self.drag.set(None);
        self.editing.set(None);
    }

    /// Asks where to save every mode's toolbar, and saves them there as a preset. The dialog opens to say how it
    /// went.
    pub(crate) fn export(self) {
        self.customize();
        let preset = Preset {
            toolbars: untracked(|| {
                Mode::ALL
                    .into_iter()
                    .map(|mode| (mode, self.toolbar(mode)))
                    .collect()
            }),
            unknown: 0,
        };
        let json = preset.to_json();
        let dialog = SaveDialog::new()
            .file_name(t!("toolbar.file_name"))
            .filter(FileFilter::new(t!("toolbar.filter"), ["json"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let notice = match answer.await {
                Ok(Some(path)) => match settings::write_atomically(&path, &json) {
                    Ok(()) => t!("toolbar.exported", name = file_name(&path)),
                    Err(err) => {
                        log::warn!("toolbar export: {err}");
                        t!("toolbar.export_failed", name = file_name(&path))
                    }
                },
                Ok(None) => return,
                Err(err) => {
                    log::warn!("toolbar export: the dialog failed: {err}");
                    t!("toolbar.dialog_failed")
                }
            };
            self.notice.set(Some(notice));
        });
    }

    /// Asks for a preset file and takes the toolbars it has; modes it doesn't name keep theirs.
    pub(crate) fn import(self) {
        self.customize();
        let dialog = OpenDialog::files().filter(FileFilter::new(t!("toolbar.filter"), ["json"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let path = match answer.await {
                Ok(Some(paths)) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(None) => return,
                Err(err) => {
                    log::warn!("toolbar import: the dialog failed: {err}");
                    self.notice.set(Some(t!("toolbar.dialog_failed")));
                    return;
                }
            };
            let notice = match std::fs::read_to_string(&path) {
                Ok(json) => match Preset::from_json(&json, known) {
                    Ok(preset) => {
                        for (mode, toolbar) in &preset.toolbars {
                            self.set_toolbar(*mode, toolbar);
                        }
                        imported(&preset, &path)
                    }
                    Err(err) => refused(&err, &path),
                },
                Err(err) => {
                    log::warn!("toolbar import: {err}");
                    t!("toolbar.unreadable", name = file_name(&path))
                }
            };
            self.notice.set(Some(notice));
        });
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What the dialog says after `preset` came in from `path`.
fn imported(preset: &Preset, path: &Path) -> String {
    let modes = preset
        .toolbars
        .iter()
        .map(|(mode, _)| mode_name(*mode))
        .collect::<Vec<_>>()
        .join(", ");
    match preset.unknown {
        0 => t!("toolbar.imported", name = file_name(path), modes = modes),
        unknown => t!(
            "toolbar.imported_some",
            name = file_name(path),
            modes = modes,
            count = unknown
        ),
    }
}

/// What the dialog says when `path` is no preset this version reads.
fn refused(err: &PresetError, path: &Path) -> String {
    let name = file_name(path);
    match err {
        PresetError::NotJson | PresetError::NotPreset => t!("toolbar.not_preset", name = name),
        PresetError::Newer(_) => t!("toolbar.newer", name = name),
    }
}

/// The mode bar: the mode switcher and the mode's toolbar, one stop of Tab whose arrows move along it, then the
/// customize button. Its bottom edge and the chosen mode take the mode's accent. `has_document` dims the tools that
/// need one; `hidden` hides the bar (full screen).
pub(crate) fn mode_bar(
    modes: Modes,
    has_document: impl Fn() -> bool + Copy + 'static,
    hidden: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let shown_tools = dynamic(move || {
        let toolbar = modes.toolbar(modes.current());
        untracked(move || tools(&toolbar, has_document, run).into_element())
    });
    view()
        .h(px(40.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .bg(move || theme().background)
        .border_b(px(2.0))
        .border_b_color(move || accent(modes.current()))
        .style_when(hidden, |s| s.hidden())
        .child(
            toolbar()
                .accessibility_label(t!("modes.toolbar"))
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap_1()
                .overflow_hidden()
                .child(switcher(modes))
                .child(rule())
                // A narrow window scrolls the tools sideways rather than clipping the last ones.
                .child(
                    scroll()
                        .horizontal()
                        .scrollbar(false)
                        .flex_1()
                        .min_w(px(0.0))
                        .child(shown_tools),
                ),
        )
        .child(bar_text_button(
            t!("toolbar.customize"),
            commands::label_with_shortcut(CommandId::CustomizeToolbar),
            move || run(CommandId::CustomizeToolbar),
        ))
}

/// The modes offered, as one group of toggles: the chosen one filled with its accent, the others marked by a dot of
/// theirs.
fn switcher(modes: Modes) -> impl IntoElement {
    Mode::ALL
        .into_iter()
        .fold(toggle_group().flex_row().gap_1(), |group, mode| {
            group.child(when(
                move || modes.offered(mode),
                move || mode_toggle(modes, mode),
            ))
        })
}

fn mode_toggle(modes: Modes, mode: Mode) -> impl IntoElement {
    let chosen = move || modes.current() == mode;
    toggle()
        .pressed(chosen)
        .accessibility_label(mode_name(mode))
        .on_change(move |_: &bool| modes.set(mode))
        .h(px(28.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .pressed_style(move |s| s.bg(move || accent(mode)))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(move || {
            if chosen() {
                on_accent(mode)
            } else {
                accent(mode)
            }
        }))
        .child(
            text(move || mode_name(mode))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if chosen() {
                        on_accent(mode)
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// A thin upright line between runs of tools.
fn rule() -> impl IntoElement {
    toolbar_separator()
        .w(px(1.0))
        .h(px(20.0))
        .mx_1()
        .bg(move || theme().border)
}

/// `toolbar`'s tools, or a line saying it has none and how to add some.
fn tools(
    toolbar: &Toolbar,
    has_document: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> View {
    if toolbar.is_empty() {
        return view().child(
            text(t!("toolbar.empty"))
                .px_2()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        );
    }
    toolbar.items().iter().fold(
        view().flex_row().items_center().gap_1(),
        |row, item| match item {
            Item::Separator => row.child(rule()),
            Item::Command(name) => match commands::find(name) {
                Some(id) => row.child(tool_button(id, short_label(name), has_document, run)),
                None => row,
            },
        },
    )
}

/// The bar's shorter word for a command whose palette label is long, so a full toolbar fits a laptop's width.
fn short_label(name: &str) -> Option<&'static str> {
    match name {
        "view.layout_single" => Some("toolbar.short.single"),
        "view.layout_facing" => Some("toolbar.short.facing"),
        "view.toggle_continuous" => Some("toolbar.short.continuous"),
        "find.open" => Some("toolbar.short.find"),
        "view.reading_theme" => Some("toolbar.short.theme"),
        "bookmark.add" => Some("toolbar.short.bookmark"),
        "outline.add" => Some("toolbar.short.outline"),
        "markup.highlight" => Some("markup.highlight"),
        "markup.underline" => Some("markup.underline"),
        "markup.squiggly" => Some("markup.squiggly"),
        "markup.strikeout" => Some("markup.strikeout"),
        "markup.replace" => Some("markup.replace"),
        "markup.insert" => Some("markup.insert"),
        "annotate.note" => Some("toolbar.short.note"),
        "annotate.sticky" => Some("toolbar.short.sticky"),
        "annotate.text_box" => Some("toolbar.short.text_box"),
        "annotate.typewriter" => Some("toolbar.short.typewriter"),
        "annotate.callout" => Some("toolbar.short.callout"),
        "annotate.line" => Some("toolbar.short.line"),
        "annotate.arrow" => Some("toolbar.short.arrow"),
        "annotate.rectangle" => Some("toolbar.short.rectangle"),
        "annotate.ellipse" => Some("toolbar.short.ellipse"),
        "annotate.polygon" => Some("toolbar.short.polygon"),
        "annotate.polyline" => Some("toolbar.short.polyline"),
        "annotate.cloud" => Some("toolbar.short.cloud"),
        "annotate.regular_polygon" => Some("toolbar.short.regular_polygon"),
        "annotate.star" => Some("toolbar.short.star"),
        "annotate.colors" => Some("toolbar.short.colors"),
        "annotate.margin_notes" => Some("toolbar.short.margin_notes"),
        "annotate.list" => Some("toolbar.short.annotation_list"),
        "annotate.transfer" => Some("toolbar.short.transfer"),
        "annotate.cut" => Some("toolbar.short.cut"),
        "annotate.paste" => Some("toolbar.short.paste"),
        "annotate.delete" => Some("toolbar.short.delete"),
        "go.screen_up" => Some("toolbar.short.screen_up"),
        "go.screen_down" => Some("toolbar.short.screen_down"),
        _ => None,
    }
}

/// A tool: its short label (or its command's label); screen readers hear the full label and its shortcut.
fn tool_button(
    id: CommandId,
    short: Option<&'static str>,
    has_document: impl Fn() -> bool + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let off = move || !id.is_available(has_document());
    button()
        .accessibility_label(commands::label_with_shortcut(id))
        .disabled(off)
        .on_press(move || run(id))
        .h(px(28.0))
        .px_2()
        .flex_shrink_0()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(move || short.map_or_else(|| commands::label(id), |key| t!(key)))
                .text_size(px(13.0))
                .text_color(move || {
                    if off() {
                        theme().muted_foreground
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// A quiet text button on the mode bar or in the dialog's lists, showing `label` and heard as `spoken`.
fn bar_text_button(label: String, spoken: String, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(spoken)
        .on_press(press)
        .h(px(28.0))
        .px_2()
        .flex_shrink_0()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(label)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
}

/// A small square button with a glyph (a move or remove arrow), named for screen readers.
fn glyph_button(
    glyph: &'static str,
    label: String,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(label)
        .on_press(press)
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(glyph)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// The customize dialog, while it is open.
pub(crate) fn customize_dialog(modes: Modes) -> impl IntoElement {
    when(
        move || modes.is_customizing(),
        move || {
            let close = move || modes.close();
            let editing = move || modes.editing.get().unwrap_or_default();
            let content = view()
                .gap_3()
                .p_5()
                .child(
                    text(t!("toolbar.customize_title"))
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    text(t!("toolbar.customize_hint"))
                        .text_size(px(12.0))
                        .line_height(px(18.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(dialog_modes(modes))
                .child(dynamic(move || {
                    let mode = editing();
                    let toolbar = modes.toolbar(mode);
                    untracked(move || on_toolbar(modes, mode, &toolbar).into_element())
                }))
                .child(dynamic(move || {
                    let mode = editing();
                    let toolbar = modes.toolbar(mode);
                    untracked(move || addable(modes, mode, &toolbar).into_element())
                }))
                .child(when(
                    move || modes.notice.with(Option::is_some),
                    move || {
                        text(move || modes.notice.get().unwrap_or_default())
                            .text_size(px(12.0))
                            .line_height(px(18.0))
                            .text_color(move || theme().popover_foreground)
                    },
                ))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .pt_1()
                        .child(dialog_button(t!("toolbar.reset"), Tone::Quiet, move || {
                            let mode = untracked(editing);
                            modes.set_toolbar(mode, &default_toolbar(mode));
                        }))
                        .child(dialog_button(
                            t!("toolbar.export"),
                            Tone::Quiet,
                            move || modes.export(),
                        ))
                        .child(dialog_button(
                            t!("toolbar.import"),
                            Tone::Quiet,
                            move || modes.import(),
                        ))
                        .child(view().flex_1())
                        .child(dialog_button(t!("toolbar.done"), Tone::Primary, close)),
                );
            list_dialog(
                t!("toolbar.customize_title"),
                close,
                card(content).w(px(560.0)),
            )
        },
    )
}

/// Which mode's toolbar the dialog edits: every mode, Write and Study too, so the reader can fill them.
fn dialog_modes(modes: Modes) -> impl IntoElement {
    Mode::ALL.into_iter().fold(
        toggle_group()
            .flex_row()
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(8.0))
            .bg(move || theme().muted),
        |group, mode| {
            let chosen = move || modes.editing.get() == Some(mode);
            group.child(
                toggle()
                    .pressed(chosen)
                    .accessibility_label(mode_name(mode))
                    .on_change(move |_: &bool| {
                        modes.drag.set(None);
                        modes.editing.set(Some(mode));
                    })
                    .flex_1()
                    .h(px(28.0))
                    .rounded(px(6.0))
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .pressed_style(|s| s.bg(move || theme().background).shadow_sm())
                    .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                    .child(
                        view()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .bg(move || accent(mode)),
                    )
                    .child(
                        text(mode_name(mode))
                            .text_size(px(13.0))
                            .text_color(move || {
                                if chosen() {
                                    theme().popover_foreground
                                } else {
                                    theme().muted_foreground
                                }
                            }),
                    ),
            )
        },
    )
}

/// What is on `mode`'s toolbar, a row each. A row's name can be dragged up or down to move it; its arrow buttons do
/// the same from the keyboard.
fn on_toolbar(modes: Modes, mode: Mode, toolbar: &Toolbar) -> impl IntoElement {
    let count = toolbar.items().len();
    let heading = text(t!("toolbar.on_toolbar", mode = mode_name(mode)))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || theme().muted_foreground);
    if count == 0 {
        return view().gap_1().child(heading).child(
            text(t!("toolbar.none_yet"))
                .py_2()
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        );
    }
    let rows = toolbar
        .items()
        .iter()
        .enumerate()
        .fold(view().gap(px(0.0)), |rows, (at, item)| {
            rows.child(toolbar_row(modes, mode, at, count, item))
        });
    let drag = modes.drag;
    let list = scroll().max_h(px(ROW_HEIGHT * 6.0)).child(
        rows.on_mouse_move(move |event: &MouseEvent| {
            let Some(mut moving) = drag.get_untracked() else {
                return;
            };
            if event.pressed_button.is_none() {
                drag.set(None);
                return;
            }
            let rows = ((event.page_position.y.0 - moving.start_y) / ROW_HEIGHT).round() as isize;
            let to = (moving.from as isize + rows).clamp(0, count as isize - 1) as usize;
            if to != moving.to {
                moving.to = to;
                drag.set(Some(moving));
            }
        })
        .on_mouse_up(move |_: &MouseEvent| {
            if let Some(moving) = drag.get_untracked() {
                drag.set(None);
                if moving.to != moving.from {
                    modes.edit(mode, |toolbar| toolbar.move_item(moving.from, moving.to));
                }
            }
        }),
    );
    view().gap_1().child(heading).child(list)
}

fn toolbar_row(modes: Modes, mode: Mode, at: usize, count: usize, item: &Item) -> impl IntoElement {
    let name = match item {
        Item::Command(name) => commands::find(name)
            .map(commands::label)
            .unwrap_or_default(),
        Item::Separator => t!("toolbar.separator"),
    };
    let separator = *item == Item::Separator;
    let drag = modes.drag;
    let lifted = move || drag.with(|d| d.is_some_and(|d| d.from == at));
    let landing = move || drag.with(|d| d.is_some_and(|d| d.to == at && d.from != at));
    let mut row = view()
        .h(px(ROW_HEIGHT))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(6.0))
        .border_t(px(2.0))
        .border_t_color(move || {
            if landing() {
                accent(mode)
            } else {
                theme().popover
            }
        })
        .bg(move || {
            if lifted() {
                theme().accent
            } else {
                theme().popover
            }
        })
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex_row()
                .items_center()
                .gap_2()
                .on_press_down(move |event: &PressEvent| {
                    drag.set(Some(Drag {
                        from: at,
                        start_y: event.page_position.y.0,
                        to: at,
                    }));
                })
                // A grip: the row's name is where it is picked up.
                .child(
                    text("⠿")
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(name.clone())
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(13.0))
                        .text_color(move || {
                            if separator {
                                theme().muted_foreground
                            } else {
                                theme().popover_foreground
                            }
                        }),
                ),
        );
    if at > 0 {
        row = row.child(glyph_button(
            "↑",
            t!("toolbar.move_up", name = name.clone()),
            move || modes.edit(mode, |toolbar| toolbar.move_item(at, at - 1)),
        ));
    }
    if at + 1 < count {
        row = row.child(glyph_button(
            "↓",
            t!("toolbar.move_down", name = name.clone()),
            move || modes.edit(mode, |toolbar| toolbar.move_item(at, at + 1)),
        ));
    }
    row.child(glyph_button(
        "×",
        t!("toolbar.remove", name = name),
        move || modes.edit(mode, |toolbar| toolbar.remove(at)),
    ))
}

/// The commands that could go on `mode`'s toolbar, filtered by what the reader types, and a separator.
fn addable(modes: Modes, mode: Mode, toolbar: &Toolbar) -> impl IntoElement {
    let typed = signal(String::new());
    let missing: Vec<CommandId> = COMMANDS
        .iter()
        .filter(|command| !toolbar.contains(command.name))
        .map(|command| command.id)
        .collect();
    let rows = missing.into_iter().fold(view(), |rows, id| {
        let label = commands::label(id);
        let matches = {
            let label = label.to_lowercase();
            move || typed.with(|typed| label.contains(typed.trim().to_lowercase().as_str()))
        };
        let name = COMMANDS
            .iter()
            .find(|command| command.id == id)
            .map_or("", |command| command.name);
        rows.child(when(matches, move || {
            view()
                .h(px(32.0))
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .child(
                    text(label.clone())
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(13.0))
                        .text_color(move || theme().popover_foreground),
                )
                .child(bar_text_button(
                    t!("toolbar.add"),
                    t!("toolbar.add_named", name = label.clone()),
                    move || modes.edit(mode, |toolbar| toolbar.add(name)),
                ))
        }))
    });
    view()
        .gap_1()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(t!("toolbar.add_heading"))
                        .flex_1()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .child(bar_text_button(
                    t!("toolbar.add_separator"),
                    t!("toolbar.add_separator"),
                    move || modes.edit(mode, Toolbar::add_separator),
                )),
        )
        .child(
            input()
                .value(move || typed.get())
                .on_change(move |text: &str| typed.set(text.to_owned()))
                .placeholder(t!("toolbar.search"))
                .label(t!("toolbar.search"))
                .h(px(32.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || theme().border)
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(scroll().max_h(px(160.0)).child(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toolbars_name_only_commands_that_exist() {
        for mode in Mode::ALL {
            let toolbar = default_toolbar(mode);
            assert_eq!(
                toolbar.to_text(),
                default_words(mode).join(" "),
                "{mode:?}'s toolbar names a command the table lacks"
            );
        }
    }

    #[test]
    fn read_and_present_show_only_their_own_tools() {
        let read = default_toolbar(Mode::Read);
        let present = default_toolbar(Mode::Present);
        assert!(read.contains("view.zoom_in") && !read.contains("go.first_page"));
        assert!(present.contains("go.first_page") && !present.contains("view.zoom_in"));
        assert!(default_toolbar(Mode::Write).is_empty());
    }
}
