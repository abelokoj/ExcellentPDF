//! Searchable handwriting index and cross-platform recognizer interface (plan H.23).
//!
//! Defines the [`Recognizer`] trait for OS/pack backends (Windows `InkAnalyzer`, Apple Vision,
//! ML Kit Digital Ink, ONNX model pack, or built-in offline matcher), tracks idle-priority
//! recognition queues per page, persists recognized words with stroke IDs and bounding boxes in
//! SQLite FTS5, and resolves search queries back to highlighted stroke IDs.

use std::collections::BTreeSet;
use std::fmt;

use excellent_mupdf::{PagePoint, PageRect};
use rusqlite::{Connection, params};
use serde_json::json;

/// Platform or pack backend supplying handwriting recognition (plan H.23 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Windows `Windows.UI.Input.Inking.Analysis.InkAnalyzer`.
    WindowsInkAnalysis,
    /// Apple Vision text recognition on rendered strokes (macOS / iPadOS).
    AppleVision,
    /// Google ML Kit Digital Ink (Android).
    MlKitDigitalInk,
    /// Downloadable offline ONNX digital ink model pack (Linux / fallback).
    OnnxPack,
    /// Built-in deterministic geometric stroke-feature and lexicon matcher.
    BuiltinOffline,
}

/// Errors produced by handwriting recognition backends.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecognitionError {
    /// Requested language is not supported by this backend.
    UnsupportedLanguage(String),
    /// Backend is unavailable on this OS or missing its optional pack.
    BackendUnavailable(String),
    /// Input strokes were empty or malformed.
    InvalidInput(String),
}

impl fmt::Display for RecognitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLanguage(lang) => {
                write!(f, "handwriting language not supported: {lang}")
            }
            Self::BackendUnavailable(msg) => write!(f, "handwriting backend unavailable: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid handwriting input: {msg}"),
        }
    }
}

impl std::error::Error for RecognitionError {}

/// Errors produced by the SQLite FTS5 handwriting index.
#[derive(Debug)]
pub enum IndexError {
    /// SQLite database or FTS5 failure.
    Sqlite(rusqlite::Error),
    /// JSON serialization failure.
    Json(serde_json::Error),
    /// Recognition failed while indexing strokes.
    Recognition(RecognitionError),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "handwriting index sqlite error: {err}"),
            Self::Json(err) => write!(f, "handwriting index json error: {err}"),
            Self::Recognition(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::Recognition(err) => Some(err),
        }
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<serde_json::Error> for IndexError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<RecognitionError> for IndexError {
    fn from(err: RecognitionError) -> Self {
        Self::Recognition(err)
    }
}

/// An ink stroke submitted for recognition or indexing, identified by its annotation/stroke ID.
#[derive(Debug, Clone, PartialEq)]
pub struct HandwritingStroke {
    /// Stable stroke or annotation identifier.
    pub id: String,
    /// Polyline points in page points (`x` right, `y` down).
    pub points: Vec<PagePoint>,
    /// Optional stroke capture timestamps in milliseconds (same length as `points` when present).
    pub timestamps_ms: Vec<f64>,
}

impl HandwritingStroke {
    /// Computes the tight axis-aligned bounding box of this stroke, or `None` if empty.
    #[must_use]
    pub fn bbox(&self) -> Option<PageRect> {
        let first = self.points.first()?;
        let (mut x0, mut x1) = (first.x, first.x);
        let (mut y0, mut y1) = (first.y, first.y);
        for p in self.points.iter().skip(1) {
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
        }
        Some(PageRect { x0, y0, x1, y1 })
    }
}

/// A recognized handwritten word linked back to its source stroke IDs and page bounding box.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedWord {
    /// Primary recognized word text.
    pub text: String,
    /// Alternative recognition candidates, ranked best-first.
    pub candidates: Vec<String>,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
    /// Zero-based line index within the recognized block (for preserving line breaks in H.24).
    pub line_index: usize,
    /// Source stroke IDs contributing to this word.
    pub stroke_ids: Vec<String>,
    /// Enclosing bounding box in page points.
    pub bbox: PageRect,
}

/// Cross-platform digital ink recognizer trait (plan H.23 step 1).
pub trait Recognizer: Send + Sync {
    /// Backend category implemented by this recognizer.
    fn backend(&self) -> BackendKind;

    /// BCP-47 language tags supported by this backend (e.g. `["en-US", "en"]`).
    fn supported_languages(&self) -> &[&str];

    /// Recognizes handwritten words from ordered `strokes` for language `lang`.
    ///
    /// # Errors
    ///
    /// Returns [`RecognitionError`] on unsupported languages, missing backends, or empty input.
    fn recognize(
        &self,
        strokes: &[HandwritingStroke],
        lang: &str,
    ) -> std::result::Result<Vec<RecognizedWord>, RecognitionError>;
}

