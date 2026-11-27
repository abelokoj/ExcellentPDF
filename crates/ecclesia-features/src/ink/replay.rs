//! Page-level stroke replay controller (H.22, plan §27.7): replays a page's ink in the exact order and timing it was
//! written, with play, pause, speed control, scrubbing, stroke stepping, and private-key JSON serialization for raw
//! sample payloads.

use ecclesia_core::annot::{Annotation, Shape};
use ecclesia_ink::pens::Blend;
use ecclesia_ink::replay::{
    DecodeSamplesError, ReplayFrame, TimedPoint, TimedStroke, Timeline, decode_samples,
    encode_samples,
};
use ecclesia_ink::sample::Sample;
use ecclesia_ink::stroke::InkPoint;
use serde_json::{Value, json};

/// Allowed playback speed range (`0.25×` to `8.0×`).
pub const SPEED_RANGE: (f32, f32) = (0.25, 8.0);

/// Preset playback speeds offered in the replay toolbar.
pub const SPEED_PRESETS: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 4.0];

/// Fallback writing speed (points per millisecond) used to synthesize timing for legacy strokes without raw samples.
const FALLBACK_SPEED_PT_PER_MS: f64 = 0.25;

/// Fallback pause between legacy strokes without raw samples, in milliseconds.
const FALLBACK_STROKE_GAP_MS: f64 = 180.0;

/// Validated playback speed multiplier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSpeed(f32);

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

impl PlaybackSpeed {
    /// Constructs a speed clamped to [`SPEED_RANGE`] (defaulting to `1.0` for non-finite values).
    #[must_use]
    pub fn new(multiplier: f32) -> Self {
        let v = if multiplier.is_finite() {
            multiplier.clamp(SPEED_RANGE.0, SPEED_RANGE.1)
        } else {
            1.0
        };
        Self(v)
    }

    /// The numeric multiplier (`1.0` = real-time).
    #[must_use]
    pub fn factor(self) -> f32 {
        self.0
    }
}

/// Transport state of the stroke replay controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlaybackState {
    /// Reset to the beginning (`0.0 ms`).
    #[default]
    Stopped,
    /// Actively advancing along the timeline.
    Playing,
    /// Held at the current timestamp.
    Paused,
    /// Reached the end of the timeline.
    Finished,
}

/// Encodes raw [`Sample`]s into a compact hex string suitable for embedding in a PDF annotation's private JSON key
/// (`"samples"`).
#[must_use]
pub fn encode_samples_hex(samples: &[Sample]) -> String {
    use std::fmt::Write as _;
    let bytes = encode_samples(samples);
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Decodes raw [`Sample`]s previously serialized with [`encode_samples_hex`].
///
/// # Errors
///
/// Returns [`DecodeSamplesError`] if the hex string is malformed or the binary payload is invalid.
pub fn decode_samples_hex(text: &str) -> Result<Vec<Sample>, DecodeSamplesError> {
    if !text.len().is_multiple_of(2) {
        return Err(DecodeSamplesError::Truncated);
    }
    let bytes: Option<Vec<u8>> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect();
    let bytes = bytes.ok_or(DecodeSamplesError::Truncated)?;
    decode_samples(&bytes)
}

/// Serializes one or more raw sample strokes into a JSON value for annotation private storage.
#[must_use]
pub fn raw_samples_to_json(strokes: &[Vec<Sample>]) -> Value {
    json!({
        "format": 1,
        "strokes": strokes.iter().map(|s| encode_samples_hex(s)).collect::<Vec<_>>(),
    })
}

/// Deserializes raw sample strokes written by [`raw_samples_to_json`].
///
/// # Errors
///
/// Fails if any encoded stroke payload is corrupted.
pub fn raw_samples_from_json(value: &Value) -> Result<Vec<Vec<Sample>>, DecodeSamplesError> {
    let Some(list) = value.get("strokes").and_then(Value::as_array) else {
        return Err(DecodeSamplesError::BadMagic);
    };
    list.iter()
        .map(|v| {
            let hex = v.as_str().ok_or(DecodeSamplesError::BadMagic)?;
            decode_samples_hex(hex)
        })
        .collect()
}

/// Pairs an ink [`Annotation`] with its optional raw recorded samples (one `Vec<Sample>` per stroke in `Shape::Ink`).
#[derive(Debug, Clone)]
pub struct PageInkRecord<'a> {
    /// The PDF ink annotation.
    pub annotation: &'a Annotation,
    /// Raw samples recorded when the annotation was drawn, if available.
    pub raw_samples: Option<&'a [Vec<Sample>]>,
}

