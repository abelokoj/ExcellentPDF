//! Android `MotionEvent` batching, historical sample ingestion, `AXIS_TILT` / `AXIS_ORIENTATION`
//! mapping, palm rejection via `toolType`, and front-buffered motion prediction (plan K.12, §7.3, H.8).

use excellent_canvas::page::gestures::{Palms, Verdict};
use excellent_ink::pens::Pen;
use excellent_ink::predict::predict;
use excellent_ink::sample::{Sample, Tilt, Tool};
use excellent_ink::stroke::{InkPoint, Stroke};

/// Millimetres per inch used to convert `AXIS_TOUCH_MAJOR` pixels into millimetres for palm rejection.
const MM_PER_INCH: f32 = 25.4;

/// Android `MotionEvent` action classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MotionAction {
    /// `ACTION_DOWN` or `ACTION_POINTER_DOWN`.
    Down,
    /// `ACTION_MOVE`.
    Move,
    /// `ACTION_UP` or `ACTION_POINTER_UP`.
    Up,
    /// `ACTION_CANCEL`.
    Cancel,
    /// `ACTION_HOVER_ENTER`.
    HoverEnter,
    /// `ACTION_HOVER_MOVE`.
    HoverMove,
    /// `ACTION_HOVER_EXIT`.
    HoverExit,
}

/// Android `MotionEvent.getToolType(pointerIndex)` classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MotionToolType {
    /// `TOOL_TYPE_STYLUS`.
    #[default]
    Stylus,
    /// `TOOL_TYPE_ERASER`.
    Eraser,
    /// `TOOL_TYPE_FINGER`.
    Finger,
    /// `TOOL_TYPE_MOUSE`.
    Mouse,
    /// `TOOL_TYPE_UNKNOWN`.
    Unknown,
}

/// Whether a sample came from Android `MotionEvent` history, current pointer state, or `MotionEventPredictor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SampleOrigin {
    /// `MotionEvent.getHistorical*` sub-frame sample.
    Historical,
    /// Primary current event sample.
    #[default]
    Current,
    /// Predicted sample from `androidx.input.motionprediction.MotionEventPredictor`.
    Predicted,
}

/// Android `View.requestUnbufferedDispatch` configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DispatchLatencyPolicy {
    /// Request unbuffered input dispatch for stylus/hover streams so ink hits the front buffer immediately.
    #[default]
    UnbufferedStylus,
    /// Use standard Choreographer-aligned VSync batching.
    SystemBuffered,
}

/// Single pointer sample extracted from an Android `MotionEvent` (historical or current).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AndroidMotionSample {
    /// Stable pointer ID (`MotionEvent.getPointerId`).
    pub pointer_id: u32,
    /// X coordinate in view pixels.
    pub x: f32,
    /// Y coordinate in view pixels.
    pub y: f32,
    /// Normalized pressure from `AXIS_PRESSURE` (`0.0..=1.0`).
    pub axis_pressure: f32,
    /// Tilt from perpendicular in radians (`MotionEvent.AXIS_TILT`, `0` upright to `π/2` flat).
    pub axis_tilt_rad: f32,
    /// Azimuth orientation in radians clockwise from `-Y` (`MotionEvent.AXIS_ORIENTATION`, `[-π, π]`).
    pub axis_orientation_rad: f32,
    /// Contact major axis length in pixels (`MotionEvent.AXIS_TOUCH_MAJOR`).
    pub touch_major_px: f32,
    /// Monotonic event time in milliseconds (`MotionEvent.getEventTime` / `getHistoricalEventTime`).
    pub event_time_ms: f64,
    /// Hardware tool type (`MotionEvent.getToolType`).
    pub tool_type: MotionToolType,
    /// Action kind.
    pub action: MotionAction,
    /// Whether this sample is historical, current, or predicted.
    pub origin: SampleOrigin,
}

impl AndroidMotionSample {
    /// Converts Android `AXIS_TILT` (0 upright) and `AXIS_ORIENTATION` (0 toward `-Y`, clockwise)
    /// into [`Tilt`] (`x` right positive, `y` toward reader `+Y` positive).
    #[must_use]
    pub fn tilt(&self) -> Option<Tilt> {
        if !self.axis_tilt_rad.is_finite() || !self.axis_orientation_rad.is_finite() {
            return None;
        }
        let clamped_tilt = self.axis_tilt_rad.clamp(0.0, 1.55);
        let radius = clamped_tilt.tan();
        let tx = radius * self.axis_orientation_rad.sin();
        let ty = -radius * self.axis_orientation_rad.cos();
        Some(Tilt {
            x: tx.atan().to_degrees(),
            y: ty.atan().to_degrees(),
        })
    }

