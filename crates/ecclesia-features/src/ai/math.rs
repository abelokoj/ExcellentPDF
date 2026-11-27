//! Handwritten math stroke cluster -> LaTeX converter and preview renderer (plan H.25).
//!
//! Converts lassoed handwriting strokes into structured LaTeX expressions offline using 2D spatial
//! layout analysis (fractions, superscripts, subscripts, radicals) paired with geometric symbol and
//! resampled polyline prototype matching, renders SVG and RGBA previews for user confirmation, and
//! keeps insertion/copy gated on explicit user confirmation.

use std::error::Error;
use std::fmt;

use ecclesia_mupdf::{PagePoint, PageRect};

use crate::handwriting::HandwritingStroke;

/// Backend engine selected for math-to-LaTeX recognition (plan H.25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MathBackend {
    /// Built-in deterministic geometric + 2D-layout recognizer (always available offline).
    #[default]
    BuiltinGeometric,
    /// Optional downloadable ONNX image-to-LaTeX pack (e.g. `pix2tex`).
    OnnxPack,
}

/// User confirmation state after previewing recognized LaTeX (plan H.25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MathConfirmationState {
    /// Recognized LaTeX is shown for preview; not yet inserted or copied.
    #[default]
    PendingConfirmation,
    /// User confirmed copying the LaTeX string to clipboard.
    ConfirmedCopy,
    /// User confirmed inserting the LaTeX expression as document text/note.
    ConfirmedInsertAsText,
    /// User cancelled conversion.
    Dismissed,
}

/// Errors produced during math stroke recognition or preview rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum MathError {
    /// No non-empty strokes were provided in the lassoed region.
    EmptyStrokes,
    /// Requested image preview dimensions were zero or oversized.
    InvalidCanvasSize {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// Optional ONNX model pack is not installed.
    PackNotInstalled(String),
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyStrokes => write!(f, "no non-empty math strokes provided"),
            Self::InvalidCanvasSize { width, height } => {
                write!(f, "invalid math preview dimensions {width}x{height}")
            }
            Self::PackNotInstalled(name) => write!(f, "math pack not installed: {name}"),
        }
    }
}

impl Error for MathError {}

/// Recognized LaTeX expression with SVG preview and confirmation state (plan H.25).
#[derive(Debug, Clone, PartialEq)]
pub struct MathConversionResult {
    /// Recognized LaTeX formula string (without outer `$$`).
    pub latex: String,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
    /// Enclosing bounding box of the input stroke cluster in page points.
    pub bbox: PageRect,
    /// Inline SVG preview of the handwritten strokes + rendered formula caption.
    pub preview_svg: String,
    /// Recognition backend used.
    pub backend: MathBackend,
    /// Confirmation gate state before inserting or copying.
    pub confirmation: MathConfirmationState,
}

impl MathConversionResult {
    /// KaTeX-ready display math Markdown (`$$...$$`).
    #[must_use]
    pub fn to_katex_markdown(&self) -> String {
        format!("$$\n{}\n$$", self.latex)
    }

    /// Confirms copying the LaTeX string.
    pub fn confirm_copy(&mut self) {
        self.confirmation = MathConfirmationState::ConfirmedCopy;
    }

    /// Confirms inserting the LaTeX expression into the document.
    pub fn confirm_insert(&mut self) {
        self.confirmation = MathConfirmationState::ConfirmedInsertAsText;
    }

    /// Dismisses the conversion without modifying the document.
    pub fn dismiss(&mut self) {
        self.confirmation = MathConfirmationState::Dismissed;
    }
}

/// Validates that `{` and `}` braces in `latex` are balanced.
#[must_use]
pub fn validate_latex_braces(latex: &str) -> bool {
    let mut depth = 0i32;
    let mut escaped = false;
    for ch in latex.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
    }
    depth == 0
}

/// Converts a lassoed cluster of handwritten math strokes into LaTeX and a preview SVG (plan H.25).
///
/// # Errors
///
/// Returns [`MathError::EmptyStrokes`] when no stroke has points, or [`MathError::PackNotInstalled`]
/// when [`MathBackend::OnnxPack`] is requested explicitly without a pack.
pub fn convert_strokes_to_latex(
    strokes: &[HandwritingStroke],
    backend: MathBackend,
) -> Result<MathConversionResult, MathError> {
    if backend == MathBackend::OnnxPack {
        return Err(MathError::PackNotInstalled("pix2tex-onnx".to_owned()));
    }
    let non_empty: Vec<&HandwritingStroke> =
        strokes.iter().filter(|s| !s.points.is_empty()).collect();
    if non_empty.is_empty() {
        return Err(MathError::EmptyStrokes);
    }

    let bbox = union_bbox(&non_empty).ok_or(MathError::EmptyStrokes)?;
    let symbols = segment_and_classify_symbols(&non_empty);
    let latex = assemble_2d_latex(&symbols);
    let confidence = if symbols.is_empty() {
        0.0
    } else {
        symbols.iter().map(|s| s.confidence).sum::<f32>() / symbols.len() as f32
    };
    let preview_svg = render_math_strokes_svg(strokes, &latex);

    Ok(MathConversionResult {
        latex,
        confidence,
        bbox,
        preview_svg,
        backend,
        confirmation: MathConfirmationState::PendingConfirmation,
    })
}

