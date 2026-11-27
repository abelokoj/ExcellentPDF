//! Text-to-speech platform abstraction (plan U.5, K.19): a mockable [`SpeechSynth`] trait with word-boundary events,
//! voice selection, speed control and a native Windows SAPI 5 implementation.
//!
//! Callers in `ecclesia-features::read_aloud` depend only on [`SpeechSynth`], [`VoiceInfo`], [`SpeechEvent`] and
//! [`SpeechError`], keeping reading-order text mapping and playback state pure and testable with [`MockSpeechSynth`].

use std::collections::VecDeque;
use std::fmt;

/// Minimum supported playback speed multiplier (matching SumatraPDF Enhanced).
pub const MIN_SPEED: f32 = 0.5;

/// Maximum supported playback speed multiplier (matching SumatraPDF Enhanced).
pub const MAX_SPEED: f32 = 3.0;

/// Metadata describing an installed text-to-speech voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceInfo {
    /// Platform-specific stable voice identifier.
    pub id: String,
    /// Human-readable voice display name.
    pub name: String,
    /// BCP-47 or LCID language tag (may be empty if the engine reports none).
    pub language: String,
}

/// An asynchronous event emitted by a [`SpeechSynth`] backend during playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechEvent {
    /// The synthesizer reached the word at `char_offset` (in Unicode scalar values of the spoken text) spanning
    /// `char_len` characters.
    WordBoundary {
        /// Character offset into the current spoken utterance.
        char_offset: usize,
        /// Length of the word in characters (`0` if the backend only reports start offsets).
        char_len: usize,
    },
    /// The current utterance finished playing.
    Finished,
    /// The backend reported a playback error.
    Error(String),
}

/// Errors produced by a text-to-speech backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpeechError {
    /// No text-to-speech backend is available on this platform.
    Unsupported,
    /// The requested voice identifier was not found.
    VoiceNotFound(String),
    /// The OS speech API returned an error.
    Backend(String),
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("text-to-speech is not supported on this platform"),
            Self::VoiceNotFound(id) => write!(f, "speech voice not found: {id}"),
            Self::Backend(msg) => write!(f, "speech backend error: {msg}"),
        }
    }
}

impl std::error::Error for SpeechError {}

/// Platform text-to-speech synthesizer interface (plan K.19, U.5).
pub trait SpeechSynth: Send {
    /// Lists the available voices on this system, sorted by language then name.
    ///
    /// # Errors
    ///
    /// The platform speech subsystem could not be queried.
    fn voices(&self) -> Result<Vec<VoiceInfo>, SpeechError>;

    /// Selects a voice by [`VoiceInfo::id`], or restores the system default when `voice_id` is `None`.
    ///
    /// # Errors
    ///
    /// The voice does not exist or the backend rejected the selection.
    fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError>;

    /// Sets the playback speed multiplier, clamped to `[MIN_SPEED, MAX_SPEED]`.
    ///
    /// # Errors
    ///
    /// The backend rejected the rate change.
    fn set_rate(&mut self, speed: f32) -> Result<(), SpeechError>;

    /// Begins speaking `text` asynchronously, cancelling any previous utterance.
    ///
    /// # Errors
    ///
    /// The backend failed to start synthesis.
    fn speak(&mut self, text: &str) -> Result<(), SpeechError>;

    /// Pauses active playback.
    ///
    /// # Errors
    ///
    /// The backend failed to pause.
    fn pause(&mut self) -> Result<(), SpeechError>;

    /// Resumes paused playback.
    ///
    /// # Errors
    ///
    /// The backend failed to resume.
    fn resume(&mut self) -> Result<(), SpeechError>;

    /// Stops playback and discards any pending events.
    ///
    /// # Errors
    ///
    /// The backend failed to stop.
    fn stop(&mut self) -> Result<(), SpeechError>;

    /// Drains any queued word-boundary or completion events since the last poll.
    fn poll_events(&mut self) -> Vec<SpeechEvent>;

    /// Whether the synthesizer is currently speaking (not stopped or finished).
    fn is_speaking(&self) -> bool;
}

/// Clamps `speed` into `[MIN_SPEED, MAX_SPEED]`, replacing non-finite values with `1.0`.
#[must_use]
pub fn clamp_speed(speed: f32) -> f32 {
    if !speed.is_finite() {
        return 1.0;
    }
    speed.clamp(MIN_SPEED, MAX_SPEED)
}

/// Converts a playback speed multiplier in `[0.5, 3.0]` to a SAPI 5 rate in `[-10, 10]`
/// (`round(10 * log3(speed))`, matching SumatraPDF Enhanced).
#[must_use]
pub fn speed_to_sapi_rate(speed: f32) -> i32 {
    let clamped = f64::from(clamp_speed(speed));
    let rate = 10.0 * clamped.ln() / 3.0_f64.ln();
    let rounded = rate.round() as i32;
    rounded.clamp(-10, 10)
}

