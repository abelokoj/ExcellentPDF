//! Present mode state machine, USB clicker keys, and pointer-triggered toolbar (plan G.1).

/// Screen blanking state triggered by clicker keys (`B` = black, `W` = white).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenBlank {
    /// Normal slide content visible.
    #[default]
    None,
    /// Solid black screen (`B` or `.`).
    Black,
    /// Solid white screen (`W` or `,`).
    White,
}

/// Logical presentation keys mapped from keyboard and standard USB presentation clickers (G.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClickerKey {
    /// Advance to next slide (`Right`, `Down`, `PageDown`, `Space`, `Enter`).
    Next,
    /// Return to previous slide (`Left`, `Up`, `PageUp`, `Backspace`).
    Prev,
    /// Jump to first slide (`Home`).
    First,
    /// Jump to last slide (`End`).
    Last,
    /// Toggle black screen (`B`, `.`).
    ToggleBlack,
    /// Toggle white screen (`W`, `,`).
    ToggleWhite,
    /// Exit presentation mode (`Escape`).
    Exit,
}

impl ClickerKey {
    /// Maps a standard DOM/NSH/OS key name to a [`ClickerKey`], matching standard USB clickers
    /// (Logitech R400/R800/Spotlight, Kensington, etc.) without any user configuration.
    #[must_use]
    pub fn from_key_name(key: &str) -> Option<Self> {
        match key {
            "PageDown" | "ArrowRight" | "ArrowDown" | "Right" | "Down" | "Space" | " "
            | "Enter" | "Return" => Some(Self::Next),
            "PageUp" | "ArrowLeft" | "ArrowUp" | "Left" | "Up" | "Backspace" => Some(Self::Prev),
            "Home" => Some(Self::First),
            "End" => Some(Self::Last),
            "b" | "B" | "." | "Period" => Some(Self::ToggleBlack),
            "w" | "W" | "," | "Comma" => Some(Self::ToggleWhite),
            "Escape" | "Esc" => Some(Self::Exit),
            _ => None,
        }
    }
}

/// Visibility state of the floating present-mode toolbar (shown on pointer movement only; G.1 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToolbarVisibility {
    /// Hidden while presenting so slides stay unobstructed.
    #[default]
    Hidden,
    /// Temporarily visible following pointer motion.
    Visible,
}

/// High-level action resulting from a presentation input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentAction {
    /// Navigated to a different slide index.
    PageChanged(usize),
    /// Blank screen toggled.
    BlankChanged(ScreenBlank),
    /// Request to leave present mode and restore the previous layout.
    ExitPresent,
    /// Key had no effect (e.g., `Prev` on slide 0).
    NoChange,
}

/// Default idle timeout before the present-mode toolbar hides again, in milliseconds.
pub const DEFAULT_TOOLBAR_HIDE_MS: u64 = 2_500;

/// Full-screen single-page presentation session state (plan G.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentSession {
    page_count: usize,
    current_page: usize,
    blank: ScreenBlank,
    active: bool,
    toolbar: ToolbarVisibility,
    last_pointer_move_ms: Option<u64>,
    toolbar_hide_delay_ms: u64,
}

impl PresentSession {
    /// Starts a new presentation session over a document with `page_count` pages, starting at
    /// `start_page` (clamped to valid range).
    #[must_use]
    pub fn start(page_count: usize, start_page: usize) -> Self {
        let clamped = if page_count == 0 {
            0
        } else {
            start_page.min(page_count - 1)
        };
        Self {
            page_count,
            current_page: clamped,
            blank: ScreenBlank::None,
            active: true,
            toolbar: ToolbarVisibility::Hidden,
            last_pointer_move_ms: None,
            toolbar_hide_delay_ms: DEFAULT_TOOLBAR_HIDE_MS,
        }
    }

    /// Whether present mode is currently active (full-screen, single-page, fit-page).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Zero-based index of the current slide.
    #[must_use]
    pub fn current_page(&self) -> usize {
        self.current_page
    }

    /// Zero-based index of the next slide, if any.
    #[must_use]
    pub fn next_page(&self) -> Option<usize> {
        let next = self.current_page + 1;
        (next < self.page_count).then_some(next)
    }

