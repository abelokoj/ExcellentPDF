//! Offline audio transcription trait, Whisper model pack descriptor, SRT/WebVTT import/export,
//! and FTS5-indexed transcript store linked to document pages (plan L.2).

use std::fmt::{self, Write as _};

use rusqlite::{Connection, params};

use crate::audio::sync::AudioTimeline;

/// Whisper model size tier distributed as an optional pack (plan L.2 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhisperModelSize {
    /// Fastest (~75 MB GGML/ONNX pack).
    Tiny,
    /// Balanced (~145 MB pack).
    Base,
    /// High accuracy (~465 MB pack).
    Small,
    /// Maximum offline accuracy (~1.5 GB pack).
    Medium,
}

impl WhisperModelSize {
    /// Pack identifier suffix.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Base => "base",
            Self::Small => "small",
            Self::Medium => "medium",
        }
    }
}

/// Metadata describing an installed Whisper model pack (plan L.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperModelPack {
    /// Model size tier.
    pub size: WhisperModelSize,
    /// Pack ID (`"whisper-tiny"`, etc.).
    pub pack_id: String,
    /// Path or filename of the model weights.
    pub model_filename: String,
    /// License identifier (e.g. `"MIT"`).
    pub license: String,
}

/// Errors produced by offline transcription and transcript storage.
#[derive(Debug)]
#[non_exhaustive]
pub enum TranscribeError {
    /// Whisper model pack is not installed.
    ModelMissing(String),
    /// Audio buffer is empty or invalid.
    InvalidAudio(String),
    /// SRT/VTT parsing error.
    SubtitleParse(String),
    /// SQLite / FTS5 failure.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelMissing(id) => write!(f, "whisper model pack missing: {id}"),
            Self::InvalidAudio(msg) => write!(f, "invalid audio input: {msg}"),
            Self::SubtitleParse(msg) => write!(f, "subtitle parse error: {msg}"),
            Self::Sqlite(err) => write!(f, "transcript database error: {err}"),
        }
    }
}

impl std::error::Error for TranscribeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::ModelMissing(_) | Self::InvalidAudio(_) | Self::SubtitleParse(_) => None,
        }
    }
}

impl From<rusqlite::Error> for TranscribeError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

/// Convenience result alias for transcription operations.
pub type Result<T> = std::result::Result<T, TranscribeError>;

/// A timestamped transcript segment linked to the page that was open when spoken (plan L.2 step 2).
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSegment {
    /// Segment index within the recording.
    pub id: usize,
    /// Start offset into the audio recording, in milliseconds.
    pub start_ms: u64,
    /// End offset into the audio recording, in milliseconds.
    pub end_ms: u64,
    /// Transcribed utterance text.
    pub text: String,
    /// Zero-based document page index open when this segment started.
    pub page: usize,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
}

/// Offline speech-to-text transcriber abstraction (implemented over `whisper-rs` or mock backends
/// in tests) (plan L.2).
pub trait Transcriber: Send + Sync {
    /// Active Whisper model pack tier.
    fn model_size(&self) -> WhisperModelSize;

    /// Transcribes mono 16 kHz PCM samples (`-1.0..=1.0`) into timestamped segments.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::ModelMissing`] or [`TranscribeError::InvalidAudio`].
    fn transcribe(
        &self,
        pcm_mono_16khz: &[f32],
        language_hint: Option<&str>,
    ) -> Result<Vec<TranscriptSegment>>;
}

/// Deterministic mock [`Transcriber`] for automated tests.
#[derive(Debug, Clone)]
pub struct MockTranscriber {
    size: WhisperModelSize,
    canned: Vec<(u64, u64, String)>,
}

impl MockTranscriber {
    /// Creates a mock transcriber returning `canned` `(start_ms, end_ms, text)` segments.
    #[must_use]
    pub fn new(size: WhisperModelSize, canned: Vec<(u64, u64, String)>) -> Self {
        Self { size, canned }
    }
}

impl Transcriber for MockTranscriber {
    fn model_size(&self) -> WhisperModelSize {
        self.size
    }

