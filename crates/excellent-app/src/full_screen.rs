//! Full-screen reading (R.19), the app's side: the window fills the screen with the pages alone. The docks close
//! and come back as they were; the title bar hides and comes back while the pointer is at the screen's top edge.
//!
//! The platform switches full screen on F11 itself; this follows the window's mode, so the chrome hides whichever
//! way the window got there.

use nsh_base::dock::{DockAreaLayout, DockRegion};
use nsh_core::JsonValue;
use nsh_core::desktop::WindowMode;
use nsh_core::element::desktop::{current_window, window_state};
use nsh_core::prelude::*;

/// Full screen, as the shell shows it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FullScreen {
    /// The mode to go back to: the last one that wasn't full screen.
    windowed: Signal<WindowMode>,
    /// The pointer brought the title bar back.
    revealed: Signal<bool>,
}

impl FullScreen {
    /// Follows the current window's mode while the current component lives, closing the docks of `layout` on
    /// entering full screen and opening them as they were on leaving it.
    pub(crate) fn new(layout: Signal<DockAreaLayout>) -> Self {
        let start = untracked(window_state).map_or(WindowMode::Normal, |state| state.mode);
        let windowed = signal(if start == WindowMode::FullScreen {
            WindowMode::Normal
        } else {
            start
        });
        let revealed = signal(false);
        current_window().on_mode_change(move |mode, _| {
            if mode != WindowMode::FullScreen {
                windowed.set(mode);
            }
        });
        let this = Self { windowed, revealed };
        // Each dock's open flag before full screen closed it; `None` while not in full screen.
        let docks: Signal<Option<(Option<bool>, Option<bool>)>> = signal(None);
        effect(move || {
            let full = this.is_on();
            untracked(|| match (full, docks.get()) {
                (true, None) => layout.update(|layout| {
                    let close = |dock: &mut Option<DockRegion>| {
                        let open = dock.as_ref().and_then(|dock| dock.open);
                        if let Some(dock) = dock {
                            dock.open = Some(false);
                        }
                        open
                    };
                    docks.set(Some((close(&mut layout.left), close(&mut layout.right))));
                }),
                (false, Some((left, right))) => {
                    docks.set(None);
                    revealed.set(false);
                    layout.update(|layout| {
                        if let Some(dock) = &mut layout.left {
                            dock.open = left;
                        }
                        if let Some(dock) = &mut layout.right {
                            dock.open = right;
                        }
                    });
                }
                _ => {}
            });
        });
        this
    }

    /// Whether the window is in full screen (reactive).
    pub(crate) fn is_on(self) -> bool {
        window_state().is_some_and(|state| state.mode == WindowMode::FullScreen)
    }

    /// Goes into full screen, or back to the mode before it.
    pub(crate) fn toggle(self) {
        let mode = if untracked(|| self.is_on()) {
            untracked(|| self.windowed.get())
        } else {
            WindowMode::FullScreen
        };
        current_window().set_mode(mode);
    }

    /// Leaves full screen, if the window is in it.
    pub(crate) fn leave(self) {
        if untracked(|| self.is_on()) {
            self.toggle();
        }
    }

    /// Whether the title bar is hidden (reactive): in full screen, until the pointer reaches the top edge.
    pub(crate) fn hides_bar(self) -> bool {
        self.is_on() && !self.revealed.get()
    }

    /// The pointer left the title bar: it hides again.
    pub(crate) fn bar_left(self) {
        self.revealed.set(false);
    }

    /// Takes the canvas's `edge` event: the pointer at the top edge brings back the title bar. Only reaching the
    /// edge counts: the bar, once back, pushes the pages down from under the pointer, and leaving the bar hides it.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name == "edge"
            && event.data.get("at").and_then(JsonValue::as_bool) == Some(true)
            && untracked(|| self.is_on())
        {
            self.revealed.set(true);
        }
    }
}
