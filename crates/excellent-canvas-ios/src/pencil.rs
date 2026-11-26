//! Apple Pencil input pipeline for iPadOS (plan K.11, §7.3, H.8): `UITouch` force, altitude, azimuth,
//! coalesced vs predicted touch samples, `PencilHover`, `PencilSqueeze` / double-tap tool toggling, palm
//! rejection by touch type, and Scribble suppression on the PDF canvas.

use std::f32::consts::FRAC_PI_2;

use excellent_canvas::page::gestures::{Palms, Verdict};
use excellent_ink::pens::Pen;
use excellent_ink::predict::predict;
use excellent_ink::sample::{Sample, Tilt, Tool};
use excellent_ink::stroke::{InkPoint, Stroke};

/// Source classification of a UIKit `UITouch` (`UITouch.TouchType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TouchKind {
    /// Apple Pencil (`UITouch.TouchType.pencil`).
    #[default]
    Pencil,
    /// Direct finger touch (`UITouch.TouchType.direct`).
    DirectFinger,
    /// Indirect trackpad/pointer input (`UITouch.TouchType.indirectPointer`).
    IndirectPointer,
}

/// Whether a `UITouch` sample came from the hardware event stream, coalesced history, or UIKit prediction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TouchSampleOrigin {
    /// Primary touch location for the current frame.
    #[default]
    Actual,
    /// High-frequency sub-frame sample from `UIEvent.coalescedTouches(for:)`.
    Coalesced,
    /// Speculative sample from `UIEvent.predictedTouches(for:)` (preview only; never committed).
    Predicted,
}

/// Phase of a touch or hover stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HoverPhase {
    /// Apple Pencil entered hover range above the view.
    Entered,
    /// Apple Pencil moved while hovering above the view.
    Moved,
    /// Apple Pencil left hover range.
    Exited,
}

/// Apple Pencil hardware gesture from `UIPencilInteraction` (2nd-gen double-tap or Pro squeeze).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PencilGesture {
    /// Double-tap on Apple Pencil barrel (2nd gen / Pro).
    DoubleTap,
    /// Barrel squeeze on Apple Pencil Pro.
    Squeeze,
}

/// User preference (`UIPencilPreferredAction`) controlling what a pencil gesture triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PencilPreferredAction {
    /// Toggle between the active ink pen and the eraser.
    #[default]
    SwitchEraser,
    /// Toggle between the active ink pen and the previously selected pen/tool.
    SwitchPreviousTool,
    /// Ask the host UI to present the ink color palette.
    ShowColorPalette,
    /// Ask the host UI to present ink thickness/attribute controls.
    ShowInkAttributes,
    /// Ignore the hardware gesture.
    Ignore,
}

/// Currently active tool selected on the iPad canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PencilToolMode {
    /// Drawing ink with the configured pen.
    #[default]
    Pen,
    /// Erasing ink annotations under the pencil tip.
    Eraser,
}

/// UIKit `UIScribbleInteraction` policy on the PDF canvas (plan K.11 item 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ScribblePolicy {
    /// Disable system Scribble handwriting-to-text over the PDF canvas so pencil strokes write ink cleanly.
    #[default]
    DisabledOnCanvas,
    /// Allow Scribble only inside native form text fields.
    FormsOnly,
}

/// Outcome returned when a [`PencilGesture`] is processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToolToggleOutcome {
    /// Switched the active tool mode (e.g. Pen <-> Eraser).
    ToolChanged(PencilToolMode),
    /// Host UI should toggle the color palette popover.
    OpenColorPalette,
    /// Host UI should toggle the ink attributes popover.
    OpenInkAttributes,
    /// No change applied.
    NoChange,
}

/// One Apple Pencil or touch sample captured from a UIKit `UIView`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PencilSample {
    /// Stable pointer identifier for the active stroke.
    pub pointer_id: u32,
    /// Horizontal position in view points.
    pub x: f32,
    /// Vertical position in view points.
    pub y: f32,
    /// Raw `UITouch.force` (typically `0.0..=maximum_possible_force`).
    pub force: f32,
    /// `UITouch.maximumPossibleForce`; `0.0` if force is unavailable.
    pub maximum_possible_force: f32,
    /// `UITouch.altitudeAngle` in radians (`π/2` upright, `0.0` flat on screen).
    pub altitude_rad: f32,
    /// `UITouch.azimuthAngle(in: view)` in radians (`0` along `+X`, clockwise on screen).
    pub azimuth_rad: f32,
    /// Timestamp in milliseconds on UIKit's monotonic clock.
    pub timestamp_ms: f64,
    /// Touch hardware kind (`UITouch.type`).
    pub touch_kind: TouchKind,
    /// Whether this sample is actual, coalesced, or predicted.
    pub origin: TouchSampleOrigin,
    /// Estimated touch radius in millimetres, used for direct-finger palm rejection.
    pub major_radius_mm: Option<f32>,
}

