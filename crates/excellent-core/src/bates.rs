//! Bates numbering with prefix, suffix, zero-padding, page ranges, and sequential batch
//! continuation across multiple PDFs (plan O.17).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::headers::{
    BATES_OCG, OverlayInjection, WatermarkLayer, inject_ocg_overlays, resolve_scope,
};
use crate::page_ops::PageApplyScope;
use crate::page_ops::pdf_doc::{ParsedPdf, PdfDocError, escape_pdf_string, fmt_f32};

/// Result alias for Bates numbering operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Corner or center position where a Bates stamp is placed (plan O.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BatesPosition {
    /// Top-left margin.
    TopLeft,
    /// Top-center margin.
    TopCenter,
    /// Top-right margin.
    TopRight,
    /// Bottom-left margin.
    BottomLeft,
    /// Bottom-center margin.
    BottomCenter,
    /// Bottom-right margin.
    #[default]
    BottomRight,
}

/// Configuration for Bates numbering (plan O.17).
#[derive(Debug, Clone, PartialEq)]
pub struct BatesConfig {
    /// Optional prefix (e.g. `"EXHIBIT-"`).
    pub prefix: String,
    /// Optional suffix (e.g. `"-CONF"`).
    pub suffix: String,
    /// Zero-padded digit width (`1..=12`, default `6`).
    pub digits: usize,
    /// Starting Bates sequence number.
    pub start_number: u64,
    /// Placement on the page.
    pub position: BatesPosition,
    /// Font size in points.
    pub font_size_pt: f32,
    /// RGB text color (`0.0..=1.0`).
    pub color_rgb: [f32; 3],
    /// Margin from page edges in points.
    pub margin_pt: f32,
    /// Target page range within each document.
    pub scope: PageApplyScope,
}

impl Default for BatesConfig {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            suffix: String::new(),
            digits: 6,
            start_number: 1,
            position: BatesPosition::BottomRight,
            font_size_pt: 10.0,
            color_rgb: [0.0, 0.0, 0.0],
            margin_pt: 24.0,
            scope: PageApplyScope::All,
        }
    }
}

/// Summary of Bates numbering applied to one PDF (plan O.17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatesStampOutput {
    /// Stamped PDF bytes.
    pub bytes: Vec<u8>,
    /// Number of pages stamped.
    pub pages_stamped: usize,
    /// First formatted Bates label applied (if any pages stamped).
    pub first_label: Option<String>,
    /// Last formatted Bates label applied (if any pages stamped).
    pub last_label: Option<String>,
    /// Next sequential counter value for subsequent files.
    pub next_number: u64,
}

/// Formats one Bates number according to `config` and `counter` (plan O.17).
#[must_use]
pub fn format_bates_number(config: &BatesConfig, counter: u64) -> String {
    let width = config.digits.clamp(1, 12);
    format!("{}{:0width$}{}", config.prefix, counter, config.suffix)
}