/// Deterministic in-memory [`SpeechSynth`] for automated tests and headless environments.
///
/// Each call to [`SpeechSynth::speak`] splits the utterance into word-boundary events; call [`Self::step`] or
/// [`Self::finish_utterance`] to release events at a controlled pace.
#[derive(Debug, Clone)]
pub struct MockSpeechSynth {
    available_voices: Vec<VoiceInfo>,
    selected_voice: Option<String>,
    speed: f32,
    speaking: bool,
    paused: bool,
    spoken_history: Vec<String>,
    scheduled: VecDeque<SpeechEvent>,
    ready: VecDeque<SpeechEvent>,
}

impl Default for MockSpeechSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSpeechSynth {
    /// Creates a mock synthesizer with two default English voices.
    #[must_use]
    pub fn new() -> Self {
        Self {
            available_voices: vec![
                VoiceInfo {
                    id: "mock-en-us".to_owned(),
                    name: "Mock US English".to_owned(),
                    language: "en-US".to_owned(),
                },
                VoiceInfo {
                    id: "mock-en-gb".to_owned(),
                    name: "Mock UK English".to_owned(),
                    language: "en-GB".to_owned(),
                },
            ],
            selected_voice: None,
            speed: 1.0,
            speaking: false,
            paused: false,
            spoken_history: Vec::new(),
            scheduled: VecDeque::new(),
            ready: VecDeque::new(),
        }
    }

    /// Releases the next scheduled event into the poll queue, returning whether one existed.
    pub fn step(&mut self) -> bool {
        if self.paused {
            return false;
        }
        let Some(event) = self.scheduled.pop_front() else {
            return false;
        };
        if matches!(event, SpeechEvent::Finished | SpeechEvent::Error(_)) {
            self.speaking = false;
        }
        self.ready.push_back(event);
        true
    }

    /// Releases all remaining scheduled events for the current utterance.
    pub fn finish_utterance(&mut self) {
        while self.step() {}
    }

    /// Every utterance passed to [`SpeechSynth::speak`], in order.
    #[must_use]
    pub fn spoken_history(&self) -> &[String] {
        &self.spoken_history
    }

    /// Current selected voice id.
    #[must_use]
    pub fn selected_voice(&self) -> Option<&str> {
        self.selected_voice.as_deref()
    }

    /// Current clamped speed multiplier.
    #[must_use]
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Whether playback is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

impl SpeechSynth for MockSpeechSynth {
    fn voices(&self) -> Result<Vec<VoiceInfo>, SpeechError> {
        Ok(self.available_voices.clone())
    }

    fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError> {
        match voice_id {
            None => {
                self.selected_voice = None;
                Ok(())
            }
            Some(id) => {
                if self.available_voices.iter().any(|v| v.id == id) {
                    self.selected_voice = Some(id.to_owned());
                    Ok(())
                } else {
                    Err(SpeechError::VoiceNotFound(id.to_owned()))
                }
            }
        }
    }

    fn set_rate(&mut self, speed: f32) -> Result<(), SpeechError> {
        self.speed = clamp_speed(speed);
        Ok(())
    }

    fn speak(&mut self, text: &str) -> Result<(), SpeechError> {
        self.scheduled.clear();
        self.ready.clear();
        self.paused = false;
        self.spoken_history.push(text.to_owned());
        if text.trim().is_empty() {
            self.speaking = false;
            self.ready.push_back(SpeechEvent::Finished);
            return Ok(());
        }
        self.speaking = true;
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            if i > start {
                self.scheduled.push_back(SpeechEvent::WordBoundary {
                    char_offset: start,
                    char_len: i - start,
                });
            }
        }
        self.scheduled.push_back(SpeechEvent::Finished);
        Ok(())
    }

