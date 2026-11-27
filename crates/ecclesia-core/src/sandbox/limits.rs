//! Central parser security resource limits and untrusted-input preflight validation (plan K.23).
//!
//! Enforces explicit bounds on page counts, object/container nesting depth, decompressed stream
//! sizes, XRef table entries, zip-bomb compression ratios, image dimensions, attachment names,
//! and external link schemes before untrusted bytes reach MuPDF or archive decoders.

use std::fmt;
use std::io::{Cursor, Read};
use std::time::Duration;

use ecclesia_mupdf::{Document, MuError};
use flate2::read::{GzDecoder, ZlibDecoder};
use zip::ZipArchive;

/// Default maximum PDF pages accepted without explicit override.
pub const DEFAULT_MAX_PAGES: usize = 50_000;
/// Default maximum container / outline / XML nesting depth.
pub const DEFAULT_MAX_RECURSION_DEPTH: usize = 64;
/// Default maximum decompressed stream or archive output in bytes (512 MiB).
pub const DEFAULT_MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
/// Default maximum cross-reference table entries in a PDF.
pub const DEFAULT_MAX_XREF_ENTRIES: usize = 1_000_000;
/// Default maximum decompressed-to-compressed inflation ratio before flagging a zip bomb.
pub const DEFAULT_MAX_COMPRESSION_RATIO: u64 = 100;
/// Minimum decompressed payload (64 KiB) before compression-ratio bomb checks apply.
pub const DEFAULT_RATIO_CHECK_FLOOR_BYTES: u64 = 64 * 1024;
/// Default maximum decoded image pixel count (100 megapixels).
pub const DEFAULT_MAX_IMAGE_PIXELS: u64 = 100_000_000;
/// Default maximum PDF indirect object count or XML entity expansions.
pub const DEFAULT_MAX_OBJECT_COUNT: usize = 2_000_000;
/// Default maximum raw input file size (1 GiB).
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;
/// Default maximum single IPC frame size (64 MiB).
pub const DEFAULT_MAX_IPC_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Default sandboxed worker memory cap (1 GiB).
pub const DEFAULT_WORKER_MEMORY_CAP_BYTES: u64 = 1024 * 1024 * 1024;
/// Default wall-clock watchdog timeout for a single worker request (30 s).
pub const DEFAULT_WORKER_TIMEOUT: Duration = Duration::from_secs(30);

/// Central resource caps applied to untrusted documents, streams, archives, and worker processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityLimits {
    /// Maximum number of pages permitted in a document.
    pub max_pages: usize,
    /// Maximum nesting depth for PDF dictionaries/arrays, XML tags, or outline trees.
    pub max_recursion_depth: usize,
    /// Maximum decompressed bytes across streams or archive entries.
    pub max_decompressed_bytes: u64,
    /// Maximum cross-reference table entries declared or parsed in a PDF.
    pub max_xref_entries: usize,
    /// Maximum allowed ratio of decompressed bytes to compressed bytes once above
    /// [`Self::ratio_check_floor_bytes`].
    pub max_compression_ratio: u64,
    /// Minimum decompressed byte count before compression-ratio checks enforce a zip-bomb limit.
    pub ratio_check_floor_bytes: u64,
    /// Maximum decoded pixel area (`width * height`) for embedded or standalone images.
    pub max_image_pixels: u64,
    /// Maximum indirect object count in a PDF or entity/node count in XML/XFDF.
    pub max_object_count: usize,
    /// Maximum raw input size in bytes.
    pub max_input_bytes: u64,
    /// Maximum encoded IPC frame payload length in bytes.
    pub max_ipc_frame_bytes: usize,
    /// Memory cap enforced on a sandboxed worker process.
    pub worker_memory_cap_bytes: u64,
    /// Wall-clock timeout enforced by the worker watchdog.
    pub worker_timeout: Duration,
}

