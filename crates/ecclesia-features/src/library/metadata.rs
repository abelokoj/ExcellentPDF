//! Metadata extraction (PDF info, XMP, DOI, arXiv, ISBN, online lookup) and duplicate detection
//! (SHA-256 exact hash + DOI + MinHash text shingling) (plan F.7, F.8).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ecclesia_mupdf::Document;
use ecclesia_store::library::LibraryItem;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Explicit user consent for optional online metadata lookups (AGENTS §5, plan F.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OnlineConsent {
    /// Offline only: never perform network requests.
    #[default]
    Denied,
    /// User explicitly consented to querying Crossref / arXiv / Open Library.
    Allowed,
}

/// Error returned by online metadata lookup or response parsing (plan F.7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataError {
    /// Online lookup was refused because the user has not granted consent.
    ConsentRequired,
    /// The HTTP request or response read failed.
    Network(String),
    /// The remote service returned malformed JSON/XML or lacked expected fields.
    InvalidResponse(String),
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConsentRequired => f.write_str("online metadata lookup requires user consent"),
            Self::Network(msg) => write!(f, "metadata network error: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "invalid metadata response: {msg}"),
        }
    }
}

impl std::error::Error for MetadataError {}

/// Metadata extracted from a document file and/or online APIs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedMetadata {
    /// Document title.
    pub title: Option<String>,
    /// List of author names.
    pub authors: Vec<String>,
    /// Publication year.
    pub year: Option<i32>,
    /// Digital Object Identifier (`10.xxxx/...`).
    pub doi: Option<String>,
    /// arXiv identifier (`2301.00001` or `hep-th/9901001`).
    pub arxiv_id: Option<String>,
    /// ISBN-10 or ISBN-13 (digits / X).
    pub isbn: Option<String>,
    /// Journal, conference, or publisher name.
    pub venue: Option<String>,
    /// Abstract or description, if found.
    pub abstract_text: Option<String>,
}

/// How two library documents relate as duplicates (plan F.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DuplicateKind {
    /// Identical file bytes (same SHA-256 content hash).
    Exact,
    /// Same DOI across different files (e.g. preprint vs published PDF, or re-encoded copy).
    RelatedDoi,
    /// High textual overlap via MinHash shingles over the opening pages.
    NearText {
        /// Estimated Jaccard similarity in `[0.0, 1.0]`.
        similarity: f64,
    },
}

/// A pair of library items flagged as exact or near duplicates.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicatePair {
    /// First file path.
    pub first: PathBuf,
    /// Second file path.
    pub second: PathBuf,
    /// Classification of the duplicate relationship.
    pub kind: DuplicateKind,
}

/// Extracts metadata from a PDF (or other MuPDF-readable document) on disk:
/// reads `/Info` + XMP from raw bytes and scans the first 3 pages for DOI, arXiv id, ISBN, and title.
///
/// # Errors
///
/// Returns an [`io::Error`] if the file cannot be read or parsed.
pub fn extract_from_pdf(path: &Path) -> io::Result<ExtractedMetadata> {
    let bytes = fs::read(path)?;
    let mut meta = extract_pdf_bytes_metadata(&bytes);

    if let Ok(doc) = Document::open(path) {
        let count = doc.page_count().unwrap_or(0).min(3);
        let mut combined = String::new();
        for page_idx in 0..count {
            if let Ok(page) = doc.load_page(page_idx)
                && let Ok(stext) = page.text(None)
                && let Ok(text) = stext.text()
            {
                combined.push_str(&text);
                combined.push('\n');
            }
        }
        merge_text_identifiers(&mut meta, &combined);
        if meta.title.as_deref().is_none_or(str::is_empty) {
            meta.title = infer_title_from_text(&combined)
                .or_else(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned));
        }
    } else if meta.title.as_deref().is_none_or(str::is_empty) {
        meta.title = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned);
    }

    Ok(meta)
}