    fn transcribe(
        &self,
        pcm_mono_16khz: &[f32],
        _language_hint: Option<&str>,
    ) -> Result<Vec<TranscriptSegment>> {
        if pcm_mono_16khz.is_empty() {
            return Err(TranscribeError::InvalidAudio("empty PCM buffer".into()));
        }
        Ok(self
            .canned
            .iter()
            .enumerate()
            .map(|(id, (start_ms, end_ms, text))| TranscriptSegment {
                id,
                start_ms: *start_ms,
                end_ms: *end_ms,
                text: text.clone(),
                page: 0,
                confidence: 0.95,
            })
            .collect())
    }
}

/// Attaches the active document page from `timeline`'s `page_marks` to each transcribed segment
/// (plan L.2 step 2).
pub fn link_segments_to_timeline(segments: &mut [TranscriptSegment], timeline: &AudioTimeline) {
    for seg in segments {
        let start = seg.start_ms as f64;
        let page = timeline
            .page_marks
            .iter()
            .rev()
            .find(|mark| mark.audio_offset_ms <= start)
            .map_or(0, |mark| mark.page);
        seg.page = page;
    }
}

/// Search hit returned by [`TranscriptStore::search`], locating both the moment and the page
/// (plan L.2 **Done when**).
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptHit {
    /// Document or notebook identifier.
    pub doc_id: String,
    /// Recording identifier.
    pub recording_id: String,
    /// Matched segment.
    pub segment: TranscriptSegment,
}

/// SQLite FTS5-backed searchable transcript store (plan L.2 step 3).
pub struct TranscriptStore {
    conn: Connection,
}

impl fmt::Debug for TranscriptStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranscriptStore").finish_non_exhaustive()
    }
}

