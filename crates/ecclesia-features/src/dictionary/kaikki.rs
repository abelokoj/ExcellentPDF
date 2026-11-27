//! Kaikki (Wiktionary) JSONL pack indexing and lookup (plan S.6), ported from SumatraPDF Enhanced's
//! `KaikkiDictionary.cpp`.
//!
//! Uses a 256-bucket FNV-1a 64-bit sorted index (`index.bin`) with `u64` byte offsets into `records.jsonl`, so packs
//! can exceed 4 GiB. Every lookup verifies each hash-matched JSONL record's normalized headword and `forms[].form`
//! entries so a hash collision can never return the wrong word.

use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde_json::{Value, json};

use super::index::{DictionarySource, OfflineMeaning, parse_online_dictionary};
use super::normalize::{percent_encode, word_hash, word_key};

/// Magic tag `"KAIK"` (`0x4b41494b`) stored in `index.bin` headers.
pub const INDEX_MAGIC: u32 = 0x4b41_494b;

/// Binary index format version (matching Enhanced's `kStoreVersion`).
pub const STORE_VERSION: u32 = 1;

/// Number of hash-prefix buckets (`hash >> 56`).
const BUCKET_COUNT: usize = 256;

/// Header size in bytes (`u32 + u32 + u64 + u64`).
pub const HEADER_BYTES: usize = 24;

/// Entry size in bytes (`u64 + u64 + u32 + u32`).
pub const ENTRY_BYTES: usize = 24;

/// Maximum JSONL line length (8 MiB, matching Enhanced).
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum total index size (2 GiB, matching Enhanced).
const MAX_INDEX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Maximum expanded `records.jsonl` size (16 GiB, matching Enhanced).
const MAX_EXPANDED_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Maximum candidate records inspected for one lookup hash.
const MAX_CANDIDATES: usize = 4_096;

/// Maximum matching JSONL records returned by one lookup.
const MAX_MATCHES: usize = 256;

/// Maximum combined bytes returned by one lookup (32 MiB).
const MAX_RESULT_BYTES: u64 = 32 * 1024 * 1024;

/// Metadata for a Kaikki downloadable pack (matching Enhanced's `KaikkiPack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KaikkiPack {
    /// Stable pack identifier.
    pub id: &'static str,
    /// Display title.
    pub title: &'static str,
    /// Download URL.
    pub url: &'static str,
    /// Attribution source page URL.
    pub source_url: &'static str,
    /// Content licence notice.
    pub license: &'static str,
    /// Approximate download size in bytes.
    pub download_bytes: u64,
    /// Approximate expanded JSONL size in bytes.
    pub expanded_bytes: u64,
    /// Whether the download stream is gzip-compressed.
    pub compressed: bool,
}

/// Built-in Kaikki English pack catalog (matching Enhanced's `kPacks`).
pub const KAIKKI_PACKS: [KaikkiPack; 2] = [
    KaikkiPack {
        id: "kaikki-english",
        title: "Kaikki English",
        url: "https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl",
        source_url: "https://kaikki.org/dictionary/English/index.html",
        license: "CC BY-SA 4.0 and GFDL; Wiktionary contributors",
        download_bytes: 3_335_559_018,
        expanded_bytes: 3_335_559_018,
        compressed: false,
    },
    KaikkiPack {
        id: "kaikki-simple-english",
        title: "Kaikki Simple English",
        url: "https://kaikki.org/dictionary/downloads/simple/simple-extract.jsonl.gz",
        source_url: "https://kaikki.org/simplewiktionary/",
        license: "CC BY-SA 4.0 and GFDL; Wiktionary contributors",
        download_bytes: 4_719_269,
        expanded_bytes: 37_900_246,
        compressed: true,
    },
];

/// Looks up a built-in [`KaikkiPack`] by id.
#[must_use]
pub fn find_kaikki_pack(id: &str) -> Option<&'static KaikkiPack> {
    KAIKKI_PACKS.iter().find(|pack| pack.id == id)
}

/// Errors produced while indexing, activating or querying a Kaikki store.
#[derive(Debug)]
#[non_exhaustive]
pub enum KaikkiError {
    /// Filesystem or gzip stream I/O error.
    Io(std::io::Error),
    /// Unknown pack identifier or unsafe path component.
    InvalidPack(String),
    /// Malformed JSONL record or corrupt binary index.
    Corrupt(String),
    /// Installation was cancelled by caller.
    Cancelled,
}

