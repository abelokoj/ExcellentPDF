//! Notebook covers and colored sections (plan N.8).
//!
//! Covers can use a built-in template or a bundled image asset. Sections map consecutive page
//! ranges to named colored tabs beside the page, export cleanly as standard PDF `/Outlines`
//! bookmarks, and support drag-reordering by rearranging the underlying PDF pages.

use excellent_core::anchor::new_id;
use excellent_core::notebook::{
    CoverMeta, CoverStyle, LinkDestination, Notebook, NotebookError, OutlineEntrySpec, Result,
    SectionMeta, pdf_ops,
};

/// Visual tab entry laid out along the edge of the page for the section tab strip (N.8).
#[derive(Debug, Clone, PartialEq)]
pub struct SectionTab {
    /// Section identifier.
    pub section_id: String,
    /// Section display title.
    pub title: String,
    /// Tab RGB color.
    pub color: [u8; 3],
    /// Zero-based first page of this section.
    pub start_page: usize,
    /// Normalized vertical start `[0.0..=1.0]` along the tab strip.
    pub y_start_norm: f32,
    /// Normalized vertical end `[0.0..=1.0]` along the tab strip.
    pub y_end_norm: f32,
}

/// Sets or replaces the cover page metadata on `notebook` (N.8).
pub fn set_cover(
    notebook: &mut Notebook,
    style: CoverStyle,
    title: impl Into<String>,
    subtitle: Option<String>,
    accent_rgb: [u8; 3],
    image_asset: Option<String>,
) {
    notebook.metadata.cover = Some(CoverMeta {
        style,
        title: title.into(),
        subtitle,
        accent_rgb,
        image_asset,
    });
}

/// Adds a new section spanning `[start_page .. start_page + page_count)` to `notebook`.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the page range is empty or exceeds the notebook's
/// page count.
pub fn add_section(
    notebook: &mut Notebook,
    title: impl Into<String>,
    color: [u8; 3],
    start_page: usize,
    page_count: usize,
) -> Result<SectionMeta> {
    let total = notebook.metadata.pages.len();
    if page_count == 0 || start_page + page_count > total {
        return Err(NotebookError::InvalidFormat(format!(
            "section range {start_page}..{} exceeds page count {total}",
            start_page + page_count
        )));
    }
    let section = SectionMeta {
        id: new_id(),
        title: title.into(),
        color,
        start_page,
        page_count,
    };
    for page in &mut notebook.metadata.pages[start_page..start_page + page_count] {
        page.section_id = Some(section.id.clone());
    }
    notebook.metadata.sections.push(section.clone());
    notebook.metadata.sections.sort_by_key(|sec| sec.start_page);
    Ok(section)
}

/// Computes evenly distributed colored tab strip geometry for all sections (N.8).
#[must_use]
pub fn tab_strip(sections: &[SectionMeta]) -> Vec<SectionTab> {
    if sections.is_empty() {
        return Vec::new();
    }
    let step = 1.0 / sections.len() as f32;
    sections
        .iter()
        .enumerate()
        .map(|(i, sec)| SectionTab {
            section_id: sec.id.clone(),
            title: sec.title.clone(),
            color: sec.color,
            start_page: sec.start_page,
            y_start_norm: i as f32 * step,
            y_end_norm: (i + 1) as f32 * step,
        })
        .collect()
}

/// Builds PDF outline bookmark specifications from notebook sections (N.8).
#[must_use]
pub fn outline_entries(sections: &[SectionMeta]) -> Vec<OutlineEntrySpec> {
    sections
        .iter()
        .map(|sec| OutlineEntrySpec {
            title: sec.title.clone(),
            page: sec.start_page,
            y_pt: None,
        })
        .collect()
}