/// Converts a page's ink annotations (with optional raw samples) into [`TimedStroke`]s ready for [`ReplayPlayer`].
///
/// Annotations with raw samples use their exact recorded timestamps; legacy annotations without raw samples receive
/// deterministic arc-length timing ordered by `annotation.created`.
#[must_use]
pub fn build_page_strokes(records: &[PageInkRecord<'_>]) -> Vec<TimedStroke> {
    let mut out = Vec::new();
    let mut fallback_clock = 0.0_f64;

    for record in records {
        let Shape::Ink(ink_strokes) = &record.annotation.shape else {
            continue;
        };
        let color = rgb_to_hex(record.annotation.style.color.unwrap_or([0.0, 0.0, 0.0]));
        let opacity = record.annotation.style.opacity;
        let blend = if record.annotation.multiply {
            Blend::Multiply
        } else {
            Blend::Normal
        };
        let base_width = record.annotation.style.width;

        if let Some(raw_list) = record.raw_samples
            && !raw_list.is_empty()
        {
            for (idx, samples) in raw_list.iter().enumerate() {
                if samples.is_empty() {
                    continue;
                }
                let widths = record
                    .annotation
                    .widths
                    .get(idx)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let stroke = TimedStroke::from_samples(
                    format!("{}#{idx}", record.annotation.id),
                    color,
                    opacity,
                    blend,
                    samples,
                    widths,
                    base_width,
                );
                if let Some(end) = stroke.end_ms() {
                    fallback_clock = fallback_clock.max(end + FALLBACK_STROKE_GAP_MS);
                }
                out.push(stroke);
            }
            continue;
        }

        // Legacy ink without raw samples: synthesize smooth arc-length timing.
        for (idx, path) in ink_strokes.iter().enumerate() {
            if path.is_empty() {
                continue;
            }
            let widths = record.annotation.widths.get(idx);
            let mut t = fallback_clock;
            let mut timed = Vec::with_capacity(path.len());
            for (k, pt) in path.iter().enumerate() {
                if k > 0 {
                    let prev = path[k - 1];
                    let dist = f64::from((pt.x - prev.x).hypot(pt.y - prev.y));
                    t += (dist / FALLBACK_SPEED_PT_PER_MS).max(4.0);
                }
                let w = widths
                    .and_then(|list| list.get(k).copied())
                    .unwrap_or(base_width);
                timed.push(TimedPoint {
                    point: InkPoint {
                        x: pt.x,
                        y: pt.y,
                        width: w,
                    },
                    time_ms: t,
                });
            }
            fallback_clock = t + FALLBACK_STROKE_GAP_MS;
            out.push(TimedStroke {
                id: format!("{}#{idx}", record.annotation.id),
                color,
                opacity,
                blend,
                points: timed,
            });
        }
    }
    out
}

/// Interactive transport controller for replaying a page's ink strokes.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayPlayer {
    timeline: Timeline,
    state: PlaybackState,
    position_ms: f64,
    speed: PlaybackSpeed,
}

impl ReplayPlayer {
    /// Creates a stopped player over `timeline` at `1.0×` speed.
    #[must_use]
    pub fn new(timeline: Timeline) -> Self {
        Self {
            timeline,
            state: PlaybackState::Stopped,
            position_ms: 0.0,
            speed: PlaybackSpeed::default(),
        }
    }

    /// Creates a player from page ink records, preserving exact verbatim recording timing (`max_pause_ms = None`).
    #[must_use]
    pub fn from_page_records(records: &[PageInkRecord<'_>], max_pause_ms: Option<f64>) -> Self {
        Self::new(Timeline::new(build_page_strokes(records), max_pause_ms))
    }

    /// The underlying stroke timeline.
    #[must_use]
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Current transport state.
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Current playback position in milliseconds.
    #[must_use]
    pub fn position_ms(&self) -> f64 {
        self.position_ms
    }

    /// Current playback speed.
    #[must_use]
    pub fn speed(&self) -> PlaybackSpeed {
        self.speed
    }

    /// Sets the playback speed multiplier.
    pub fn set_speed(&mut self, speed: PlaybackSpeed) {
        self.speed = speed;
    }

    /// Starts or resumes playback. If already finished, restarts from `0.0 ms`.
    pub fn play(&mut self) {
        if self.timeline.is_empty() {
            self.state = PlaybackState::Finished;
            return;
        }
        if self.state == PlaybackState::Finished
            || self.position_ms >= self.timeline.total_duration_ms()
        {
            self.position_ms = 0.0;
        }
        self.state = PlaybackState::Playing;
    }

    /// Pauses playback at the current timestamp.
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    /// Toggles between [`PlaybackState::Playing`] and [`PlaybackState::Paused`].
    pub fn toggle_play_pause(&mut self) {
        match self.state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Stopped | PlaybackState::Paused | PlaybackState::Finished => self.play(),
        }
    }

