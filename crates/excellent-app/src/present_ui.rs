//! Classroom Presentation Mode UI & Overlays (G.1–G.8):
//! - Full-screen present session, USB clicker keys & Black/White blank screen (G.1)
//! - Laser pointer dot & fading trail (G.2)
//! - Spotlight region & Magnifier lens overlays (G.3, G.4)
//! - Temporary & Fading slide ink + Keep/Discard prompt on exit (G.5)
//! - Temporary Whiteboard overlay & export to `.expdf` (G.6)
//! - Curtain reveal overlay (Top/Bottom/Left/Right) + keyboard step reveal (G.7)
//! - Secondary Presenter View window with next-slide preview, notes & elapsed timer (G.8)

use excellent_core::edit::Change;
use excellent_features::present::{
    ActivePointerTool, ClickerKey, CurtainDirection, CurtainReveal, InkPersistence, LaserPointer,
    MagnifierLens, PresentAction, PresentSession, PresenterNotes, PresenterTimer, ScreenBlank,
    SlideInkLayer, Spotlight, WhiteboardBackground, WhiteboardDeck,
};
use excellent_mupdf::PagePoint;
use nsh_base::{button, theme};
use nsh_core::element::desktop::{OpenChange, open_window};
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::annotate::AnnotateState;
use crate::commands::CommandId;
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// Presentation & Classroom session state attached to the main window.
#[derive(Clone, Copy)]
pub(crate) struct PresentState {
    annotate: AnnotateState,
    active: Signal<bool>,
    session: Stored<Option<PresentSession>>,
    blank: Signal<ScreenBlank>,
    pointer_tool: Signal<ActivePointerTool>,
    laser: Stored<LaserPointer>,
    spotlight: Stored<Spotlight>,
    magnifier: Stored<MagnifierLens>,
    pointer_pos: Signal<Option<(f32, f32)>>,
    ink_persistence: Signal<InkPersistence>,
    slide_ink: Stored<SlideInkLayer>,
    whiteboard: Stored<WhiteboardDeck>,
    showing_whiteboard: Signal<bool>,
    curtain: Stored<CurtainReveal>,
    curtain_active: Signal<bool>,
    curtain_fraction: Signal<f32>,
    curtain_dir: Signal<CurtainDirection>,
    notes: Stored<PresenterNotes>,
    timer: Stored<PresenterTimer>,
    presenter_open: Signal<bool>,
    keep_prompt_open: Signal<bool>,
}

impl PresentState {
    /// Creates classroom presentation state.
    pub(crate) fn new(annotate: AnnotateState) -> Self {
        Self {
            annotate,
            active: signal(false),
            session: stored(None),
            blank: signal(ScreenBlank::None),
            pointer_tool: signal(ActivePointerTool::None),
            laser: stored(LaserPointer::default()),
            spotlight: stored(Spotlight::default()),
            magnifier: stored(MagnifierLens::default()),
            pointer_pos: signal(None),
            ink_persistence: signal(InkPersistence::Temporary),
            slide_ink: stored(SlideInkLayer::new()),
            whiteboard: stored(WhiteboardDeck::new()),
            showing_whiteboard: signal(false),
            curtain: stored(CurtainReveal::default()),
            curtain_active: signal(false),
            curtain_fraction: signal(0.0),
            curtain_dir: signal(CurtainDirection::TopToBottom),
            notes: stored(PresenterNotes::new()),
            timer: stored(PresenterTimer::default()),
            presenter_open: signal(false),
            keep_prompt_open: signal(false),
        }
    }

    /// Starts or refreshes presentation state for a document with `page_count` pages at `start_page` (G.1).
    pub(crate) fn start(self, page_count: usize, start_page: usize) {
        let sess = PresentSession::start(page_count.max(1), start_page);
        self.session.with(|slot| *slot = Some(sess));
        self.blank.set(ScreenBlank::None);
        self.active.set(true);
        self.timer.with(|t| t.start(now_ms()));
    }

    /// Ends presentation mode; prompts to Keep or Discard any temporary slide ink first (G.5).
    pub(crate) fn end(self) {
        let has_ink = self.slide_ink.with(|layer| layer.total_strokes() > 0);
        if has_ink {
            self.keep_prompt_open.set(true);
            return;
        }
        self.finish_end();
    }

