//! Table and figure structure extractor exporting CSV, Markdown, PNG, and SVG (plan F.16).

use std::fmt::{self, Write as _};

use excellent_mupdf::{Background, DisplayList, Matrix, PageRect, TextLayout, TextLine};

/// Error returned when extracting or rendering tables and figures.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ExtractError {
    /// MuPDF rendering or image encoding failed.
    Render(String),
    /// Target rectangle was degenerate.
    InvalidBounds(String),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(msg) => write!(f, "figure render error: {msg}"),
            Self::InvalidBounds(msg) => write!(f, "invalid figure bounds: {msg}"),
        }
    }
}

impl std::error::Error for ExtractError {}

/// Convenience result alias for extraction operations.
pub type Result<T> = std::result::Result<T, ExtractError>;

/// Extracted table with inferred column alignment and optional caption (plan F.16).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedTable {
    /// Zero-based page index.
    pub page: usize,
    /// Bounding box in page points.
    pub bounds: PageRect,
    /// Optional caption matched above or below the table.
    pub caption: Option<String>,
    /// Detected horizontal column left boundaries in page points.
    pub column_starts: Vec<f32>,
    /// Grid rows (`row[col]`).
    pub rows: Vec<Vec<String>>,
}

/// Kind of extracted figure region (plan F.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FigureKind {
    /// Raster image block.
    RasterImage,
    /// Vector graphic / diagram region.
    VectorDiagram,
}

/// Extracted figure bounding box and caption (plan F.16).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedFigure {
    /// Zero-based page index.
    pub page: usize,
    /// Bounding box in page points.
    pub bounds: PageRect,
    /// Optional figure caption.
    pub caption: Option<String>,
    /// Raster or vector figure classification.
    pub kind: FigureKind,
}

struct TableRowCandidate {
    y0: f32,
    bbox: PageRect,
    cells: Vec<(f32, String)>,
}

/// Extracts structured tables from a page's K.13 [`TextLayout`] using coordinate-aligned row &
/// column clustering as well as wide inter-cell whitespace gaps (plan F.16).
#[must_use]
pub fn extract_tables(page: usize, layout: &TextLayout) -> Vec<ExtractedTable> {
    let mut tables = Vec::new();
    let line_rows = group_lines_into_y_rows(&layout.lines);

    // Collect candidate multi-cell rows (either multiple lines in same Y-band or wide cell gaps).
    let mut row_entries: Vec<TableRowCandidate> = Vec::new();
    for y_group in line_rows {
        let mut cells: Vec<(f32, String)> = Vec::new();
        let mut row_bbox: Option<PageRect> = None;
        for &line_idx in &y_group {
            let line = &layout.lines[line_idx];
            row_bbox = Some(match row_bbox {
                None => line.bbox,
                Some(b) => union_rect(b, line.bbox),
            });
            cells.extend(split_line_cells(layout, line));
        }
        cells.sort_by(|a, b| a.0.total_cmp(&b.0));
        if cells.len() >= 2
            && let Some(bbox) = row_bbox
        {
            row_entries.push(TableRowCandidate {
                y0: bbox.y0,
                bbox,
                cells,
            });
        }
    }

    // Group consecutive multi-cell rows with close vertical spacing into table regions.
    let mut current: Vec<(PageRect, Vec<(f32, String)>)> = Vec::new();
    for entry in row_entries {
        let fits_prev = current.last().is_some_and(|(prev_box, _)| {
            (entry.y0 - prev_box.y1) <= prev_box.height().max(12.0) * 2.5
        });
        if !fits_prev && current.len() >= 2 {
            tables.push(finalize_table(page, layout, &current));
            current.clear();
        } else if !fits_prev {
            current.clear();
        }
        current.push((entry.bbox, entry.cells));
    }
    if current.len() >= 2 {
        tables.push(finalize_table(page, layout, &current));
    }

    tables
}

fn finalize_table(
    page: usize,
    layout: &TextLayout,
    raw_rows: &[(PageRect, Vec<(f32, String)>)],
) -> ExtractedTable {
    let mut bounds = raw_rows[0].0;
    for (b, _) in &raw_rows[1..] {
        bounds = union_rect(bounds, *b);
    }

    // Cluster cell x0 positions into column starts (tolerance 18 pt).
    let mut all_x: Vec<f32> = raw_rows
        .iter()
        .flat_map(|(_, cells)| cells.iter().map(|(x, _)| *x))
        .collect();
    all_x.sort_by(f32::total_cmp);
    let mut col_starts: Vec<f32> = Vec::new();
    for x in all_x {
        if col_starts.last().is_none_or(|&prev| (x - prev) > 18.0) {
            col_starts.push(x);
        }
    }
    let cols = col_starts.len().max(1);
    let mut grid = Vec::with_capacity(raw_rows.len());
    for (_, cells) in raw_rows {
        let mut row = vec![String::new(); cols];
        for (x, text) in cells {
            let col_idx = col_starts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (**a - *x).abs().total_cmp(&(**b - *x).abs()))
                .map_or(0, |(idx, _)| idx);
            if !row[col_idx].is_empty() {
                row[col_idx].push(' ');
            }
            row[col_idx].push_str(text);
        }
        grid.push(row);
    }

    let caption = find_nearby_caption(layout, bounds, &["table "]);
    ExtractedTable {
        page,
        bounds,
        caption,
        column_starts: col_starts,
        rows: grid,
    }
}

