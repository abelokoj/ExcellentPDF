//! Progressive HTTP range-request reader and block cache for opening remote PDFs (plan R.30).
//!
//! Fetches fixed-size blocks on demand via HTTP `Range` requests (with [`ureq`] as the default
//! transport and a mockable [`RangeTransport`] trait for offline tests), falls back to full
//! download with progress reporting when a server lacks range support, detects linearized PDFs so
//! page 1 can appear before the full file finishes downloading, and supports saving a copy.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

/// Default progressive cache block size (64 KiB).
pub const DEFAULT_BLOCK_SIZE: usize = 64 * 1024;

/// Maximum bytes read into memory for a single HTTP block fetch before rejecting malformed headers.
const MAX_SINGLE_READ_BYTES: usize = 64 * 1024 * 1024;

/// Errors returned by remote range-request reading and progressive caching.
#[derive(Debug)]
#[non_exhaustive]
pub enum RemoteError {
    /// Invalid URL or unsupported scheme (only `http://` and `https://` are supported by [`UreqTransport`]).
    InvalidUrl(String),
    /// Invalid byte range or configuration argument.
    InvalidRange(String),
    /// Network transport or HTTP status failure.
    Network(String),
    /// Local filesystem I/O failure while saving a copy.
    Io(io::Error),
    /// Requested data is not cached yet; caller should fetch the indicated range and retry
    /// (mirrors MuPDF `FZ_ERROR_TRYLATER`).
    TryLater {
        /// Inclusive start byte offset of the missing block.
        start: u64,
        /// Exclusive end byte offset (best estimate when total length is still unknown).
        end: u64,
    },
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(msg) => write!(f, "invalid remote URL: {msg}"),
            Self::InvalidRange(msg) => write!(f, "invalid byte range: {msg}"),
            Self::Network(msg) => write!(f, "remote network error: {msg}"),
            Self::Io(err) => write!(f, "remote file I/O error: {err}"),
            Self::TryLater { start, end } => {
                write!(f, "remote range {start}..{end} not cached yet (try later)")
            }
        }
    }
}

impl std::error::Error for RemoteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidUrl(_)
            | Self::InvalidRange(_)
            | Self::Network(_)
            | Self::TryLater { .. } => None,
        }
    }
}

impl From<io::Error> for RemoteError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Convenience result alias for remote streaming operations.
pub type Result<T> = std::result::Result<T, RemoteError>;

/// Metadata probed from an HTTP server before or during progressive reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProbe {
    /// Total resource length in bytes, if advertised (`Content-Length` or `Content-Range`).
    pub total_len: Option<u64>,
    /// Whether the server advertises byte-range support (`Accept-Ranges: bytes`).
    pub supports_ranges: bool,
}

/// Result of fetching a byte range from a remote server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeResponse {
    /// Server honored `Range: bytes=start-end` (`206 Partial Content`).
    Partial {
        /// Starting byte offset of `data`.
        offset: u64,
        /// Bytes returned for this range.
        data: Vec<u8>,
        /// Total length from `Content-Range`, if present.
        total_len: Option<u64>,
    },
    /// Server ignored `Range` and returned the full representation (`200 OK`).
    Full {
        /// Complete body bytes.
        data: Vec<u8>,
    },
}

/// Abstract HTTP range-request transport (mockable for deterministic tests).
pub trait RangeTransport: Send + Sync {
    /// Probes total length and range support (typically via `HEAD`, or `Range: bytes=0-0`).
    ///
    /// # Errors
    ///
    /// Returns [`RemoteError::InvalidUrl`] or [`RemoteError::Network`] if the endpoint cannot be reached.
    fn probe(&self, url: &str) -> Result<RemoteProbe>;

    /// Fetches `[start, end_inclusive]` from `url`.
    ///
    /// # Errors
    ///
    /// Returns [`RemoteError::Network`] on HTTP or transport failure.
    fn fetch_range(&self, url: &str, start: u64, end_inclusive: u64) -> Result<RangeResponse>;
}