impl Default for SecurityLimits {
    fn default() -> Self {
        Self {
            max_pages: DEFAULT_MAX_PAGES,
            max_recursion_depth: DEFAULT_MAX_RECURSION_DEPTH,
            max_decompressed_bytes: DEFAULT_MAX_DECOMPRESSED_BYTES,
            max_xref_entries: DEFAULT_MAX_XREF_ENTRIES,
            max_compression_ratio: DEFAULT_MAX_COMPRESSION_RATIO,
            ratio_check_floor_bytes: DEFAULT_RATIO_CHECK_FLOOR_BYTES,
            max_image_pixels: DEFAULT_MAX_IMAGE_PIXELS,
            max_object_count: DEFAULT_MAX_OBJECT_COUNT,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_ipc_frame_bytes: DEFAULT_MAX_IPC_FRAME_BYTES,
            worker_memory_cap_bytes: DEFAULT_WORKER_MEMORY_CAP_BYTES,
            worker_timeout: DEFAULT_WORKER_TIMEOUT,
        }
    }
}

impl SecurityLimits {
    /// Returns the default production [`SecurityLimits`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns tighter limits suited for untrusted previews, thumbnails, and fuzzing harnesses.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            max_pages: 1_000,
            max_recursion_depth: 32,
            max_decompressed_bytes: 32 * 1024 * 1024,
            max_xref_entries: 100_000,
            max_compression_ratio: 50,
            ratio_check_floor_bytes: 16 * 1024,
            max_image_pixels: 25_000_000,
            max_object_count: 250_000,
            max_input_bytes: 64 * 1024 * 1024,
            max_ipc_frame_bytes: 16 * 1024 * 1024,
            worker_memory_cap_bytes: 256 * 1024 * 1024,
            worker_timeout: Duration::from_secs(5),
        }
    }

    /// Validates that `count` does not exceed [`Self::max_pages`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::PageCountExceeded`] if `count > self.max_pages`.
    pub fn validate_page_count(&self, count: usize) -> Result<(), LimitViolation> {
        if count > self.max_pages {
            return Err(LimitViolation::PageCountExceeded {
                actual: count,
                limit: self.max_pages,
            });
        }
        Ok(())
    }

    /// Validates that container/tree `depth` does not exceed [`Self::max_recursion_depth`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::RecursionDepthExceeded`] if `depth > self.max_recursion_depth`.
    pub fn validate_recursion_depth(&self, depth: usize) -> Result<(), LimitViolation> {
        if depth > self.max_recursion_depth {
            return Err(LimitViolation::RecursionDepthExceeded {
                depth,
                limit: self.max_recursion_depth,
            });
        }
        Ok(())
    }

    /// Validates a decompressed stream against both [`Self::max_decompressed_bytes`] and
    /// [`Self::max_compression_ratio`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::DecompressedSizeExceeded`] or
    /// [`LimitViolation::CompressionRatioExceeded`] when the bounds are violated.
    pub fn validate_decompressed_stream(
        &self,
        compressed_bytes: u64,
        decompressed_bytes: u64,
    ) -> Result<(), LimitViolation> {
        if decompressed_bytes > self.max_decompressed_bytes {
            return Err(LimitViolation::DecompressedSizeExceeded {
                actual: decompressed_bytes,
                limit: self.max_decompressed_bytes,
            });
        }
        if decompressed_bytes > self.ratio_check_floor_bytes {
            let denom = compressed_bytes.max(1);
            let allowed = denom.saturating_mul(self.max_compression_ratio);
            if decompressed_bytes > allowed {
                return Err(LimitViolation::CompressionRatioExceeded {
                    compressed: compressed_bytes,
                    decompressed: decompressed_bytes,
                    max_ratio: self.max_compression_ratio,
                });
            }
        }
        Ok(())
    }

    /// Validates that XRef table `entries` does not exceed [`Self::max_xref_entries`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::XrefEntriesExceeded`] if `entries > self.max_xref_entries`.
    pub fn validate_xref_entries(&self, entries: usize) -> Result<(), LimitViolation> {
        if entries > self.max_xref_entries {
            return Err(LimitViolation::XrefEntriesExceeded {
                actual: entries,
                limit: self.max_xref_entries,
            });
        }
        Ok(())
    }

    /// Validates image dimensions (`width * height`) against [`Self::max_image_pixels`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::ImageSizeExceeded`] if `width * height > self.max_image_pixels`.
    pub fn validate_image_dimensions(&self, width: u32, height: u32) -> Result<(), LimitViolation> {
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        if pixels > self.max_image_pixels {
            return Err(LimitViolation::ImageSizeExceeded {
                width,
                height,
                pixels,
                limit: self.max_image_pixels,
            });
        }
        Ok(())
    }

    /// Validates that an object or entity count does not exceed [`Self::max_object_count`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::ObjectCountExceeded`] if `objects > self.max_object_count`.
    pub fn validate_object_count(&self, objects: usize) -> Result<(), LimitViolation> {
        if objects > self.max_object_count {
            return Err(LimitViolation::ObjectCountExceeded {
                actual: objects,
                limit: self.max_object_count,
            });
        }
        Ok(())
    }

    /// Validates raw input byte size against [`Self::max_input_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation::InputSizeExceeded`] if `bytes > self.max_input_bytes`.
    pub fn validate_input_size(&self, bytes: u64) -> Result<(), LimitViolation> {
        if bytes > self.max_input_bytes {
            return Err(LimitViolation::InputSizeExceeded {
                actual: bytes,
                limit: self.max_input_bytes,
            });
        }
        Ok(())
    }

    /// Scans raw PDF bytes for structural resource-limit violations before handing them to MuPDF.
    ///
    /// Checks input size, container nesting depth (`<< ... >>` and `[ ... ]`), declared `/Size`
    /// and `xref` subsection entry counts, indirect object counts, and bounded zlib stream
    /// inflation ratios.
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation`] if any limit is exceeded or the header is missing.
    pub fn validate_pdf_bytes(&self, bytes: &[u8]) -> Result<PdfPreflightReport, LimitViolation> {
        self.validate_input_size(bytes.len() as u64)?;
        let has_header = bytes.windows(5).take(1024).any(|w| w == b"%PDF-");
        if !has_header {
            return Err(LimitViolation::MalformedInput {
                format: "pdf",
                detail: "missing %PDF- signature in first 1024 bytes".to_owned(),
            });
        }

        let stats = scan_pdf_structure(bytes, self)?;
        self.validate_recursion_depth(stats.max_depth)?;
        self.validate_xref_entries(stats.xref_entries)?;
        self.validate_object_count(stats.object_count)?;
        self.validate_page_count(stats.declared_pages)?;
        validate_pdf_zlib_streams(bytes, self)?;
        Ok(stats)
    }

    /// Validates a ZIP-backed container (`.expdf`, `.epub`, `.cbz`, `.xps`) for path-traversal
    /// entry names and zip-bomb inflation ratios.
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation`] on malformed ZIP headers, unsafe entry paths, or ratio/size
    /// cap violations.
    pub fn validate_zip_archive_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<ZipPreflightReport, LimitViolation> {
        self.validate_input_size(bytes.len() as u64)?;
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).map_err(|err| LimitViolation::MalformedInput {
                format: "zip",
                detail: err.to_string(),
            })?;

        let entry_count = archive.len();
        self.validate_object_count(entry_count)?;

        let mut total_compressed = 0u64;
        let mut total_decompressed = 0u64;
        let mut buf = [0u8; 8192];

        for index in 0..entry_count {
            let mut file =
                archive
                    .by_index(index)
                    .map_err(|err| LimitViolation::MalformedInput {
                        format: "zip",
                        detail: err.to_string(),
                    })?;
            let entry_name = file
                .name()
                .map_err(|err| LimitViolation::MalformedInput {
                    format: "zip",
                    detail: err.to_string(),
                })?
                .to_string();
            sanitize_archive_entry_path(&entry_name)?;

            let declared_compressed = file.compressed_size();
            let declared_uncompressed = file.size();
            self.validate_decompressed_stream(declared_compressed, declared_uncompressed)?;

            let mut actual_decompressed = 0u64;
            loop {
                let remaining = self
                    .max_decompressed_bytes
                    .saturating_sub(total_decompressed.saturating_add(actual_decompressed));
                let max_step = usize::try_from(remaining)
                    .unwrap_or(buf.len())
                    .min(buf.len())
                    .max(1);
                let n = file.read(&mut buf[..max_step]).map_err(|err| {
                    LimitViolation::MalformedInput {
                        format: "zip",
                        detail: format!("entry '{entry_name}' read failed: {err}"),
                    }
                })?;
                if n == 0 {
                    break;
                }
                actual_decompressed = actual_decompressed.saturating_add(n as u64);
                let cumulative = total_decompressed.saturating_add(actual_decompressed);
                if cumulative > self.max_decompressed_bytes {
                    return Err(LimitViolation::DecompressedSizeExceeded {
                        actual: cumulative,
                        limit: self.max_decompressed_bytes,
                    });
                }
                self.validate_decompressed_stream(declared_compressed, actual_decompressed)?;
            }

            total_compressed = total_compressed.saturating_add(declared_compressed);
            total_decompressed = total_decompressed.saturating_add(actual_decompressed);
            self.validate_decompressed_stream(total_compressed, total_decompressed)?;
        }

        Ok(ZipPreflightReport {
            entry_count,
            total_compressed_bytes: total_compressed,
            total_decompressed_bytes: total_decompressed,
        })
    }

    /// Validates an XFDF (XML Forms Data Format) payload for tag nesting depth and entity bombs.
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation`] when the input is oversized, contains NUL bytes, exceeds
    /// entity/object limits, or nests beyond [`Self::max_recursion_depth`].
    pub fn validate_xfdf_bytes(&self, bytes: &[u8]) -> Result<(), LimitViolation> {
        self.validate_input_size(bytes.len() as u64)?;
        if bytes.contains(&0) {
            return Err(LimitViolation::MalformedInput {
                format: "xfdf",
                detail: "unexpected NUL byte".to_owned(),
            });
        }
        let text = std::str::from_utf8(bytes).map_err(|err| LimitViolation::MalformedInput {
            format: "xfdf",
            detail: format!("invalid UTF-8: {err}"),
        })?;
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with('<') {
            return Err(LimitViolation::MalformedInput {
                format: "xfdf",
                detail: "XFDF payload must begin with an XML element".to_owned(),
            });
        }

        let entity_defs = trimmed.matches("<!ENTITY").count();
        if entity_defs > 8 {
            return Err(LimitViolation::ObjectCountExceeded {
                actual: entity_defs,
                limit: 8,
            });
        }

        let mut depth = 0usize;
        let mut tag_count = 0usize;
        let mut pos = 0usize;
        let raw = trimmed.as_bytes();
        while let Some(rel) = raw[pos..].iter().position(|&b| b == b'<') {
            let start = pos + rel;
            let Some(end_rel) = raw[start + 1..].iter().position(|&b| b == b'>') else {
                return Err(LimitViolation::MalformedInput {
                    format: "xfdf",
                    detail: "unclosed XML tag".to_owned(),
                });
            };
            let tag = &trimmed[start + 1..start + 1 + end_rel];
            let tag_trim = tag.trim();
            if tag_trim.starts_with('?') || tag_trim.starts_with('!') {
                pos = start + 2 + end_rel;
                continue;
            }
            tag_count = tag_count.saturating_add(1);
            self.validate_object_count(tag_count)?;
            if tag_trim.starts_with('/') {
                depth = depth.saturating_sub(1);
            } else if !tag_trim.ends_with('/') {
                depth = depth.saturating_add(1);
                self.validate_recursion_depth(depth)?;
            }
            pos = start + 2 + end_rel;
        }
        Ok(())
    }

    /// Validates a plain or gzipped SyncTeX payload for gzip bombs, size caps, and excessive line
    /// counts.
    ///
    /// # Errors
    ///
    /// Returns [`LimitViolation`] if decompression fails, blows past ratio/size caps, or is
    /// structurally invalid.
    pub fn validate_synctex_bytes(&self, bytes: &[u8]) -> Result<(), LimitViolation> {
        self.validate_input_size(bytes.len() as u64)?;
        let decoded = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut decoder = GzDecoder::new(Cursor::new(bytes));
            read_bounded(&mut decoder, self, bytes.len() as u64, "synctex")?
        } else {
            bytes.to_vec()
        };
        if decoded.is_empty() {
            return Err(LimitViolation::MalformedInput {
                format: "synctex",
                detail: "empty SyncTeX data".to_owned(),
            });
        }
        let line_count = decoded.iter().filter(|&&b| b == b'\n').count();
        self.validate_object_count(line_count)?;
        if !decoded.starts_with(b"SyncTeX Version:") {
            return Err(LimitViolation::MalformedInput {
                format: "synctex",
                detail: "missing SyncTeX Version header".to_owned(),
            });
        }
        Ok(())
    }
}

/// Summary produced by [`SecurityLimits::validate_pdf_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfPreflightReport {
    /// Maximum bracket/dictionary nesting depth encountered outside streams.
    pub max_depth: usize,
    /// Estimated XRef entry count across `/Size` and `xref` subsections.
    pub xref_entries: usize,
    /// Count of `obj` definitions encountered.
    pub object_count: usize,
    /// Largest declared `/Count` or counted `/Type /Page` markers.
    pub declared_pages: usize,
}

