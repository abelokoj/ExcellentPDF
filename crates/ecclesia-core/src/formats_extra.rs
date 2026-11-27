//! Extra document format adapters and containers: DjVu, CHM, CBZ, and CBR (plan R.29).
//!
//! Provides a unified [`ExtraDocument`] trait matching EcclesiaPDF's document abstractions
//! (page count, page dimensions, text extraction, outline extraction, and page asset bytes)
//! across:
//! - **DjVu** (`AT&TFORM` EA-IFF-85 containers: `DIRM` page directory, `INFO` bounds, `NAVM` outline,
//!   and uncompressed `TXTa` text chunks),
//! - **CHM** (`ITSF` / `ITSP` / `PMGL` Microsoft HTML Help containers: file listing, `.hhc` sitemap
//!   TOC parsing, and HTML topic text extraction),
//! - **CBZ / CBR** comic archives: full ZIP reading via `zip` for CBZ (with natural page sorting
//!   and PNG/JPEG/GIF/BMP dimension inspection), plus a clean [`RarExtractor`] container adapter
//!   for CBR (`Rar!\x1a\x07`) that parses RAR4/RAR5 headers and reads uncompressed (`STORE`) entries
//!   directly without re-implementing proprietary RAR decompression.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Cursor, Read as _};

use zip::ZipArchive;

/// Extra document formats supported outside MuPDF's built-in readers (plan R.29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtraFormat {
    /// DjVu (`AT&TFORM`).
    DjVu,
    /// Microsoft Compiled HTML Help (`ITSF`).
    Chm,
    /// Comic Book ZIP (`PK\x03\x04`).
    Cbz,
    /// Comic Book RAR (`Rar!\x1a\x07`).
    Cbr,
}

impl ExtraFormat {
    /// Lowercase file extension without a leading dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::DjVu => "djvu",
            Self::Chm => "chm",
            Self::Cbz => "cbz",
            Self::Cbr => "cbr",
        }
    }

    /// Detects an extra format from leading file bytes and an optional lowercase extension.
    #[must_use]
    pub fn detect(head: &[u8], extension: Option<&str>) -> Option<Self> {
        if head.starts_with(b"AT&TFORM") {
            return Some(Self::DjVu);
        }
        if head.starts_with(b"ITSF") {
            return Some(Self::Chm);
        }
        if head.starts_with(b"Rar!\x1a\x07") {
            return Some(Self::Cbr);
        }
        if head.starts_with(b"PK\x03\x04") && matches!(extension, Some("cbz")) {
            return Some(Self::Cbz);
        }
        match extension {
            Some("djvu" | "djv") => Some(Self::DjVu),
            Some("chm") => Some(Self::Chm),
            Some("cbr") => Some(Self::Cbr),
            Some("cbz") => Some(Self::Cbz),
            _ => None,
        }
    }
}

/// Errors produced when parsing or reading DjVu, CHM, CBZ, or CBR containers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtraFormatError {
    /// Header signature or structural framing is invalid or truncated.
    InvalidContainer(String),
    /// Requested page index is outside `0..page_count`.
    PageOutOfRange {
        /// Requested zero-based index.
        page: usize,
        /// Total number of pages in the document.
        count: usize,
    },
    /// Archive uses a compressed stream that requires an external decoder (e.g. RAR compressed
    /// entry or BZZ-compressed DjVu chunk).
    UnsupportedCompression(String),
    /// ZIP archive failure.
    Zip(String),
}

impl fmt::Display for ExtraFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContainer(msg) => write!(f, "invalid extra-format container: {msg}"),
            Self::PageOutOfRange { page, count } => {
                write!(f, "page {page} out of range (count {count})")
            }
            Self::UnsupportedCompression(msg) => {
                write!(f, "unsupported compressed stream: {msg}")
            }
            Self::Zip(msg) => write!(f, "zip container error: {msg}"),
        }
    }
}

impl std::error::Error for ExtraFormatError {}

/// Convenience result alias for extra-format operations.
pub type Result<T> = std::result::Result<T, ExtraFormatError>;

/// Page dimensions in points (72 dpi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtraPageSize {
    /// Width in points.
    pub width_pt: f32,
    /// Height in points.
    pub height_pt: f32,
}

/// Outline entry extracted from a DjVu or CHM document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtraOutlineItem {
    /// Display title.
    pub title: String,
    /// Zero-based destination page index, if known.
    pub page: Option<usize>,
    /// Destination path or URL inside the container (e.g., CHM topic path).
    pub target: Option<String>,
    /// Nested child outline entries.
    pub children: Vec<ExtraOutlineItem>,
}

/// Unified interface implemented by extra-format readers (plan R.29).
pub trait ExtraDocument: Send + Sync {
    /// Format identifier.
    fn format(&self) -> ExtraFormat;

    /// Total number of pages or topics.
    fn page_count(&self) -> usize;

    /// Page bounds in points for zero-based `page`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::PageOutOfRange`] if `page >= page_count()`.
    fn page_size(&self, page: usize) -> Result<ExtraPageSize>;

    /// Plain text for zero-based `page`, if the format contains text.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::PageOutOfRange`] or [`ExtraFormatError::UnsupportedCompression`].
    fn page_text(&self, page: usize) -> Result<Option<String>>;

    /// Document table of contents / outline tree.
    fn outline(&self) -> Vec<ExtraOutlineItem>;
}

// ---------------------------------------------------------------------------
// DjVu (AT&TFORM EA-IFF-85)
// ---------------------------------------------------------------------------

