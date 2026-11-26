//! Tablet two-pane split controller and stylus/hover routing (plan V.14).
//!
//! Determines split eligibility from screen dimensions in device-independent points (`dp` / UIKit `pt`),
//! keeps independent viewport state per pane, routes stylus and hover samples exclusively to the pane
//! under the pointer without moving sibling pane state, and supports V.9 open-link-in-other-pane.

use excellent_canvas::Rect;

use crate::pencil::{PencilHoverSample, PencilProcessor, PencilSample, SampleAcceptance};

/// Minimum shortest-side dimension in `dp`/`pt` required to qualify as a tablet (Android `sw600dp`).
pub const MIN_TABLET_SHORTEST_SIDE_DP: f32 = 600.0;

/// Minimum landscape width in `dp`/`pt` required to display two side-by-side panes cleanly.
pub const MIN_TWO_PANE_WIDTH_DP: f32 = 840.0;

/// Default horizontal split ratio (50% / 50%).
pub const DEFAULT_SPLIT_RATIO: f32 = 0.5;

/// Minimum and maximum split ratio bounds so neither pane collapses.
const MIN_SPLIT_RATIO: f32 = 0.25;
const MAX_SPLIT_RATIO: f32 = 0.75;

/// Whether the current device screen geometry supports two side-by-side panes (plan V.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SplitEligibility {
    /// Landscape tablet wide enough for two independent side-by-side panes.
    TwoPane,
    /// Tablet held in portrait (or narrow split window); stays single-pane until rotated/widened.
    SinglePanePortrait,
    /// Phone form factor (`shortest_side_dp < 600`); always remains single-pane even in landscape.
    SinglePanePhone,
}

impl SplitEligibility {
    /// Evaluates split eligibility from screen width and height in `dp` (Android) or `pt` (iPadOS).
    #[must_use]
    pub fn from_dimensions_dp(width_dp: f32, height_dp: f32) -> Self {
        if !width_dp.is_finite() || !height_dp.is_finite() || width_dp <= 0.0 || height_dp <= 0.0 {
            return Self::SinglePanePhone;
        }
        let shortest = width_dp.min(height_dp);
        if shortest < MIN_TABLET_SHORTEST_SIDE_DP {
            return Self::SinglePanePhone;
        }
        if width_dp > height_dp && width_dp >= MIN_TWO_PANE_WIDTH_DP {
            Self::TwoPane
        } else {
            Self::SinglePanePortrait
        }
    }

    /// Returns whether two side-by-side panes are eligible.
    #[must_use]
    pub const fn allows_two_panes(self) -> bool {
        matches!(self, Self::TwoPane)
    }
}

/// Identifies one pane in the tablet two-pane layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PaneSlot {
    /// Left/primary pane (always present).
    #[default]
    Primary,
    /// Right/secondary pane (present only when two-pane mode is active).
    Secondary,
}

impl PaneSlot {
    /// Returns the sibling pane slot.
    #[must_use]
    pub const fn sibling(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }
}

/// Screen dimensions in `dp`/`pt` and pixel density used for tablet split breakpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitScreenMetrics {
    /// Viewport width in `dp`/`pt`.
    pub width_dp: f32,
    /// Viewport height in `dp`/`pt`.
    pub height_dp: f32,
    /// Divider width in `dp`/`pt`.
    pub divider_dp: f32,
}

impl SplitScreenMetrics {
    /// Computes [`SplitEligibility`] for these metrics.
    #[must_use]
    pub fn eligibility(&self) -> SplitEligibility {
        SplitEligibility::from_dimensions_dp(self.width_dp, self.height_dp)
    }
}

/// Independent viewport and stylus state belonging to one pane.
#[derive(Debug)]
pub struct PaneViewportState {
    /// Current zero-based page shown in this pane.
    pub page: usize,
    /// Horizontal scroll offset in points.
    pub scroll_x: f32,
    /// Vertical scroll offset in points.
    pub scroll_y: f32,
    /// Current zoom factor (`1.0` = 100%).
    pub zoom: f32,
    /// Independent pencil processor for this pane.
    pub pencil: PencilProcessor,
}