/// Summary produced by [`SecurityLimits::validate_zip_archive_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipPreflightReport {
    /// Number of entries in the ZIP archive.
    pub entry_count: usize,
    /// Sum of compressed entry byte counts.
    pub total_compressed_bytes: u64,
    /// Sum of decompressed entry byte counts.
    pub total_decompressed_bytes: u64,
}

/// Error returned when an untrusted input or operation violates [`SecurityLimits`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub enum LimitViolation {
    /// Document page count exceeds `limit`.
    PageCountExceeded {
        /// Observed or declared page count.
        actual: usize,
        /// Configured ceiling.
        limit: usize,
    },
    /// Nested structure depth exceeds `limit`.
    RecursionDepthExceeded {
        /// Observed nesting depth.
        depth: usize,
        /// Configured ceiling.
        limit: usize,
    },
    /// Decompressed stream or archive size exceeds `limit`.
    DecompressedSizeExceeded {
        /// Decompressed byte total.
        actual: u64,
        /// Configured ceiling.
        limit: u64,
    },
    /// Decompressed-to-compressed inflation ratio exceeds `max_ratio`.
    CompressionRatioExceeded {
        /// Compressed byte length.
        compressed: u64,
        /// Decompressed byte length.
        decompressed: u64,
        /// Configured maximum ratio.
        max_ratio: u64,
    },
    /// PDF cross-reference table entries exceed `limit`.
    XrefEntriesExceeded {
        /// Observed or declared XRef entry count.
        actual: usize,
        /// Configured ceiling.
        limit: usize,
    },
    /// Image pixel area exceeds `limit`.
    ImageSizeExceeded {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Total pixel count (`width * height`).
        pixels: u64,
        /// Configured ceiling.
        limit: u64,
    },
    /// Indirect object or XML/line count exceeds `limit`.
    ObjectCountExceeded {
        /// Observed count.
        actual: usize,
        /// Configured ceiling.
        limit: usize,
    },
    /// Raw input byte size exceeds `limit`.
    InputSizeExceeded {
        /// Input length in bytes.
        actual: u64,
        /// Configured ceiling.
        limit: u64,
    },
    /// An archive entry or attachment path attempts directory traversal or uses unsafe characters.
    UnsafeArchivePath {
        /// Offending raw entry name.
        entry: String,
    },
    /// Input failed basic structural sanity checks for `format`.
    MalformedInput {
        /// Format label (for example `"pdf"`, `"zip"`, `"xfdf"`, `"synctex"`).
        format: &'static str,
        /// Human-readable reason.
        detail: String,
    },
}

