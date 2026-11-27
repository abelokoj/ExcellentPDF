//! Local document chunking and hybrid BM25 + vector/TF-IDF SQLite index (plan I.2, §12.2).
//!
//! Splits page text into overlapping paragraph windows preserving page numbers and bounding quads,
//! stores chunks and FTS5 lexical entries in SQLite via [`ensure_schema`] (keeping AI index tables
//! decoupled from `ecclesia-store` migrations), and retrieves passages with hybrid BM25 + cosine
//! vector scoring plus metadata filters.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::ops::RangeInclusive;

use ecclesia_text::PageText;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::provider::AiProvider;

/// Dimensionality of the built-in hashed TF-IDF dense embedding vector.
pub const DEFAULT_EMBEDDING_DIM: usize = 128;

/// Errors produced by chunking, schema setup, indexing, or hybrid search.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum IndexError {
    /// SQLite database or FTS5 query error.
    Sqlite(rusqlite::Error),
    /// JSON encoding/decoding error for stored quads or vectors.
    Json(serde_json::Error),
    /// External embedding provider failed.
    Provider(crate::provider::ProviderError),
    /// Caller passed invalid chunking or query parameters.
    InvalidInput(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "ai index sqlite error: {err}"),
            Self::Json(err) => write!(f, "ai index json error: {err}"),
            Self::Provider(err) => write!(f, "ai index embedding error: {err}"),
            Self::InvalidInput(msg) => write!(f, "ai index invalid input: {msg}"),
        }
    }
}

impl Error for IndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::Provider(err) => Some(err),
            Self::InvalidInput(_) => None,
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

impl From<crate::provider::ProviderError> for IndexError {
    fn from(err: crate::provider::ProviderError) -> Self {
        Self::Provider(err)
    }
}

/// Four-corner page-point quad anchoring a citation back to the PDF canvas (plan §12.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CitationQuad {
    /// Upper-left `[x, y]` in page points.
    pub ul: [f32; 2],
    /// Upper-right `[x, y]` in page points.
    pub ur: [f32; 2],
    /// Lower-left `[x, y]` in page points.
    pub ll: [f32; 2],
    /// Lower-right `[x, y]` in page points.
    pub lr: [f32; 2],
}

impl CitationQuad {
    /// Creates an axis-aligned quad from `(x0, y0, x1, y1)` page points.
    #[must_use]
    pub fn from_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self {
            ul: [x0, y0],
            ur: [x1, y0],
            ll: [x0, y1],
            lr: [x1, y1],
        }
    }

    fn to_array(self) -> [f32; 8] {
        [
            self.ul[0], self.ul[1], self.ur[0], self.ur[1], self.ll[0], self.ll[1], self.lr[0],
            self.lr[1],
        ]
    }

    fn from_array(a: [f32; 8]) -> Self {
        Self {
            ul: [a[0], a[1]],
            ur: [a[2], a[3]],
            ll: [a[4], a[5]],
            lr: [a[6], a[7]],
        }
    }
}

/// One logical line or paragraph span with its page-point quads.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLineSpan {
    /// Text content of the line or paragraph.
    pub text: String,
    /// Bounding quads on the page corresponding to `text`.
    pub quads: Vec<CitationQuad>,
}

/// Input page representation consumed by [`chunk_document`] and [`DocumentIndex`].
#[derive(Debug, Clone, PartialEq)]
pub struct PageSource {
    /// Zero-based page index.
    pub page: usize,
    /// Ordered text lines or paragraphs on the page.
    pub lines: Vec<PageLineSpan>,
    /// Optional heading/chapter label covering this page.
    pub chapter: Option<String>,
}