/// Builds a complete [`LibraryItem`] from a file on disk, including metadata, SHA-256 hash,
/// and MinHash text shingle signature over the opening pages.
///
/// # Errors
///
/// Returns an [`io::Error`] if the file cannot be read.
pub fn build_library_item(path: &Path, now_secs: i64) -> io::Result<LibraryItem> {
    let fs_meta = fs::metadata(path)?;
    let file_size = fs_meta.len();
    let modified_at = fs_meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let bytes = fs::read(path)?;
    let content_hash = sha256_bytes(&bytes);
    let mut extracted = extract_pdf_bytes_metadata(&bytes);
    let mut page_count = 0usize;
    let mut first_pages_text = String::new();

    if let Ok(doc) = Document::open(path) {
        page_count = doc.page_count().unwrap_or(0);
        for page_idx in 0..page_count.min(3) {
            if let Ok(page) = doc.load_page(page_idx)
                && let Ok(stext) = page.text(None)
                && let Ok(text) = stext.text()
            {
                first_pages_text.push_str(&text);
                first_pages_text.push('\n');
            }
        }
    }
    merge_text_identifiers(&mut extracted, &first_pages_text);
    let title = extracted
        .title
        .filter(|t| !t.trim().is_empty())
        .or_else(|| infer_title_from_text(&first_pages_text))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_owned()
        });
    let minhash = if first_pages_text.trim().is_empty() {
        Vec::new()
    } else {
        compute_minhash(&first_pages_text)
    };
    let meta_json = serde_json::json!({
        "venue": extracted.venue,
        "abstract": extracted.abstract_text,
    })
    .to_string();

    Ok(LibraryItem {
        path: path.to_path_buf(),
        title,
        authors: extracted.authors.join("; "),
        year: extracted.year,
        doi: extracted.doi.unwrap_or_default(),
        arxiv_id: extracted.arxiv_id.unwrap_or_default(),
        isbn: extracted.isbn.unwrap_or_default(),
        page_count,
        content_hash,
        minhash,
        file_size,
        modified_at,
        added_at: now_secs,
        meta_json,
        ..LibraryItem::default()
    })
}

/// Extracts metadata from raw PDF bytes (`/Info` fields and embedded XMP packet).
#[must_use]
pub fn extract_pdf_bytes_metadata(bytes: &[u8]) -> ExtractedMetadata {
    let text = String::from_utf8_lossy(bytes);
    let mut meta = ExtractedMetadata::default();

    // Parse XMP packet if embedded.
    if let Some(start) = text.find("<x:xmpmeta")
        && let Some(rel_end) = text[start..].find("</x:xmpmeta>")
    {
        let xmp = &text[start..start + rel_end + "</x:xmpmeta>".len()];
        parse_xmp_into(xmp, &mut meta);
    }

    // Fallback to PDF `/Info` dictionary string entries.
    if meta.title.is_none() {
        meta.title = extract_pdf_info_string(&text, "/Title");
    }
    if meta.authors.is_empty()
        && let Some(author) = extract_pdf_info_string(&text, "/Author")
    {
        meta.authors = split_authors(&author);
    }
    if meta.doi.is_none()
        && let Some(subject) = extract_pdf_info_string(&text, "/Subject")
    {
        meta.doi = extract_doi(&subject);
    }
    if meta.year.is_none()
        && let Some(date) = extract_pdf_info_string(&text, "/CreationDate")
    {
        meta.year = extract_pdf_date_year(&date);
    }

    meta
}

/// Scans `text` for DOI, arXiv id, ISBN, and publication year when missing in `meta`.
pub fn merge_text_identifiers(meta: &mut ExtractedMetadata, text: &str) {
    if meta.doi.is_none() {
        meta.doi = extract_doi(text);
    }
    if meta.arxiv_id.is_none() {
        meta.arxiv_id = extract_arxiv_id(text);
    }
    if meta.isbn.is_none() {
        meta.isbn = extract_isbn(text);
    }
    if meta.year.is_none() {
        meta.year = extract_year_from_text(text);
    }
}

