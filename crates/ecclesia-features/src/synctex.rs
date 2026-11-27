//! SyncTeX `.synctex` / `.synctex.gz` parser, forward/inverse search, auto-reload tracking, and
//! editor command templates (plan R.27, ported from SumatraPDF Enhanced's `src/PdfSync.cpp`).

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ecclesia_mupdf::{PagePoint, PageRect};
use flate2::read::GzDecoder;

/// Maximum vertical distance (in points) for fallback inverse-search line matching.
const INVERSE_FALLBACK_EPSILON_Y: f32 = 24.0;
/// Maximum line offset tolerated when forward-searching for a line without an exact record.
const FORWARD_LINE_EPSILON: u32 = 5;

/// Errors returned by SyncTeX parsing and forward/inverse search.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyncTexError {
    /// No `.synctex` or `.synctex.gz` file exists beside the PDF.
    NotFound(PathBuf),
    /// Could not read or decompress the SyncTeX file.
    Io(String),
    /// SyncTeX preamble or record syntax is invalid.
    Malformed(String),
    /// Source file was not referenced in the SyncTeX input table.
    UnknownSourceFile(String),
    /// Source filename contained unsafe shell/CLI characters (GHSA-jf4v-rw66-j4w2 defense).
    UnsafeSourcePath(String),
    /// No SyncTeX node matched the requested page/point or source line.
    NoMatch,
}

impl fmt::Display for SyncTexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "synctex file not found for {}", p.display()),
            Self::Io(msg) => write!(f, "synctex I/O error: {msg}"),
            Self::Malformed(msg) => write!(f, "malformed synctex data: {msg}"),
            Self::UnknownSourceFile(name) => write!(f, "unknown synctex source file: {name}"),
            Self::UnsafeSourcePath(name) => {
                write!(f, "rejected unsafe synctex source path: {name}")
            }
            Self::NoMatch => f.write_str("no matching synctex location"),
        }
    }
}

impl std::error::Error for SyncTexError {}

/// Convenience result alias for SyncTeX operations.
pub type Result<T> = std::result::Result<T, SyncTexError>;

/// Built-in editor command templates for inverse search (plan R.27 step 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorPreset {
    /// Visual Studio Code (`code --goto "%f:%l"`).
    VsCode,
    /// Neovim / Vim (`nvim +%l "%f"`).
    Vim,
    /// Emacs (`emacsclient -n +%l "%f"`).
    Emacs,
    /// TeXstudio (`texstudio "%f" --line %l`).
    TexStudio,
    /// User-supplied command template containing `%f` (file path) and `%l` (1-based line).
    Custom(String),
}

impl EditorPreset {
    /// Command template string with `%f` and `%l` placeholders.
    #[must_use]
    pub fn template(&self) -> &str {
        match self {
            Self::VsCode => "code --goto \"%f:%l\"",
            Self::Vim => "nvim +%l \"%f\"",
            Self::Emacs => "emacsclient -n +%l \"%f\"",
            Self::TexStudio => "texstudio \"%f\" --line %l",
            Self::Custom(tmpl) => tmpl.as_str(),
        }
    }

    /// Formats the editor invocation command for `file` and 1-based `line`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncTexError::UnsafeSourcePath`] if `file` contains shell-escape characters
    /// (`"`, `<`, `>`, `|`, `\r`, `\n`, `\t`).
    pub fn format_command(&self, file: &Path, line: u32) -> Result<String> {
        let raw = file.to_string_lossy();
        if contains_illegal_path_chars(&raw) {
            return Err(SyncTexError::UnsafeSourcePath(raw.into_owned()));
        }
        let line_str = line.max(1).to_string();
        Ok(self.template().replace("%f", &raw).replace("%l", &line_str))
    }
}

fn contains_illegal_path_chars(path: &str) -> bool {
    path.chars()
        .any(|c| matches!(c, '"' | '<' | '>' | '|' | '\r' | '\n' | '\t'))
}