impl PageSource {
    /// Constructs a [`PageSource`] directly from plain text and synthetic line bounds.
    #[must_use]
    pub fn from_plain_text(page: usize, text: &str, chapter: Option<&str>) -> Self {
        let lines = text
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let y0 = (idx as f32) * 14.0 + 36.0;
                Some(PageLineSpan {
                    text: trimmed.to_owned(),
                    quads: vec![CitationQuad::from_rect(36.0, y0, 540.0, y0 + 12.0)],
                })
            })
            .collect();
        Self {
            page,
            lines,
            chapter: chapter.map(str::to_owned),
        }
    }

    /// Extracts lines and exact character-run quads from an `ecclesia-text` [`PageText`].
    #[must_use]
    pub fn from_page_text(page: usize, page_text: &PageText, chapter: Option<&str>) -> Self {
        let layout = page_text.layout();
        let mut lines = Vec::with_capacity(layout.lines.len());
        for line in &layout.lines {
            let text = page_text.text(line.chars.clone());
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let quads = page_text
                .highlight(line.chars.clone())
                .into_iter()
                .map(|q| CitationQuad {
                    ul: [q.ul.x, q.ul.y],
                    ur: [q.ur.x, q.ur.y],
                    ll: [q.ll.x, q.ll.y],
                    lr: [q.lr.x, q.lr.y],
                })
                .collect();
            lines.push(PageLineSpan {
                text: trimmed.to_owned(),
                quads,
            });
        }
        Self {
            page,
            lines,
            chapter: chapter.map(str::to_owned),
        }
    }
}

/// Chunking parameters targeting overlapping ~512-token paragraph windows (plan §12.2, I.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkConfig {
    /// Target token budget per chunk (plan default: 512).
    pub target_tokens: usize,
    /// Overlap tokens carried forward into the next chunk on the same page.
    pub overlap_tokens: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            target_tokens: 512,
            overlap_tokens: 64,
        }
    }
}

/// One indexed document chunk with page, chapter, quads, and content hash.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Deterministic chunk identifier (`"{doc_id}#p{page}c{chunk_index}"`).
    pub id: String,
    /// Document identifier.
    pub doc_id: String,
    /// Zero-based page number.
    pub page: usize,
    /// Zero-based chunk ordinal within the document.
    pub chunk_index: usize,
    /// Optional chapter/section heading.
    pub chapter: Option<String>,
    /// Chunk text window.
    pub text: String,
    /// Bounding quads covered by lines in this chunk.
    pub quads: Vec<CitationQuad>,
    /// Approximate whitespace/punctuation token count.
    pub token_count: usize,
    /// SHA-256 hex digest of `text` for incremental reindexing checks.
    pub content_hash: String,
}

/// Estimates token count deterministically without requiring an external tokenizer binary.
#[must_use]
pub fn estimate_token_count(text: &str) -> usize {
    let words = text.split_whitespace().count();
    let approx_subwords = text.len().div_ceil(4);
    words.max(approx_subwords)
}

/// Splits `pages` into overlapping paragraph/line windows obeying `config` (plan I.2).
#[must_use]
pub fn chunk_document(doc_id: &str, pages: &[PageSource], config: &ChunkConfig) -> Vec<Chunk> {
    let target = config.target_tokens.max(16);
    let overlap = config.overlap_tokens.min(target / 2);
    let mut chunks = Vec::new();

    for page in pages {
        if page.lines.is_empty() {
            continue;
        }
        let line_tokens: Vec<usize> = page
            .lines
            .iter()
            .map(|l| estimate_token_count(&l.text).max(1))
            .collect();

        let mut start = 0usize;
        while start < page.lines.len() {
            let mut end = start;
            let mut used = 0usize;
            while end < page.lines.len() {
                let next_toks = line_tokens[end];
                if end > start && used + next_toks > target {
                    break;
                }
                used += next_toks;
                end += 1;
            }

            let text = page.lines[start..end]
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let quads: Vec<CitationQuad> = page.lines[start..end]
                .iter()
                .flat_map(|l| l.quads.iter().copied())
                .collect();
            let chunk_index = chunks.len();
            let id = format!("{doc_id}#p{}c{chunk_index}", page.page + 1);
            let content_hash = sha256_hex(&text);

            chunks.push(Chunk {
                id,
                doc_id: doc_id.to_owned(),
                page: page.page,
                chunk_index,
                chapter: page.chapter.clone(),
                text,
                quads,
                token_count: used,
                content_hash,
            });

            if end >= page.lines.len() {
                break;
            }

            let mut back_tokens = 0usize;
            let mut next_start = end;
            while next_start > start + 1 && back_tokens < overlap {
                next_start -= 1;
                back_tokens += line_tokens[next_start];
            }
            if next_start <= start {
                next_start = end;
            }
            start = next_start;
        }
    }

    chunks
}