    /// Stops playback and rewinds to `0.0 ms`.
    pub fn stop(&mut self) {
        self.position_ms = 0.0;
        self.state = PlaybackState::Stopped;
    }

    /// Seeks directly to `ms` (clamped to the timeline's duration).
    pub fn seek(&mut self, ms: f64) {
        let clamped = if ms.is_finite() {
            ms.clamp(0.0, self.timeline.total_duration_ms())
        } else {
            0.0
        };
        self.position_ms = clamped;
        if self.state == PlaybackState::Finished && clamped < self.timeline.total_duration_ms() {
            self.state = PlaybackState::Paused;
        }
    }

    /// Seeks to a normalized progress fraction in `0.0..=1.0`.
    pub fn seek_progress(&mut self, fraction: f32) {
        let f = if fraction.is_finite() {
            f64::from(fraction.clamp(0.0, 1.0))
        } else {
            0.0
        };
        self.seek(self.timeline.total_duration_ms() * f);
    }

    /// Steps forward to the completion of the next stroke.
    pub fn step_stroke_forward(&mut self) {
        if let Some(target) = self
            .timeline
            .strokes()
            .iter()
            .filter_map(TimedStroke::end_ms)
            .find(|&end| end > self.position_ms + 1e-3)
        {
            self.seek(target);
        } else {
            self.seek(self.timeline.total_duration_ms());
        }
    }

    /// Steps backward to the completion of the previous stroke (or `0.0 ms`).
    pub fn step_stroke_backward(&mut self) {
        if let Some(target) = self
            .timeline
            .strokes()
            .iter()
            .rev()
            .filter_map(TimedStroke::end_ms)
            .find(|&end| end < self.position_ms - 1e-3)
        {
            self.seek(target);
        } else {
            self.seek(0.0);
        }
    }

    /// Advances playback by wall-clock `dt_ms` (scaled by `speed`) when [`PlaybackState::Playing`], and returns the
    /// resulting [`ReplayFrame`]. Does not advance while paused, stopped, or finished.
    pub fn advance(&mut self, dt_ms: f64) -> ReplayFrame {
        if self.state == PlaybackState::Playing && dt_ms.is_finite() && dt_ms > 0.0 {
            let next = self.position_ms + dt_ms * f64::from(self.speed.factor());
            let total = self.timeline.total_duration_ms();
            if next >= total {
                self.position_ms = total;
                self.state = PlaybackState::Finished;
            } else {
                self.position_ms = next;
            }
        }
        self.current_frame()
    }

    /// Returns the frame at the current playback position without advancing the clock.
    #[must_use]
    pub fn current_frame(&self) -> ReplayFrame {
        self.timeline.frame_at(self.position_ms)
    }
}

