//! Global full-text search across folders and the library (plan F.2, F.3, F.5).
//!
//! Queries SQLite FTS5 (`ecclesia-store::fts`) for already-indexed documents, and incrementally
//! indexes unindexed or modified PDFs/Markdown/text files at background priority with cooperative
//! pause/cancellation, streaming results as indexing proceeds.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_core::annot::{Annotation, page_annotations};
use ecclesia_mupdf::Document;
use ecclesia_store::fts::{self, FileIndexState, FtsEntryInput, FtsHit, SearchFilters, SourceKind};
use ecclesia_store::{Result as StoreResult, Store};
use rusqlite::Connection;

use super::query::{ParsedQuery, QueryParseError};

/// Cooperative pause & cancellation handle for background indexing (plan F.3).
#[derive(Debug, Default)]
pub struct IndexControl {
    paused: AtomicBool,
    cancelled: AtomicBool,
}

impl IndexControl {
    /// Creates a new active control handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests the indexer to pause (for example while the user scrolls or inks).
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    /// Resumes background indexing after a pause.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns `true` if paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// Returns `true` if cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Summary produced by indexing a file or folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexStats {
    /// Files newly indexed or re-indexed.
    pub indexed_files: usize,
    /// Files skipped because size and modification time matched `fts_file_state`.
    pub skipped_files: usize,
    /// Total pages/entries written to FTS5.
    pub entries_written: usize,
    /// Files that failed to open or parse.
    pub failed_files: usize,
}

/// Global search error combining query parsing, store, and MuPDF failures.
#[derive(Debug)]
#[non_exhaustive]
pub enum SearchError {
    /// Query syntax error.
    Query(QueryParseError),
    /// SQLite error.
    Store(ecclesia_store::Error),
    /// Document parsing error.
    Document(ecclesia_mupdf::MuError),
    /// Filesystem I/O error.
    Io(std::io::Error),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query(err) => err.fmt(f),
            Self::Store(err) => err.fmt(f),
            Self::Document(err) => err.fmt(f),
            Self::Io(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Query(err) => Some(err),
            Self::Store(err) => Some(err),
            Self::Document(err) => Some(err),
            Self::Io(err) => Some(err),
        }
    }
}

impl From<QueryParseError> for SearchError {
    fn from(err: QueryParseError) -> Self {
        Self::Query(err)
    }
}

impl From<ecclesia_store::Error> for SearchError {
    fn from(err: ecclesia_store::Error) -> Self {
        Self::Store(err)
    }
}

impl From<ecclesia_mupdf::MuError> for SearchError {
    fn from(err: ecclesia_mupdf::MuError) -> Self {
        Self::Document(err)
    }
}

