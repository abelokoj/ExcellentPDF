//! Reflow / Liquid Mode structural extractor and bidirectional page quad mapping (plan R.16).
//!
//! Converts K.13 [`TextLayout`] pages into structured blocks (headings, paragraphs, list items,
//! tables, and figures with captions) in column-aware reading order (`read::columns`), emits
//! styled HTML suitable for MuPDF's story engine or NSH `TextView`, and keeps exact character-to-
//! quad mappings so selections, highlights, and search work seamlessly in both views.

pub mod bionic;

use std::fmt::Write as _;
use std::ops::Range;

use excellent_mupdf::{PagePoint, PageRect, Quad, TextLayout};

use crate::read::columns::detect_columns;
use bionic::{BionicMode, push_escaped_html};

/// Theme preset for reflow reading view (plan R.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReflowTheme {
    /// Black ink on crisp white paper.
    #[default]
    Light,
    /// Warm brown ink on cream paper.
    Sepia,
    /// Soft light ink on dark charcoal.
    Dark,
    /// Custom background and foreground RGB colors (`0xRRGGBB`).
    Custom {
        /// Background color.
        background: u32,
        /// Foreground text color.
        foreground: u32,
    },
}

impl ReflowTheme {
    /// Returns `(background_hex, foreground_hex)` CSS hex strings.
    #[must_use]
    pub fn css_colors(self) -> (String, String) {
        let (bg, fg) = match self {
            Self::Light => (0xFFFFFF, 0x1A1A1A),
            Self::Sepia => (0xFBF0D9, 0x3E2F20),
            Self::Dark => (0x181A1F, 0xE6E8EB),
            Self::Custom {
                background,
                foreground,
            } => (background & 0xFF_FFFF, foreground & 0xFF_FFFF),
        };
        (format!("#{bg:06X}"), format!("#{fg:06X}"))
    }
}

/// Typography and display configuration for reflow mode (plan R.16 & R.22).
#[derive(Debug, Clone, PartialEq)]
pub struct ReflowConfig {
    /// CSS font family stack.
    pub font_family: String,
    /// Base body font size in points.
    pub font_size_pt: f32,
    /// CSS line-height multiplier.
    pub line_height: f32,
    /// Horizontal/vertical padding in points.
    pub margin_pt: f32,
    /// Color theme.
    pub theme: ReflowTheme,
    /// Optional bionic reading mode (R.22).
    pub bionic: BionicMode,
}

impl Default for ReflowConfig {
    fn default() -> Self {
        Self {
            font_family: "Georgia, serif".to_owned(),
            font_size_pt: 13.0,
            line_height: 1.55,
            margin_pt: 24.0,
            theme: ReflowTheme::Light,
            bionic: BionicMode::Off,
        }
    }
}

/// Semantic kind of a reflowed structural block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// Section heading (`level` 1..=3).
    Heading {
        /// Heading level (`1` = largest).
        level: u8,
    },
    /// Flowing body paragraph.
    Paragraph,
    /// Bullet or numbered list item.
    ListItem {
        /// Ordered (`1.`, `2.`) vs unordered (`•`, `-`).
        ordered: bool,
        /// Original marker text.
        marker: String,
    },
    /// Multi-column aligned table block.
    Table {
        /// Table rows, each a vector of cell texts.
        rows: Vec<Vec<String>>,
    },
    /// Figure bounding box paired with an optional caption.
    Figure {
        /// Optional caption text.
        caption: Option<String>,
    },
}

/// Contiguous run of reflowed text mapped back to its source page character range and quads.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflowSpan {
    /// Reflowed text for this span.
    pub text: String,
    /// Character range `[start, end)` within the enclosing [`ReflowBlock::plain_text`].
    pub block_char_range: Range<usize>,
    /// Source character indices into K.13 `TextLayout::chars` (or `None` for synthetic spaces).
    pub page_char_indices: Vec<Option<usize>>,
    /// Source page quads covering this span.
    pub quads: Vec<Quad>,
}