fn group_lines_into_y_rows(lines: &[TextLine]) -> Vec<Vec<usize>> {
    let mut indexed: Vec<(usize, f32, f32)> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (i, l.bbox.y0, l.bbox.x0))
        .collect();
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)));
    let mut groups: Vec<(f32, Vec<usize>)> = Vec::new();
    for (idx, y0, _) in indexed {
        let line_h = lines[idx].bbox.height().max(8.0);
        if let Some((last_y, list)) = groups.last_mut()
            && (y0 - *last_y).abs() <= line_h * 0.45
        {
            list.push(idx);
            continue;
        }
        groups.push((y0, vec![idx]));
    }
    groups.into_iter().map(|(_, list)| list).collect()
}

fn split_line_cells(layout: &TextLayout, line: &TextLine) -> Vec<(f32, String)> {
    let chars = layout.chars.get(line.chars.clone()).unwrap_or(&[]);
    if chars.is_empty() {
        return Vec::new();
    }
    let mut cells = Vec::new();
    let mut buf = String::new();
    let mut start_x = chars[0].quad.bounds().x0;
    let mut prev_x1 = start_x;
    let gap_threshold = chars[0].size.max(8.0) * 1.1;

    for ch in chars {
        let b = ch.quad.bounds();
        if !buf.trim().is_empty() && (b.x0 - prev_x1) > gap_threshold {
            cells.push((start_x, buf.trim().to_owned()));
            buf.clear();
            start_x = b.x0;
        }
        if buf.is_empty() && ch.c.is_whitespace() {
            start_x = b.x1;
        } else {
            buf.push(ch.c);
        }
        prev_x1 = b.x1;
    }
    if !buf.trim().is_empty() {
        cells.push((start_x, buf.trim().to_owned()));
    }
    cells
}

/// Exports `table` as RFC 4180 formula-safe CSV (plan F.16).
#[must_use]
pub fn table_to_csv(table: &ExtractedTable) -> String {
    let mut out = String::new();
    for row in &table.rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&csv_escape_safe(cell));
        }
        out.push_str("\r\n");
    }
    out
}

fn csv_escape_safe(cell: &str) -> String {
    let safe = if cell.starts_with(['=', '+', '-', '@']) {
        format!("'{cell}")
    } else {
        cell.to_owned()
    };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// Exports `table` as a GitHub-Flavored Markdown table (plan F.16).
#[must_use]
pub fn table_to_markdown(table: &ExtractedTable) -> String {
    let Some(header) = table.rows.first() else {
        return String::new();
    };
    let cols = table
        .rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(header.len())
        .max(1);
    let mut out = String::new();
    if let Some(cap) = &table.caption {
        let _ = write!(out, "*{cap}*\n\n");
    }
    write_md_row(&mut out, header, cols);
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in &table.rows[1..] {
        write_md_row(&mut out, row, cols);
    }
    out
}

fn write_md_row(out: &mut String, row: &[String], cols: usize) {
    out.push('|');
    for i in 0..cols {
        let val = row.get(i).map_or("", String::as_str).replace('|', "\\|");
        let _ = write!(out, " {val} |");
    }
    out.push('\n');
}

/// Extracts raster and vector figure regions paired with nearby `"Figure N"` captions (plan F.16).
#[must_use]
pub fn extract_figures(
    page: usize,
    layout: &TextLayout,
    image_rects: &[PageRect],
    vector_rects: &[PageRect],
) -> Vec<ExtractedFigure> {
    let mut figures = Vec::with_capacity(image_rects.len() + vector_rects.len());
    for &rect in image_rects {
        figures.push(ExtractedFigure {
            page,
            bounds: rect,
            caption: find_nearby_caption(layout, rect, &["figure ", "fig. ", "fig "]),
            kind: FigureKind::RasterImage,
        });
    }
    for &rect in vector_rects {
        figures.push(ExtractedFigure {
            page,
            bounds: rect,
            caption: find_nearby_caption(layout, rect, &["figure ", "fig. ", "fig "]),
            kind: FigureKind::VectorDiagram,
        });
    }
    figures
}

/// Renders an extracted figure region from a MuPDF [`DisplayList`] to crisp PNG bytes at `dpi`
/// (plan F.16).
///
/// # Errors
///
/// Returns [`ExtractError`] if the bounds are empty or MuPDF rendering fails.
pub fn render_figure_png(list: &DisplayList, bounds: PageRect, dpi: f32) -> Result<Vec<u8>> {
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Err(ExtractError::InvalidBounds(
            "figure bounds must have positive area".into(),
        ));
    }
    let scale = (dpi / 72.0).clamp(0.5, 10.0);
    let ctm = Matrix::scale(scale, scale);
    let area = ctm.apply_rect(&bounds);
    let pix = list
        .render_area(&ctm, area, Background::White, None)
        .map_err(|e| ExtractError::Render(e.to_string()))?;
    pix.to_png()
        .map_err(|e| ExtractError::Render(e.to_string()))
}

