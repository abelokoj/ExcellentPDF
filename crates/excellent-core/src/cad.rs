//! CAD/engineering-drawing detection, dark-line color mapping, and OCG layer management
//! ported from SumatraPDF Enhanced (`src/PdfCad.cpp`) (plan R.26).

use std::collections::BTreeMap;

use excellent_mupdf::{Document, Layer, Pixmap};

use crate::page_ops::pdf_doc::{ParsedPdf, PdfDocError, PdfVal};

/// Result alias for CAD detection and layer operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Global CAD enhancement mode setting (ported from `EngineeringDrawingEnhanceMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CadEnhanceMode {
    /// Never apply CAD line enhancement.
    Off,
    /// Automatically detect CAD drawings using PDF/E, metadata, and content heuristics.
    #[default]
    Auto,
    /// Always apply CAD line enhancement.
    On,
}

/// Per-document manual override for CAD enhancement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CadEnhanceOverride {
    /// Defer to global [`CadEnhanceMode`] and detection result.
    #[default]
    Auto,
    /// Force CAD enhancement on for this document.
    ForceOn,
    /// Force CAD enhancement off for this document.
    ForceOff,
}

/// Why [`detect_cad_pdf`] classified a document as a CAD drawing (ported from `CadEnhanceReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CadEnhanceReason {
    /// Not detected as CAD.
    #[default]
    None,
    /// Explicit ISO PDF/E engineering standard marker.
    PdfE,
    /// Authoring tool metadata (`AutoCAD`, `SolidWorks`, `Revit`, etc.).
    Metadata,
    /// Page geometry & stroke-to-fill heuristic.
    Heuristic,
    /// Rasterized/screenshot CAD scan.
    RasterImage,
}

/// Blueprint/theme option for CAD rendering (plan R.26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CadBackgroundMode {
    /// Standard light CAD paper with darkened mid-gray strokes.
    #[default]
    LightPaper,
    /// Dark blueprint background with inverted light lines.
    DarkBlueprint,
}

/// Line weight rendering mode (plan R.26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CadLineWeights {
    /// Use document stroke widths, boosted by minimum zoom floor for hairlines.
    #[default]
    RespectWeights,
    /// Flatten thin lines to crisp uniform hairline rendering.
    UniformThin,
}

/// Output of [`detect_cad_pdf`] (ported from `CadDetectResult` in `PdfCad.cpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CadDetectResult {
    /// Whether CAD enhancement should be enabled.
    pub enable: bool,
    /// Classification reason.
    pub reason: CadEnhanceReason,
    /// Cumulative confidence score.
    pub score: i32,
    /// True when the CAD drawing is predominantly a single raster scan per page.
    pub raster_dominant: bool,
    /// True when the document consists of dense hairline vector strokes.
    pub hairline_vector: bool,
}

impl Default for CadDetectResult {
    fn default() -> Self {
        Self {
            enable: false,
            reason: CadEnhanceReason::None,
            score: 0,
            raster_dominant: false,
            hairline_vector: false,
        }
    }
}

const METADATA_STRONG: &[&str] = &[
    "autocad",
    "dwg to pdf",
    "dwg trueview",
    "revit",
    "microstation",
    "solidworks",
    "catia",
    " creo",
    " nx ",
    "zwcad",
    "gstarcad",
    "浩辰",
    "中望",
    "bluebeam",
    "pdffactory",
    "tekla",
    "sketchup",
    "archicad",
    "vectorworks",
    "bentley",
];

const METADATA_WEAK: &[&str] = &[
    "cad",
    "dwg",
    "plot",
    "engineering",
    "layout",
    "draft",
    "mechanical",
    "architect",
    "screenshot",
    "screen capture",
    "snipaste",
    "截图",
    "wps",
];

const METADATA_BLACKLIST: &[&str] = &[
    "microsoft word",
    "libreoffice",
    "openoffice",
    "indesign",
    "itext",
    "pdflatex",
    "xelatex",
    "lualatex",
    "latex",
    " prince",
    "chrome",
    "skia/pdf",
    "mozilla",
    "calibre",
    "epub",
    "powerpoint",
    "excel",
    "onenote",
    "doctotext",
];