/// Search hit returned from [`HandwritingIndex::search`], identifying the strokes to highlight
/// (plan H.23 step 3).
#[derive(Debug, Clone, PartialEq)]
pub struct HandwritingHit {
    /// Document identifier.
    pub doc_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Recognized word text.
    pub word: String,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
    /// Source stroke IDs to highlight on the page canvas.
    pub stroke_ids: Vec<String>,
    /// Bounding box around the matched handwritten word.
    pub bbox: PageRect,
}

/// Idle-priority dirty-page queue tracking pages with newly drawn strokes awaiting recognition
/// (plan H.23 step 2).
#[derive(Debug, Clone, Default)]
pub struct PendingPageQueue {
    dirty: BTreeSet<(String, usize)>,
}

impl PendingPageQueue {
    /// Creates an empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `(doc_id, page)` as needing background recognition.
    pub fn mark_dirty(&mut self, doc_id: impl Into<String>, page: usize) {
        self.dirty.insert((doc_id.into(), page));
    }

    /// Pops the next `(doc_id, page)` pair to recognize at idle priority.
    pub fn pop_next(&mut self) -> Option<(String, usize)> {
        let first = self.dirty.iter().next().cloned()?;
        self.dirty.remove(&first);
        Some(first)
    }

    /// Number of queued pages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dirty.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dirty.is_empty()
    }
}

/// SQLite FTS5-backed searchable index of recognized handwritten words (plan H.23).
pub struct HandwritingIndex {
    conn: Connection,
}

impl fmt::Debug for HandwritingIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandwritingIndex").finish_non_exhaustive()
    }
}

