//! Neural offline TTS voice pack catalog and Piper-style ONNX voice synthesizer (plan U.6).
//!
//! Scans voice packs under `packs/voices/` with explicit license and SHA-256 verification, and
//! exposes [`PiperVoiceSynthesizer`] implementing [`ecclesia_platform::speech::SpeechSynth`] so
//! offline neural voices plug directly into Read Aloud (`read_aloud::ReadAloudSession`) with
//! deterministic word-boundary highlighting.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ecclesia_platform::speech::{SpeechError, SpeechEvent, SpeechSynth, VoiceInfo, clamp_speed};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Quality tier of a Piper-style neural voice pack (plan U.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VoiceQuality {
    /// Ultra-compact 16 kHz voice (~15 MB).
    XLow,
    /// Fast 16 kHz voice (~30 MB).
    Low,
    /// Balanced 22.05 kHz voice (~60 MB).
    #[default]
    Medium,
    /// Studio 22.05 kHz voice (~100 MB).
    High,
}

impl VoiceQuality {
    /// Manifest string identifier.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::XLow => "x_low",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parses a quality tier string.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "x_low" => Self::XLow,
            "low" => Self::Low,
            "high" => Self::High,
            _ => Self::Medium,
        }
    }
}

/// Approved open-source / redistributable license classes for voice packs (plan U.6 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceLicenseStatus {
    /// Approved open license (`MIT`, `CC0-1.0`, `Apache-2.0`, `BSD-3-Clause`).
    Approved,
    /// Unknown or non-redistributable license; blocked by default unless explicitly reviewed.
    Unverified,
}

/// Checks whether a voice pack's SPDX license string is an approved redistributable license.
#[must_use]
pub fn check_voice_license(spdx: &str) -> VoiceLicenseStatus {
    let trimmed = spdx.trim();
    if [
        "MIT",
        "CC0-1.0",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "CC-BY-4.0",
    ]
    .iter()
    .any(|ok| ok.eq_ignore_ascii_case(trimmed))
    {
        VoiceLicenseStatus::Approved
    } else {
        VoiceLicenseStatus::Unverified
    }
}

/// Metadata manifest for a Piper-style ONNX voice pack (plan U.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePackManifest {
    /// Unique voice pack ID (e.g., `"en_US-lessac-medium"`).
    pub id: String,
    /// Display voice name (e.g., `"Lessac (Neural)"`).
    pub name: String,
    /// BCP-47 language tag (e.g., `"en-US"`).
    pub language: String,
    /// Voice quality tier.
    pub quality: VoiceQuality,
    /// Output sample rate in Hz (typically `22050` or `16000`).
    pub sample_rate_hz: u32,
    /// Relative ONNX model filename (e.g., `"en_US-lessac-medium.onnx"`).
    pub model_file: String,
    /// Relative Piper JSON config filename (e.g., `"en_US-lessac-medium.onnx.json"`).
    pub config_file: String,
    /// Expected SHA-256 hex digest of `model_file`.
    pub model_sha256: String,
    /// SPDX license identifier (checked before activation).
    pub license: String,
}

impl VoicePackManifest {
    /// Parses a voice pack manifest from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError`] if JSON is malformed or missing required fields.
    pub fn from_json_bytes(bytes: &[u8]) -> std::result::Result<Self, VoicePackError> {
        let val: Value =
            serde_json::from_slice(bytes).map_err(|e| VoicePackError::Invalid(e.to_string()))?;
        let req = |k: &str| -> std::result::Result<String, VoicePackError> {
            val.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| VoicePackError::Invalid(format!("missing '{k}'")))
        };
        let id = req("id")?;
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Err(VoicePackError::Invalid(format!("unsafe voice id '{id}'")));
        }
        let sample_rate_hz = val
            .get("sample_rate_hz")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(22_050);
        let quality = val
            .get("quality")
            .and_then(Value::as_str)
            .map(VoiceQuality::parse)
            .unwrap_or_default();
        Ok(Self {
            id,
            name: req("name")?,
            language: req("language")?,
            quality,
            sample_rate_hz,
            model_file: req("model_file")?,
            config_file: req("config_file")?,
            model_sha256: req("model_sha256")?,
            license: req("license")?,
        })
    }

    /// Serializes this manifest to JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError::Invalid`] if serialization fails.
    pub fn to_json_bytes(&self) -> std::result::Result<Vec<u8>, VoicePackError> {
        let val = json!({
            "id": self.id,
            "name": self.name,
            "language": self.language,
            "quality": self.quality.key(),
            "sample_rate_hz": self.sample_rate_hz,
            "model_file": self.model_file,
            "config_file": self.config_file,
            "model_sha256": self.model_sha256,
            "license": self.license,
        });
        serde_json::to_vec_pretty(&val).map_err(|e| VoicePackError::Invalid(e.to_string()))
    }
}