impl PencilSample {
    /// Normalizes `force / maximum_possible_force` into `0.0..=1.0`, or `None` when unsupported.
    #[must_use]
    pub fn normalized_force(&self) -> Option<f32> {
        if !self.maximum_possible_force.is_finite()
            || self.maximum_possible_force <= 0.0
            || !self.force.is_finite()
        {
            return None;
        }
        Some((self.force / self.maximum_possible_force).clamp(0.0, 1.0))
    }

    /// Converts UIKit `altitude_rad` and `azimuth_rad` into [`Tilt`].
    ///
    /// UIKit reports `altitudeAngle = π/2` when perpendicular and `0` when parallel to the glass,
    /// with `azimuthAngle` measured clockwise from `+X` in view coordinates.
    #[must_use]
    pub fn tilt(&self) -> Option<Tilt> {
        if !self.altitude_rad.is_finite() || !self.azimuth_rad.is_finite() {
            return None;
        }
        let clamped_alt = self.altitude_rad.clamp(0.02, FRAC_PI_2);
        let polar_rad = FRAC_PI_2 - clamped_alt;
        let radius = polar_rad.tan();
        let tx = radius * self.azimuth_rad.cos();
        let ty = radius * self.azimuth_rad.sin();
        Some(Tilt {
            x: tx.atan().to_degrees(),
            y: ty.atan().to_degrees(),
        })
    }

    /// Converts this UIKit touch sample into an [`excellent_ink::sample::Sample`].
    #[must_use]
    pub fn to_ink_sample(&self, tool: Tool) -> Sample {
        Sample {
            x: self.x,
            y: self.y,
            pressure: self.normalized_force(),
            tilt: self.tilt(),
            time: self.timestamp_ms,
            tool,
        }
    }
}

/// Apple Pencil hover sample (`UIHoverGestureRecognizer` / `UITouch` hover phase).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PencilHoverSample {
    /// Horizontal position in view points.
    pub x: f32,
    /// Vertical position in view points.
    pub y: f32,
    /// Normalized distance above the glass (`0.0` at surface, `1.0` at max hover height).
    pub z_offset: f32,
    /// Hover altitude angle in radians.
    pub altitude_rad: f32,
    /// Hover azimuth angle in radians.
    pub azimuth_rad: f32,
    /// Timestamp in milliseconds.
    pub timestamp_ms: f64,
    /// Hover phase.
    pub phase: HoverPhase,
}

/// Active stroke state with committed points and transient predicted preview points.
#[derive(Debug, Clone)]
pub struct ActivePencilStroke {
    /// Pointer identifier being tracked.
    pub pointer_id: u32,
    /// Underlying ink stroke built from actual and coalesced samples.
    pub stroke: Stroke,
    /// Pen preset active when this stroke started.
    pub pen: Pen,
    /// Transient predicted preview points from UIKit or `excellent_ink::predict`.
    pub predicted_preview: Vec<InkPoint>,
}

/// Result of feeding a [`PencilSample`] batch into [`PencilProcessor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleAcceptance {
    /// Sample committed to the active stroke.
    CommittedInk,
    /// Sample stored only in the transient prediction preview.
    PreviewOnly,
    /// Finger touch accepted for pan/zoom navigation.
    PassToNavigation,
    /// Touch rejected as a resting palm or disallowed touch type.
    RejectedPalm,
}

/// Coordinates Apple Pencil strokes, coalesced/predicted touch streams, hover, squeeze/double-tap,
/// palm rejection, and Scribble suppression on an iPadOS view.
#[derive(Debug)]
pub struct PencilProcessor {
    pen: Pen,
    tool_mode: PencilToolMode,
    previous_tool_mode: PencilToolMode,
    preferred_action: PencilPreferredAction,
    scribble_policy: ScribblePolicy,
    palms: Palms,
    active_stroke: Option<ActivePencilStroke>,
    hover_state: Option<PencilHoverSample>,
    committed_strokes: Vec<Vec<InkPoint>>,
}

impl Default for PencilProcessor {
    fn default() -> Self {
        Self::new(Pen::default())
    }
}