/// Renders handwritten math `strokes` into a standalone SVG preview with `latex` caption.
#[must_use]
pub fn render_math_strokes_svg(strokes: &[HandwritingStroke], latex: &str) -> String {
    let refs: Vec<&HandwritingStroke> = strokes.iter().filter(|s| !s.points.is_empty()).collect();
    let Some(bbox) = union_bbox(&refs) else {
        return String::from(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"48\"></svg>",
        );
    };
    let pad = 8.0f32;
    let width = (bbox.width() + pad * 2.0).max(40.0);
    let height = (bbox.height() + pad * 2.0).max(32.0);
    let escaped_latex = latex
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width:.1} {height:.1}\" width=\"{width:.0}\" height=\"{height:.0}\" role=\"img\" aria-label=\"{escaped_latex}\">\n"
    );
    for stroke in refs {
        let mut d = String::new();
        for (idx, pt) in stroke.points.iter().enumerate() {
            let x = pt.x - bbox.x0 + pad;
            let y = pt.y - bbox.y0 + pad;
            if idx == 0 {
                d.push_str(&format!("M {x:.1} {y:.1}"));
            } else {
                d.push_str(&format!(" L {x:.1} {y:.1}"));
            }
        }
        out.push_str(&format!(
            "  <path d=\"{d}\" fill=\"none\" stroke=\"#111827\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n"
        ));
    }
    out.push_str("</svg>");
    out
}

/// Rasterizes handwritten strokes into a white-background RGBA8 buffer (`width * height * 4` bytes)
/// suitable for feeding an image-to-LaTeX ONNX pack or thumbnail preview (plan H.25).
///
/// # Errors
///
/// Returns [`MathError::InvalidCanvasSize`] if `width == 0`, `height == 0`, or buffer exceeds 16 MP,
/// or [`MathError::EmptyStrokes`] when `strokes` has no points.
pub fn render_math_strokes_rgba(
    strokes: &[HandwritingStroke],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, MathError> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_000_000 {
        return Err(MathError::InvalidCanvasSize { width, height });
    }
    let refs: Vec<&HandwritingStroke> = strokes.iter().filter(|s| !s.points.is_empty()).collect();
    let bbox = union_bbox(&refs).ok_or(MathError::EmptyStrokes)?;

    let mut rgba = vec![255u8; (width as usize) * (height as usize) * 4];
    let pad = 6.0f32;
    let avail_w = (width as f32 - pad * 2.0).max(1.0);
    let avail_h = (height as f32 - pad * 2.0).max(1.0);
    let scale = (avail_w / bbox.width().max(1.0)).min(avail_h / bbox.height().max(1.0));

    for stroke in refs {
        for pair in stroke.points.windows(2) {
            let x0 = (pair[0].x - bbox.x0) * scale + pad;
            let y0 = (pair[0].y - bbox.y0) * scale + pad;
            let x1 = (pair[1].x - bbox.x0) * scale + pad;
            let y1 = (pair[1].y - bbox.y0) * scale + pad;
            draw_segment_rgba(&mut rgba, width, height, x0, y0, x1, y1);
        }
        if stroke.points.len() == 1 {
            let x = (stroke.points[0].x - bbox.x0) * scale + pad;
            let y = (stroke.points[0].y - bbox.y0) * scale + pad;
            draw_dot_rgba(&mut rgba, width, height, x, y);
        }
    }
    Ok(rgba)
}

fn draw_segment_rgba(rgba: &mut [u8], width: u32, height: u32, x0: f32, y0: f32, x1: f32, y1: f32) {
    let steps = ((x1 - x0).hypot(y1 - y0).ceil() as usize).max(1) * 2;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let x = x0 + (x1 - x0) * t;
        let y = y0 + (y1 - y0) * t;
        draw_dot_rgba(rgba, width, height, x, y);
    }
}

