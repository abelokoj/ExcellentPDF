//! Annotation summary reports in Markdown, HTML, CSV, and PDF with deep links and optional page crops
//! (plan A.26).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ecclesia_core::annot::{Annotation, Shape};
use ecclesia_mupdf::{Background, Document, Matrix, PagePoint, PageRect, html_to_pdf};

use super::categories::Category;
use super::list::{self, Filter, Kind};
use crate::notes::export::DeepLink;

/// Supported annotation report output formats (plan A.26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReportFormat {
    /// GitHub-flavoured Markdown with `ecclesiapdf://open?...` deep links back to each page.
    Markdown,
    /// Self-contained HTML5 report with embedded crop images and deep links.
    Html,
    /// RFC 4180 CSV with spreadsheet formula-injection neutralization.
    Csv,
    /// Compiled multi-section PDF summary via MuPDF's HTML-to-PDF engine.
    Pdf,
}

/// Configuration for generating an annotation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportConfig {
    /// Document title shown in the report header.
    pub title: String,
    /// Source file path used to build `ecclesiapdf://open?...` deep links.
    pub file_path: PathBuf,
    /// Optional filter restricting which annotations are included (`None` includes all).
    pub filter: Option<Filter>,
    /// Whether to include rendered page crops around each annotation when available.
    pub include_crops: bool,
}

/// One annotation entry formatted for inclusion in a report.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportItem {
    /// 1-based page number.
    pub page: usize,
    /// Annotation `/NM` id.
    pub annot_id: String,
    /// Annotation kind label.
    pub kind: Kind,
    /// Semantic category, if assigned.
    pub category: Option<Category>,
    /// Highlighted or anchored quote text.
    pub quote: String,
    /// Reader's comment or note text.
    pub note: String,
    /// Author name.
    pub author: String,
    /// Tags attached to the annotation.
    pub tags: Vec<String>,
    /// RGB color `0xRRGGBB`, if any.
    pub color: Option<u32>,
    /// Modification timestamp (UTC seconds since 1970).
    pub modified: i64,
    /// `ecclesiapdf://open?file=...&page=...&annot=...` deep link back to this annotation.
    pub deep_link: String,
    /// Optional PNG-encoded page crop around the annotation.
    pub crop_png: Option<Vec<u8>>,
}

/// Builds [`ReportItem`]s from `annotations`, applying `config.filter` if provided.
#[must_use]
pub fn build_report_items(annotations: &[Annotation], config: &ReportConfig) -> Vec<ReportItem> {
    let rows = list::rows(annotations);
    let indices: Vec<usize> = match &config.filter {
        Some(filter) => filter.apply(&rows),
        None => (0..rows.len()).collect(),
    };
    let by_id: std::collections::HashMap<&str, &Annotation> =
        annotations.iter().map(|a| (a.id.as_str(), a)).collect();

    indices
        .into_iter()
        .filter_map(|idx| {
            let row = rows.get(idx)?;
            let full = by_id.get(row.target.id.as_str()).copied()?;
            let page_one_based = full.page + 1;
            let deep_link = DeepLink {
                file: config.file_path.clone(),
                page: Some(page_one_based),
                annot: Some(full.id.clone()),
            }
            .to_uri();
            let quote = full
                .anchor
                .as_ref()
                .map(|q| q.exact.trim().to_owned())
                .unwrap_or_default();
            Some(ReportItem {
                page: page_one_based,
                annot_id: full.id.clone(),
                kind: row.kind,
                category: row.category,
                quote,
                note: full.contents.trim().to_owned(),
                author: full.author.clone(),
                tags: full.tags.clone(),
                color: row.color,
                modified: full.modified,
                deep_link,
                crop_png: None,
            })
        })
        .collect()
}

/// Renders small PNG page crops around each [`ReportItem`] from `document` (when `config.include_crops`
/// is `true`). Non-fatal crop errors leave `crop_png` as `None`.
pub fn attach_page_crops(
    document: &Document,
    annotations: &[Annotation],
    items: &mut [ReportItem],
    max_width_px: u32,
) {
    let by_id: std::collections::HashMap<&str, &Annotation> =
        annotations.iter().map(|a| (a.id.as_str(), a)).collect();
    for item in items {
        if let Some(&annot) = by_id.get(item.annot_id.as_str())
            && let Ok(png) = render_annotation_crop(document, annot, max_width_px)
        {
            item.crop_png = Some(png);
        }
    }
}

