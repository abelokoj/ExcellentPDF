//! Auto-summaries from highlights and notes grouped by heading (plan S.13).
//!
//! Works 100 % offline with zero optional packs by sorting highlights and notes in reading order
//! and grouping them under the nearest preceding document heading (`B.8` or PDF outline). Also
//! exposes an optional [`SummaryCondenser`] trait for AI (`I.4`) condensation that cites every
//! source highlight ID.

use std::fmt::{self, Write as _};

use crate::study::export::StudyNoteKind;

/// Heading anchor used to group highlights into sections (from PDF outline or B.8 generator).
#[derive(Debug, Clone, PartialEq)]
pub struct HeadingAnchor {
    /// Heading display title.
    pub title: String,
    /// Hierarchy level (`1` = chapter, `2` = section, etc.).
    pub level: u8,
    /// Zero-based page index where the heading appears.
    pub page: usize,
    /// Vertical position in top-left page points.
    pub y_pt: f32,
}

/// A highlight, underline, or note collected from a document for summarization (plan S.13).
#[derive(Debug, Clone, PartialEq)]
pub struct HighlightItem {
    /// Stable annotation/highlight identifier (used for AI citation references).
    pub id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Vertical position in top-left page points.
    pub y_pt: f32,
    /// Horizontal position in top-left page points.
    pub x_pt: f32,
    /// Annotation kind.
    pub kind: StudyNoteKind,
    /// Quoted text under the highlight, if any.
    pub quoted_text: String,
    /// Reader's comment or note text, if any.
    pub note: String,
    /// Associated tags.
    pub tags: Vec<String>,
}

/// Whether tags and page citations are rendered into the Markdown summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SummaryDetail {
    /// Include quoted text, reader notes, page numbers, and tags.
    #[default]
    Full,
    /// Compact bullets with quoted text and page numbers only.
    Compact,
}

/// One grouped section in a generated summary.
#[derive(Debug, Clone, PartialEq)]
pub struct SummarySection {
    /// Heading title (or `"Document Notes"` if items precede the first heading).
    pub heading: String,
    /// Heading level (`1..=6`).
    pub level: u8,
    /// Highlights belonging to this section, sorted in reading order.
    pub items: Vec<HighlightItem>,
}

/// Structured offline summary ready for Markdown export or optional AI condensation (plan S.13).
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryDocument {
    /// Document title.
    pub title: String,
    /// Grouped sections in reading order.
    pub sections: Vec<SummarySection>,
    /// Rendered Markdown output.
    pub markdown: String,
}

/// Optional AI condenser interface (`I.4`) that condenses grouped highlights while preserving
/// explicit citations (`[id]`) back to source highlights (plan S.13 step 2).
pub trait SummaryCondenser: Send + Sync {
    /// Produces a condensed Markdown summary from `summary`, citing highlight IDs.
    ///
    /// # Errors
    ///
    /// Returns a human-readable error if the AI provider fails or is disabled.
    fn condense(&self, summary: &SummaryDocument) -> std::result::Result<String, CondenseError>;
}

/// Error returned by an optional [`SummaryCondenser`] backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CondenseError(pub String);

impl fmt::Display for CondenseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CondenseError {}

/// Builds an offline Markdown summary from `highlights` grouped by `headings` in reading order,
/// requiring zero AI or optional packs (plan S.13 **Done when**).
#[must_use]
pub fn build_offline_summary(
    doc_title: &str,
    headings: &[HeadingAnchor],
    highlights: &[HighlightItem],
    detail: SummaryDetail,
) -> SummaryDocument {
    let mut sorted_headings = headings.to_vec();
    sorted_headings.sort_by(|a, b| a.page.cmp(&b.page).then_with(|| a.y_pt.total_cmp(&b.y_pt)));

    let mut sorted_items: Vec<HighlightItem> = highlights
        .iter()
        .filter(|h| !h.quoted_text.trim().is_empty() || !h.note.trim().is_empty())
        .cloned()
        .collect();
    sorted_items.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| a.y_pt.total_cmp(&b.y_pt))
            .then_with(|| a.x_pt.total_cmp(&b.x_pt))
    });

    let mut sections: Vec<SummarySection> = Vec::new();
    for item in sorted_items {
        let matched = sorted_headings
            .iter()
            .rfind(|h| h.page < item.page || (h.page == item.page && h.y_pt <= item.y_pt + 2.0));
        let (heading, level) = match matched {
            Some(h) => (h.title.clone(), h.level.clamp(1, 6)),
            None => ("Document Notes".to_owned(), 2),
        };
        if let Some(last) = sections.last_mut()
            && last.heading == heading
            && last.level == level
        {
            last.items.push(item);
        } else {
            sections.push(SummarySection {
                heading,
                level,
                items: vec![item],
            });
        }
    }

    let mut md = String::new();
    let _ = writeln!(md, "# Summary: {}\n", doc_title.trim());
    for sec in &sections {
        let hashes = "#".repeat(usize::from(sec.level.clamp(1, 6)));
        let _ = writeln!(md, "{hashes} {}\n", sec.heading);
        for item in &sec.items {
            let page_1 = item.page + 1;
            let quote = item.quoted_text.trim();
            let note = item.note.trim();
            if !quote.is_empty() {
                let _ = write!(md, "- \"{quote}\" *(p. {page_1})*");
            } else {
                let _ = write!(md, "- **{}: ** *(p. {page_1})*", item.kind.display_name());
            }
            if detail == SummaryDetail::Full {
                if !note.is_empty() {
                    let _ = write!(md, " — Note: {note}");
                }
                if !item.tags.is_empty() {
                    let tags_fmt: Vec<String> = item.tags.iter().map(|t| format!("#{t}")).collect();
                    let _ = write!(md, " `[{}]`", tags_fmt.join(", "));
                }
            }
            md.push('\n');
        }
        md.push('\n');
    }

    SummaryDocument {
        title: doc_title.to_owned(),
        sections,
        markdown: md,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_summary_groups_highlights_by_heading_in_reading_order() {
        let headings = vec![
            HeadingAnchor {
                title: "1. Introduction".into(),
                level: 1,
                page: 0,
                y_pt: 50.0,
            },
            HeadingAnchor {
                title: "2. Results".into(),
                level: 1,
                page: 2,
                y_pt: 80.0,
            },
        ];
        // Intentionally supply highlights out of reading order.
        let highlights = vec![
            HighlightItem {
                id: "h2".into(),
                page: 2,
                y_pt: 150.0,
                x_pt: 72.0,
                kind: StudyNoteKind::Highlight,
                quoted_text: "Accuracy improved by 14%.".into(),
                note: "Key metric".into(),
                tags: vec!["result".into()],
            },
            HighlightItem {
                id: "h1".into(),
                page: 0,
                y_pt: 120.0,
                x_pt: 72.0,
                kind: StudyNoteKind::Highlight,
                quoted_text: "We propose an offline-first architecture.".into(),
                note: String::new(),
                tags: Vec::new(),
            },
        ];

        let summary = build_offline_summary("Paper A", &headings, &highlights, SummaryDetail::Full);
        assert_eq!(summary.sections.len(), 2);
        assert_eq!(summary.sections[0].heading, "1. Introduction");
        assert_eq!(summary.sections[0].items[0].id, "h1");
        assert_eq!(summary.sections[1].heading, "2. Results");
        assert_eq!(summary.sections[1].items[0].id, "h2");
        assert!(summary.markdown.contains("# 1. Introduction"));
        assert!(summary.markdown.contains("*(p. 3)* — Note: Key metric"));
    }
}