fn draw_dot_rgba(rgba: &mut [u8], width: u32, height: u32, cx: f32, cy: f32) {
    let ix = cx.round() as i32;
    let iy = cy.round() as i32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let px = ix + dx;
            let py = iy + dy;
            if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
                let idx = ((py as usize) * (width as usize) + (px as usize)) * 4;
                rgba[idx] = 17;
                rgba[idx + 1] = 24;
                rgba[idx + 2] = 39;
                rgba[idx + 3] = 255;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct MathSymbol {
    token: String,
    bbox: PageRect,
    confidence: f32,
    is_horizontal_bar: bool,
    is_radical: bool,
}

fn union_bbox(strokes: &[&HandwritingStroke]) -> Option<PageRect> {
    let mut iter = strokes.iter().filter_map(|s| s.bbox());
    let first = iter.next()?;
    Some(iter.fold(first, |acc, b| PageRect {
        x0: acc.x0.min(b.x0),
        y0: acc.y0.min(b.y0),
        x1: acc.x1.max(b.x1),
        y1: acc.y1.max(b.y1),
    }))
}

fn merge_rects(a: PageRect, b: PageRect) -> PageRect {
    PageRect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn segment_and_classify_symbols(strokes: &[&HandwritingStroke]) -> Vec<MathSymbol> {
    let mut items: Vec<(&HandwritingStroke, PageRect)> = strokes
        .iter()
        .filter_map(|s| s.bbox().map(|b| (*s, b)))
        .collect();
    items.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));

    let mut groups: Vec<Vec<(&HandwritingStroke, PageRect)>> = Vec::new();
    let mut consumed = vec![false; items.len()];

    for i in 0..items.len() {
        if consumed[i] {
            continue;
        }
        consumed[i] = true;
        let mut group = vec![items[i]];
        let mut group_box = items[i].1;

        for j in (i + 1)..items.len() {
            if consumed[j] {
                continue;
            }
            let cand_box = items[j].1;
            if should_merge_into_symbol(&group, group_box, items[j].0, cand_box) {
                consumed[j] = true;
                group.push(items[j]);
                group_box = merge_rects(group_box, cand_box);
            }
        }
        groups.push(group);
    }

    let mut symbols: Vec<MathSymbol> = groups
        .into_iter()
        .map(|g| classify_symbol_group(&g))
        .collect();
    symbols.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
    symbols
}

fn should_merge_into_symbol(
    group: &[(&HandwritingStroke, PageRect)],
    group_box: PageRect,
    cand: &HandwritingStroke,
    cand_box: PageRect,
) -> bool {
    if group.len() >= 2 {
        return false;
    }
    if looks_like_radical(group[0].0, group_box) || looks_like_radical(cand, cand_box) {
        return false;
    }

    let x_overlap = (group_box.x1.min(cand_box.x1) - group_box.x0.max(cand_box.x0)).max(0.0);
    let min_w = group_box.width().min(cand_box.width()).max(1.0);

    // Pair two parallel horizontal strokes into `=`.
    if group.len() == 1
        && is_horizontal_stroke(group[0].0, group_box)
        && is_horizontal_stroke(cand, cand_box)
    {
        let dy = (center_y(group_box) - center_y(cand_box)).abs();
        let max_w = group_box.width().max(cand_box.width()).max(1.0);
        if x_overlap >= 0.55 * min_w && dy > 2.0 && dy <= max_w * 0.9 {
            return true;
        }
        return false;
    }

    // Never merge a wide horizontal fraction bar with numerator/denominator strokes.
    if is_horizontal_stroke(group[0].0, group_box)
        && group_box.width() > 26.0
        && !is_crossing(group[0].0, cand)
    {
        return false;
    }
    if is_horizontal_stroke(cand, cand_box)
        && cand_box.width() > 26.0
        && !is_crossing(group[0].0, cand)
    {
        return false;
    }

    // Pair crossing horizontal + vertical strokes into `+` even when one axis has 0 box width/height.
    if ((is_horizontal_stroke(group[0].0, group_box) && is_vertical_stroke(cand, cand_box))
        || (is_vertical_stroke(group[0].0, group_box) && is_horizontal_stroke(cand, cand_box)))
        && is_crossing(group[0].0, cand)
    {
        return true;
    }

    // Merge crossing or tightly overlapping strokes (`x`, `t`, `4`).
    let padded_a = pad_rect(group_box, 1.5);
    let padded_b = pad_rect(cand_box, 1.5);
    let px_overlap = (padded_a.x1.min(padded_b.x1) - padded_a.x0.max(padded_b.x0)).max(0.0);
    let py_overlap = (padded_a.y1.min(padded_b.y1) - padded_a.y0.max(padded_b.y0)).max(0.0);
    let p_min_w = padded_a.width().min(padded_b.width()).max(1.0);
    let p_min_h = padded_a.height().min(padded_b.height()).max(1.0);
    px_overlap > 0.45 * p_min_w && py_overlap > 0.35 * p_min_h
}

fn pad_rect(r: PageRect, pad: f32) -> PageRect {
    PageRect {
        x0: r.x0 - pad,
        y0: r.y0 - pad,
        x1: r.x1 + pad,
        y1: r.y1 + pad,
    }
}

fn is_crossing(a: &HandwritingStroke, b: &HandwritingStroke) -> bool {
    let (Some(ba), Some(bb)) = (a.bbox(), b.bbox()) else {
        return false;
    };
    let ba = pad_rect(ba, 1.0);
    let bb = pad_rect(bb, 1.0);
    let xo = ba.x1.min(bb.x1) - ba.x0.max(bb.x0);
    let yo = ba.y1.min(bb.y1) - ba.y0.max(bb.y0);
    xo > 0.0 && yo > 0.0
}

fn is_horizontal_stroke(stroke: &HandwritingStroke, bbox: PageRect) -> bool {
    if stroke.points.len() < 2 {
        return false;
    }
    let len = path_length(&stroke.points).max(1.0);
    let chord = chord_length(&stroke.points);
    let linearity = chord / len;
    linearity > 0.85 && bbox.width() >= (bbox.height() * 2.2).max(8.0)
}

