//! The first-run guide and tips (plan U.9), the view: three short steps in a dialog the first time the app starts,
//! and a one-line tip below the pages the first time each tip's moment comes. The model (the steps, when the guide
//! opens, the tips seen) is `ecclesia_features::onboarding`.
//!
//! Only the desktop binary offers them ([`offer`]): tests start the app with neither, so no dialog lies over what they
//! drive. The guide never opens over files the launch opens, and closes when a later launch brings files.

use std::sync::atomic::{AtomicBool, Ordering};

use ecclesia_features::onboarding::{Guide, STEPS, SeenTips, Step, Tip, opens_at_start};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::links::{Tone, dialog_button};
use crate::list_dialog::{card, list_dialog};
use crate::print::strip_button;
use crate::settings::Live;

/// `guide_seen` in `settings.toml`: the guide was finished or skipped, so it doesn't open by itself again.
pub(crate) const GUIDE_SEEN: &str = "guide_seen";
/// `tips_seen`: the tips shown already, by id, separated by commas.
pub(crate) const TIPS_SEEN: &str = "tips_seen";
/// `show_tips`: whether tips show at all (default on).
pub(crate) const SHOW_TIPS: &str = "show_tips";

static OFFERED: AtomicBool = AtomicBool::new(false);

/// Lets the guide and the tips show in this run; the desktop binary calls it (through `offer_first_run_guide`).
pub(crate) fn offer() {
    OFFERED.store(true, Ordering::Relaxed);
}

fn offered() -> bool {
    OFFERED.load(Ordering::Relaxed)
}

/// The guide and the tip strip's state, for the window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Onboarding {
    guide: Signal<Option<Guide>>,
    tip: Signal<Option<Tip>>,
    live: Live,
}

impl Onboarding {
    /// The window's: the guide open when this start is its moment.
    pub(crate) fn new(live: Live) -> Self {
        let launch_files = crate::open::startup().map_or(0, |launch| launch.files.len());
        let seen = live.read(|s| s.flag(GUIDE_SEEN).unwrap_or(false));
        let opens = offered() && opens_at_start(seen, launch_files);
        Self {
            guide: signal(opens.then(Guide::default)),
            tip: signal(None),
            live,
        }
    }

    /// A workspace window's (W.8): no guide until the reader asks for it; tips as the main window's.
    pub(crate) fn quiet(live: Live) -> Self {
        Self {
            guide: signal(None),
            tip: signal(None),
            live,
        }
    }

    /// Opens the guide at its first step (the replay command); it closes any tip.
    pub(crate) fn replay(self) {
        self.tip.set(None);
        self.guide.set(Some(Guide::default()));
    }

    /// Whether the guide is open.
    pub(crate) fn is_open(self) -> bool {
        self.guide.with(Option::is_some)
    }

    /// Closes the guide without counting it seen: files came, and they are what the reader wants now. It opens
    /// again at a start without files.
    pub(crate) fn make_way(self) {
        self.guide.set(None);
    }

    /// Closes the guide, finished or skipped; it won't open by itself again.
    fn finish(self) {
        self.guide.set(None);
        self.live.change(|s| s.set_flag(GUIDE_SEEN, Some(true)));
    }

    /// `tip`'s moment has come: it shows, unless tips are off, it was seen, or the guide or another tip is showing.
    /// Shown is seen: it doesn't come back in a later run.
    pub(crate) fn moment(self, tip: Tip) {
        untracked(|| {
            if !offered() || self.is_open() || self.tip.with(Option::is_some) {
                return;
            }
            let mut seen = self
                .live
                .read(|s| SeenTips::parse(s.text(TIPS_SEEN).unwrap_or_default()));
            let on = self.live.read(|s| s.flag(SHOW_TIPS).unwrap_or(true));
            if !seen.shows(tip, on) {
                return;
            }
            seen.insert(tip);
            self.live
                .change(|s| s.set_text(TIPS_SEEN, Some(&seen.to_text())));
            self.tip.set(Some(tip));
        });
    }