impl fmt::Display for LimitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PageCountExceeded { actual, limit } => {
                write!(f, "page count {actual} exceeds limit {limit}")
            }
            Self::RecursionDepthExceeded { depth, limit } => {
                write!(f, "recursion depth {depth} exceeds limit {limit}")
            }
            Self::DecompressedSizeExceeded { actual, limit } => {
                write!(f, "decompressed size {actual} bytes exceeds limit {limit}")
            }
            Self::CompressionRatioExceeded {
                compressed,
                decompressed,
                max_ratio,
            } => write!(
                f,
                "compression ratio {decompressed}/{compressed} exceeds limit {max_ratio}x"
            ),
            Self::XrefEntriesExceeded { actual, limit } => {
                write!(f, "xref entries {actual} exceed limit {limit}")
            }
            Self::ImageSizeExceeded {
                width,
                height,
                pixels,
                limit,
            } => write!(
                f,
                "image dimensions {width}x{height} ({pixels} px) exceed limit {limit} px"
            ),
            Self::ObjectCountExceeded { actual, limit } => {
                write!(f, "object count {actual} exceeds limit {limit}")
            }
            Self::InputSizeExceeded { actual, limit } => {
                write!(f, "input size {actual} bytes exceeds limit {limit}")
            }
            Self::UnsafeArchivePath { entry } => {
                write!(f, "unsafe archive or attachment path '{entry}'")
            }
            Self::MalformedInput { format, detail } => {
                write!(f, "malformed {format} input: {detail}")
            }
        }
    }
}