impl Default for PaneViewportState {
    fn default() -> Self {
        Self {
            page: 0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom: 1.0,
            pencil: PencilProcessor::default(),
        }
    }
}

/// Destination opened in the sibling pane via V.9 (`open_link_in_other_pane`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitLinkTarget {
    /// Target page index in the sibling pane.
    pub page: usize,
    /// Vertical scroll offset on the target page.
    pub scroll_y: f32,
}

/// Result of routing a stylus or hover event through [`TabletSplitController`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StylusRouteOutcome {
    /// Pane that received the event.
    pub pane: PaneSlot,
    /// Acceptance result from that pane's stylus processor.
    pub acceptance: SampleAcceptance,
}

/// Controls two-pane layout eligibility, divider bounds, V.9 link opening, and isolated stylus routing.
#[derive(Debug)]
pub struct TabletSplitController {
    metrics: SplitScreenMetrics,
    split_ratio: f32,
    split_open: bool,
    primary: PaneViewportState,
    secondary: PaneViewportState,
    captured_pointer: Option<(u32, PaneSlot)>,
    active_pane: PaneSlot,
}

impl TabletSplitController {
    /// Creates a new split controller for `metrics`.
    #[must_use]
    pub fn new(metrics: SplitScreenMetrics) -> Self {
        let split_open = metrics.eligibility().allows_two_panes();
        Self {
            metrics,
            split_ratio: DEFAULT_SPLIT_RATIO,
            split_open,
            primary: PaneViewportState::default(),
            secondary: PaneViewportState::default(),
            captured_pointer: None,
            active_pane: PaneSlot::Primary,
        }
    }

    /// Updates screen dimensions and recalculates split eligibility.
    pub fn update_metrics(&mut self, metrics: SplitScreenMetrics) -> SplitEligibility {
        self.metrics = metrics;
        let eligibility = metrics.eligibility();
        if !eligibility.allows_two_panes() {
            self.split_open = false;
            self.active_pane = PaneSlot::Primary;
        }
        eligibility
    }

    /// Requests opening two panes if the current screen geometry is [`SplitEligibility::TwoPane`].
    pub fn request_two_panes(&mut self) -> SplitEligibility {
        let eligibility = self.metrics.eligibility();
        self.split_open = eligibility.allows_two_panes();
        eligibility
    }

    /// Collapses back to single-pane mode.
    pub fn collapse_to_single_pane(&mut self) {
        self.split_open = false;
        self.active_pane = PaneSlot::Primary;
    }

    /// Returns current split eligibility.
    #[must_use]
    pub fn eligibility(&self) -> SplitEligibility {
        self.metrics.eligibility()
    }

    /// Returns whether two panes are currently visible.
    #[must_use]
    pub const fn is_two_pane_active(&self) -> bool {
        self.split_open
    }

    /// Sets the horizontal divider ratio (clamped to `0.25..=0.75`).
    pub fn set_split_ratio(&mut self, ratio: f32) {
        if ratio.is_finite() {
            self.split_ratio = ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        }
    }

    /// Returns the active pane slot.
    #[must_use]
    pub const fn active_pane(&self) -> PaneSlot {
        self.active_pane
    }

    /// Returns immutable access to `slot`'s state.
    #[must_use]
    pub const fn pane(&self, slot: PaneSlot) -> &PaneViewportState {
        match slot {
            PaneSlot::Primary => &self.primary,
            PaneSlot::Secondary => &self.secondary,
        }
    }

    /// Returns mutable access to `slot`'s state.
    pub fn pane_mut(&mut self, slot: PaneSlot) -> &mut PaneViewportState {
        match slot {
            PaneSlot::Primary => &mut self.primary,
            PaneSlot::Secondary => &mut self.secondary,
        }
    }