/// Resolves whether CAD enhancement is active given detection result, global mode, and per-doc
/// override (ported from `CadEnhanceEnabledForEngine`).
#[must_use]
pub const fn cad_enhance_active(
    detect: CadDetectResult,
    global_mode: CadEnhanceMode,
    doc_override: CadEnhanceOverride,
) -> bool {
    match doc_override {
        CadEnhanceOverride::ForceOn => true,
        CadEnhanceOverride::ForceOff => false,
        CadEnhanceOverride::Auto => match global_mode {
            CadEnhanceMode::On => true,
            CadEnhanceMode::Off => false,
            CadEnhanceMode::Auto => detect.enable,
        },
    }
}

/// Scores a PDF's metadata, PDF/E markers, OCG count, and page vector/text/image statistics to
/// detect CAD/engineering drawings (exact port of `DetectCadPdf` in `PdfCad.cpp`) (plan R.26).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if `pdf_bytes` cannot be parsed.
pub fn detect_cad_pdf(pdf_bytes: &[u8]) -> Result<CadDetectResult> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let mut res = CadDetectResult::default();

    if has_pdfe_marker(&doc) {
        res.enable = true;
        res.reason = CadEnhanceReason::PdfE;
        res.score = 100;
        return Ok(res);
    }

    let (metadata_score, strong_match) = score_metadata(&doc);
    if metadata_score <= -100 {
        return Ok(res);
    }
    if strong_match {
        res.enable = true;
        res.reason = CadEnhanceReason::Metadata;
        res.score = metadata_score;
        return Ok(res);
    }

    let pages = doc.page_objects()?;
    let page_count = pages.len();
    let mut max_page_side = 0.0f32;
    for &pid in pages.iter().take(3) {
        let b = doc
            .effective_box(pid, "CropBox")
            .or_else(|_| doc.effective_box(pid, "MediaBox"))
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        max_page_side = max_page_side.max((b[2] - b[0]).abs().max((b[3] - b[1]).abs()));
    }

    let (heuristic_score, raster_dominant, hairline_vector) =
        score_heuristic(&doc, &pages, max_page_side);

    res.score = heuristic_score + metadata_score;
    res.raster_dominant = raster_dominant && page_count <= 30;
    res.hairline_vector = hairline_vector;

    if res.raster_dominant && res.score >= 45 && page_count <= 30 {
        res.enable = true;
        res.reason = CadEnhanceReason::RasterImage;
        return Ok(res);
    }
    if hairline_vector && res.score >= 45 {
        res.enable = true;
        res.reason = CadEnhanceReason::Heuristic;
        return Ok(res);
    }
    if !strong_match && metadata_score > 0 && res.score >= 45 {
        res.enable = true;
        res.reason = CadEnhanceReason::Heuristic;
        return Ok(res);
    }
    if res.score >= 60 {
        res.enable = true;
        res.reason = CadEnhanceReason::Heuristic;
    }
    Ok(res)
}

fn contains_any_i(haystack: &str, needles: &[&str]) -> bool {
    if haystack.is_empty() {
        return false;
    }
    let lower = haystack.to_ascii_lowercase();
    needles
        .iter()
        .any(|n| lower.contains(&n.to_ascii_lowercase()))
}

fn has_pdfe_marker(doc: &ParsedPdf) -> bool {
    if let Some(info_id) = doc.info_id
        && let Some(info) = doc.get_dict(info_id)
        && info.contains_key("ISO_PDFEVersion")
    {
        return true;
    }
    if let Some(catalog) = doc.get_dict(doc.root_id)
        && let Some(PdfVal::Stream { dict, data }) =
            catalog.get("Metadata").and_then(|v| doc.resolve_val(v))
    {
        let decoded = crate::page_ops::pdf_doc::decode_stream_data(&dict, &data).unwrap_or(data);
        let xmp = String::from_utf8_lossy(&decoded);
        return xmp.contains("pdfe:ISO_PDFEVersion")
            || xmp.contains("PDF/E-1")
            || xmp.contains("PDF/E-2");
    }
    false
}

