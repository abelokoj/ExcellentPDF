//! Drawing guides (H.11), the app's side: the ruler, protractor and compass commands lay one on the page (the canvas's
//! `guide` prop) or put it away, and what a stroke drawn along it measures (`guideReading`) shows in the title bar.

use nsh_base::theme;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::AnnotateState;

/// A guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuideKind {
    /// A straight edge.
    Ruler,
    /// A half circle of angles.
    Protractor,
    /// A circle of a set radius.
    Compass,
}

impl GuideKind {
    fn name(self) -> &'static str {
        match self {
            Self::Ruler => "ruler",
            Self::Protractor => "protractor",
            Self::Compass => "compass",
        }
    }
}

impl AnnotateState {
    /// Lays `kind` on the page, or puts it away when it is the one there.
    pub(crate) fn toggle_guide(self, kind: GuideKind) {
        let next = (untracked(|| self.guide.get()) != Some(kind)).then_some(kind);
        self.guide.set(next);
        self.reading.set(None);
    }

    /// The guide on the page, for the toolbar.
    pub(crate) fn guide_on(self, kind: GuideKind) -> bool {
        self.guide.get() == Some(kind)
    }

    /// The canvas's `guide` prop.
    pub(crate) fn guide_prop(self) -> JsonValue {
        self.guide
            .get()
            .map_or(JsonValue::Null, |kind| JsonValue::from(kind.name()))
    }

    /// The canvas measured a stroke drawn along the guide (`guideReading`).
    pub(super) fn guide_reading(self, data: &JsonValue) {
        let number = |key: &str| {
            data.get(key)
                .and_then(JsonValue::as_f64)
                .unwrap_or_default()
        };
        let line = match data.get("kind").and_then(JsonValue::as_str) {
            Some("line") => t!(
                "guide.reading_line",
                length = format!("{:.1}", number("length") / 72.0 * 25.4),
                degrees = format!("{:.0}", number("degrees").rem_euclid(360.0))
            ),
            Some("arc") => t!(
                "guide.reading_arc",
                radius = format!("{:.1}", number("radius") / 72.0 * 25.4),
                degrees = format!("{:.0}", number("degrees"))
            ),
            _ => return,
        };
        self.reading.set(Some(line));
    }
}

/// The last measurement, in the title bar while a guide is on the page.
pub(crate) fn reading_line(state: AnnotateState) -> impl IntoElement {
    when(
        move || state.guide.get().is_some() && state.reading.with(Option::is_some),
        move || {
            text(move || state.reading.get().unwrap_or_default())
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground)
        },
    )
}