    /// Total slide count.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.page_count
    }

    /// Current screen blanking mode.
    #[must_use]
    pub fn blank(&self) -> ScreenBlank {
        self.blank
    }

    /// Current floating toolbar visibility.
    #[must_use]
    pub fn toolbar_visibility(&self) -> ToolbarVisibility {
        self.toolbar
    }

    /// Jumps directly to `page` (clamped to valid slide range) and clears screen blanking.
    pub fn go_to(&mut self, page: usize) -> PresentAction {
        if self.page_count == 0 {
            return PresentAction::NoChange;
        }
        let target = page.min(self.page_count - 1);
        self.blank = ScreenBlank::None;
        if target != self.current_page {
            self.current_page = target;
            PresentAction::PageChanged(target)
        } else {
            PresentAction::NoChange
        }
    }

    /// Handles a keyboard or USB clicker key press.
    pub fn handle_key(&mut self, key: ClickerKey) -> PresentAction {
        if !self.active {
            return PresentAction::NoChange;
        }
        match key {
            ClickerKey::Next => {
                if self.blank != ScreenBlank::None {
                    self.blank = ScreenBlank::None;
                    return PresentAction::BlankChanged(ScreenBlank::None);
                }
                if self.current_page + 1 < self.page_count {
                    self.current_page += 1;
                    PresentAction::PageChanged(self.current_page)
                } else {
                    PresentAction::NoChange
                }
            }
            ClickerKey::Prev => {
                if self.blank != ScreenBlank::None {
                    self.blank = ScreenBlank::None;
                    return PresentAction::BlankChanged(ScreenBlank::None);
                }
                if self.current_page > 0 {
                    self.current_page -= 1;
                    PresentAction::PageChanged(self.current_page)
                } else {
                    PresentAction::NoChange
                }
            }
            ClickerKey::First => self.go_to(0),
            ClickerKey::Last => self.go_to(self.page_count.saturating_sub(1)),
            ClickerKey::ToggleBlack => {
                self.blank = if self.blank == ScreenBlank::Black {
                    ScreenBlank::None
                } else {
                    ScreenBlank::Black
                };
                PresentAction::BlankChanged(self.blank)
            }
            ClickerKey::ToggleWhite => {
                self.blank = if self.blank == ScreenBlank::White {
                    ScreenBlank::None
                } else {
                    ScreenBlank::White
                };
                PresentAction::BlankChanged(self.blank)
            }
            ClickerKey::Exit => {
                self.active = false;
                self.blank = ScreenBlank::None;
                PresentAction::ExitPresent
            }
        }
    }

    /// Shows the present-mode toolbar in response to pointer movement at `now_ms`.
    pub fn on_pointer_move(&mut self, now_ms: u64) {
        self.last_pointer_move_ms = Some(now_ms);
        self.toolbar = ToolbarVisibility::Visible;
    }

    /// Advances idle timers at `now_ms`, hiding the toolbar once the pointer has been still for
    /// `toolbar_hide_delay_ms`.
    pub fn tick(&mut self, now_ms: u64) {
        if let Some(last) = self.last_pointer_move_ms
            && now_ms.saturating_sub(last) >= self.toolbar_hide_delay_ms
        {
            self.toolbar = ToolbarVisibility::Hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_clicker_keys_navigate_blank_and_exit_without_setup() {
        let mut session = PresentSession::start(5, 0);
        assert_eq!(session.toolbar_visibility(), ToolbarVisibility::Hidden);

        assert_eq!(
            session.handle_key(ClickerKey::from_key_name("PageDown").expect("pgdn")),
            PresentAction::PageChanged(1)
        );
        assert_eq!(
            session.handle_key(ClickerKey::from_key_name("PageUp").expect("pgup")),
            PresentAction::PageChanged(0)
        );
        // Black screen via 'B' and Period '.' (Logitech clicker black button).
        assert_eq!(
            session.handle_key(ClickerKey::from_key_name(".").expect("dot")),
            PresentAction::BlankChanged(ScreenBlank::Black)
        );
        assert_eq!(session.blank(), ScreenBlank::Black);
        assert_eq!(
            session.handle_key(ClickerKey::from_key_name("b").expect("b")),
            PresentAction::BlankChanged(ScreenBlank::None)
        );
        // White screen via 'W'.
        assert_eq!(
            session.handle_key(ClickerKey::from_key_name("W").expect("W")),
            PresentAction::BlankChanged(ScreenBlank::White)
        );

        // Toolbar shows only on pointer movement and auto-hides after idle timeout.
        session.on_pointer_move(1000);
        assert_eq!(session.toolbar_visibility(), ToolbarVisibility::Visible);
        session.tick(3499);
        assert_eq!(session.toolbar_visibility(), ToolbarVisibility::Visible);
        session.tick(3500);
        assert_eq!(session.toolbar_visibility(), ToolbarVisibility::Hidden);

        assert_eq!(
            session.handle_key(ClickerKey::from_key_name("Escape").expect("esc")),
            PresentAction::ExitPresent
        );
        assert!(!session.is_active());
    }
}