/// Extracts the first valid DOI (`10.xxxx/...`) from `text`, stripping trailing punctuation.
#[must_use]
pub fn extract_doi(text: &str) -> Option<String> {
    let mut idx = 0;
    while let Some(pos) = text[idx..].find("10.") {
        let abs = idx + pos;
        let prev_ok = abs == 0
            || !text[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if prev_ok {
            let candidate = &text[abs..];
            let mut end = 0usize;
            for (offset, ch) in candidate.char_indices() {
                if ch.is_whitespace()
                    || matches!(ch, '"' | '\'' | '<' | '>' | '[' | ']' | '{' | '}')
                {
                    break;
                }
                end = offset + ch.len_utf8();
            }
            let raw = candidate[..end].trim_end_matches(['.', ',', ';', ':', ')', ']']);
            if let Some((prefix, suffix)) = raw.split_once('/')
                && prefix.len() >= 7
                && prefix[3..].chars().all(|c| c.is_ascii_digit() || c == '.')
                && prefix[3..].chars().take_while(|c| *c != '.').count() >= 4
                && !suffix.is_empty()
                && suffix.chars().any(|c| c.is_ascii_alphanumeric())
            {
                return Some(raw.to_owned());
            }
        }
        idx = abs + 3;
    }
    None
}

/// Extracts the first arXiv identifier (`arXiv:2301.12345v2`, `2301.12345`, or `hep-th/9901001`).
#[must_use]
pub fn extract_arxiv_id(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    // Prefer explicit `arxiv:` prefix first.
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("arxiv:") {
        let start = pos + found + "arxiv:".len();
        let token = text[start..]
            .trim_start()
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ')' | ']' | '>'))
            .next()
            .unwrap_or("");
        let clean = token.trim_end_matches('.');
        if is_valid_arxiv_id(clean) {
            return Some(clean.to_owned());
        }
        pos = start;
    }

    // Otherwise scan whitespace-separated tokens for YYMM.NNNNN[vN].
    for raw in text.split_whitespace() {
        let clean = raw.trim_matches(|c: char| {
            matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | '.' | '<' | '>')
        });
        if is_valid_modern_arxiv(clean) {
            return Some(clean.to_owned());
        }
    }
    None
}

fn is_valid_arxiv_id(s: &str) -> bool {
    if is_valid_modern_arxiv(s) {
        return true;
    }
    if let Some((cat, num)) = s.split_once('/') {
        let cat_ok = !cat.is_empty()
            && cat
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.');
        let num_base = strip_arxiv_version(num);
        return cat_ok && num_base.len() == 7 && num_base.chars().all(|c| c.is_ascii_digit());
    }
    false
}

fn is_valid_modern_arxiv(s: &str) -> bool {
    let base = strip_arxiv_version(s);
    let Some((yymm, seq)) = base.split_once('.') else {
        return false;
    };
    if yymm.len() != 4 || !yymm.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let month: u32 = yymm[2..4].parse().unwrap_or(0);
    (1..=12).contains(&month)
        && (4..=5).contains(&seq.len())
        && seq.chars().all(|c| c.is_ascii_digit())
}

fn strip_arxiv_version(s: &str) -> &str {
    if let Some((base, v)) = s.rsplit_once('v')
        && !v.is_empty()
        && v.chars().all(|c| c.is_ascii_digit())
    {
        base
    } else {
        s
    }
}

/// Extracts and check-digit-validates the first ISBN-13 or ISBN-10 found in `text`.
#[must_use]
pub fn extract_isbn(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find("isbn") {
        let start = search_from + found + 4;
        let window: String = text[start..].chars().take(32).collect();
        let normalized: String = window
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == ' ' || *c == 'X' || *c == 'x')
            .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
            .collect();
        if normalized.len() >= 13 {
            let cand13 = &normalized[..13];
            if is_valid_isbn13(cand13) {
                return Some(cand13.to_owned());
            }
        }
        if normalized.len() >= 10 {
            let cand10 = normalized[..10].to_ascii_uppercase();
            if is_valid_isbn10(&cand10) {
                return Some(cand10);
            }
        }
        search_from = start;
    }
    None
}