    /// Computes the bounding [`Rect`] in `dp`/`pt` for `slot`, or `None` if `Secondary` is closed.
    #[must_use]
    pub fn pane_bounds(&self, slot: PaneSlot) -> Option<Rect> {
        if !self.split_open {
            return match slot {
                PaneSlot::Primary => Some(Rect::new(
                    0.0,
                    0.0,
                    self.metrics.width_dp,
                    self.metrics.height_dp,
                )),
                PaneSlot::Secondary => None,
            };
        }
        let divider = self.metrics.divider_dp.max(0.0);
        let usable = (self.metrics.width_dp - divider).max(0.0);
        let left_w = usable * self.split_ratio;
        let right_x = left_w + divider;
        let right_w = (self.metrics.width_dp - right_x).max(0.0);
        match slot {
            PaneSlot::Primary => Some(Rect::new(0.0, 0.0, left_w, self.metrics.height_dp)),
            PaneSlot::Secondary => Some(Rect::new(right_x, 0.0, right_w, self.metrics.height_dp)),
        }
    }

    /// Determines which [`PaneSlot`] lies under `(x_dp, y_dp)`.
    #[must_use]
    pub fn pane_at(&self, x_dp: f32, y_dp: f32) -> PaneSlot {
        if !self.split_open {
            return PaneSlot::Primary;
        }
        if let Some(secondary) = self.pane_bounds(PaneSlot::Secondary)
            && x_dp >= secondary.x0
            && y_dp >= secondary.y0
            && y_dp <= secondary.y1
        {
            return PaneSlot::Secondary;
        }
        PaneSlot::Primary
    }

