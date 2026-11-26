//! PDF comparison: word-level text diff, pixel-diff visual overlay, annotation diff by `/NM`,
//! and HTML/PDF report export (plan F.15).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use excellent_core::annot::Annotation;
use excellent_mupdf::{Background, DeviceRect, Document, Matrix, Pixmap, html_to_pdf};

/// Error returned when comparing PDFs or building comparison reports.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum CompareError {
    /// MuPDF failed to open, render, or extract text.
    Document(String),
    /// Report generation failed.
    Report(String),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Document(msg) => write!(f, "compare document error: {msg}"),
            Self::Report(msg) => write!(f, "compare report error: {msg}"),
        }
    }
}

impl std::error::Error for CompareError {}

/// Convenience result alias for PDF compare.
pub type Result<T> = std::result::Result<T, CompareError>;

/// One word-level diff operation (plan F.15).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WordDiffOp {
    /// Unchanged token.
    Equal(String),
    /// Token present only in baseline (`old`).
    Delete(String),
    /// Token present only in revised (`new`).
    Insert(String),
}

/// Word-level comparison summary for one page pair (plan F.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTextDiff {
    /// Zero-based page index.
    pub page: usize,
    /// Ordered diff operations across tokens.
    pub ops: Vec<WordDiffOp>,
    /// Count of deleted tokens.
    pub deletions: usize,
    /// Count of inserted tokens.
    pub insertions: usize,
}

impl PageTextDiff {
    /// True when baseline and revised page text had zero differences.
    #[must_use]
    pub fn is_identical(&self) -> bool {
        self.deletions == 0 && self.insertions == 0
    }
}

/// Result of comparing two rendered page pixmaps pixel-by-pixel (plan F.15).
#[derive(Debug, Clone, PartialEq)]
pub struct VisualDiffResult {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Number of pixels whose RGB max-channel delta exceeded the threshold.
    pub changed_pixels: usize,
    /// Total pixels compared.
    pub total_pixels: usize,
    /// Changed pixel fraction (`0.0..=1.0`).
    pub diff_ratio: f32,
    /// Coarse bounding rectangles covering changed regions.
    pub changed_regions: Vec<DeviceRect>,
    /// RGBA overlay buffer highlighting changed pixels in translucent coral red.
    pub overlay_rgba: Vec<u8>,
}

/// Difference category for annotations matched by `/NM` ID (plan F.15).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnnotDiffKind {
    /// Annotation exists only in revised document.
    Added,
    /// Annotation exists only in baseline document.
    Removed,
    /// Annotation exists in both with different text, color, or geometry.
    Modified,
}

/// One annotation difference entry (plan F.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotDiffEntry {
    /// Annotation `/NM` ID.
    pub id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Difference kind.
    pub kind: AnnotDiffKind,
    /// Short summary of what changed.
    pub summary: String,
}

/// Full multi-page PDF comparison result (plan F.15).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentCompareReport {
    /// Baseline page count.
    pub left_pages: usize,
    /// Revised page count.
    pub right_pages: usize,
    /// Per-page text diffs (only pages with differences or all pages).
    pub text_diffs: Vec<PageTextDiff>,
    /// Visual diffs for pages checked.
    pub visual_diffs: Vec<(usize, VisualDiffResult)>,
    /// Annotation differences matched by `/NM`.
    pub annot_diffs: Vec<AnnotDiffEntry>,
}

/// Computes a fast word-level diff between `old_text` and `new_text` (plan F.15).
/// Uses prefix/suffix trimming plus Myers/DP LCS over token hashes so multi-hundred-page books
/// compare rapidly.
#[must_use]
pub fn diff_words(old_text: &str, new_text: &str) -> Vec<WordDiffOp> {
    let a: Vec<&str> = old_text.split_whitespace().collect();
    let b: Vec<&str> = new_text.split_whitespace().collect();

    let mut prefix = 0usize;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix + prefix < a.len()
        && suffix + prefix < b.len()
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut ops = Vec::with_capacity(a.len().max(b.len()));
    for &tok in &a[..prefix] {
        ops.push(WordDiffOp::Equal(tok.to_owned()));
    }

    let a_mid = &a[prefix..a.len() - suffix];
    let b_mid = &b[prefix..b.len() - suffix];
    if !a_mid.is_empty() || !b_mid.is_empty() {
        if a_mid.len() * b_mid.len() <= 250_000 {
            diff_lcs_mid(a_mid, b_mid, &mut ops);
        } else {
            for &tok in a_mid {
                ops.push(WordDiffOp::Delete(tok.to_owned()));
            }
            for &tok in b_mid {
                ops.push(WordDiffOp::Insert(tok.to_owned()));
            }
        }
    }

    for &tok in &a[a.len() - suffix..] {
        ops.push(WordDiffOp::Equal(tok.to_owned()));
    }
    ops
}