fn is_valid_isbn13(s: &str) -> bool {
    if s.len() != 13 || !s.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if !(s.starts_with("978") || s.starts_with("979")) {
        return false;
    }
    let sum: u32 = s
        .bytes()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 0 { d } else { d * 3 }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn is_valid_isbn10(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let mut sum = 0_u32;
    for (idx, b) in s.bytes().enumerate() {
        let val = if idx == 9 && b == b'X' {
            10
        } else if b.is_ascii_digit() {
            u32::from(b - b'0')
        } else {
            return false;
        };
        sum += val * u32::try_from(10 - idx).unwrap_or(0);
    }
    sum.is_multiple_of(11)
}

/// Fetches metadata for `doi` from Crossref when `consent == OnlineConsent::Allowed`.
///
/// # Errors
///
/// Returns [`MetadataError`] if consent was denied or the HTTP/JSON call failed.
pub fn fetch_crossref_metadata(
    doi: &str,
    consent: OnlineConsent,
) -> Result<ExtractedMetadata, MetadataError> {
    if consent != OnlineConsent::Allowed {
        return Err(MetadataError::ConsentRequired);
    }
    let url = format!("https://api.crossref.org/works/{}", url_encode(doi));
    let mut resp = ureq::get(&url)
        .header(
            "User-Agent",
            "EcclesiaPDF/0.1 (mailto:support@ecclesiapdf.example)",
        )
        .call()
        .map_err(|e| MetadataError::Network(format!("crossref request failed: {e}")))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| MetadataError::Network(format!("crossref body read failed: {e}")))?;
    parse_crossref_json(&body)
}

/// Parses a Crossref `/works/{doi}` JSON payload into [`ExtractedMetadata`].
///
/// # Errors
///
/// Returns [`MetadataError::InvalidResponse`] if the JSON does not contain a `message` object.
pub fn parse_crossref_json(json_str: &str) -> Result<ExtractedMetadata, MetadataError> {
    let val: Value = serde_json::from_str(json_str)
        .map_err(|e| MetadataError::InvalidResponse(format!("invalid crossref json: {e}")))?;
    let msg = val
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            MetadataError::InvalidResponse("missing `message` in crossref response".to_owned())
        })?;
    let title = msg
        .get("title")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    let authors = msg
        .get("author")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let given = a.get("given").and_then(Value::as_str).unwrap_or("");
                    let family = a.get("family").and_then(Value::as_str).unwrap_or("");
                    let full = format!("{given} {family}").trim().to_owned();
                    (!full.is_empty()).then_some(full)
                })
                .collect()
        })
        .unwrap_or_default();
    let year = msg
        .get("published")
        .or_else(|| msg.get("issued"))
        .and_then(|p| p.get("date-parts"))
        .and_then(Value::as_array)
        .and_then(|outer| outer.first())
        .and_then(Value::as_array)
        .and_then(|inner| inner.first())
        .and_then(Value::as_i64)
        .and_then(|y| i32::try_from(y).ok());
    let doi = msg.get("DOI").and_then(Value::as_str).map(str::to_owned);
    let venue = msg
        .get("container-title")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ExtractedMetadata {
        title,
        authors,
        year,
        doi,
        venue,
        ..ExtractedMetadata::default()
    })
}

/// Fetches metadata for `arxiv_id` from the arXiv Atom API when `consent == OnlineConsent::Allowed`.
///
/// # Errors
///
/// Returns [`MetadataError`] if consent was denied or the HTTP/XML call failed.
pub fn fetch_arxiv_metadata(
    arxiv_id: &str,
    consent: OnlineConsent,
) -> Result<ExtractedMetadata, MetadataError> {
    if consent != OnlineConsent::Allowed {
        return Err(MetadataError::ConsentRequired);
    }
    let url = format!(
        "https://export.arxiv.org/api/query?id_list={}",
        url_encode(arxiv_id)
    );
    let mut resp = ureq::get(&url)
        .call()
        .map_err(|e| MetadataError::Network(format!("arxiv request failed: {e}")))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| MetadataError::Network(format!("arxiv body read failed: {e}")))?;
    parse_arxiv_atom_xml(&body, arxiv_id)
}

