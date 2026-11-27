//! Multi-monitor presenter view synchronization and display-disconnect resilience (plan G.8).
//!
//! Coordinates an audience window (showing only the slide) and a presenter window (showing current
//! slide, next slide preview, speaker notes, elapsed timer, and wall clock) across any two
//! monitors. Gracefully degrades to single-display mode if a monitor is unplugged mid-talk and
//! seamlessly restores presenter view if a second display is reconnected.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::present::session::{ClickerKey, PresentAction, PresentSession, ScreenBlank};

/// Screen geometry rectangle in virtual desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayRect {
    /// Left X coordinate in virtual desktop space.
    pub x: i32,
    /// Top Y coordinate in virtual desktop space.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Connected OS monitor descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    /// OS monitor identifier.
    pub id: String,
    /// Human-readable display name (e.g., `"Built-in Display"`, `"Projector HDMI-1"`).
    pub name: String,
    /// Virtual desktop bounds.
    pub bounds: DisplayRect,
    /// Whether the OS marks this display as the primary monitor.
    pub is_primary: bool,
}

/// Role assigned to a presentation window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowRole {
    /// Audience-facing window showing only the slide (no notes, no timer).
    Audience,
    /// Presenter-facing window showing current slide, next slide, notes, timer, and clock.
    Presenter,
}

/// Placement assignment for the audience and optional presenter windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorAssignment {
    /// Monitor hosting the audience window.
    pub audience_display_id: String,
    /// Monitor hosting the presenter window (`None` when only one display is connected).
    pub presenter_display_id: Option<String>,
}

/// Speaker notes indexed by zero-based slide page, sourced from PDF annotations or a notes file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresenterNotes {
    notes_by_page: BTreeMap<usize, String>,
}

impl PresenterNotes {
    /// Creates an empty speaker notes collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets speaker notes for `page`.
    pub fn set_note(&mut self, page: usize, note: impl Into<String>) {
        self.notes_by_page.insert(page, note.into());
    }

    /// Parses a simple Markdown/text notes file split by `---` slide dividers into per-page notes.
    #[must_use]
    pub fn from_notes_text(text: &str) -> Self {
        let mut notes = Self::new();
        for (idx, section) in text.split("\n---\n").enumerate() {
            let trimmed = section.trim();
            if !trimmed.is_empty() {
                notes.set_note(idx, trimmed);
            }
        }
        notes
    }

    /// Returns the speaker notes for `page`, if any.
    #[must_use]
    pub fn note_for(&self, page: usize) -> Option<&str> {
        self.notes_by_page.get(&page).map(String::as_str)
    }
}

/// Elapsed presentation timer with pause/resume/reset (G.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenterTimer {
    accumulated_ms: u64,
    running_since_ms: Option<u64>,
}

impl PresenterTimer {
    /// Starts or resumes the timer at `now_ms`.
    pub fn start(&mut self, now_ms: u64) {
        if self.running_since_ms.is_none() {
            self.running_since_ms = Some(now_ms);
        }
    }

    /// Pauses the timer at `now_ms`.
    pub fn pause(&mut self, now_ms: u64) {
        if let Some(started) = self.running_since_ms.take() {
            self.accumulated_ms = self
                .accumulated_ms
                .saturating_add(now_ms.saturating_sub(started));
        }
    }

    /// Resets the elapsed timer to zero (and stops it).
    pub fn reset(&mut self) {
        self.accumulated_ms = 0;
        self.running_since_ms = None;
    }

    /// Returns total elapsed time as of `now_ms`.
    #[must_use]
    pub fn elapsed(&self, now_ms: u64) -> Duration {
        let live = self
            .running_since_ms
            .map_or(0, |started| now_ms.saturating_sub(started));
        Duration::from_millis(self.accumulated_ms.saturating_add(live))
    }

    /// Formats elapsed time as `MM:SS` or `HH:MM:SS`.
    #[must_use]
    pub fn format_elapsed(&self, now_ms: u64) -> String {
        let total_secs = self.elapsed(now_ms).as_secs();
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        if hours > 0 {
            format!("{hours:02}:{mins:02}:{secs:02}")
        } else {
            format!("{mins:02}:{secs:02}")
        }
    }
}

