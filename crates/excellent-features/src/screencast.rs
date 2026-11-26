//! Screen and pen synchronized video/frame recording pipeline (plan L.4).
//!
//! Captures timestamped RGBA canvas frames alongside audio PCM chunks, pen stroke events, and
//! page switches on one monotonic millisecond clock; encodes through a mockable
//! [`VideoEncoderBackend`] (abstracting OS Media Foundation / AVAssetWriter / GStreamer); and
//! measures A/V synchronization skew across long recordings.

use std::fmt;
use std::io::Write as _;

use crate::audio::codec::encode_wav;
use crate::audio::sync::AudioTimeline;

/// Error returned by the screencast recording pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ScreencastError {
    /// Recording state transition was invalid (e.g., finishing before starting).
    InvalidState(String),
    /// Video or audio encoder backend rejected frame/audio data.
    Encoder(String),
}

impl fmt::Display for ScreencastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState(msg) => write!(f, "screencast state error: {msg}"),
            Self::Encoder(msg) => write!(f, "screencast encoder error: {msg}"),
        }
    }
}

impl std::error::Error for ScreencastError {}

/// Convenience result alias for screencast operations.
pub type Result<T> = std::result::Result<T, ScreencastError>;

/// Container format produced by the platform video encoder (plan L.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum VideoContainerFormat {
    /// MP4 (H.264 + AAC via Media Foundation / AVAssetWriter / GStreamer).
    #[default]
    Mp4,
    /// Uncompressed AVI / WAV muxed bundle for deterministic offline verification.
    RawBundle,
}

/// Screencast capture configuration (plan L.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreencastConfig {
    /// Output frame width in pixels.
    pub width: u32,
    /// Output frame height in pixels.
    pub height: u32,
    /// Target frames per second.
    pub fps: u32,
    /// Audio sample rate in Hz.
    pub audio_sample_rate: u32,
    /// Audio channel count (`1` mono, `2` stereo).
    pub audio_channels: u16,
    /// Target output container format.
    pub container: VideoContainerFormat,
}

impl Default for ScreencastConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            audio_sample_rate: 48_000,
            audio_channels: 1,
            container: VideoContainerFormat::Mp4,
        }
    }
}

/// One captured canvas frame with monotonic timestamp (plan L.4).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrameSample {
    /// Capture timestamp in milliseconds since session start.
    pub timestamp_ms: f64,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// RGBA pixels (`width * height * 4`), or compact digest frame in headless tests.
    pub rgba: Vec<u8>,
}

/// One captured PCM audio chunk with monotonic timestamp (plan L.4).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunkSample {
    /// Capture timestamp in milliseconds since session start.
    pub timestamp_ms: f64,
    /// Interleaved 16-bit PCM samples.
    pub pcm_i16: Vec<i16>,
}

/// Page/pen marker synchronized onto the screencast timeline (plan L.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ScreencastEvent {
    /// Presentation offset in milliseconds.
    pub offset_ms: f64,
    /// Event description (`"page:2"`, `"stroke:id"`, etc.).
    pub label: String,
}

/// Finalized recording metadata and A/V sync diagnostics (plan L.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ScreencastSummary {
    /// Total video frames encoded.
    pub video_frames: usize,
    /// Total audio samples encoded.
    pub audio_samples: usize,
    /// Total recorded duration in milliseconds.
    pub duration_ms: f64,
    /// Absolute drift between video timeline and audio sample clock in milliseconds.
    pub av_skew_ms: f64,
    /// Encoded container bytes.
    pub container_bytes: Vec<u8>,
    /// Synchronized timeline events.
    pub events: Vec<ScreencastEvent>,
}

/// Mockable platform video+audio encoder backend (Media Foundation, AVAssetWriter, GStreamer)
/// (plan L.4).
pub trait VideoEncoderBackend {
    /// Initializes an encoding session with `config`.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError::Encoder`] if the backend cannot initialize.
    fn begin(&mut self, config: ScreencastConfig) -> Result<()>;

    /// Pushes one timestamped RGBA frame.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError::Encoder`] if frame dimensions or state are invalid.
    fn push_video_frame(&mut self, frame: &VideoFrameSample) -> Result<()>;

    /// Pushes one timestamped PCM audio chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError::Encoder`] if audio encoding fails.
    fn push_audio_chunk(&mut self, chunk: &AudioChunkSample) -> Result<()>;

    /// Finalizes the output container bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError::Encoder`] if container muxing fails.
    fn finish(&mut self) -> Result<Vec<u8>>;
}