/// Production [`RangeTransport`] backed by `ureq`.
#[derive(Debug, Default, Clone, Copy)]
pub struct UreqTransport;

impl RangeTransport for UreqTransport {
    fn probe(&self, url: &str) -> Result<RemoteProbe> {
        validate_http_url(url)?;
        if let Ok(head_resp) = ureq::head(url).call() {
            let accept_ranges = head_resp
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
            let total_len = head_resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            if total_len.is_some() || accept_ranges {
                return Ok(RemoteProbe {
                    total_len,
                    supports_ranges: accept_ranges,
                });
            }
        }
        // Fallback probe using a 1-byte range GET when HEAD omits headers.
        let resp = ureq::get(url)
            .header("Range", "bytes=0-0")
            .call()
            .map_err(|err| RemoteError::Network(err.to_string()))?;
        let status = resp.status().as_u16();
        let content_range = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_total);
        let content_length = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        Ok(RemoteProbe {
            total_len: content_range.or(content_length),
            supports_ranges: status == 206,
        })
    }

    fn fetch_range(&self, url: &str, start: u64, end_inclusive: u64) -> Result<RangeResponse> {
        validate_http_url(url)?;
        if end_inclusive < start {
            return Err(RemoteError::InvalidRange(format!(
                "end {end_inclusive} precedes start {start}"
            )));
        }
        let resp = ureq::get(url)
            .header("Range", &format!("bytes={start}-{end_inclusive}"))
            .call()
            .map_err(|err| RemoteError::Network(err.to_string()))?;
        let status = resp.status().as_u16();
        let total_len = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_total);

        let mut reader = resp.into_body().into_reader();
        let mut data = Vec::new();
        let mut chunk = [0_u8; 16 * 1024];
        loop {
            let n = reader
                .read(&mut chunk)
                .map_err(|err| RemoteError::Network(err.to_string()))?;
            if n == 0 {
                break;
            }
            if data.len() + n > MAX_SINGLE_READ_BYTES {
                return Err(RemoteError::Network(
                    "remote response exceeded single-fetch safety limit".into(),
                ));
            }
            data.extend_from_slice(&chunk[..n]);
        }

        if status == 206 {
            Ok(RangeResponse::Partial {
                offset: start,
                data,
                total_len,
            })
        } else {
            Ok(RangeResponse::Full { data })
        }
    }
}

fn validate_http_url(url: &str) -> Result<()> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(())
    } else {
        Err(RemoteError::InvalidUrl(format!(
            "unsupported scheme in {url}"
        )))
    }
}

fn parse_content_range_total(header: &str) -> Option<u64> {
    let (_, total) = header.split_once('/')?;
    if total.trim() == "*" {
        return None;
    }
    total.trim().parse::<u64>().ok()
}

/// Whether the remote server supports HTTP byte-range requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RangeSupport {
    /// Range capability has not been determined yet.
    #[default]
    Unknown,
    /// Server supports `206 Partial Content`.
    Supported,
    /// Server returns full `200 OK` bodies without range support.
    Unsupported,
}

/// Progress snapshot for progressive or full remote downloads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownloadProgress {
    /// Bytes cached so far.
    pub downloaded_bytes: u64,
    /// Total file size in bytes, if known.
    pub total_bytes: Option<u64>,
    /// Completion ratio in `0.0..=1.0`, when total length is known.
    pub fraction: Option<f32>,
}

/// Outcome of a non-blocking progressive cache read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadChunk {
    /// Requested bytes were present in cache (truncated at EOF if `total_len` is known).
    Ready(Vec<u8>),
}

/// Block-based progressive HTTP stream cache (plan R.30).
///
/// Splits the remote file into fixed-size blocks (`block_size`, default 64 KiB). Non-blocking
/// reads return [`RemoteError::TryLater`] when any covered block is missing, allowing a caller
/// (such as a progressive `fz_stream`) to fetch that block asynchronously and retry.
#[derive(Debug)]
pub struct RemoteStreamCache<T: RangeTransport = UreqTransport> {
    url: String,
    transport: T,
    block_size: usize,
    total_len: Option<u64>,
    range_support: RangeSupport,
    blocks: BTreeMap<u64, Vec<u8>>,
    full_body: Option<Vec<u8>>,
}