fn diff_lcs_mid(a: &[&str], b: &[&str], ops: &mut Vec<WordDiffOp>) {
    let n = a.len();
    let m = b.len();
    let stride = m + 1;
    let mut dp = vec![0u32; (n + 1) * stride];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * stride + j] = if a[i] == b[j] {
                dp[(i + 1) * stride + (j + 1)] + 1
            } else {
                dp[(i + 1) * stride + j].max(dp[i * stride + (j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(WordDiffOp::Equal(a[i].to_owned()));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)] {
            ops.push(WordDiffOp::Delete(a[i].to_owned()));
            i += 1;
        } else {
            ops.push(WordDiffOp::Insert(b[j].to_owned()));
            j += 1;
        }
    }
    while i < n {
        ops.push(WordDiffOp::Delete(a[i].to_owned()));
        i += 1;
    }
    while j < m {
        ops.push(WordDiffOp::Insert(b[j].to_owned()));
        j += 1;
    }
}

/// Compares text across two vectors of page strings (e.g. a 300-page book) and returns per-page
/// diffs (plan F.15).
#[must_use]
pub fn diff_page_texts(left_pages: &[String], right_pages: &[String]) -> Vec<PageTextDiff> {
    let count = left_pages.len().max(right_pages.len());
    let mut out = Vec::new();
    for page in 0..count {
        let l = left_pages.get(page).map_or("", String::as_str);
        let r = right_pages.get(page).map_or("", String::as_str);
        if l == r {
            continue;
        }
        let ops = diff_words(l, r);
        let deletions = ops
            .iter()
            .filter(|op| matches!(op, WordDiffOp::Delete(_)))
            .count();
        let insertions = ops
            .iter()
            .filter(|op| matches!(op, WordDiffOp::Insert(_)))
            .count();
        if deletions > 0 || insertions > 0 {
            out.push(PageTextDiff {
                page,
                ops,
                deletions,
                insertions,
            });
        }
    }
    out
}

/// Compares two rendered [`Pixmap`]s pixel-by-pixel and returns changed pixel counts, coarse
/// change rectangles, and a red-highlight RGBA overlay (plan F.15).
#[must_use]
pub fn diff_pixmaps(left: &Pixmap, right: &Pixmap, channel_threshold: u8) -> VisualDiffResult {
    let w = left.width().min(right.width());
    let h = left.height().min(right.height());
    let lw = w as usize;
    let lh = h as usize;
    let mut overlay = vec![0u8; lw * lh * 4];
    let mut changed_pixels = 0usize;
    let mut min_x = lw;
    let mut min_y = lh;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    let ls = left.samples();
    let rs = right.samples();
    let lcomp = left.components();
    let rcomp = right.components();
    let lstride = left.stride();
    let rstride = right.stride();

    if lcomp >= 3 && rcomp >= 3 {
        for y in 0..lh {
            let lrow = &ls[y * lstride..y * lstride + lw * lcomp];
            let rrow = &rs[y * rstride..y * rstride + lw * rcomp];
            for x in 0..lw {
                let li = x * lcomp;
                let ri = x * rcomp;
                let dr = lrow[li].abs_diff(rrow[ri]);
                let dg = lrow[li + 1].abs_diff(rrow[ri + 1]);
                let db = lrow[li + 2].abs_diff(rrow[ri + 2]);
                if dr.max(dg).max(db) > channel_threshold {
                    changed_pixels += 1;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + 1);
                    max_y = max_y.max(y + 1);
                    let oi = (y * lw + x) * 4;
                    overlay[oi] = 235;
                    overlay[oi + 1] = 64;
                    overlay[oi + 2] = 52;
                    overlay[oi + 3] = 160;
                }
            }
        }
    }

    let total_pixels = lw * lh;
    let changed_regions = if changed_pixels > 0 {
        vec![DeviceRect {
            x0: min_x as i32,
            y0: min_y as i32,
            x1: max_x as i32,
            y1: max_y as i32,
        }]
    } else {
        Vec::new()
    };

    VisualDiffResult {
        width: w,
        height: h,
        changed_pixels,
        total_pixels,
        diff_ratio: if total_pixels > 0 {
            changed_pixels as f32 / total_pixels as f32
        } else {
            0.0
        },
        changed_regions,
        overlay_rgba: overlay,
    }
}