impl std::error::Error for LimitViolation {}

/// Classification of an external URI extracted from a document link or action (plan K.23 step 3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub enum UriSafety {
    /// Standard web destination (`https://` or `http://`) that still requires user confirmation
    /// or policy check before launching a browser.
    WebLinkRequiresConfirmation {
        /// Normalized lowercase scheme (`"https"` or `"http"`).
        scheme: String,
    },
    /// Non-web URI scheme (e.g. `mailto:`, `file:`) that must prompt explicitly before opening.
    ExternalRequiresExplicitPrompt {
        /// Normalized lowercase scheme.
        scheme: String,
    },
    /// Dangerous script or executable launch URI (`javascript:`, `vbscript:`, `data:`, etc.)
    /// that must never be executed.
    Blocked {
        /// Reason the URI was blocked.
        reason: String,
    },
}

/// Classifies an external link `uri` according to plan K.23 step 3 (links never open silently).
pub fn classify_external_uri(uri: &str) -> UriSafety {
    let trimmed = uri.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|b| b.is_ascii_control()) {
        return UriSafety::Blocked {
            reason: "empty or control-character URI".to_owned(),
        };
    }
    let Some((raw_scheme, _rest)) = trimmed.split_once(':') else {
        return UriSafety::Blocked {
            reason: "missing URI scheme".to_owned(),
        };
    };
    let scheme = raw_scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "javascript" | "vbscript" | "data" | "cmd" | "ms-msdt" | "shell" => UriSafety::Blocked {
            reason: format!("disallowed active/script scheme '{scheme}'"),
        },
        "https" | "http" => UriSafety::WebLinkRequiresConfirmation { scheme },
        _ => UriSafety::ExternalRequiresExplicitPrompt { scheme },
    }
}