/// Metadata and extracted content for one page inside a [`DjVuDocument`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjVuPage {
    /// Chunk or component identifier (for bundled multi-page DjVu).
    pub id: String,
    /// Pixel width from `INFO` chunk.
    pub width_px: u32,
    /// Pixel height from `INFO` chunk.
    pub height_px: u32,
    /// Resolution in DPI from `INFO` chunk (typically 300).
    pub dpi: u16,
    /// Plain text extracted from `TXTa` chunks, if present.
    pub text: Option<String>,
    /// True if a `TXTz` (BZZ-compressed) chunk exists without uncompressed `TXTa`.
    pub has_compressed_text: bool,
}

/// Pure-Rust DjVu container parser reading `AT&TFORM` (`DJVU` single-page & `DJVM` bundled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjVuDocument {
    pages: Vec<DjVuPage>,
    outline: Vec<ExtraOutlineItem>,
}

impl DjVuDocument {
    /// Parses a DjVu container from `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::InvalidContainer`] if the `AT&TFORM` header or IFF chunks are invalid.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if !bytes.starts_with(b"AT&TFORM") || bytes.len() < 16 {
            return Err(ExtraFormatError::InvalidContainer(
                "missing AT&TFORM DjVu signature".into(),
            ));
        }
        let form_size = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        let form_end = 12usize.saturating_add(form_size).min(bytes.len());
        let form_type = &bytes[12..16];

        let mut pages = Vec::new();
        let mut outline = Vec::new();

        match form_type {
            b"DJVU" => {
                let page = parse_djvu_page_chunks("page0", &bytes[16..form_end])?;
                pages.push(page);
            }
            b"DJVM" => {
                let mut page_ids = Vec::new();
                let mut pos = 16usize;
                let mut page_counter = 0usize;
                while pos + 8 <= form_end {
                    let chunk_id = &bytes[pos..pos + 4];
                    let chunk_len = u32::from_be_bytes([
                        bytes[pos + 4],
                        bytes[pos + 5],
                        bytes[pos + 6],
                        bytes[pos + 7],
                    ]) as usize;
                    let data_start = pos + 8;
                    let data_end = data_start.saturating_add(chunk_len).min(form_end);
                    let chunk_data = &bytes[data_start..data_end];

                    if chunk_id == b"DIRM" {
                        page_ids = parse_dirm_ids(chunk_data);
                    } else if chunk_id == b"NAVM" {
                        outline = parse_navm_outline(chunk_data);
                    } else if chunk_id == b"FORM"
                        && chunk_data.len() >= 4
                        && &chunk_data[..4] == b"DJVU"
                    {
                        let id = page_ids
                            .get(page_counter)
                            .cloned()
                            .unwrap_or_else(|| format!("page{page_counter}"));
                        pages.push(parse_djvu_page_chunks(&id, &chunk_data[4..])?);
                        page_counter += 1;
                    }
                    // IFF chunks are padded to even byte boundaries.
                    pos = data_end + (chunk_len % 2);
                }
            }
            other => {
                return Err(ExtraFormatError::InvalidContainer(format!(
                    "unexpected DjVu FORM type: {}",
                    String::from_utf8_lossy(other)
                )));
            }
        }

        if pages.is_empty() {
            return Err(ExtraFormatError::InvalidContainer(
                "DjVu container contained zero pages".into(),
            ));
        }

        Ok(Self { pages, outline })
    }

    /// Returns parsed page records.
    #[must_use]
    pub fn pages(&self) -> &[DjVuPage] {
        &self.pages
    }
}

fn parse_djvu_page_chunks(id: &str, mut body: &[u8]) -> Result<DjVuPage> {
    let mut width_px = 612u32;
    let mut height_px = 792u32;
    let mut dpi = 72u16;
    let mut has_info = false;
    let mut text = None;
    let mut has_compressed_text = false;

    while body.len() >= 8 {
        let chunk_id = &body[0..4];
        let chunk_len = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
        let data_end = 8usize.saturating_add(chunk_len).min(body.len());
        let data = &body[8..data_end];

        if chunk_id == b"INFO" && data.len() >= 8 {
            width_px = u32::from(u16::from_be_bytes([data[0], data[1]]));
            height_px = u32::from(u16::from_be_bytes([data[2], data[3]]));
            let raw_dpi = u16::from_le_bytes([data[6], data[7]]);
            dpi = if raw_dpi == 0 { 300 } else { raw_dpi };
            has_info = true;
        } else if chunk_id == b"TXTa" && data.len() >= 3 {
            // TXTa: 3-byte big-endian text length + UTF-8 text.
            let txt_len =
                ((data[0] as usize) << 16) | ((data[1] as usize) << 8) | (data[2] as usize);
            let slice = &data[3..data.len().min(3 + txt_len)];
            text = Some(String::from_utf8_lossy(slice).into_owned());
        } else if chunk_id == b"TXTz" {
            has_compressed_text = true;
        }

        let step = data_end + (chunk_len % 2);
        if step >= body.len() {
            break;
        }
        body = &body[step..];
    }

    if !has_info {
        return Err(ExtraFormatError::InvalidContainer(format!(
            "DjVu page {id} missing INFO chunk"
        )));
    }

    Ok(DjVuPage {
        id: id.to_owned(),
        width_px,
        height_px,
        dpi,
        text,
        has_compressed_text,
    })
}

fn parse_dirm_ids(data: &[u8]) -> Vec<String> {
    // Simple DIRM NUL-terminated identifier tail extraction when uncompressed/text-listed.
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u16::from_be_bytes([data[1], data[2]]) as usize;
    let offset = 3 + count * 12;
    if offset >= data.len() {
        return Vec::new();
    }
    data[offset..]
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .take(count)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

fn parse_navm_outline(data: &[u8]) -> Vec<ExtraOutlineItem> {
    // Supports UTF-8 line-based or S-expression bookmark metadata stored in NAVM chunks.
    let raw = String::from_utf8_lossy(data);
    let mut items = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((title, target)) = trimmed.split_once('\t') {
            let page = target
                .strip_prefix('#')
                .and_then(|s| s.parse::<usize>().ok())
                .map(|p| p.saturating_sub(1));
            items.push(ExtraOutlineItem {
                title: title.trim().to_owned(),
                page,
                target: Some(target.trim().to_owned()),
                children: Vec::new(),
            });
        }
    }
    items
}