/// Synchronized state snapshot consumed by the presenter window UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenterSnapshot {
    /// Zero-based current slide index shown to the audience.
    pub current_page: usize,
    /// Zero-based next slide index preview (`None` on the final slide).
    pub next_page: Option<usize>,
    /// Total slides.
    pub page_count: usize,
    /// Current audience screen blank state.
    pub audience_blank: ScreenBlank,
    /// Speaker notes for the current slide.
    pub current_notes: String,
    /// Formatted elapsed presentation timer (`MM:SS` / `HH:MM:SS`).
    pub elapsed_text: String,
    /// Formatted current wall clock (`HH:MM`).
    pub clock_text: String,
    /// Whether the second presenter window is currently active on a separate monitor.
    pub dual_display_active: bool,
}

/// Synchronized audience + presenter view coordinator across multiple monitors (plan G.8).
#[derive(Debug, Clone)]
pub struct PresenterViewSync {
    session: PresentSession,
    notes: PresenterNotes,
    timer: PresenterTimer,
    displays: Vec<DisplayInfo>,
    swapped_displays: bool,
    assignment: Option<MonitorAssignment>,
}

impl PresenterViewSync {
    /// Starts presenter view synchronization over `displays`, automatically placing the audience
    /// window on the secondary display (projector) and presenter view on the primary display when
    /// two or more monitors exist, or falling back gracefully to single-monitor audience mode.
    #[must_use]
    pub fn start(
        page_count: usize,
        start_page: usize,
        notes: PresenterNotes,
        displays: Vec<DisplayInfo>,
        now_ms: u64,
    ) -> Self {
        let mut timer = PresenterTimer::default();
        timer.start(now_ms);
        let mut sync = Self {
            session: PresentSession::start(page_count, start_page),
            notes,
            timer,
            displays,
            swapped_displays: false,
            assignment: None,
        };
        sync.recompute_assignment();
        sync
    }

    /// Current monitor assignment (`None` only if the OS reported zero displays).
    #[must_use]
    pub fn assignment(&self) -> Option<&MonitorAssignment> {
        self.assignment.as_ref()
    }

    /// Whether dual-display presenter mode is currently active.
    #[must_use]
    pub fn is_dual_display(&self) -> bool {
        self.assignment
            .as_ref()
            .is_some_and(|a| a.presenter_display_id.is_some())
    }

    /// Swaps which monitor holds the audience window vs. the presenter window.
    pub fn swap_displays(&mut self) {
        self.swapped_displays = !self.swapped_displays;
        self.recompute_assignment();
    }

    /// Handles hot-plug or unplug display topology changes without dropping presentation state,
    /// current slide, notes, or elapsed timer (plan G.8 Done when).
    pub fn on_displays_changed(&mut self, displays: Vec<DisplayInfo>) {
        self.displays = displays;
        self.recompute_assignment();
    }

    /// Routes a keyboard or USB clicker key to keep both audience and presenter views in lockstep.
    pub fn handle_key(&mut self, key: ClickerKey) -> PresentAction {
        self.session.handle_key(key)
    }

    /// Jumps both audience and presenter windows to `page`.
    pub fn go_to(&mut self, page: usize) -> PresentAction {
        self.session.go_to(page)
    }

    /// Mutable access to the elapsed presentation timer.
    pub fn timer_mut(&mut self) -> &mut PresenterTimer {
        &mut self.timer
    }

    /// Current underlying presentation session.
    #[must_use]
    pub fn session(&self) -> &PresentSession {
        &self.session
    }

    /// Builds a synchronized snapshot for the presenter view window at `now_ms` and `clock_hh_mm`.
    #[must_use]
    pub fn presenter_snapshot(&self, now_ms: u64, clock_hh_mm: &str) -> PresenterSnapshot {
        let cur = self.session.current_page();
        PresenterSnapshot {
            current_page: cur,
            next_page: self.session.next_page(),
            page_count: self.session.page_count(),
            audience_blank: self.session.blank(),
            current_notes: self.notes.note_for(cur).unwrap_or("").to_owned(),
            elapsed_text: self.timer.format_elapsed(now_ms),
            clock_text: clock_hh_mm.to_owned(),
            dual_display_active: self.is_dual_display(),
        }
    }