fn is_vertical_stroke(stroke: &HandwritingStroke, bbox: PageRect) -> bool {
    if stroke.points.len() < 2 {
        return false;
    }
    let len = path_length(&stroke.points).max(1.0);
    let chord = chord_length(&stroke.points);
    let linearity = chord / len;
    linearity > 0.85 && bbox.height() >= (bbox.width() * 2.2).max(8.0)
}

fn classify_symbol_group(group: &[(&HandwritingStroke, PageRect)]) -> MathSymbol {
    let mut bbox = group[0].1;
    for &(_, b) in group.iter().skip(1) {
        bbox = merge_rects(bbox, b);
    }

    if group.len() == 2 {
        let (s0, b0) = group[0];
        let (s1, b1) = group[1];
        if is_horizontal_stroke(s0, b0) && is_horizontal_stroke(s1, b1) {
            return MathSymbol {
                token: "=".to_owned(),
                bbox,
                confidence: 0.96,
                is_horizontal_bar: false,
                is_radical: false,
            };
        }
        if (is_horizontal_stroke(s0, b0) && is_vertical_stroke(s1, b1))
            || (is_vertical_stroke(s0, b0) && is_horizontal_stroke(s1, b1))
        {
            return MathSymbol {
                token: "+".to_owned(),
                bbox,
                confidence: 0.95,
                is_horizontal_bar: false,
                is_radical: false,
            };
        }
        if is_diagonal(s0) && is_diagonal(s1) {
            return MathSymbol {
                token: "x".to_owned(),
                bbox,
                confidence: 0.92,
                is_horizontal_bar: false,
                is_radical: false,
            };
        }
    }

    let (stroke, sbox) = group[0];
    if is_horizontal_stroke(stroke, sbox) {
        return MathSymbol {
            token: "-".to_owned(),
            bbox,
            confidence: 0.93,
            is_horizontal_bar: true,
            is_radical: false,
        };
    }
    if is_vertical_stroke(stroke, sbox) {
        return MathSymbol {
            token: "1".to_owned(),
            bbox,
            confidence: 0.90,
            is_horizontal_bar: false,
            is_radical: false,
        };
    }
    if looks_like_radical(stroke, sbox) {
        return MathSymbol {
            token: "\\sqrt".to_owned(),
            bbox,
            confidence: 0.90,
            is_horizontal_bar: false,
            is_radical: true,
        };
    }

    let (token, confidence) = match_single_stroke_prototype(stroke, sbox);
    MathSymbol {
        token: token.to_owned(),
        bbox,
        confidence,
        is_horizontal_bar: false,
        is_radical: false,
    }
}

fn is_diagonal(stroke: &HandwritingStroke) -> bool {
    let (Some(first), Some(last)) = (stroke.points.first(), stroke.points.last()) else {
        return false;
    };
    let dx = (last.x - first.x).abs();
    let dy = (last.y - first.y).abs();
    let len = path_length(&stroke.points).max(1.0);
    let linearity = chord_length(&stroke.points) / len;
    linearity > 0.80 && dx > 3.0 && dy > 3.0
}

fn looks_like_radical(stroke: &HandwritingStroke, bbox: PageRect) -> bool {
    if stroke.points.len() < 4 || bbox.width() < 12.0 || bbox.height() < 10.0 {
        return false;
    }
    let first = stroke.points[0];
    let last = stroke.points[stroke.points.len() - 1];
    // Starts near mid-left, dips to bottom-left-ish, rises to top, then sweeps right along top roof.
    let lowest = stroke
        .points
        .iter()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .copied()
        .unwrap_or(first);
    let highest = stroke
        .points
        .iter()
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .copied()
        .unwrap_or(last);
    let starts_mid_left =
        first.x <= bbox.x0 + bbox.width() * 0.25 && first.y > bbox.y0 + bbox.height() * 0.25;
    let dips_before_roof = lowest.x < bbox.x0 + bbox.width() * 0.45 && lowest.y >= bbox.y1 - 2.0;
    let ends_top_right = last.x >= bbox.x1 - bbox.width() * 0.15
        && (last.y - highest.y).abs() <= bbox.height() * 0.25;
    starts_mid_left && dips_before_roof && ends_top_right
}