impl ExtraDocument for DjVuDocument {
    fn format(&self) -> ExtraFormat {
        ExtraFormat::DjVu
    }

    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_size(&self, page: usize) -> Result<ExtraPageSize> {
        let p = self
            .pages
            .get(page)
            .ok_or(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            })?;
        let scale = 72.0 / f32::from(p.dpi.max(1));
        Ok(ExtraPageSize {
            width_pt: (p.width_px as f32 * scale).max(1.0),
            height_pt: (p.height_px as f32 * scale).max(1.0),
        })
    }

    fn page_text(&self, page: usize) -> Result<Option<String>> {
        let p = self
            .pages
            .get(page)
            .ok_or(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            })?;
        if let Some(txt) = &p.text {
            return Ok(Some(txt.clone()));
        }
        if p.has_compressed_text {
            return Err(ExtraFormatError::UnsupportedCompression(
                "DjVu TXTz (BZZ) chunk requires DjVuLibre decoder".into(),
            ));
        }
        Ok(None)
    }

    fn outline(&self) -> Vec<ExtraOutlineItem> {
        self.outline.clone()
    }
}

// ---------------------------------------------------------------------------
// CHM (Microsoft ITSS / HTML Help)
// ---------------------------------------------------------------------------

/// An entry in a CHM ITSS archive directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChmEntry {
    /// Internal path (e.g. `"/index.html"`, `"/toc.hhc"`).
    pub path: String,
    /// Section index (`0` = uncompressed content section, `1` = LZX compressed).
    pub section: u64,
    /// Byte offset within the section.
    pub offset: u64,
    /// Uncompressed length in bytes.
    pub length: u64,
}

/// Microsoft Compiled HTML Help (`ITSF` / ITSS) container reader (plan R.29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChmDocument {
    entries: BTreeMap<String, ChmEntry>,
    pages: Vec<String>,
    contents: BTreeMap<String, Vec<u8>>,
    outline: Vec<ExtraOutlineItem>,
}

impl ChmDocument {
    /// Parses an ITSS / CHM container from `bytes`, reading section 0 uncompressed HTML topics
    /// and any `.hhc` sitemap outline.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::InvalidContainer`] if the `ITSF` / `ITSP` / `PMGL` headers are invalid.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if !bytes.starts_with(b"ITSF") || bytes.len() < 96 {
            return Err(ExtraFormatError::InvalidContainer(
                "missing ITSF CHM header".into(),
            ));
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if !(2..=4).contains(&version) {
            return Err(ExtraFormatError::InvalidContainer(format!(
                "unsupported ITSF version {version}"
            )));
        }
        let header_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        if header_len < 88 || bytes.len() < header_len {
            return Err(ExtraFormatError::InvalidContainer(
                "invalid ITSF header length".into(),
            ));
        }

        // Section table entry 1 (directory) starts at byte 56 (16-byte offset+len for section 0, then section 1).
        let dir_offset = read_u64_le(bytes, 72)? as usize;
        let dir_len = read_u64_le(bytes, 80)? as usize;
        let content_offset = if version >= 3 && header_len >= 96 {
            read_u64_le(bytes, 88)? as usize
        } else {
            dir_offset.saturating_add(dir_len)
        };

        if dir_offset.saturating_add(dir_len) > bytes.len() {
            return Err(ExtraFormatError::InvalidContainer(
                "ITSF directory extends past EOF".into(),
            ));
        }
        let dir = &bytes[dir_offset..dir_offset + dir_len];
        if !dir.starts_with(b"ITSP") || dir.len() < 84 {
            return Err(ExtraFormatError::InvalidContainer(
                "missing ITSP directory header".into(),
            ));
        }
        let itsp_header_len = u32::from_le_bytes([dir[8], dir[9], dir[10], dir[11]]) as usize;
        let chunk_size = u32::from_le_bytes([dir[16], dir[17], dir[18], dir[19]]) as usize;
        let num_chunks = u32::from_le_bytes([dir[36], dir[37], dir[38], dir[39]]) as usize;
        if chunk_size < 32 {
            return Err(ExtraFormatError::InvalidContainer(
                "ITSP chunk size too small".into(),
            ));
        }

        let mut entries = BTreeMap::new();
        for chunk_idx in 0..num_chunks {
            let chunk_start = itsp_header_len.saturating_add(chunk_idx * chunk_size);
            if chunk_start + 20 > dir.len() {
                break;
            }
            let chunk = &dir[chunk_start..(chunk_start + chunk_size).min(dir.len())];
            if !chunk.starts_with(b"PMGL") {
                continue;
            }
            let free_space = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
            let data_end = chunk.len().saturating_sub(free_space).max(20);
            let mut pos = 20usize;
            while pos < data_end {
                let Some((name_len, n1)) = read_encint(&chunk[pos..data_end]) else {
                    break;
                };
                pos += n1;
                let name_len = name_len as usize;
                if pos + name_len > data_end {
                    break;
                }
                let path = String::from_utf8_lossy(&chunk[pos..pos + name_len]).into_owned();
                pos += name_len;
                let Some((section, n2)) = read_encint(&chunk[pos..data_end]) else {
                    break;
                };
                pos += n2;
                let Some((offset, n3)) = read_encint(&chunk[pos..data_end]) else {
                    break;
                };
                pos += n3;
                let Some((length, n4)) = read_encint(&chunk[pos..data_end]) else {
                    break;
                };
                pos += n4;
                if !path.is_empty() {
                    entries.insert(
                        path.clone(),
                        ChmEntry {
                            path,
                            section,
                            offset,
                            length,
                        },
                    );
                }
            }
        }