impl PencilProcessor {
    /// Creates a new pencil processor with `pen` and Scribble disabled over the PDF canvas.
    #[must_use]
    pub fn new(pen: Pen) -> Self {
        Self {
            pen,
            tool_mode: PencilToolMode::Pen,
            previous_tool_mode: PencilToolMode::Pen,
            preferred_action: PencilPreferredAction::SwitchEraser,
            scribble_policy: ScribblePolicy::DisabledOnCanvas,
            palms: Palms::default(),
            active_stroke: None,
            hover_state: None,
            committed_strokes: Vec::new(),
        }
    }

    /// Returns the active tool mode (`Pen` or `Eraser`).
    #[must_use]
    pub const fn tool_mode(&self) -> PencilToolMode {
        self.tool_mode
    }

    /// Updates the preferred hardware gesture action (`UIPencilInteraction.preferredTapAction`).
    pub fn set_preferred_action(&mut self, action: PencilPreferredAction) {
        self.preferred_action = action;
    }

    /// Returns the Scribble policy for the PDF canvas.
    #[must_use]
    pub const fn scribble_policy(&self) -> ScribblePolicy {
        self.scribble_policy
    }

    /// Sets the Scribble policy for the PDF canvas.
    pub fn set_scribble_policy(&mut self, policy: ScribblePolicy) {
        self.scribble_policy = policy;
    }

    /// Returns whether UIKit Scribble is suppressed on the page canvas.
    #[must_use]
    pub const fn should_suppress_scribble(&self) -> bool {
        matches!(self.scribble_policy, ScribblePolicy::DisabledOnCanvas)
    }

    /// Returns the current hover state while Apple Pencil is in hover range.
    #[must_use]
    pub const fn hover_state(&self) -> Option<PencilHoverSample> {
        self.hover_state
    }

    /// Returns the currently active stroke, if any.
    #[must_use]
    pub const fn active_stroke(&self) -> Option<&ActivePencilStroke> {
        self.active_stroke.as_ref()
    }

    /// Returns all finished strokes committed during this session.
    #[must_use]
    pub fn committed_strokes(&self) -> &[Vec<InkPoint>] {
        &self.committed_strokes
    }

    /// Handles an Apple Pencil barrel double-tap or Pro squeeze interaction.
    pub fn handle_pencil_gesture(&mut self, gesture: PencilGesture) -> ToolToggleOutcome {
        let _ = gesture;
        match self.preferred_action {
            PencilPreferredAction::SwitchEraser => {
                let next = match self.tool_mode {
                    PencilToolMode::Pen => PencilToolMode::Eraser,
                    PencilToolMode::Eraser => PencilToolMode::Pen,
                };
                self.previous_tool_mode = self.tool_mode;
                self.tool_mode = next;
                ToolToggleOutcome::ToolChanged(next)
            }
            PencilPreferredAction::SwitchPreviousTool => {
                std::mem::swap(&mut self.previous_tool_mode, &mut self.tool_mode);
                ToolToggleOutcome::ToolChanged(self.tool_mode)
            }
            PencilPreferredAction::ShowColorPalette => ToolToggleOutcome::OpenColorPalette,
            PencilPreferredAction::ShowInkAttributes => ToolToggleOutcome::OpenInkAttributes,
            PencilPreferredAction::Ignore => ToolToggleOutcome::NoChange,
        }
    }

    /// Processes a hover event from `UIHoverGestureRecognizer`.
    pub fn process_hover(&mut self, hover: PencilHoverSample) {
        match hover.phase {
            HoverPhase::Entered | HoverPhase::Moved => {
                self.palms.pen_near(hover.timestamp_ms);
                self.hover_state = Some(hover);
            }
            HoverPhase::Exited => {
                self.palms.pen_gone(hover.timestamp_ms);
                self.hover_state = None;
            }
        }
    }