/// Deterministic in-memory [`VideoEncoderBackend`] producing a valid RIFF/WAV + frame index
/// stream for tests and headless verification (plan L.4).
#[derive(Debug, Default)]
pub struct MockVideoEncoder {
    config: Option<ScreencastConfig>,
    frame_timestamps_ms: Vec<f64>,
    pcm_samples: Vec<i16>,
    finished: bool,
}

impl MockVideoEncoder {
    /// Number of video frames received.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frame_timestamps_ms.len()
    }

    /// Number of PCM samples received.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.pcm_samples.len()
    }
}

impl VideoEncoderBackend for MockVideoEncoder {
    fn begin(&mut self, config: ScreencastConfig) -> Result<()> {
        if config.width == 0
            || config.height == 0
            || config.fps == 0
            || config.audio_sample_rate == 0
        {
            return Err(ScreencastError::Encoder(
                "screencast dimensions, fps, and sample rate must be non-zero".into(),
            ));
        }
        self.config = Some(config);
        self.frame_timestamps_ms.clear();
        self.pcm_samples.clear();
        self.finished = false;
        Ok(())
    }

    fn push_video_frame(&mut self, frame: &VideoFrameSample) -> Result<()> {
        let cfg = self
            .config
            .ok_or_else(|| ScreencastError::InvalidState("encoder not started".into()))?;
        if frame.width != cfg.width || frame.height != cfg.height {
            return Err(ScreencastError::Encoder(format!(
                "frame {}x{} does not match encoder {}x{}",
                frame.width, frame.height, cfg.width, cfg.height
            )));
        }
        self.frame_timestamps_ms.push(frame.timestamp_ms);
        Ok(())
    }

    fn push_audio_chunk(&mut self, chunk: &AudioChunkSample) -> Result<()> {
        if self.config.is_none() {
            return Err(ScreencastError::InvalidState("encoder not started".into()));
        }
        self.pcm_samples.extend_from_slice(&chunk.pcm_i16);
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let cfg = self
            .config
            .ok_or_else(|| ScreencastError::InvalidState("encoder not started".into()))?;
        if self.finished {
            return Err(ScreencastError::InvalidState(
                "encoder already finished".into(),
            ));
        }
        self.finished = true;

        // Emit a valid WAV track wrapped with a deterministic header marker for offline checks.
        let wav = encode_wav(&self.pcm_samples, cfg.audio_sample_rate, cfg.audio_channels)
            .map_err(|e| ScreencastError::Encoder(e.to_string()))?;

        let mut out = Vec::with_capacity(wav.len() + 32);
        out.extend_from_slice(b"EXMP4SIM");
        let _ = write!(
            out,
            "{}:{}:{}:",
            cfg.width,
            cfg.height,
            self.frame_timestamps_ms.len()
        );
        out.extend_from_slice(&wav);
        Ok(out)
    }
}

/// Synchronized screen, pen, and audio recording session (plan L.4).
#[derive(Debug)]
pub struct ScreencastSession<E: VideoEncoderBackend> {
    config: ScreencastConfig,
    encoder: E,
    timeline: AudioTimeline,
    video_frames: usize,
    audio_samples: usize,
    last_video_offset_ms: f64,
    events: Vec<ScreencastEvent>,
    active: bool,
}

impl<E: VideoEncoderBackend> ScreencastSession<E> {
    /// Starts a new synchronized recording session at `wall_start_ms` on `initial_page`.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError`] if the underlying encoder fails to start.
    pub fn start(
        recording_id: &str,
        wall_start_ms: f64,
        initial_page: usize,
        config: ScreencastConfig,
        mut encoder: E,
    ) -> Result<Self> {
        encoder.begin(config)?;
        Ok(Self {
            config,
            encoder,
            timeline: AudioTimeline::start(recording_id, wall_start_ms, initial_page),
            video_frames: 0,
            audio_samples: 0,
            last_video_offset_ms: 0.0,
            events: vec![ScreencastEvent {
                offset_ms: 0.0,
                label: format!("page:{initial_page}"),
            }],
            active: true,
        })
    }

    /// Pauses recording at `wall_now_ms`.
    pub fn pause(&mut self, wall_now_ms: f64) {
        self.timeline.pause(wall_now_ms);
    }

    /// Resumes recording at `wall_now_ms`.
    pub fn resume(&mut self, wall_now_ms: f64) {
        self.timeline.resume(wall_now_ms);
    }