/// Diffs two annotation lists by `/NM` id (plan F.15).
#[must_use]
pub fn diff_annotations(left: &[Annotation], right: &[Annotation]) -> Vec<AnnotDiffEntry> {
    let lmap: BTreeMap<&str, &Annotation> = left.iter().map(|a| (a.id.as_str(), a)).collect();
    let rmap: BTreeMap<&str, &Annotation> = right.iter().map(|a| (a.id.as_str(), a)).collect();
    let ids: BTreeSet<&str> = lmap.keys().chain(rmap.keys()).copied().collect();
    let mut diffs = Vec::new();
    for id in ids {
        match (lmap.get(id), rmap.get(id)) {
            (None, Some(r)) => diffs.push(AnnotDiffEntry {
                id: id.to_owned(),
                page: r.page,
                kind: AnnotDiffKind::Added,
                summary: format!("added on page {}", r.page + 1),
            }),
            (Some(l), None) => diffs.push(AnnotDiffEntry {
                id: id.to_owned(),
                page: l.page,
                kind: AnnotDiffKind::Removed,
                summary: format!("removed from page {}", l.page + 1),
            }),
            (Some(l), Some(r)) => {
                if l.contents != r.contents || l.shape != r.shape || l.style != r.style {
                    diffs.push(AnnotDiffEntry {
                        id: id.to_owned(),
                        page: r.page,
                        kind: AnnotDiffKind::Modified,
                        summary: format!("modified on page {}", r.page + 1),
                    });
                }
            }
            (None, None) => {}
        }
    }
    diffs
}

/// Runs text and optional visual comparison across two PDF byte buffers using MuPDF (plan F.15).
///
/// # Errors
///
/// Returns [`CompareError::Document`] if MuPDF cannot open or render either PDF.
pub fn compare_pdfs(
    left_pdf: &[u8],
    right_pdf: &[u8],
    visual_dpi: Option<f32>,
) -> Result<DocumentCompareReport> {
    let ldoc = Document::from_bytes(left_pdf, "application/pdf")
        .map_err(|e| CompareError::Document(e.to_string()))?;
    let rdoc = Document::from_bytes(right_pdf, "application/pdf")
        .map_err(|e| CompareError::Document(e.to_string()))?;
    let lc = ldoc
        .page_count()
        .map_err(|e| CompareError::Document(e.to_string()))?;
    let rc = rdoc
        .page_count()
        .map_err(|e| CompareError::Document(e.to_string()))?;

    let mut l_texts = Vec::with_capacity(lc);
    for i in 0..lc {
        let txt = ldoc
            .load_page(i)
            .and_then(|p| p.text(None)?.text())
            .map_err(|e| CompareError::Document(e.to_string()))?;
        l_texts.push(txt);
    }
    let mut r_texts = Vec::with_capacity(rc);
    for i in 0..rc {
        let txt = rdoc
            .load_page(i)
            .and_then(|p| p.text(None)?.text())
            .map_err(|e| CompareError::Document(e.to_string()))?;
        r_texts.push(txt);
    }

    let text_diffs = diff_page_texts(&l_texts, &r_texts);

    let mut visual_diffs = Vec::new();
    if let Some(dpi) = visual_dpi {
        let scale = (dpi / 72.0).max(0.25);
        let ctm = Matrix::scale(scale, scale);
        for i in 0..lc.min(rc) {
            let lpix = ldoc
                .load_page(i)
                .and_then(|p| p.render(&ctm, Background::White))
                .map_err(|e| CompareError::Document(e.to_string()))?;
            let rpix = rdoc
                .load_page(i)
                .and_then(|p| p.render(&ctm, Background::White))
                .map_err(|e| CompareError::Document(e.to_string()))?;
            let vd = diff_pixmaps(&lpix, &rpix, 12);
            if vd.changed_pixels > 0 {
                visual_diffs.push((i, vd));
            }
        }
    }

    Ok(DocumentCompareReport {
        left_pages: lc,
        right_pages: rc,
        text_diffs,
        visual_diffs,
        annot_diffs: Vec::new(),
    })
}