/// Validates a relative archive entry path, rejecting traversal (`..`), absolute paths, backslashes,
/// drive letters, and control characters (plan K.23 step 3).
///
/// # Errors
///
/// Returns [`LimitViolation::UnsafeArchivePath`] if the entry path could escape a target directory.
pub fn sanitize_archive_entry_path(raw: &str) -> Result<String, LimitViolation> {
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains('\\')
        || raw.contains('\0')
        || raw.bytes().any(|b| b < 0x20)
    {
        return Err(LimitViolation::UnsafeArchivePath {
            entry: raw.to_owned(),
        });
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(LimitViolation::UnsafeArchivePath {
            entry: raw.to_owned(),
        });
    }
    for segment in raw.split('/') {
        if segment == ".." || segment == "." {
            return Err(LimitViolation::UnsafeArchivePath {
                entry: raw.to_owned(),
            });
        }
    }
    Ok(raw.to_owned())
}

/// Sanitizes an embedded PDF attachment filename into a safe single-component filename.
///
/// Rejects directory separators, `..`, NUL bytes, control characters, empty names, and Windows
/// reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`..`COM9`, `LPT1`..`LPT9`).
///
/// # Errors
///
/// Returns [`LimitViolation::UnsafeArchivePath`] if `raw` is unsafe.
pub fn sanitize_attachment_name(raw: &str) -> Result<String, LimitViolation> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
        || trimmed.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(LimitViolation::UnsafeArchivePath {
            entry: raw.to_owned(),
        });
    }
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if reserved {
        return Err(LimitViolation::UnsafeArchivePath {
            entry: raw.to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

/// Opens PDF `bytes` with MuPDF only after [`SecurityLimits::validate_pdf_bytes`] passes, and
/// confirms the resulting document's page count fits within [`SecurityLimits::max_pages`].
///
/// # Errors
///
/// Returns [`crate::sandbox::SandboxError::LimitViolation`] if any resource bound is breached, or
/// [`crate::sandbox::SandboxError::Mupdf`] if MuPDF rejects the bytes.
pub fn open_pdf_with_limits(
    bytes: &[u8],
    limits: &SecurityLimits,
) -> Result<Document, crate::sandbox::SandboxError> {
    limits.validate_pdf_bytes(bytes)?;
    let doc = Document::from_bytes(bytes, "application/pdf").map_err(map_mupdf_error)?;
    let pages = doc.page_count().map_err(map_mupdf_error)?;
    limits.validate_page_count(pages)?;
    Ok(doc)
}

fn map_mupdf_error(err: MuError) -> crate::sandbox::SandboxError {
    crate::sandbox::SandboxError::Mupdf(err.to_string())
}

fn read_bounded<R: Read>(
    reader: &mut R,
    limits: &SecurityLimits,
    compressed_len: u64,
    format: &'static str,
) -> Result<Vec<u8>, LimitViolation> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|err| LimitViolation::MalformedInput {
                format,
                detail: err.to_string(),
            })?;
        if n == 0 {
            break;
        }
        let next_len = (out.len() as u64).saturating_add(n as u64);
        limits.validate_decompressed_stream(compressed_len, next_len)?;
        out.extend_from_slice(&chunk[..n]);
    }
    Ok(out)
}