impl fmt::Display for KaikkiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "kaikki i/o error: {e}"),
            Self::InvalidPack(msg) => write!(f, "invalid kaikki pack: {msg}"),
            Self::Corrupt(msg) => write!(f, "corrupt kaikki dictionary: {msg}"),
            Self::Cancelled => f.write_str("dictionary installation cancelled"),
        }
    }
}

impl std::error::Error for KaikkiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::InvalidPack(_) | Self::Corrupt(_) | Self::Cancelled => None,
        }
    }
}

impl From<std::io::Error> for KaikkiError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Binary header of `index.bin` (24 bytes, little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreHeader {
    /// Must equal [`INDEX_MAGIC`].
    pub magic: u32,
    /// Must equal [`STORE_VERSION`].
    pub version: u32,
    /// Total number of 24-byte [`StoreEntry`] records following this header.
    pub count: u64,
    /// Exact byte length of the paired `records.jsonl` file.
    pub data_bytes: u64,
}

impl StoreHeader {
    /// Encodes the header as 24 little-endian bytes.
    #[must_use]
    pub fn to_le_bytes(self) -> [u8; HEADER_BYTES] {
        let mut out = [0_u8; HEADER_BYTES];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..8].copy_from_slice(&self.version.to_le_bytes());
        out[8..16].copy_from_slice(&self.count.to_le_bytes());
        out[16..24].copy_from_slice(&self.data_bytes.to_le_bytes());
        out
    }

    /// Decodes a 24-byte little-endian header.
    #[must_use]
    pub fn from_le_bytes(buf: [u8; HEADER_BYTES]) -> Self {
        Self {
            magic: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            version: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            count: u64::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]),
            data_bytes: u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]),
        }
    }
}

/// One 24-byte sorted entry in `index.bin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreEntry {
    /// FNV-1a 64-bit hash of the normalized word key.
    pub hash: u64,
    /// Byte offset of the JSONL line inside `records.jsonl` (`u64`, supports > 4 GiB).
    pub offset: u64,
    /// Byte length of the JSONL line (excluding trailing `\n`).
    pub bytes: u32,
    /// Reserved zero field.
    pub reserved: u32,
}

impl StoreEntry {
    /// Encodes as 24 little-endian bytes.
    #[must_use]
    pub fn to_le_bytes(self) -> [u8; ENTRY_BYTES] {
        let mut out = [0_u8; ENTRY_BYTES];
        out[0..8].copy_from_slice(&self.hash.to_le_bytes());
        out[8..16].copy_from_slice(&self.offset.to_le_bytes());
        out[16..20].copy_from_slice(&self.bytes.to_le_bytes());
        out[20..24].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }

    /// Decodes 24 little-endian bytes.
    #[must_use]
    pub fn from_le_bytes(buf: [u8; ENTRY_BYTES]) -> Self {
        Self {
            hash: u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]),
            offset: u64::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]),
            bytes: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            reserved: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        }
    }
}

/// Validates a generation folder name (`gen-<16 lower hex>-<8 lower hex>`, 29 bytes), rejecting path traversal.
#[must_use]
pub fn is_valid_generation(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() != 29 || !name.starts_with("gen-") || bytes[20] != b'-' {
        return false;
    }
    bytes[4..]
        .iter()
        .enumerate()
        .all(|(idx, &b)| idx == 16 || matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Parses a single Kaikki JSONL record and extracts its normalized index keys (`/word` and `/forms[*]/form`).
///
/// Hard-redirect records (`{"redirect": ...}`) return `Ok(Vec::new())` (valid JSONL, zero indexed headwords).
///
/// # Errors
///
/// Returns [`KaikkiError::Corrupt`] if `line` is not a valid JSON object or has neither `word` nor `redirect`.
pub fn extract_record_words(line: &str) -> Result<Vec<String>, KaikkiError> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(KaikkiError::Corrupt(
            "JSONL record is not a JSON object".to_owned(),
        ));
    }
    let val: Value = serde_json::from_str(trimmed)
        .map_err(|e| KaikkiError::Corrupt(format!("invalid JSONL record: {e}")))?;
    let Some(obj) = val.as_object() else {
        return Err(KaikkiError::Corrupt(
            "JSONL record is not a JSON object".to_owned(),
        ));
    };

    let has_redirect = obj.get("redirect").and_then(Value::as_str).is_some();
    let headword = obj.get("word").and_then(Value::as_str);
    if headword.is_none() && !has_redirect {
        return Err(KaikkiError::Corrupt(
            "JSONL record lacks both word and redirect".to_owned(),
        ));
    }

    let mut keys = Vec::new();
    let mut push_unique = |raw: &str| {
        if let Some(key) = word_key(raw)
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    };
    if let Some(hw) = headword {
        push_unique(hw);
    }
    if let Some(forms) = obj.get("forms").and_then(Value::as_array) {
        for form_obj in forms {
            if let Some(form_str) = form_obj.get("form").and_then(Value::as_str) {
                push_unique(form_str);
            }
        }
    }
    Ok(keys)
}

