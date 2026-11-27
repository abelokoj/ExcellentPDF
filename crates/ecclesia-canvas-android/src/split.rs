//! Tablet two-pane split controller and stylus/hover routing for Android (plan V.14).
//!
//! Evaluates `SplitEligibility` from screen dimensions in `dp` (`sw600dp` + `width_dp >= 840` landscape),
//! isolates each pane's viewport and [`MotionBatcher`], and routes stylus & hover events directly to the
//! pane under the pointer without moving sibling pane state.

use ecclesia_canvas::Rect;

use crate::motion::{AndroidMotionSample, MotionAcceptance, MotionBatcher};

/// Minimum shortest-side width in `dp` for tablet classification (`sw600dp`).
pub const MIN_TABLET_SHORTEST_SIDE_DP: f32 = 600.0;

/// Minimum landscape width in `dp` for Material 3 expanded two-pane layout.
pub const MIN_TWO_PANE_WIDTH_DP: f32 = 840.0;

/// Default 50/50 horizontal split ratio.
pub const DEFAULT_SPLIT_RATIO: f32 = 0.5;

const MIN_SPLIT_RATIO: f32 = 0.25;
const MAX_SPLIT_RATIO: f32 = 0.75;

/// Whether the Android device dimensions qualify for two side-by-side panes (plan V.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SplitEligibility {
    /// Landscape tablet wide enough for two side-by-side panes.
    TwoPane,
    /// Tablet in portrait or narrow split window; single pane.
    SinglePanePortrait,
    /// Phone form factor (`shortest_side_dp < 600`); always single pane.
    SinglePanePhone,
}

impl SplitEligibility {
    /// Evaluates [`SplitEligibility`] from viewport `width_dp` and `height_dp`.
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

    /// Returns whether two-pane split mode is allowed.
    #[must_use]
    pub const fn allows_two_panes(self) -> bool {
        matches!(self, Self::TwoPane)
    }
}

/// Identifies one pane in the Android tablet split view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PaneSlot {
    /// Left/primary pane.
    #[default]
    Primary,
    /// Right/secondary pane.
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

/// Screen dimensions in `dp` and display density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitScreenMetrics {
    /// Viewport width in `dp`.
    pub width_dp: f32,
    /// Viewport height in `dp`.
    pub height_dp: f32,
    /// Divider width in `dp`.
    pub divider_dp: f32,
    /// Display density (`px / dp`).
    pub density: f32,
}

impl SplitScreenMetrics {
    /// Evaluates split eligibility for these dimensions.
    #[must_use]
    pub fn eligibility(&self) -> SplitEligibility {
        SplitEligibility::from_dimensions_dp(self.width_dp, self.height_dp)
    }
}

/// Independent viewport and stylus state for one pane.
#[derive(Debug)]
pub struct PaneViewportState {
    /// Zero-based current page.
    pub page: usize,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// Current zoom factor.
    pub zoom: f32,
    /// Independent motion batcher for this pane.
    pub motion: MotionBatcher,
}

impl Default for PaneViewportState {
    fn default() -> Self {
        Self {
            page: 0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom: 1.0,
            motion: MotionBatcher::default(),
        }
    }
}

/// Link destination to open in the sibling pane (plan V.9 / V.14).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitLinkTarget {
    /// Destination page index.
    pub page: usize,
    /// Vertical scroll offset on that page.
    pub scroll_y: f32,
}

/// Outcome of routing an Android stylus or hover event batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StylusRouteOutcome {
    /// Pane that received the batch.
    pub pane: PaneSlot,
    /// Acceptance result from that pane's [`MotionBatcher`].
    pub acceptance: MotionAcceptance,
}

