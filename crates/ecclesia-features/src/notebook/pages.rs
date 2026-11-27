//! Page insertion anywhere (plan N.2) and margin expansion / restoration (plan N.3).

use ecclesia_core::notebook::{
    LinkDestination, Notebook, NotebookError, Orientation, OriginalBoxes, PageMeta, PageSize,
    PaperTemplateId, Result, pdf_ops,
};
use ecclesia_mupdf::{Document, PageBox};

use crate::notebook::paper::PaperSpec;

/// Where a new page is inserted relative to a reference page (N.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertPosition {
    /// Insert immediately before `page_index`.
    Before(usize),
    /// Insert immediately after `page_index`.
    After(usize),
}

impl InsertPosition {
    /// Converts this relative position into a concrete zero-based insertion index for a document
    /// with `page_count` pages.
    #[must_use]
    pub fn to_index(self, page_count: usize) -> usize {
        match self {
            Self::Before(p) => p.min(page_count),
            Self::After(p) => p.saturating_add(1).min(page_count),
        }
    }
}

/// What kind of page to insert (N.2).
#[derive(Debug, Clone, PartialEq)]
pub enum InsertPageSource {
    /// A blank page with explicit size and orientation.
    Blank {
        /// Page size.
        size: PageSize,
        /// Page orientation.
        orientation: Orientation,
    },
    /// A page backed by a procedural paper template (N.1).
    Template {
        /// Paper template parameters.
        spec: PaperSpec,
        /// Page size.
        size: PageSize,
        /// Page orientation.
        orientation: Orientation,
    },
    /// Copy the dimensions of the reference page (falling back to A4 if empty).
    CopyCurrentSize {
        /// Procedural template assigned to the new page metadata.
        template: PaperTemplateId,
    },
}

/// Inserts a new page into raw PDF bytes at `position`, returning `(updated_pdf, inserted_index)` (N.2).
///
/// Outline entries pointing to existing pages keep pointing to their exact original pages.
///
/// # Errors
///
/// Returns an error if `pdf_bytes` cannot be opened or modified.
pub fn insert_page_in_pdf(
    pdf_bytes: &[u8],
    position: InsertPosition,
    source: &InsertPageSource,
) -> Result<(Vec<u8>, usize)> {
    let doc = Document::from_bytes(pdf_bytes, "application/pdf")?;
    let count = doc.page_count()?;
    let insert_idx = position.to_index(count);
    let ref_page = match position {
        InsertPosition::Before(p) | InsertPosition::After(p) => p,
    };

    let (width, height, content) = match source {
        InsertPageSource::Blank { size, orientation } => {
            let (w, h) = size.dimensions(*orientation);
            (w, h, String::new())
        }
        InsertPageSource::Template {
            spec,
            size,
            orientation,
        } => {
            let (w, h) = size.dimensions(*orientation);
            (w, h, spec.to_pdf_content_stream(w, h))
        }
        InsertPageSource::CopyCurrentSize { .. } => {
            if ref_page < count {
                let rect = doc.page_box(ref_page, PageBox::Media)?;
                (rect.x1 - rect.x0, rect.y1 - rect.y0, String::new())
            } else {
                let (w, h) = PageSize::A4.dimensions(Orientation::Portrait);
                (w, h, String::new())
            }
        }
    };

    let updated = pdf_ops::insert_page(pdf_bytes, insert_idx, width, height, &content)?;
    Ok((updated, insert_idx))
}

/// Inserts a new page into a [`Notebook`] at `position`, updating `document.pdf`, `pages` metadata,
/// `sections` ranges, and internal `links` targets atomically in memory (N.2).
///
/// # Errors
///
/// Returns an error if the underlying PDF cannot be updated.
pub fn insert_page_in_notebook(
    notebook: &mut Notebook,
    position: InsertPosition,
    source: &InsertPageSource,
) -> Result<usize> {
    let template_id = match source {
        InsertPageSource::Blank { .. } => PaperTemplateId::Blank,
        InsertPageSource::Template { spec, .. } => spec.template,
        InsertPageSource::CopyCurrentSize { template } => *template,
    };

    // For notebooks, templates render procedurally on screen (zero file bloat) and bake on export.
    let pdf_source = match source {
        InsertPageSource::Template {
            size, orientation, ..
        } => InsertPageSource::Blank {
            size: *size,
            orientation: *orientation,
        },
        other => other.clone(),
    };

    let (updated_pdf, insert_idx) =
        insert_page_in_pdf(notebook.pdf_bytes(), position, &pdf_source)?;
    notebook.replace_pdf_bytes(updated_pdf)?;

    // Shift metadata pages so the new PageMeta lands at insert_idx.
    let new_meta = PageMeta::new(template_id);
    if notebook.metadata.pages.len() > insert_idx {
        // replace_pdf_bytes appended a placeholder at the end; move our new metadata into place.
        notebook.metadata.pages.pop();
        notebook.metadata.pages.insert(insert_idx, new_meta);
    } else {
        notebook.metadata.pages.push(new_meta);
    }

    // Keep section start pages and counts consistent.
    for sec in &mut notebook.metadata.sections {
        if insert_idx <= sec.start_page {
            sec.start_page += 1;
        } else if insert_idx < sec.start_page + sec.page_count {
            sec.page_count += 1;
        }
    }

    // Keep internal link source and destination page indices accurate.
    for link in &mut notebook.metadata.links {
        if link.source_page >= insert_idx {
            link.source_page += 1;
        }
        if let LinkDestination::Page { ref mut page, .. } = link.destination
            && *page >= insert_idx
        {
            *page += 1;
        }
    }

    Ok(insert_idx)
}