impl<T: RangeTransport> RemoteStreamCache<T> {
    /// Creates a new progressive cache for `url` using `transport` and [`DEFAULT_BLOCK_SIZE`].
    ///
    /// # Errors
    ///
    /// Returns [`RemoteError::InvalidUrl`] if `url` is not `http://` or `https://`.
    pub fn new(url: impl Into<String>, transport: T) -> Result<Self> {
        Self::with_block_size(url, transport, DEFAULT_BLOCK_SIZE)
    }

    /// Creates a new progressive cache with a custom `block_size` (clamped to at least 1024 bytes).
    ///
    /// # Errors
    ///
    /// Returns [`RemoteError::InvalidUrl`] if `url` is not `http://` or `https://`.
    pub fn with_block_size(
        url: impl Into<String>,
        transport: T,
        block_size: usize,
    ) -> Result<Self> {
        let url = url.into();
        validate_http_url(&url)?;
        Ok(Self {
            url,
            transport,
            block_size: block_size.max(1024),
            total_len: None,
            range_support: RangeSupport::Unknown,
            blocks: BTreeMap::new(),
            full_body: None,
        })
    }

    /// Remote URL associated with this cache.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Cache block size in bytes.
    #[must_use]
    pub const fn block_size(&self) -> usize {
        self.block_size
    }

    /// Known total file length in bytes, if discovered.
    #[must_use]
    pub const fn total_len(&self) -> Option<u64> {
        self.total_len
    }

    /// Current range-support status.
    #[must_use]
    pub const fn range_support(&self) -> RangeSupport {
        self.range_support
    }

    /// Probes the server for `total_len` and `Range` capability.
    ///
    /// # Errors
    ///
    /// Propagates any transport error from [`RangeTransport::probe`].
    pub fn probe(&mut self) -> Result<RemoteProbe> {
        let probe = self.transport.probe(&self.url)?;
        if let Some(len) = probe.total_len {
            self.total_len = Some(len);
        }
        self.range_support = if probe.supports_ranges {
            RangeSupport::Supported
        } else {
            RangeSupport::Unsupported
        };
        Ok(probe)
    }

    /// Number of unique cached bytes currently held in memory.
    #[must_use]
    pub fn cached_bytes(&self) -> u64 {
        if let Some(full) = &self.full_body {
            return full.len() as u64;
        }
        self.blocks.values().map(|b| b.len() as u64).sum()
    }