/// Coordinates Android tablet split layout, stylus capture, hover routing, and V.9 link opening.
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
        let dpi = (metrics.density.max(1.0)) * 160.0;
        let mut primary = PaneViewportState::default();
        let mut secondary = PaneViewportState::default();
        primary.motion = MotionBatcher::new(dpi, Default::default());
        secondary.motion = MotionBatcher::new(dpi, Default::default());
        Self {
            metrics,
            split_ratio: DEFAULT_SPLIT_RATIO,
            split_open,
            primary,
            secondary,
            captured_pointer: None,
            active_pane: PaneSlot::Primary,
        }
    }

    /// Updates screen dimensions and re-evaluates split eligibility.
    pub fn update_metrics(&mut self, metrics: SplitScreenMetrics) -> SplitEligibility {
        self.metrics = metrics;
        let eligibility = metrics.eligibility();
        if !eligibility.allows_two_panes() {
            self.split_open = false;
            self.active_pane = PaneSlot::Primary;
        }
        eligibility
    }

    /// Requests entering two-pane mode if eligible.
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

    /// Returns whether two panes are active.
    #[must_use]
    pub const fn is_two_pane_active(&self) -> bool {
        self.split_open
    }

    /// Updates the horizontal divider ratio (`0.25..=0.75`).
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

    /// Returns immutable access to `slot`.
    #[must_use]
    pub const fn pane(&self, slot: PaneSlot) -> &PaneViewportState {
        match slot {
            PaneSlot::Primary => &self.primary,
            PaneSlot::Secondary => &self.secondary,
        }
    }

    /// Returns mutable access to `slot`.
    pub fn pane_mut(&mut self, slot: PaneSlot) -> &mut PaneViewportState {
        match slot {
            PaneSlot::Primary => &mut self.primary,
            PaneSlot::Secondary => &mut self.secondary,
        }
    }

    /// Returns the bounds of `slot` in view pixels, or `None` when `Secondary` is closed.
    #[must_use]
    pub fn pane_bounds_px(&self, slot: PaneSlot) -> Option<Rect> {
        let density = self.metrics.density.max(1.0);
        let width_px = self.metrics.width_dp * density;
        let height_px = self.metrics.height_dp * density;
        if !self.split_open {
            return match slot {
                PaneSlot::Primary => Some(Rect::new(0.0, 0.0, width_px, height_px)),
                PaneSlot::Secondary => None,
            };
        }
        let divider_px = self.metrics.divider_dp.max(0.0) * density;
        let usable = (width_px - divider_px).max(0.0);
        let left_w = usable * self.split_ratio;
        let right_x = left_w + divider_px;
        let right_w = (width_px - right_x).max(0.0);
        match slot {
            PaneSlot::Primary => Some(Rect::new(0.0, 0.0, left_w, height_px)),
            PaneSlot::Secondary => Some(Rect::new(right_x, 0.0, right_w, height_px)),
        }
    }

    /// Returns the pane lying under `(x_px, y_px)` in container view pixels.
    #[must_use]
    pub fn pane_at_px(&self, x_px: f32, y_px: f32) -> PaneSlot {
        if !self.split_open {
            return PaneSlot::Primary;
        }
        if let Some(secondary) = self.pane_bounds_px(PaneSlot::Secondary)
            && x_px >= secondary.x0
            && y_px >= secondary.y0
            && y_px <= secondary.y1
        {
            return PaneSlot::Secondary;
        }
        PaneSlot::Primary
    }

    /// Routes an Android `MotionEvent` batch strictly to the pane under the pointer without moving sibling state.
    pub fn route_motion_batch(
        &mut self,
        samples: &[AndroidMotionSample],
    ) -> Option<StylusRouteOutcome> {
        let first = samples.first()?;
        let is_hover = matches!(
            first.action,
            crate::motion::MotionAction::HoverEnter
                | crate::motion::MotionAction::HoverMove
                | crate::motion::MotionAction::HoverExit
        );
        let slot = if is_hover {
            self.pane_at_px(first.x, first.y)
        } else if let Some((id, captured)) = self.captured_pointer {
            if id == first.pointer_id {
                captured
            } else {
                self.pane_at_px(first.x, first.y)
            }
        } else {
            self.pane_at_px(first.x, first.y)
        };

        if !is_hover {
            self.active_pane = slot;
        }

        let bounds = self.pane_bounds_px(slot).unwrap_or_else(|| {
            let d = self.metrics.density.max(1.0);
            Rect::new(
                0.0,
                0.0,
                self.metrics.width_dp * d,
                self.metrics.height_dp * d,
            )
        });
        let localized: Vec<AndroidMotionSample> = samples
            .iter()
            .map(|&s| AndroidMotionSample {
                x: s.x - bounds.x0,
                y: s.y - bounds.y0,
                ..s
            })
            .collect();

        let acceptance = self.pane_mut(slot).motion.process_batch(&localized);
        match first.action {
            crate::motion::MotionAction::Down
                if matches!(acceptance, MotionAcceptance::CommittedInk) =>
            {
                self.captured_pointer = Some((first.pointer_id, slot));
            }
            crate::motion::MotionAction::Up | crate::motion::MotionAction::Cancel
                if self
                    .captured_pointer
                    .is_some_and(|(id, _)| id == first.pointer_id) =>
            {
                self.captured_pointer = None;
            }
            _ => {}
        }

        Some(StylusRouteOutcome {
            pane: slot,
            acceptance,
        })
    }

    /// Opens a link target in the sibling pane without altering `source_pane`'s viewport.
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
    use crate::motion::{MotionAction, MotionToolType, SampleOrigin};

    #[test]
    fn android_tablet_routes_stylus_to_pane_under_pointer_without_moving_sibling() {
        let mut ctrl = TabletSplitController::new(SplitScreenMetrics {
            width_dp: 1280.0,
            height_dp: 800.0,
            divider_dp: 8.0,
            density: 2.0,
        });
        assert_eq!(ctrl.eligibility(), SplitEligibility::TwoPane);
        ctrl.pane_mut(PaneSlot::Primary).page = 4;
        ctrl.pane_mut(PaneSlot::Primary).scroll_y = 250.0;
        ctrl.pane_mut(PaneSlot::Secondary).page = 11;
        ctrl.pane_mut(PaneSlot::Secondary).scroll_y = 90.0;

        // Touch in secondary pane (x = 1800 px > 1288 px).
        let routed = ctrl
            .route_motion_batch(&[AndroidMotionSample {
                pointer_id: 5,
                x: 1800.0,
                y: 400.0,
                axis_pressure: 0.8,
                axis_tilt_rad: 0.4,
                axis_orientation_rad: 0.5,
                touch_major_px: 0.0,
                event_time_ms: 100.0,
                tool_type: MotionToolType::Stylus,
                action: MotionAction::Down,
                origin: SampleOrigin::Current,
            }])
            .expect("route");
        assert_eq!(routed.pane, PaneSlot::Secondary);
        assert_eq!(routed.acceptance, MotionAcceptance::CommittedInk);

        // Primary pane stayed untouched.
        assert_eq!(ctrl.pane(PaneSlot::Primary).page, 4);
        assert_eq!(ctrl.pane(PaneSlot::Primary).scroll_y, 250.0);
        assert!(
            ctrl.pane(PaneSlot::Primary)
                .motion
                .active_stroke()
                .is_none()
        );
        assert!(
            ctrl.pane(PaneSlot::Secondary)
                .motion
                .active_stroke()
                .is_some()
        );
    }
}