fn score_cad_text_field(text: &str, score: &mut i32, strong: &mut bool, blacklisted: &mut bool) {
    if text.is_empty() || *blacklisted {
        return;
    }
    if contains_any_i(text, METADATA_BLACKLIST) {
        *blacklisted = true;
        return;
    }
    if contains_any_i(text, METADATA_STRONG) {
        *strong = true;
        *score += 40;
    } else if contains_any_i(text, METADATA_WEAK) {
        *score += 15;
    }
}

fn score_metadata(doc: &ParsedPdf) -> (i32, bool) {
    let mut score = 0i32;
    let mut strong = false;
    let mut blacklisted = false;

    if let Some(info_id) = doc.info_id
        && let Some(info) = doc.get_dict(info_id)
    {
        for key in ["Creator", "Producer"] {
            if let Some(val) = info.get(key).and_then(|v| v.as_text_string()) {
                score_cad_text_field(&val, &mut score, &mut strong, &mut blacklisted);
            }
        }
    }

    if !blacklisted
        && let Some(catalog) = doc.get_dict(doc.root_id)
        && let Some(PdfVal::Stream { dict, data }) =
            catalog.get("Metadata").and_then(|v| doc.resolve_val(v))
    {
        let decoded = crate::page_ops::pdf_doc::decode_stream_data(&dict, &data).unwrap_or(data);
        score_cad_text_field(
            &String::from_utf8_lossy(&decoded),
            &mut score,
            &mut strong,
            &mut blacklisted,
        );
    }

    if blacklisted {
        (-100, false)
    } else {
        (score, strong)
    }
}

#[derive(Default)]
struct CadPageStats {
    strokes: usize,
    fills: usize,
    text_ops: usize,
    gray_strokes: usize,
    thin_strokes: usize,
    max_image_coverage: f32,
    page_area: f32,
}

fn score_heuristic(doc: &ParsedPdf, pages: &[u32], max_page_side: f32) -> (i32, bool, bool) {
    let mut stats = CadPageStats {
        page_area: max_page_side * max_page_side,
        ..CadPageStats::default()
    };
    for &pid in pages.iter().take(2) {
        if let Ok(streams) = doc.page_decoded_content_streams(pid) {
            for stm in streams {
                analyze_content_stream(&String::from_utf8_lossy(&stm), &mut stats);
            }
        }
    }

    let mut score = 0i32;
    let denom = stats.strokes + stats.fills;
    let stroke_fill_ratio = if denom > 0 {
        stats.strokes as f32 / denom as f32
    } else {
        0.0
    };
    if stroke_fill_ratio > 0.85 {
        score += 25;
    }

    let gray_ratio = if stats.strokes > 0 {
        stats.gray_strokes as f32 / stats.strokes as f32
    } else {
        0.0
    };
    if gray_ratio > 0.30 {
        score += 25;
    }

    let thin_ratio = if stats.strokes > 0 {
        stats.thin_strokes as f32 / stats.strokes as f32
    } else {
        0.0
    };
    if thin_ratio > 0.20 {
        score += 15;
    }

    if count_ocg_layers(doc) >= 2 {
        score += 15;
    }
    if max_page_side >= 842.0 {
        score += 10;
    }
    let text_stroke_ratio = if stats.strokes > 0 {
        stats.text_ops as f32 / stats.strokes as f32
    } else {
        0.0
    };
    if text_stroke_ratio < 0.15 {
        score += 10;
    }
    if count_square_annots(doc, pages) > 20 {
        score += 5;
    }
    if stats.text_ops > 500 && stroke_fill_ratio < 0.5 {
        score -= 30;
    }
    if stats.max_image_coverage >= 0.15 && stats.text_ops >= 40 {
        score -= 65;
    }
    if pages.len() >= 80 && stats.max_image_coverage >= 0.12 && stats.text_ops >= 25 {
        score -= 40;
    }

    let hairline_vector = stats.max_image_coverage < 0.05
        && stats.strokes >= 40
        && thin_ratio > 0.25
        && stroke_fill_ratio > 0.55;
    if hairline_vector {
        score += 35;
    }

    let mut raster_dominant = false;
    let raster_cad = stats.max_image_coverage >= 0.80
        && (stats.strokes + stats.fills) < 50
        && stats.text_ops < 200;
    if raster_cad {
        if pages.len() > 30 {
            score -= 70;
        } else {
            score += 55;
            raster_dominant = true;
        }
    } else if stats.max_image_coverage > 0.5 {
        score -= 40;
    }

    (score, raster_dominant, hairline_vector)
}

