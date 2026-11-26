//! Heading, numbering-rule, and PDF structure-tag outline generator with an editable preview
//! tree (plan B.8).

use std::collections::HashMap;

use excellent_mupdf::{OutlineStyle, TextLayout};

use crate::outline::Entry;

/// Minimum font-size ratio relative to median body text to treat an unnumbered line as a heading.
const DEFAULT_HEADING_SCALE: f32 = 1.15;

/// Maximum heading line character length.
const MAX_HEADING_CHARS: usize = 140;

/// How a heading candidate was discovered (plan B.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingSource {
    /// Detected from larger font size / visual prominence.
    FontScale,
    /// Matched a hierarchical numbering prefix (`1.2.`, `II.A.`, `Chapter 1`, etc.).
    NumberingRule,
    /// Read from PDF structure tree tags (`H1`–`H6`, `Part`, `Sect`).
    StructureTag,
    /// Synthesized from combined source file boundaries (O.2).
    CombinedFile,
}

/// Recognized heading numbering style (ported from SumatraPDF Enhanced's heading rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberingPattern {
    /// Decimal hierarchical (`"1."`, `"1.2"`, `"1.2.3."`).
    Decimal {
        /// Hierarchy depth (`1` for `"1."`, `2` for `"1.2."`, etc.).
        depth: u8,
    },
    /// Roman numeral (`"I."`, `"II."`, `"IV."`).
    Roman,
    /// Uppercase alphabetic (`"A."`, `"B."`).
    AlphaUpper,
    /// Named section keyword (`"Chapter 1"`, `"Section 2.1"`, `"Part I"`, `"Appendix A"`).
    NamedKeyword {
        /// Hierarchy depth (`1` for Part/Chapter/Appendix, `2` for Section).
        depth: u8,
    },
}

/// Options controlling outline generation from page layouts (plan B.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenerateOptions {
    /// Minimum font-size multiplier relative to median body font size (default `1.15`).
    pub min_heading_scale: f32,
    /// Maximum nesting depth in the generated outline (`1..=6`, default `4`).
    pub max_depth: u8,
    /// Whether to include numbering-rule detection on body-sized lines (`1.2. Title`).
    pub use_numbering_rules: NumberingMode,
}

/// Whether numbering rules are evaluated during outline generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NumberingMode {
    /// Use both font-size heuristics and numbering rules (`1.2.`, `II.A.`, `Chapter 1`).
    #[default]
    Enabled,
    /// Use font-size heuristics only.
    Disabled,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            min_heading_scale: DEFAULT_HEADING_SCALE,
            max_depth: 4,
            use_numbering_rules: NumberingMode::Enabled,
        }
    }
}

/// A flat detected heading before tree nesting.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadingCandidate {
    /// Display title.
    pub title: String,
    /// 1-based hierarchy level (`1` = top-level chapter).
    pub level: u8,
    /// Zero-based destination page index.
    pub page: usize,
    /// Vertical coordinate on the page in points.
    pub y_pt: f32,
    /// Detection source.
    pub source: HeadingSource,
    /// Heuristic confidence in `0.0..=1.0`.
    pub confidence: f32,
}

/// Whether a preview node is accepted into the final outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeDecision {
    /// Include in the finalized outline.
    #[default]
    Accept,
    /// Discard from the finalized outline.
    Discard,
}

/// Editable preview tree node allowing the user to accept, rename, or discard entries before
/// committing them to the PDF outline (plan B.8 step 4).
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewNode {
    /// Proposed outline entry.
    pub entry: Entry,
    /// Whether this node is accepted or discarded.
    pub decision: NodeDecision,
    /// How this node was detected.
    pub source: HeadingSource,
    /// Heuristic confidence in `0.0..=1.0`.
    pub confidence: f32,
    /// Nested child preview nodes.
    pub children: Vec<PreviewNode>,
}