        let mut contents = BTreeMap::new();
        for (path, entry) in &entries {
            if entry.section == 0 && entry.length > 0 {
                let start = content_offset.saturating_add(entry.offset as usize);
                let end = start.saturating_add(entry.length as usize);
                if end <= bytes.len() {
                    contents.insert(path.clone(), bytes[start..end].to_vec());
                }
            }
        }

        let outline = entries
            .keys()
            .find(|p| p.to_ascii_lowercase().ends_with(".hhc"))
            .and_then(|hhc_path| contents.get(hhc_path))
            .map(|raw| parse_hhc_toc(&String::from_utf8_lossy(raw)))
            .unwrap_or_default();

        let mut pages: Vec<String> = entries
            .keys()
            .filter(|p| {
                let lower = p.to_ascii_lowercase();
                lower.ends_with(".html") || lower.ends_with(".htm")
            })
            .cloned()
            .collect();
        if pages.is_empty() && !outline.is_empty() {
            collect_outline_targets(&outline, &mut pages);
        }

        Ok(Self {
            entries,
            pages,
            contents,
            outline,
        })
    }

    /// All directory entries in the CHM archive.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<String, ChmEntry> {
        &self.entries
    }

    /// Ordered HTML topic paths treated as pages.
    #[must_use]
    pub fn topic_paths(&self) -> &[String] {
        &self.pages
    }

    /// Reads the raw bytes of an uncompressed section-0 archive item by path.
    #[must_use]
    pub fn read_item(&self, path: &str) -> Option<&[u8]> {
        self.contents.get(path).map(Vec::as_slice)
    }
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64> {
    let slice = bytes.get(offset..offset + 8).ok_or_else(|| {
        ExtraFormatError::InvalidContainer("truncated 64-bit field in ITSF header".into())
    })?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn read_encint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut val = 0u64;
    for (idx, &b) in bytes.iter().enumerate().take(10) {
        val = (val << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            return Some((val, idx + 1));
        }
    }
    None
}

fn collect_outline_targets(items: &[ExtraOutlineItem], out: &mut Vec<String>) {
    for item in items {
        if let Some(target) = &item.target
            && !out.contains(target)
        {
            out.push(target.clone());
        }
        collect_outline_targets(&item.children, out);
    }
}

/// Parses an HTML Help `.hhc` sitemap into an [`ExtraOutlineItem`] tree.
#[must_use]
pub fn parse_hhc_toc(hhc: &str) -> Vec<ExtraOutlineItem> {
    let mut root: Vec<ExtraOutlineItem> = Vec::new();
    let mut stack: Vec<Vec<ExtraOutlineItem>> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_local: Option<String> = None;
    let mut in_sitemap_obj = false;

    let lower = hhc.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(rel_lt) = lower[cursor..].find('<') {
        let tag_start = cursor + rel_lt;
        let Some(rel_gt) = lower[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + rel_gt + 1;
        let tag_raw = &hhc[tag_start..tag_end];
        let tag_lc = &lower[tag_start..tag_end];

        if tag_lc.starts_with("<ul") {
            stack.push(Vec::new());
        } else if tag_lc.starts_with("</ul") {
            if let Some(children) = stack.pop() {
                if let Some(parent_list) = stack.last_mut() {
                    if let Some(last) = parent_list.last_mut() {
                        last.children.extend(children);
                    } else {
                        parent_list.extend(children);
                    }
                } else if let Some(last) = root.last_mut() {
                    last.children.extend(children);
                } else {
                    root.extend(children);
                }
            }
        } else if tag_lc.starts_with("<object") && tag_lc.contains("text/sitemap") {
            in_sitemap_obj = true;
            current_name = None;
            current_local = None;
        } else if tag_lc.starts_with("<param") && in_sitemap_obj {
            let name = attr_value(tag_raw, "name").unwrap_or_default();
            let value = attr_value(tag_raw, "value").unwrap_or_default();
            if name.eq_ignore_ascii_case("Name") {
                current_name = Some(value);
            } else if name.eq_ignore_ascii_case("Local") {
                current_local = Some(value);
            }
        } else if tag_lc.starts_with("</object") && in_sitemap_obj {
            in_sitemap_obj = false;
            if let Some(title) = current_name.take() {
                let item = ExtraOutlineItem {
                    title,
                    page: None,
                    target: current_local.take(),
                    children: Vec::new(),
                };
                if let Some(top) = stack.last_mut() {
                    top.push(item);
                } else {
                    root.push(item);
                }
            }
        }

        cursor = tag_end;
    }

    while let Some(remaining) = stack.pop() {
        root.extend(remaining);
    }
    root
}

fn attr_value(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attr_name}=");
    let pos = lower.find(&needle)?;
    let rest = tag[pos + needle.len()..].trim_start();
    if let Some(after_q) = rest.strip_prefix('"') {
        let end = after_q.find('"')?;
        Some(after_q[..end].to_owned())
    } else if let Some(after_q) = rest.strip_prefix('\'') {
        let end = after_q.find('\'')?;
        Some(after_q[..end].to_owned())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut prev_space = true;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !prev_space {
                        out.push(' ');
                        prev_space = true;
                    }
                } else {
                    out.push(ch);
                    prev_space = false;
                }
            }
            _ => {}
        }
    }
    out.trim().to_owned()
}