fn scan_pdf_structure(
    bytes: &[u8],
    limits: &SecurityLimits,
) -> Result<PdfPreflightReport, LimitViolation> {
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    let mut object_count = 0usize;
    let mut xref_entries = 0usize;
    let mut page_markers = 0usize;
    let mut declared_pages = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        // Skip comments.
        if b == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        // Skip literal strings `(...)` with balanced parens and `\` escapes.
        if b == b'(' {
            i += 1;
            let mut paren_depth = 1usize;
            while i < bytes.len() && paren_depth > 0 {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'(' {
                    paren_depth += 1;
                } else if bytes[i] == b')' {
                    paren_depth -= 1;
                }
                i += 1;
            }
            continue;
        }
        // Dictionary delimiters `<<` / `>>` or hex string `<...>`.
        if b == b'<' {
            if bytes.get(i + 1) == Some(&b'<') {
                depth += 1;
                max_depth = max_depth.max(depth);
                limits.validate_recursion_depth(depth)?;
                i += 2;
                continue;
            }
            i += 1;
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        if b == b'>' && bytes.get(i + 1) == Some(&b'>') {
            depth = depth.saturating_sub(1);
            i += 2;
            continue;
        }
        if b == b'[' {
            depth += 1;
            max_depth = max_depth.max(depth);
            limits.validate_recursion_depth(depth)?;
            i += 1;
            continue;
        }
        if b == b']' {
            depth = depth.saturating_sub(1);
            i += 1;
            continue;
        }
        // Skip `stream ... endstream` payloads so binary stream data isn't counted as brackets.
        if bytes[i..].starts_with(b"stream")
            && (i == 0 || is_pdf_delim_or_ws(bytes[i - 1]))
            && matches!(bytes.get(i + 6), Some(b'\r' | b'\n'))
        {
            let after_kw = i + 6;
            let body_start = if bytes.get(after_kw..after_kw + 2) == Some(b"\r\n".as_slice()) {
                after_kw + 2
            } else {
                after_kw + 1
            };
            if let Some(rel_end) = find_subslice(&bytes[body_start..], b"endstream") {
                i = body_start + rel_end + "endstream".len();
                continue;
            }
            // Unterminated stream: advance safely to EOF without looping.
            break;
        }
        if bytes[i..].starts_with(b" obj") && is_preceded_by_digits(bytes, i) {
            object_count = object_count.saturating_add(1);
            limits.validate_object_count(object_count)?;
            i += 4;
            continue;
        }
        if bytes[i..].starts_with(b"/Size")
            && is_pdf_delim_or_ws(bytes.get(i + 5).copied().unwrap_or(b' '))
        {
            if let Some(n) = parse_following_usize(&bytes[i + 5..]) {
                xref_entries = xref_entries.max(n);
                limits.validate_xref_entries(xref_entries)?;
            }
            i += 5;
            continue;
        }
        if bytes[i..].starts_with(b"/Count")
            && is_pdf_delim_or_ws(bytes.get(i + 6).copied().unwrap_or(b' '))
        {
            if let Some(n) = parse_following_usize(&bytes[i + 6..]) {
                declared_pages = declared_pages.max(n);
                limits.validate_page_count(declared_pages)?;
            }
            i += 6;
            continue;
        }
        if bytes[i..].starts_with(b"/Type") {
            let tail = skip_pdf_ws(&bytes[i + 5..]);
            if tail.starts_with(b"/Page")
                && tail
                    .get(5)
                    .copied()
                    .is_none_or(|c| c != b's' && is_pdf_delim_or_ws(c))
            {
                page_markers = page_markers.saturating_add(1);
            }
            i += 5;
            continue;
        }
        if bytes[i..].starts_with(b"xref") && (i == 0 || is_pdf_delim_or_ws(bytes[i - 1])) {
            if let Some(sub_count) = parse_xref_subsection_count(&bytes[i + 4..]) {
                xref_entries = xref_entries.saturating_add(sub_count);
                limits.validate_xref_entries(xref_entries)?;
            }
            i += 4;
            continue;
        }
        i += 1;
    }

    Ok(PdfPreflightReport {
        max_depth,
        xref_entries,
        object_count,
        declared_pages: declared_pages.max(page_markers),
    })
}