/// Streaming 256-bucket Kaikki index builder producing `records.jsonl` and sorted `index.bin` (plan S.6).
#[derive(Debug)]
pub struct KaikkiStoreBuilder {
    generation_dir: PathBuf,
    data_file: File,
    buckets: Vec<Option<File>>,
    bucket_counts: [u64; BUCKET_COUNT],
    data_bytes: u64,
    expanded_bytes: u64,
    count: u64,
    line_buf: Vec<u8>,
}

impl KaikkiStoreBuilder {
    /// Creates `records.jsonl` inside `generation_dir` and prepares 256 hash buckets.
    ///
    /// # Errors
    ///
    /// Returns [`KaikkiError::Io`] if the folder or output file cannot be created.
    pub fn create(generation_dir: &Path) -> Result<Self, KaikkiError> {
        fs::create_dir_all(generation_dir)?;
        let data_file = File::create(generation_dir.join("records.jsonl"))?;
        let mut buckets = Vec::with_capacity(BUCKET_COUNT);
        buckets.resize_with(BUCKET_COUNT, || None);
        Ok(Self {
            generation_dir: generation_dir.to_path_buf(),
            data_file,
            buckets,
            bucket_counts: [0; BUCKET_COUNT],
            data_bytes: 0,
            expanded_bytes: 0,
            count: 0,
            line_buf: Vec::new(),
        })
    }