    fn finish_end(self) {
        self.active.set(false);
        self.blank.set(ScreenBlank::None);
        self.pointer_tool.set(ActivePointerTool::None);
        self.showing_whiteboard.set(false);
        self.curtain_active.set(false);
        self.keep_prompt_open.set(false);
    }

    /// Commits temporary slide ink as standard PDF Ink annotations and ends presentation mode (G.5).
    pub(crate) fn keep_slide_ink(self) {
        let now = now_ms();
        let changes: Vec<Change> = self
            .slide_ink
            .with(|layer| layer.keep_to_changes("ExcellentPDF", now));
        if !changes.is_empty() {
            self.annotate
                .editing_apply(t!("present.ink.keep_edit"), changes);
        }
        self.finish_end();
    }

    /// Discards temporary slide ink and ends presentation mode (G.5).
    pub(crate) fn discard_slide_ink(self) {
        self.slide_ink.with(SlideInkLayer::discard_all);
        self.finish_end();
    }

    /// Handles a USB clicker / keyboard key in Present mode (G.1, G.7).
    pub(crate) fn handle_key(
        self,
        key: &str,
        goto_page: impl FnOnce(usize),
        exit_present: impl FnOnce(),
    ) -> bool {
        if !untracked(|| self.active.get()) {
            return false;
        }
        // Curtain reveal steps with Up/Down when curtain overlay is active (G.7).
        if untracked(|| self.curtain_active.get()) {
            match key {
                "ArrowDown" | "Down" | "ArrowRight" | "Right" => {
                    let frac = self.curtain.with(|c| {
                        c.step_reveal();
                        c.revealed_fraction()
                    });
                    self.curtain_fraction.set(frac);
                    return true;
                }
                "ArrowUp" | "Up" | "ArrowLeft" | "Left" => {
                    let frac = self.curtain.with(|c| {
                        c.step_hide();
                        c.revealed_fraction()
                    });
                    self.curtain_fraction.set(frac);
                    return true;
                }
                _ => {}
            }
        }
        let Some(ckey) = ClickerKey::from_key_name(key) else {
            return false;
        };
        let action = self.session.with(|slot| {
            slot.as_mut()
                .map_or(PresentAction::NoChange, |s| s.handle_key(ckey))
        });
        match action {
            PresentAction::PageChanged(page) => {
                goto_page(page);
                true
            }
            PresentAction::BlankChanged(b) => {
                self.blank.set(b);
                true
            }
            PresentAction::ExitPresent => {
                exit_present();
                true
            }
            PresentAction::NoChange => false,
        }
    }

    /// Selects Laser, Spotlight, Magnifier, or Normal pointer overlay tool (G.2–G.4).
    pub(crate) fn select_pointer_tool(self, tool: ActivePointerTool) {
        let current = untracked(|| self.pointer_tool.get());
        let next = if current == tool {
            ActivePointerTool::None
        } else {
            tool
        };
        self.pointer_tool.set(next);
    }

    /// Toggles Black or White blank screen (G.1).
    pub(crate) fn toggle_blank(self, screen: ScreenBlank) {
        let cur = untracked(|| self.blank.get());
        let next = if cur == screen {
            ScreenBlank::None
        } else {
            screen
        };
        self.blank.set(next);
    }

    /// Switches between Temporary and Fading slide ink persistence (G.5).
    pub(crate) fn toggle_ink_persistence(self) {
        let next = match untracked(|| self.ink_persistence.get()) {
            InkPersistence::Temporary => InkPersistence::Fading { lifetime_ms: 3_500 },
            InkPersistence::Fading { .. } => InkPersistence::Temporary,
        };
        self.ink_persistence.set(next);
    }

    /// Opens a temporary Whiteboard page or returns to slides (G.6).
    pub(crate) fn toggle_whiteboard(self, bg: WhiteboardBackground) {
        let showing = untracked(|| self.showing_whiteboard.get());
        if showing {
            self.whiteboard.with(WhiteboardDeck::return_to_slides);
            self.showing_whiteboard.set(false);
        } else {
            let page = self.annotate.active_page();
            self.whiteboard
                .with(|deck| deck.insert_and_activate(page, bg));
            self.showing_whiteboard.set(true);
        }
    }