    /// Routes a stylus touch-down to the pane under the pointer without modifying the sibling pane.
    pub fn route_touch_down(&mut self, mut sample: PencilSample) -> StylusRouteOutcome {
        let slot = self.pane_at(sample.x, sample.y);
        let bounds = self
            .pane_bounds(slot)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, self.metrics.width_dp, self.metrics.height_dp));
        sample.x -= bounds.x0;
        sample.y -= bounds.y0;
        self.active_pane = slot;
        let acceptance = self.pane_mut(slot).pencil.touch_down(sample);
        if matches!(acceptance, SampleAcceptance::CommittedInk) {
            self.captured_pointer = Some((sample.pointer_id, slot));
        }
        StylusRouteOutcome {
            pane: slot,
            acceptance,
        }
    }

    /// Routes stylus move samples to the captured pane (or pane under the first sample) without touching the sibling.
    pub fn route_touch_moved(&mut self, samples: &[PencilSample]) -> Option<StylusRouteOutcome> {
        let first = samples.first()?;
        let slot = self
            .captured_pointer
            .and_then(|(id, captured_slot)| (id == first.pointer_id).then_some(captured_slot))
            .unwrap_or_else(|| self.pane_at(first.x, first.y));
        let bounds = self
            .pane_bounds(slot)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, self.metrics.width_dp, self.metrics.height_dp));
        let local: Vec<PencilSample> = samples
            .iter()
            .map(|&s| PencilSample {
                x: s.x - bounds.x0,
                y: s.y - bounds.y0,
                ..s
            })
            .collect();
        let acceptance = self.pane_mut(slot).pencil.touch_moved(&local);
        Some(StylusRouteOutcome {
            pane: slot,
            acceptance,
        })
    }

    /// Completes a routed stylus stroke on its owning pane without altering the sibling pane.
    pub fn route_touch_up(
        &mut self,
        pointer_id: u32,
        timestamp_ms: f64,
    ) -> Option<(PaneSlot, Vec<excellent_ink::stroke::InkPoint>)> {
        let slot = self
            .captured_pointer
            .take()
            .map_or(self.active_pane, |(_, s)| s);
        let stroke = self
            .pane_mut(slot)
            .pencil
            .touch_up(pointer_id, timestamp_ms)?;
        Some((slot, stroke))
    }

    /// Routes a stylus hover event strictly to the pane under `(hover.x, hover.y)` without moving sibling state.
    pub fn route_hover(&mut self, mut hover: PencilHoverSample) -> PaneSlot {
        let slot = self.pane_at(hover.x, hover.y);
        let bounds = self
            .pane_bounds(slot)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, self.metrics.width_dp, self.metrics.height_dp));
        hover.x -= bounds.x0;
        hover.y -= bounds.y0;
        self.pane_mut(slot).pencil.process_hover(hover);
        slot
    }

    /// Opens a link target in the sibling pane (plan V.9 / V.14) without moving `source_pane`'s viewport.
    ///
    /// Returns `None` when the device is in single-pane mode.
    pub fn open_link_in_other_pane(
        &mut self,
        source_pane: PaneSlot,
        target: SplitLinkTarget,
    ) -> Option<PaneSlot> {
        if !self.split_open && !self.request_two_panes().allows_two_panes() {
            return None;
        }
        let other = source_pane.sibling();
        let pane = self.pane_mut(other);
        pane.page = target.page;
        pane.scroll_y = target.scroll_y;
        Some(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pencil::{HoverPhase, TouchKind, TouchSampleOrigin};

    #[test]
    fn breakpoint_distinguishes_landscape_tablet_portrait_tablet_and_phone() {
        assert_eq!(
            SplitEligibility::from_dimensions_dp(1194.0, 834.0),
            SplitEligibility::TwoPane
        );
        assert_eq!(
            SplitEligibility::from_dimensions_dp(834.0, 1194.0),
            SplitEligibility::SinglePanePortrait
        );
        assert_eq!(
            SplitEligibility::from_dimensions_dp(844.0, 390.0),
            SplitEligibility::SinglePanePhone
        );
    }

    #[test]
    fn stylus_and_hover_route_to_pointer_pane_without_touching_sibling() {
        let mut ctrl = TabletSplitController::new(SplitScreenMetrics {
            width_dp: 1200.0,
            height_dp: 800.0,
            divider_dp: 8.0,
        });
        assert!(ctrl.is_two_pane_active());
        ctrl.pane_mut(PaneSlot::Primary).page = 2;
        ctrl.pane_mut(PaneSlot::Primary).scroll_y = 140.0;
        ctrl.pane_mut(PaneSlot::Secondary).page = 9;
        ctrl.pane_mut(PaneSlot::Secondary).scroll_y = 420.0;

        // Stylus touches right pane (x = 900 > 604).
        let sample = PencilSample {
            pointer_id: 1,
            x: 900.0,
            y: 300.0,
            force: 2.0,
            maximum_possible_force: 4.0,
            altitude_rad: 1.1,
            azimuth_rad: 0.4,
            timestamp_ms: 10.0,
            touch_kind: TouchKind::Pencil,
            origin: TouchSampleOrigin::Actual,
            major_radius_mm: None,
        };
        let routed = ctrl.route_touch_down(sample);
        assert_eq!(routed.pane, PaneSlot::Secondary);
        assert_eq!(routed.acceptance, SampleAcceptance::CommittedInk);

        // Primary pane state and pencil remained completely untouched.
        assert_eq!(ctrl.pane(PaneSlot::Primary).page, 2);
        assert_eq!(ctrl.pane(PaneSlot::Primary).scroll_y, 140.0);
        assert!(
            ctrl.pane(PaneSlot::Primary)
                .pencil
                .active_stroke()
                .is_none()
        );
        assert!(
            ctrl.pane(PaneSlot::Secondary)
                .pencil
                .active_stroke()
                .is_some()
        );

        // Hover over left pane does not alter right pane's active stroke or viewport.
        let hover_pane = ctrl.route_hover(PencilHoverSample {
            x: 100.0,
            y: 200.0,
            z_offset: 0.5,
            altitude_rad: 1.2,
            azimuth_rad: 0.0,
            timestamp_ms: 20.0,
            phase: HoverPhase::Moved,
        });
        assert_eq!(hover_pane, PaneSlot::Primary);
        assert_eq!(ctrl.pane(PaneSlot::Secondary).page, 9);
        assert_eq!(ctrl.pane(PaneSlot::Secondary).scroll_y, 420.0);

        // V.9 open link from Primary into Secondary leaves Primary untouched.
        let target_pane = ctrl.open_link_in_other_pane(
            PaneSlot::Primary,
            SplitLinkTarget {
                page: 15,
                scroll_y: 80.0,
            },
        );
        assert_eq!(target_pane, Some(PaneSlot::Secondary));
        assert_eq!(ctrl.pane(PaneSlot::Primary).page, 2);
        assert_eq!(ctrl.pane(PaneSlot::Secondary).page, 15);
    }
}
