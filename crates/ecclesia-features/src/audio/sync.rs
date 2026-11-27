//! Offset-synchronized audio recording and ink stroke playback timeline (plan L.1 steps 2–3).
//!
//! Records the wall-clock start time (and any pause/resume segments) on the same monotonic
//! millisecond clock that ink samples carry (`ecclesia_ink::sample::Sample::time`). Tapping any
//! stroke or annotation maps directly to its exact playback offset, and during playback the
//! timeline identifies which strokes are active at the current audio position.

use std::time::Duration;

use ecclesia_ink::stroke::Stroke;

use crate::audio::device::{AudioPlayback, AudioResult};

/// A contiguous recorded audio segment supporting pause/resume without drift.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordingSegment {
    /// Monotonic wall-clock start time in milliseconds (matching ink `Sample::time`).
    pub wall_start_ms: f64,
    /// Cumulative audio stream offset at the start of this segment, in milliseconds.
    pub audio_offset_ms: f64,
    /// Recorded length of this segment, in milliseconds.
    pub duration_ms: f64,
}

/// Marker recording which page was open at an audio offset (for playback page-follow and L.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageMark {
    /// Audio offset in milliseconds.
    pub audio_offset_ms: f64,
    /// Zero-based page index open at that moment.
    pub page: usize,
}

/// Timestamp span of a stroke or annotation on the recording's wall clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeTimestamp {
    /// First sample timestamp in milliseconds.
    pub start_ms: f64,
    /// Last sample timestamp in milliseconds.
    pub end_ms: f64,
}

impl StrokeTimestamp {
    /// Extracts the start and end timestamps from an ink [`Stroke`]'s hardware samples.
    #[must_use]
    pub fn from_stroke(stroke: &Stroke) -> Option<Self> {
        let first = stroke.samples().first()?;
        let last = stroke.samples().last()?;
        Some(Self {
            start_ms: first.time,
            end_ms: last.time.max(first.time),
        })
    }
}

/// Synchronizes an audio recording's timeline with ink strokes and annotations (plan L.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTimeline {
    /// Stable recording identifier.
    pub recording_id: String,
    /// Ordered non-overlapping recorded segments (handles pause/resume cleanly).
    pub segments: Vec<RecordingSegment>,
    /// Page navigation markers recorded during the lecture.
    pub page_marks: Vec<PageMark>,
    active_segment_start_wall_ms: Option<f64>,
    recorded_audio_ms: f64,
}

impl AudioTimeline {
    /// Creates a new timeline for `recording_id` starting at `wall_start_ms`.
    #[must_use]
    pub fn start(recording_id: impl Into<String>, wall_start_ms: f64, initial_page: usize) -> Self {
        Self {
            recording_id: recording_id.into(),
            segments: Vec::new(),
            page_marks: vec![PageMark {
                audio_offset_ms: 0.0,
                page: initial_page,
            }],
            active_segment_start_wall_ms: Some(wall_start_ms),
            recorded_audio_ms: 0.0,
        }
    }

    /// Pauses recording at `wall_now_ms`, closing the active segment.
    pub fn pause(&mut self, wall_now_ms: f64) {
        if let Some(start) = self.active_segment_start_wall_ms.take() {
            let dur = (wall_now_ms - start).max(0.0);
            self.segments.push(RecordingSegment {
                wall_start_ms: start,
                audio_offset_ms: self.recorded_audio_ms,
                duration_ms: dur,
            });
            self.recorded_audio_ms += dur;
        }
    }

    /// Resumes recording at `wall_now_ms`, opening a new segment.
    pub fn resume(&mut self, wall_now_ms: f64) {
        if self.active_segment_start_wall_ms.is_none() {
            self.active_segment_start_wall_ms = Some(wall_now_ms);
        }
    }

    /// Stops recording at `wall_now_ms`, finalizing any open segment.
    pub fn finish(&mut self, wall_now_ms: f64) {
        self.pause(wall_now_ms);
    }

    /// Records a page switch during active recording.
    pub fn mark_page(&mut self, wall_now_ms: f64, page: usize) {
        let offset = self
            .audio_offset_for_wall_ms(wall_now_ms)
            .map_or(self.recorded_audio_ms, |d| d.as_secs_f64() * 1000.0);
        self.page_marks.push(PageMark {
            audio_offset_ms: offset,
            page,
        });
    }