    /// Exports temporary whiteboard pages as a standalone `.expdf` notebook (G.6).
    pub(crate) fn export_whiteboard(self) {
        let nb = match self.whiteboard.with(|d| d.export_to_notebook("Whiteboard")) {
            Ok(Some(nb)) => nb,
            Ok(None) => {
                show_message(t!("command.whiteboard"), t!("notebooks.empty"));
                return;
            }
            Err(err) => {
                show_message(t!("command.whiteboard"), err.to_string());
                return;
            }
        };
        let dialog = SaveDialog::new()
            .file_name("Presentation Whiteboard.expdf".to_owned())
            .filter(FileFilter::new(t!("open.filter.expdf"), ["expdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = nb.save_atomic(&path) {
                show_message(t!("command.whiteboard"), err.to_string());
            }
        });
    }

    /// Toggles the Curtain Reveal overlay (G.7).
    pub(crate) fn toggle_curtain(self) {
        let on = !untracked(|| self.curtain_active.get());
        if on {
            self.curtain.with(|c| c.enable());
            self.curtain_fraction
                .set(self.curtain.with(|c| c.revealed_fraction()));
        } else {
            self.curtain.with(|c| c.disable());
        }
        self.curtain_active.set(on);
    }

    /// Opens the secondary Presenter View window via NSH `open_window` (G.8).
    pub(crate) fn open_presenter_view(self) {
        if untracked(|| self.presenter_open.get()) {
            return;
        }
        let opened = open_window(
            nsh_core::element::desktop::window()
                .id("presenter-view".to_owned())
                .title(move || t!("present.presenter_view.title"))
                .default_size(size(px(760.0), px(520.0)))
                .on_open_change(move |chg: &OpenChange| {
                    if !chg.open {
                        self.presenter_open.set(false);
                    }
                })
                .content(move || presenter_window_content(self)),
        );
        if opened.is_some() {
            self.presenter_open.set(true);
        }
    }

    /// Whether any blocking Present dialog (`keep_prompt_open`) is showing.
    pub(crate) fn is_keep_prompt_open(self) -> bool {
        self.keep_prompt_open.get()
    }

    /// Whether a full-screen overlay (`blank`, `whiteboard`, `curtain`) needs the native canvas hidden or covered.
    pub(crate) fn is_overlay_active(self) -> bool {
        self.blank.get() != ScreenBlank::None
            || self.showing_whiteboard.get()
            || self.curtain_active.get()
            || self.keep_prompt_open.get()
    }

    /// Active pointer overlay tool for UI pressed states.
    pub(crate) fn pointer_tool(self) -> ActivePointerTool {
        self.pointer_tool.get()
    }

    /// Whether a Present toolbar command should appear toggled on (G.1–G.7).
    pub(crate) fn is_on(self, id: CommandId) -> bool {
        match id {
            CommandId::PresentLaser => self.pointer_tool.get() == ActivePointerTool::Laser,
            CommandId::PresentSpotlight => self.pointer_tool.get() == ActivePointerTool::Spotlight,
            CommandId::PresentMagnifier => self.pointer_tool.get() == ActivePointerTool::Magnifier,
            CommandId::PresentBlankBlack => self.blank.get() == ScreenBlank::Black,
            CommandId::PresentBlankWhite => self.blank.get() == ScreenBlank::White,
            CommandId::PresentFadingInk => {
                matches!(self.ink_persistence.get(), InkPersistence::Fading { .. })
            }
            CommandId::PresentWhiteboard => self.showing_whiteboard.get(),
            CommandId::PresentCurtain => self.curtain_active.get(),
            _ => false,
        }
    }