/// One structural block on a reflowed page (plan R.16).
#[derive(Debug, Clone, PartialEq)]
pub struct ReflowBlock {
    /// Zero-based block identifier within the page.
    pub id: usize,
    /// Zero-based source PDF page index.
    pub page: usize,
    /// Semantic role of this block.
    pub kind: BlockKind,
    /// Bounding box on the source PDF page.
    pub bbox: PageRect,
    /// Plain text of the block in reading order.
    pub plain_text: String,
    /// Spans linking substrings back to source page characters and quads.
    pub spans: Vec<ReflowSpan>,
}

impl ReflowBlock {
    /// Maps a character range `[start, end)` in `self.plain_text` back to exact PDF page [`Quad`]s
    /// for highlighting or selection synchronization (plan R.16 step 4).
    #[must_use]
    pub fn quads_for_char_range(&self, layout: &TextLayout, range: Range<usize>) -> Vec<Quad> {
        let mut selected = Vec::new();
        for span in &self.spans {
            let s0 = span.block_char_range.start;
            let s1 = span.block_char_range.end;
            let o0 = range.start.max(s0);
            let o1 = range.end.min(s1);
            if o0 >= o1 {
                continue;
            }
            for rel in (o0 - s0)..(o1 - s0) {
                if let Some(Some(char_idx)) = span.page_char_indices.get(rel)
                    && let Some(ch) = layout.chars.get(*char_idx)
                {
                    selected.push(ch.quad);
                }
            }
        }
        merge_line_quads(&selected)
    }
}

/// Reflowed representation of a single page with bidirectional quad mapping (plan R.16).
#[derive(Debug, Clone, PartialEq)]
pub struct ReflowPage {
    /// Zero-based page index.
    pub page: usize,
    /// Extracted blocks in column-aware reading order.
    pub blocks: Vec<ReflowBlock>,
}

impl ReflowPage {
    /// Extracts structured reflow blocks from K.13 `layout`, `page_bounds`, and any detected
    /// figure rectangles (`images`).
    #[must_use]
    pub fn extract(
        page: usize,
        layout: &TextLayout,
        page_bounds: PageRect,
        images: &[PageRect],
    ) -> Self {
        let analysis = detect_columns(layout, page_bounds);
        let body_size = median_char_size(layout).max(6.0);

        let mut blocks = Vec::new();
        let mut next_id = 0usize;

        for &block_idx in &analysis.reading_order {
            let Some(raw_block) = layout.blocks.get(block_idx) else {
                continue;
            };
            let line_slice = layout.lines.get(raw_block.lines.clone()).unwrap_or(&[]);
            if line_slice.is_empty() {
                continue;
            }

            let (plain_text, spans, avg_size) = build_block_text_and_spans(layout, line_slice);
            let trimmed = plain_text.trim();
            if trimmed.is_empty() {
                continue;
            }

            let kind = classify_block(trimmed, avg_size, body_size, line_slice, layout);
            blocks.push(ReflowBlock {
                id: next_id,
                page,
                kind,
                bbox: raw_block.bbox,
                plain_text,
                spans,
            });
            next_id += 1;
        }

        // Pair uncaptioned or captioned figure boxes.
        for &img_rect in images {
            let mut caption: Option<String> = None;
            for blk in &blocks {
                let vertical_gap = blk.bbox.y0 - img_rect.y1;
                let horiz_overlap =
                    (blk.bbox.x1.min(img_rect.x1) - blk.bbox.x0.max(img_rect.x0)) > 0.0;
                if horiz_overlap
                    && (-4.0..=48.0).contains(&vertical_gap)
                    && is_figure_caption(&blk.plain_text)
                {
                    caption = Some(blk.plain_text.trim().to_owned());
                    break;
                }
            }
            blocks.push(ReflowBlock {
                id: next_id,
                page,
                kind: BlockKind::Figure { caption },
                bbox: img_rect,
                plain_text: String::new(),
                spans: Vec::new(),
            });
            next_id += 1;
        }

        Self { page, blocks }
    }