/// Margin expansion amounts in points on each side of a page (plan N.3).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MarginExpansion {
    /// Additional space added to the left of the page, in points.
    pub left_pt: f32,
    /// Additional space added to the right of the page, in points.
    pub right_pt: f32,
    /// Additional space added to the top of the page, in points.
    pub top_pt: f32,
    /// Additional space added to the bottom of the page, in points.
    pub bottom_pt: f32,
}

impl MarginExpansion {
    /// Convenience constructor for expanding only the right note margin.
    #[must_use]
    pub fn right(points: f32) -> Self {
        Self {
            right_pt: points.max(0.0),
            ..Self::default()
        }
    }

    /// Convenience constructor for symmetric horizontal margin expansion.
    #[must_use]
    pub fn horizontal(points: f32) -> Self {
        let p = points.max(0.0);
        Self {
            left_pt: p,
            right_pt: p,
            ..Self::default()
        }
    }

    /// Whether all four sides are zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.left_pt <= 0.0 && self.right_pt <= 0.0 && self.top_pt <= 0.0 && self.bottom_pt <= 0.0
    }
}

/// Expands `/MediaBox` and `/CropBox` on `page_index` of raw PDF bytes without shifting existing
/// content, storing the original boxes in `/EcclesiaPdfOrigBoxes` (N.3).
///
/// # Errors
///
/// Returns an error if `page_index` is invalid or the PDF cannot be modified.
pub fn expand_page_margins_pdf(
    pdf_bytes: &[u8],
    page_index: usize,
    expansion: MarginExpansion,
) -> Result<(Vec<u8>, OriginalBoxes)> {
    if expansion.is_zero() {
        return Err(NotebookError::InvalidFormat(
            "margin expansion must be positive on at least one side".into(),
        ));
    }
    pdf_ops::expand_margins(
        pdf_bytes,
        page_index,
        expansion.left_pt,
        expansion.right_pt,
        expansion.top_pt,
        expansion.bottom_pt,
    )
}

/// Restores original `/MediaBox` and `/CropBox` on `page_index` of raw PDF bytes (N.3).
///
/// # Errors
///
/// Returns an error if `page_index` is invalid or the PDF cannot be modified.
pub fn remove_added_margins_pdf(pdf_bytes: &[u8], page_index: usize) -> Result<Vec<u8>> {
    pdf_ops::remove_added_margins(pdf_bytes, page_index)
}

/// Expands margins on `page_index` of a [`Notebook`], recording original boxes both in the PDF's
/// `/EcclesiaPdfOrigBoxes` key and in `notebook.json` (N.3).
///
/// # Errors
///
/// Returns an error if `page_index` is out of range or PDF modification fails.
pub fn expand_page_margins_notebook(
    notebook: &mut Notebook,
    page_index: usize,
    expansion: MarginExpansion,
) -> Result<OriginalBoxes> {
    let (updated, orig) = expand_page_margins_pdf(notebook.pdf_bytes(), page_index, expansion)?;
    notebook.replace_pdf_bytes(updated)?;
    if let Some(meta) = notebook.metadata.pages.get_mut(page_index)
        && meta.original_boxes.is_none()
    {
        meta.original_boxes = Some(orig);
    }
    Ok(orig)
}