/// Ensures `ecclesia-ai` SQLite tables and FTS5 index exist idempotently on `conn` (plan I.2).
///
/// # Errors
///
/// Returns [`IndexError::Sqlite`] if table or virtual table creation fails.
pub fn ensure_schema(conn: &Connection) -> Result<(), IndexError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS ai_doc_meta (
            doc_id TEXT PRIMARY KEY,
            doc_hash TEXT NOT NULL,
            indexed_at INTEGER NOT NULL,
            chunk_count INTEGER NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS ai_chunks (
            chunk_id TEXT PRIMARY KEY,
            doc_id TEXT NOT NULL,
            page INTEGER NOT NULL,
            chunk_index INTEGER NOT NULL,
            chapter TEXT,
            text TEXT NOT NULL,
            quads_json TEXT NOT NULL,
            token_count INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            vector_json TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS ai_chunks_doc_page
            ON ai_chunks (doc_id, page, chunk_index);

        CREATE VIRTUAL TABLE IF NOT EXISTS ai_chunks_fts USING fts5(
            chunk_id UNINDEXED,
            doc_id UNINDEXED,
            text,
            chapter,
            tokenize='unicode61'
        );
        ",
    )?;
    Ok(())
}

/// Query parameters and metadata filters for [`DocumentIndex::search`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Natural-language query text.
    pub text: String,
    /// Restrict results to these document IDs (empty = search all indexed documents).
    pub doc_ids: Vec<String>,
    /// Optional inclusive zero-based page range filter.
    pub page_range: Option<(usize, usize)>,
    /// Optional substring filter matched against `chapter`.
    pub chapter: Option<String>,
    /// Maximum number of ranked hits to return.
    pub limit: usize,
}

impl SearchQuery {
    /// Creates a search query across all indexed documents returning up to `limit` hits.
    #[must_use]
    pub fn new(text: impl Into<String>, limit: usize) -> Self {
        Self {
            text: text.into(),
            doc_ids: Vec::new(),
            page_range: None,
            chapter: None,
            limit: limit.max(1),
        }
    }

    /// Restricts retrieval to `doc_ids`.
    #[must_use]
    pub fn with_docs<I, S>(mut self, doc_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.doc_ids = doc_ids.into_iter().map(Into::into).collect();
        self
    }

    /// Restricts retrieval to zero-based `pages`.
    #[must_use]
    pub fn with_page_range(mut self, pages: RangeInclusive<usize>) -> Self {
        self.page_range = Some((*pages.start(), *pages.end()));
        self
    }

    /// Restricts retrieval to chunks whose chapter label contains `chapter`.
    #[must_use]
    pub fn with_chapter(mut self, chapter: impl Into<String>) -> Self {
        self.chapter = Some(chapter.into());
        self
    }
}

/// Ranked retrieval result combining BM25 lexical relevance and vector cosine similarity.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Retrieved chunk with page and quad citations.
    pub chunk: Chunk,
    /// Normalized lexical BM25 score (`>= 0.0`).
    pub bm25_score: f64,
    /// Cosine similarity score in `[0.0, 1.0]`.
    pub vector_score: f64,
    /// Combined hybrid ranking score.
    pub hybrid_score: f64,
}