impl PreviewNode {
    /// Converts accepted nodes recursively into finalized [`Entry`] values.
    #[must_use]
    pub fn to_entries(&self) -> Vec<Entry> {
        let children: Vec<Entry> = self.children.iter().flat_map(Self::to_entries).collect();
        if self.decision == NodeDecision::Discard {
            return children;
        }
        let mut entry = self.entry.clone();
        entry.children = children;
        vec![entry]
    }
}

/// Interactive preview tree returned by outline generators (plan B.8 step 4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OutlinePreviewTree {
    /// Root-level preview nodes.
    pub roots: Vec<PreviewNode>,
}

impl OutlinePreviewTree {
    /// Finds a mutable node by path of indices (e.g. `[0, 2]`).
    pub fn get_mut(&mut self, path: &[usize]) -> Option<&mut PreviewNode> {
        let (&first, rest) = path.split_first()?;
        let mut node = self.roots.get_mut(first)?;
        for &idx in rest {
            node = node.children.get_mut(idx)?;
        }
        Some(node)
    }

    /// Renames the preview node at `path`. Returns `true` if `path` existed.
    pub fn rename(&mut self, path: &[usize], new_title: impl Into<String>) -> bool {
        let Some(node) = self.get_mut(path) else {
            return false;
        };
        node.entry.title = new_title.into();
        true
    }

    /// Sets accept/discard decision on the node at `path`. Returns `true` if `path` existed.
    pub fn set_decision(&mut self, path: &[usize], decision: NodeDecision) -> bool {
        let Some(node) = self.get_mut(path) else {
            return false;
        };
        node.decision = decision;
        true
    }

    /// Produces the final [`Entry`] tree containing all accepted nodes.
    #[must_use]
    pub fn finalize(&self) -> Vec<Entry> {
        self.roots
            .iter()
            .flat_map(PreviewNode::to_entries)
            .collect()
    }
}

/// A PDF structure tree tag node (e.g., `H1`..`H6`, `Part`, `Sect`) for tagged PDFs (plan B.8 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureTagNode {
    /// PDF structure tag role (`"H1"`, `"H2"`, ..., `"Part"`, `"Sect"`).
    pub tag: String,
    /// Visible heading text.
    pub title: String,
    /// Zero-based destination page index.
    pub page: usize,
    /// Optional target y coordinate in page points.
    pub y_pt: Option<u32>,
    /// Nested structure children.
    pub children: Vec<StructureTagNode>,
}

/// Detects hierarchical numbering prefixes such as `"1."`, `"1.2."`, `"II.A."`, `"Chapter 1"`.
/// Returns `(NumberingPattern, depth)` when matched.
#[must_use]
pub fn detect_numbering_level(line: &str) -> Option<(NumberingPattern, u8)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    for (kw, depth) in [
        ("part ", 1u8),
        ("chapter ", 1),
        ("appendix ", 1),
        ("section ", 2),
    ] {
        if let Some(after) = lower.strip_prefix(kw)
            && after
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric())
        {
            return Some((NumberingPattern::NamedKeyword { depth }, depth));
        }
    }

    // Split leading token before first whitespace.
    let (token, rest) = trimmed.split_once(char::is_whitespace)?;
    if rest.trim().is_empty() {
        return None;
    }

    // Decimal multi-level: "1.", "1.2", "1.2.3."
    let core = token.trim_end_matches('.');
    if !core.is_empty()
        && core
            .split('.')
            .all(|seg| !seg.is_empty() && seg.len() <= 3 && seg.chars().all(|c| c.is_ascii_digit()))
    {
        let segs = core.split('.').count();
        // Require a trailing dot on single-level numbers ("1. Intro") to avoid matching years/counts ("2026 report").
        if segs > 1 || token.ends_with('.') {
            let depth = u8::try_from(segs.clamp(1, 6)).unwrap_or(6);
            return Some((NumberingPattern::Decimal { depth }, depth));
        }
    }

    // Roman + optional sub-alpha: "I.", "II.", "IV.", "II.A."
    if token.ends_with('.') {
        let parts: Vec<&str> = core.split('.').collect();
        if let Some(&first) = parts.first()
            && !first.is_empty()
            && first.len() <= 6
            && first.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L'))
        {
            if parts.len() == 1 {
                return Some((NumberingPattern::Roman, 1));
            }
            if parts.len() == 2
                && parts[1].len() == 1
                && parts[1].chars().all(|c| c.is_ascii_uppercase())
            {
                return Some((NumberingPattern::AlphaUpper, 2));
            }
        }
        if parts.len() == 1
            && first_is_single_upper_alpha(first_token(parts.first().copied().unwrap_or("")))
        {
            return Some((NumberingPattern::AlphaUpper, 2));
        }
    }

    None
}