    /// Feeds an arbitrary chunk of uncompressed UTF-8 JSONL bytes (works even byte-by-byte).
    ///
    /// # Errors
    ///
    /// Returns [`KaikkiError::Corrupt`] on malformed JSONL or size-limit overflow, or [`KaikkiError::Io`].
    pub fn feed(&mut self, mut bytes: &[u8]) -> Result<(), KaikkiError> {
        self.expanded_bytes = self
            .expanded_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| KaikkiError::Corrupt("expanded dictionary overflow".to_owned()))?;
        if self.expanded_bytes > MAX_EXPANDED_BYTES {
            return Err(KaikkiError::Corrupt(
                "expanded dictionary exceeds 16 GiB".to_owned(),
            ));
        }
        while !bytes.is_empty() {
            let nl = bytes.iter().position(|&b| b == b'\n');
            let take = nl.unwrap_or(bytes.len());
            if self.line_buf.len().saturating_add(take) > MAX_LINE_BYTES {
                return Err(KaikkiError::Corrupt(
                    "dictionary record exceeds 8 MiB".to_owned(),
                ));
            }
            self.line_buf.extend_from_slice(&bytes[..take]);
            if nl.is_some() {
                self.flush_line()?;
                bytes = &bytes[take + 1..];
            } else {
                break;
            }
        }
        Ok(())
    }

    fn flush_line(&mut self) -> Result<(), KaikkiError> {
        if self.line_buf.last() == Some(&b'\r') {
            self.line_buf.pop();
        }
        if self.line_buf.is_empty() {
            return Err(KaikkiError::Corrupt(
                "dictionary contains an empty JSONL record".to_owned(),
            ));
        }
        let record_str = std::str::from_utf8(&self.line_buf)
            .map_err(|_| KaikkiError::Corrupt("dictionary record is not valid UTF-8".to_owned()))?;
        let words = extract_record_words(record_str)?;
        if words.is_empty() {
            self.line_buf.clear();
            return Ok(());
        }
        let rec_len = self.line_buf.len() as u64;
        if self.data_bytes.saturating_add(rec_len + 1) > MAX_EXPANDED_BYTES {
            return Err(KaikkiError::Corrupt(
                "expanded dictionary exceeds 16 GiB".to_owned(),
            ));
        }
        self.data_file.write_all(&self.line_buf)?;
        self.data_file.write_all(b"\n")?;
        let offset = self.data_bytes;
        let bytes_u32 = u32::try_from(self.line_buf.len())
            .map_err(|_| KaikkiError::Corrupt("record exceeds u32 length".to_owned()))?;

        for word in words {
            let hash = word_hash(&word);
            let entry = StoreEntry {
                hash,
                offset,
                bytes: bytes_u32,
                reserved: 0,
            };
            let bucket_idx = (hash >> 56) as usize;
            if (self.count + 1).saturating_mul(ENTRY_BYTES as u64) > MAX_INDEX_BYTES {
                return Err(KaikkiError::Corrupt(
                    "dictionary index exceeds 2 GiB limit".to_owned(),
                ));
            }
            let bucket_file = match &mut self.buckets[bucket_idx] {
                Some(file) => file,
                slot @ None => {
                    let path = bucket_path(&self.generation_dir, bucket_idx);
                    *slot = Some(File::create(path)?);
                    slot.as_mut().expect("just inserted")
                }
            };
            bucket_file.write_all(&entry.to_le_bytes())?;
            self.bucket_counts[bucket_idx] += 1;
            self.count += 1;
        }

        self.data_bytes += rec_len + 1;
        self.line_buf.clear();
        Ok(())
    }

    /// Finalizes `records.jsonl`, sorts each of the 256 buckets by `(hash, offset)`, writes `index.bin`, cleans up
    /// temporary bucket files and returns the finalized [`StoreHeader`].
    ///
    /// # Errors
    ///
    /// Returns [`KaikkiError::Cancelled`] if `is_cancelled()` returns `true`, [`KaikkiError::Corrupt`] if no words
    /// were indexed, or [`KaikkiError::Io`].
    pub fn finish(
        mut self,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<StoreHeader, KaikkiError> {
        if !self.line_buf.is_empty() {
            self.flush_line()?;
        }
        if self.count == 0 {
            return Err(KaikkiError::Corrupt(
                "dictionary contains no words".to_owned(),
            ));
        }
        self.data_file.flush()?;
        drop(self.data_file);
        // Close all open bucket writers before reading them back.
        for slot in &mut self.buckets {
            if let Some(mut f) = slot.take() {
                f.flush()?;
            }
        }

        let header = StoreHeader {
            magic: INDEX_MAGIC,
            version: STORE_VERSION,
            count: self.count,
            data_bytes: self.data_bytes,
        };
        let mut index_file = File::create(self.generation_dir.join("index.bin"))?;
        index_file.write_all(&header.to_le_bytes())?;

        for (bucket_idx, &bucket_count) in self.bucket_counts.iter().enumerate() {
            if is_cancelled() {
                return Err(KaikkiError::Cancelled);
            }
            if bucket_count == 0 {
                continue;
            }
            let bpath = bucket_path(&self.generation_dir, bucket_idx);
            let raw = fs::read(&bpath)?;
            let expected_len = usize::try_from(bucket_count)
                .ok()
                .and_then(|c| c.checked_mul(ENTRY_BYTES))
                .ok_or_else(|| KaikkiError::Corrupt("bucket count overflow".to_owned()))?;
            if raw.len() != expected_len {
                return Err(KaikkiError::Corrupt("bucket size mismatch".to_owned()));
            }
            let mut entries = Vec::with_capacity(bucket_count as usize);
            let (chunks, _) = raw.as_chunks::<ENTRY_BYTES>();
            for &arr in chunks {
                entries.push(StoreEntry::from_le_bytes(arr));
            }
            entries
                .sort_unstable_by(|a, b| a.hash.cmp(&b.hash).then_with(|| a.offset.cmp(&b.offset)));
            for entry in entries {
                index_file.write_all(&entry.to_le_bytes())?;
            }
            let _ = fs::remove_file(&bpath);
        }
        index_file.flush()?;
        Ok(header)
    }
}

/// Compression format of an incoming Kaikki JSONL byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KaikkiStreamKind {
    /// Uncompressed UTF-8 JSONL.
    Plain,
    /// Gzip-compressed UTF-8 JSONL (`.jsonl.gz`).
    Gzip,
}