fn rgb_to_hex(rgb: [f32; 3]) -> u32 {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (ch(rgb[0]) << 16) | (ch(rgb[1]) << 8) | ch(rgb[2])
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::Style;
    use ecclesia_ink::session::Session;
    use ecclesia_mupdf::PagePoint;

    use super::*;

    fn ink_from_samples(id: &str, strokes: &[Vec<Sample>]) -> Annotation {
        let paths = strokes
            .iter()
            .map(|s| s.iter().map(|p| PagePoint { x: p.x, y: p.y }).collect())
            .collect();
        let mut a = Annotation::new(
            0,
            Shape::Ink(paths),
            Style {
                color: Some([0.1, 0.2, 0.7]),
                opacity: 1.0,
                width: 2.0,
            },
            "Ada",
        );
        a.id = id.to_owned();
        a
    }

    #[test]
    fn replay_timing_matches_the_recording() {
        // Load the real recorded session fixture from ecclesia-ink.
        let fixture =
            include_str!("../../../ecclesia-ink/tests/sessions/synthetic-e-line-circle.session");
        let session = Session::parse(fixture).expect("parses session fixture");
        assert!(session.strokes.len() >= 3);

        // Round-trip the raw samples through private-key JSON hex storage.
        let json_payload = raw_samples_to_json(&session.strokes);
        let decoded = raw_samples_from_json(&json_payload).expect("decodes raw samples");
        assert_eq!(decoded.len(), session.strokes.len());

        let annot = ink_from_samples("page-ink-1", &decoded);
        let records = [PageInkRecord {
            annotation: &annot,
            raw_samples: Some(&decoded),
        }];
        let mut player = ReplayPlayer::from_page_records(&records, None);

        let expected_start = session
            .strokes
            .iter()
            .filter_map(|s| s.first().map(|p| p.time))
            .fold(f64::INFINITY, f64::min);
        let expected_end = session
            .strokes
            .iter()
            .filter_map(|s| s.last().map(|p| p.time))
            .fold(f64::NEG_INFINITY, f64::max);
        let expected_duration = expected_end - expected_start;

        assert!(
            (player.timeline().total_duration_ms() - expected_duration).abs() < 0.01,
            "timeline duration {} vs recording {}",
            player.timeline().total_duration_ms(),
            expected_duration
        );

        // Replay in 10 ms ticks at 1.0x speed and verify every stroke starts and finishes at its recorded time.
        player.play();
        let mut wall_ms = 0.0_f64;
        let mut first_seen: Vec<Option<f64>> = vec![None; decoded.len()];
        let mut first_done: Vec<Option<f64>> = vec![None; decoded.len()];

        while player.state() == PlaybackState::Playing {
            wall_ms += 10.0;
            let frame = player.advance(10.0);
            for s in &frame.strokes {
                if first_seen[s.stroke_index].is_none() {
                    first_seen[s.stroke_index] = Some(wall_ms);
                }
                if s.complete && first_done[s.stroke_index].is_none() {
                    first_done[s.stroke_index] = Some(wall_ms);
                }
            }
        }
        assert_eq!(player.state(), PlaybackState::Finished);
        assert!(
            (wall_ms - expected_duration).abs() <= 10.01,
            "1x wall clock {wall_ms} vs recording {expected_duration}"
        );
        for (i, stroke) in player.timeline().strokes().iter().enumerate() {
            let rec_start = stroke.start_ms().expect("start");
            let rec_end = stroke.end_ms().expect("end");
            let seen = first_seen[i].expect("seen");
            let done = first_done[i].expect("done");
            assert!(
                (seen - rec_start).abs() <= 10.01,
                "stroke {i} appeared at {seen}, recorded at {rec_start}"
            );
            assert!(
                (done - rec_end).abs() <= 10.01,
                "stroke {i} completed at {done}, recorded at {rec_end}"
            );
        }

        // At 2.0x speed, total wall time is half of the recording duration.
        player.set_speed(PlaybackSpeed::new(2.0));
        player.play();
        let mut fast_wall = 0.0;
        while player.state() == PlaybackState::Playing {
            fast_wall += 10.0;
            player.advance(10.0);
        }
        assert!(
            (fast_wall - expected_duration / 2.0).abs() <= 10.01,
            "2x wall clock {fast_wall} vs expected {}",
            expected_duration / 2.0
        );
    }

    #[test]
    fn pause_seek_and_legacy_fallback_work() {
        let s1 = vec![Sample::at(0.0, 0.0, 0.0), Sample::at(100.0, 0.0, 200.0)];
        let s2 = vec![Sample::at(0.0, 50.0, 400.0), Sample::at(100.0, 50.0, 600.0)];
        let raw = vec![s1, s2];
        let annot = ink_from_samples("a", &raw);
        let mut player = ReplayPlayer::from_page_records(
            &[PageInkRecord {
                annotation: &annot,
                raw_samples: Some(&raw),
            }],
            None,
        );

        player.play();
        let f1 = player.advance(100.0);
        assert!((f1.elapsed_ms - 100.0).abs() < 1e-6);
        player.pause();
        let f2 = player.advance(250.0);
        assert_eq!(f2.elapsed_ms, 100.0, "paused does not advance");

        player.step_stroke_forward();
        assert!((player.position_ms() - 200.0).abs() < 1e-6);
        player.step_stroke_forward();
        assert!((player.position_ms() - 600.0).abs() < 1e-6);
        player.step_stroke_backward();
        assert!((player.position_ms() - 200.0).abs() < 1e-6);

        // Legacy annotation without raw samples still builds a positive-duration timeline.
        let legacy = ReplayPlayer::from_page_records(
            &[PageInkRecord {
                annotation: &annot,
                raw_samples: None,
            }],
            None,
        );
        assert_eq!(legacy.timeline().len(), 2);
        assert!(legacy.timeline().total_duration_ms() > 0.0);
    }
}