impl From<std::io::Error> for SearchError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Searches already-indexed content in `conn` using `query` (plan F.2, F.4).
///
/// # Errors
///
/// Returns [`SearchError::Query`] if `query` is malformed or [`SearchError::Store`] if SQLite fails.
pub fn search_indexed(
    conn: &Connection,
    query: &str,
    extra_filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<FtsHit>, SearchError> {
    let parsed = ParsedQuery::parse(query)?;
    search_parsed(conn, &parsed, extra_filters, limit)
}

/// Runs a pre-parsed query against `conn`.
///
/// # Errors
///
/// SQLite failed.
pub fn search_parsed(
    conn: &Connection,
    parsed: &ParsedQuery,
    extra_filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<FtsHit>, SearchError> {
    let filters = merge_filters(&parsed.filters, extra_filters);
    let candidate_limit = if parsed.needs_post_filter {
        limit.saturating_mul(8).max(200)
    } else {
        limit
    };

    let mut hits = if let Some(fts_match) = &parsed.fts_match {
        fts::search_fts(conn, fts_match, &filters, candidate_limit)?
    } else if let Some(sub) = &parsed.trigram_hint {
        fts::search_trigram(conn, sub, &filters, candidate_limit)?
    } else {
        fts::search_filtered(conn, &filters, candidate_limit)?
    };

    if parsed.needs_post_filter {
        hits.retain(|hit| parsed.matches_hit(hit));
        hits.truncate(limit);
    }
    Ok(hits)
}

/// Searches a folder incrementally: first returns indexed FTS5 results via `on_hit`, then scans any
/// unindexed or modified documents in `folder` and streams newly discovered matches immediately (plan F.2).
///
/// # Errors
///
/// Returns [`SearchError`] if `query` is invalid or database/IO fails.
pub fn search_folder_streaming(
    store: &Store,
    folder: &Path,
    query: &str,
    limit: usize,
    control: &IndexControl,
    mut on_hit: impl FnMut(FtsHit),
) -> Result<IndexStats, SearchError> {
    let parsed = ParsedQuery::parse(query)?;
    let prefix = folder.to_string_lossy().into_owned();
    let folder_filter = SearchFilters {
        path_prefix: Some(prefix),
        ..SearchFilters::default()
    };

    let initial_hits = store.read(|conn| {
        search_parsed(conn, &parsed, &folder_filter, limit).map_err(|e| match e {
            SearchError::Store(err) => err,
            other => ecclesia_store::Error::Io(std::io::Error::other(other.to_string())),
        })
    })?;
    let mut emitted_count = initial_hits.len();
    for hit in initial_hits {
        on_hit(hit);
    }

    let mut stats = IndexStats::default();
    let files = collect_indexable_files(folder)?;
    for path in files {
        if control.is_cancelled() {
            break;
        }
        while control.is_paused() && !control.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let indexed_now = index_file_if_needed(store, &path, &mut stats)?;
        if indexed_now && emitted_count < limit {
            let file_filter = SearchFilters {
                file_path: Some(path.clone()),
                ..SearchFilters::default()
            };
            let new_hits = store.read(|conn| {
                search_parsed(conn, &parsed, &file_filter, limit - emitted_count).map_err(|e| {
                    match e {
                        SearchError::Store(err) => err,
                        other => {
                            ecclesia_store::Error::Io(std::io::Error::other(other.to_string()))
                        }
                    }
                })
            })?;
            for hit in new_hits {
                emitted_count += 1;
                on_hit(hit);
                if emitted_count >= limit {
                    break;
                }
            }
        }
    }

    Ok(stats)
}

/// Indexes a single document file if it is unindexed or changed on disk (plan F.3).
/// Returns `Ok(true)` if the file was indexed, `Ok(false)` if skipped as up to date.
///
/// # Errors
///
/// Returns [`SearchError`] if file metadata or SQLite fails.
pub fn index_file_if_needed(
    store: &Store,
    path: &Path,
    stats: &mut IndexStats,
) -> Result<bool, SearchError> {
    let meta = fs::metadata(path)?;
    let size = meta.len();
    let modified_at = file_mtime_secs(&meta);
    let needed = store.read(|conn| fts::needs_reindex(conn, path, size, modified_at))?;
    if !needed {
        stats.skipped_files += 1;
        return Ok(false);
    }

    let (page_entries, annot_entries) = match extract_entries_for_file(path, modified_at) {
        Ok(pair) => pair,
        Err(_) => {
            stats.failed_files += 1;
            return Ok(false);
        }
    };

    let written = page_entries.len() + annot_entries.len();
    let owned_path = path.to_path_buf();
    let now = now_secs();
    store
        .write(move |tx| {
            fts::replace_file_entries(tx, &owned_path, SourceKind::Page, &page_entries)?;
            fts::replace_file_entries(tx, &owned_path, SourceKind::Annotation, &annot_entries)?;
            fts::set_file_index_state(
                tx,
                &owned_path,
                FileIndexState {
                    size,
                    modified_at,
                    indexed_at: now,
                },
            )?;
            Ok(())
        })
        .wait()?;

    stats.indexed_files += 1;
    stats.entries_written += written;
    Ok(true)
}

/// Indexes all supported documents inside `folder` recursively.
///
/// # Errors
///
/// Returns [`SearchError`] on filesystem or SQLite failures.
pub fn index_folder(
    store: &Store,
    folder: &Path,
    control: &IndexControl,
) -> Result<IndexStats, SearchError> {
    let mut stats = IndexStats::default();
    for path in collect_indexable_files(folder)? {
        if control.is_cancelled() {
            break;
        }
        while control.is_paused() && !control.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = index_file_if_needed(store, &path, &mut stats)?;
    }
    Ok(stats)
}

/// Indexes an arbitrary list of page texts and annotations directly (useful for in-memory or pre-extracted text).
///
/// # Errors
///
/// SQLite failed.
pub fn index_document_content(
    store: &Store,
    path: &Path,
    title: &str,
    pages: &[String],
    annotations: &[Annotation],
    size: u64,
    modified_at: i64,
) -> StoreResult<()> {
    let owned_path = path.to_path_buf();
    let page_entries: Vec<FtsEntryInput> = pages
        .iter()
        .enumerate()
        .map(|(idx, body)| FtsEntryInput {
            path: Some(owned_path.clone()),
            source_kind: SourceKind::Page,
            source_id: format!("page:{idx}"),
            page: idx,
            author: String::new(),
            color: String::new(),
            tags: Vec::new(),
            created_at: modified_at,
            quads_json: "[]".to_owned(),
            title: title.to_owned(),
            body: body.clone(),
        })
        .collect();
    let annot_entries: Vec<FtsEntryInput> = annotations
        .iter()
        .map(|a| annotation_to_fts_entry(&owned_path, title, a))
        .collect();
    let now = now_secs();
    store
        .write(move |tx| {
            fts::replace_file_entries(tx, &owned_path, SourceKind::Page, &page_entries)?;
            fts::replace_file_entries(tx, &owned_path, SourceKind::Annotation, &annot_entries)?;
            fts::set_file_index_state(
                tx,
                &owned_path,
                FileIndexState {
                    size,
                    modified_at,
                    indexed_at: now,
                },
            )?;
            Ok(())
        })
        .wait()
}

/// Counts live matching hits for every saved search in `library.db` (plan F.5).
///
/// # Errors
///
/// SQLite failed.
pub fn smart_folder_counts(conn: &Connection) -> Result<Vec<(i64, String, usize)>, SearchError> {
    let saved = fts::list_searches(conn)?;
    let mut counts = Vec::with_capacity(saved.len());
    for item in saved {
        let count = match ParsedQuery::parse(&item.query) {
            Ok(parsed) => search_parsed(conn, &parsed, &SearchFilters::default(), 10_000)?.len(),
            Err(_) => 0,
        };
        counts.push((item.id, item.name, count));
    }
    Ok(counts)
}

fn extract_entries_for_file(
    path: &Path,
    modified_at: i64,
) -> Result<(Vec<FtsEntryInput>, Vec<FtsEntryInput>), SearchError> {
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Document")
        .to_owned();
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if matches!(ext.as_str(), "md" | "txt") {
        let text = fs::read_to_string(path)?;
        let entry = FtsEntryInput {
            path: Some(path.to_path_buf()),
            source_kind: SourceKind::Page,
            source_id: "page:0".to_owned(),
            page: 0,
            author: String::new(),
            color: String::new(),
            tags: Vec::new(),
            created_at: modified_at,
            quads_json: "[]".to_owned(),
            title,
            body: text,
        };
        return Ok((vec![entry], Vec::new()));
    }

    let doc = Document::open(path)?;
    let page_count = doc.page_count()?;
    let is_pdf = doc.is_pdf();
    let mut page_entries = Vec::with_capacity(page_count);
    let mut annot_entries = Vec::new();
    for page_idx in 0..page_count {
        let page = doc.load_page(page_idx)?;
        let text = page.text(None)?.text()?;
        page_entries.push(FtsEntryInput {
            path: Some(path.to_path_buf()),
            source_kind: SourceKind::Page,
            source_id: format!("page:{page_idx}"),
            page: page_idx,
            author: String::new(),
            color: String::new(),
            tags: Vec::new(),
            created_at: modified_at,
            quads_json: "[]".to_owned(),
            title: title.clone(),
            body: text,
        });
        drop(page);
        if is_pdf && let Ok(annots) = page_annotations(&doc, page_idx) {
            for annot in annots {
                annot_entries.push(annotation_to_fts_entry(path, &title, &annot));
            }
        }
    }
    Ok((page_entries, annot_entries))
}

fn annotation_to_fts_entry(path: &Path, title: &str, annot: &Annotation) -> FtsEntryInput {
    let quote = annot
        .anchor
        .as_ref()
        .map(|q| q.exact.as_str())
        .unwrap_or("");
    let body = if annot.contents.trim().is_empty() {
        quote.to_owned()
    } else if quote.is_empty() {
        annot.contents.clone()
    } else {
        format!("{}\n{}", annot.contents, quote)
    };
    let color = annot
        .style
        .color
        .or(annot.fill)
        .map(|[r, g, b]| {
            format!(
                "#{:02x}{:02x}{:02x}",
                (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                (b.clamp(0.0, 1.0) * 255.0).round() as u8,
            )
        })
        .unwrap_or_default();
    let quads_json = serde_json::to_string(&match &annot.shape {
        ecclesia_core::annot::Shape::Markup(_, quads) => quads
            .iter()
            .map(|q| {
                [
                    q.ul.x, q.ul.y, q.ur.x, q.ur.y, q.ll.x, q.ll.y, q.lr.x, q.lr.y,
                ]
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    })
    .unwrap_or_else(|_| "[]".to_owned());

    FtsEntryInput {
        path: Some(path.to_path_buf()),
        source_kind: SourceKind::Annotation,
        source_id: annot.id.clone(),
        page: annot.page,
        author: annot.author.clone(),
        color,
        tags: annot.tags.clone(),
        created_at: annot.modified,
        quads_json,
        title: title.to_owned(),
        body,
    }
}

fn merge_filters(a: &SearchFilters, b: &SearchFilters) -> SearchFilters {
    let mut source_kinds = a.source_kinds.clone();
    for k in &b.source_kinds {
        if !source_kinds.contains(k) {
            source_kinds.push(*k);
        }
    }
    let mut tags = a.tags.clone();
    for t in &b.tags {
        if !tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            tags.push(t.clone());
        }
    }
    SearchFilters {
        source_kinds,
        author: b.author.clone().or_else(|| a.author.clone()),
        color: b.color.clone().or_else(|| a.color.clone()),
        tags,
        after: match (a.after, b.after) {
            (Some(x), Some(y)) => Some(x.max(y)),
            (x, y) => x.or(y),
        },
        before: match (a.before, b.before) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        },
        page_min: match (a.page_min, b.page_min) {
            (Some(x), Some(y)) => Some(x.max(y)),
            (x, y) => x.or(y),
        },
        page_max: match (a.page_max, b.page_max) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        },
        path_prefix: b.path_prefix.clone().or_else(|| a.path_prefix.clone()),
        file_path: b.file_path.clone().or_else(|| a.file_path.clone()),
        collection_id: b.collection_id.or(a.collection_id),
    }
}

fn collect_indexable_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        for entry in fs::read_dir(&cur)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_indexable_ext(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn is_indexable_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "pdf" | "xps" | "epub" | "cbz" | "fb2" | "mobi" | "html" | "md" | "txt"
            )
        })
}