impl TranscriptStore {
    /// Opens an in-memory searchable transcript store.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::Sqlite`] if initialization fails.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    /// Initializes transcript tables and FTS5 index on `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::Sqlite`] if DDL execution fails.
    pub fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS audio_transcript_segment (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                doc_id TEXT NOT NULL,
                recording_id TEXT NOT NULL,
                seg_index INTEGER NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                page INTEGER NOT NULL,
                confidence REAL NOT NULL,
                text TEXT NOT NULL
            ) STRICT;
            CREATE INDEX IF NOT EXISTS audio_transcript_doc_rec
                ON audio_transcript_segment (doc_id, recording_id, start_ms);
            CREATE VIRTUAL TABLE IF NOT EXISTS audio_transcript_fts USING fts5(
                text,
                content='audio_transcript_segment',
                content_rowid='id',
                tokenize='unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS audio_transcript_ai AFTER INSERT ON audio_transcript_segment BEGIN
                INSERT INTO audio_transcript_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS audio_transcript_ad AFTER DELETE ON audio_transcript_segment BEGIN
                INSERT INTO audio_transcript_fts(audio_transcript_fts, rowid, text)
                    VALUES ('delete', old.id, old.text);
            END;
            ",
        )?;
        Ok(Self { conn })
    }

    /// Runs `transcriber` on `pcm_mono_16khz`, links segments to `timeline` pages, and stores
    /// them in the FTS5 index.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError`] if transcription or SQLite indexing fails.
    pub fn transcribe_and_index(
        &mut self,
        doc_id: &str,
        timeline: &AudioTimeline,
        pcm_mono_16khz: &[f32],
        language_hint: Option<&str>,
        transcriber: &dyn Transcriber,
    ) -> Result<Vec<TranscriptSegment>> {
        let mut segments = transcriber.transcribe(pcm_mono_16khz, language_hint)?;
        link_segments_to_timeline(&mut segments, timeline);
        self.replace_recording_segments(doc_id, &timeline.recording_id, &segments)?;
        Ok(segments)
    }

    /// Replaces indexed segments for `(doc_id, recording_id)` atomically.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::Sqlite`] on database failure.
    pub fn replace_recording_segments(
        &mut self,
        doc_id: &str,
        recording_id: &str,
        segments: &[TranscriptSegment],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM audio_transcript_segment WHERE doc_id = ?1 AND recording_id = ?2",
            params![doc_id, recording_id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO audio_transcript_segment
                    (doc_id, recording_id, seg_index, start_ms, end_ms, page, confidence, text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for seg in segments {
                stmt.execute(params![
                    doc_id,
                    recording_id,
                    seg.id as i64,
                    seg.start_ms as i64,
                    seg.end_ms as i64,
                    seg.page as i64,
                    f64::from(seg.confidence),
                    seg.text,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Searches indexed spoken words across transcripts (or within `doc_id`), returning matching
    /// moments and page indices.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::Sqlite`] on database failure.
    pub fn search(&self, query: &str, doc_id: Option<&str>) -> Result<Vec<TranscriptHit>> {
        let fts_expr = build_fts_query(query);
        if fts_expr.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        if let Some(doc) = doc_id {
            let mut stmt = self.conn.prepare(
                "SELECT s.doc_id, s.recording_id, s.seg_index, s.start_ms, s.end_ms, s.page, s.confidence, s.text
                 FROM audio_transcript_fts f
                 JOIN audio_transcript_segment s ON s.id = f.rowid
                 WHERE audio_transcript_fts MATCH ?1 AND s.doc_id = ?2
                 ORDER BY s.start_ms ASC",
            )?;
            let rows = stmt.query_map(params![fts_expr, doc], row_to_hit)?;
            for row in rows {
                hits.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT s.doc_id, s.recording_id, s.seg_index, s.start_ms, s.end_ms, s.page, s.confidence, s.text
                 FROM audio_transcript_fts f
                 JOIN audio_transcript_segment s ON s.id = f.rowid
                 WHERE audio_transcript_fts MATCH ?1
                 ORDER BY s.doc_id ASC, s.start_ms ASC",
            )?;
            let rows = stmt.query_map(params![fts_expr], row_to_hit)?;
            for row in rows {
                hits.push(row?);
            }
        }
        Ok(hits)
    }
}

fn row_to_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<TranscriptHit> {
    let seg_id: i64 = row.get(2)?;
    let start_ms: i64 = row.get(3)?;
    let end_ms: i64 = row.get(4)?;
    let page: i64 = row.get(5)?;
    let conf: f64 = row.get(6)?;
    Ok(TranscriptHit {
        doc_id: row.get(0)?,
        recording_id: row.get(1)?,
        segment: TranscriptSegment {
            id: usize::try_from(seg_id).unwrap_or(0),
            start_ms: u64::try_from(start_ms).unwrap_or(0),
            end_ms: u64::try_from(end_ms).unwrap_or(0),
            page: usize::try_from(page).unwrap_or(0),
            confidence: conf as f32,
            text: row.get(7)?,
        },
    })
}

fn build_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    terms.join(" ")
}

/// Exports `segments` in SubRip (`.srt`) format.
#[must_use]
pub fn to_srt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (idx, seg) in segments.iter().enumerate() {
        let _ = writeln!(out, "{}", idx + 1);
        let _ = writeln!(
            out,
            "{} --> {}",
            fmt_timestamp(seg.start_ms, ','),
            fmt_timestamp(seg.end_ms, ',')
        );
        let _ = writeln!(out, "{}\n", seg.text.trim());
    }
    out
}

/// Parses SubRip (`.srt`) content into [`TranscriptSegment`]s.
///
/// # Errors
///
/// Returns [`TranscribeError::SubtitleParse`] if timestamp blocks are malformed.
pub fn from_srt(srt: &str) -> Result<Vec<TranscriptSegment>> {
    let normalized = srt.replace("\r\n", "\n");
    let mut segments = Vec::new();
    for (idx, block) in normalized.split("\n\n").enumerate() {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let ts_line_idx = if lines.first().is_some_and(|l| l.contains("-->")) {
            0
        } else if lines.get(1).is_some_and(|l| l.contains("-->")) {
            1
        } else {
            return Err(TranscribeError::SubtitleParse(format!(
                "SRT block {} missing '-->' timestamp",
                idx + 1
            )));
        };
        let (start_ms, end_ms) = parse_timestamp_range(lines[ts_line_idx])?;
        let text = lines[ts_line_idx + 1..].join(" ");
        segments.push(TranscriptSegment {
            id: segments.len(),
            start_ms,
            end_ms,
            text,
            page: 0,
            confidence: 1.0,
        });
    }
    Ok(segments)
}

/// Exports `segments` in WebVTT (`.vtt`) format.
#[must_use]
pub fn to_vtt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for seg in segments {
        let _ = writeln!(
            out,
            "{} --> {}",
            fmt_timestamp(seg.start_ms, '.'),
            fmt_timestamp(seg.end_ms, '.')
        );
        let _ = writeln!(out, "{}\n", seg.text.trim());
    }
    out
}

/// Parses WebVTT (`.vtt`) content into [`TranscriptSegment`]s.
///
/// # Errors
///
/// Returns [`TranscribeError::SubtitleParse`] if header or cues are invalid.
pub fn from_vtt(vtt: &str) -> Result<Vec<TranscriptSegment>> {
    let normalized = vtt.replace("\r\n", "\n");
    let trimmed = normalized.trim_start();
    if !trimmed.starts_with("WEBVTT") {
        return Err(TranscribeError::SubtitleParse(
            "WebVTT missing WEBVTT header".into(),
        ));
    }
    let mut segments = Vec::new();
    for block in trimmed.split("\n\n").skip(1) {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let Some(ts_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let (start_ms, end_ms) = parse_timestamp_range(lines[ts_idx])?;
        let text = lines[ts_idx + 1..].join(" ");
        segments.push(TranscriptSegment {
            id: segments.len(),
            start_ms,
            end_ms,
            text,
            page: 0,
            confidence: 1.0,
        });
    }
    Ok(segments)
}

fn fmt_timestamp(ms: u64, sep: char) -> String {
    let hours = ms / 3_600_000;
    let mins = (ms / 60_000) % 60;
    let secs = (ms / 1_000) % 60;
    let rem_ms = ms % 1_000;
    format!("{hours:02}:{mins:02}:{secs:02}{sep}{rem_ms:03}")
}

fn parse_timestamp_range(line: &str) -> Result<(u64, u64)> {
    let (a, b) = line
        .split_once("-->")
        .ok_or_else(|| TranscribeError::SubtitleParse(format!("bad range: {line}")))?;
    let start = parse_ts(a.trim())?;
    let end_tok = b.split_whitespace().next().unwrap_or("");
    let end = parse_ts(end_tok)?;
    Ok((start, end))
}

fn parse_ts(ts: &str) -> Result<u64> {
    let (hms, ms_str) = ts
        .split_once(',')
        .or_else(|| ts.split_once('.'))
        .ok_or_else(|| TranscribeError::SubtitleParse(format!("bad timestamp: {ts}")))?;
    let ms: u64 = ms_str
        .parse()
        .map_err(|_| TranscribeError::SubtitleParse(format!("bad ms: {ts}")))?;
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [hh, mm, ss] => (
            hh.parse::<u64>().ok(),
            mm.parse::<u64>().ok(),
            ss.parse::<u64>().ok(),
        ),
        [mm, ss] => (Some(0), mm.parse::<u64>().ok(), ss.parse::<u64>().ok()),
        _ => (None, None, None),
    };
    match (h, m, s) {
        (Some(hh), Some(mm), Some(ss)) => Ok(((hh * 3600) + (mm * 60) + ss) * 1000 + ms),
        _ => Err(TranscribeError::SubtitleParse(format!("bad hms: {ts}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searching_spoken_word_finds_moment_and_page() {
        let mut timeline = AudioTimeline::start("lec-1", 10_000.0, 0);
        timeline.mark_page(25_000.0, 2); // at 15,000 ms user turned to page 2
        timeline.finish(40_000.0);

        let transcriber = MockTranscriber::new(
            WhisperModelSize::Base,
            vec![
                (1_000, 4_500, "Welcome to introductory calculus".into()),
                (16_000, 21_500, "The fundamental theorem of calculus".into()),
            ],
        );

        let mut store = TranscriptStore::in_memory().expect("store");
        let pcm = vec![0.0_f32; 16_000];
        let segs = store
            .transcribe_and_index("calc.expdf", &timeline, &pcm, Some("en"), &transcriber)
            .expect("transcribe");
        assert_eq!(segs[0].page, 0);
        assert_eq!(segs[1].page, 2);

        let hits = store.search("theorem", Some("calc.expdf")).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].segment.page, 2);
        assert_eq!(hits[0].segment.start_ms, 16_000);

        let srt = to_srt(&segs);
        let parsed_srt = from_srt(&srt).expect("srt");
        assert_eq!(parsed_srt[1].start_ms, 16_000);
        assert_eq!(parsed_srt[1].text, "The fundamental theorem of calculus");

        let vtt = to_vtt(&segs);
        let parsed_vtt = from_vtt(&vtt).expect("vtt");
        assert_eq!(parsed_vtt[0].text, "Welcome to introductory calculus");
    }
}
