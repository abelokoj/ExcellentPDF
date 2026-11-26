//! Mockable audio capture and playback traits plus microphone permission model (plan L.1).

use std::fmt;
use std::time::Duration;

use excellent_core::notebook::AudioFormat;

/// Errors returned by audio capture, playback, or codec operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// Microphone permission was denied by the OS or user.
    PermissionDenied,
    /// No audio input or output device is available.
    NoDevice(String),
    /// Invalid state transition (e.g., starting capture while already recording).
    InvalidState(String),
    /// Corrupted or unsupported audio container / codec data.
    InvalidAudio(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => write!(f, "microphone permission denied"),
            Self::NoDevice(msg) => write!(f, "audio device unavailable: {msg}"),
            Self::InvalidState(msg) => write!(f, "invalid audio state: {msg}"),
            Self::InvalidAudio(msg) => write!(f, "invalid audio data: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Convenience result alias for audio operations.
pub type AudioResult<T> = std::result::Result<T, AudioError>;

/// OS microphone permission status (required on macOS, iPadOS, and Android; plan L.1 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MicPermission {
    /// Permission already granted.
    Granted,
    /// User denied microphone access.
    Denied,
    /// Platform requires prompting the user before capture can begin.
    PromptRequired,
    /// Platform does not require an explicit permission prompt.
    NotRequired,
}

/// Audio capture configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureConfig {
    /// Sample rate in Hz (default 48000 Hz, native for Opus).
    pub sample_rate: u32,
    /// Channel count (`1` = mono lecture voice, `2` = stereo).
    pub channels: u16,
    /// Target storage container format.
    pub format: AudioFormat,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 1,
            format: AudioFormat::OggOpus,
        }
    }
}

/// Playback transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlaybackState {
    /// Transport stopped at offset 0.
    #[default]
    Stopped,
    /// Currently playing audio.
    Playing,
    /// Paused at the current offset.
    Paused,
}

/// Platform-independent audio capture interface (backed by `cpal` on desktop/mobile).
pub trait AudioCapture {
    /// Checks the current OS microphone permission status.
    fn permission_status(&self) -> MicPermission;

    /// Requests microphone permission from the OS if needed.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::PermissionDenied`] if the user rejects access.
    fn request_permission(&mut self) -> AudioResult<MicPermission>;

    /// Starts capturing PCM frames using `config`.
    ///
    /// # Errors
    ///
    /// Returns an error if permission is denied, no device exists, or capture is already active.
    fn start(&mut self, config: CaptureConfig) -> AudioResult<()>;

    /// Stops capture and returns any remaining buffered 16-bit PCM samples.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidState`] if capture is not running.
    fn stop(&mut self) -> AudioResult<Vec<i16>>;

    /// Drains newly captured 16-bit PCM samples into `out`, returning how many samples were appended.
    fn drain_samples(&mut self, out: &mut Vec<i16>) -> usize;

    /// Whether capture is currently active.
    fn is_recording(&self) -> bool;
}

/// Platform-independent audio playback interface (backed by `cpal` output on desktop/mobile).
pub trait AudioPlayback {
    /// Loads decoded 16-bit PCM samples with the given sample rate and channel count.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidAudio`] if `sample_rate` or `channels` is zero.
    fn load_pcm(&mut self, samples: Vec<i16>, sample_rate: u32, channels: u16) -> AudioResult<()>;

    /// Starts or resumes playback.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidState`] if no audio is loaded.
    fn play(&mut self) -> AudioResult<()>;

    /// Pauses playback at the current offset.
    fn pause(&mut self);

    /// Stops playback and resets the position to zero.
    fn stop(&mut self);

    /// Seeks playback to `offset`, clamped to `[0, duration]`.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidState`] if no audio is loaded.
    fn seek(&mut self, offset: Duration) -> AudioResult<Duration>;

    /// Returns the current playback offset.
    fn position(&self) -> Duration;

    /// Returns the total loaded audio duration.
    fn duration(&self) -> Duration;

    /// Returns the current transport state.
    fn state(&self) -> PlaybackState;
}

/// Deterministic mock [`AudioCapture`] for unit and integration tests.
#[derive(Debug, Clone)]
pub struct MockAudioCapture {
    permission: MicPermission,
    grant_on_request: bool,
    recording: bool,
    pending_samples: Vec<i16>,
    config: Option<CaptureConfig>,
}