    /// Begins tracking a touch down event.
    pub fn touch_down(&mut self, sample: PencilSample) -> SampleAcceptance {
        match sample.touch_kind {
            TouchKind::Pencil => {
                self.palms.pen_near(sample.timestamp_ms);
                if matches!(sample.origin, TouchSampleOrigin::Predicted) {
                    return SampleAcceptance::PreviewOnly;
                }
                let mut stroke = Stroke::new();
                let tool = match self.tool_mode {
                    PencilToolMode::Pen => Tool::Pen,
                    PencilToolMode::Eraser => Tool::Eraser,
                };
                self.pen.push(&mut stroke, sample.to_ink_sample(tool));
                self.active_stroke = Some(ActivePencilStroke {
                    pointer_id: sample.pointer_id,
                    stroke,
                    pen: self.pen,
                    predicted_preview: Vec::new(),
                });
                SampleAcceptance::CommittedInk
            }
            TouchKind::DirectFinger => {
                let diameter_mm = sample.major_radius_mm.map(|r| r * 2.0);
                match self
                    .palms
                    .touch_down(sample.pointer_id, sample.timestamp_ms, diameter_mm)
                {
                    Verdict::Ignore => SampleAcceptance::RejectedPalm,
                    Verdict::Pass => SampleAcceptance::PassToNavigation,
                    Verdict::Draw => SampleAcceptance::CommittedInk,
                }
            }
            TouchKind::IndirectPointer => SampleAcceptance::PassToNavigation,
        }
    }

    /// Feeds a batch of actual, coalesced, and predicted samples for an active touch.
    pub fn touch_moved(&mut self, samples: &[PencilSample]) -> SampleAcceptance {
        let mut last_outcome = SampleAcceptance::RejectedPalm;
        let mut saw_actual = false;

        for &sample in samples {
            if sample.touch_kind != TouchKind::Pencil {
                last_outcome = match self.palms.verdict(sample.pointer_id) {
                    Verdict::Ignore => SampleAcceptance::RejectedPalm,
                    Verdict::Pass => SampleAcceptance::PassToNavigation,
                    Verdict::Draw => SampleAcceptance::CommittedInk,
                };
                continue;
            }
            self.palms.pen_near(sample.timestamp_ms);

            let Some(active) = self.active_stroke.as_mut() else {
                continue;
            };
            if active.pointer_id != sample.pointer_id {
                continue;
            }

            let tool = match self.tool_mode {
                PencilToolMode::Pen => Tool::Pen,
                PencilToolMode::Eraser => Tool::Eraser,
            };
            let ink_sample = sample.to_ink_sample(tool);

            match sample.origin {
                TouchSampleOrigin::Actual | TouchSampleOrigin::Coalesced => {
                    if !saw_actual {
                        active.predicted_preview.clear();
                        saw_actual = true;
                    }
                    active.pen.push(&mut active.stroke, ink_sample);
                    last_outcome = SampleAcceptance::CommittedInk;
                }
                TouchSampleOrigin::Predicted => {
                    let prev = active
                        .stroke
                        .points()
                        .last()
                        .map_or(1.0, |p| p.width)
                        .max(0.5);
                    active.predicted_preview.push(InkPoint {
                        x: ink_sample.x,
                        y: ink_sample.y,
                        width: prev,
                    });
                    if !saw_actual {
                        last_outcome = SampleAcceptance::PreviewOnly;
                    }
                }
            }
        }

        if let Some(active) = self.active_stroke.as_mut()
            && active.predicted_preview.is_empty()
            && active.stroke.samples().len() >= 2
        {
            let tail_start = active.stroke.samples().len().saturating_sub(6);
            let predicted = predict(&active.stroke.samples()[tail_start..], 16.0, 24.0);
            let width = active
                .stroke
                .points()
                .last()
                .map_or(1.0, |p| p.width)
                .max(0.5);
            active
                .predicted_preview
                .extend(predicted.into_iter().map(|s| InkPoint {
                    x: s.0,
                    y: s.1,
                    width,
                }));
        }

        last_outcome
    }

    /// Completes the active touch; predicted points are discarded and actual/coalesced points are smoothed.
    pub fn touch_up(&mut self, pointer_id: u32, timestamp_ms: f64) -> Option<Vec<InkPoint>> {
        self.palms.touch_up(pointer_id);
        let active = self.active_stroke.as_ref()?;
        if active.pointer_id != pointer_id {
            return None;
        }
        let active = self.active_stroke.take()?;
        self.palms.pen_gone(timestamp_ms);
        let finished = active.pen.finish(&active.stroke, 0.1);
        if !finished.is_empty() {
            self.committed_strokes.push(finished.clone());
        }
        Some(finished)
    }
}