    fn pause(&mut self) -> Result<(), SpeechError> {
        if self.speaking {
            self.paused = true;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), SpeechError> {
        self.paused = false;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SpeechError> {
        self.speaking = false;
        self.paused = false;
        self.scheduled.clear();
        self.ready.clear();
        Ok(())
    }

    fn poll_events(&mut self) -> Vec<SpeechEvent> {
        self.ready.drain(..).collect()
    }

    fn is_speaking(&self) -> bool {
        self.speaking
    }
}

#[cfg(windows)]
pub use imp::WindowsSpeechSynth;

/// Opens the system's native text-to-speech synthesizer.
///
/// # Errors
///
/// Returns [`SpeechError::Unsupported`] on platforms without a native implementation yet, or [`SpeechError::Backend`]
/// if the OS speech service fails to initialize.
pub fn system_synth() -> Result<Box<dyn SpeechSynth>, SpeechError> {
    imp::system_synth()
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    };
    use windows::core::{GUID, HRESULT, IUnknown, Interface};

    use super::{
        SpeechError, SpeechEvent, SpeechSynth, VoiceInfo, clamp_speed, speed_to_sapi_rate,
    };

    const CLSID_SP_VOICE: GUID = GUID::from_u128(0x9674_9377_3391_11d2_9ee3_00c0_4f79_7396);
    const IID_ISP_VOICE: GUID = GUID::from_u128(0x6c44_df74_72b9_4992_a1ec_ef99_6e04_22d4);
    const IID_ISP_EVENT_SOURCE: GUID = GUID::from_u128(0xbe7a_9cce_5f9e_11d2_960f_00c0_4f8e_e628);
    const CLSID_SP_OBJECT_TOKEN_CATEGORY: GUID =
        GUID::from_u128(0xa910_187f_0c7a_45ac_92cc_59ed_afb7_7b53);
    const IID_ISP_OBJECT_TOKEN_CATEGORY: GUID =
        GUID::from_u128(0x2d3d_3845_39af_4850_bbf9_40b4_9780_011d);

    const SPF_ASYNC: u32 = 1 << 0;
    const SPF_PURGEBEFORESPEAK: u32 = 1 << 1;
    const SPF_IS_NOT_XML: u32 = 1 << 4;

    const SPEI_END_INPUT_STREAM: u16 = 2;
    const SPEI_WORD_BOUNDARY: u16 = 5;
    const SPFEI_FLAGCHECK: u64 = (1_u64 << 34) | (1_u64 << 35);

    const SPET_LPARAM_IS_OBJECT: u16 = 1;
    const SPET_LPARAM_IS_POINTER: u16 = 2;
    const SPET_LPARAM_IS_STRING: u16 = 3;
    const SPET_LPARAM_IS_TOKEN: u16 = 4;

    const S_OK: i32 = 0;

    #[repr(C)]
    struct IUnknownVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct SpEvent {
        event_id: u16,
        param_type: u16,
        stream_num: u32,
        audio_stream_offset: u64,
        wparam: usize,
        lparam: isize,
    }

    // ISpEventSource vtable: IUnknown(0..3), ISpNotifySource(3..10), ISpEventSource(10..13)
    #[repr(C)]
    struct ISpEventSourceVtbl {
        unknown: IUnknownVtbl,
        _set_notify_sink: *const c_void,
        _set_notify_window_message: *const c_void,
        _set_notify_callback_function: *const c_void,
        _set_notify_callback_interface: *const c_void,
        _set_notify_win32_event: *const c_void,
        _wait_for_notify_event: *const c_void,
        _get_notify_event_handle: *const c_void,
        set_interest: unsafe extern "system" fn(*mut c_void, u64, u64) -> HRESULT,
        get_events: unsafe extern "system" fn(*mut c_void, u32, *mut SpEvent, *mut u32) -> HRESULT,
        _get_info: *const c_void,
    }

    // ISpVoice vtable: IUnknown(0..3), ISpNotifySource(3..10), ISpEventSource(10..13), ISpVoice(13..)
    #[repr(C)]
    struct ISpVoiceVtbl {
        event_source: ISpEventSourceVtbl,
        _set_output: *const c_void,
        _get_output_object_token: *const c_void,
        _get_output_stream: *const c_void,
        pause: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        resume: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        set_voice: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        _get_voice: *const c_void,
        speak: unsafe extern "system" fn(*mut c_void, *const u16, u32, *mut u32) -> HRESULT,
        _speak_stream: *const c_void,
        _get_status: *const c_void,
        _skip: *const c_void,
        _set_priority: *const c_void,
        _get_priority: *const c_void,
        _set_alert_boundary: *const c_void,
        _get_alert_boundary: *const c_void,
        set_rate: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    }

    // ISpDataKey vtable: IUnknown(0..3), SetData(3), GetData(4), SetStringValue(5), GetStringValue(6), ...
    #[repr(C)]
    struct ISpDataKeyVtbl {
        unknown: IUnknownVtbl,
        _set_data: *const c_void,
        _get_data: *const c_void,
        _set_string_value: *const c_void,
        get_string_value:
            unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut u16) -> HRESULT,
        _set_dword: *const c_void,
        _get_dword: *const c_void,
        open_key: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
        _create_key: *const c_void,
        _delete_key: *const c_void,
        _delete_value: *const c_void,
        _enum_keys: *const c_void,
        _enum_values: *const c_void,
    }

    // ISpObjectToken vtable: ISpDataKey(0..15), SetId(15), GetId(16), ...
    #[repr(C)]
    struct ISpObjectTokenVtbl {
        data_key: ISpDataKeyVtbl,
        _set_id: *const c_void,
        get_id: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HRESULT,
    }