    /// Updates live pointer coordinates over the slide stage for same-frame Laser/Spotlight/Magnifier rendering (G.2–G.4).
    pub(crate) fn update_pointer(self, x: f32, y: f32) {
        self.pointer_pos.set(Some((x, y)));
        match untracked(|| self.pointer_tool.get()) {
            ActivePointerTool::Laser => {
                self.laser
                    .with(|l| l.move_pointer(PagePoint { x, y }, now_ms()));
            }
            ActivePointerTool::Spotlight => {
                self.spotlight.with(|s| s.move_to(PagePoint { x, y }));
            }
            ActivePointerTool::Magnifier => {
                self.magnifier.with(|m| m.move_to(PagePoint { x, y }));
            }
            ActivePointerTool::None => {}
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Overlay layers rendered in Present mode: Black/White screen, Whiteboard, Curtain, and Pointer reticle (G.1–G.7).
pub(crate) fn present_overlays(state: PresentState) -> impl IntoElement {
    dynamic(move || {
        if !state.active.get() {
            return view();
        }
        let mut root = view().gap_0();
        // Blank screen overlay (G.1)
        match state.blank.get() {
            ScreenBlank::Black => {
                return view()
                    .flex_1()
                    .bg(Hsla::from(rgb(0x00_00_00)))
                    .items_center()
                    .justify_center()
                    .child(action_button(
                        "present.resume_slides",
                        Tone::Plain,
                        move || {
                            state.toggle_blank(ScreenBlank::None);
                        },
                    ));
            }
            ScreenBlank::White => {
                return view()
                    .flex_1()
                    .bg(Hsla::from(rgb(0xff_ff_ff)))
                    .items_center()
                    .justify_center()
                    .child(action_button(
                        "present.resume_slides",
                        Tone::Plain,
                        move || {
                            state.toggle_blank(ScreenBlank::None);
                        },
                    ));
            }
            ScreenBlank::None => {}
        }
        // Temporary whiteboard (G.6)
        if state.showing_whiteboard.get() {
            return view()
                .flex_1()
                .bg(Hsla::from(rgb(0xff_ff_ff)))
                .p_4()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(move || t!("command.whiteboard"))
                                .flex_1()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(Hsla::from(rgb(0x11_18_27))),
                        )
                        .child(action_button(
                            "present.whiteboard.export",
                            Tone::Plain,
                            move || state.export_whiteboard(),
                        ))
                        .child(action_button(
                            "present.resume_slides",
                            Tone::Plain,
                            move || state.toggle_whiteboard(WhiteboardBackground::Blank),
                        )),
                );
        }
        // Curtain reveal controls strip (G.7)
        if state.curtain_active.get() {
            let pct = (state.curtain_fraction.get() * 100.0).round() as u32;
            root = root.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .bg(move || theme().muted)
                    .border_b_1()
                    .border_b_color(move || theme().border)
                    .child(
                        text(move || t!("present.curtain.revealed", percent = pct))
                            .text_size(px(13.0))
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        segmented()
                            .child(segment_of(
                                move || t!("present.curtain.top"),
                                move || state.curtain_dir.get() == CurtainDirection::TopToBottom,
                                move || {
                                    state.curtain_dir.set(CurtainDirection::TopToBottom);
                                    state
                                        .curtain
                                        .with(|c| c.direction = CurtainDirection::TopToBottom);
                                },
                            ))
                            .child(segment_of(
                                move || t!("present.curtain.left"),
                                move || state.curtain_dir.get() == CurtainDirection::LeftToRight,
                                move || {
                                    state.curtain_dir.set(CurtainDirection::LeftToRight);
                                    state
                                        .curtain
                                        .with(|c| c.direction = CurtainDirection::LeftToRight);
                                },
                            )),
                    )
                    .child(action_button(
                        "present.curtain.step_more",
                        Tone::Plain,
                        move || {
                            let f = state.curtain.with(|c| {
                                c.step_reveal();
                                c.revealed_fraction()
                            });
                            state.curtain_fraction.set(f);
                        },
                    ))
                    .child(action_button(
                        "present.curtain.step_less",
                        Tone::Plain,
                        move || {
                            let f = state.curtain.with(|c| {
                                c.step_hide();
                                c.revealed_fraction()
                            });
                            state.curtain_fraction.set(f);
                        },
                    )),
            );
        }
        // Active pointer overlay status (G.2–G.4)
        let tool = state.pointer_tool();
        if tool != ActivePointerTool::None {
            let (tool_key, coords) = match tool {
                ActivePointerTool::Laser => {
                    ("command.present.laser", state.laser.with(|l| l.position))
                }
                ActivePointerTool::Spotlight => (
                    "command.present.spotlight",
                    state.spotlight.with(|s| Some(s.overlay().center)),
                ),
                ActivePointerTool::Magnifier => (
                    "command.present.magnifier",
                    state.magnifier.with(|m| Some(m.center)),
                ),
                ActivePointerTool::None => ("", None),
            };
            let pos = state
                .pointer_pos
                .get()
                .or_else(|| coords.map(|p| (p.x, p.y)));
            let label = match pos {
                Some((x, y)) => format!("{} ({:.0}, {:.0})", t!(tool_key), x, y),
                None => t!(tool_key),
            };
            root = root.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_1()
                    .bg(move || theme().muted)
                    .border_b_1()
                    .border_b_color(move || theme().border)
                    .child(
                        text(label)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    ),
            );
        }
        root
    })
}