/// Exports a vector figure bounding box and caption stub as a clean SVG wrapper referencing a
/// high-DPI PNG data URI or vector viewBox (plan F.16).
#[must_use]
pub fn figure_to_svg_with_png_embed(figure: &ExtractedFigure, png_base64: &str) -> String {
    let w = figure.bounds.width().max(1.0);
    let h = figure.bounds.height().max(1.0);
    let title = figure
        .caption
        .as_deref()
        .unwrap_or("Extracted Figure")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.2}\" height=\"{h:.2}\" viewBox=\"0 0 {w:.2} {h:.2}\">\
         <title>{title}</title>\
         <image href=\"data:image/png;base64,{png_base64}\" width=\"{w:.2}\" height=\"{h:.2}\"/>\
         </svg>"
    )
}

fn find_nearby_caption(layout: &TextLayout, target: PageRect, prefixes: &[&str]) -> Option<String> {
    for block in &layout.blocks {
        let lines = layout.lines.get(block.lines.clone()).unwrap_or(&[]);
        let mut text = String::new();
        for l in lines {
            for c in layout.chars.get(l.chars.clone()).unwrap_or(&[]) {
                text.push(c.c);
            }
            text.push(' ');
        }
        let trimmed = text.trim();
        let lower = trimmed.to_ascii_lowercase();
        let matches_prefix = prefixes.iter().any(|p| lower.starts_with(p));
        let horiz_overlap = (block.bbox.x1.min(target.x1) - block.bbox.x0.max(target.x0)) > -40.0;
        let vertical_near =
            (block.bbox.y0 - target.y1).abs() <= 48.0 || (target.y0 - block.bbox.y1).abs() <= 48.0;
        if matches_prefix && horiz_overlap && vertical_near {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn union_rect(a: PageRect, b: PageRect) -> PageRect {
    PageRect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{PagePoint, Quad, TextBlock, TextChar};

    use super::*;

    fn push_cell_line(layout: &mut TextLayout, block_idx: usize, cells: &[(&str, f32)], y: f32) {
        let line_start = layout.lines.len();
        for &(text, x0) in cells {
            let c0 = layout.chars.len();
            let mut x = x0;
            for ch in text.chars() {
                layout.chars.push(TextChar {
                    c: ch,
                    quad: Quad {
                        ul: PagePoint { x, y },
                        ur: PagePoint { x: x + 6.0, y },
                        ll: PagePoint { x, y: y + 10.0 },
                        lr: PagePoint {
                            x: x + 6.0,
                            y: y + 10.0,
                        },
                    },
                    size: 10.0,
                });
                x += 6.0;
            }
            let c1 = layout.chars.len();
            layout.lines.push(TextLine {
                bbox: PageRect {
                    x0,
                    y0: y,
                    x1: x,
                    y1: y + 10.0,
                },
                dir: PagePoint { x: 1.0, y: 0.0 },
                vertical: false,
                joined: false,
                block: block_idx,
                chars: c0..c1,
            });
        }
        let line_end = layout.lines.len();
        let x0 = cells.first().map_or(0.0, |c| c.1);
        let x1 = cells.last().map_or(100.0, |c| c.1 + c.0.len() as f32 * 6.0);
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0,
                y0: y,
                x1,
                y1: y + 10.0,
            },
            lines: line_start..line_end,
        });
    }

    #[test]
    fn extracts_table_rows_columns_caption_csv_and_markdown() {
        let mut layout = TextLayout::default();
        push_cell_line(
            &mut layout,
            0,
            &[("Table 1: Performance summary", 72.0)],
            80.0,
        );
        push_cell_line(
            &mut layout,
            1,
            &[("Model", 72.0), ("Accuracy", 200.0), ("Latency", 340.0)],
            110.0,
        );
        push_cell_line(
            &mut layout,
            2,
            &[("Alpha", 72.0), ("94.2%", 200.0), ("12ms", 340.0)],
            128.0,
        );
        push_cell_line(
            &mut layout,
            3,
            &[("Beta", 72.0), ("96.8%", 200.0), ("15ms", 340.0)],
            146.0,
        );

        let tables = extract_tables(0, &layout);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows.len(), 3);
        assert_eq!(tables[0].rows[0], vec!["Model", "Accuracy", "Latency"]);
        assert_eq!(tables[0].rows[2], vec!["Beta", "96.8%", "15ms"]);
        assert_eq!(
            tables[0].caption.as_deref(),
            Some("Table 1: Performance summary")
        );

        let csv = table_to_csv(&tables[0]);
        assert!(csv.contains("Model,Accuracy,Latency\r\n"));
        let md = table_to_markdown(&tables[0]);
        assert!(md.contains("| Model | Accuracy | Latency |"));
        assert!(md.contains("| Beta | 96.8% | 15ms |"));
    }
}