/// Summary returned after indexing a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStats {
    /// Document identifier.
    pub doc_id: String,
    /// Number of chunks stored.
    pub chunk_count: usize,
    /// Aggregate content digest across all chunks.
    pub doc_hash: String,
}

/// Hybrid SQLite FTS5 + dense vector document index (plan I.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DocumentIndex;

impl DocumentIndex {
    /// Creates a stateless handle for indexing and querying an SQLite connection.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Indexes or re-indexes `pages` for `doc_id` inside `conn`, optionally using `embedder`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if SQLite operations, JSON serialization, or embeddings fail.
    pub fn index_document(
        &self,
        conn: &Connection,
        doc_id: &str,
        pages: &[PageSource],
        config: &ChunkConfig,
        embedder: Option<&dyn AiProvider>,
        indexed_at: i64,
    ) -> Result<IndexStats, IndexError> {
        ensure_schema(conn)?;
        let chunks = chunk_document(doc_id, pages, config);
        let doc_hash = compute_doc_hash(&chunks);

        let existing_hash: Option<String> = conn
            .query_row(
                "SELECT doc_hash FROM ai_doc_meta WHERE doc_id = ?1",
                params![doc_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing_hash.as_deref() == Some(doc_hash.as_str()) {
            return Ok(IndexStats {
                doc_id: doc_id.to_owned(),
                chunk_count: chunks.len(),
                doc_hash,
            });
        }

        let vectors = compute_vectors(&chunks, embedder)?;

        conn.execute(
            "DELETE FROM ai_chunks_fts WHERE doc_id = ?1",
            params![doc_id],
        )?;
        conn.execute("DELETE FROM ai_chunks WHERE doc_id = ?1", params![doc_id])?;

        {
            let mut insert_chunk = conn.prepare(
                "INSERT INTO ai_chunks (
                    chunk_id, doc_id, page, chunk_index, chapter, text, quads_json,
                    token_count, content_hash, vector_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            let mut insert_fts = conn.prepare(
                "INSERT INTO ai_chunks_fts (chunk_id, doc_id, text, chapter)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;

            for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
                let quad_arrays: Vec<[f32; 8]> = chunk
                    .quads
                    .iter()
                    .copied()
                    .map(CitationQuad::to_array)
                    .collect();
                let quads_json = serde_json::to_string(&quad_arrays)?;
                let vector_json = serde_json::to_string(vector)?;
                insert_chunk.execute(params![
                    chunk.id,
                    chunk.doc_id,
                    chunk.page,
                    chunk.chunk_index,
                    chunk.chapter,
                    chunk.text,
                    quads_json,
                    chunk.token_count,
                    chunk.content_hash,
                    vector_json,
                ])?;
                insert_fts.execute(params![
                    chunk.id,
                    chunk.doc_id,
                    chunk.text,
                    chunk.chapter.as_deref().unwrap_or(""),
                ])?;
            }
        }

        conn.execute(
            "INSERT INTO ai_doc_meta (doc_id, doc_hash, indexed_at, chunk_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(doc_id) DO UPDATE SET
               doc_hash = excluded.doc_hash,
               indexed_at = excluded.indexed_at,
               chunk_count = excluded.chunk_count",
            params![doc_id, doc_hash, indexed_at, chunks.len()],
        )?;

        Ok(IndexStats {
            doc_id: doc_id.to_owned(),
            chunk_count: chunks.len(),
            doc_hash,
        })
    }

    /// Removes all indexed chunks and metadata for `doc_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Sqlite`] if deletion fails.
    pub fn remove_document(&self, conn: &Connection, doc_id: &str) -> Result<(), IndexError> {
        ensure_schema(conn)?;
        conn.execute(
            "DELETE FROM ai_chunks_fts WHERE doc_id = ?1",
            params![doc_id],
        )?;
        conn.execute("DELETE FROM ai_chunks WHERE doc_id = ?1", params![doc_id])?;
        conn.execute("DELETE FROM ai_doc_meta WHERE doc_id = ?1", params![doc_id])?;
        Ok(())
    }

    /// Loads all indexed chunks matching optional metadata filters (useful for full-doc summaries).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if SQLite reading or JSON decoding fails.
    pub fn list_chunks(
        &self,
        conn: &Connection,
        doc_ids: &[&str],
        page_range: Option<RangeInclusive<usize>>,
    ) -> Result<Vec<Chunk>, IndexError> {
        ensure_schema(conn)?;
        let rows = load_filtered_chunks(
            conn,
            doc_ids,
            page_range.map(|r| (*r.start(), *r.end())),
            None,
        )?;
        Ok(rows.into_iter().map(|(c, _)| c).collect())
    }

    /// Runs hybrid BM25 + vector retrieval with metadata filters (plan I.2).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query is empty, SQLite fails, or embedding fails.
    pub fn search(
        &self,
        conn: &Connection,
        query: &SearchQuery,
        embedder: Option<&dyn AiProvider>,
    ) -> Result<Vec<SearchHit>, IndexError> {
        ensure_schema(conn)?;
        let trimmed = query.text.trim();
        if trimmed.is_empty() {
            return Err(IndexError::InvalidInput(
                "search query cannot be empty".to_owned(),
            ));
        }

        let doc_refs: Vec<&str> = query.doc_ids.iter().map(String::as_str).collect();
        let candidates =
            load_filtered_chunks(conn, &doc_refs, query.page_range, query.chapter.as_deref())?;
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let fts_scores = query_fts5_bm25(conn, trimmed, candidates.len().max(32))?;
        let query_tokens = tokenize_normalized(trimmed);
        let query_vec = if let Some(provider) = embedder {
            let mut vecs = provider.embed(&[trimmed])?;
            vecs.pop().unwrap_or_else(|| hashed_text_embedding(trimmed))
        } else {
            hashed_text_embedding(trimmed)
        };

        let mut scored: Vec<SearchHit> = candidates
            .into_iter()
            .map(|(chunk, vec)| {
                let fts_bm25 = fts_scores.get(&chunk.id).copied().unwrap_or(0.0);
                let lexical_overlap = compute_lexical_bm25_boost(&query_tokens, &chunk.text);
                let bm25_score = fts_bm25 + lexical_overlap;
                let vector_score = cosine_similarity(&query_vec, &vec).max(0.0);
                let hybrid_score = 0.55 * (bm25_score / (1.0 + bm25_score)) + 0.45 * vector_score;
                SearchHit {
                    chunk,
                    bm25_score,
                    vector_score,
                    hybrid_score,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.hybrid_score
                .total_cmp(&a.hybrid_score)
                .then_with(|| a.chunk.page.cmp(&b.chunk.page))
                .then_with(|| a.chunk.chunk_index.cmp(&b.chunk.chunk_index))
        });
        scored.truncate(query.limit);
        Ok(scored)
    }
}

fn compute_vectors(
    chunks: &[Chunk],
    embedder: Option<&dyn AiProvider>,
) -> Result<Vec<Vec<f32>>, IndexError> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(provider) = embedder {
        let refs: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let vectors = provider.embed(&refs)?;
        if vectors.len() == chunks.len() {
            return Ok(vectors);
        }
    }
    Ok(chunks
        .iter()
        .map(|c| hashed_text_embedding(&c.text))
        .collect())
}

fn load_filtered_chunks(
    conn: &Connection,
    doc_ids: &[&str],
    page_range: Option<(usize, usize)>,
    chapter: Option<&str>,
) -> Result<Vec<(Chunk, Vec<f32>)>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT chunk_id, doc_id, page, chunk_index, chapter, text, quads_json, token_count, content_hash, vector_json
         FROM ai_chunks
         ORDER BY doc_id, page, chunk_index",
    )?;
    let chapter_needle = chapter.map(|c| c.to_ascii_lowercase());
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let doc_id: String = row.get(1)?;
        if !doc_ids.is_empty() && !doc_ids.iter().any(|d| *d == doc_id) {
            continue;
        }
        let page: usize = row.get(2)?;
        if let Some((start, end)) = page_range
            && (page < start || page > end)
        {
            continue;
        }
        let row_chapter: Option<String> = row.get(4)?;
        if let Some(needle) = &chapter_needle {
            let matches_chapter = row_chapter
                .as_deref()
                .is_some_and(|ch| ch.to_ascii_lowercase().contains(needle));
            if !matches_chapter {
                continue;
            }
        }
        let quads_json: String = row.get(6)?;
        let quad_arrays: Vec<[f32; 8]> = serde_json::from_str(&quads_json)?;
        let quads = quad_arrays
            .into_iter()
            .map(CitationQuad::from_array)
            .collect();
        let vector_json: String = row.get(9)?;
        let vector: Vec<f32> = serde_json::from_str(&vector_json)?;
        out.push((
            Chunk {
                id: row.get(0)?,
                doc_id,
                page,
                chunk_index: row.get(3)?,
                chapter: row_chapter,
                text: row.get(5)?,
                quads,
                token_count: row.get(7)?,
                content_hash: row.get(8)?,
            },
            vector,
        ));
    }
    Ok(out)
}