/// Removes added margins on `page_index` of a [`Notebook`], restoring exact original MediaBox and
/// CropBox (N.3).
///
/// # Errors
///
/// Returns an error if `page_index` is out of range or PDF modification fails.
pub fn remove_added_margins_notebook(notebook: &mut Notebook, page_index: usize) -> Result<()> {
    let restored = remove_added_margins_pdf(notebook.pdf_bytes(), page_index)?;
    notebook.replace_pdf_bytes(restored)?;
    if let Some(meta) = notebook.metadata.pages.get_mut(page_index) {
        meta.original_boxes = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::annot::{Annotation, Boxed, Shape, Style};
    use ecclesia_core::notebook::{OutlineEntrySpec, PdfBuilder, PdfPageInput};
    use ecclesia_mupdf::PageRect;

    #[test]
    fn inserted_pages_show_in_mupdf_and_outline_points_to_right_pages() {
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 700 Td (Intro) Tj ET".into(),
            crop_box: None,
        });
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 700 Td (Methods) Tj ET".into(),
            crop_box: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: "Intro".into(),
            page: 0,
            y_pt: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: "Methods".into(),
            page: 1,
            y_pt: None,
        });
        let base = builder.build().expect("base");

        // Insert a Cornell template page BEFORE page 1 ("Methods").
        let (updated, idx) = insert_page_in_pdf(
            &base,
            InsertPosition::Before(1),
            &InsertPageSource::Template {
                spec: PaperSpec::for_template(PaperTemplateId::Cornell),
                size: PageSize::Letter,
                orientation: Orientation::Portrait,
            },
        )
        .expect("insert");
        assert_eq!(idx, 1);

        let doc = Document::from_bytes(&updated, "application/pdf").expect("mupdf");
        assert_eq!(doc.page_count().expect("pages"), 3);
        let outline = doc.outline().expect("outline");
        assert_eq!(outline[0].title, "Intro");
        assert_eq!(outline[0].page, Some(0));
        assert_eq!(outline[1].title, "Methods");
        assert_eq!(
            outline[1].page,
            Some(2),
            "Methods outline entry must follow its page to index 2"
        );
        let text2 = doc
            .load_page(2)
            .expect("p2")
            .text(None)
            .expect("stext")
            .text()
            .expect("text");
        assert!(text2.contains("Methods"));
    }

    #[test]
    fn expand_margins_allows_margin_notes_and_remove_restores_exact_boxes() {
        let mut nb = Notebook::new_blank(
            "Margin Study",
            PageSize::Letter,
            Orientation::Portrait,
            PaperTemplateId::Blank,
        )
        .expect("nb");

        let orig = expand_page_margins_notebook(
            &mut nb,
            0,
            MarginExpansion {
                left_pt: 0.0,
                right_pt: 200.0,
                top_pt: 20.0,
                bottom_pt: 0.0,
            },
        )
        .expect("expand");
        assert_eq!(orig.media_box, [0.0, 0.0, 612.0, 792.0]);

        // Verify a note annotation placed in the newly expanded right margin sits cleanly inside the page.
        let mut doc = nb.open_mupdf().expect("open");
        let media = doc.page_box(0, PageBox::Media).expect("media");
        assert!(((media.x1 - media.x0) - 812.0).abs() < 0.05);
        assert!(((media.y1 - media.y0) - 812.0).abs() < 0.05);

        let mut note = Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 660.0,
                    y0: 120.0,
                    x1: 684.0,
                    y1: 144.0,
                },
            ),
            Style::default(),
            "Ada",
        );
        note.contents = "Note written in expanded right margin".into();
        doc.operation("Add margin note", |edit| {
            edit.create_annot(0, &note.to_data())
        })
        .expect("add annot");
        let page0 = doc.load_page(0).expect("p0");
        let annots = page0.annots().expect("annots");
        assert_eq!(annots.len(), 1);
        assert!(annots[0].rect().expect("rect").x0 >= 612.0);
        drop(annots);
        drop(page0);

        // Now remove added margins and verify exact restoration.
        remove_added_margins_notebook(&mut nb, 0).expect("remove margins");
        let doc2 = nb.open_mupdf().expect("reopen");
        let media2 = doc2.page_box(0, PageBox::Media).expect("media2");
        let crop2 = doc2.page_box(0, PageBox::Crop).expect("crop2");
        assert!(((media2.x1 - media2.x0) - 612.0).abs() < 0.01);
        assert!(((media2.y1 - media2.y0) - 792.0).abs() < 0.01);
        assert!(((crop2.x1 - crop2.x0) - 612.0).abs() < 0.01);
        assert!(((crop2.y1 - crop2.y0) - 792.0).abs() < 0.01);
        assert!(nb.metadata.pages[0].original_boxes.is_none());
    }
}