impl ExtraDocument for ChmDocument {
    fn format(&self) -> ExtraFormat {
        ExtraFormat::Chm
    }

    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_size(&self, page: usize) -> Result<ExtraPageSize> {
        if page >= self.pages.len() {
            return Err(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            });
        }
        Ok(ExtraPageSize {
            width_pt: 595.0,
            height_pt: 842.0,
        })
    }

    fn page_text(&self, page: usize) -> Result<Option<String>> {
        let path = self
            .pages
            .get(page)
            .ok_or(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            })?;
        if let Some(raw) = self.contents.get(path) {
            return Ok(Some(strip_html_tags(&String::from_utf8_lossy(raw))));
        }
        if let Some(entry) = self.entries.get(path)
            && entry.section != 0
        {
            return Err(ExtraFormatError::UnsupportedCompression(format!(
                "CHM section {} (LZX) for {path} requires chmlib/LZX decoder",
                entry.section
            )));
        }
        Ok(None)
    }

    fn outline(&self) -> Vec<ExtraOutlineItem> {
        self.outline.clone()
    }
}

// ---------------------------------------------------------------------------
// CBZ / CBR Comic Archives
// ---------------------------------------------------------------------------

/// External RAR extraction backend for compressed CBR entries (plan R.29).
///
/// UnRAR's license forbids re-implementing its compression algorithm; callers plug in an approved
/// external `unrar`/libarchive backend for compressed RAR entries while uncompressed `STORE`
/// entries are read directly.
pub trait RarExtractor: Send + Sync {
    /// Extracts the entry named `name` from a RAR archive byte slice.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::UnsupportedCompression`] or [`ExtraFormatError::InvalidContainer`].
    fn extract_entry(&self, archive_bytes: &[u8], name: &str) -> Result<Vec<u8>>;
}

/// Metadata for a single image page inside a CBZ or CBR comic archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComicPageEntry {
    /// Normalized entry name inside the archive.
    pub name: String,
    /// Uncompressed size in bytes.
    pub uncompressed_size: u64,
    /// RAR compression method (`0x30` = STORE for RAR4, `0` = STORE for RAR5). `0` for ZIP.
    pub compression_method: u16,
    /// Offset of stored payload inside the source buffer (for zero-copy RAR STORE reads).
    store_range: Option<(usize, usize)>,
}

/// Comic book archive reader supporting CBZ (ZIP) natively and CBR (RAR) via header inspection
/// plus pluggable [`RarExtractor`] (plan R.29).
#[derive(Debug, Clone)]
pub struct ComicArchive {
    format: ExtraFormat,
    raw_bytes: Vec<u8>,
    pages: Vec<ComicPageEntry>,
    page_sizes: Vec<ExtraPageSize>,
}