/// Renders a PNG crop of the page region surrounding `annotation`, padded by 16 points.
///
/// # Errors
///
/// Returns [`ecclesia_mupdf::MuError`] if the page cannot be loaded or rendered.
pub fn render_annotation_crop(
    document: &Document,
    annotation: &Annotation,
    max_width_px: u32,
) -> ecclesia_mupdf::Result<Vec<u8>> {
    let page = document.load_page(annotation.page)?;
    let bounds = page.bounds()?;
    let raw_rect = annotation_bounds(annotation).unwrap_or(bounds);
    let pad = 16.0_f32;
    let crop = PageRect {
        x0: (raw_rect.x0 - pad).max(bounds.x0),
        y0: (raw_rect.y0 - pad).max(bounds.y0),
        x1: (raw_rect.x1 + pad).min(bounds.x1),
        y1: (raw_rect.y1 + pad).min(bounds.y1),
    };
    let width_pts = crop.width().max(24.0);
    let target_px = max_width_px.max(64) as f32;
    let scale = (target_px / width_pts).clamp(0.5, 2.5);
    let ctm = Matrix::scale(scale, scale);
    let area = ctm.apply_rect(&crop);
    let list = page.display_list(None)?;
    let pixmap = list.render_area(&ctm, area, Background::White, None)?;
    pixmap.to_png()
}

/// Exports `items` as a Markdown summary report with `ecclesiapdf://open?...` deep links back to each page (plan A.26).
#[must_use]
pub fn export_markdown(config: &ReportConfig, items: &[ReportItem]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {} — Annotation Summary\n", config.title);
    let doc_uri = DeepLink {
        file: config.file_path.clone(),
        page: None,
        annot: None,
    }
    .to_uri();
    let _ = writeln!(
        out,
        "- **Source:** [{}]({doc_uri})",
        file_label(&config.file_path)
    );
    let _ = writeln!(out, "- **Annotations:** {}\n", items.len());

    let mut current_page: Option<usize> = None;
    for item in items {
        if current_page != Some(item.page) {
            current_page = Some(item.page);
            let page_uri = DeepLink {
                file: config.file_path.clone(),
                page: Some(item.page),
                annot: None,
            }
            .to_uri();
            let _ = writeln!(out, "## [Page {}]({page_uri})\n", item.page);
        }
        let cat = item
            .category
            .map(|c| format!(" · **{}**", c.key()))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "- [Page {} ({})]({}){cat} — *{}*",
            item.page,
            item.kind.name(),
            item.deep_link,
            if item.author.is_empty() {
                "Unknown"
            } else {
                item.author.as_str()
            }
        );
        if !item.quote.is_empty() {
            for line in item.quote.lines() {
                let _ = writeln!(out, "  > {line}");
            }
        }
        if !item.note.is_empty() {
            let _ = writeln!(out, "  - Note: {}", item.note);
        }
        if !item.tags.is_empty() {
            let _ = writeln!(out, "  - Tags: {}", item.tags.join(", "));
        }
        if item.crop_png.is_some() {
            let _ = writeln!(out, "  - *(Page crop attached)*");
        }
    }
    out
}

