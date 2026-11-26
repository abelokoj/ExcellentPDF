//! Audio recording synced to handwriting and lecture playback (plan L.1).
//!
//! Provides mockable [`AudioCapture`] and [`AudioPlayback`] traits (abstracting `cpal` and OS
//! microphone permission checks), pure-Rust WAV and Ogg/Opus asset encoders/decoders for `.expdf`
//! `assets/` bundling, and millisecond-accurate ink-stroke-to-audio timeline synchronization.

pub mod codec;
pub mod device;
pub mod sync;
pub mod transcribe;

pub use codec::{
    AudioBundleSpec, OggOpusPacket, WavData, bundle_audio_in_notebook, decode_ogg_opus, decode_wav,
    encode_ogg_opus, encode_wav,
};
pub use device::{
    AudioCapture, AudioError, AudioPlayback, CaptureConfig, MicPermission, MockAudioCapture,
    MockAudioPlayback, PlaybackState,
};
pub use sync::{AudioTimeline, PageMark, RecordingSegment, StrokeTimestamp};