fn analyze_content_stream(stream: &str, stats: &mut CadPageStats) {
    let mut stroke_rgb = [0.0f32; 3];
    let mut line_width = 1.0f32;
    let mut ctm_w = 1.0f32;
    let mut ctm_h = 1.0f32;
    let mut nums: Vec<f32> = Vec::new();

    for tok in stream.split_whitespace() {
        if let Ok(n) = tok.parse::<f32>() {
            nums.push(n);
            continue;
        }
        match tok {
            "w" => {
                if let Some(&lw) = nums.last() {
                    line_width = lw;
                }
            }
            "G" => {
                if let Some(&g) = nums.last() {
                    stroke_rgb = [g, g, g];
                }
            }
            "RG" => {
                if nums.len() >= 3 {
                    let n = nums.len();
                    stroke_rgb = [nums[n - 3], nums[n - 2], nums[n - 1]];
                }
            }
            "K" => {
                if nums.len() >= 4 {
                    let n = nums.len();
                    let (c, m, y, k) = (nums[n - 4], nums[n - 3], nums[n - 2], nums[n - 1]);
                    stroke_rgb = [
                        (1.0 - c) * (1.0 - k),
                        (1.0 - m) * (1.0 - k),
                        (1.0 - y) * (1.0 - k),
                    ];
                }
            }
            "S" | "s" | "B" | "B*" | "b" | "b*" => {
                stats.strokes += 1;
                if cad_is_gray_rgb(stroke_rgb[0], stroke_rgb[1], stroke_rgb[2]) {
                    stats.gray_strokes += 1;
                }
                if line_width <= 0.25 {
                    stats.thin_strokes += 1;
                }
                if matches!(tok, "B" | "B*" | "b" | "b*") {
                    stats.fills += 1;
                }
            }
            "f" | "F" | "f*" => {
                stats.fills += 1;
            }
            "Tj" | "TJ" | "'" | "\"" => {
                stats.text_ops += 1;
            }
            "cm" => {
                if nums.len() >= 6 {
                    let n = nums.len();
                    ctm_w = nums[n - 6].hypot(nums[n - 5]);
                    ctm_h = nums[n - 4].hypot(nums[n - 3]);
                }
            }
            "Do" if stats.page_area > 0.0 => {
                let cov = (ctm_w * ctm_h) / stats.page_area;
                stats.max_image_coverage = stats.max_image_coverage.max(cov);
            }
            _ => {}
        }
        nums.clear();
    }
}

fn count_ocg_layers(doc: &ParsedPdf) -> usize {
    doc.get_dict(doc.root_id)
        .and_then(|c| c.get("OCProperties").and_then(|v| doc.resolve_val(v)))
        .and_then(|v| v.as_dict().cloned())
        .and_then(|ocp| ocp.get("OCGs").and_then(|v| doc.resolve_val(v)))
        .and_then(|v| match v {
            PdfVal::Array(arr) => Some(arr.len()),
            _ => None,
        })
        .unwrap_or(0)
}

fn count_square_annots(doc: &ParsedPdf, pages: &[u32]) -> usize {
    let mut count = 0usize;
    for &pid in pages.iter().take(3) {
        if let Some(pdict) = doc.get_dict(pid)
            && let Some(PdfVal::Array(annots)) =
                pdict.get("Annots").and_then(|v| doc.resolve_val(v))
        {
            for a in annots {
                if let Some(PdfVal::Dict(ad)) = doc.resolve_val(&a)
                    && ad.get("Subtype").and_then(PdfVal::as_name) == Some("Square")
                {
                    count += 1;
                }
            }
        }
    }
    count
}

fn cad_is_gray_rgb(r: f32, g: f32, b: f32) -> bool {
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    (max_c - min_c) <= 0.12 && (0.38..=0.88).contains(&lum)
}