/// Parses an arXiv Atom feed response into [`ExtractedMetadata`].
///
/// # Errors
///
/// Returns [`MetadataError::InvalidResponse`] if the XML cannot be parsed.
pub fn parse_arxiv_atom_xml(xml: &str, arxiv_id: &str) -> Result<ExtractedMetadata, MetadataError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_entry = false;
    let mut current_tag = String::new();
    let mut in_author = false;
    let mut meta = ExtractedMetadata {
        arxiv_id: Some(arxiv_id.to_owned()),
        ..ExtractedMetadata::default()
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_owned();
                if name == "entry" {
                    in_entry = true;
                } else if in_entry && name == "author" {
                    in_author = true;
                }
                current_tag = name;
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_owned();
                if name == "entry" {
                    break;
                }
                if name == "author" {
                    in_author = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(t)) if in_entry => {
                let txt = t.xml_content(XmlVersion::Implicit1_0);
                match current_tag.as_str() {
                    "title" if meta.title.is_none() => {
                        meta.title = Some(txt.split_whitespace().collect::<Vec<_>>().join(" "));
                    }
                    "name" if in_author => meta.authors.push(txt.into_owned()),
                    "published" if meta.year.is_none() => {
                        meta.year = txt.get(..4).and_then(|y| y.parse().ok());
                    }
                    "summary" if meta.abstract_text.is_none() => {
                        meta.abstract_text =
                            Some(txt.split_whitespace().collect::<Vec<_>>().join(" "));
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(MetadataError::InvalidResponse(format!(
                    "arxiv xml parse error: {e}"
                )));
            }
            _ => {}
        }
    }
    Ok(meta)
}

/// Fetches metadata for `isbn` from Open Library when `consent == OnlineConsent::Allowed`.
///
/// # Errors
///
/// Returns [`MetadataError`] if consent was denied or the HTTP/JSON call failed.
pub fn fetch_openlibrary_metadata(
    isbn: &str,
    consent: OnlineConsent,
) -> Result<ExtractedMetadata, MetadataError> {
    if consent != OnlineConsent::Allowed {
        return Err(MetadataError::ConsentRequired);
    }
    let key = format!("ISBN:{isbn}");
    let url = format!(
        "https://openlibrary.org/api/books?bibkeys={}&format=json&jscmd=data",
        url_encode(&key)
    );
    let mut resp = ureq::get(&url)
        .call()
        .map_err(|e| MetadataError::Network(format!("openlibrary request failed: {e}")))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| MetadataError::Network(format!("openlibrary body read failed: {e}")))?;
    parse_openlibrary_json(&body, isbn)
}

/// Parses an Open Library `jscmd=data` JSON response.
///
/// # Errors
///
/// Returns [`MetadataError::InvalidResponse`] if JSON parsing fails or the ISBN key is missing.
pub fn parse_openlibrary_json(
    json_str: &str,
    isbn: &str,
) -> Result<ExtractedMetadata, MetadataError> {
    let val: Value = serde_json::from_str(json_str)
        .map_err(|e| MetadataError::InvalidResponse(format!("invalid openlibrary json: {e}")))?;
    let key = format!("ISBN:{isbn}");
    let entry = val.get(&key).and_then(Value::as_object).ok_or_else(|| {
        MetadataError::InvalidResponse(format!("no Open Library record for {key}"))
    })?;
    let title = entry
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let authors = entry
        .get("authors")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let year = entry
        .get("publish_date")
        .and_then(Value::as_str)
        .and_then(extract_year_from_text);
    let venue = entry
        .get("publishers")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ExtractedMetadata {
        title,
        authors,
        year,
        isbn: Some(isbn.to_owned()),
        venue,
        ..ExtractedMetadata::default()
    })
}

/// Computes the lowercase SHA-256 hex digest of `bytes` (plan F.8).
#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Computes the lowercase SHA-256 hex digest of the file at `path`.
///
/// # Errors
///
/// Returns an [`io::Error`] if the file cannot be read.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

/// Number of MinHash permutations used for near-duplicate detection (plan F.8).
pub const MINHASH_LEN: usize = 64;

/// Computes a 64-element MinHash signature over normalized 3-word shingles of `text` (plan F.8).
#[must_use]
pub fn compute_minhash(text: &str) -> Vec<u64> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut shingles = BTreeSet::new();
    if words.len() < 3 {
        shingles.insert(fnv1a64(words.join(" ").as_bytes()));
    } else {
        for win in words.windows(3) {
            let s = format!("{} {} {}", win[0], win[1], win[2]);
            shingles.insert(fnv1a64(s.as_bytes()));
        }
    }

    let mut sig = vec![u64::MAX; MINHASH_LEN];
    for shingle in shingles {
        for (idx, slot) in sig.iter_mut().enumerate() {
            let h = splitmix64(
                shingle
                    ^ (u64::try_from(idx)
                        .unwrap_or(0)
                        .wrapping_mul(0x9e37_79b9_7f4a_7c15)),
            );
            if h < *slot {
                *slot = h;
            }
        }
    }
    sig
}