fn first_token(s: &str) -> &str {
    s
}

fn first_is_single_upper_alpha(s: &str) -> bool {
    s.len() == 1 && s.chars().all(|c| c.is_ascii_uppercase())
}

/// Generates an interactive [`OutlinePreviewTree`] from per-page K.13 [`TextLayout`]s (plan B.8 step 1).
#[must_use]
pub fn generate_from_layouts(
    pages: &[(usize, &TextLayout)],
    options: &GenerateOptions,
) -> OutlinePreviewTree {
    let body_size = estimate_body_size(pages).max(6.0);
    let min_scale = options.min_heading_scale.max(1.02);

    // Collect candidate headings and distinct heading font sizes to assign levels cleanly.
    let mut raw_candidates: Vec<(HeadingCandidate, f32)> = Vec::new();
    let mut heading_sizes: Vec<f32> = Vec::new();

    for &(page_idx, layout) in pages {
        for line in &layout.lines {
            let chars = layout.chars.get(line.chars.clone()).unwrap_or(&[]);
            if chars.is_empty() {
                continue;
            }
            let text: String = chars.iter().map(|c| c.c).collect();
            let trimmed = text.trim();
            if trimmed.len() < 2 || trimmed.len() > MAX_HEADING_CHARS {
                continue;
            }
            let avg_size = average_non_space_size(chars);
            let scale = avg_size / body_size;
            let numbered = if options.use_numbering_rules == NumberingMode::Enabled {
                detect_numbering_level(trimmed)
            } else {
                None
            };

            if let Some((_, depth)) = numbered
                && scale >= 0.95
            {
                raw_candidates.push((
                    HeadingCandidate {
                        title: trimmed.to_owned(),
                        level: depth.min(options.max_depth.max(1)),
                        page: page_idx,
                        y_pt: line.bbox.y0,
                        source: HeadingSource::NumberingRule,
                        confidence: if scale >= min_scale { 0.95 } else { 0.80 },
                    },
                    avg_size,
                ));
                if scale >= min_scale {
                    heading_sizes.push(avg_size);
                }
            } else if scale >= min_scale && !trimmed.ends_with('.') && !trimmed.ends_with(',') {
                heading_sizes.push(avg_size);
                raw_candidates.push((
                    HeadingCandidate {
                        title: trimmed.to_owned(),
                        level: 0, // assigned below from font size tiers
                        page: page_idx,
                        y_pt: line.bbox.y0,
                        source: HeadingSource::FontScale,
                        confidence: ((scale - 1.0) * 1.5).clamp(0.55, 0.95),
                    },
                    avg_size,
                ));
            }
        }
    }

    // Cluster distinct font-size tiers (within 0.8 pt) descending -> levels 1, 2, 3...
    heading_sizes.sort_by(|a, b| b.total_cmp(a));
    let mut tiers: Vec<f32> = Vec::new();
    for sz in heading_sizes {
        if tiers.last().is_none_or(|&prev| (prev - sz).abs() > 0.8) {
            tiers.push(sz);
        }
    }

    let max_depth = options.max_depth.clamp(1, 6);
    let candidates: Vec<HeadingCandidate> = raw_candidates
        .into_iter()
        .map(|(mut cand, sz)| {
            if cand.level == 0 {
                let tier_idx = tiers
                    .iter()
                    .position(|&t| (t - sz).abs() <= 0.8)
                    .unwrap_or(0);
                cand.level = u8::try_from((tier_idx + 1).min(max_depth as usize)).unwrap_or(1);
            }
            cand
        })
        .collect();

    OutlinePreviewTree {
        roots: nest_candidates(&candidates),
    }
}