/// Builds, manifests and atomically activates a Kaikki pack generation inside `root_dir/<pack_id>/<generation>`
/// (plan S.6).
///
/// # Errors
///
/// Returns [`KaikkiError`] if the pack id or generation name is invalid, decompression/JSONL fails, or writing fails.
pub fn build_kaikki_store(
    pack: &KaikkiPack,
    root_dir: &Path,
    generation_name: &str,
    stream: &[u8],
    kind: KaikkiStreamKind,
) -> Result<StoreHeader, KaikkiError> {
    if !is_valid_generation(generation_name) {
        return Err(KaikkiError::InvalidPack(format!(
            "invalid generation name: {generation_name}"
        )));
    }
    let pack_dir = root_dir.join(pack.id);
    let gen_dir = pack_dir.join(generation_name);
    let mut builder = KaikkiStoreBuilder::create(&gen_dir)?;

    match kind {
        KaikkiStreamKind::Plain => builder.feed(stream)?,
        KaikkiStreamKind::Gzip => {
            let mut decoder = GzDecoder::new(stream);
            let mut buf = [0_u8; 64 * 1024];
            loop {
                let n = decoder.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                builder.feed(&buf[..n])?;
            }
        }
    }

    let header = builder.finish(|| false)?;
    let manifest = json!({
        "format": "sumatra-kaikki-jsonl",
        "version": STORE_VERSION,
        "pack": pack.id,
        "source": pack.source_url,
        "url": pack.url,
        "license": pack.license,
        "download_bytes": stream.len(),
        "data_bytes": header.data_bytes,
        "index_entries": header.count,
    });
    fs::write(gen_dir.join("manifest.json"), format!("{manifest}\n"))?;

    let old_gen = current_generation(&pack_dir);
    let pending = gen_dir.join("current.tmp");
    fs::write(&pending, generation_name.as_bytes())?;
    let current = pack_dir.join("current");
    let _ = fs::remove_file(&current);
    fs::rename(&pending, &current)?;
    if let Some(old) = old_gen
        && old != generation_name
    {
        let _ = fs::remove_dir_all(pack_dir.join(old));
    }
    Ok(header)
}

/// Returns whether a Kaikki pack is installed and its `index.bin` and `records.jsonl` headers are valid.
#[must_use]
pub fn kaikki_pack_installed(pack_id: &str, root_dir: &Path) -> bool {
    if find_kaikki_pack(pack_id).is_none() {
        return false;
    }
    let pack_dir = root_dir.join(pack_id);
    let Some(gen_name) = current_generation(&pack_dir) else {
        return false;
    };
    open_store(&pack_dir.join(gen_name)).is_ok()
}

/// Removes an installed Kaikki pack generation and its `current` pointer.
///
/// # Errors
///
/// Returns [`KaikkiError::InvalidPack`] for unknown/unsafe pack ids, or [`KaikkiError::Io`].
pub fn remove_kaikki_pack(pack_id: &str, root_dir: &Path) -> Result<(), KaikkiError> {
    if find_kaikki_pack(pack_id).is_none() {
        return Err(KaikkiError::InvalidPack(format!(
            "unknown Kaikki dictionary: {pack_id}"
        )));
    }
    let pack_dir = root_dir.join(pack_id);
    let old = current_generation(&pack_dir);
    let current = pack_dir.join("current");
    if current.exists() {
        fs::remove_file(&current)?;
    }
    if let Some(gen_name) = old {
        let gen_dir = pack_dir.join(gen_name);
        if gen_dir.exists() {
            fs::remove_dir_all(&gen_dir)?;
        }
    }
    Ok(())
}