    // ISpObjectTokenCategory vtable: ISpDataKey(0..15), SetId(15), GetId(16), GetDataKey(17), EnumTokens(18)
    #[repr(C)]
    struct ISpObjectTokenCategoryVtbl {
        data_key: ISpDataKeyVtbl,
        set_id: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
        _get_id: *const c_void,
        _get_data_key: *const c_void,
        enum_tokens: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            *const u16,
            *mut *mut c_void,
        ) -> HRESULT,
    }

    // IEnumSpObjectTokens vtable: IUnknown(0..3), Next(3), ...
    #[repr(C)]
    struct IEnumSpObjectTokensVtbl {
        unknown: IUnknownVtbl,
        next: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> HRESULT,
    }

    /// RAII wrapper around a raw COM interface pointer. Strictly `!Send`: created, invoked and released exclusively
    /// on the dedicated STA speech thread.
    struct ComPtr(*mut c_void);

    impl Drop for ComPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` is a valid COM interface pointer whose vtable begins with `IUnknownVtbl`, dropped
                // on the STA thread that created it.
                unsafe {
                    let vtbl = *(self.0 as *const *const IUnknownVtbl);
                    ((*vtbl).release)(self.0);
                }
            }
        }
    }

    enum Command {
        Voices(mpsc::SyncSender<Result<Vec<VoiceInfo>, SpeechError>>),
        SetVoice(Option<String>, mpsc::SyncSender<Result<(), SpeechError>>),
        SetRate(f32, mpsc::SyncSender<Result<(), SpeechError>>),
        Speak(String, mpsc::SyncSender<Result<(), SpeechError>>),
        Pause(mpsc::SyncSender<Result<(), SpeechError>>),
        Resume(mpsc::SyncSender<Result<(), SpeechError>>),
        Stop(mpsc::SyncSender<Result<(), SpeechError>>),
        PollEvents(mpsc::SyncSender<Vec<SpeechEvent>>),
        Shutdown,
    }

    /// Thread-affine SAPI 5 state living exclusively on its dedicated STA worker thread.
    struct StaVoice {
        voice: ComPtr,
        event_source: Option<ComPtr>,
        co_initialized: bool,
        stream_num: u32,
        speaking: Arc<AtomicBool>,
        spoken_utf16_to_char: Vec<usize>,
    }

    impl StaVoice {
        fn new(speaking: Arc<AtomicBool>) -> Result<Self, SpeechError> {
            // SAFETY: Standard Win32 STA COM initialization on the dedicated speech worker thread.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let co_initialized = if hr.is_ok() {
                true
            } else if hr == RPC_E_CHANGED_MODE {
                false
            } else {
                return Err(SpeechError::Backend(format!(
                    "CoInitializeEx failed: 0x{:08x}",
                    hr.0
                )));
            };

            let voice = match co_create_raw(&CLSID_SP_VOICE, &IID_ISP_VOICE) {
                Ok(v) => v,
                Err(err) => {
                    if co_initialized {
                        // SAFETY: Balances `CoInitializeEx` on this same STA thread.
                        unsafe { CoUninitialize() };
                    }
                    return Err(err);
                }
            };

            let mut raw_events: *mut c_void = ptr::null_mut();
            // SAFETY: Querying `IID_ISpEventSource` on the valid `ISpVoice` COM pointer on this STA thread.
            let qhr = unsafe {
                let vtbl = *(voice.0 as *const *const IUnknownVtbl);
                ((*vtbl).query_interface)(voice.0, &IID_ISP_EVENT_SOURCE, &raw mut raw_events)
            };
            let event_source = if qhr.is_ok() && !raw_events.is_null() {
                let src = ComPtr(raw_events);
                let interest = (1_u64 << SPEI_END_INPUT_STREAM)
                    | (1_u64 << SPEI_WORD_BOUNDARY)
                    | SPFEI_FLAGCHECK;
                // SAFETY: Calling `ISpEventSource::SetInterest` on the queried `ISpEventSource` pointer.
                unsafe {
                    let vtbl = *(src.0 as *const *const ISpEventSourceVtbl);
                    let _ = ((*vtbl).set_interest)(src.0, interest, interest);
                }
                Some(src)
            } else {
                None
            };

            Ok(Self {
                voice,
                event_source,
                co_initialized,
                stream_num: 0,
                speaking,
                spoken_utf16_to_char: Vec::new(),
            })
        }

        fn utf16_offset_to_char(&self, utf16_pos: usize) -> usize {
            if self.spoken_utf16_to_char.is_empty() {
                return utf16_pos;
            }
            self.spoken_utf16_to_char
                .get(utf16_pos)
                .copied()
                .unwrap_or_else(|| *self.spoken_utf16_to_char.last().unwrap_or(&0))
        }

        fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError> {
            let token = match voice_id {
                None => None,
                Some(id) => Some(find_sapi_voice_token(id)?),
            };
            let ptr = token.as_ref().map_or(ptr::null_mut(), |t| t.0);
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread; `ptr` is null or a valid `ISpObjectToken`.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).set_voice)(self.voice.0, ptr)
            };
            if hr.is_err() {
                return Err(SpeechError::Backend(format!(
                    "SetVoice failed: 0x{:08x}",
                    hr.0
                )));
            }
            Ok(())
        }

        fn set_rate(&mut self, speed: f32) -> Result<(), SpeechError> {
            let rate = speed_to_sapi_rate(clamp_speed(speed));
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).set_rate)(self.voice.0, rate)
            };
            if hr.is_err() {
                return Err(SpeechError::Backend(format!(
                    "SetRate failed: 0x{:08x}",
                    hr.0
                )));
            }
            Ok(())
        }

        fn speak(&mut self, text: &str) -> Result<(), SpeechError> {
            self.spoken_utf16_to_char.clear();
            let mut wide = Vec::with_capacity(text.len() + 1);
            for (char_idx, ch) in text.chars().enumerate() {
                let mut buf = [0_u16; 2];
                for unit in ch.encode_utf16(&mut buf).iter().copied() {
                    wide.push(unit);
                    self.spoken_utf16_to_char.push(char_idx);
                }
            }
            self.spoken_utf16_to_char.push(text.chars().count());
            wide.push(0);

            let mut stream_num = 0_u32;
            let flags = SPF_ASYNC | SPF_PURGEBEFORESPEAK | SPF_IS_NOT_XML;
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread and `wide` is NUL-terminated UTF-16.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).speak)(self.voice.0, wide.as_ptr(), flags, &raw mut stream_num)
            };
            if hr.is_err() {
                return Err(SpeechError::Backend(format!(
                    "Speak failed: 0x{:08x}",
                    hr.0
                )));
            }
            self.stream_num = stream_num;
            self.speaking
                .store(!text.trim().is_empty(), Ordering::Relaxed);
            Ok(())
        }

        fn pause(&mut self) -> Result<(), SpeechError> {
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).pause)(self.voice.0)
            };
            if hr.is_err() {
                return Err(SpeechError::Backend(format!(
                    "Pause failed: 0x{:08x}",
                    hr.0
                )));
            }
            Ok(())
        }

        fn resume(&mut self) -> Result<(), SpeechError> {
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).resume)(self.voice.0)
            };
            if hr.is_err() {
                return Err(SpeechError::Backend(format!(
                    "Resume failed: 0x{:08x}",
                    hr.0
                )));
            }
            Ok(())
        }

        fn stop(&mut self) -> Result<(), SpeechError> {
            let flags = SPF_ASYNC | SPF_PURGEBEFORESPEAK;
            // SAFETY: `self.voice.0` is a valid `ISpVoice` on this STA thread; NULL text purges playback.
            let hr = unsafe {
                let vtbl = *(self.voice.0 as *const *const ISpVoiceVtbl);
                ((*vtbl).speak)(self.voice.0, ptr::null(), flags, ptr::null_mut())
            };
            self.speaking.store(false, Ordering::Relaxed);
            self.stream_num = 0;
            if hr.is_err() {
                return Err(SpeechError::Backend(format!("Stop failed: 0x{:08x}", hr.0)));
            }
            Ok(())
        }

        fn poll_events(&mut self) -> Vec<SpeechEvent> {
            let mut out = Vec::new();
            let Some(src) = self.event_source.as_ref() else {
                return out;
            };
            loop {
                let mut ev = SpEvent {
                    event_id: 0,
                    param_type: 0,
                    stream_num: 0,
                    audio_stream_offset: 0,
                    wparam: 0,
                    lparam: 0,
                };
                let mut fetched = 0_u32;
                // SAFETY: `src.0` is a valid `ISpEventSource` on this STA thread; `ev` and `fetched` are valid out-pointers.
                let hr = unsafe {
                    let vtbl = *(src.0 as *const *const ISpEventSourceVtbl);
                    ((*vtbl).get_events)(src.0, 1, &raw mut ev, &raw mut fetched)
                };
                if hr.0 != S_OK || fetched == 0 {
                    break;
                }
                if ev.stream_num == self.stream_num {
                    if ev.event_id == SPEI_WORD_BOUNDARY {
                        let utf16_start = usize::try_from(ev.lparam).unwrap_or(0);
                        let utf16_len = ev.wparam;
                        let start = self.utf16_offset_to_char(utf16_start);
                        let end = self.utf16_offset_to_char(utf16_start.saturating_add(utf16_len));
                        out.push(SpeechEvent::WordBoundary {
                            char_offset: start,
                            char_len: end.saturating_sub(start),
                        });
                    } else if ev.event_id == SPEI_END_INPUT_STREAM {
                        self.speaking.store(false, Ordering::Relaxed);
                        out.push(SpeechEvent::Finished);
                    }
                }
                clear_sp_event(&ev);
            }
            out
        }
    }

    impl Drop for StaVoice {
        fn drop(&mut self) {
            let _ = self.stop();
            // Release all COM pointers before `CoUninitialize` on this same STA thread.
            self.event_source = None;
            let voice_ptr = std::mem::replace(&mut self.voice.0, ptr::null_mut());
            if !voice_ptr.is_null() {
                // SAFETY: `voice_ptr` was owned by `self.voice` on this STA thread and has not been released yet.
                unsafe {
                    let vtbl = *(voice_ptr as *const *const IUnknownVtbl);
                    ((*vtbl).release)(voice_ptr);
                }
            }
            if self.co_initialized {
                // SAFETY: Balances `CoInitializeEx` on this exact same STA thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Native Windows SAPI 5 text-to-speech synthesizer backed by a dedicated STA COM worker thread.
    ///
    /// All COM initialization (`CoInitializeEx`), SAPI calls, pointer releases and `CoUninitialize` execute strictly on
    /// the dedicated STA thread, making `WindowsSpeechSynth` soundly `Send` without any `unsafe impl Send`.
    pub struct WindowsSpeechSynth {
        tx: mpsc::Sender<Command>,
        worker: Option<JoinHandle<()>>,
        speaking: Arc<AtomicBool>,
    }

    const _: () = {
        const fn assert_send<T: Send>() {}
        assert_send::<WindowsSpeechSynth>();
    };

    impl std::fmt::Debug for WindowsSpeechSynth {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("WindowsSpeechSynth")
                .field("speaking", &self.speaking.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl WindowsSpeechSynth {
        /// Spawns a dedicated STA COM thread and initializes a SAPI 5 `SpVoice` with word-boundary and end-of-stream
        /// notifications.
        ///
        /// # Errors
        ///
        /// Returns [`SpeechError::Backend`] if the worker thread cannot spawn or COM / `SpVoice` initialization fails.
        pub fn new() -> Result<Self, SpeechError> {
            let (tx, rx) = mpsc::channel::<Command>();
            let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), SpeechError>>(1);
            let speaking = Arc::new(AtomicBool::new(false));
            let worker_speaking = Arc::clone(&speaking);

            let handle = thread::Builder::new()
                .name("ecclesia-speech-sta".to_owned())
                .spawn(move || {
                    let mut sta = match StaVoice::new(worker_speaking) {
                        Ok(voice) => {
                            let _ = ready_tx.send(Ok(()));
                            voice
                        }
                        Err(err) => {
                            let _ = ready_tx.send(Err(err));
                            return;
                        }
                    };
                    for cmd in rx {
                        match cmd {
                            Command::Voices(reply) => {
                                let _ = reply.send(enumerate_sapi_voices());
                            }
                            Command::SetVoice(id, reply) => {
                                let _ = reply.send(sta.set_voice(id.as_deref()));
                            }
                            Command::SetRate(speed, reply) => {
                                let _ = reply.send(sta.set_rate(speed));
                            }
                            Command::Speak(text, reply) => {
                                let _ = reply.send(sta.speak(&text));
                            }
                            Command::Pause(reply) => {
                                let _ = reply.send(sta.pause());
                            }
                            Command::Resume(reply) => {
                                let _ = reply.send(sta.resume());
                            }
                            Command::Stop(reply) => {
                                let _ = reply.send(sta.stop());
                            }
                            Command::PollEvents(reply) => {
                                let _ = reply.send(sta.poll_events());
                            }
                            Command::Shutdown => break,
                        }
                    }
                    // `sta` drops here on the STA thread, releasing COM pointers before `CoUninitialize`.
                })
                .map_err(|e| SpeechError::Backend(format!("failed to spawn speech thread: {e}")))?;

            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    tx,
                    worker: Some(handle),
                    speaking,
                }),
                Ok(Err(err)) => {
                    let _ = handle.join();
                    Err(err)
                }
                Err(_) => {
                    let _ = handle.join();
                    Err(SpeechError::Backend(
                        "speech worker exited before initializing".to_owned(),
                    ))
                }
            }
        }

        fn call<T>(
            &self,
            build: impl FnOnce(mpsc::SyncSender<Result<T, SpeechError>>) -> Command,
        ) -> Result<T, SpeechError> {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            self.tx
                .send(build(reply_tx))
                .map_err(|_| SpeechError::Backend("speech worker stopped".to_owned()))?;
            reply_rx
                .recv()
                .map_err(|_| SpeechError::Backend("speech worker disconnected".to_owned()))?
        }
    }

    impl Drop for WindowsSpeechSynth {
        fn drop(&mut self) {
            let _ = self.tx.send(Command::Shutdown);
            if let Some(handle) = self.worker.take() {
                let _ = handle.join();
            }
        }
    }

    impl SpeechSynth for WindowsSpeechSynth {
        fn voices(&self) -> Result<Vec<VoiceInfo>, SpeechError> {
            self.call(Command::Voices)
        }

        fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError> {
            let owned = voice_id.map(str::to_owned);
            self.call(|reply| Command::SetVoice(owned, reply))
        }

        fn set_rate(&mut self, speed: f32) -> Result<(), SpeechError> {
            self.call(|reply| Command::SetRate(speed, reply))
        }

        fn speak(&mut self, text: &str) -> Result<(), SpeechError> {
            let owned = text.to_owned();
            self.call(|reply| Command::Speak(owned, reply))
        }

        fn pause(&mut self) -> Result<(), SpeechError> {
            self.call(Command::Pause)
        }

        fn resume(&mut self) -> Result<(), SpeechError> {
            self.call(Command::Resume)
        }

        fn stop(&mut self) -> Result<(), SpeechError> {
            self.call(Command::Stop)
        }

        fn poll_events(&mut self) -> Vec<SpeechEvent> {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            if self.tx.send(Command::PollEvents(reply_tx)).is_err() {
                return Vec::new();
            }
            reply_rx.recv().unwrap_or_default()
        }

        fn is_speaking(&self) -> bool {
            self.speaking.load(Ordering::Relaxed)
        }
    }

    fn clear_sp_event(ev: &SpEvent) {
        if ev.lparam == 0 {
            return;
        }
        match ev.param_type {
            SPET_LPARAM_IS_TOKEN | SPET_LPARAM_IS_OBJECT => {
                let ptr = ev.lparam as *mut c_void;
                // SAFETY: SAPI event with object/token param type owns one COM reference on `lparam`.
                unsafe {
                    let vtbl = *(ptr as *const *const IUnknownVtbl);
                    ((*vtbl).release)(ptr);
                }
            }
            SPET_LPARAM_IS_POINTER | SPET_LPARAM_IS_STRING => {
                // SAFETY: SAPI event with pointer/string param type was allocated with `CoTaskMemAlloc`.
                unsafe { CoTaskMemFree(Some(ev.lparam as *const c_void)) };
            }
            _ => {}
        }
    }

    fn wide_nul(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn take_co_string(ptr: *mut u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: `ptr` is a NUL-terminated UTF-16 string allocated by COM (`CoTaskMemAlloc`).
        let text = unsafe {
            let mut len = 0_usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            String::from_utf16_lossy(slice)
        };
        // SAFETY: `ptr` was allocated via `CoTaskMemAlloc` by SAPI.
        unsafe { CoTaskMemFree(Some(ptr.cast())) };
        text
    }

    fn co_create_raw(clsid: &GUID, iid: &GUID) -> Result<ComPtr, SpeechError> {
        // SAFETY: Creating a standard SAPI COM object via `CoCreateInstance` returning `IUnknown`.
        let unk: IUnknown = unsafe { CoCreateInstance(clsid, None, CLSCTX_ALL) }
            .map_err(|e| SpeechError::Backend(format!("CoCreateInstance failed: {e}")))?;
        let unk_raw = unk.as_raw();
        let mut out: *mut c_void = ptr::null_mut();
        // SAFETY: `unk_raw` is a live `IUnknown` COM pointer; querying `iid` stores a new owned reference in `out`.
        let hr = unsafe {
            let vtbl = *(unk_raw as *const *const IUnknownVtbl);
            ((*vtbl).query_interface)(unk_raw, iid, &raw mut out)
        };
        if hr.is_err() || out.is_null() {
            return Err(SpeechError::Backend(format!(
                "QueryInterface failed: 0x{:08x}",
                hr.0
            )));
        }
        Ok(ComPtr(out))
    }

    fn open_voice_enum() -> Result<ComPtr, SpeechError> {
        let cat = co_create_raw(
            &CLSID_SP_OBJECT_TOKEN_CATEGORY,
            &IID_ISP_OBJECT_TOKEN_CATEGORY,
        )?;
        let spcat_voices = wide_nul("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices");
        // SAFETY: `cat.0` is a valid `ISpObjectTokenCategory` and `spcat_voices` is NUL-terminated UTF-16.
        let hr = unsafe {
            let vtbl = *(cat.0 as *const *const ISpObjectTokenCategoryVtbl);
            ((*vtbl).set_id)(cat.0, spcat_voices.as_ptr(), 0)
        };
        if hr.is_err() {
            return Err(SpeechError::Backend(format!(
                "SetId failed: 0x{:08x}",
                hr.0
            )));
        }
        let mut raw_enum: *mut c_void = ptr::null_mut();
        // SAFETY: Enumerating all voice tokens without attribute filters.
        let hr = unsafe {
            let vtbl = *(cat.0 as *const *const ISpObjectTokenCategoryVtbl);
            ((*vtbl).enum_tokens)(cat.0, ptr::null(), ptr::null(), &raw mut raw_enum)
        };
        if hr.is_err() || raw_enum.is_null() {
            return Err(SpeechError::Backend(format!(
                "EnumTokens failed: 0x{:08x}",
                hr.0
            )));
        }
        Ok(ComPtr(raw_enum))
    }

    fn enumerate_sapi_voices() -> Result<Vec<VoiceInfo>, SpeechError> {
        let enumerator = open_voice_enum()?;
        let mut voices = Vec::new();
        let attr_key_name = wide_nul("Attributes");
        let lang_val_name = wide_nul("Language");
        loop {
            let mut raw_token: *mut c_void = ptr::null_mut();
            let mut fetched = 0_u32;
            // SAFETY: `enumerator.0` is a valid `IEnumSpObjectTokens`.
            let hr = unsafe {
                let vtbl = *(enumerator.0 as *const *const IEnumSpObjectTokensVtbl);
                ((*vtbl).next)(enumerator.0, 1, &raw mut raw_token, &raw mut fetched)
            };
            if hr.0 != S_OK || fetched == 0 || raw_token.is_null() {
                break;
            }
            let token = ComPtr(raw_token);
            let mut id_ptr: *mut u16 = ptr::null_mut();
            let mut name_ptr: *mut u16 = ptr::null_mut();
            // SAFETY: `token.0` is a valid `ISpObjectToken` (inheriting `ISpDataKey`).
            let (id_hr, name_hr) = unsafe {
                let vtbl = *(token.0 as *const *const ISpObjectTokenVtbl);
                let id_hr = ((*vtbl).get_id)(token.0, &raw mut id_ptr);
                let name_hr =
                    ((*vtbl).data_key.get_string_value)(token.0, ptr::null(), &raw mut name_ptr);
                (id_hr, name_hr)
            };
            let id = take_co_string(id_ptr);
            let name = take_co_string(name_ptr);
            let mut language = String::new();
            let mut raw_attrs: *mut c_void = ptr::null_mut();
            // SAFETY: Opening the `Attributes` sub-key on `token`.
            let attr_hr = unsafe {
                let vtbl = *(token.0 as *const *const ISpDataKeyVtbl);
                ((*vtbl).open_key)(token.0, attr_key_name.as_ptr(), &raw mut raw_attrs)
            };
            if attr_hr.is_ok() && !raw_attrs.is_null() {
                let attrs = ComPtr(raw_attrs);
                let mut lang_ptr: *mut u16 = ptr::null_mut();
                // SAFETY: Reading the `Language` string from the attributes data key.
                let lang_hr = unsafe {
                    let vtbl = *(attrs.0 as *const *const ISpDataKeyVtbl);
                    ((*vtbl).get_string_value)(attrs.0, lang_val_name.as_ptr(), &raw mut lang_ptr)
                };
                if lang_hr.is_ok() {
                    language = take_co_string(lang_ptr);
                } else {
                    let _ = take_co_string(lang_ptr);
                }
            }
            if id_hr.is_ok() && name_hr.is_ok() && !id.is_empty() {
                voices.push(VoiceInfo { id, name, language });
            }
        }
        voices.sort_by(|a, b| {
            let lang_a = if a.language.is_empty() {
                "ffff"
            } else {
                a.language.as_str()
            };
            let lang_b = if b.language.is_empty() {
                "ffff"
            } else {
                b.language.as_str()
            };
            lang_a
                .to_ascii_lowercase()
                .cmp(&lang_b.to_ascii_lowercase())
                .then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
        });
        Ok(voices)
    }

    fn find_sapi_voice_token(wanted: &str) -> Result<ComPtr, SpeechError> {
        let enumerator = open_voice_enum()?;
        loop {
            let mut raw_token: *mut c_void = ptr::null_mut();
            let mut fetched = 0_u32;
            // SAFETY: `enumerator.0` is a valid `IEnumSpObjectTokens`.
            let hr = unsafe {
                let vtbl = *(enumerator.0 as *const *const IEnumSpObjectTokensVtbl);
                ((*vtbl).next)(enumerator.0, 1, &raw mut raw_token, &raw mut fetched)
            };
            if hr.0 != S_OK || fetched == 0 || raw_token.is_null() {
                break;
            }
            let token = ComPtr(raw_token);
            let mut id_ptr: *mut u16 = ptr::null_mut();
            // SAFETY: `token.0` is a valid `ISpObjectToken`.
            let id_hr = unsafe {
                let vtbl = *(token.0 as *const *const ISpObjectTokenVtbl);
                ((*vtbl).get_id)(token.0, &raw mut id_ptr)
            };
            let id = take_co_string(id_ptr);
            if id_hr.is_ok() && id.eq_ignore_ascii_case(wanted) {
                return Ok(token);
            }
        }
        Err(SpeechError::VoiceNotFound(wanted.to_owned()))
    }

    pub(super) fn system_synth() -> Result<Box<dyn SpeechSynth>, SpeechError> {
        Ok(Box::new(WindowsSpeechSynth::new()?))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{SpeechError, SpeechSynth};

    pub(super) fn system_synth() -> Result<Box<dyn SpeechSynth>, SpeechError> {
        Err(SpeechError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_synth_emits_word_boundaries_and_respects_pause_and_stop() {
        let mut synth = MockSpeechSynth::new();
        assert_eq!(synth.voices().expect("voices").len(), 2);
        synth
            .set_voice(Some("mock-en-gb"))
            .expect("select valid voice");
        assert_eq!(synth.selected_voice(), Some("mock-en-gb"));
        assert!(matches!(
            synth.set_voice(Some("missing")),
            Err(SpeechError::VoiceNotFound(_))
        ));

        synth.set_rate(5.0).expect("clamp high");
        assert!((synth.speed() - MAX_SPEED).abs() < f32::EPSILON);
        synth.set_rate(0.1).expect("clamp low");
        assert!((synth.speed() - MIN_SPEED).abs() < f32::EPSILON);

        synth.speak("Hello brave world").expect("speak");
        assert!(synth.is_speaking());
        assert!(synth.step());
        assert_eq!(
            synth.poll_events(),
            vec![SpeechEvent::WordBoundary {
                char_offset: 0,
                char_len: 5
            }]
        );

        synth.pause().expect("pause");
        assert!(!synth.step(), "paused synth does not advance");
        synth.resume().expect("resume");
        synth.finish_utterance();
        assert!(!synth.is_speaking());
        assert_eq!(
            synth.poll_events(),
            vec![
                SpeechEvent::WordBoundary {
                    char_offset: 6,
                    char_len: 5
                },
                SpeechEvent::WordBoundary {
                    char_offset: 12,
                    char_len: 5
                },
                SpeechEvent::Finished,
            ]
        );
    }

    #[test]
    fn sapi_rate_mapping_matches_logarithmic_scale() {
        assert_eq!(speed_to_sapi_rate(1.0), 0);
        assert_eq!(speed_to_sapi_rate(3.0), 10);
        assert_eq!(speed_to_sapi_rate(0.5), -6);
        assert_eq!(speed_to_sapi_rate(f32::NAN), 0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_sapi_initializes_and_enumerates_voices() {
        let mut synth = system_synth().expect("Windows SAPI 5 initializes");
        let voices = synth.voices().expect("enumerate voices");
        assert!(!voices.is_empty(), "Windows ships at least one SAPI voice");
        synth.set_rate(1.25).expect("set rate");
        synth.stop().expect("stop idle");
        // Moving across threads is sound because COM lives exclusively on the dedicated STA thread.
        std::thread::spawn(move || {
            let voices_again = synth.voices().expect("voices from another thread");
            assert!(!voices_again.is_empty());
            synth.stop().expect("stop from another thread");
        })
        .join()
        .expect("join thread");
    }
}