/// UIKit `UIScribbleInteraction` / `UIPencilInteraction` constants when compiling for iOS targets.
#[cfg(target_os = "ios")]
pub mod darwin {
    /// UIKit `UITouch.TouchType.pencil` raw value (`2`).
    pub const UI_TOUCH_TYPE_PENCIL: isize = 2;
    /// UIKit `UITouch.TouchType.direct` raw value (`0`).
    pub const UI_TOUCH_TYPE_DIRECT: isize = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pencil_sample(
        x: f32,
        y: f32,
        force: f32,
        t: f64,
        origin: TouchSampleOrigin,
    ) -> PencilSample {
        PencilSample {
            pointer_id: 7,
            x,
            y,
            force,
            maximum_possible_force: 4.0,
            altitude_rad: FRAC_PI_2 - 30.0_f32.to_radians(),
            azimuth_rad: 45.0_f32.to_radians(),
            timestamp_ms: t,
            touch_kind: TouchKind::Pencil,
            origin,
            major_radius_mm: None,
        }
    }

    #[test]
    fn altitude_and_azimuth_round_trip_through_tilt() {
        let sample = pencil_sample(10.0, 20.0, 2.0, 1.0, TouchSampleOrigin::Actual);
        let tilt = sample.tilt().expect("valid tilt");
        assert!(
            (tilt.altitude() - 60.0).abs() < 0.1,
            "altitude={}",
            tilt.altitude()
        );
        assert!(
            (tilt.azimuth() - 45.0).abs() < 0.1,
            "azimuth={}",
            tilt.azimuth()
        );
        assert_eq!(sample.normalized_force(), Some(0.5));
    }

    #[test]
    fn predicted_touches_never_commit_into_finished_stroke() {
        let mut proc = PencilProcessor::default();
        assert_eq!(
            proc.touch_down(pencil_sample(0.0, 0.0, 2.0, 0.0, TouchSampleOrigin::Actual)),
            SampleAcceptance::CommittedInk
        );
        assert_eq!(
            proc.touch_moved(&[
                pencil_sample(10.0, 0.0, 2.0, 4.0, TouchSampleOrigin::Coalesced),
                pencil_sample(20.0, 0.0, 2.0, 8.0, TouchSampleOrigin::Actual),
                pencil_sample(500.0, 500.0, 2.0, 24.0, TouchSampleOrigin::Predicted),
            ]),
            SampleAcceptance::CommittedInk
        );
        let preview = &proc
            .active_stroke()
            .expect("active stroke")
            .predicted_preview;
        assert!(preview.iter().any(|p| (p.x - 500.0).abs() < 0.01));

        let finished = proc.touch_up(7, 12.0).expect("finished stroke");
        assert!(
            finished.iter().all(|p| p.x < 100.0),
            "predicted sample leaked into committed stroke: {finished:?}"
        );
    }

    #[test]
    fn squeeze_and_double_tap_toggle_pen_and_eraser() {
        let mut proc = PencilProcessor::default();
        assert_eq!(proc.tool_mode(), PencilToolMode::Pen);
        assert_eq!(
            proc.handle_pencil_gesture(PencilGesture::Squeeze),
            ToolToggleOutcome::ToolChanged(PencilToolMode::Eraser)
        );
        assert_eq!(
            proc.handle_pencil_gesture(PencilGesture::DoubleTap),
            ToolToggleOutcome::ToolChanged(PencilToolMode::Pen)
        );
        assert!(proc.should_suppress_scribble());
    }

    #[test]
    fn hover_triggers_palm_rejection_for_direct_fingers() {
        let mut proc = PencilProcessor::default();
        proc.process_hover(PencilHoverSample {
            x: 50.0,
            y: 60.0,
            z_offset: 0.4,
            altitude_rad: 1.2,
            azimuth_rad: 0.5,
            timestamp_ms: 100.0,
            phase: HoverPhase::Entered,
        });
        assert!(proc.hover_state().is_some());

        let finger = PencilSample {
            pointer_id: 99,
            x: 200.0,
            y: 220.0,
            force: 0.0,
            maximum_possible_force: 0.0,
            altitude_rad: FRAC_PI_2,
            azimuth_rad: 0.0,
            timestamp_ms: 110.0,
            touch_kind: TouchKind::DirectFinger,
            origin: TouchSampleOrigin::Actual,
            major_radius_mm: Some(5.0),
        };
        assert_eq!(proc.touch_down(finger), SampleAcceptance::RejectedPalm);

        proc.process_hover(PencilHoverSample {
            x: 50.0,
            y: 60.0,
            z_offset: 0.0,
            altitude_rad: 1.2,
            azimuth_rad: 0.5,
            timestamp_ms: 150.0,
            phase: HoverPhase::Exited,
        });
        let late_finger = PencilSample {
            pointer_id: 100,
            timestamp_ms: 1200.0,
            ..finger
        };
        assert_eq!(
            proc.touch_down(late_finger),
            SampleAcceptance::PassToNavigation
        );
    }
}