/// Estimates Jaccard similarity in `[0.0, 1.0]` between two MinHash signatures.
#[must_use]
pub fn minhash_similarity(a: &[u64], b: &[u64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / n as f64
}

/// Detects exact duplicates (same SHA-256), DOI-related versions (e.g. preprint vs published),
/// and near-duplicate text pairs (MinHash >= `min_similarity`) across `items` (plan F.8).
#[must_use]
pub fn detect_duplicates(items: &[LibraryItem], min_similarity: f64) -> Vec<DuplicatePair> {
    let mut pairs = Vec::new();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let a = &items[i];
            let b = &items[j];
            if !a.content_hash.is_empty() && a.content_hash == b.content_hash {
                pairs.push(DuplicatePair {
                    first: a.path.clone(),
                    second: b.path.clone(),
                    kind: DuplicateKind::Exact,
                });
            } else if !a.doi.is_empty() && a.doi.eq_ignore_ascii_case(&b.doi) {
                pairs.push(DuplicatePair {
                    first: a.path.clone(),
                    second: b.path.clone(),
                    kind: DuplicateKind::RelatedDoi,
                });
            } else {
                let sim = minhash_similarity(&a.minhash, &b.minhash);
                if sim >= min_similarity {
                    pairs.push(DuplicatePair {
                        first: a.path.clone(),
                        second: b.path.clone(),
                        kind: DuplicateKind::NearText { similarity: sim },
                    });
                }
            }
        }
    }
    pairs
}

fn parse_xmp_into(xmp: &str, meta: &mut ExtractedMetadata) {
    let mut reader = Reader::from_str(xmp);
    reader.config_mut().trim_text(true);
    let mut tag_stack: Vec<String> = Vec::new();
    let mut in_title = false;
    let mut in_creator = false;
    let mut in_description = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_owned();
                if name.ends_with("title") {
                    in_title = true;
                } else if name.ends_with("creator") {
                    in_creator = true;
                } else if name.ends_with("description") {
                    in_description = true;
                }
                tag_stack.push(name);
            }
            Ok(Event::End(e)) => {
                let qname = e.name();
                let name = qname.as_ref();
                if name.ends_with("title") {
                    in_title = false;
                } else if name.ends_with("creator") {
                    in_creator = false;
                } else if name.ends_with("description") {
                    in_description = false;
                }
                tag_stack.pop();
            }
            Ok(Event::Text(t)) => {
                let decoded = t.xml_content(XmlVersion::Implicit1_0);
                let txt = decoded.trim();
                if txt.is_empty() {
                    continue;
                }
                let cur = tag_stack.last().map(String::as_str).unwrap_or("");
                if in_title && meta.title.is_none() {
                    meta.title = Some(txt.to_owned());
                } else if in_creator {
                    meta.authors.push(txt.to_owned());
                } else if in_description && meta.abstract_text.is_none() {
                    meta.abstract_text = Some(txt.to_owned());
                } else if cur.ends_with("doi") && meta.doi.is_none() {
                    meta.doi = extract_doi(txt);
                } else if cur.ends_with("isbn") && meta.isbn.is_none() {
                    meta.isbn = extract_isbn(txt);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

fn extract_pdf_info_string(pdf_text: &str, key: &str) -> Option<String> {
    let idx = pdf_text.find(key)?;
    let rest = pdf_text[idx + key.len()..].trim_start();
    if let Some(after) = rest.strip_prefix('(') {
        let mut out = String::new();
        let mut escaped = false;
        for ch in after.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == ')' {
                return (!out.trim().is_empty()).then(|| out.trim().to_owned());
            } else {
                out.push(ch);
            }
        }
    }
    None
}

fn extract_pdf_date_year(date: &str) -> Option<i32> {
    let digits: String = date.chars().filter(char::is_ascii_digit).take(4).collect();
    if digits.len() == 4 {
        digits
            .parse::<i32>()
            .ok()
            .filter(|&y| (1900..=2100).contains(&y))
    } else {
        None
    }
}

fn extract_year_from_text(text: &str) -> Option<i32> {
    for tok in text.split(|c: char| !c.is_ascii_digit()) {
        if tok.len() == 4
            && let Ok(y) = tok.parse::<i32>()
            && (1950..=2050).contains(&y)
        {
            return Some(y);
        }
    }
    None
}

fn infer_title_from_text(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.len() >= 4 && line.len() <= 180)
        .map(str::to_owned)
}