fn query_fts5_bm25(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<HashMap<String, f64>, IndexError> {
    let fts_expr = build_safe_fts5_query(query);
    if fts_expr.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare(
        "SELECT chunk_id, -bm25(ai_chunks_fts) AS score
         FROM ai_chunks_fts
         WHERE ai_chunks_fts MATCH ?1
         ORDER BY score DESC
         LIMIT ?2",
    )?;
    let mut rows = stmt.query(params![fts_expr, limit])?;
    let mut scores = HashMap::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let score: f64 = row.get(1)?;
        scores.insert(id, score.max(0.0));
    }
    Ok(scores)
}

fn build_safe_fts5_query(query: &str) -> String {
    let tokens = tokenize_normalized(query);
    if tokens.is_empty() {
        return String::new();
    }
    tokens
        .into_iter()
        .map(|tok| format!("\"{tok}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Produces a deterministic L2-normalized feature-hashed TF-IDF embedding over word unigrams,
/// bigrams, and character trigrams (used when no ONNX/remote embedding pack is active).
#[must_use]
pub fn hashed_text_embedding(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0f32; DEFAULT_EMBEDDING_DIM];
    let tokens = tokenize_normalized(text);
    if tokens.is_empty() {
        return vec;
    }

    let mut tf: BTreeMap<String, f32> = BTreeMap::new();
    for tok in &tokens {
        *tf.entry(format!("w:{tok}")).or_insert(0.0) += 1.0;
        let chars: Vec<char> = tok.chars().collect();
        if chars.len() >= 3 {
            for win in chars.windows(3) {
                let trigram: String = win.iter().collect();
                *tf.entry(format!("g:{trigram}")).or_insert(0.0) += 0.4;
            }
        }
    }
    for pair in tokens.windows(2) {
        *tf.entry(format!("b:{}_{}", pair[0], pair[1]))
            .or_insert(0.0) += 0.8;
    }

    for (feature, freq) in tf {
        let hash = fnv1a_64(feature.as_bytes());
        let bucket = usize::try_from(hash % (DEFAULT_EMBEDDING_DIM as u64)).unwrap_or(0);
        let sign = if ((hash >> 32) & 1) == 0 { 1.0 } else { -1.0 };
        let weight = 1.0 + freq.ln();
        vec[bucket] += sign * weight;
    }

    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let fx = f64::from(x);
        let fy = f64::from(y);
        dot += fx * fy;
        na += fx * fx;
        nb += fy * fy;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f64::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

fn compute_lexical_bm25_boost(query_tokens: &[String], chunk_text: &str) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let doc_tokens = tokenize_normalized(chunk_text);
    if doc_tokens.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for tok in &doc_tokens {
        *counts.entry(tok.as_str()).or_insert(0) += 1;
    }
    let k1 = 1.2f64;
    let len_norm = 0.75 * (doc_tokens.len() as f64 / 120.0) + 0.25;
    let mut unique_hits = 0usize;
    let mut score = 0.0f64;
    let mut seen = HashMap::new();
    for qt in query_tokens {
        if seen.insert(qt.as_str(), ()).is_some() {
            continue;
        }
        let tf = counts.get(qt.as_str()).copied().unwrap_or(0) as f64;
        if tf > 0.0 {
            unique_hits += 1;
            score += (tf * (k1 + 1.0)) / (tf + k1 * len_norm);
        }
    }
    let coverage = unique_hits as f64 / seen.len().max(1) as f64;
    score * (0.5 + coverage)
}

fn tokenize_normalized(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn compute_doc_hash(chunks: &[Chunk]) -> String {
    let mut hasher = Sha256::new();
    for c in chunks {
        hasher.update(c.id.as_bytes());
        hasher.update(b":");
        hasher.update(c.content_hash.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_preserves_pages_quads_and_overlap() {
        let mut lines = Vec::new();
        for i in 0..20 {
            lines.push(PageLineSpan {
                text: format!(
                    "Line {i}: attention mechanisms compute scaled dot product weights across keys and values."
                ),
                quads: vec![CitationQuad::from_rect(
                    40.0,
                    (i as f32) * 15.0,
                    500.0,
                    (i as f32) * 15.0 + 12.0,
                )],
            });
        }
        let pages = vec![PageSource {
            page: 2,
            lines,
            chapter: Some("Architecture".to_owned()),
        }];
        let cfg = ChunkConfig {
            target_tokens: 100,
            overlap_tokens: 25,
        };
        let chunks = chunk_document("paper-1", &pages, &cfg);
        assert!(
            chunks.len() >= 3,
            "expected overlapping chunks, got {}",
            chunks.len()
        );
        assert!(chunks.iter().all(|c| c.page == 2));
        assert!(chunks.iter().all(|c| !c.quads.is_empty()));
        assert_eq!(chunks[0].chapter.as_deref(), Some("Architecture"));
    }

    #[test]
    fn hybrid_sqlite_index_retrieves_relevant_pages_and_respects_filters() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite opens");
        let index = DocumentIndex::new();
        let pages = vec![
            PageSource::from_plain_text(
                0,
                "Introduction to FSRS spaced repetition memory stability and difficulty.",
                Some("Memory"),
            ),
            PageSource::from_plain_text(
                1,
                "MuPDF renders PDF pages using display lists and pixmap caches.",
                Some("Rendering"),
            ),
            PageSource::from_plain_text(
                2,
                "True redaction permanently strips underlying PDF content streams and metadata.",
                Some("Security"),
            ),
        ];
        let stats = index
            .index_document(
                &conn,
                "manual",
                &pages,
                &ChunkConfig::default(),
                None,
                1_700_000_000,
            )
            .expect("indexes document");
        assert_eq!(stats.chunk_count, 3);

        let hits = index
            .search(
                &conn,
                &SearchQuery::new("How does true redaction remove PDF content?", 5),
                None,
            )
            .expect("search succeeds");
        assert!(!hits.is_empty());
        assert_eq!(
            hits[0].chunk.page, 2,
            "page 2 redaction chunk should rank first"
        );

        let filtered = index
            .search(
                &conn,
                &SearchQuery::new("display lists", 5).with_chapter("Memory"),
                None,
            )
            .expect("filtered search succeeds");
        assert!(
            filtered
                .iter()
                .all(|h| h.chunk.chapter.as_deref() == Some("Memory")),
            "chapter filter must be respected"
        );
    }
}