fn validate_pdf_zlib_streams(bytes: &[u8], limits: &SecurityLimits) -> Result<(), LimitViolation> {
    let mut pos = 0usize;
    let mut total_decompressed = 0u64;
    while let Some(rel) = find_subslice(&bytes[pos..], b"stream") {
        let kw = pos + rel;
        let valid_before = kw == 0 || is_pdf_delim_or_ws(bytes[kw - 1]);
        let after_kw = kw + 6;
        let valid_after = matches!(bytes.get(after_kw), Some(b'\r' | b'\n'));
        if !valid_before || !valid_after {
            pos = after_kw.min(bytes.len());
            continue;
        }
        let body_start = if bytes.get(after_kw..after_kw + 2) == Some(b"\r\n".as_slice()) {
            after_kw + 2
        } else {
            after_kw + 1
        };
        let Some(rel_end) = find_subslice(&bytes[body_start..], b"endstream") else {
            break;
        };
        let body = trim_stream_trailing_eol(&bytes[body_start..body_start + rel_end]);
        // Detect standard zlib CMF/FLG header (e.g. 0x78 ...) and verify bounded inflation.
        if body.len() >= 2
            && (body[0] & 0x0f) == 8
            && (u16::from(body[0]) * 256 + u16::from(body[1])) % 31 == 0
        {
            let mut decoder = ZlibDecoder::new(Cursor::new(body));
            let mut buf = [0u8; 8192];
            let mut stream_out = 0u64;
            loop {
                match decoder.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        stream_out = stream_out.saturating_add(n as u64);
                        total_decompressed = total_decompressed.saturating_add(n as u64);
                        limits.validate_decompressed_stream(body.len() as u64, stream_out)?;
                        if total_decompressed > limits.max_decompressed_bytes {
                            return Err(LimitViolation::DecompressedSizeExceeded {
                                actual: total_decompressed,
                                limit: limits.max_decompressed_bytes,
                            });
                        }
                    }
                    // Corrupted zlib bytes inside an otherwise arbitrary stream shouldn't panic;
                    // MuPDF will handle or reject the PDF cleanly.
                    Err(_) => break,
                }
            }
        }
        pos = body_start + rel_end + "endstream".len();
    }
    Ok(())
}

fn trim_stream_trailing_eol(slice: &[u8]) -> &[u8] {
    if let Some(stripped) = slice.strip_suffix(b"\r\n") {
        stripped
    } else if let Some(stripped) = slice.strip_suffix(b"\n") {
        stripped
    } else if let Some(stripped) = slice.strip_suffix(b"\r") {
        stripped
    } else {
        slice
    }
}

fn is_pdf_delim_or_ws(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

fn is_preceded_by_digits(bytes: &[u8], idx: usize) -> bool {
    let Some(prev) = idx.checked_sub(1).and_then(|p| bytes.get(p)) else {
        return false;
    };
    prev.is_ascii_digit()
}

fn skip_pdf_ws(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if b.is_ascii_whitespace() {
            s = &s[1..];
        } else {
            break;
        }
    }
    s
}

fn parse_following_usize(s: &[u8]) -> Option<usize> {
    let s = skip_pdf_ws(s);
    let end = s
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    std::str::from_utf8(&s[..end]).ok()?.parse().ok()
}

fn parse_xref_subsection_count(s: &[u8]) -> Option<usize> {
    let s = skip_pdf_ws(s);
    let first_end = s.iter().position(|b| !b.is_ascii_digit())?;
    if first_end == 0 {
        return None;
    }
    parse_following_usize(&s[first_end..])
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