/// Forward-search result mapping a `.tex` source line to PDF highlight rectangles (plan R.27).
#[derive(Debug, Clone, PartialEq)]
pub struct ForwardMatch {
    /// Zero-based PDF page index.
    pub page: usize,
    /// Highlight boxes in top-left PDF page points.
    pub rects: Vec<PageRect>,
}

/// Inverse-search result mapping a PDF click to a source file, line, and column (plan R.27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InverseMatch {
    /// Normalized source `.tex` path.
    pub file: PathBuf,
    /// One-based source line number.
    pub line: u32,
    /// Zero-based source column number.
    pub column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SyncNode {
    page: usize,
    tag: u32,
    line: u32,
    column: u32,
    bbox: PageRect,
}

/// Parsed SyncTeX index supporting forward search, inverse search, and mtime auto-reload.
#[derive(Debug, Clone)]
pub struct SyncTexIndex {
    base_dir: PathBuf,
    sync_path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
    inputs: HashMap<u32, PathBuf>,
    nodes: Vec<SyncNode>,
}

impl SyncTexIndex {
    /// Discovers and parses `<pdf_stem>.synctex` or `<pdf_stem>.synctex.gz` alongside `pdf_path`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncTexError::NotFound`] or [`SyncTexError::Malformed`] if unavailable or corrupt.
    pub fn open_for_pdf(pdf_path: &Path) -> Result<Self> {
        let stem = pdf_path.with_extension("");
        let plain = PathBuf::from(format!("{}.synctex", stem.display()));
        let gz = PathBuf::from(format!("{}.synctex.gz", stem.display()));
        let sync_path = if plain.exists() {
            plain
        } else if gz.exists() {
            gz
        } else {
            return Err(SyncTexError::NotFound(pdf_path.to_path_buf()));
        };
        let bytes = fs::read(&sync_path).map_err(|e| SyncTexError::Io(e.to_string()))?;
        let base_dir = pdf_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mtime = fs::metadata(&sync_path).and_then(|m| m.modified()).ok();
        let mut index = Self::from_bytes(&bytes, &base_dir)?;
        index.sync_path = Some(sync_path);
        index.last_mtime = mtime;
        Ok(index)
    }

    /// Parses raw `.synctex` or gzip-compressed `.synctex.gz` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SyncTexError::Malformed`] or [`SyncTexError::Io`] on invalid data.
    pub fn from_bytes(bytes: &[u8], base_dir: &Path) -> Result<Self> {
        let text = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut decoder = GzDecoder::new(bytes);
            let mut decompressed = String::new();
            decoder
                .read_to_string(&mut decompressed)
                .map_err(|e| SyncTexError::Io(format!("gzip decode failed: {e}")))?;
            decompressed
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };

        let mut unit: f64 = 1.0;
        let mut magnification: f64 = 1000.0;
        let mut x_offset: f64 = 0.0;
        let mut y_offset: f64 = 0.0;
        let mut inputs: HashMap<u32, PathBuf> = HashMap::new();
        let mut nodes: Vec<SyncNode> = Vec::new();
        let mut current_page: Option<usize> = None;

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("Unit:") {
                if let Ok(v) = rest.trim().parse::<f64>()
                    && v > 0.0
                {
                    unit = v;
                }
            } else if let Some(rest) = line.strip_prefix("Magnification:") {
                if let Ok(v) = rest.trim().parse::<f64>()
                    && v > 0.0
                {
                    magnification = v;
                }
            } else if let Some(rest) = line.strip_prefix("X Offset:") {
                if let Ok(v) = rest.trim().parse::<f64>() {
                    x_offset = v;
                }
            } else if let Some(rest) = line.strip_prefix("Y Offset:") {
                if let Ok(v) = rest.trim().parse::<f64>() {
                    y_offset = v;
                }
            } else if let Some(rest) = line.strip_prefix("Input:") {
                if let Some((tag_str, path_str)) = rest.split_once(':')
                    && let Ok(tag) = tag_str.trim().parse::<u32>()
                {
                    let cleaned = path_str.trim().replace('*', " ");
                    let pb = PathBuf::from(&cleaned);
                    let resolved = if pb.is_absolute() {
                        pb
                    } else {
                        base_dir.join(pb)
                    };
                    inputs.insert(tag, resolved);
                }
            } else if let Some(rest) = line.strip_prefix('{') {
                if let Ok(page_1based) = rest.trim().parse::<usize>()
                    && page_1based > 0
                {
                    current_page = Some(page_1based - 1);
                }
            } else if line.starts_with('}') {
                current_page = None;
            } else if let Some(page) = current_page {
                let scale = unit / 65781.76 * (magnification / 1000.0);
                if let Some(node) = parse_synctex_record(line, page, scale, x_offset, y_offset) {
                    nodes.push(node);
                }
            }
        }

        if inputs.is_empty() && nodes.is_empty() {
            return Err(SyncTexError::Malformed(
                "no SyncTeX input declarations or page nodes found".into(),
            ));
        }

        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            sync_path: None,
            last_mtime: None,
            inputs,
            nodes,
        })
    }

    /// Checks whether the backing `.synctex(.gz)` file has changed on disk and reloads it if so
    /// (plan R.27 step 4). Returns `Ok(true)` if reloaded.
    ///
    /// # Errors
    ///
    /// Propagates parsing or I/O errors when reloading a changed file.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let Some(path) = &self.sync_path else {
            return Ok(false);
        };
        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime != self.last_mtime {
            let bytes = fs::read(path).map_err(|e| SyncTexError::Io(e.to_string()))?;
            let mut refreshed = Self::from_bytes(&bytes, &self.base_dir)?;
            refreshed.sync_path = self.sync_path.clone();
            refreshed.last_mtime = mtime;
            *self = refreshed;
            return Ok(true);
        }
        Ok(false)
    }

    /// Forward search: maps `(source_file, line)` to the matching PDF page and highlight boxes
    /// (plan R.27 step 2).
    ///
    /// # Errors
    ///
    /// Returns [`SyncTexError::UnknownSourceFile`] or [`SyncTexError::NoMatch`].
    pub fn forward_search(
        &self,
        source_file: &Path,
        line: u32,
        _column: Option<u32>,
    ) -> Result<ForwardMatch> {
        let target_tags = self.matching_tags(source_file);
        if target_tags.is_empty() {
            return Err(SyncTexError::UnknownSourceFile(
                source_file.display().to_string(),
            ));
        }

        let mut exact_nodes: Vec<&SyncNode> = self
            .nodes
            .iter()
            .filter(|n| target_tags.contains(&n.tag) && n.line == line)
            .collect();

        if exact_nodes.is_empty() {
            // Find closest line within FORWARD_LINE_EPSILON.
            let mut best_diff = u32::MAX;
            let mut best_line = None;
            for n in &self.nodes {
                if !target_tags.contains(&n.tag) {
                    continue;
                }
                let diff = n.line.abs_diff(line);
                if diff <= FORWARD_LINE_EPSILON && diff < best_diff {
                    best_diff = diff;
                    best_line = Some(n.line);
                }
            }
            if let Some(nearest) = best_line {
                exact_nodes = self
                    .nodes
                    .iter()
                    .filter(|n| target_tags.contains(&n.tag) && n.line == nearest)
                    .collect();
            }
        }

        let first_page = exact_nodes
            .first()
            .map(|n| n.page)
            .ok_or(SyncTexError::NoMatch)?;
        let rects: Vec<PageRect> = exact_nodes
            .into_iter()
            .filter(|n| n.page == first_page)
            .map(|n| n.bbox)
            .collect();

        Ok(ForwardMatch {
            page: first_page,
            rects,
        })
    }

    /// Inverse search: maps `(page, point)` in PDF page points to the source `.tex` file, line,
    /// and column (plan R.27 step 3).
    ///
    /// # Errors
    ///
    /// Returns [`SyncTexError::NoMatch`], [`SyncTexError::UnknownSourceFile`], or
    /// [`SyncTexError::UnsafeSourcePath`].
    pub fn inverse_search(&self, page: usize, point: PagePoint) -> Result<InverseMatch> {
        let page_nodes: Vec<&SyncNode> = self.nodes.iter().filter(|n| n.page == page).collect();
        if page_nodes.is_empty() {
            return Err(SyncTexError::NoMatch);
        }

        let mut best_inside: Option<(&SyncNode, f32)> = None;
        let mut best_near: Option<(&SyncNode, f32)> = None;

        for node in page_nodes {
            let r = node.bbox;
            let inside = point.x >= r.x0 && point.x <= r.x1 && point.y >= r.y0 && point.y <= r.y1;
            if inside {
                let area = r.width().max(1.0) * r.height().max(1.0);
                if best_inside.is_none_or(|(_, best_area)| area < best_area) {
                    best_inside = Some((node, area));
                }
            } else {
                let dx = if point.x < r.x0 {
                    r.x0 - point.x
                } else if point.x > r.x1 {
                    point.x - r.x1
                } else {
                    0.0
                };
                let dy = if point.y < r.y0 {
                    r.y0 - point.y
                } else if point.y > r.y1 {
                    point.y - r.y1
                } else {
                    0.0
                };
                if dy <= INVERSE_FALLBACK_EPSILON_Y {
                    let dist2 = dx * dx + dy * dy;
                    if best_near.is_none_or(|(_, d)| dist2 < d) {
                        best_near = Some((node, dist2));
                    }
                }
            }
        }

        let chosen = best_inside
            .map(|(n, _)| n)
            .or_else(|| best_near.map(|(n, _)| n))
            .ok_or(SyncTexError::NoMatch)?;

        let path = self
            .inputs
            .get(&chosen.tag)
            .ok_or_else(|| SyncTexError::UnknownSourceFile(format!("tag {}", chosen.tag)))?;
        let path_str = path.to_string_lossy();
        if contains_illegal_path_chars(&path_str) {
            return Err(SyncTexError::UnsafeSourcePath(path_str.into_owned()));
        }

        Ok(InverseMatch {
            file: path.clone(),
            line: chosen.line,
            column: chosen.column,
        })
    }

    fn matching_tags(&self, query: &Path) -> Vec<u32> {
        let q_norm = normalize_path_key(query);
        let q_base = query
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_ascii_lowercase);
        let mut exact = Vec::new();
        let mut by_name = Vec::new();
        for (&tag, path) in &self.inputs {
            if normalize_path_key(path) == q_norm {
                exact.push(tag);
            } else if let Some(base) = &q_base
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|b| b.eq_ignore_ascii_case(base))
            {
                by_name.push(tag);
            }
        }
        if exact.is_empty() { by_name } else { exact }
    }
}

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn parse_synctex_record(
    line: &str,
    page: usize,
    scale: f64,
    x_offset: f64,
    y_offset: f64,
) -> Option<SyncNode> {
    let first = line.as_bytes().first().copied()?;
    if !matches!(first, b'[' | b'(' | b'v' | b'h' | b'x' | b'k' | b'g' | b'$') {
        return None;
    }
    let rest = &line[1..];
    let mut parts = rest.split(':');
    let src_part = parts.next()?;
    let pos_part = parts.next()?;
    let size_part = parts.next();

    let mut src_nums = src_part.split(',');
    let tag: u32 = src_nums.next()?.parse().ok()?;
    let line_no: u32 = src_nums.next()?.parse().ok()?;
    let col_no: u32 = src_nums.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut pos_nums = pos_part.split(',');
    let h: f64 = pos_nums.next()?.parse().ok()?;
    let v: f64 = pos_nums.next()?.parse().ok()?;

    let (w, ht, dp) = if let Some(sz) = size_part {
        let mut s = sz.split(',');
        let w: f64 = s.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let ht: f64 = s.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let dp: f64 = s.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        (w, ht, dp)
    } else {
        (657817.6, 657817.6, 0.0)
    };

    let x0 = ((h - x_offset) * scale) as f32;
    let width = (w.abs() * scale).max(6.0) as f32;
    let baseline_y = ((v - y_offset) * scale) as f32;
    let top_h = (ht.abs() * scale).max(8.0) as f32;
    let depth = (dp.abs() * scale) as f32;

    Some(SyncNode {
        page,
        tag,
        line: line_no,
        column: col_no,
        bbox: PageRect {
            x0,
            y0: baseline_y - top_h,
            x1: x0 + width,
            y1: baseline_y + depth,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const SAMPLE_SYNCTEX: &str = "\
SyncTeX Version:1
Input:1:main.tex
Input:2:chapters/intro.tex
Output:pdf
Unit:1
Magnification:1000
Content:
{1
[1,10:4736286,6578176:26214400,786432,0
h2,42,3:4736286,9867264:19660800,655360,131072
]
}
";

    #[test]
    fn parses_gz_synctex_and_performs_forward_and_inverse_search() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(SAMPLE_SYNCTEX.as_bytes())
            .expect("write gz");
        let gz_bytes = encoder.finish().expect("finish gz");

        let dir = Path::new("/proj");
        let index = SyncTexIndex::from_bytes(&gz_bytes, dir).expect("parse gz");

        // Forward search on chapters/intro.tex line 42
        let fwd = index
            .forward_search(Path::new("chapters/intro.tex"), 42, None)
            .expect("forward match");
        assert_eq!(fwd.page, 0);
        assert_eq!(fwd.rects.len(), 1);
        let r = fwd.rects[0];
        assert!((r.x0 - 72.0).abs() < 0.5, "x0={}", r.x0);
        assert!((r.y1 - 152.0).abs() < 2.0, "y1={}", r.y1);

        // Inverse search inside that box returns chapters/intro.tex:42:3
        let inv = index
            .inverse_search(
                0,
                PagePoint {
                    x: (r.x0 + r.x1) * 0.5,
                    y: (r.y0 + r.y1) * 0.5,
                },
            )
            .expect("inverse match");
        assert_eq!(inv.line, 42);
        assert_eq!(inv.column, 3);
        assert!(inv.file.ends_with("intro.tex"));
    }

    #[test]
    fn formats_editor_presets_and_rejects_shell_injection() {
        let file = Path::new("/docs/paper.tex");
        assert_eq!(
            EditorPreset::VsCode
                .format_command(file, 27)
                .expect("vscode"),
            "code --goto \"/docs/paper.tex:27\""
        );
        assert_eq!(
            EditorPreset::Vim.format_command(file, 27).expect("vim"),
            "nvim +27 \"/docs/paper.tex\""
        );
        assert_eq!(
            EditorPreset::Emacs.format_command(file, 27).expect("emacs"),
            "emacsclient -n +27 \"/docs/paper.tex\""
        );
        assert_eq!(
            EditorPreset::TexStudio
                .format_command(file, 27)
                .expect("texstudio"),
            "texstudio \"/docs/paper.tex\" --line 27"
        );

        let evil = "SyncTeX Version:1\nInput:1:foo.tex\" --install-extension evil\n{1\nh1,5:4736286,6578176:6578176,655360,0\n}\n";
        let idx = SyncTexIndex::from_bytes(evil.as_bytes(), Path::new("/tmp")).expect("parse");
        let err = idx
            .inverse_search(0, PagePoint { x: 80.0, y: 98.0 })
            .expect_err("must reject unsafe path");
        assert!(matches!(err, SyncTexError::UnsafeSourcePath(_)));
    }
}