    fn recompute_assignment(&mut self) {
        if self.displays.is_empty() {
            self.assignment = None;
            return;
        }
        if self.displays.len() == 1 {
            self.assignment = Some(MonitorAssignment {
                audience_display_id: self.displays[0].id.clone(),
                presenter_display_id: None,
            });
            return;
        }
        // Pick primary for presenter, first non-primary for audience (reversed when swapped).
        let primary_idx = self.displays.iter().position(|d| d.is_primary).unwrap_or(0);
        let secondary_idx = (0..self.displays.len())
            .find(|&i| i != primary_idx)
            .unwrap_or(0);

        let (pres_idx, aud_idx) = if self.swapped_displays {
            (secondary_idx, primary_idx)
        } else {
            (primary_idx, secondary_idx)
        };
        self.assignment = Some(MonitorAssignment {
            audience_display_id: self.displays[aud_idx].id.clone(),
            presenter_display_id: Some(self.displays[pres_idx].id.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop_and_projector() -> Vec<DisplayInfo> {
        vec![
            DisplayInfo {
                id: "laptop".into(),
                name: "Internal Display".into(),
                bounds: DisplayRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                is_primary: true,
            },
            DisplayInfo {
                id: "projector".into(),
                name: "HDMI Projector".into(),
                bounds: DisplayRect {
                    x: 1920,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                is_primary: false,
            },
        ]
    }

    #[test]
    fn presenter_view_syncs_two_monitors_and_survives_display_unplug() {
        let notes = PresenterNotes::from_notes_text(
            "Welcome everyone!\n---\nExplain the key equation here.\n---\nWrap up and Q&A.",
        );
        let mut sync = PresenterViewSync::start(3, 0, notes, laptop_and_projector(), 10_000);

        assert!(sync.is_dual_display());
        let assigned = sync.assignment().expect("assigned");
        assert_eq!(assigned.presenter_display_id.as_deref(), Some("laptop"));
        assert_eq!(assigned.audience_display_id, "projector");

        // Swap monitors works cleanly.
        sync.swap_displays();
        let swapped = sync.assignment().expect("swapped");
        assert_eq!(swapped.audience_display_id, "laptop");
        assert_eq!(swapped.presenter_display_id.as_deref(), Some("projector"));
        sync.swap_displays();

        // Advance slide; verify presenter snapshot stays in sync.
        assert_eq!(
            sync.handle_key(ClickerKey::Next),
            PresentAction::PageChanged(1)
        );
        let snap = sync.presenter_snapshot(75_000, "14:05");
        assert_eq!(snap.current_page, 1);
        assert_eq!(snap.next_page, Some(2));
        assert_eq!(snap.current_notes, "Explain the key equation here.");
        assert_eq!(snap.elapsed_text, "01:05");
        assert_eq!(snap.clock_text, "14:05");
        assert!(snap.dual_display_active);

        // Unplug projector mid-presentation! Session survives without losing slide, timer, or notes.
        let laptop_only = vec![laptop_and_projector().remove(0)];
        sync.on_displays_changed(laptop_only);
        assert!(!sync.is_dual_display());
        assert_eq!(
            sync.assignment().expect("single").audience_display_id,
            "laptop"
        );
        assert_eq!(sync.session().current_page(), 1);
        let snap_after_unplug = sync.presenter_snapshot(135_000, "14:06");
        assert_eq!(snap_after_unplug.elapsed_text, "02:05");
        assert_eq!(
            snap_after_unplug.current_notes,
            "Explain the key equation here."
        );
        assert!(!snap_after_unplug.dual_display_active);

        // Plug projector back in: dual display restores automatically!
        sync.on_displays_changed(laptop_and_projector());
        assert!(sync.is_dual_display());
    }
}