    /// Maps a reflow selection `(block_id, char_range)` back to source PDF page [`Quad`]s.
    #[must_use]
    pub fn reflow_to_page_quads(
        &self,
        layout: &TextLayout,
        block_id: usize,
        char_range: Range<usize>,
    ) -> Vec<Quad> {
        self.blocks
            .iter()
            .find(|b| b.id == block_id)
            .map(|b| b.quads_for_char_range(layout, char_range))
            .unwrap_or_default()
    }

    /// Maps a point on the source PDF page back to `(block_id, char_offset)` in reflow space.
    #[must_use]
    pub fn page_point_to_reflow(
        &self,
        layout: &TextLayout,
        point: PagePoint,
    ) -> Option<(usize, usize)> {
        for block in &self.blocks {
            for span in &block.spans {
                for (rel, opt_idx) in span.page_char_indices.iter().enumerate() {
                    if let Some(char_idx) = opt_idx
                        && let Some(ch) = layout.chars.get(*char_idx)
                    {
                        let r = ch.quad.bounds();
                        if point.x >= r.x0 && point.x <= r.x1 && point.y >= r.y0 && point.y <= r.y1
                        {
                            return Some((block.id, span.block_char_range.start + rel));
                        }
                    }
                }
            }
        }
        None
    }

    /// Renders this reflowed page to self-contained HTML + CSS styled by `config`.
    #[must_use]
    pub fn to_html(&self, config: &ReflowConfig) -> String {
        let (bg, fg) = config.theme.css_colors();
        let font_size = config.font_size_pt.clamp(8.0, 36.0);
        let line_height = config.line_height.clamp(1.0, 2.5);
        let margin = config.margin_pt.clamp(0.0, 96.0);

        let mut html = String::new();
        let _ = writeln!(
            html,
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>\
            body{{font-family:{};font-size:{font_size:.1}pt;line-height:{line_height:.2};margin:{margin:.1}pt;background:{bg};color:{fg};}}\
            h1{{font-size:1.5em;margin:0.8em 0 0.4em;}}h2{{font-size:1.25em;margin:0.7em 0 0.35em;}}h3{{font-size:1.1em;margin:0.6em 0 0.3em;}}\
            p{{margin:0.5em 0;}}table{{border-collapse:collapse;margin:0.7em 0;width:100%;}}td{{border:1px solid #8884;padding:4px 6px;}}\
            figure{{margin:0.8em 0;padding:8px;border:1px dashed #8886;}}figcaption{{font-size:0.9em;opacity:0.85;}}\
            </style></head><body>",
            config.font_family
        );

        let format_text = |text: &str| -> String {
            match config.bionic.strength() {
                Some(s) => bionic::to_html(text, s),
                None => {
                    let mut buf = String::new();
                    push_escaped_html(&mut buf, text);
                    buf
                }
            }
        };

        let mut in_ul = false;
        let mut in_ol = false;
        let close_lists = |html: &mut String, in_ul: &mut bool, in_ol: &mut bool| {
            if *in_ul {
                html.push_str("</ul>\n");
                *in_ul = false;
            }
            if *in_ol {
                html.push_str("</ol>\n");
                *in_ol = false;
            }
        };

        for blk in &self.blocks {
            let text = blk.plain_text.trim();
            match &blk.kind {
                BlockKind::Heading { level } => {
                    close_lists(&mut html, &mut in_ul, &mut in_ol);
                    let lvl = (*level).clamp(1, 3);
                    let _ = writeln!(
                        html,
                        "<h{lvl} data-page=\"{}\" data-block-id=\"{}\">{}</h{lvl}>",
                        blk.page,
                        blk.id,
                        format_text(text)
                    );
                }
                BlockKind::Paragraph => {
                    close_lists(&mut html, &mut in_ul, &mut in_ol);
                    let _ = writeln!(
                        html,
                        "<p data-page=\"{}\" data-block-id=\"{}\">{}</p>",
                        blk.page,
                        blk.id,
                        format_text(text)
                    );
                }
                BlockKind::ListItem { ordered, .. } => {
                    if *ordered {
                        if in_ul {
                            html.push_str("</ul>\n");
                            in_ul = false;
                        }
                        if !in_ol {
                            html.push_str("<ol>\n");
                            in_ol = true;
                        }
                    } else {
                        if in_ol {
                            html.push_str("</ol>\n");
                            in_ol = false;
                        }
                        if !in_ul {
                            html.push_str("<ul>\n");
                            in_ul = true;
                        }
                    }
                    let _ = writeln!(
                        html,
                        "<li data-page=\"{}\" data-block-id=\"{}\">{}</li>",
                        blk.page,
                        blk.id,
                        format_text(text)
                    );
                }
                BlockKind::Table { rows } => {
                    close_lists(&mut html, &mut in_ul, &mut in_ol);
                    let _ = writeln!(
                        html,
                        "<table data-page=\"{}\" data-block-id=\"{}\">",
                        blk.page, blk.id
                    );
                    for row in rows {
                        html.push_str("<tr>");
                        for cell in row {
                            let _ = write!(html, "<td>{}</td>", format_text(cell));
                        }
                        html.push_str("</tr>\n");
                    }
                    html.push_str("</table>\n");
                }
                BlockKind::Figure { caption } => {
                    close_lists(&mut html, &mut in_ul, &mut in_ol);
                    let _ = write!(
                        html,
                        "<figure data-page=\"{}\" data-block-id=\"{}\">",
                        blk.page, blk.id
                    );
                    if let Some(cap) = caption {
                        let _ = write!(html, "<figcaption>{}</figcaption>", format_text(cap));
                    }
                    html.push_str("</figure>\n");
                }
            }
        }
        close_lists(&mut html, &mut in_ul, &mut in_ol);
        html.push_str("</body></html>");
        html
    }
}

fn median_char_size(layout: &TextLayout) -> f32 {
    let mut sizes: Vec<f32> = layout
        .chars
        .iter()
        .filter(|c| !c.c.is_whitespace() && c.size > 1.0)
        .map(|c| c.size)
        .collect();
    if sizes.is_empty() {
        return 11.0;
    }
    sizes.sort_by(f32::total_cmp);
    sizes[sizes.len() / 2]
}

fn build_block_text_and_spans(
    layout: &TextLayout,
    lines: &[excellent_mupdf::TextLine],
) -> (String, Vec<ReflowSpan>, f32) {
    let mut plain = String::new();
    let mut spans = Vec::new();
    let mut char_cursor = 0usize;
    let mut size_sum = 0.0_f32;
    let mut size_count = 0usize;
    let mut prev_joined = false;

    for (line_idx, line) in lines.iter().enumerate() {
        if line_idx > 0 && !prev_joined && !plain.ends_with(' ') {
            plain.push(' ');
            spans.push(ReflowSpan {
                text: " ".to_owned(),
                block_char_range: char_cursor..char_cursor + 1,
                page_char_indices: vec![None],
                quads: Vec::new(),
            });
            char_cursor += 1;
        }

        let mut end = line.chars.end.min(layout.chars.len());
        let start = line.chars.start.min(end);
        if line.joined && end > start {
            let last_c = layout.chars[end - 1].c;
            if matches!(last_c, '-' | '\u{00AD}' | '\u{2010}') {
                end -= 1;
            }
        }

        let mut line_str = String::new();
        let mut page_indices = Vec::new();
        let mut line_quads = Vec::new();

        for idx in start..end {
            let tc = &layout.chars[idx];
            if tc.c.is_control() && tc.c != '\t' {
                continue;
            }
            let emit = if tc.c == '\t' { ' ' } else { tc.c };
            line_str.push(emit);
            page_indices.push(Some(idx));
            line_quads.push(tc.quad);
            if !emit.is_whitespace() {
                size_sum += tc.size;
                size_count += 1;
            }
        }

        let char_len = page_indices.len();
        if char_len > 0 {
            plain.push_str(&line_str);
            spans.push(ReflowSpan {
                text: line_str,
                block_char_range: char_cursor..char_cursor + char_len,
                page_char_indices: page_indices,
                quads: merge_line_quads(&line_quads),
            });
            char_cursor += char_len;
        }
        prev_joined = line.joined;
    }

    let avg_size = if size_count > 0 {
        size_sum / size_count as f32
    } else {
        11.0
    };
    (plain, spans, avg_size)
}

fn classify_block(
    trimmed: &str,
    avg_size: f32,
    body_size: f32,
    lines: &[excellent_mupdf::TextLine],
    layout: &TextLayout,
) -> BlockKind {
    // Check list item markers first.
    if trimmed.starts_with("• ") || trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let marker = trimmed.chars().take(1).collect();
        return BlockKind::ListItem {
            ordered: false,
            marker,
        };
    }
    if let Some((prefix, rest)) = trimmed.split_once(". ")
        && !prefix.is_empty()
        && prefix.len() <= 4
        && prefix.chars().all(|c| c.is_ascii_digit())
        && !rest.is_empty()
    {
        return BlockKind::ListItem {
            ordered: true,
            marker: format!("{prefix}."),
        };
    }

    // Check heading by font scale and shortness.
    let ratio = avg_size / body_size.max(1.0);
    if ratio >= 1.15 && trimmed.len() <= 140 && lines.len() <= 3 {
        let level = if ratio >= 1.5 {
            1
        } else if ratio >= 1.3 {
            2
        } else {
            3
        };
        return BlockKind::Heading { level };
    }

    // Check table: multiple lines sharing wide internal whitespace gaps (tabs or >= 3 spaces).
    if lines.len() >= 2 {
        let mut rows = Vec::new();
        for line in lines {
            let row_text: String = layout
                .chars
                .get(line.chars.clone())
                .unwrap_or(&[])
                .iter()
                .map(|c| c.c)
                .collect();
            let cells: Vec<String> = row_text
                .split("   ")
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect();
            if cells.len() >= 2 {
                rows.push(cells);
            }
        }
        if rows.len() >= 2 && rows.len() * 2 >= lines.len() {
            return BlockKind::Table { rows };
        }
    }

    BlockKind::Paragraph
}

fn is_figure_caption(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("figure ")
        || lower.starts_with("fig. ")
        || lower.starts_with("fig ")
        || lower.starts_with("table ")
}

/// Merges adjacent character quads on the same horizontal line into compact highlight quads.
fn merge_line_quads(quads: &[Quad]) -> Vec<Quad> {
    let mut merged: Vec<Quad> = Vec::new();
    for &q in quads {
        let qb = q.bounds();
        if let Some(last) = merged.last_mut() {
            let lb = last.bounds();
            let same_line = (lb.y0 - qb.y0).abs() < (lb.height().max(qb.height()) * 0.6);
            let close_x = qb.x0 - lb.x1 < lb.height().max(8.0) * 0.8 && qb.x1 >= lb.x0;
            if same_line && close_x {
                let x0 = lb.x0.min(qb.x0);
                let y0 = lb.y0.min(qb.y0);
                let x1 = lb.x1.max(qb.x1);
                let y1 = lb.y1.max(qb.y1);
                *last = Quad {
                    ul: PagePoint { x: x0, y: y0 },
                    ur: PagePoint { x: x1, y: y0 },
                    ll: PagePoint { x: x0, y: y1 },
                    lr: PagePoint { x: x1, y: y1 },
                };
                continue;
            }
        }
        merged.push(q);
    }
    merged
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{TextBlock, TextChar, TextLine};

    use super::*;

    fn make_char(c: char, x: f32, y: f32, size: f32) -> TextChar {
        let w = size * 0.55;
        TextChar {
            c,
            quad: Quad {
                ul: PagePoint { x, y },
                ur: PagePoint { x: x + w, y },
                ll: PagePoint { x, y: y + size },
                lr: PagePoint {
                    x: x + w,
                    y: y + size,
                },
            },
            size,
        }
    }

    fn push_line(
        layout: &mut TextLayout,
        block_idx: usize,
        text: &str,
        x0: f32,
        y0: f32,
        size: f32,
    ) {
        let start = layout.chars.len();
        let mut x = x0;
        for ch in text.chars() {
            layout.chars.push(make_char(ch, x, y0, size));
            x += size * 0.55;
        }
        let end = layout.chars.len();
        layout.lines.push(TextLine {
            bbox: PageRect {
                x0,
                y0,
                x1: x,
                y1: y0 + size,
            },
            dir: PagePoint { x: 1.0, y: 0.0 },
            vertical: false,
            joined: false,
            block: block_idx,
            chars: start..end,
        });
    }

    #[test]
    fn extracts_two_column_reflow_and_maps_selection_quads_bidirectionally() {
        let mut layout = TextLayout::default();

        // Block 0: Full-width heading (18 pt)
        let l0 = layout.lines.len();
        push_line(&mut layout, 0, "Introduction", 72.0, 50.0, 18.0);
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 72.0,
                y0: 50.0,
                x1: 500.0,
                y1: 70.0,
            },
            lines: l0..l0 + 1,
        });

        // Block 1: Right column paragraph (11 pt) placed earlier in stream
        let l1 = layout.lines.len();
        push_line(&mut layout, 1, "Second column text.", 320.0, 110.0, 11.0);
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 320.0,
                y0: 110.0,
                x1: 540.0,
                y1: 125.0,
            },
            lines: l1..l1 + 1,
        });

        // Block 2: Left column paragraph (11 pt)
        let l2 = layout.lines.len();
        push_line(
            &mut layout,
            2,
            "First column highlight target.",
            72.0,
            110.0,
            11.0,
        );
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 72.0,
                y0: 110.0,
                x1: 290.0,
                y1: 125.0,
            },
            lines: l2..l2 + 1,
        });

        let page_rect = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        let reflow = ReflowPage::extract(0, &layout, page_rect, &[]);
        assert_eq!(reflow.blocks.len(), 3);
        assert!(matches!(
            reflow.blocks[0].kind,
            BlockKind::Heading { level: 1 }
        ));
        assert_eq!(
            reflow.blocks[1].plain_text,
            "First column highlight target."
        );
        assert_eq!(reflow.blocks[2].plain_text, "Second column text.");

        // Select "highlight" inside reflow block 1 (index 13..22) and verify PDF quads match left column!
        let quads = reflow.reflow_to_page_quads(&layout, reflow.blocks[1].id, 13..22);
        assert_eq!(quads.len(), 1);
        let qb = quads[0].bounds();
        assert!(
            qb.x0 >= 72.0 && qb.x1 <= 290.0,
            "quad in left column: {qb:?}"
        );

        // Reverse lookup: clicking inside that quad maps back to reflow block 1.
        let hit = reflow
            .page_point_to_reflow(
                &layout,
                PagePoint {
                    x: (qb.x0 + qb.x1) * 0.5,
                    y: (qb.y0 + qb.y1) * 0.5,
                },
            )
            .expect("reverse lookup");
        assert_eq!(hit.0, reflow.blocks[1].id);
        assert!((13..22).contains(&hit.1));

        // HTML with Bionic Reading enabled bolds prefixes cleanly.
        let html = reflow.to_html(&ReflowConfig {
            theme: ReflowTheme::Sepia,
            bionic: BionicMode::On { strength: 0.4 },
            ..ReflowConfig::default()
        });
        assert!(html.contains("<h1 "));
        assert!(html.contains("<strong>Intro</strong>duction"));
        assert!(html.contains("#FBF0D9"));
    }
}