fn file_mtime_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::query::create_saved_search;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ecclesia-search-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn streams_corpus_pdf_search_and_updates_smart_folders() {
        let dir = temp_dir("stream");
        let db_path = dir.join("library.db");
        let docs_dir = dir.join("docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");

        let corpus_pdf =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        fs::copy(&corpus_pdf, docs_dir.join("zlib-manual.pdf")).expect("copy pdf");
        fs::write(
            docs_dir.join("notes.md"),
            "Custom deflate stream calibration note.\n",
        )
        .expect("write md");

        let store = Store::open(&db_path).expect("open store");
        let ctrl = IndexControl::new();
        let mut streamed = Vec::new();
        let stats = search_folder_streaming(&store, &docs_dir, "deflate", 50, &ctrl, |hit| {
            streamed.push(hit);
        })
        .expect("stream search");

        assert_eq!(stats.indexed_files, 2);
        assert!(!streamed.is_empty(), "deflate hits streamed");

        // Second run skips unchanged files and returns indexed results immediately.
        let mut second = Vec::new();
        let stats2 = search_folder_streaming(&store, &docs_dir, "deflate", 50, &ctrl, |hit| {
            second.push(hit);
        })
        .expect("second");
        assert_eq!(stats2.skipped_files, 2);
        assert_eq!(stats2.indexed_files, 0);
        assert_eq!(second.len(), streamed.len());

        // Smart folders count live matches.
        store
            .write(|tx| {
                create_saved_search(tx, "Deflate hits", "deflate", 100)
                    .map_err(|e| ecclesia_store::Error::Io(std::io::Error::other(e.to_string())))?;
                Ok(())
            })
            .wait()
            .expect("save search");
        let counts = store
            .read(|conn| {
                smart_folder_counts(conn)
                    .map_err(|e| ecclesia_store::Error::Io(std::io::Error::other(e.to_string())))
            })
            .expect("counts");
        assert_eq!(counts.len(), 1);
        assert!(counts[0].2 > 0);

        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }
}