/// Dialog prompting whether to Keep or Discard temporary presentation ink (G.5).
pub(crate) fn keep_ink_dialog(state: PresentState) -> impl IntoElement {
    let close = move || state.discard_slide_ink();
    let body = view()
        .w(px(380.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("present.ink.keep_title"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(move || t!("present.ink.keep_hint"))
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button(
                    "present.ink.discard",
                    Tone::Plain,
                    move || {
                        state.discard_slide_ink();
                    },
                ))
                .child(crate::shell::primary_button(
                    "present.ink.keep",
                    move || {
                        state.keep_slide_ink();
                    },
                )),
        );
    list_dialog(t!("present.ink.keep_title"), close, card(body))
}

/// Secondary Presenter View window body: slide indicator, speaker notes & elapsed timer (G.8).
fn presenter_window_content(state: PresentState) -> impl IntoElement {
    let page = move || state.annotate.active_page() + 1;
    let note_draft = signal(String::new());
    view()
        .flex_1()
        .p_4()
        .gap_3()
        .bg(move || theme().background)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    text(move || t!("present.presenter_view.slide", page = page()))
                        .flex_1()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || {
                        let elapsed = state.timer.with(|t| t.elapsed(now_ms()));
                        let secs = elapsed.as_secs();
                        format!("{:02}:{:02}", secs / 60, secs % 60)
                    })
                    .text_size(px(18.0))
                    .tabular_nums()
                    .text_color(move || theme().primary),
                ),
        )
        .child(
            nsh_base::input()
                .value(move || {
                    let p = state.annotate.active_page();
                    state
                        .notes
                        .with(|n| n.note_for(p).map(str::to_owned))
                        .unwrap_or_else(|| note_draft.get())
                })
                .multiline(true)
                .h(px(260.0))
                .on_change(move |txt: &str| {
                    let p = state.annotate.active_page();
                    note_draft.set(txt.to_owned());
                    state.notes.with(|n| n.set_note(p, txt));
                })
                .placeholder(t!("present.presenter_view.notes_placeholder"))
                .label(t!("present.presenter_view.notes")),
        )
        .child(
            view().flex_row().gap_2().child(
                button()
                    .on_press(move || {
                        state.timer.with(|t| t.reset());
                        state.timer.with(|t| t.start(now_ms()));
                    })
                    .child(text(move || t!("present.presenter_view.reset_timer"))),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_ink::stroke::Stroke;

    #[test]
    fn clicker_keys_advance_and_blank_slides() {
        let mut session = PresentSession::start(5, 1);
        assert_eq!(
            session.handle_key(ClickerKey::Next),
            PresentAction::PageChanged(2)
        );
        assert_eq!(
            session.handle_key(ClickerKey::ToggleBlack),
            PresentAction::BlankChanged(ScreenBlank::Black)
        );
        assert_eq!(
            session.handle_key(ClickerKey::ToggleWhite),
            PresentAction::BlankChanged(ScreenBlank::White)
        );
    }

    #[test]
    fn slide_ink_keeps_strokes_as_annotations() {
        let mut layer = SlideInkLayer::new();
        let mut stroke = Stroke::new();
        stroke.push(excellent_ink::stroke::InkPoint {
            x: 100.0,
            y: 100.0,
            width: 3.0,
        });
        stroke.push(excellent_ink::stroke::InkPoint {
            x: 200.0,
            y: 140.0,
            width: 3.0,
        });
        layer.add_stroke(excellent_features::present::SlideStroke {
            page: 0,
            stroke,
            color: [1.0, 0.0, 0.0],
            base_opacity: 1.0,
            persistence: InkPersistence::Temporary,
            finished_at_ms: 0,
        });
        assert!(layer.total_strokes() > 0);
        let changes = layer.keep_to_changes("Presenter", 1_000);
        assert_eq!(changes.len(), 1);
    }
}