    /// Converts `touch_major_px` to millimetres given screen `dpi`, if positive and finite.
    #[must_use]
    pub fn touch_major_mm(&self, dpi: f32) -> Option<f32> {
        if !self.touch_major_px.is_finite()
            || self.touch_major_px <= 0.0
            || !dpi.is_finite()
            || dpi <= 0.0
        {
            return None;
        }
        Some(self.touch_major_px / dpi * MM_PER_INCH)
    }

    /// Converts this sample into an [`excellent_ink::sample::Sample`].
    #[must_use]
    pub fn to_ink_sample(&self) -> Option<Sample> {
        let tool = match self.tool_type {
            MotionToolType::Stylus => Tool::Pen,
            MotionToolType::Eraser => Tool::Eraser,
            MotionToolType::Finger => Tool::Touch,
            MotionToolType::Mouse => Tool::Mouse,
            MotionToolType::Unknown => return None,
        };
        let pressure = self
            .axis_pressure
            .is_finite()
            .then(|| self.axis_pressure.clamp(0.0, 1.0));
        Some(Sample {
            x: self.x,
            y: self.y,
            pressure,
            tilt: self.tilt(),
            time: self.event_time_ms,
            tool,
        })
    }
}

/// Acceptance verdict for a motion event batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MotionAcceptance {
    /// Sample committed to the active ink stroke.
    CommittedInk,
    /// Sample rendered only on the front-buffered prediction overlay.
    FrontBufferPreview,
    /// Finger touch passed to Android pan/zoom gesture detectors.
    PassToNavigation,
    /// Rejected as a palm or unknown pointer.
    RejectedPalm,
}

/// Active stylus stroke with committed points and front-buffered predicted preview points.
#[derive(Debug, Clone)]
pub struct ActiveAndroidStroke {
    /// Pointer ID owning this stroke.
    pub pointer_id: u32,
    /// Tool type that initiated the stroke (`Stylus` or `Eraser`).
    pub tool_type: MotionToolType,
    /// Committed ink stroke built from historical and current samples.
    pub stroke: Stroke,
    /// Active pen preset.
    pub pen: Pen,
    /// Transient predicted points for `GLFrontBufferedRenderer` / `CanvasFrontBufferedRenderer`.
    pub front_buffer_preview: Vec<InkPoint>,
}

/// Batches historical & current `MotionEvent` samples, enforces palm rejection via `toolType`,
/// and manages front-buffered motion prediction.
#[derive(Debug)]
pub struct MotionBatcher {
    dpi: f32,
    pen: Pen,
    latency_policy: DispatchLatencyPolicy,
    palms: Palms,
    active_stroke: Option<ActiveAndroidStroke>,
    hover_point: Option<AndroidMotionSample>,
    committed_strokes: Vec<Vec<InkPoint>>,
}

impl Default for MotionBatcher {
    fn default() -> Self {
        Self::new(320.0, Pen::default())
    }
}

impl MotionBatcher {
    /// Creates a new batcher for a display at `dpi` using `pen`.
    #[must_use]
    pub fn new(dpi: f32, pen: Pen) -> Self {
        Self {
            dpi: dpi.max(72.0),
            pen,
            latency_policy: DispatchLatencyPolicy::UnbufferedStylus,
            palms: Palms::default(),
            active_stroke: None,
            hover_point: None,
            committed_strokes: Vec::new(),
        }
    }

    /// Returns whether `View.requestUnbufferedDispatch` should be enabled for stylus input.
    #[must_use]
    pub const fn latency_policy(&self) -> DispatchLatencyPolicy {
        self.latency_policy
    }

    /// Updates the dispatch latency policy.
    pub fn set_latency_policy(&mut self, policy: DispatchLatencyPolicy) {
        self.latency_policy = policy;
    }

    /// Returns the currently active stroke, if any.
    #[must_use]
    pub const fn active_stroke(&self) -> Option<&ActiveAndroidStroke> {
        self.active_stroke.as_ref()
    }

    /// Returns the current stylus hover sample, if hovering.
    #[must_use]
    pub const fn hover_point(&self) -> Option<AndroidMotionSample> {
        self.hover_point
    }

    /// Returns all finished strokes committed in this batcher.
    #[must_use]
    pub fn committed_strokes(&self) -> &[Vec<InkPoint>] {
        &self.committed_strokes
    }