/// Renders a comparison report as a self-contained HTML summary (plan F.15).
#[must_use]
pub fn export_compare_html(report: &DocumentCompareReport) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
        body{font-family:sans-serif;max-width:56rem;margin:2rem auto;color:#1b1f24}\
        del{background:#ffd7d5;color:#82071e;text-decoration:line-through}\
        ins{background:#dafbe1;color:#116329;text-decoration:none}\
        section{border-top:1px solid #d0d7de;padding:1rem 0}\
        </style></head><body><h1>PDF Comparison Report</h1>",
    );
    let _ = writeln!(
        html,
        "<p>Baseline pages: {} | Revised pages: {} | Changed text pages: {} | Changed visual pages: {}</p>",
        report.left_pages,
        report.right_pages,
        report.text_diffs.len(),
        report.visual_diffs.len()
    );
    for td in &report.text_diffs {
        let _ = writeln!(
            html,
            "<section><h2>Page {} (-{} / +{})</h2><p>",
            td.page + 1,
            td.deletions,
            td.insertions
        );
        for op in &td.ops {
            match op {
                WordDiffOp::Equal(w) => {
                    let _ = write!(html, "{} ", esc_html(w));
                }
                WordDiffOp::Delete(w) => {
                    let _ = write!(html, "<del>{}</del> ", esc_html(w));
                }
                WordDiffOp::Insert(w) => {
                    let _ = write!(html, "<ins>{}</ins> ", esc_html(w));
                }
            }
        }
        html.push_str("</p></section>");
    }
    html.push_str("</body></html>");
    html
}

/// Renders a comparison report as a PDF via MuPDF's HTML layout engine (plan F.15).
///
/// # Errors
///
/// Returns [`CompareError::Report`] if MuPDF cannot lay out the HTML into PDF.
pub fn export_compare_pdf(report: &DocumentCompareReport) -> Result<Vec<u8>> {
    html_to_pdf(&export_compare_html(report), "", 540.0)
        .map_err(|e| CompareError::Report(e.to_string()))
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn finds_one_word_change_in_300_page_book_and_renders_visual_and_report() {
        let mut left = Vec::with_capacity(300);
        let mut right = Vec::with_capacity(300);
        for i in 0..300 {
            let line =
                format!("Chapter {i} discusses steady thermodynamics and boundary conditions.");
            left.push(line.clone());
            if i == 177 {
                right.push(format!(
                    "Chapter {i} discusses transient thermodynamics and boundary conditions."
                ));
            } else {
                right.push(line);
            }
        }
        let diffs = diff_page_texts(&left, &right);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].page, 177);
        assert_eq!((diffs[0].deletions, diffs[0].insertions), (1, 1));
        assert!(diffs[0].ops.contains(&WordDiffOp::Delete("steady".into())));
        assert!(
            diffs[0]
                .ops
                .contains(&WordDiffOp::Insert("transient".into()))
        );

        let mut b1 = PdfBuilder::new();
        b1.enable_standard_fonts();
        b1.add_page(PdfPageInput {
            width: 300.0,
            height: 200.0,
            content: "BT /F1 14 Tf 30 140 Td (Hello World) Tj ET".into(),
            crop_box: None,
        });
        let mut b2 = PdfBuilder::new();
        b2.enable_standard_fonts();
        b2.add_page(PdfPageInput {
            width: 300.0,
            height: 200.0,
            content: "BT /F1 14 Tf 30 140 Td (Hello Rust) Tj ET".into(),
            crop_box: None,
        });
        let report = compare_pdfs(
            &b1.build().expect("p1"),
            &b2.build().expect("p2"),
            Some(72.0),
        )
        .expect("compare");
        assert_eq!(report.text_diffs.len(), 1);
        assert_eq!(report.visual_diffs.len(), 1);
        assert!(report.visual_diffs[0].1.changed_pixels > 0);
        let pdf_report = export_compare_pdf(&report).expect("pdf report");
        assert!(pdf_report.starts_with(b"%PDF"));
    }
}