/// Applies Bates numbers to a single PDF starting at `config.start_number` (plan O.17).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be parsed or `config.scope` is out of range.
pub fn apply_bates_to_pdf(pdf_bytes: &[u8], config: &BatesConfig) -> Result<BatesStampOutput> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let target_indices = resolve_scope(&config.scope, pages.len())?;

    let font_size = config.font_size_pt.max(4.0);
    let [r, g, b] = config.color_rgb.map(|c| c.clamp(0.0, 1.0));
    let margin = config.margin_pt.max(4.0);

    let mut per_page_ops = BTreeMap::new();
    let mut counter = config.start_number;
    let mut first_label = None;
    let mut last_label = None;

    for idx in target_indices {
        let pid = pages[idx];
        let media = doc
            .effective_box(pid, "MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let label = format_bates_number(config, counter);
        if first_label.is_none() {
            first_label = Some(label.clone());
        }
        last_label = Some(label.clone());
        counter = counter.saturating_add(1);

        let est_w = label.chars().count() as f32 * font_size * 0.55;
        let w = media[2] - media[0];
        let (x, y) = match config.position {
            BatesPosition::TopLeft => (media[0] + margin, media[3] - margin - font_size),
            BatesPosition::TopCenter => (
                media[0] + ((w - est_w) * 0.5).max(margin),
                media[3] - margin - font_size,
            ),
            BatesPosition::TopRight => (
                (media[2] - margin - est_w).max(media[0] + margin),
                media[3] - margin - font_size,
            ),
            BatesPosition::BottomLeft => (media[0] + margin, media[1] + margin),
            BatesPosition::BottomCenter => (
                media[0] + ((w - est_w) * 0.5).max(margin),
                media[1] + margin,
            ),
            BatesPosition::BottomRight => (
                (media[2] - margin - est_w).max(media[0] + margin),
                media[1] + margin,
            ),
        };

        let mut ops = String::new();
        let _ = writeln!(
            ops,
            "q\n/ExGsBt gs\n{} {} {} rg\nBT /ExF1 {} Tf 1 0 0 1 {} {} Tm ({}) Tj ET\nQ",
            fmt_f32(r),
            fmt_f32(g),
            fmt_f32(b),
            fmt_f32(font_size),
            fmt_f32(x),
            fmt_f32(y),
            escape_pdf_string(&label)
        );
        per_page_ops.insert(idx, ops);
    }

    let pages_stamped = per_page_ops.len();
    let bytes = inject_ocg_overlays(
        &doc,
        OverlayInjection {
            ocg_name: BATES_OCG,
            oc_res_key: "ExOcBt",
            gs_res_key: "ExGsBt",
            alpha: 1.0,
            layer: WatermarkLayer::Foreground,
            per_page_ops: &per_page_ops,
            extra_xobj: None,
        },
    )?;

    Ok(BatesStampOutput {
        bytes,
        pages_stamped,
        first_label,
        last_label,
        next_number: counter,
    })
}

/// Applies continuous Bates numbering across an ordered slice of PDFs (plan O.17).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any file cannot be parsed or stamped.
pub fn apply_bates_batch(files: &[&[u8]], config: &BatesConfig) -> Result<Vec<BatesStampOutput>> {
    let mut current_cfg = config.clone();
    let mut results = Vec::with_capacity(files.len());
    for pdf in files {
        let out = apply_bates_to_pdf(pdf, &current_cfg)?;
        current_cfg.start_number = out.next_number;
        results.push(out);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::Document;

    use super::*;
    use crate::notebook::pdf_ops::build_blank_pdf;

    #[test]
    fn bates_numbering_continues_sequentially_across_twenty_files() {
        let one_page = build_blank_pdf(&[(612.0, 792.0)]).expect("blank");
        let two_page = build_blank_pdf(&[(612.0, 792.0), (612.0, 792.0)]).expect("blank2");
        let mut files: Vec<&[u8]> = Vec::new();
        for i in 0..20 {
            files.push(if i % 2 == 0 { &one_page } else { &two_page });
        }
        let cfg = BatesConfig {
            prefix: "CASE-".into(),
            suffix: "-PROD".into(),
            digits: 5,
            start_number: 100,
            ..BatesConfig::default()
        };
        let outs = apply_bates_batch(&files, &cfg).expect("batch bates");
        assert_eq!(outs.len(), 20);
        // 10 * 1 + 10 * 2 = 30 pages stamped => 100..=129, next = 130
        assert_eq!(outs[0].first_label.as_deref(), Some("CASE-00100-PROD"));
        assert_eq!(outs[19].last_label.as_deref(), Some("CASE-00129-PROD"));
        assert_eq!(outs[19].next_number, 130);

        let last_doc = Document::from_bytes(&outs[19].bytes, "application/pdf").expect("mupdf");
        let text = last_doc
            .load_page(1)
            .expect("p1")
            .text(None)
            .expect("s")
            .text()
            .expect("t");
        assert!(text.contains("CASE-00129-PROD"), "got {text}");
    }
}