    /// Current download progress snapshot.
    #[must_use]
    pub fn progress(&self) -> DownloadProgress {
        let downloaded = self.cached_bytes();
        let fraction = self.total_len.map(|total| {
            if total == 0 {
                1.0
            } else {
                (downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32
            }
        });
        DownloadProgress {
            downloaded_bytes: downloaded,
            total_bytes: self.total_len,
            fraction,
        }
    }

    /// True when the entire file is cached locally.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        if self.full_body.is_some() {
            return true;
        }
        let Some(total) = self.total_len else {
            return false;
        };
        if total == 0 {
            return true;
        }
        let block_sz = self.block_size as u64;
        let expected_blocks = total.div_ceil(block_sz);
        (0..expected_blocks).all(|idx| self.blocks.contains_key(&idx))
    }

    /// Non-blocking cache read of `[offset, offset + len)`.
    ///
    /// Returns [`RemoteError::TryLater`] naming the first uncached block if data is missing.
    ///
    /// # Errors
    ///
    /// Returns [`RemoteError::TryLater`] when any covered block has not yet been fetched.
    pub fn read_cached(&self, offset: u64, len: usize) -> Result<ReadChunk> {
        if len == 0 {
            return Ok(ReadChunk::Ready(Vec::new()));
        }
        if let Some(full) = &self.full_body {
            let start = usize::try_from(offset)
                .unwrap_or(full.len())
                .min(full.len());
            let end = start.saturating_add(len).min(full.len());
            return Ok(ReadChunk::Ready(full[start..end].to_vec()));
        }
        if let Some(total) = self.total_len
            && offset >= total
        {
            return Ok(ReadChunk::Ready(Vec::new()));
        }
        let effective_end = match self.total_len {
            Some(total) => offset.saturating_add(len as u64).min(total),
            None => offset.saturating_add(len as u64),
        };
        let block_sz = self.block_size as u64;
        let first_block = offset / block_sz;
        let last_block = effective_end.saturating_sub(1) / block_sz;
        let mut out = Vec::with_capacity(usize::try_from(effective_end - offset).unwrap_or(0));

        for block_idx in first_block..=last_block {
            let block_start = block_idx * block_sz;
            let Some(block) = self.blocks.get(&block_idx) else {
                let block_end = match self.total_len {
                    Some(total) => (block_start + block_sz).min(total),
                    None => block_start + block_sz,
                };
                return Err(RemoteError::TryLater {
                    start: block_start,
                    end: block_end,
                });
            };
            let slice_start = usize::try_from(offset.saturating_sub(block_start)).unwrap_or(0);
            let slice_end = usize::try_from(effective_end.saturating_sub(block_start))
                .unwrap_or(block.len())
                .min(block.len());
            if slice_start < slice_end {
                out.extend_from_slice(&block[slice_start..slice_end]);
            }
        }
        Ok(ReadChunk::Ready(out))
    }

    /// Reads `[offset, offset + len)`, automatically fetching any missing blocks from the transport.
    ///
    /// # Errors
    ///
    /// Returns transport errors or [`RemoteError::InvalidRange`].
    pub fn read_or_fetch(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        loop {
            match self.read_cached(offset, len) {
                Ok(ReadChunk::Ready(bytes)) => return Ok(bytes),
                Err(RemoteError::TryLater { start, end }) => {
                    self.fetch_byte_range(start, end)?;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Fetches `[start, end_exclusive)` into the block cache (or downloads the full body if the
    /// server does not support ranges).
    ///
    /// # Errors
    ///
    /// Returns transport or range validation errors.
    pub fn fetch_byte_range(&mut self, start: u64, end_exclusive: u64) -> Result<()> {
        if end_exclusive <= start {
            return Ok(());
        }
        let block_sz = self.block_size as u64;
        let aligned_start = (start / block_sz) * block_sz;
        let aligned_end_excl = end_exclusive.div_ceil(block_sz) * block_sz;
        let clamped_end_excl = match self.total_len {
            Some(total) if total > 0 => aligned_end_excl.min(total),
            _ => aligned_end_excl,
        };
        if clamped_end_excl <= aligned_start {
            return Ok(());
        }
        let end_inclusive = clamped_end_excl - 1;
        match self
            .transport
            .fetch_range(&self.url, aligned_start, end_inclusive)?
        {
            RangeResponse::Partial {
                offset,
                data,
                total_len,
            } => {
                self.range_support = RangeSupport::Supported;
                if let Some(total) = total_len {
                    self.total_len = Some(total);
                }
                self.store_aligned_blocks(offset, &data);
            }
            RangeResponse::Full { data } => {
                self.range_support = RangeSupport::Unsupported;
                self.total_len = Some(data.len() as u64);
                self.full_body = Some(data);
                self.blocks.clear();
            }
        }
        Ok(())
    }

    fn store_aligned_blocks(&mut self, start_offset: u64, data: &[u8]) {
        let block_sz = self.block_size;
        let mut cursor = 0usize;
        let mut offset = start_offset;
        while cursor < data.len() {
            let block_idx = offset / block_sz as u64;
            let in_block = (offset % block_sz as u64) as usize;
            let take = (block_sz - in_block).min(data.len() - cursor);
            if in_block == 0 && take <= block_sz {
                self.blocks
                    .insert(block_idx, data[cursor..cursor + take].to_vec());
            } else {
                let entry = self
                    .blocks
                    .entry(block_idx)
                    .or_insert_with(|| vec![0; in_block + take]);
                if entry.len() < in_block + take {
                    entry.resize(in_block + take, 0);
                }
                entry[in_block..in_block + take].copy_from_slice(&data[cursor..cursor + take]);
            }
            cursor += take;
            offset += take as u64;
        }
    }

    /// Downloads any remaining uncached blocks, invoking `on_progress` after each step, and
    /// returns the complete file bytes.
    ///
    /// # Errors
    ///
    /// Returns transport errors if fetching fails or total length cannot be determined.
    pub fn download_all(
        &mut self,
        mut on_progress: impl FnMut(DownloadProgress),
    ) -> Result<Vec<u8>> {
        if let Some(full) = &self.full_body {
            on_progress(self.progress());
            return Ok(full.clone());
        }
        if self.total_len.is_none() {
            self.probe()?;
        }
        if self.total_len.is_none() {
            // Probe gave no length; request block 0 to learn total length or get full body.
            self.fetch_byte_range(0, self.block_size as u64)?;
            on_progress(self.progress());
        }
        if let Some(full) = &self.full_body {
            return Ok(full.clone());
        }
        let total = self.total_len.ok_or_else(|| {
            RemoteError::Network("server did not advertise total content length".into())
        })?;
        let block_sz = self.block_size as u64;
        let num_blocks = total.div_ceil(block_sz);
        for idx in 0..num_blocks {
            if !self.blocks.contains_key(&idx) {
                let start = idx * block_sz;
                let end = (start + block_sz).min(total);
                self.fetch_byte_range(start, end)?;
                on_progress(self.progress());
                if self.full_body.is_some() {
                    break;
                }
            }
        }
        if let Some(full) = &self.full_body {
            return Ok(full.clone());
        }
        let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
        for idx in 0..num_blocks {
            let block = self.blocks.get(&idx).ok_or_else(|| RemoteError::TryLater {
                start: idx * block_sz,
                end: ((idx + 1) * block_sz).min(total),
            })?;
            out.extend_from_slice(block);
        }
        out.truncate(usize::try_from(total).unwrap_or(out.len()));
        Ok(out)
    }

    /// Downloads any missing bytes and saves an atomic copy to `path` (plan R.30 step 4).
    ///
    /// # Errors
    ///
    /// Returns transport or filesystem I/O errors.
    pub fn save_copy(
        &mut self,
        path: &Path,
        on_progress: impl FnMut(DownloadProgress),
    ) -> Result<()> {
        let bytes = self.download_all(on_progress)?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("remote.pdf");
        let tmp_path = path.with_file_name(format!(".{file_name}.part"));
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Checks whether the initial bytes already cached represent a linearized (Fast Web View) PDF
    /// so page 1 can be prioritized before the rest of the file finishes downloading.
    ///
    /// # Errors
    ///
    /// Propagates [`RemoteError::TryLater`] if the first kilobyte is not yet cached.
    pub fn is_linearized_pdf(&mut self) -> Result<bool> {
        let head = self.read_or_fetch(0, 1024)?;
        Ok(has_linearized_pdf_marker(&head))
    }
}

/// Returns `true` when `head` (the leading ~1 KiB of a PDF) contains `/Linearized`.
#[must_use]
pub fn has_linearized_pdf_marker(head: &[u8]) -> bool {
    let limit = head.len().min(2048);
    head[..limit]
        .windows(b"/Linearized".len())
        .any(|w| w == b"/Linearized")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug)]
    struct MockServer {
        data: Vec<u8>,
        supports_ranges: bool,
        fetch_calls: Arc<AtomicUsize>,
        requested_ranges: Arc<Mutex<Vec<(u64, u64)>>>,
    }

    impl MockServer {
        fn new(data: Vec<u8>, supports_ranges: bool) -> Self {
            Self {
                data,
                supports_ranges,
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                requested_ranges: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl RangeTransport for MockServer {
        fn probe(&self, _url: &str) -> Result<RemoteProbe> {
            Ok(RemoteProbe {
                total_len: Some(self.data.len() as u64),
                supports_ranges: self.supports_ranges,
            })
        }

        fn fetch_range(&self, _url: &str, start: u64, end_inclusive: u64) -> Result<RangeResponse> {
            self.fetch_calls.fetch_add(1, Ordering::Relaxed);
            self.requested_ranges
                .lock()
                .expect("lock")
                .push((start, end_inclusive));
            if !self.supports_ranges {
                return Ok(RangeResponse::Full {
                    data: self.data.clone(),
                });
            }
            let total = self.data.len() as u64;
            let s = usize::try_from(start)
                .unwrap_or(self.data.len())
                .min(self.data.len());
            let e = usize::try_from(end_inclusive.saturating_add(1))
                .unwrap_or(self.data.len())
                .min(self.data.len());
            Ok(RangeResponse::Partial {
                offset: start,
                data: self.data[s..e].to_vec(),
                total_len: Some(total),
            })
        }
    }

    #[test]
    fn progressive_range_reads_detect_linearized_and_trigger_trylater() {
        let mut pdf = b"%PDF-1.7\n1 0 obj << /Linearized 1 /L 10000 >> endobj\n".to_vec();
        pdf.resize(4096, b'x');
        let server = MockServer::new(pdf.clone(), true);
        let calls = Arc::clone(&server.fetch_calls);

        let mut cache = RemoteStreamCache::with_block_size(
            "https://arxiv.org/pdf/2501.00001.pdf",
            server,
            1024,
        )
        .expect("valid url");

        // Before fetching, cached read yields TryLater with exact block bounds.
        let err = cache.read_cached(0, 512).expect_err("must request block");
        assert!(matches!(
            err,
            RemoteError::TryLater {
                start: 0,
                end: 1024
            }
        ));

        // Checking linearization fetches only block 0 (1024 bytes), not the whole 4096-byte file.
        assert!(cache.is_linearized_pdf().expect("linearized check"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(cache.cached_bytes(), 1024);
        assert!(!cache.is_complete());

        // Reading near the end fetches only the missing tail block.
        let tail = cache.read_or_fetch(3500, 100).expect("tail read");
        assert_eq!(tail, &pdf[3500..3600]);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(cache.cached_bytes(), 2048);

        // Full download finishes remaining blocks without re-fetching blocks 0 and 3.
        let mut updates = Vec::new();
        let all = cache
            .download_all(|p| updates.push(p.downloaded_bytes))
            .expect("download_all");
        assert_eq!(all, pdf);
        assert!(cache.is_complete());
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        assert_eq!(updates.last().copied(), Some(4096));
    }

    #[test]
    fn non_range_server_downloads_fully_with_progress_and_saves_copy() {
        let payload = b"%PDF-1.4\nhello world stream".to_vec();
        let server = MockServer::new(payload.clone(), false);
        let mut cache =
            RemoteStreamCache::with_block_size("https://example.org/manual.pdf", server, 1024)
                .expect("valid url");

        let mut fractions = Vec::new();
        let dir = std::env::temp_dir().join(format!("ex-remote-test-{}", std::process::id()));
        let out_path = dir.join("saved.pdf");
        cache
            .save_copy(&out_path, |p| {
                if let Some(f) = p.fraction {
                    fractions.push(f);
                }
            })
            .expect("save copy");

        assert_eq!(cache.range_support(), RangeSupport::Unsupported);
        assert!(cache.is_complete());
        assert_eq!(fractions.last().copied(), Some(1.0));
        let on_disk = fs::read(&out_path).expect("read saved file");
        assert_eq!(on_disk, payload);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_non_http_urls() {
        let server = MockServer::new(Vec::new(), true);
        let err =
            RemoteStreamCache::new("ftp://example.org/a.pdf", server).expect_err("ftp rejected");
        assert!(matches!(err, RemoteError::InvalidUrl(_)));
    }
}