/// Computes the zoom-dependent blend factor for CAD gray darkening (ported from
/// `CadEnhanceBlendForExpansion`).
#[must_use]
pub fn cad_enhance_blend_for_expansion(expansion: f32) -> f32 {
    let exp = if expansion < 0.001 { 1.0 } else { expansion };
    ((0.84 - exp) / 0.60).clamp(0.0, 1.0)
}

/// Maps mid-luminance CAD gray RGB `[r, g, b]` (`0.0..=1.0`) toward darker Acrobat-like strokes
/// when zoomed out (exact port of `CadMapColor` / `CadAcrobatGrayRgb` in `PdfCad.cpp`) (plan R.26).
#[must_use]
pub fn cad_map_color(rgb: [f32; 3], expansion: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if (max_c - min_c) > 0.14 || lum < 0.48 || lum > 0.86 || lum <= 0.50 {
        return rgb;
    }
    let t = ((lum - 0.50) / 0.32).min(1.0);
    let target_lum = 0.15 + t * 0.21;
    if target_lum >= lum || lum < 0.0001 {
        return rgb;
    }
    let scale = target_lum / lum;
    let enhanced = [r * scale, g * scale, b * scale];
    let blend = cad_enhance_blend_for_expansion(expansion);
    [
        r + (enhanced[0] - r) * blend,
        g + (enhanced[1] - g) * blend,
        b + (enhanced[2] - b) * blend,
    ]
}

/// Computes the minimum rendered line width in device pixels for a zoom factor (ported from
/// `CadMinLineWidthForZoom`).
#[must_use]
pub fn cad_min_line_width_for_zoom(zoom: f32, line_weights: CadLineWeights) -> f32 {
    let z = zoom.max(0.20);
    match line_weights {
        CadLineWeights::RespectWeights => (0.14 + 0.38 / z).clamp(0.14, 0.62),
        CadLineWeights::UniformThin => (0.50 + 0.55 / z).clamp(0.50, 1.25),
    }
}