/// Exports `items` as a self-contained HTML5 report with deep links and embedded crop PNGs (plan A.26).
#[must_use]
pub fn export_html(config: &ReportConfig, items: &[ReportItem]) -> String {
    let mut out =
        String::from("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(
        out,
        "<title>{} — Annotation Report</title>",
        escape_html(&config.title)
    );
    out.push_str(
        "<style>
body { font-family: system-ui, -apple-system, sans-serif; max-width: 860px; margin: 2rem auto; padding: 0 1rem; color: #1b1f23; line-height: 1.5; }
h1 { border-bottom: 1px solid #d0d7de; padding-bottom: 0.4rem; }
.card { border: 1px solid #d0d7de; border-radius: 8px; padding: 0.85rem 1rem; margin: 0.9rem 0; background: #fff; }
.meta { font-size: 0.85rem; color: #57606a; margin-bottom: 0.4rem; }
.badge { display: inline-block; padding: 0.1rem 0.45rem; border-radius: 999px; background: #eaeef2; font-size: 0.78rem; margin-left: 0.4rem; }
blockquote { margin: 0.4rem 0; padding: 0.35rem 0.75rem; border-left: 3px solid #0969da; background: #f6f8fa; }
.crop { margin-top: 0.5rem; max-width: 100%; border: 1px solid #d0d7de; border-radius: 4px; }
</style>\n</head>\n<body>\n",
    );
    let _ = writeln!(
        out,
        "<h1>{} — Annotation Summary</h1>",
        escape_html(&config.title)
    );
    let _ = writeln!(
        out,
        "<p class=\"meta\">Total annotations: {}</p>",
        items.len()
    );

    for item in items {
        out.push_str("<div class=\"card\">\n");
        let cat_badge = item
            .category
            .map(|c| format!("<span class=\"badge\">{}</span>", escape_html(c.key())))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "<div class=\"meta\"><a href=\"{}\">Page {}</a> · {} · {}{cat_badge}</div>",
            escape_html(&item.deep_link),
            item.page,
            escape_html(item.kind.name()),
            escape_html(if item.author.is_empty() {
                "Unknown"
            } else {
                &item.author
            }),
        );
        if !item.quote.is_empty() {
            let _ = writeln!(out, "<blockquote>{}</blockquote>", escape_html(&item.quote));
        }
        if !item.note.is_empty() {
            let _ = writeln!(out, "<p>{}</p>", escape_html(&item.note));
        }
        if !item.tags.is_empty() {
            let _ = writeln!(
                out,
                "<div class=\"meta\">Tags: {}</div>",
                escape_html(&item.tags.join(", "))
            );
        }
        if let Some(png) = &item.crop_png {
            let b64 = BASE64.encode(png);
            let _ = writeln!(
                out,
                "<img class=\"crop\" alt=\"Page {} crop\" src=\"data:image/png;base64,{b64}\">",
                item.page
            );
        }
        out.push_str("</div>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Exports `items` as RFC 4180 CSV with formula-injection protection (plan A.26).
#[must_use]
pub fn export_csv(items: &[ReportItem]) -> String {
    let mut out = String::from("page,kind,category,author,quote,note,tags,color,deep_link\r\n");
    for item in items {
        let cat = item.category.map(Category::key).unwrap_or("");
        let color = item.color.map(|c| format!("#{c:06x}")).unwrap_or_default();
        let tags = item.tags.join("; ");
        let fields = [
            item.page.to_string(),
            item.kind.name().to_owned(),
            cat.to_owned(),
            csv_safe(&item.author),
            csv_safe(&item.quote),
            csv_safe(&item.note),
            csv_safe(&tags),
            color,
            csv_safe(&item.deep_link),
        ];
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_csv_cell(&mut out, f);
        }
        out.push_str("\r\n");
    }
    out
}

/// Exports `items` as a PDF summary document using MuPDF's HTML-to-PDF layout engine (plan A.26).
///
/// # Errors
///
/// Returns [`ecclesia_mupdf::MuError`] if MuPDF cannot render the HTML summary to PDF.
pub fn export_pdf_summary(
    config: &ReportConfig,
    items: &[ReportItem],
) -> ecclesia_mupdf::Result<Vec<u8>> {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "<h1>{} — Annotation Summary</h1>",
        escape_html(&config.title)
    );
    let _ = writeln!(body, "<p>Annotations: {}</p>", items.len());
    for item in items {
        let cat = item
            .category
            .map(|c| format!(" [{}]", escape_html(c.key())))
            .unwrap_or_default();
        let _ = writeln!(
            body,
            "<div class=\"item\"><strong>Page {} ({}){}</strong> — {}</div>",
            item.page,
            escape_html(item.kind.name()),
            cat,
            escape_html(if item.author.is_empty() {
                "Unknown"
            } else {
                &item.author
            }),
        );
        if !item.quote.is_empty() {
            let _ = writeln!(
                body,
                "<blockquote>{}</blockquote>",
                escape_html(&item.quote)
            );
        }
        if !item.note.is_empty() {
            let _ = writeln!(body, "<p>{}</p>", escape_html(&item.note));
        }
    }
    let css = "
body { font-family: sans-serif; font-size: 10pt; color: #111; }
h1 { font-size: 14pt; border-bottom: 1pt solid #ccc; padding-bottom: 4pt; }
.item { margin-top: 8pt; font-size: 9.5pt; color: #333; }
blockquote { margin: 3pt 0 3pt 8pt; padding-left: 6pt; border-left: 2pt solid #0969da; }
p { margin: 2pt 0 6pt 0; }
";
    html_to_pdf(&body, css, 540.0)
}

fn annotation_bounds(annotation: &Annotation) -> Option<PageRect> {
    match &annotation.shape {
        Shape::Markup(_, quads) => {
            if quads.is_empty() {
                return None;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for q in quads {
                let b = q.bounds();
                x0 = x0.min(b.x0);
                y0 = y0.min(b.y0);
                x1 = x1.max(b.x1);
                y1 = y1.max(b.y1);
            }
            Some(PageRect { x0, y0, x1, y1 })
        }
        Shape::Boxed(_, r) | Shape::Callout(r, _) => Some(*r),
        Shape::Line(a, b) => Some(bounds_of_points(&[*a, *b])),
        Shape::Path(_, pts) => (!pts.is_empty()).then(|| bounds_of_points(pts)),
        Shape::Ink(strokes) => {
            let pts: Vec<PagePoint> = strokes.iter().flatten().copied().collect();
            (!pts.is_empty()).then(|| bounds_of_points(&pts))
        }
    }
}

fn bounds_of_points(pts: &[PagePoint]) -> PageRect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    PageRect { x0, y0, x1, y1 }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Document")
        .to_owned()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn csv_safe(s: &str) -> String {
    if matches!(
        s.as_bytes().first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')
    ) {
        format!("'{s}")
    } else {
        s.to_owned()
    }
}

fn write_csv_cell(out: &mut String, cell: &str) {
    if cell.contains([',', '"', '\n', '\r']) {
        out.push('"');
        for ch in cell.chars() {
            if ch == '"' {
                out.push_str("\"\"");
            } else {
                out.push(ch);
            }
        }
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::anchor::Quote;
    use ecclesia_core::annot::{Boxed, Markup, Style};

    use super::*;

    #[test]
    fn generates_markdown_html_csv_and_pdf_reports_with_deep_links() {
        let mut h = Annotation::new(
            2,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        h.anchor = Quote::of("Bayesian inference", 0..18);
        h.contents = "Check prior assumption".to_owned();
        h.category = Some("claim".to_owned());
        h.tags = vec!["stats".to_owned()];

        let mut n = Annotation::new(
            4,
            Shape::Boxed(Boxed::Note, PageRect::default()),
            Style::default(),
            "Grace",
        );
        n.contents = "=SUM(A1:A2) formula check".to_owned();

        let cfg = ReportConfig {
            title: "Thesis Paper".to_owned(),
            file_path: PathBuf::from("C:/papers/thesis.pdf"),
            filter: None,
            include_crops: true,
        };
        let mut items = build_report_items(&[h.clone(), n], &cfg);
        assert_eq!(items.len(), 2);

        // Attach a real MuPDF page crop from a corpus PDF.
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let doc = Document::open(&corpus).expect("open");
        let crop_annot = Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 120.0,
                    y1: 80.0,
                },
            ),
            Style::default(),
            "Ada",
        );
        let png = render_annotation_crop(&doc, &crop_annot, 180).expect("crop png");
        assert!(png.starts_with(b"\x89PNG"));
        items[0].crop_png = Some(png);

        let md = export_markdown(&cfg, &items);
        assert!(md.contains("ecclesiapdf://open?file="));
        assert!(md.contains(&format!("&page=3&annot={}", h.id)));
        assert!(md.contains("Bayesian inference"));

        let html = export_html(&cfg, &items);
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("ecclesiapdf://open?file="));

        let csv = export_csv(&items);
        assert!(csv.contains("'=SUM(A1:A2)"));

        let pdf = export_pdf_summary(&cfg, &items).expect("pdf summary");
        let parsed = Document::from_bytes(&pdf, "application/pdf").expect("valid pdf");
        assert!(parsed.page_count().expect("pages") >= 1);
    }
}