    /// Records one video frame mapped onto the active audio timeline.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError`] if the session is stopped or encoding fails.
    pub fn record_frame(&mut self, wall_now_ms: f64, rgba: Vec<u8>) -> Result<()> {
        if !self.active {
            return Err(ScreencastError::InvalidState(
                "cannot record frame after finish".into(),
            ));
        }
        let offset_ms = self
            .timeline
            .audio_offset_for_wall_ms(wall_now_ms)
            .map_or(self.last_video_offset_ms, |d| d.as_secs_f64() * 1000.0);
        self.last_video_offset_ms = offset_ms;
        self.encoder.push_video_frame(&VideoFrameSample {
            timestamp_ms: offset_ms,
            width: self.config.width,
            height: self.config.height,
            rgba,
        })?;
        self.video_frames += 1;
        Ok(())
    }

    /// Records one audio PCM chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError`] if the session is stopped or encoding fails.
    pub fn record_audio(&mut self, wall_now_ms: f64, pcm_i16: Vec<i16>) -> Result<()> {
        if !self.active {
            return Err(ScreencastError::InvalidState(
                "cannot record audio after finish".into(),
            ));
        }
        let offset_ms = self
            .timeline
            .audio_offset_for_wall_ms(wall_now_ms)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        self.audio_samples += pcm_i16.len();
        self.encoder.push_audio_chunk(&AudioChunkSample {
            timestamp_ms: offset_ms,
            pcm_i16,
        })
    }

    /// Records a synchronized pen stroke or page navigation marker.
    pub fn record_event(&mut self, wall_now_ms: f64, label: &str) {
        let offset_ms = self
            .timeline
            .audio_offset_for_wall_ms(wall_now_ms)
            .map_or(self.last_video_offset_ms, |d| d.as_secs_f64() * 1000.0);
        self.events.push(ScreencastEvent {
            offset_ms,
            label: label.to_owned(),
        });
    }

    /// Stops recording at `wall_now_ms`, finalizes the container, and computes A/V skew.
    ///
    /// # Errors
    ///
    /// Returns [`ScreencastError`] if already finished or muxing fails.
    pub fn finish(mut self, wall_now_ms: f64) -> Result<ScreencastSummary> {
        if !self.active {
            return Err(ScreencastError::InvalidState(
                "session already finished".into(),
            ));
        }
        self.active = false;
        self.timeline.finish(wall_now_ms);
        let container_bytes = self.encoder.finish()?;
        let audio_duration_ms = if self.config.audio_sample_rate > 0 {
            (self.audio_samples as f64
                / (f64::from(self.config.audio_sample_rate)
                    * f64::from(self.config.audio_channels.max(1))))
                * 1000.0
        } else {
            0.0
        };
        let duration_ms = self.timeline.total_duration().as_secs_f64() * 1000.0;
        let av_skew_ms = (self.last_video_offset_ms - audio_duration_ms).abs();
        Ok(ScreencastSummary {
            video_frames: self.video_frames,
            audio_samples: self.audio_samples,
            duration_ms,
            av_skew_ms,
            container_bytes,
            events: self.events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_minute_recording_stays_in_audio_video_sync() {
        let cfg = ScreencastConfig {
            width: 320,
            height: 180,
            fps: 1,
            audio_sample_rate: 100,
            audio_channels: 1,
            container: VideoContainerFormat::Mp4,
        };
        let mut session =
            ScreencastSession::start("lecture-30m", 1_000.0, 0, cfg, MockVideoEncoder::default())
                .expect("start");

        // Simulate 1,800 seconds (30 minutes) of 1 fps video frames + 100 Hz audio chunks.
        for sec in 0..1800u32 {
            let wall = 1_000.0 + f64::from(sec) * 1000.0;
            session.record_frame(wall, Vec::new()).expect("frame");
            session.record_audio(wall, vec![0i16; 100]).expect("audio");
            if sec == 900 {
                session.record_event(wall, "stroke:pen-1");
            }
        }
        let summary = session.finish(1_000.0 + 1_800_000.0).expect("finish");
        assert_eq!(summary.video_frames, 1800);
        assert_eq!(summary.audio_samples, 180_000);
        assert!(
            summary.av_skew_ms <= 1000.0,
            "30-minute A/V skew too large: {} ms",
            summary.av_skew_ms
        );
        assert!(!summary.container_bytes.is_empty());
        assert!(
            summary
                .events
                .iter()
                .any(|e| e.label == "stroke:pen-1" && (e.offset_ms - 900_000.0).abs() < 1.0)
        );
    }
}