/// Looks up matching Kaikki JSONL records for `query` using binary search on `index.bin` and exact normalized-key
/// collision verification (plan S.6).
///
/// # Errors
///
/// Returns [`KaikkiError::InvalidPack`] for unknown pack ids, or [`KaikkiError::Corrupt`] if `index.bin` or
/// `records.jsonl` is damaged.
pub fn lookup_kaikki_records(
    pack_id: &str,
    root_dir: &Path,
    query: &str,
) -> Result<Vec<String>, KaikkiError> {
    if find_kaikki_pack(pack_id).is_none() {
        return Err(KaikkiError::InvalidPack(format!(
            "unknown Kaikki dictionary: {pack_id}"
        )));
    }
    let pack_dir = root_dir.join(pack_id);
    let Some(gen_name) = current_generation(&pack_dir) else {
        return Ok(Vec::new());
    };
    let (mut index_file, mut data_file, header) = open_store(&pack_dir.join(gen_name))?;
    let Some(target_key) = word_key(query) else {
        return Ok(Vec::new());
    };
    let target_hash = word_hash(&target_key);

    // Lower-bound binary search over the sorted 24-byte entries in `index.bin`.
    let mut low = 0_u64;
    let mut high = header.count;
    while low < high {
        let mid = low + (high - low) / 2;
        let entry = read_entry(&mut index_file, header, mid)?;
        if entry.hash < target_hash {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    let mut records = Vec::new();
    let mut last_offset = u64::MAX;
    let mut result_bytes = 0_u64;
    let mut candidates = 0_usize;

    while low < header.count {
        let entry = read_entry(&mut index_file, header, low)?;
        low += 1;
        if entry.hash != target_hash {
            break;
        }
        candidates += 1;
        if candidates > MAX_CANDIDATES {
            return Err(KaikkiError::Corrupt(
                "too many dictionary index candidates".to_owned(),
            ));
        }
        if entry.offset == last_offset {
            continue;
        }
        last_offset = entry.offset;

        data_file.seek(SeekFrom::Start(entry.offset))?;
        let mut buf = vec![0_u8; entry.bytes as usize];
        data_file.read_exact(&mut buf)?;
        let record = String::from_utf8(buf)
            .map_err(|_| KaikkiError::Corrupt("dictionary record is not UTF-8".to_owned()))?;
        let words = extract_record_words(&record)?;
        // Hash collisions can never return the wrong word.
        if !words.iter().any(|w| w == &target_key) {
            continue;
        }
        result_bytes = result_bytes.saturating_add(u64::from(entry.bytes));
        if result_bytes > MAX_RESULT_BYTES {
            return Err(KaikkiError::Corrupt(
                "dictionary results exceed 32 MiB lookup limit".to_owned(),
            ));
        }
        records.push(record);
        if records.len() >= MAX_MATCHES {
            break;
        }
    }
    Ok(records)
}

/// Parses one Kaikki JSONL record and appends deduplicated [`OfflineMeaning`] entries attributed to `pack`.
///
/// # Errors
///
/// Returns `false` if the JSONL record is malformed.
#[must_use]
pub fn parse_kaikki_record(
    record: &str,
    word: &str,
    pack: &KaikkiPack,
    out: &mut Vec<OfflineMeaning>,
) -> bool {
    let mut parsed = Vec::new();
    if !parse_online_dictionary(record, word, DictionarySource::Wiktionary, &mut parsed) {
        return false;
    }
    let base_url = if pack.id == "kaikki-simple-english" {
        "https://simple.wiktionary.org/wiki/"
    } else {
        "https://en.wiktionary.org/wiki/"
    };
    for mut meaning in parsed {
        if out.len() >= 64 {
            break;
        }
        let duplicate = out.iter().any(|existing| {
            existing.dictionary_id == pack.id
                && existing.headword == meaning.headword
                && existing.part_of_speech == meaning.part_of_speech
                && existing.definition == meaning.definition
        });
        if duplicate {
            continue;
        }
        meaning.dictionary = pack.title.to_owned();
        meaning.dictionary_id = pack.id.to_owned();
        meaning.license = pack.license.to_owned();
        meaning.source_url = format!("{base_url}{}", percent_encode(&meaning.headword));
        out.push(meaning);
    }
    true
}

fn bucket_path(gen_dir: &Path, bucket: usize) -> PathBuf {
    gen_dir.join(format!("bucket-{bucket:02x}.tmp"))
}

fn current_generation(pack_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(pack_dir.join("current")).ok()?;
    let name = raw.trim();
    is_valid_generation(name).then(|| name.to_owned())
}

fn open_store(gen_dir: &Path) -> Result<(BufReader<File>, File, StoreHeader), KaikkiError> {
    let index_path = gen_dir.join("index.bin");
    let data_path = gen_dir.join("records.jsonl");
    let index_meta = fs::metadata(&index_path)?;
    let data_meta = fs::metadata(&data_path)?;
    let mut index_file = File::open(&index_path)?;
    let mut hdr_buf = [0_u8; HEADER_BYTES];
    index_file.read_exact(&mut hdr_buf)?;
    let header = StoreHeader::from_le_bytes(hdr_buf);
    let expected_index_size = (HEADER_BYTES as u64)
        .checked_add(
            header
                .count
                .checked_mul(ENTRY_BYTES as u64)
                .ok_or_else(|| KaikkiError::Corrupt("index entry count overflow".to_owned()))?,
        )
        .ok_or_else(|| KaikkiError::Corrupt("index size overflow".to_owned()))?;
    if header.magic != INDEX_MAGIC
        || header.version != STORE_VERSION
        || header.count == 0
        || header.count > MAX_INDEX_BYTES / (ENTRY_BYTES as u64)
        || header.data_bytes == 0
        || header.data_bytes > MAX_EXPANDED_BYTES
        || index_meta.len() != expected_index_size
        || data_meta.len() != header.data_bytes
    {
        return Err(KaikkiError::Corrupt(
            "dictionary index header or file size mismatch".to_owned(),
        ));
    }
    let data_file = File::open(&data_path)?;
    Ok((BufReader::new(index_file), data_file, header))
}

fn read_entry(
    index_file: &mut BufReader<File>,
    header: StoreHeader,
    pos: u64,
) -> Result<StoreEntry, KaikkiError> {
    if pos >= header.count {
        return Err(KaikkiError::Corrupt(
            "index position out of range".to_owned(),
        ));
    }
    let seek_pos = (HEADER_BYTES as u64) + pos * (ENTRY_BYTES as u64);
    index_file.seek(SeekFrom::Start(seek_pos))?;
    let mut buf = [0_u8; ENTRY_BYTES];
    index_file.read_exact(&mut buf)?;
    let entry = StoreEntry::from_le_bytes(buf);
    if entry.reserved != 0
        || entry.bytes == 0
        || (entry.bytes as usize) > MAX_LINE_BYTES
        || entry.offset > header.data_bytes
        || u64::from(entry.bytes) > header.data_bytes - entry.offset
    {
        return Err(KaikkiError::Corrupt(
            "invalid dictionary index entry".to_owned(),
        ));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "ecclesiapdf-kaikki-{label}-{}-{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Port of SumatraPDF Enhanced's `KaikkiDictionary_UnitTests()`: indexing, byte-by-byte streaming, gzip,
    /// cancellation, path-traversal guards, collision filtering and u64 offsets.
    #[test]
    fn matches_enhanced_kaikki_unit_tests() {
        let root = TempDir::new("main");
        let gen_name = "gen-0000000000000001-00000001";
        let fixture = concat!(
            "{\"word\":\"Cat\",\"forms\":[{\"form\":\"cats\"}],\"senses\":[{\"glosses\":[\"animal\"]}]}\n",
            "{\"word\":\"cat\",\"senses\":[{\"glosses\":[\"person\"]}]}\n",
            "{\"word\":\"!\",\"pos\":\"symbol\",\"senses\":[{\"glosses\":[\"exclamation\"]}]}\n",
            "{\"word\":\";\",\"pos\":\"symbol\"}\n",
            "{\"word\":\")\",\"pos\":\"symbol\"}\n",
            "{\"word\":\"(\",\"pos\":\"symbol\"}\n",
            "{\"word\":\"\\\"\",\"pos\":\"symbol\"}\n"
        );

        // Byte-by-byte streaming build.
        let pack = &KAIKKI_PACKS[0];
        let gen_dir = root.0.join(pack.id).join(gen_name);
        let mut builder = KaikkiStoreBuilder::create(&gen_dir).expect("create");
        for byte in fixture.as_bytes() {
            builder.feed(std::slice::from_ref(byte)).expect("feed byte");
        }
        builder.finish(|| false).expect("finish");
        fs::write(root.0.join(pack.id).join("current"), gen_name).expect("activate");

        assert!(kaikki_pack_installed(pack.id, &root.0));
        assert_eq!(
            lookup_kaikki_records(pack.id, &root.0, "CAT")
                .expect("lookup CAT")
                .len(),
            2
        );
        assert_eq!(
            lookup_kaikki_records(pack.id, &root.0, "cats")
                .expect("lookup cats")
                .len(),
            1
        );
        assert!(
            lookup_kaikki_records(pack.id, &root.0, "missing")
                .expect("lookup missing")
                .is_empty()
        );

        // Path traversal and malformed JSONL rejection.
        assert!(remove_kaikki_pack("../bad", &root.0).is_err());
        assert!(!is_valid_generation("gen-../../../../../../../../../"));
        assert!(extract_record_words("{\"word\":\"cat\"").is_err());
        assert!(
            extract_record_words(
                "{\"title\":\"Main page\",\"redirect\":\"Main Page\",\"pos\":\"hard-redirect\"}"
            )
            .expect("redirect valid")
            .is_empty()
        );

        // Cancellation leaves prior active generation intact.
        let cancel_dir = root.0.join(pack.id).join("gen-0000000000000002-00000001");
        let mut cancel_builder = KaikkiStoreBuilder::create(&cancel_dir).expect("create cancel");
        cancel_builder.feed(fixture.as_bytes()).expect("feed");
        assert!(matches!(
            cancel_builder.finish(|| true),
            Err(KaikkiError::Cancelled)
        ));
        assert_eq!(
            current_generation(&root.0.join(pack.id)).as_deref(),
            Some(gen_name)
        );

        // Gzip round-trip via `build_kaikki_store`.
        let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
        gz_encoder.write_all(fixture.as_bytes()).expect("gz write");
        let compressed = gz_encoder.finish().expect("gz finish");
        build_kaikki_store(
            &KAIKKI_PACKS[1],
            &root.0,
            gen_name,
            &compressed,
            KaikkiStreamKind::Gzip,
        )
        .expect("build gzip");
        assert_eq!(
            lookup_kaikki_records(KAIKKI_PACKS[1].id, &root.0, "cats")
                .expect("gzip lookup")
                .len(),
            1
        );

        // Collision filtering: point "cat"'s hash directly at a "dog" record — must return 0 matches!
        let col_dir = root.0.join(KAIKKI_PACKS[1].id).join(gen_name);
        let dog = "{\"word\":\"dog\",\"senses\":[{\"glosses\":[\"animal\"]}]}";
        fs::write(col_dir.join("records.jsonl"), dog).expect("write dog");
        let header = StoreHeader {
            magic: INDEX_MAGIC,
            version: STORE_VERSION,
            count: 1,
            data_bytes: dog.len() as u64,
        };
        let colliding_entry = StoreEntry {
            hash: word_hash("cat"),
            offset: 0,
            bytes: dog.len() as u32,
            reserved: 0,
        };
        let mut idx_bytes = Vec::new();
        idx_bytes.extend_from_slice(&header.to_le_bytes());
        idx_bytes.extend_from_slice(&colliding_entry.to_le_bytes());
        fs::write(col_dir.join("index.bin"), &idx_bytes).expect("write collision index");
        assert!(
            lookup_kaikki_records(KAIKKI_PACKS[1].id, &root.0, "cat")
                .expect("collision filtered")
                .is_empty(),
            "hash collision must never return the wrong word"
        );

        // Corrupt out-of-range u64 offset must fail cleanly.
        let bad_entry = StoreEntry {
            hash: word_hash("dog"),
            offset: u64::MAX,
            bytes: 100,
            reserved: 0,
        };
        let mut bad_idx = Vec::new();
        bad_idx.extend_from_slice(&header.to_le_bytes());
        bad_idx.extend_from_slice(&bad_entry.to_le_bytes());
        fs::write(col_dir.join("index.bin"), &bad_idx).expect("write corrupt index");
        assert!(lookup_kaikki_records(KAIKKI_PACKS[1].id, &root.0, "dog").is_err());

        // Remove pack cleanly.
        remove_kaikki_pack(pack.id, &root.0).expect("remove");
        assert!(!kaikki_pack_installed(pack.id, &root.0));
    }

    #[test]
    fn parse_kaikki_record_extracts_specific_gloss_ipa_and_deduplicates() {
        let simple = &KAIKKI_PACKS[1];
        let fixture = concat!(
            "{\"word\":\"sick\",\"forms\":[{\"form\":\"sick\",\"tags\":[\"positive\"]},",
            "{\"form\":\"sicker\",\"tags\":[\"comparative\"]}],\"lang_code\":\"en\",\"pos\":\"adj\",",
            "\"senses\":[{\"glosses\":[\"If a person, animal, or plant is sick, it has a disease and is not healthy.\"],",
            "\"examples\":[{\"text\":\"I'm sorry, I'm too sick to go to work today.\"}]},",
            "{\"glosses\":[\"A way of saying something is bad.\",\"If something is sick it makes you feel like vomiting.\"],",
            "\"examples\":[{\"text\":\"Oh sick!\"}]}],",
            "\"sounds\":[{\"ipa\":\"/sɪk/\"},{\"audio\":\"en-us-sick.ogg\",\"tags\":[\"US\"]}]}"
        );
        let mut entries = vec![OfflineMeaning {
            headword: "sick".to_owned(),
            definition: "An existing WM meaning".to_owned(),
            dictionary: "WM vocabulary".to_owned(),
            dictionary_id: "wmkeyboard-vocab-en".to_owned(),
            ..OfflineMeaning::default()
        }];
        assert!(parse_kaikki_record(fixture, "sicker", simple, &mut entries));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].dictionary_id, simple.id);
        assert_eq!(entries[1].part_of_speech, "adjective");
        assert_eq!(entries[1].phonetic, "/sɪk/");
        assert_eq!(
            entries[1].source_url,
            "https://simple.wiktionary.org/wiki/sick"
        );
        assert!(entries[2].definition.starts_with("If something is sick"));
        assert!(
            !entries[2]
                .definition
                .contains("A way of saying something is bad.")
        );
        // Re-parsing deduplicates cleanly.
        assert!(parse_kaikki_record(fixture, "sicker", simple, &mut entries));
        assert_eq!(entries.len(), 3);
    }
}