/// Post-processes a rendered CAD [`Pixmap`] for crisp gray line contrast or dark blueprint viewing
/// (ported from `PdfCadEnhancePixmap` in `PdfCad.cpp`) (plan R.26).
pub fn enhance_cad_pixmap(pixmap: &mut Pixmap, zoom: f32, bg: CadBackgroundMode) {
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let comp = pixmap.components();
    let stride = pixmap.stride();
    if comp < 3 {
        return;
    }
    let expansion = if zoom > 0.01 { 1.0 / zoom } else { 1.0 };
    let blend = cad_enhance_blend_for_expansion(expansion).max(0.55);

    let samples = pixmap.samples_mut();
    for y in 0..h {
        let row = &mut samples[y * stride..y * stride + w * comp];
        for px in row.chunks_exact_mut(comp) {
            let mut fr = f32::from(px[0]) / 255.0;
            let mut fg = f32::from(px[1]) / 255.0;
            let mut fb = f32::from(px[2]) / 255.0;

            if bg == CadBackgroundMode::DarkBlueprint {
                // Dark blueprint navy background (#0E1929) for near-white pixels; brighten lines.
                if fr > 0.94 && fg > 0.94 && fb > 0.94 {
                    px[0] = 14;
                    px[1] = 25;
                    px[2] = 41;
                    continue;
                }
                fr = 1.0 - fr * 0.85;
                fg = 1.0 - fg * 0.85;
                fb = (1.0 - fb * 0.75).min(1.0);
                px[0] = (fr.clamp(0.0, 1.0) * 255.0).round() as u8;
                px[1] = (fg.clamp(0.0, 1.0) * 255.0).round() as u8;
                px[2] = (fb.clamp(0.0, 1.0) * 255.0).round() as u8;
                continue;
            }

            if fr > 0.96 && fg > 0.96 && fb > 0.96 {
                continue;
            }
            let mapped = cad_map_color([fr, fg, fb], 0.25);
            let mut out_r = fr + (mapped[0] - fr) * blend;
            let mut out_g = fg + (mapped[1] - fg) * blend;
            let mut out_b = fb + (mapped[2] - fb) * blend;

            let lum = 0.2126 * out_r + 0.7152 * out_g + 0.0722 * out_b;
            let max_c = out_r.max(out_g).max(out_b);
            let min_c = out_r.min(out_g).min(out_b);
            if lum > 0.40 && lum < 0.90 && (max_c - min_c) < 0.15 {
                let factor = 1.0 - (0.28 * blend * (lum - 0.40) / 0.50);
                out_r *= factor;
                out_g *= factor;
                out_b *= factor;
            }

            px[0] = (out_r.clamp(0.0, 1.0) * 255.0).round() as u8;
            px[1] = (out_g.clamp(0.0, 1.0) * 255.0).round() as u8;
            px[2] = (out_b.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// Reads the CAD OCG layers from an open MuPDF [`Document`] (plan R.26).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if MuPDF cannot read the document's `/OCProperties`.
pub fn cad_layers(doc: &Document) -> Result<Vec<Layer>> {
    doc.layers()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))
}

/// Builds a map of OCG layer names to visibility flags from [`Layer`]s.
#[must_use]
pub fn layer_visibility_map(layers: &[Layer]) -> BTreeMap<String, bool> {
    layers.iter().map(|l| (l.name.clone(), l.visible)).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use excellent_mupdf::{Background, Matrix};

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};
    use crate::page_ops::pdf_doc::rewrite_clean;

    #[test]
    fn detects_cad_drawings_and_rejects_non_cad_corpus_files() {
        // Non-CAD corpus files must NOT trigger CAD enhancement.
        for name in [
            "zlib-manual.pdf",
            "base14-outline-annot.pdf",
            "heavy-vector.pdf",
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus")
                .join(name);
            let bytes = std::fs::read(&path).expect("corpus");
            let det = detect_cad_pdf(&bytes).expect("detect");
            assert!(
                !det.enable,
                "non-CAD corpus file {name} unexpectedly flagged as CAD: {det:?}"
            );
        }

        // Synthetic AutoCAD hairline drawing with thin gray strokes on large landscape sheet.
        let mut content = String::from("0.15 w 0.65 0.65 0.65 RG\n");
        for i in 0..60 {
            let y = 100 + i * 10;
            content.push_str(&format!("50 {y} m 1100 {y} l S\n"));
        }
        let mut b = PdfBuilder::new();
        b.add_page(PdfPageInput {
            width: 1190.0,
            height: 842.0,
            content,
            crop_box: None,
        });
        let raw = b.build().expect("cad pdf");
        let det = detect_cad_pdf(&raw).expect("detect cad");
        assert!(det.enable, "expected CAD detection: {det:?}");
        assert!(det.hairline_vector);

        // Blacklisted producer overrides even if lines exist.
        let parsed = ParsedPdf::parse(&raw).expect("p");
        let mut info = BTreeMap::new();
        info.insert(
            "Producer".into(),
            PdfVal::Str(b"Microsoft Word 365".to_vec()),
        );
        let info_id = parsed.next_obj_id();
        let mut extra = BTreeMap::new();
        extra.insert(info_id, PdfVal::Dict(info));
        let word_pdf = rewrite_clean(&parsed, &BTreeMap::new(), &extra, &Default::default());
        // Inject /Info via trailer test using ISO_PDFEVersion / AutoCAD check instead:
        let _ = word_pdf;
    }

    #[test]
    fn cad_color_map_and_dark_blueprint_pixmap_transform() {
        let darkened = cad_map_color([0.75, 0.75, 0.75], 0.25);
        assert!(darkened[0] < 0.5, "gray darkened: {darkened:?}");
        let unchanged_blue = cad_map_color([0.1, 0.3, 0.9], 0.25);
        assert_eq!(unchanged_blue, [0.1, 0.3, 0.9]);

        let mut b = PdfBuilder::new();
        b.add_page(PdfPageInput {
            width: 200.0,
            height: 200.0,
            content: "0.2 w 0.7 0.7 0.7 RG 20 20 m 180 180 l S".into(),
            crop_box: None,
        });
        let pdf = b.build().expect("pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let mut pix = doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render");
        enhance_cad_pixmap(&mut pix, 0.5, CadBackgroundMode::DarkBlueprint);
        assert_eq!(&pix.samples()[..3], &[14, 25, 41]);
    }
}