/// Reorders section at `from_index` to `to_index` by rearranging the underlying PDF pages,
/// reordering `metadata.pages`, updating section start pages, remapping internal links, and
/// updating the PDF `/Outlines` bookmarks (N.8 step 2).
///
/// Returns the applied page permutation (`new_pos -> old_pos`).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if indices are out of bounds, or an error if PDF page
/// rearrangement fails.
pub fn reorder_section(
    notebook: &mut Notebook,
    from_index: usize,
    to_index: usize,
) -> Result<Vec<usize>> {
    let sec_len = notebook.metadata.sections.len();
    if from_index >= sec_len || to_index >= sec_len {
        return Err(NotebookError::NotFound(format!(
            "section index out of range (from={from_index}, to={to_index}, len={sec_len})"
        )));
    }
    let page_count = notebook.metadata.pages.len();
    if from_index == to_index {
        return Ok((0..page_count).collect());
    }

    // Build chunks: pages before any section, each section's page range, and trailing unassigned pages.
    let mut sections_reordered = notebook.metadata.sections.clone();
    let moved = sections_reordered.remove(from_index);
    sections_reordered.insert(to_index, moved);

    // Collect pages owned by sections in their new order, keeping non-section pages (e.g., cover
    // at page 0) anchored at the front and any trailing loose pages at the back.
    let mut assigned = vec![false; page_count];
    for sec in &notebook.metadata.sections {
        for slot in assigned
            .iter_mut()
            .take((sec.start_page + sec.page_count).min(page_count))
            .skip(sec.start_page)
        {
            *slot = true;
        }
    }

    let first_section_page = notebook
        .metadata
        .sections
        .iter()
        .map(|s| s.start_page)
        .min()
        .unwrap_or(0);

    let mut permutation = Vec::with_capacity(page_count);
    // Leading pages before the first section (such as cover page 0).
    for (p, &is_assigned) in assigned.iter().enumerate().take(first_section_page) {
        if !is_assigned {
            permutation.push(p);
        }
    }
    // Reordered sections.
    let mut updated_sections = Vec::with_capacity(sections_reordered.len());
    for mut sec in sections_reordered {
        let new_start = permutation.len();
        for p in sec.start_page..(sec.start_page + sec.page_count).min(page_count) {
            permutation.push(p);
        }
        sec.start_page = new_start;
        updated_sections.push(sec);
    }
    // Any remaining unassigned pages.
    for (p, &is_assigned) in assigned.iter().enumerate().skip(first_section_page) {
        if !is_assigned {
            permutation.push(p);
        }
    }

    // Build inverse mapping old_page -> new_page.
    let mut old_to_new = vec![0usize; page_count];
    for (new_idx, &old_idx) in permutation.iter().enumerate() {
        old_to_new[old_idx] = new_idx;
    }

    // Rearrange PDF pages and refresh PDF outlines.
    let rearranged_pdf = pdf_ops::rearrange_pages(notebook.pdf_bytes(), &permutation)?;
    let outlined_pdf = pdf_ops::set_outlines(&rearranged_pdf, &outline_entries(&updated_sections))?;
    notebook.replace_pdf_bytes(outlined_pdf)?;

    // Rearrange metadata.pages in the same permutation.
    let old_pages = notebook.metadata.pages.clone();
    notebook.metadata.pages = permutation.iter().map(|&i| old_pages[i].clone()).collect();
    notebook.metadata.sections = updated_sections;

    // Remap internal links to follow their moved pages.
    for link in &mut notebook.metadata.links {
        if link.source_page < old_to_new.len() {
            link.source_page = old_to_new[link.source_page];
        }
        if let LinkDestination::Page { ref mut page, .. } = link.destination
            && *page < old_to_new.len()
        {
            *page = old_to_new[*page];
        }
    }

    Ok(permutation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::notebook::{ExportOptions, PaperTemplateId, PdfBuilder, PdfPageInput};
    use excellent_mupdf::Document;

    #[test]
    fn sections_export_as_pdf_bookmarks_and_reorder_moves_pages() {
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        for label in ["Cover", "SecA-1", "SecA-2", "SecB-1"] {
            builder.add_page(PdfPageInput {
                width: 612.0,
                height: 792.0,
                content: format!("BT /F1 18 Tf 72 700 Td ({label}) Tj ET"),
                crop_box: None,
            });
        }
        let pdf = builder.build().expect("pdf");
        let mut nb = Notebook::from_pdf_bytes("Biology", pdf, PaperTemplateId::Lined).expect("nb");
        set_cover(
            &mut nb,
            CoverStyle::Linen,
            "Biology Notes",
            Some("Fall 2026".into()),
            [0x1d, 0x4e, 0xd8],
            None,
        );

        add_section(&mut nb, "Cells", [0x16, 0xa3, 0x4a], 1, 2).expect("sec A");
        add_section(&mut nb, "Genetics", [0xdc, 0x26, 0x26], 3, 1).expect("sec B");
        assert_eq!(tab_strip(&nb.metadata.sections).len(), 2);

        // Drag "Genetics" (index 1) before "Cells" (index 0). Cover (page 0) stays page 0!
        let perm = reorder_section(&mut nb, 1, 0).expect("reorder");
        assert_eq!(perm, vec![0, 3, 1, 2]);
        assert_eq!(nb.metadata.sections[0].title, "Genetics");
        assert_eq!(nb.metadata.sections[0].start_page, 1);
        assert_eq!(nb.metadata.sections[1].title, "Cells");
        assert_eq!(nb.metadata.sections[1].start_page, 2);

        let exported = nb
            .export_pdf(ExportOptions::default(), |_, _, _, _| String::new())
            .expect("export");
        let doc = Document::from_bytes(&exported, "application/pdf").expect("open");
        let outline = doc.outline().expect("outline");
        assert_eq!(outline.len(), 2);
        assert_eq!(outline[0].title, "Genetics");
        assert_eq!(outline[0].page, Some(1));
        assert_eq!(outline[1].title, "Cells");
        assert_eq!(outline[1].page, Some(2));

        let p1_text = doc
            .load_page(1)
            .expect("p1")
            .text(None)
            .expect("st")
            .text()
            .expect("t");
        assert!(
            p1_text.contains("SecB-1"),
            "Genetics page moved to index 1: {p1_text}"
        );
        let p2_text = doc
            .load_page(2)
            .expect("p2")
            .text(None)
            .expect("st")
            .text()
            .expect("t");
        assert!(
            p2_text.contains("SecA-1"),
            "Cells page moved to index 2: {p2_text}"
        );
    }
}