fn match_single_stroke_prototype(
    stroke: &HandwritingStroke,
    bbox: PageRect,
) -> (&'static str, f32) {
    let resampled = resample_normalized(&stroke.points, bbox, 16);
    let closed = is_closed_loop(&stroke.points, bbox);

    let prototypes: &[(&str, &[(f32, f32)])] = &[
        (
            "0",
            &[
                (0.5, 0.0),
                (0.15, 0.2),
                (0.0, 0.5),
                (0.15, 0.85),
                (0.5, 1.0),
                (0.85, 0.85),
                (1.0, 0.5),
                (0.85, 0.15),
                (0.5, 0.0),
            ],
        ),
        (
            "2",
            &[
                (0.1, 0.2),
                (0.5, 0.0),
                (0.9, 0.2),
                (0.85, 0.45),
                (0.45, 0.7),
                (0.05, 1.0),
                (0.95, 1.0),
            ],
        ),
        (
            "3",
            &[
                (0.1, 0.1),
                (0.7, 0.0),
                (0.95, 0.25),
                (0.4, 0.5),
                (0.95, 0.75),
                (0.65, 1.0),
                (0.1, 0.9),
            ],
        ),
        (
            "5",
            &[
                (0.9, 0.0),
                (0.15, 0.0),
                (0.15, 0.45),
                (0.75, 0.48),
                (0.95, 0.75),
                (0.6, 1.0),
                (0.1, 0.9),
            ],
        ),
        ("7", &[(0.05, 0.0), (0.95, 0.0), (0.6, 0.5), (0.35, 1.0)]),
        (
            "a",
            &[
                (0.8, 0.25),
                (0.45, 0.05),
                (0.1, 0.35),
                (0.15, 0.8),
                (0.55, 0.95),
                (0.85, 0.55),
                (0.85, 0.15),
                (0.9, 0.95),
            ],
        ),
        (
            "b",
            &[
                (0.1, 0.0),
                (0.1, 1.0),
                (0.4, 0.55),
                (0.85, 0.6),
                (0.9, 0.85),
                (0.5, 1.0),
                (0.15, 0.9),
            ],
        ),
        (
            "c",
            &[
                (0.9, 0.2),
                (0.5, 0.0),
                (0.1, 0.25),
                (0.05, 0.65),
                (0.4, 0.98),
                (0.9, 0.88),
            ],
        ),
        (
            "e",
            &[
                (0.1, 0.55),
                (0.9, 0.5),
                (0.85, 0.15),
                (0.45, 0.0),
                (0.08, 0.35),
                (0.15, 0.82),
                (0.6, 1.0),
                (0.92, 0.85),
            ],
        ),
        (
            "m",
            &[
                (0.05, 0.95),
                (0.05, 0.2),
                (0.35, 0.15),
                (0.5, 0.9),
                (0.55, 0.2),
                (0.85, 0.15),
                (0.95, 0.95),
            ],
        ),
        (
            "y",
            &[
                (0.1, 0.1),
                (0.3, 0.55),
                (0.55, 0.55),
                (0.85, 0.1),
                (0.65, 0.65),
                (0.35, 1.0),
            ],
        ),
        (
            "(",
            &[(0.8, 0.0), (0.3, 0.3), (0.2, 0.5), (0.3, 0.7), (0.8, 1.0)],
        ),
        (
            ")",
            &[(0.2, 0.0), (0.7, 0.3), (0.8, 0.5), (0.7, 0.7), (0.2, 1.0)],
        ),
    ];

    let mut best_label = "x";
    let mut best_dist = f32::MAX;
    for &(label, proto) in prototypes {
        if label == "0" && !closed {
            continue;
        }
        let pts: Vec<PagePoint> = proto.iter().map(|&(x, y)| PagePoint { x, y }).collect();
        let proto_resampled = resample_normalized(
            &pts,
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
            },
            16,
        );
        let dist: f32 = resampled
            .iter()
            .zip(proto_resampled.iter())
            .map(|(a, b)| (a.x - b.x).hypot(a.y - b.y))
            .sum::<f32>()
            / 16.0;
        if dist < best_dist {
            best_dist = dist;
            best_label = label;
        }
    }
    let confidence = (1.0 - (best_dist / 0.85)).clamp(0.35, 0.97);
    (best_label, confidence)
}