impl HandwritingIndex {
    /// Opens an in-memory searchable handwriting index (useful for tests or transient sessions).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Sqlite`] if schema initialization fails.
    pub fn in_memory() -> Result<Self, IndexError> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    /// Initializes the handwriting tables and FTS5 index on an existing SQLite `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Sqlite`] if DDL execution fails.
    pub fn from_connection(conn: Connection) -> Result<Self, IndexError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS handwriting_words (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                doc_id TEXT NOT NULL,
                page INTEGER NOT NULL,
                word TEXT NOT NULL,
                confidence REAL NOT NULL,
                stroke_ids_json TEXT NOT NULL,
                x0 REAL NOT NULL,
                y0 REAL NOT NULL,
                x1 REAL NOT NULL,
                y1 REAL NOT NULL
            ) STRICT;
            CREATE INDEX IF NOT EXISTS handwriting_words_doc_page
                ON handwriting_words (doc_id, page);
            CREATE VIRTUAL TABLE IF NOT EXISTS handwriting_fts USING fts5(
                word,
                content='handwriting_words',
                content_rowid='id',
                tokenize='unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS handwriting_words_ai AFTER INSERT ON handwriting_words BEGIN
                INSERT INTO handwriting_fts(rowid, word) VALUES (new.id, new.word);
            END;
            CREATE TRIGGER IF NOT EXISTS handwriting_words_ad AFTER DELETE ON handwriting_words BEGIN
                INSERT INTO handwriting_fts(handwriting_fts, rowid, word)
                    VALUES ('delete', old.id, old.word);
            END;
            ",
        )?;
        Ok(Self { conn })
    }

    /// Recognizes `strokes` on `(doc_id, page)` using `recognizer` and replaces that page's
    /// entries in the FTS5 index.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] on recognition or SQLite failure.
    pub fn index_page(
        &mut self,
        doc_id: &str,
        page: usize,
        strokes: &[HandwritingStroke],
        recognizer: &dyn Recognizer,
        lang: &str,
    ) -> Result<Vec<RecognizedWord>, IndexError> {
        let words = if strokes.is_empty() {
            Vec::new()
        } else {
            recognizer.recognize(strokes, lang)?
        };
        self.replace_page_words(doc_id, page, &words)?;
        Ok(words)
    }

    /// Replaces indexed recognized words for `(doc_id, page)` atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Sqlite`] or [`IndexError::Json`].
    pub fn replace_page_words(
        &mut self,
        doc_id: &str,
        page: usize,
        words: &[RecognizedWord],
    ) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM handwriting_words WHERE doc_id = ?1 AND page = ?2",
            params![doc_id, page as i64],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO handwriting_words
                    (doc_id, page, word, confidence, stroke_ids_json, x0, y0, x1, y1)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for w in words {
                let ids_json = serde_json::to_string(&json!(w.stroke_ids))?;
                stmt.execute(params![
                    doc_id,
                    page as i64,
                    w.text,
                    f64::from(w.confidence),
                    ids_json,
                    f64::from(w.bbox.x0),
                    f64::from(w.bbox.y0),
                    f64::from(w.bbox.x1),
                    f64::from(w.bbox.y1),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Searches indexed handwritten words across all documents (or filtered to `doc_id` when
    /// `Some`) using FTS5 prefix/word matching, returning hits with stroke IDs to highlight.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Sqlite`] or [`IndexError::Json`].
    pub fn search(
        &self,
        query: &str,
        doc_id: Option<&str>,
    ) -> Result<Vec<HandwritingHit>, IndexError> {
        let fts_expr = build_fts_query(query);
        if fts_expr.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = Vec::new();
        if let Some(doc) = doc_id {
            let mut stmt = self.conn.prepare(
                "SELECT w.doc_id, w.page, w.word, w.confidence, w.stroke_ids_json,
                        w.x0, w.y0, w.x1, w.y1
                 FROM handwriting_fts f
                 JOIN handwriting_words w ON w.id = f.rowid
                 WHERE handwriting_fts MATCH ?1 AND w.doc_id = ?2
                 ORDER BY w.page ASC, w.y0 ASC, w.x0 ASC",
            )?;
            let rows = stmt.query_map(params![fts_expr, doc], row_to_hit)?;
            for row in rows {
                hits.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT w.doc_id, w.page, w.word, w.confidence, w.stroke_ids_json,
                        w.x0, w.y0, w.x1, w.y1
                 FROM handwriting_fts f
                 JOIN handwriting_words w ON w.id = f.rowid
                 WHERE handwriting_fts MATCH ?1
                 ORDER BY w.doc_id ASC, w.page ASC, w.y0 ASC, w.x0 ASC",
            )?;
            let rows = stmt.query_map(params![fts_expr], row_to_hit)?;
            for row in rows {
                hits.push(row?);
            }
        }
        Ok(hits)
    }
}

fn row_to_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<HandwritingHit> {
    let page_i64: i64 = row.get(1)?;
    let ids_json: String = row.get(4)?;
    let stroke_ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
    let conf: f64 = row.get(3)?;
    let x0: f64 = row.get(5)?;
    let y0: f64 = row.get(6)?;
    let x1: f64 = row.get(7)?;
    let y1: f64 = row.get(8)?;
    Ok(HandwritingHit {
        doc_id: row.get(0)?,
        page: usize::try_from(page_i64).unwrap_or(0),
        word: row.get(2)?,
        confidence: conf as f32,
        stroke_ids,
        bbox: PageRect {
            x0: x0 as f32,
            y0: y0 as f32,
            x1: x1 as f32,
            y1: y1 as f32,
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

#[cfg(test)]
mod tests {
    use super::*;

    struct StubRecognizer;
    impl Recognizer for StubRecognizer {
        fn backend(&self) -> BackendKind {
            BackendKind::BuiltinOffline
        }

        fn supported_languages(&self) -> &[&str] {
            &["en", "en-US"]
        }

        fn recognize(
            &self,
            strokes: &[HandwritingStroke],
            _lang: &str,
        ) -> std::result::Result<Vec<RecognizedWord>, RecognitionError> {
            let ids: Vec<String> = strokes.iter().map(|s| s.id.clone()).collect();
            let bbox = strokes
                .first()
                .and_then(HandwritingStroke::bbox)
                .unwrap_or_default();
            Ok(vec![
                RecognizedWord {
                    text: "Photosynthesis".into(),
                    candidates: vec!["Photosynthesis".into()],
                    confidence: 0.94,
                    line_index: 0,
                    stroke_ids: vec![ids[0].clone()],
                    bbox,
                },
                RecognizedWord {
                    text: "notes".into(),
                    candidates: vec!["notes".into()],
                    confidence: 0.91,
                    line_index: 0,
                    stroke_ids: ids.get(1..).unwrap_or(&[]).to_vec(),
                    bbox,
                },
            ])
        }
    }

    #[test]
    fn fts5_index_finds_handwritten_word_and_returns_stroke_ids() {
        let mut index = HandwritingIndex::in_memory().expect("in-memory fts5");
        let strokes = vec![
            HandwritingStroke {
                id: "stroke-1".into(),
                points: vec![
                    PagePoint { x: 40.0, y: 80.0 },
                    PagePoint { x: 140.0, y: 98.0 },
                ],
                timestamps_ms: Vec::new(),
            },
            HandwritingStroke {
                id: "stroke-2".into(),
                points: vec![
                    PagePoint { x: 150.0, y: 80.0 },
                    PagePoint { x: 200.0, y: 98.0 },
                ],
                timestamps_ms: Vec::new(),
            },
        ];
        index
            .index_page("notebook-1", 3, &strokes, &StubRecognizer, "en-US")
            .expect("index page");

        let hits = index.search("photo", Some("notebook-1")).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, 3);
        assert_eq!(hits[0].word, "Photosynthesis");
        assert_eq!(hits[0].stroke_ids, vec!["stroke-1"]);
    }
}