    /// Processes a chronologically ordered batch of `MotionEvent` samples (historical first, then current).
    pub fn process_batch(&mut self, samples: &[AndroidMotionSample]) -> MotionAcceptance {
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.event_time_ms.total_cmp(&b.event_time_ms));

        let mut outcome = MotionAcceptance::RejectedPalm;
        let mut saw_real = false;

        for sample in sorted {
            match sample.action {
                MotionAction::HoverEnter | MotionAction::HoverMove => {
                    if matches!(
                        sample.tool_type,
                        MotionToolType::Stylus | MotionToolType::Eraser
                    ) {
                        self.palms.pen_near(sample.event_time_ms);
                        self.hover_point = Some(sample);
                        outcome = MotionAcceptance::FrontBufferPreview;
                    }
                }
                MotionAction::HoverExit => {
                    if matches!(
                        sample.tool_type,
                        MotionToolType::Stylus | MotionToolType::Eraser
                    ) {
                        self.palms.pen_gone(sample.event_time_ms);
                        self.hover_point = None;
                        outcome = MotionAcceptance::FrontBufferPreview;
                    }
                }
                MotionAction::Down => {
                    outcome = self.handle_down(sample);
                }
                MotionAction::Move => {
                    let step = self.handle_move(sample, &mut saw_real);
                    if !matches!(outcome, MotionAcceptance::CommittedInk) {
                        outcome = step;
                    }
                }
                MotionAction::Up => {
                    let _ = self.handle_up(sample.pointer_id, sample.event_time_ms);
                    outcome = MotionAcceptance::CommittedInk;
                }
                MotionAction::Cancel => {
                    self.palms.touch_up(sample.pointer_id);
                    if self
                        .active_stroke
                        .as_ref()
                        .is_some_and(|s| s.pointer_id == sample.pointer_id)
                    {
                        self.active_stroke = None;
                    }
                    outcome = MotionAcceptance::RejectedPalm;
                }
            }
        }

        if let Some(active) = self.active_stroke.as_mut()
            && active.front_buffer_preview.is_empty()
            && active.stroke.samples().len() >= 2
        {
            let start = active.stroke.samples().len().saturating_sub(6);
            let predicted = predict(&active.stroke.samples()[start..], 16.0, 24.0);
            let width = active
                .stroke
                .points()
                .last()
                .map_or(1.0, |p| p.width)
                .max(0.5);
            active
                .front_buffer_preview
                .extend(predicted.into_iter().map(|s| InkPoint {
                    x: s.0,
                    y: s.1,
                    width,
                }));
        }