/// Builds an [`OutlinePreviewTree`] from a tagged PDF's structure tags (plan B.8 step 2).
#[must_use]
pub fn from_structure_tags(tags: &[StructureTagNode]) -> OutlinePreviewTree {
    fn convert_tag(node: &StructureTagNode) -> Vec<PreviewNode> {
        let lower = node.tag.to_ascii_lowercase();
        let is_heading = matches!(
            lower.as_str(),
            "h" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "part" | "sect" | "art"
        );
        let child_nodes: Vec<PreviewNode> = node.children.iter().flat_map(convert_tag).collect();
        if !is_heading || node.title.trim().is_empty() {
            return child_nodes;
        }
        let y_suffix = node
            .y_pt
            .map_or_else(String::new, |y| format!("&zoom=nan,0,{y}"));
        let uri = Some(format!("#page={}{y_suffix}", node.page + 1));
        vec![PreviewNode {
            entry: Entry {
                title: node.title.trim().to_owned(),
                page: Some(node.page),
                uri,
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
            decision: NodeDecision::Accept,
            source: HeadingSource::StructureTag,
            confidence: 1.0,
            children: child_nodes,
        }]
    }

    OutlinePreviewTree {
        roots: tags.iter().flat_map(convert_tag).collect(),
    }
}

/// Creates one top-level outline [`Entry`] per combined input file `(title, start_page)` (plan B.8 step 3).
#[must_use]
pub fn for_combined_files(files: &[(String, usize)]) -> Vec<Entry> {
    files
        .iter()
        .map(|(title, start_page)| Entry {
            title: title.clone(),
            page: Some(*start_page),
            uri: Some(format!("#page={}", start_page + 1)),
            open: true,
            style: OutlineStyle::default(),
            children: Vec::new(),
        })
        .collect()
}

fn estimate_body_size(pages: &[(usize, &TextLayout)]) -> f32 {
    let mut bucket_counts: HashMap<i32, usize> = HashMap::new();
    for &(_, layout) in pages {
        for ch in &layout.chars {
            if !ch.c.is_whitespace() && ch.size > 2.0 {
                let bucket = (ch.size * 2.0).round() as i32;
                *bucket_counts.entry(bucket).or_insert(0) += 1;
            }
        }
    }
    bucket_counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map_or(11.0, |(bucket, _)| bucket as f32 / 2.0)
}

fn average_non_space_size(chars: &[excellent_mupdf::TextChar]) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0usize;
    for ch in chars {
        if !ch.c.is_whitespace() && ch.size > 0.0 {
            sum += ch.size;
            count += 1;
        }
    }
    if count > 0 { sum / count as f32 } else { 11.0 }
}