impl Default for MockAudioCapture {
    fn default() -> Self {
        Self::new(MicPermission::Granted)
    }
}

impl MockAudioCapture {
    /// Creates a mock capture device with initial `permission`.
    #[must_use]
    pub fn new(permission: MicPermission) -> Self {
        Self {
            permission,
            grant_on_request: permission != MicPermission::Denied,
            recording: false,
            pending_samples: Vec::new(),
            config: None,
        }
    }

    /// Feeds synthetic PCM frames into the mock microphone buffer.
    pub fn push_synthetic_frames(&mut self, samples: &[i16]) {
        self.pending_samples.extend_from_slice(samples);
    }

    /// Returns the last active capture configuration.
    #[must_use]
    pub fn active_config(&self) -> Option<CaptureConfig> {
        self.config
    }
}

impl AudioCapture for MockAudioCapture {
    fn permission_status(&self) -> MicPermission {
        self.permission
    }

    fn request_permission(&mut self) -> AudioResult<MicPermission> {
        if self.grant_on_request {
            self.permission = MicPermission::Granted;
            Ok(MicPermission::Granted)
        } else {
            self.permission = MicPermission::Denied;
            Err(AudioError::PermissionDenied)
        }
    }

    fn start(&mut self, config: CaptureConfig) -> AudioResult<()> {
        if self.permission == MicPermission::Denied {
            return Err(AudioError::PermissionDenied);
        }
        if self.permission == MicPermission::PromptRequired {
            self.request_permission()?;
        }
        if self.recording {
            return Err(AudioError::InvalidState("capture already running".into()));
        }
        self.config = Some(config);
        self.recording = true;
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<Vec<i16>> {
        if !self.recording {
            return Err(AudioError::InvalidState("capture not running".into()));
        }
        self.recording = false;
        Ok(std::mem::take(&mut self.pending_samples))
    }

    fn drain_samples(&mut self, out: &mut Vec<i16>) -> usize {
        let count = self.pending_samples.len();
        out.append(&mut self.pending_samples);
        count
    }

    fn is_recording(&self) -> bool {
        self.recording
    }
}

/// Deterministic mock [`AudioPlayback`] for unit and integration tests.
#[derive(Debug, Clone, Default)]
pub struct MockAudioPlayback {
    loaded: bool,
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
    duration: Duration,
    position: Duration,
    state: PlaybackState,
}

impl MockAudioPlayback {
    /// Creates an empty mock playback device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances playback time by `delta` when currently [`PlaybackState::Playing`].
    pub fn advance(&mut self, delta: Duration) {
        if self.state == PlaybackState::Playing {
            self.position = (self.position + delta).min(self.duration);
            if self.position >= self.duration {
                self.state = PlaybackState::Paused;
            }
        }
    }
}

impl AudioPlayback for MockAudioPlayback {
    fn load_pcm(&mut self, samples: Vec<i16>, sample_rate: u32, channels: u16) -> AudioResult<()> {
        if sample_rate == 0 || channels == 0 {
            return Err(AudioError::InvalidAudio(
                "sample_rate and channels must be positive".into(),
            ));
        }
        let frames = samples.len() / usize::from(channels);
        let secs = frames as f64 / f64::from(sample_rate);
        self.samples = samples;
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.duration = Duration::from_secs_f64(secs);
        self.position = Duration::ZERO;
        self.state = PlaybackState::Stopped;
        self.loaded = true;
        Ok(())
    }

    fn play(&mut self) -> AudioResult<()> {
        if !self.loaded {
            return Err(AudioError::InvalidState("no audio loaded".into()));
        }
        self.state = PlaybackState::Playing;
        Ok(())
    }

    fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.position = Duration::ZERO;
    }

    fn seek(&mut self, offset: Duration) -> AudioResult<Duration> {
        if !self.loaded {
            return Err(AudioError::InvalidState("no audio loaded".into()));
        }
        self.position = offset.min(self.duration);
        Ok(self.position)
    }

    fn position(&self) -> Duration {
        self.position
    }

    fn duration(&self) -> Duration {
        self.duration
    }

    fn state(&self) -> PlaybackState {
        self.state
    }
}