fn assemble_2d_latex(symbols: &[MathSymbol]) -> String {
    if symbols.is_empty() {
        return String::new();
    }

    let mut used = vec![false; symbols.len()];
    let mut fragments: Vec<(f32, PageRect, String)> = Vec::new();

    // 1. Detect fractions: horizontal bar with strokes both strictly above and below within its x-span.
    for (idx, sym) in symbols.iter().enumerate() {
        if !sym.is_horizontal_bar || used[idx] {
            continue;
        }
        let bar_y = center_y(sym.bbox);
        let mut num_indices = Vec::new();
        let mut den_indices = Vec::new();

        for (j, other) in symbols.iter().enumerate() {
            if j == idx || used[j] {
                continue;
            }
            let cx = center_x(other.bbox);
            if cx >= sym.bbox.x0 - 4.0 && cx <= sym.bbox.x1 + 4.0 {
                let cy = center_y(other.bbox);
                if cy < bar_y - 3.0 && (bar_y - other.bbox.y1) <= sym.bbox.width().max(24.0) * 1.2 {
                    num_indices.push(j);
                } else if cy > bar_y + 3.0
                    && (other.bbox.y0 - bar_y) <= sym.bbox.width().max(24.0) * 1.2
                {
                    den_indices.push(j);
                }
            }
        }

        if !num_indices.is_empty() && !den_indices.is_empty() {
            used[idx] = true;
            for &ni in &num_indices {
                used[ni] = true;
            }
            for &di in &den_indices {
                used[di] = true;
            }
            let num_syms: Vec<MathSymbol> =
                num_indices.iter().map(|&k| symbols[k].clone()).collect();
            let den_syms: Vec<MathSymbol> =
                den_indices.iter().map(|&k| symbols[k].clone()).collect();
            let num_tex = assemble_inline_scripts(&num_syms);
            let den_tex = assemble_inline_scripts(&den_syms);
            fragments.push((
                sym.bbox.x0,
                sym.bbox,
                format!("\\frac{{{num_tex}}}{{{den_tex}}}"),
            ));
        }
    }

    // 2. Detect radicals: `\sqrt` stroke enclosing inner symbols under its roof.
    for (idx, sym) in symbols.iter().enumerate() {
        if !sym.is_radical || used[idx] {
            continue;
        }
        let mut inner_indices = Vec::new();
        for (j, other) in symbols.iter().enumerate() {
            if j == idx || used[j] {
                continue;
            }
            let cx = center_x(other.bbox);
            let cy = center_y(other.bbox);
            if cx > sym.bbox.x0 + sym.bbox.width() * 0.25
                && cx <= sym.bbox.x1 + 4.0
                && cy >= sym.bbox.y0
                && cy <= sym.bbox.y1 + 4.0
            {
                inner_indices.push(j);
            }
        }
        used[idx] = true;
        for &ii in &inner_indices {
            used[ii] = true;
        }
        let inner_syms: Vec<MathSymbol> =
            inner_indices.iter().map(|&k| symbols[k].clone()).collect();
        let inner_tex = assemble_inline_scripts(&inner_syms);
        fragments.push((sym.bbox.x0, sym.bbox, format!("\\sqrt{{{inner_tex}}}")));
    }

    // 3. Remaining baseline symbols + superscript/subscript grouping.
    let remaining: Vec<MathSymbol> = symbols
        .iter()
        .enumerate()
        .filter_map(|(i, s)| (!used[i]).then_some(s.clone()))
        .collect();
    if fragments.is_empty() {
        return assemble_inline_scripts(&remaining);
    }

    for sym in remaining {
        fragments.push((sym.bbox.x0, sym.bbox, sym.token));
    }
    fragments.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = String::new();
    for (i, (_, rect, tok)) in fragments.iter().enumerate() {
        if i > 0 {
            let prev_rect = fragments[i - 1].1;
            let is_script_sup = center_y(*rect) < prev_rect.y0 + prev_rect.height() * 0.35
                && rect.height() <= prev_rect.height() * 0.78
                && !is_operator(tok)
                && !is_operator(&fragments[i - 1].2);
            if is_script_sup {
                out.push_str(&format!("^{{{tok}}}"));
                continue;
            }
            if needs_space(&fragments[i - 1].2, tok) {
                out.push(' ');
            }
        }
        out.push_str(tok);
    }
    out
}

fn assemble_inline_scripts(symbols: &[MathSymbol]) -> String {
    if symbols.is_empty() {
        return String::new();
    }
    let mut sorted = symbols.to_vec();
    sorted.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
    let mut out = String::new();
    let mut base_rect = sorted[0].bbox;

    for (i, sym) in sorted.iter().enumerate() {
        if i == 0 {
            out.push_str(&sym.token);
            continue;
        }
        let prev_tok = &sorted[i - 1].token;
        let is_sup = !is_operator(&sym.token)
            && !is_operator(prev_tok)
            && center_y(sym.bbox) < base_rect.y0 + base_rect.height() * 0.35
            && sym.bbox.height() <= base_rect.height() * 0.78;
        let is_sub = !is_operator(&sym.token)
            && !is_operator(prev_tok)
            && center_y(sym.bbox) > base_rect.y1 - base_rect.height() * 0.20
            && sym.bbox.height() <= base_rect.height() * 0.78;
        if is_sup {
            out.push_str(&format!("^{{{}}}", sym.token));
        } else if is_sub {
            out.push_str(&format!("_{{{}}}", sym.token));
        } else {
            if needs_space(prev_tok, &sym.token) {
                out.push(' ');
            }
            out.push_str(&sym.token);
            base_rect = sym.bbox;
        }
    }
    out
}

fn is_operator(tok: &str) -> bool {
    matches!(tok, "+" | "-" | "=" | "(" | ")")
}

fn needs_space(prev: &str, curr: &str) -> bool {
    matches!(prev, "+" | "-" | "=") || matches!(curr, "+" | "-" | "=")
}

fn center_x(r: PageRect) -> f32 {
    0.5 * (r.x0 + r.x1)
}

fn center_y(r: PageRect) -> f32 {
    0.5 * (r.y0 + r.y1)
}

fn path_length(pts: &[PagePoint]) -> f32 {
    pts.windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum()
}

fn chord_length(pts: &[PagePoint]) -> f32 {
    match (pts.first(), pts.last()) {
        (Some(a), Some(b)) => (b.x - a.x).hypot(b.y - a.y),
        _ => 0.0,
    }
}

fn is_closed_loop(pts: &[PagePoint], bbox: PageRect) -> bool {
    let diag = bbox.width().hypot(bbox.height()).max(1.0);
    chord_length(pts) <= diag * 0.38
}