fn nest_candidates(candidates: &[HeadingCandidate]) -> Vec<PreviewNode> {
    let mut roots: Vec<PreviewNode> = Vec::new();
    let mut stack: Vec<(u8, PreviewNode)> = Vec::new();

    let flush_top = |stack: &mut Vec<(u8, PreviewNode)>, roots: &mut Vec<PreviewNode>| {
        if let Some((_, finished)) = stack.pop() {
            if let Some((_, parent)) = stack.last_mut() {
                parent.children.push(finished);
            } else {
                roots.push(finished);
            }
        }
    };

    for cand in candidates {
        while stack.last().is_some_and(|(lvl, _)| *lvl >= cand.level) {
            flush_top(&mut stack, &mut roots);
        }
        let node = PreviewNode {
            entry: Entry {
                title: cand.title.clone(),
                page: Some(cand.page),
                uri: Some(format!(
                    "#page={}&zoom=nan,0,{}",
                    cand.page + 1,
                    cand.y_pt.round()
                )),
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
            decision: NodeDecision::Accept,
            source: cand.source,
            confidence: cand.confidence,
            children: Vec::new(),
        };
        stack.push((cand.level, node));
    }
    while !stack.is_empty() {
        flush_top(&mut stack, &mut roots);
    }
    roots
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{PagePoint, PageRect, Quad, TextBlock, TextChar, TextLine};

    use super::*;

    fn push_line(layout: &mut TextLayout, text: &str, y0: f32, size: f32) {
        let start = layout.chars.len();
        let mut x = 72.0;
        for ch in text.chars() {
            layout.chars.push(TextChar {
                c: ch,
                quad: Quad {
                    ul: PagePoint { x, y: y0 },
                    ur: PagePoint {
                        x: x + size * 0.5,
                        y: y0,
                    },
                    ll: PagePoint { x, y: y0 + size },
                    lr: PagePoint {
                        x: x + size * 0.5,
                        y: y0 + size,
                    },
                },
                size,
            });
            x += size * 0.5;
        }
        let end = layout.chars.len();
        let block_idx = layout.blocks.len();
        let line_idx = layout.lines.len();
        layout.lines.push(TextLine {
            bbox: PageRect {
                x0: 72.0,
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
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 72.0,
                y0,
                x1: x,
                y1: y0 + size,
            },
            lines: line_idx..line_idx + 1,
        });
    }

    #[test]
    fn generates_nested_outline_from_headings_and_numbering_with_preview_editing() {
        let mut p0 = TextLayout::default();
        push_line(&mut p0, "1. Introduction", 60.0, 18.0);
        push_line(
            &mut p0,
            "This is a long body paragraph explaining the background of the work in detail.",
            95.0,
            10.0,
        );
        push_line(&mut p0, "1.1. Motivation", 150.0, 10.0);
        push_line(
            &mut p0,
            "More body text that sets the dominant median font size at ten points.",
            175.0,
            10.0,
        );

        let mut p1 = TextLayout::default();
        push_line(&mut p1, "II. Methods", 60.0, 18.0);
        push_line(&mut p1, "II.A. Experimental Setup", 100.0, 13.0);

        let mut preview = generate_from_layouts(&[(0, &p0), (1, &p1)], &GenerateOptions::default());
        assert_eq!(preview.roots.len(), 2);
        assert_eq!(preview.roots[0].entry.title, "1. Introduction");
        assert_eq!(preview.roots[0].children[0].entry.title, "1.1. Motivation");
        assert_eq!(preview.roots[1].entry.title, "II. Methods");
        assert_eq!(
            preview.roots[1].children[0].entry.title,
            "II.A. Experimental Setup"
        );

        assert!(preview.rename(&[0], "Chapter 1: Introduction"));
        assert!(preview.set_decision(&[1, 0], NodeDecision::Discard));
        let finalized = preview.finalize();
        assert_eq!(finalized[0].title, "Chapter 1: Introduction");
        assert_eq!(finalized[0].children.len(), 1);
        assert!(finalized[1].children.is_empty());
    }

    #[test]
    fn generates_from_structure_tags_and_combined_files() {
        let tags = vec![StructureTagNode {
            tag: "H1".into(),
            title: "Part One".into(),
            page: 0,
            y_pt: Some(72),
            children: vec![StructureTagNode {
                tag: "H2".into(),
                title: "Section A".into(),
                page: 2,
                y_pt: None,
                children: Vec::new(),
            }],
        }];
        let entries = from_structure_tags(&tags).finalize();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Part One");
        assert_eq!(entries[0].children[0].page, Some(2));

        let combined = for_combined_files(&[("a.pdf".into(), 0), ("b.pdf".into(), 5)]);
        assert_eq!(combined.len(), 2);
        assert_eq!(combined[1].page, Some(5));
    }
}