        outcome
    }

    fn handle_down(&mut self, sample: AndroidMotionSample) -> MotionAcceptance {
        match sample.tool_type {
            MotionToolType::Stylus | MotionToolType::Eraser => {
                self.palms.pen_near(sample.event_time_ms);
                if matches!(sample.origin, SampleOrigin::Predicted) {
                    return MotionAcceptance::FrontBufferPreview;
                }
                let Some(ink_sample) = sample.to_ink_sample() else {
                    return MotionAcceptance::RejectedPalm;
                };
                let mut stroke = Stroke::new();
                self.pen.push(&mut stroke, ink_sample);
                self.active_stroke = Some(ActiveAndroidStroke {
                    pointer_id: sample.pointer_id,
                    tool_type: sample.tool_type,
                    stroke,
                    pen: self.pen,
                    front_buffer_preview: Vec::new(),
                });
                MotionAcceptance::CommittedInk
            }
            MotionToolType::Finger => {
                let width_mm = sample.touch_major_mm(self.dpi);
                match self
                    .palms
                    .touch_down(sample.pointer_id, sample.event_time_ms, width_mm)
                {
                    Verdict::Ignore => MotionAcceptance::RejectedPalm,
                    Verdict::Pass => MotionAcceptance::PassToNavigation,
                    Verdict::Draw => MotionAcceptance::CommittedInk,
                }
            }
            MotionToolType::Mouse => MotionAcceptance::PassToNavigation,
            MotionToolType::Unknown => MotionAcceptance::RejectedPalm,
        }
    }

    fn handle_move(
        &mut self,
        sample: AndroidMotionSample,
        saw_real: &mut bool,
    ) -> MotionAcceptance {
        match sample.tool_type {
            MotionToolType::Stylus | MotionToolType::Eraser => {
                self.palms.pen_near(sample.event_time_ms);
                let Some(active) = self.active_stroke.as_mut() else {
                    return MotionAcceptance::RejectedPalm;
                };
                if active.pointer_id != sample.pointer_id {
                    return MotionAcceptance::RejectedPalm;
                }
                let Some(ink_sample) = sample.to_ink_sample() else {
                    return MotionAcceptance::RejectedPalm;
                };
                match sample.origin {
                    SampleOrigin::Historical | SampleOrigin::Current => {
                        if !*saw_real {
                            active.front_buffer_preview.clear();
                            *saw_real = true;
                        }
                        active.pen.push(&mut active.stroke, ink_sample);
                        MotionAcceptance::CommittedInk
                    }
                    SampleOrigin::Predicted => {
                        let width = active
                            .stroke
                            .points()
                            .last()
                            .map_or(1.0, |p| p.width)
                            .max(0.5);
                        active.front_buffer_preview.push(InkPoint {
                            x: ink_sample.x,
                            y: ink_sample.y,
                            width,
                        });
                        MotionAcceptance::FrontBufferPreview
                    }
                }
            }
            MotionToolType::Finger => match self.palms.verdict(sample.pointer_id) {
                Verdict::Ignore => MotionAcceptance::RejectedPalm,
                Verdict::Pass => MotionAcceptance::PassToNavigation,
                Verdict::Draw => MotionAcceptance::CommittedInk,
            },
            MotionToolType::Mouse => MotionAcceptance::PassToNavigation,
            MotionToolType::Unknown => MotionAcceptance::RejectedPalm,
        }
    }

    /// Completes the active stroke for `pointer_id` and returns the smoothed ink points.
    pub fn handle_up(&mut self, pointer_id: u32, event_time_ms: f64) -> Option<Vec<InkPoint>> {
        self.palms.touch_up(pointer_id);
        let active = self.active_stroke.as_ref()?;
        if active.pointer_id != pointer_id {
            return None;
        }
        let active = self.active_stroke.take()?;
        self.palms.pen_gone(event_time_ms);
        let finished = active.pen.finish(&active.stroke, 0.1);
        if !finished.is_empty() {
            self.committed_strokes.push(finished.clone());
        }
        Some(finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn stylus(
        x: f32,
        y: f32,
        t: f64,
        action: MotionAction,
        origin: SampleOrigin,
    ) -> AndroidMotionSample {
        AndroidMotionSample {
            pointer_id: 1,
            x,
            y,
            axis_pressure: 0.75,
            axis_tilt_rad: 30.0_f32.to_radians(),
            axis_orientation_rad: FRAC_PI_2,
            touch_major_px: 0.0,
            event_time_ms: t,
            tool_type: MotionToolType::Stylus,
            action,
            origin,
        }
    }

    #[test]
    fn android_tilt_and_orientation_map_to_ink_tilt() {
        let s = stylus(0.0, 0.0, 0.0, MotionAction::Down, SampleOrigin::Current);
        let tilt = s.tilt().expect("tilt");
        assert!((tilt.altitude() - 60.0).abs() < 0.1);
        assert!(tilt.azimuth().abs() < 0.1, "azimuth={}", tilt.azimuth());
    }

    #[test]
    fn historical_samples_sort_chronologically_and_finger_palms_are_rejected() {
        let mut batcher = MotionBatcher::new(320.0, Pen::default());
        assert_eq!(
            batcher.process_batch(&[stylus(
                0.0,
                0.0,
                10.0,
                MotionAction::Down,
                SampleOrigin::Current
            )]),
            MotionAcceptance::CommittedInk
        );
        // Out-of-order historical + current batch gets sorted before ingestion.
        batcher.process_batch(&[
            stylus(30.0, 0.0, 18.0, MotionAction::Move, SampleOrigin::Current),
            stylus(
                15.0,
                0.0,
                14.0,
                MotionAction::Move,
                SampleOrigin::Historical,
            ),
        ]);
        let samples = batcher.active_stroke().expect("active").stroke.samples();
        assert_eq!(samples.len(), 3);
        assert!(samples[1].time <= samples[2].time);

        // A finger touching while stylus is active is rejected as a palm.
        let palm = AndroidMotionSample {
            pointer_id: 42,
            x: 200.0,
            y: 300.0,
            axis_pressure: 0.5,
            axis_tilt_rad: 0.0,
            axis_orientation_rad: 0.0,
            touch_major_px: 320.0,
            event_time_ms: 20.0,
            tool_type: MotionToolType::Finger,
            action: MotionAction::Down,
            origin: SampleOrigin::Current,
        };
        assert_eq!(
            batcher.process_batch(&[palm]),
            MotionAcceptance::RejectedPalm
        );
    }
}