    fn dismiss_tip(self) {
        self.tip.set(None);
    }

    fn turn_tips_off(self) {
        self.tip.set(None);
        self.live.change(|s| s.set_flag(SHOW_TIPS, Some(false)));
    }
}

/// The guide's title and text for `step`, with the keys the reader has bound.
fn words(step: Step) -> (String, String) {
    let keys = commands::shortcut_label;
    match step {
        Step::Open => (
            t!("guide.open.title"),
            t!("guide.open.body", open = keys(CommandId::OpenFile)),
        ),
        Step::Read => (
            t!("guide.read.title"),
            t!("guide.read.body", settings = keys(CommandId::OpenSettings)),
        ),
        Step::Palette => (
            t!("guide.palette.title"),
            t!("guide.palette.body", palette = keys(CommandId::OpenPalette)),
        ),
    }
}

/// The guide, while it is open. `run` runs the first step's Open (after the guide closes, counted seen).
pub(crate) fn guide_dialog(
    state: Onboarding,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    when(
        move || state.is_open(),
        move || {
            let skip = move || state.finish();
            let at = move || state.guide.get().unwrap_or_default();
            dynamic(move || {
                let guide = at();
                untracked(move || guide_card(state, guide, skip, run).into_element())
            })
        },
    )
}

fn guide_card(
    state: Onboarding,
    guide: Guide,
    skip: impl Fn() + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let (title, body) = words(guide.step());
    let go = move |to: Option<Guide>| match to {
        Some(to) => state.guide.set(Some(to)),
        None => state.finish(),
    };
    // Skip first, so Enter on the first focus doesn't move on unseen; the primary button is the way forward.
    let mut actions = view()
        .flex_row()
        .items_center()
        .gap_2()
        .pt_3()
        .child(dialog_button(t!("guide.skip"), Tone::Quiet, skip))
        .child(view().flex_1());
    if guide.step() == Step::Open {
        actions = actions.child(dialog_button(
            t!("guide.open_now"),
            Tone::Quiet,
            move || {
                state.finish();
                run(CommandId::OpenFile);
            },
        ));
    }
    if !guide.is_first() {
        actions = actions.child(dialog_button(t!("guide.back"), Tone::Quiet, move || {
            go(guide.back())
        }));
    }
    let next = if guide.is_last() {
        t!("guide.done")
    } else {
        t!("guide.next")
    };
    actions = actions.child(dialog_button(next, Tone::Primary, move || go(guide.next())));
    let progress = text(t!(
        "guide.step",
        number = guide.number(),
        total = STEPS.len()
    ))
    .text_size(px(12.0))
    .tabular_nums()
    .text_color(move || theme().muted_foreground);
    let content = view()
        .gap_2()
        .p_5()
        .child(progress)
        .child(
            text(title)
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(body)
                .text_size(px(13.0))
                .line_height(px(20.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(actions);
    list_dialog(t!("guide.title"), skip, card(content).w(px(460.0)))
}

/// The tip strip below the pages, while a tip shows.
pub(crate) fn tip_strip(state: Onboarding) -> impl IntoElement {
    when(
        move || state.tip.with(Option::is_some),
        move || {
            let line = move || match state.tip.get() {
                Some(Tip::Palette) => t!(
                    "tip.palette",
                    keys = commands::shortcut_label(CommandId::OpenPalette)
                ),
                Some(Tip::TabList) => t!(
                    "tip.tab_list",
                    keys = commands::shortcut_label(CommandId::TabList)
                ),
                None => String::new(),
            };
            view()
                .label(t!("tip.strip"))
                .h(px(36.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .bg(move || theme().sidebar)
                .border_t_1()
                .border_t_color(move || theme().border)
                .child(
                    text(line)
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(move || theme().sidebar_foreground),
                )
                .child(strip_button("tip.turn_off", move || state.turn_tips_off()))
                .child(strip_button("tip.dismiss", move || state.dismiss_tip()))
        },
    )
}