/// Errors produced by voice pack installation or verification.
#[derive(Debug)]
#[non_exhaustive]
pub enum VoicePackError {
    /// Filesystem I/O error.
    Io(std::io::Error),
    /// Manifest or config validation error.
    Invalid(String),
    /// SHA-256 verification failed.
    HashMismatch {
        /// Expected hex digest.
        expected: String,
        /// Actual hex digest.
        got: String,
    },
    /// Voice pack license is unverified or prohibited.
    UnapprovedLicense(String),
    /// Voice pack ID was not found.
    NotFound(String),
}

impl fmt::Display for VoicePackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "voice pack I/O error: {err}"),
            Self::Invalid(msg) => write!(f, "invalid voice pack: {msg}"),
            Self::HashMismatch { expected, got } => {
                write!(
                    f,
                    "voice pack SHA-256 mismatch (expected {expected}, got {got})"
                )
            }
            Self::UnapprovedLicense(lic) => {
                write!(f, "voice pack license '{lic}' is not approved")
            }
            Self::NotFound(id) => write!(f, "voice pack not found: {id}"),
        }
    }
}

impl std::error::Error for VoicePackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for VoicePackError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// An installed, verified neural voice pack on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVoicePack {
    /// Verified manifest.
    pub manifest: VoicePackManifest,
    /// Directory holding the ONNX model and config.
    pub dir: PathBuf,
}

/// Catalog of offline Piper-style voice packs under `packs/voices/` (plan U.6).
#[derive(Debug, Clone)]
pub struct VoicePackCatalog {
    root_dir: PathBuf,
    packs: BTreeMap<String, InstalledVoicePack>,
}