    /// Total recorded audio duration across all segments.
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        Duration::from_secs_f64((self.recorded_audio_ms / 1000.0).max(0.0))
    }

    /// Maps a wall-clock timestamp (`wall_ms`, from ink `Sample::time` or annotation creation) to
    /// its corresponding playback [`Duration`] offset. Returns `None` if written outside the
    /// recording window.
    #[must_use]
    pub fn audio_offset_for_wall_ms(&self, wall_ms: f64) -> Option<Duration> {
        for seg in &self.segments {
            if wall_ms >= seg.wall_start_ms && wall_ms <= seg.wall_start_ms + seg.duration_ms {
                let offset_ms = seg.audio_offset_ms + (wall_ms - seg.wall_start_ms);
                return Some(Duration::from_secs_f64((offset_ms / 1000.0).max(0.0)));
            }
        }
        if let Some(active_start) = self.active_segment_start_wall_ms
            && wall_ms >= active_start
        {
            let offset_ms = self.recorded_audio_ms + (wall_ms - active_start);
            return Some(Duration::from_secs_f64((offset_ms / 1000.0).max(0.0)));
        }
        None
    }

    /// Maps an ink [`Stroke`] to the audio playback offset when it began being written.
    #[must_use]
    pub fn offset_for_stroke(&self, stroke: &Stroke) -> Option<Duration> {
        let ts = StrokeTimestamp::from_stroke(stroke)?;
        self.audio_offset_for_wall_ms(ts.start_ms)
    }

    /// Seeks `playback` to the moment `stroke` was written (and starts playback if requested via
    /// `SeekAction`), returning the exact sought [`Duration`].
    ///
    /// # Errors
    ///
    /// Returns an [`crate::audio::AudioError`] if playback seeking fails.
    pub fn seek_playback_to_stroke<P: AudioPlayback>(
        &self,
        playback: &mut P,
        stroke: &Stroke,
    ) -> AudioResult<Option<Duration>> {
        let Some(target) = self.offset_for_stroke(stroke) else {
            return Ok(None);
        };
        let actual = playback.seek(target)?;
        Ok(Some(actual))
    }

    /// Returns the indices of strokes whose written timestamp falls within `[0 .. playback_offset]`
    /// (plus `highlight_window` for actively highlighted strokes being written right now).
    #[must_use]
    pub fn highlighted_strokes(
        &self,
        playback_offset: Duration,
        highlight_window: Duration,
        strokes: &[Stroke],
    ) -> Vec<usize> {
        let pos_ms = playback_offset.as_secs_f64() * 1000.0;
        let win_ms = highlight_window.as_secs_f64() * 1000.0;
        strokes
            .iter()
            .enumerate()
            .filter_map(|(idx, stroke)| {
                let ts = StrokeTimestamp::from_stroke(stroke)?;
                let start = self.audio_offset_for_wall_ms(ts.start_ms)?.as_secs_f64() * 1000.0;
                let end = self
                    .audio_offset_for_wall_ms(ts.end_ms)
                    .map_or(start, |d| d.as_secs_f64() * 1000.0);
                let active = pos_ms >= start && pos_ms <= end + win_ms;
                active.then_some(idx)
            })
            .collect()
    }

    /// Returns the page index that was open at `playback_offset`.
    #[must_use]
    pub fn page_at_offset(&self, playback_offset: Duration) -> Option<usize> {
        let pos_ms = playback_offset.as_secs_f64() * 1000.0;
        self.page_marks
            .iter()
            .rev()
            .find(|m| m.audio_offset_ms <= pos_ms)
            .map(|m| m.page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::device::{AudioCapture, CaptureConfig, MockAudioCapture, MockAudioPlayback};
    use ecclesia_ink::sample::Sample;
    use ecclesia_ink::width::WidthCurve;

    #[test]
    fn tapping_stroke_written_ten_minutes_into_lecture_seeks_within_half_second() {
        // Simulate a 15-minute (900 s) lecture recording starting at wall clock t = 5_000 ms,
        // with a 30-second pause at minute 5 (audio offset 300 s, wall 305_000..335_000 ms).
        let mut capture = MockAudioCapture::default();
        capture
            .start(CaptureConfig::default())
            .expect("start capture");
        capture.push_synthetic_frames(&[100, -100, 200, -200]);

        let mut timeline = AudioTimeline::start("lec-physics", 5_000.0, 0);
        timeline.pause(305_000.0); // 5 min recorded (300 s)
        timeline.resume(335_000.0); // 30 s pause
        timeline.mark_page(400_000.0, 1);
        timeline.finish(935_000.0); // +10 min recorded = 15 min total (900 s)

        let captured = capture.stop().expect("stop capture");
        assert_eq!(captured.len(), 4);
        assert_eq!(timeline.total_duration(), Duration::from_secs(900));

        // Load a 900-second mock playback buffer (100 Hz synthetic rate to stay tiny in RAM).
        let mut playback = MockAudioPlayback::new();
        playback
            .load_pcm(vec![0i16; 90_000], 100, 1)
            .expect("load 900s clip");
        playback.play().expect("play");

        // Stroke written exactly 10 minutes (600 s) into the recorded audio:
        // 300 s in segment 1 + 300 s into segment 2 => wall time = 335_000 + 300_000 = 635_000 ms.
        let stroke_10min = Stroke::from_samples(
            &[
                Sample::at(100.0, 200.0, 635_000.0),
                Sample::at(120.0, 210.0, 635_400.0),
            ],
            &WidthCurve::default(),
        );

        let sought = timeline
            .seek_playback_to_stroke(&mut playback, &stroke_10min)
            .expect("seek")
            .expect("in recording");
        let diff = (sought.as_secs_f64() - 600.0).abs();
        assert!(
            diff <= 0.5,
            "sought {sought:?} must be within ±0.5s of 600s (diff={diff}s)"
        );
        assert_eq!(playback.position(), sought);
        assert_eq!(timeline.page_at_offset(sought), Some(1));

        // During playback at 600.2s, stroke_10min is highlighted!
        let active = timeline.highlighted_strokes(
            Duration::from_millis(600_200),
            Duration::from_millis(500),
            std::slice::from_ref(&stroke_10min),
        );
        assert_eq!(active, vec![0]);
    }
}