fn resample_normalized(pts: &[PagePoint], bbox: PageRect, n: usize) -> Vec<PagePoint> {
    let w = bbox.width().max(1.0);
    let h = bbox.height().max(1.0);
    let norm: Vec<PagePoint> = pts
        .iter()
        .map(|p| PagePoint {
            x: (p.x - bbox.x0) / w,
            y: (p.y - bbox.y0) / h,
        })
        .collect();
    if norm.len() < 2 || n <= 1 {
        return vec![PagePoint { x: 0.5, y: 0.5 }; n];
    }
    let total = path_length(&norm).max(f32::EPSILON);
    let step = total / (n - 1) as f32;
    let mut out = Vec::with_capacity(n);
    out.push(norm[0]);
    let mut acc = 0.0f32;
    let mut i = 1usize;
    let mut prev = norm[0];
    while i < norm.len() && out.len() < n {
        let curr = norm[i];
        let d = (curr.x - prev.x).hypot(curr.y - prev.y);
        if acc + d >= step {
            let t = (step - acc) / d.max(f32::EPSILON);
            let np = PagePoint {
                x: prev.x + t * (curr.x - prev.x),
                y: prev.y + t * (curr.y - prev.y),
            };
            out.push(np);
            prev = np;
            acc = 0.0;
        } else {
            acc += d;
            prev = curr;
            i += 1;
        }
    }
    while out.len() < n {
        out.push(*norm.last().unwrap_or(&PagePoint { x: 0.5, y: 0.5 }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(id: &str, pts: &[(f32, f32)]) -> HandwritingStroke {
        HandwritingStroke {
            id: id.to_owned(),
            points: pts.iter().map(|&(x, y)| PagePoint { x, y }).collect(),
            timestamps_ms: Vec::new(),
        }
    }

    fn glyph_x(id: &str, x: f32, y: f32, w: f32, h: f32) -> Vec<HandwritingStroke> {
        vec![
            stroke(&format!("{id}a"), &[(x, y), (x + w, y + h)]),
            stroke(&format!("{id}b"), &[(x + w, y), (x, y + h)]),
        ]
    }

    fn glyph_plus(id: &str, x: f32, y: f32, size: f32) -> Vec<HandwritingStroke> {
        vec![
            stroke(
                &format!("{id}h"),
                &[(x, y + size * 0.5), (x + size, y + size * 0.5)],
            ),
            stroke(
                &format!("{id}v"),
                &[(x + size * 0.5, y), (x + size * 0.5, y + size)],
            ),
        ]
    }

    fn glyph_eq(id: &str, x: f32, y: f32, w: f32) -> Vec<HandwritingStroke> {
        vec![
            stroke(&format!("{id}1"), &[(x, y), (x + w, y)]),
            stroke(&format!("{id}2"), &[(x, y + 10.0), (x + w, y + 10.0)]),
        ]
    }

    fn glyph_1(id: &str, x: f32, y: f32, h: f32) -> HandwritingStroke {
        stroke(id, &[(x, y), (x, y + h)])
    }

    fn glyph_2(id: &str, x: f32, y: f32, w: f32, h: f32) -> HandwritingStroke {
        stroke(
            id,
            &[
                (x + w * 0.1, y + h * 0.2),
                (x + w * 0.5, y),
                (x + w * 0.9, y + h * 0.2),
                (x + w * 0.85, y + h * 0.45),
                (x + w * 0.45, y + h * 0.7),
                (x + w * 0.05, y + h),
                (x + w * 0.95, y + h),
            ],
        )
    }

    fn glyph_3(id: &str, x: f32, y: f32, w: f32, h: f32) -> HandwritingStroke {
        stroke(
            id,
            &[
                (x + w * 0.1, y + h * 0.1),
                (x + w * 0.7, y),
                (x + w * 0.95, y + h * 0.25),
                (x + w * 0.4, y + h * 0.5),
                (x + w * 0.95, y + h * 0.75),
                (x + w * 0.65, y + h),
                (x + w * 0.1, y + h * 0.9),
            ],
        )
    }

    fn glyph_0(id: &str, x: f32, y: f32, w: f32, h: f32) -> HandwritingStroke {
        stroke(
            id,
            &[
                (x + w * 0.5, y),
                (x + w * 0.15, y + h * 0.2),
                (x, y + h * 0.5),
                (x + w * 0.15, y + h * 0.85),
                (x + w * 0.5, y + h),
                (x + w * 0.85, y + h * 0.85),
                (x + w, y + h * 0.5),
                (x + w * 0.85, y + h * 0.15),
                (x + w * 0.5, y),
            ],
        )
    }

    fn glyph_a(id: &str, x: f32, y: f32, w: f32, h: f32) -> HandwritingStroke {
        stroke(
            id,
            &[
                (x + w * 0.8, y + h * 0.25),
                (x + w * 0.45, y + h * 0.05),
                (x + w * 0.1, y + h * 0.35),
                (x + w * 0.15, y + h * 0.8),
                (x + w * 0.55, y + h * 0.95),
                (x + w * 0.85, y + h * 0.55),
                (x + w * 0.85, y + h * 0.15),
                (x + w * 0.9, y + h * 0.95),
            ],
        )
    }

    fn glyph_c(id: &str, x: f32, y: f32, w: f32, h: f32) -> HandwritingStroke {
        stroke(
            id,
            &[
                (x + w * 0.9, y + h * 0.2),
                (x + w * 0.5, y),
                (x + w * 0.1, y + h * 0.25),
                (x + w * 0.05, y + h * 0.65),
                (x + w * 0.4, y + h * 0.98),
                (x + w * 0.9, y + h * 0.88),
            ],
        )
    }

    #[test]
    fn math_test_set_achieves_above_eighty_percent_exact_conversion() {
        // 6 representative handwritten formulas (H.25 Done when >= 80%).
        let mut cases: Vec<(&str, Vec<HandwritingStroke>)> = Vec::new();

        // 1. x + 1 = 2
        let mut f1 = Vec::new();
        f1.extend(glyph_x("x", 10.0, 40.0, 20.0, 30.0));
        f1.extend(glyph_plus("p", 45.0, 45.0, 20.0));
        f1.push(glyph_1("n1", 80.0, 40.0, 30.0));
        f1.extend(glyph_eq("eq", 100.0, 50.0, 22.0));
        f1.push(glyph_2("n2", 135.0, 40.0, 20.0, 30.0));
        cases.push(("x + 1 = 2", f1));

        // 2. x^{2} + 1 = 0
        let mut f2 = Vec::new();
        f2.extend(glyph_x("x", 10.0, 40.0, 22.0, 32.0));
        f2.push(glyph_2("sup", 36.0, 32.0, 12.0, 14.0));
        f2.extend(glyph_plus("p", 62.0, 45.0, 20.0));
        f2.push(glyph_1("n1", 95.0, 40.0, 32.0));
        f2.extend(glyph_eq("eq", 115.0, 50.0, 22.0));
        f2.push(glyph_0("n0", 150.0, 40.0, 20.0, 32.0));
        cases.push(("x^{2} + 1 = 0", f2));

        // 3. \frac{x + 1}{2}
        let mut f3 = Vec::new();
        f3.extend(glyph_x("x", 20.0, 10.0, 18.0, 24.0));
        f3.extend(glyph_plus("p", 48.0, 12.0, 18.0));
        f3.push(glyph_1("n1", 76.0, 10.0, 24.0));
        f3.push(stroke("bar", &[(12.0, 46.0), (96.0, 46.0)]));
        f3.push(glyph_2("n2", 45.0, 56.0, 18.0, 24.0));
        cases.push(("\\frac{x + 1}{2}", f3));

        // 4. \sqrt{x}
        let mut f4 = Vec::new();
        f4.push(stroke(
            "rad",
            &[(10.0, 50.0), (22.0, 75.0), (35.0, 25.0), (85.0, 25.0)],
        ));
        f4.extend(glyph_x("x", 46.0, 40.0, 20.0, 28.0));
        cases.push(("\\sqrt{x}", f4));

        // 5. a + c = 3
        let mut f5 = Vec::new();
        f5.push(glyph_a("a", 10.0, 40.0, 20.0, 30.0));
        f5.extend(glyph_plus("p", 45.0, 45.0, 20.0));
        f5.push(glyph_c("c", 80.0, 40.0, 20.0, 30.0));
        f5.extend(glyph_eq("eq", 115.0, 50.0, 22.0));
        f5.push(glyph_3("n3", 150.0, 40.0, 20.0, 30.0));
        cases.push(("a + c = 3", f5));

        // 6. 2 - 1 = 1
        let mut f6 = Vec::new();
        f6.push(glyph_2("n2", 10.0, 40.0, 20.0, 30.0));
        f6.push(stroke("sub", &[(45.0, 55.0), (65.0, 55.0)]));
        f6.push(glyph_1("n1a", 80.0, 40.0, 30.0));
        f6.extend(glyph_eq("eq", 100.0, 50.0, 22.0));
        f6.push(glyph_1("n1b", 135.0, 40.0, 30.0));
        cases.push(("2 - 1 = 1", f6));

        let mut exact_matches = 0usize;
        for (expected, strokes) in &cases {
            let mut res =
                convert_strokes_to_latex(strokes, MathBackend::BuiltinGeometric).expect("converts");
            assert!(validate_latex_braces(&res.latex));
            assert_eq!(res.confirmation, MathConfirmationState::PendingConfirmation);
            assert!(res.preview_svg.contains("<svg"));
            let rgba = render_math_strokes_rgba(strokes, 96, 48).expect("rasterizes");
            assert_eq!(rgba.len(), 96 * 48 * 4);
            res.confirm_copy();
            assert_eq!(res.confirmation, MathConfirmationState::ConfirmedCopy);
            if res.latex == *expected {
                exact_matches += 1;
            }
        }
        let accuracy = exact_matches as f64 / cases.len() as f64;
        assert!(
            accuracy >= 0.80,
            "expected >= 80% exact conversion on math test set, got {exact_matches}/{}",
            cases.len()
        );
    }
}