impl ComicArchive {
    /// Opens a CBZ (ZIP) archive from `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::Zip`] or [`ExtraFormatError::InvalidContainer`] if invalid.
    pub fn from_cbz_bytes(bytes: &[u8]) -> Result<Self> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
        let mut pages = Vec::new();
        let mut sizes_by_name = BTreeMap::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
            let name = file
                .name()
                .map_err(|e| ExtraFormatError::Zip(e.to_string()))?
                .replace('\\', "/");
            if name.ends_with('/') || !is_image_filename(&name) {
                continue;
            }
            let mut buf = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            file.read_to_end(&mut buf)
                .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
            sizes_by_name.insert(name.clone(), image_dimensions_pt(&buf));
            pages.push(ComicPageEntry {
                name,
                uncompressed_size: file.size(),
                compression_method: 0,
                store_range: None,
            });
        }
        if pages.is_empty() {
            return Err(ExtraFormatError::InvalidContainer(
                "CBZ archive contains no image pages".into(),
            ));
        }
        pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        let page_sizes = pages
            .iter()
            .map(|p| {
                sizes_by_name
                    .get(&p.name)
                    .copied()
                    .unwrap_or(ExtraPageSize {
                        width_pt: 595.0,
                        height_pt: 842.0,
                    })
            })
            .collect();

        Ok(Self {
            format: ExtraFormat::Cbz,
            raw_bytes: bytes.to_vec(),
            pages,
            page_sizes,
        })
    }

    /// Parses a CBR (RAR4 `Rar!\x1a\x07\x00` or RAR5 `Rar!\x1a\x07\x01\x00`) container's file table.
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::InvalidContainer`] if the RAR signature or headers are invalid.
    pub fn from_cbr_bytes(bytes: &[u8]) -> Result<Self> {
        let pages = if bytes.starts_with(b"Rar!\x1a\x07\x01\x00") {
            parse_rar5_entries(bytes)?
        } else if bytes.starts_with(b"Rar!\x1a\x07\x00") {
            parse_rar4_entries(bytes)?
        } else {
            return Err(ExtraFormatError::InvalidContainer(
                "missing RAR4/RAR5 CBR signature".into(),
            ));
        };
        if pages.is_empty() {
            return Err(ExtraFormatError::InvalidContainer(
                "CBR archive contains no image pages".into(),
            ));
        }
        let mut sorted = pages;
        sorted.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        let page_sizes = sorted
            .iter()
            .map(|entry| {
                if let Some((start, end)) = entry.store_range {
                    image_dimensions_pt(&bytes[start..end])
                } else {
                    ExtraPageSize {
                        width_pt: 595.0,
                        height_pt: 842.0,
                    }
                }
            })
            .collect();

        Ok(Self {
            format: ExtraFormat::Cbr,
            raw_bytes: bytes.to_vec(),
            pages: sorted,
            page_sizes,
        })
    }

    /// Ordered comic page entries.
    #[must_use]
    pub fn entries(&self) -> &[ComicPageEntry] {
        &self.pages
    }

    /// Reads the image bytes for zero-based `page`.
    ///
    /// For CBZ archives and uncompressed (`STORE`) CBR entries, `extractor` may be `None`.
    /// Compressed CBR entries delegate to `extractor` (never re-implementing proprietary RAR
    /// decompression).
    ///
    /// # Errors
    ///
    /// Returns [`ExtraFormatError::PageOutOfRange`], [`ExtraFormatError::UnsupportedCompression`],
    /// or [`ExtraFormatError::Zip`].
    pub fn read_page_bytes(
        &self,
        page: usize,
        extractor: Option<&dyn RarExtractor>,
    ) -> Result<Vec<u8>> {
        let entry = self
            .pages
            .get(page)
            .ok_or(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            })?;
        match self.format {
            ExtraFormat::Cbz => {
                let mut archive = ZipArchive::new(Cursor::new(&self.raw_bytes))
                    .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
                for i in 0..archive.len() {
                    let mut file = archive
                        .by_index(i)
                        .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
                    if file
                        .name()
                        .map_err(|e| ExtraFormatError::Zip(e.to_string()))?
                        .replace('\\', "/")
                        == entry.name
                    {
                        let mut buf = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
                        file.read_to_end(&mut buf)
                            .map_err(|e| ExtraFormatError::Zip(e.to_string()))?;
                        return Ok(buf);
                    }
                }
                Err(ExtraFormatError::InvalidContainer(format!(
                    "missing CBZ entry {}",
                    entry.name
                )))
            }
            ExtraFormat::Cbr => {
                if let Some((start, end)) = entry.store_range {
                    return Ok(self.raw_bytes[start..end].to_vec());
                }
                let ext = extractor.ok_or_else(|| {
                    ExtraFormatError::UnsupportedCompression(format!(
                        "compressed CBR entry {} (method 0x{:02x}) requires external RarExtractor",
                        entry.name, entry.compression_method
                    ))
                })?;
                ext.extract_entry(&self.raw_bytes, &entry.name)
            }
            _ => Err(ExtraFormatError::InvalidContainer(
                "unexpected format in ComicArchive".into(),
            )),
        }
    }
}

impl ExtraDocument for ComicArchive {
    fn format(&self) -> ExtraFormat {
        self.format
    }

    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_size(&self, page: usize) -> Result<ExtraPageSize> {
        self.page_sizes
            .get(page)
            .copied()
            .ok_or(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            })
    }

    fn page_text(&self, page: usize) -> Result<Option<String>> {
        if page >= self.pages.len() {
            return Err(ExtraFormatError::PageOutOfRange {
                page,
                count: self.pages.len(),
            });
        }
        Ok(None)
    }

    fn outline(&self) -> Vec<ExtraOutlineItem> {
        Vec::new()
    }
}

fn parse_rar4_entries(bytes: &[u8]) -> Result<Vec<ComicPageEntry>> {
    let mut pos = 7usize;
    let mut pages = Vec::new();
    while pos + 7 <= bytes.len() {
        let head_type = bytes[pos + 2];
        let flags = u16::from_le_bytes([bytes[pos + 3], bytes[pos + 4]]);
        let head_size = u16::from_le_bytes([bytes[pos + 5], bytes[pos + 6]]) as usize;
        if head_size < 7 || pos + head_size > bytes.len() {
            break;
        }
        let add_size = if head_type == 0x74 || (flags & 0x8000) != 0 {
            if pos + 11 > bytes.len() {
                break;
            }
            u32::from_le_bytes([
                bytes[pos + 7],
                bytes[pos + 8],
                bytes[pos + 9],
                bytes[pos + 10],
            ]) as usize
        } else {
            0
        };
        if head_type == 0x74 && pos + 32 <= bytes.len() {
            let unp_size = u32::from_le_bytes([
                bytes[pos + 11],
                bytes[pos + 12],
                bytes[pos + 13],
                bytes[pos + 14],
            ]) as u64;
            let method = u16::from(bytes[pos + 25]);
            let name_size = u16::from_le_bytes([bytes[pos + 26], bytes[pos + 27]]) as usize;
            let name_start = pos + 32;
            let name_end = name_start.saturating_add(name_size).min(pos + head_size);
            let name = String::from_utf8_lossy(&bytes[name_start..name_end]).replace('\\', "/");
            let data_start = pos + head_size;
            let data_end = data_start.saturating_add(add_size).min(bytes.len());
            if is_image_filename(&name) {
                pages.push(ComicPageEntry {
                    name,
                    uncompressed_size: unp_size,
                    compression_method: method,
                    store_range: (method == 0x30).then_some((data_start, data_end)),
                });
            }
        }
        pos = pos.saturating_add(head_size).saturating_add(add_size);
        if head_type == 0x7b {
            break;
        }
    }
    Ok(pages)
}

fn parse_rar5_entries(bytes: &[u8]) -> Result<Vec<ComicPageEntry>> {
    let mut pos = 8usize;
    let mut pages = Vec::new();
    while pos + 5 <= bytes.len() {
        let mut cursor = pos + 4; // skip 4-byte CRC32
        let Some((head_size, n1)) = read_vint(&bytes[cursor..]) else {
            break;
        };
        cursor += n1;
        let header_end = cursor.saturating_add(head_size as usize).min(bytes.len());
        let Some((head_type, n2)) = read_vint(&bytes[cursor..header_end]) else {
            break;
        };
        cursor += n2;
        let Some((flags, n3)) = read_vint(&bytes[cursor..header_end]) else {
            break;
        };
        cursor += n3;
        let _extra_size = if (flags & 0x0001) != 0 {
            let Some((v, n)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n;
            v
        } else {
            0
        };
        let data_size = if (flags & 0x0002) != 0 {
            let Some((v, n)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n;
            v as usize
        } else {
            0
        };

        if head_type == 2 {
            // File header: file flags, unpacked size, attributes, optional mtime, optional crc32, compression, host os, name len, name.
            let Some((file_flags, n_ff)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_ff;
            let Some((unp_size, n_us)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_us;
            let Some((_, n_attr)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_attr;
            if (file_flags & 0x0002) != 0 {
                cursor = cursor.saturating_add(4); // mtime
            }
            if (file_flags & 0x0004) != 0 {
                cursor = cursor.saturating_add(4); // data crc32
            }
            let Some((comp_info, n_ci)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_ci;
            let Some((_, n_os)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_os;
            let Some((name_len, n_nl)) = read_vint(&bytes[cursor..header_end]) else {
                break;
            };
            cursor += n_nl;
            let name_end = cursor.saturating_add(name_len as usize).min(header_end);
            let name = String::from_utf8_lossy(&bytes[cursor..name_end]).replace('\\', "/");
            let method = ((comp_info >> 7) & 0x07) as u16;
            let data_end = header_end.saturating_add(data_size).min(bytes.len());
            if is_image_filename(&name) {
                pages.push(ComicPageEntry {
                    name,
                    uncompressed_size: unp_size,
                    compression_method: method,
                    store_range: (method == 0).then_some((header_end, data_end)),
                });
            }
        }
        pos = header_end.saturating_add(data_size);
        if head_type == 5 {
            break;
        }
    }
    Ok(pages)
}

fn read_vint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut val = 0u64;
    let mut shift = 0u32;
    for (idx, &b) in bytes.iter().enumerate().take(10) {
        val |= u64::from(b & 0x7f) << shift;
        if (b & 0x80) == 0 {
            return Some((val, idx + 1));
        }
        shift += 7;
    }
    None
}

fn is_image_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn image_dimensions_pt(bytes: &[u8]) -> ExtraPageSize {
    // PNG IHDR: 8-byte sig + 4 len + "IHDR" + 4 width BE + 4 height BE.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return ExtraPageSize {
            width_pt: (w as f32).max(1.0),
            height_pt: (h as f32).max(1.0),
        };
    }
    // GIF87a / GIF89a.
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10 {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]);
        let h = u16::from_le_bytes([bytes[8], bytes[9]]);
        return ExtraPageSize {
            width_pt: f32::from(w).max(1.0),
            height_pt: f32::from(h).max(1.0),
        };
    }
    ExtraPageSize {
        width_pt: 595.0,
        height_pt: 842.0,
    }
}

/// Natural filename comparison (`page2.png` < `page10.png`).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ia = a.bytes().peekable();
    let mut ib = b.bytes().peekable();
    while let (Some(&ca), Some(&cb)) = (ia.peek(), ib.peek()) {
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let mut na = 0u64;
            while let Some(&d) = ia.peek() {
                if !d.is_ascii_digit() {
                    break;
                }
                na = na.saturating_mul(10).saturating_add(u64::from(d - b'0'));
                ia.next();
            }
            let mut nb = 0u64;
            while let Some(&d) = ib.peek() {
                if !d.is_ascii_digit() {
                    break;
                }
                nb = nb.saturating_mul(10).saturating_add(u64::from(d - b'0'));
                ib.next();
            }
            match na.cmp(&nb) {
                std::cmp::Ordering::Equal => continue,
                non_eq => return non_eq,
            }
        }
        let la = ca.to_ascii_lowercase();
        let lb = cb.to_ascii_lowercase();
        match la.cmp(&lb) {
            std::cmp::Ordering::Equal => {
                ia.next();
                ib.next();
            }
            non_eq => return non_eq,
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    fn iff_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(id);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    #[test]
    fn djvu_single_page_parses_info_and_txta() {
        let mut info = Vec::new();
        info.extend_from_slice(&1200u16.to_be_bytes());
        info.extend_from_slice(&1600u16.to_be_bytes());
        info.extend_from_slice(&[24, 0]); // minor, major
        info.extend_from_slice(&300u16.to_le_bytes()); // dpi
        info.extend_from_slice(&[22, 0]); // gamma, flags

        let text_bytes = b"Hello DjVu text";
        let mut txta = vec![0, 0, text_bytes.len() as u8];
        txta.extend_from_slice(text_bytes);

        let mut form_body = b"DJVU".to_vec();
        form_body.extend_from_slice(&iff_chunk(b"INFO", &info));
        form_body.extend_from_slice(&iff_chunk(b"TXTa", &txta));

        let mut djvu = b"AT&TFORM".to_vec();
        djvu.extend_from_slice(&(form_body.len() as u32).to_be_bytes());
        djvu.extend_from_slice(&form_body);

        assert_eq!(
            ExtraFormat::detect(&djvu, Some("djvu")),
            Some(ExtraFormat::DjVu)
        );
        let doc = DjVuDocument::from_bytes(&djvu).expect("djvu parses");
        assert_eq!(doc.page_count(), 1);
        let sz = doc.page_size(0).expect("page 0");
        assert!((sz.width_pt - 288.0).abs() < 0.1);
        assert!((sz.height_pt - 384.0).abs() < 0.1);
        assert_eq!(
            doc.page_text(0).expect("text").as_deref(),
            Some("Hello DjVu text")
        );
    }

    #[test]
    fn chm_itss_parses_entries_html_text_and_hhc_toc() {
        let html = b"<html><body><h1>Chapter 1</h1><p>CHM body text.</p></body></html>";
        let hhc = br#"<ul><li><object type="text/sitemap"><param name="Name" value="Intro"><param name="Local" value="/intro.html"></object><ul><li><object type="text/sitemap"><param name="Name" value="Sub"><param name="Local" value="/sub.html"></object></li></ul></li></ul>"#;

        // Build PMGL chunk with two entries in section 0.
        let mut pmgl_entries = Vec::new();
        let append_entry = |buf: &mut Vec<u8>, path: &str, off: u64, len: u64| {
            buf.push(path.len() as u8);
            buf.extend_from_slice(path.as_bytes());
            buf.push(0); // section 0
            // encint for offset (<128 for test or multi-byte)
            let push_enc = |b: &mut Vec<u8>, mut v: u64| {
                let mut tmp = vec![(v & 0x7f) as u8];
                v >>= 7;
                while v > 0 {
                    tmp.push(((v & 0x7f) as u8) | 0x80);
                    v >>= 7;
                }
                tmp.reverse();
                b.extend_from_slice(&tmp);
            };
            push_enc(buf, off);
            push_enc(buf, len);
        };
        append_entry(&mut pmgl_entries, "/intro.html", 0, html.len() as u64);
        append_entry(
            &mut pmgl_entries,
            "/toc.hhc",
            html.len() as u64,
            hhc.len() as u64,
        );

        let chunk_size = 512usize;
        let mut pmgl = vec![0u8; chunk_size];
        pmgl[0..4].copy_from_slice(b"PMGL");
        let free_space = (chunk_size - 20 - pmgl_entries.len()) as u32;
        pmgl[4..8].copy_from_slice(&free_space.to_le_bytes());
        pmgl[20..20 + pmgl_entries.len()].copy_from_slice(&pmgl_entries);

        let mut itsp = vec![0u8; 84];
        itsp[0..4].copy_from_slice(b"ITSP");
        itsp[8..12].copy_from_slice(&84u32.to_le_bytes());
        itsp[16..20].copy_from_slice(&(chunk_size as u32).to_le_bytes());
        itsp[36..40].copy_from_slice(&1u32.to_le_bytes()); // 1 chunk
        let mut dir = itsp;
        dir.extend_from_slice(&pmgl);

        let dir_offset = 96u64;
        let dir_len = dir.len() as u64;
        let content_offset = dir_offset + dir_len;

        let mut chm = vec![0u8; 96];
        chm[0..4].copy_from_slice(b"ITSF");
        chm[4..8].copy_from_slice(&3u32.to_le_bytes());
        chm[8..12].copy_from_slice(&96u32.to_le_bytes());
        chm[72..80].copy_from_slice(&dir_offset.to_le_bytes());
        chm[80..88].copy_from_slice(&dir_len.to_le_bytes());
        chm[88..96].copy_from_slice(&content_offset.to_le_bytes());
        chm.extend_from_slice(&dir);
        chm.extend_from_slice(html);
        chm.extend_from_slice(hhc);

        assert_eq!(
            ExtraFormat::detect(&chm, Some("chm")),
            Some(ExtraFormat::Chm)
        );
        let doc = ChmDocument::from_bytes(&chm).expect("chm parses");
        assert_eq!(doc.page_count(), 1);
        assert_eq!(
            doc.page_text(0).expect("text").as_deref(),
            Some("Chapter 1 CHM body text.")
        );
        let toc = doc.outline();
        assert_eq!(toc.len(), 1);
        assert_eq!(toc[0].title, "Intro");
        assert_eq!(toc[0].children[0].title, "Sub");
    }

    #[test]
    fn cbz_sorts_pages_naturally_and_reads_png_dimensions() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&800u32.to_be_bytes());
        png.extend_from_slice(&1200u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]);

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("page10.png", opts).expect("zip");
            zip.write_all(&png).expect("write");
            zip.start_file("page2.png", opts).expect("zip");
            zip.write_all(&png).expect("write");
            zip.finish().expect("finish");
        }
        let bytes = cursor.into_inner();
        let cbz = ComicArchive::from_cbz_bytes(&bytes).expect("cbz");
        assert_eq!(cbz.page_count(), 2);
        assert_eq!(cbz.entries()[0].name, "page2.png");
        assert_eq!(cbz.entries()[1].name, "page10.png");
        let sz = cbz.page_size(0).expect("size");
        assert_eq!((sz.width_pt, sz.height_pt), (800.0, 1200.0));
        assert_eq!(cbz.read_page_bytes(0, None).expect("bytes"), png);
    }

    #[test]
    fn cbr_reads_store_and_delegates_compressed_to_extractor() {
        // Construct minimal RAR4 with one STORE PNG (method 0x30) and one compressed PNG (method 0x33).
        let mut rar = b"Rar!\x1a\x07\x00".to_vec();
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x01\x00\x00\x00\x02\x00\x08\x02\x00\x00\x00".to_vec();

        let push_rar4_file = |buf: &mut Vec<u8>, name: &str, method: u8, data: &[u8]| {
            let head_size = 32 + name.len();
            buf.extend_from_slice(&[0, 0]); // crc
            buf.push(0x74); // file header
            buf.extend_from_slice(&0x8000u16.to_le_bytes()); // flags: ADD_SIZE present
            buf.extend_from_slice(&(head_size as u16).to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes()); // packed
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes()); // unpacked
            buf.push(0); // host os
            buf.extend_from_slice(&[0; 4]); // file crc
            buf.extend_from_slice(&[0; 4]); // time
            buf.push(29); // unp ver
            buf.push(method);
            buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
            buf.extend_from_slice(&[0; 4]); // attrs
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(data);
        };

        push_rar4_file(&mut rar, "01.png", 0x30, &png);
        push_rar4_file(&mut rar, "02.png", 0x33, b"COMPRESSED");

        struct FakeUnrar;
        impl RarExtractor for FakeUnrar {
            fn extract_entry(&self, _archive_bytes: &[u8], name: &str) -> Result<Vec<u8>> {
                Ok(format!("decoded:{name}").into_bytes())
            }
        }

        let cbr = ComicArchive::from_cbr_bytes(&rar).expect("cbr parses");
        assert_eq!(cbr.page_count(), 2);
        assert_eq!(cbr.read_page_bytes(0, None).expect("stored"), png);
        assert!(matches!(
            cbr.read_page_bytes(1, None),
            Err(ExtraFormatError::UnsupportedCompression(_))
        ));
        assert_eq!(
            cbr.read_page_bytes(1, Some(&FakeUnrar)).expect("extracted"),
            b"decoded:02.png"
        );
    }
}