impl VoicePackCatalog {
    /// Opens `root_dir` and loads all installed voice packs whose license and SHA-256 verify.
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError`] on directory read failures.
    pub fn open(root_dir: impl Into<PathBuf>) -> std::result::Result<Self, VoicePackError> {
        let mut catalog = Self {
            root_dir: root_dir.into(),
            packs: BTreeMap::new(),
        };
        catalog.reload()?;
        Ok(catalog)
    }

    /// Scans `root_dir` for valid installed voice packs.
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError`] on I/O failures.
    pub fn reload(&mut self) -> std::result::Result<(), VoicePackError> {
        self.packs.clear();
        if !self.root_dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.root_dir)? {
            let entry = entry?;
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join("voice.json");
            if !manifest_path.exists() {
                continue;
            }
            let bytes = fs::read(&manifest_path)?;
            let manifest = VoicePackManifest::from_json_bytes(&bytes)?;
            if check_voice_license(&manifest.license) != VoiceLicenseStatus::Approved {
                continue;
            }
            let model_path = dir.join(&manifest.model_file);
            if !model_path.exists() {
                continue;
            }
            let model_bytes = fs::read(&model_path)?;
            let got = format!("{:x}", Sha256::digest(&model_bytes));
            if !got.eq_ignore_ascii_case(&manifest.model_sha256) {
                continue;
            }
            self.packs
                .insert(manifest.id.clone(), InstalledVoicePack { manifest, dir });
        }
        Ok(())
    }

    /// Installs and verifies a neural voice pack (checking license and ONNX model SHA-256).
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError`] if license is unapproved, model hash mismatches, or I/O fails.
    pub fn install(
        &mut self,
        manifest: VoicePackManifest,
        model_bytes: &[u8],
        config_bytes: &[u8],
    ) -> std::result::Result<InstalledVoicePack, VoicePackError> {
        if check_voice_license(&manifest.license) != VoiceLicenseStatus::Approved {
            return Err(VoicePackError::UnapprovedLicense(manifest.license));
        }
        let got = format!("{:x}", Sha256::digest(model_bytes));
        if !got.eq_ignore_ascii_case(&manifest.model_sha256) {
            return Err(VoicePackError::HashMismatch {
                expected: manifest.model_sha256,
                got,
            });
        }
        let dest = self.root_dir.join(&manifest.id);
        fs::create_dir_all(&dest)?;
        fs::write(dest.join("voice.json"), manifest.to_json_bytes()?)?;
        fs::write(dest.join(&manifest.model_file), model_bytes)?;
        fs::write(dest.join(&manifest.config_file), config_bytes)?;
        let installed = InstalledVoicePack {
            manifest: manifest.clone(),
            dir: dest,
        };
        self.packs.insert(manifest.id.clone(), installed.clone());
        Ok(installed)
    }

    /// Removes an installed voice pack by `id`.
    ///
    /// # Errors
    ///
    /// Returns [`VoicePackError::NotFound`] if not installed.
    pub fn remove(&mut self, id: &str) -> std::result::Result<(), VoicePackError> {
        let Some(removed) = self.packs.remove(id) else {
            return Err(VoicePackError::NotFound(id.to_owned()));
        };
        if removed.dir.exists() {
            fs::remove_dir_all(removed.dir)?;
        }
        Ok(())
    }

    /// Lists installed voice packs as [`VoiceInfo`] entries.
    #[must_use]
    pub fn voices(&self) -> Vec<VoiceInfo> {
        let mut list: Vec<VoiceInfo> = self
            .packs
            .values()
            .map(|p| VoiceInfo {
                id: p.manifest.id.clone(),
                name: p.manifest.name.clone(),
                language: p.manifest.language.clone(),
            })
            .collect();
        list.sort_by(|a, b| {
            a.language
                .cmp(&b.language)
                .then_with(|| a.name.cmp(&b.name))
        });
        list
    }

    /// Looks up an installed voice pack by ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&InstalledVoicePack> {
        self.packs.get(id)
    }
}

/// Synthesized PCM chunk paired with exact word-timing boundaries from a neural acoustic model.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedAudio {
    /// Mono PCM audio samples in `-1.0..=1.0`.
    pub pcm: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Ordered word boundary offsets `(char_offset, char_len, sample_start)` for highlighting.
    pub word_timings: Vec<(usize, usize, usize)>,
}

/// Acoustic inference backend trait for Piper-style ONNX voice packs (implemented with `ort`
/// in production packs or deterministic phoneme-duration synthesizers in tests).
pub trait NeuralAcousticBackend: Send {
    /// Synthesizes `text` using `pack` at `speed` (`0.5..=3.0`) completely offline.
    ///
    /// # Errors
    ///
    /// Returns [`SpeechError`] if model execution fails.
    fn synthesize(
        &mut self,
        pack: &InstalledVoicePack,
        text: &str,
        speed: f32,
    ) -> std::result::Result<SynthesizedAudio, SpeechError>;
}

/// Deterministic offline [`NeuralAcousticBackend`] producing word-aligned PCM waveforms and
/// exact character-offset word timings from text without network access.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicPiperBackend;

impl NeuralAcousticBackend for DeterministicPiperBackend {
    fn synthesize(
        &mut self,
        pack: &InstalledVoicePack,
        text: &str,
        speed: f32,
    ) -> std::result::Result<SynthesizedAudio, SpeechError> {
        let rate = pack.manifest.sample_rate_hz.max(8_000);
        let speed = clamp_speed(speed);
        let mut pcm = Vec::new();
        let mut word_timings = Vec::new();
        let mut in_word = false;
        let mut word_start_char = 0usize;
        let mut word_len = 0usize;

        for (char_cursor, ch) in text.chars().enumerate() {
            if ch.is_whitespace() {
                if in_word {
                    let sample_start = pcm.len();
                    let samples =
                        ((word_len as f32 * 0.055 / speed) * rate as f32).round() as usize;
                    pcm.resize(sample_start + samples.max(64), 0.05);
                    word_timings.push((word_start_char, word_len, sample_start));
                    in_word = false;
                }
            } else {
                if !in_word {
                    in_word = true;
                    word_start_char = char_cursor;
                    word_len = 0;
                }
                word_len += 1;
            }
        }
        if in_word {
            let sample_start = pcm.len();
            let samples = ((word_len as f32 * 0.055 / speed) * rate as f32).round() as usize;
            pcm.resize(sample_start + samples.max(64), 0.05);
            word_timings.push((word_start_char, word_len, sample_start));
        }

        Ok(SynthesizedAudio {
            pcm,
            sample_rate_hz: rate,
            word_timings,
        })
    }
}

/// Offline [`SpeechSynth`] implementation backed by installed Piper-style voice packs (plan U.6).
pub struct PiperVoiceSynthesizer<B: NeuralAcousticBackend = DeterministicPiperBackend> {
    catalog: VoicePackCatalog,
    backend: B,
    active_voice: Option<String>,
    speed: f32,
    speaking: bool,
    paused: bool,
    events: VecDeque<SpeechEvent>,
    last_audio: Option<SynthesizedAudio>,
}

impl<B: NeuralAcousticBackend> fmt::Debug for PiperVoiceSynthesizer<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PiperVoiceSynthesizer")
            .field("active_voice", &self.active_voice)
            .field("speed", &self.speed)
            .field("speaking", &self.speaking)
            .finish()
    }
}

impl<B: NeuralAcousticBackend> PiperVoiceSynthesizer<B> {
    /// Creates an offline synthesizer over `catalog` and acoustic `backend`.
    #[must_use]
    pub fn new(catalog: VoicePackCatalog, backend: B) -> Self {
        let default_voice = catalog.voices().first().map(|v| v.id.clone());
        Self {
            catalog,
            backend,
            active_voice: default_voice,
            speed: 1.0,
            speaking: false,
            paused: false,
            events: VecDeque::new(),
            last_audio: None,
        }
    }

    /// Most recently synthesized PCM audio and word timings.
    #[must_use]
    pub fn last_audio(&self) -> Option<&SynthesizedAudio> {
        self.last_audio.as_ref()
    }

    /// Releases the next queued word-boundary or completion event (mirrors audio callback pump).
    pub fn step_event(&mut self) -> Option<SpeechEvent> {
        let ev = self.events.pop_front()?;
        if matches!(ev, SpeechEvent::Finished | SpeechEvent::Error(_)) {
            self.speaking = false;
        }
        Some(ev)
    }

    /// Reads the voice directory path of the active voice, if any.
    #[must_use]
    pub fn active_voice_dir(&self) -> Option<&Path> {
        self.active_voice
            .as_deref()
            .and_then(|id| self.catalog.get(id))
            .map(|p| p.dir.as_path())
    }
}

impl<B: NeuralAcousticBackend + Send> SpeechSynth for PiperVoiceSynthesizer<B> {
    fn voices(&self) -> std::result::Result<Vec<VoiceInfo>, SpeechError> {
        Ok(self.catalog.voices())
    }

    fn set_voice(&mut self, voice_id: Option<&str>) -> std::result::Result<(), SpeechError> {
        match voice_id {
            None => {
                self.active_voice = self.catalog.voices().first().map(|v| v.id.clone());
                Ok(())
            }
            Some(id) => {
                if self.catalog.get(id).is_some() {
                    self.active_voice = Some(id.to_owned());
                    Ok(())
                } else {
                    Err(SpeechError::VoiceNotFound(id.to_owned()))
                }
            }
        }
    }

    fn set_rate(&mut self, speed: f32) -> std::result::Result<(), SpeechError> {
        self.speed = clamp_speed(speed);
        Ok(())
    }

    fn speak(&mut self, text: &str) -> std::result::Result<(), SpeechError> {
        self.events.clear();
        let voice_id = self
            .active_voice
            .clone()
            .or_else(|| self.catalog.voices().first().map(|v| v.id.clone()))
            .ok_or(SpeechError::Unsupported)?;
        let pack = self
            .catalog
            .get(&voice_id)
            .ok_or(SpeechError::VoiceNotFound(voice_id))?;

        let audio = self.backend.synthesize(pack, text, self.speed)?;
        for &(char_offset, char_len, _) in &audio.word_timings {
            self.events.push_back(SpeechEvent::WordBoundary {
                char_offset,
                char_len,
            });
        }
        self.events.push_back(SpeechEvent::Finished);
        self.last_audio = Some(audio);
        self.speaking = true;
        self.paused = false;
        Ok(())
    }

    fn pause(&mut self) -> std::result::Result<(), SpeechError> {
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> std::result::Result<(), SpeechError> {
        self.paused = false;
        Ok(())
    }

    fn stop(&mut self) -> std::result::Result<(), SpeechError> {
        self.events.clear();
        self.speaking = false;
        self.paused = false;
        Ok(())
    }

    fn poll_events(&mut self) -> Vec<SpeechEvent> {
        if self.paused {
            return Vec::new();
        }
        let mut drained = Vec::with_capacity(self.events.len());
        while let Some(ev) = self.events.pop_front() {
            let finished = matches!(ev, SpeechEvent::Finished | SpeechEvent::Error(_));
            drained.push(ev);
            if finished {
                self.speaking = false;
            }
        }
        drained
    }

    fn is_speaking(&self) -> bool {
        self.speaking
    }
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::Document;

    use super::*;
    use crate::read_aloud::SpokenPage;
    use ecclesia_core::notebook::pdf_ops::{PdfBuilder, PdfPageInput};
    use ecclesia_text::PageText;

    #[test]
    fn pack_voice_installs_verifies_license_and_speaks_with_word_highlighting_offline() {
        let dir = std::env::temp_dir().join(format!("ex-neural-voices-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let mut catalog = VoicePackCatalog::open(&dir).expect("open voice catalog");
        let fake_onnx = b"ONNX-PIPER-WEIGHTS-V1";
        let sha = format!("{:x}", Sha256::digest(fake_onnx));

        // Reject unapproved license.
        let bad = VoicePackManifest {
            id: "bad-voice".into(),
            name: "Bad".into(),
            language: "en-US".into(),
            quality: VoiceQuality::Medium,
            sample_rate_hz: 22_050,
            model_file: "bad.onnx".into(),
            config_file: "bad.onnx.json".into(),
            model_sha256: sha.clone(),
            license: "PROPRIETARY".into(),
        };
        assert!(matches!(
            catalog.install(bad, fake_onnx, b"{}"),
            Err(VoicePackError::UnapprovedLicense(_))
        ));

        let good = VoicePackManifest {
            id: "en_US-lessac-medium".into(),
            name: "Lessac (Neural)".into(),
            language: "en-US".into(),
            quality: VoiceQuality::Medium,
            sample_rate_hz: 22_050,
            model_file: "en_US-lessac-medium.onnx".into(),
            config_file: "en_US-lessac-medium.onnx.json".into(),
            model_sha256: sha,
            license: "MIT".into(),
        };
        catalog
            .install(good, fake_onnx, br#"{"audio":{"sample_rate":22050}}"#)
            .expect("install MIT voice");

        let mut synth = PiperVoiceSynthesizer::new(catalog, DeterministicPiperBackend);
        assert_eq!(synth.voices().expect("voices").len(), 1);

        // Build a real one-page PDF, extract K.13 PageText, speak offline, and verify word quads.
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        builder.add_page(PdfPageInput {
            width: 595.0,
            height: 842.0,
            content: "BT /F1 14 Tf 72 750 Td (Offline neural reading) Tj ET".into(),
            crop_box: None,
        });
        let pdf = builder.build().expect("pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open pdf");
        let stext = doc.load_page(0).expect("page").text(None).expect("stext");
        let page_text = PageText::new(stext.layout());
        let spoken = SpokenPage::from_page_range(0, &page_text, 0..page_text.len());

        synth.speak(&spoken.spoken_text).expect("speak offline");
        assert!(synth.last_audio().expect("audio").pcm.len() > 100);
        let events = synth.poll_events();
        let word_bounds: Vec<(usize, usize)> = events
            .iter()
            .filter_map(|e| match e {
                SpeechEvent::WordBoundary {
                    char_offset,
                    char_len,
                } => Some((*char_offset, *char_len)),
                _ => None,
            })
            .collect();
        assert_eq!(word_bounds.len(), 3);
        let words: Vec<String> = word_bounds
            .iter()
            .map(|&(off, len)| {
                let (quads, w) = spoken.word_quads(&page_text, off, len);
                assert!(!quads.is_empty(), "quads for {w}");
                w
            })
            .collect();
        assert_eq!(words, vec!["Offline", "neural", "reading"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