fn split_authors(raw: &str) -> Vec<String> {
    let sep = if raw.contains(';') { ';' } else { ',' };
    raw.split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Groups exact duplicates by SHA-256 hash in O(N).
#[must_use]
pub fn group_exact_duplicates(items: &[LibraryItem]) -> BTreeMap<String, Vec<PathBuf>> {
    let mut by_hash: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for item in items {
        if !item.content_hash.is_empty() {
            by_hash
                .entry(item.content_hash.clone())
                .or_default()
                .push(item.path.clone());
        }
    }
    by_hash.retain(|_, paths| paths.len() > 1);
    by_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_doi_arxiv_and_isbn_with_validation() {
        let sample = "Preprint arXiv:2305.12345v2\nPublished at https://doi.org/10.1145/3295222.3295349.\nISBN 978-0-13-110362-7 and bad ISBN 978-0-13-110362-0.";
        assert_eq!(
            extract_doi(sample).as_deref(),
            Some("10.1145/3295222.3295349")
        );
        assert_eq!(extract_arxiv_id(sample).as_deref(), Some("2305.12345v2"));
        assert_eq!(extract_isbn(sample).as_deref(), Some("9780131103627"));
    }

    #[test]
    fn preprint_and_published_version_are_flagged_related_not_identical() {
        let preprint_text = "Attention Is All You Need. The dominant sequence transduction models are based on complex recurrent or convolutional neural networks that include an encoder and a decoder. We propose a new simple network architecture based solely on attention mechanisms.";
        let published_text = "Attention Is All You Need. The dominant sequence transduction models are based on complex recurrent or convolutional neural networks that include an encoder and a decoder. We propose a new simple network architecture based solely on attention mechanisms. Appears in NeurIPS 2017.";

        let preprint = LibraryItem {
            path: PathBuf::from("C:/papers/preprint.pdf"),
            content_hash: sha256_bytes(b"preprint-pdf-bytes"),
            minhash: compute_minhash(preprint_text),
            ..LibraryItem::default()
        };
        let published = LibraryItem {
            path: PathBuf::from("C:/papers/published.pdf"),
            content_hash: sha256_bytes(b"published-pdf-bytes"),
            minhash: compute_minhash(published_text),
            ..LibraryItem::default()
        };
        let exact_copy = LibraryItem {
            path: PathBuf::from("C:/papers/copy.pdf"),
            content_hash: sha256_bytes(b"published-pdf-bytes"),
            minhash: compute_minhash(published_text),
            ..LibraryItem::default()
        };

        let pairs = detect_duplicates(&[preprint, published, exact_copy], 0.55);
        assert!(
            pairs
                .iter()
                .any(|p| matches!(p.kind, DuplicateKind::NearText { .. }))
        );
        assert!(pairs.iter().any(|p| p.kind == DuplicateKind::Exact));
    }

    #[test]
    fn online_lookups_refuse_without_consent_and_parse_responses() {
        assert!(fetch_crossref_metadata("10.1000/test", OnlineConsent::Denied).is_err());
        assert!(fetch_arxiv_metadata("2305.12345", OnlineConsent::Denied).is_err());
        assert!(fetch_openlibrary_metadata("9780131103627", OnlineConsent::Denied).is_err());

        let cr = parse_crossref_json(
            r#"{"message":{"title":["Test Paper"],"DOI":"10.1000/xyz","author":[{"given":"Ada","family":"Lovelace"}],"issued":{"date-parts":[[2024]]},"container-title":["J. Comput."]}}"#,
        )
        .expect("crossref");
        assert_eq!(cr.title.as_deref(), Some("Test Paper"));
        assert_eq!(cr.authors, ["Ada Lovelace"]);
        assert_eq!(cr.year, Some(2024));
    }
}
