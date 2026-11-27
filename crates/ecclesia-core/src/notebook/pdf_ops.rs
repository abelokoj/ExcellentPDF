//! Pure-Rust PDF 1.7 builder and structural editor for notebook operations (N.1–N.4, N.8, N.9, L.3).
//!
//! Produces standard PDF 1.7 files and incremental PDF revisions verified against MuPDF. Supports
//! page creation, page insertion with outline preservation, margin expansion/restoration with
//! `/EcclesiaPdfOrigBoxes`, page rearrangement, `/Outlines` bookmarks, `/Link` annotations,
//! `/EmbeddedFiles` attachments, vector content underlays, and vector Form XObject slide condensation.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;

use ecclesia_mupdf::{PagePoint, PageRect};

use crate::notebook::model::{NotebookError, OriginalBoxes, Result};

/// Private PDF page dictionary key storing original MediaBox and CropBox before margin expansion (N.3).
pub const ORIG_BOXES_KEY: &str = "EcclesiaPdfOrigBoxes";

/// Input specification for a page built by [`PdfBuilder`].
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPageInput {
    /// Page width in points.
    pub width: f32,
    /// Page height in points.
    pub height: f32,
    /// PDF vector content stream operators (in PDF user space, origin bottom-left).
    pub content: String,
    /// Optional explicit CropBox `[x0, y0, x1, y1]`; defaults to `[0, 0, width, height]`.
    pub crop_box: Option<[f32; 4]>,
}

/// Specification for a PDF outline (bookmark) entry.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineEntrySpec {
    /// Bookmark display title.
    pub title: String,
    /// Zero-based destination page index.
    pub page: usize,
    /// Optional target y-coordinate in top-left page points.
    pub y_pt: Option<f32>,
}

/// Destination of a PDF `/Link` annotation.
#[derive(Debug, Clone, PartialEq)]
pub enum PdfLinkTarget {
    /// Internal page destination.
    InternalPage {
        /// Zero-based destination page index.
        page: usize,
        /// Optional target point in top-left page points.
        point: Option<PagePoint>,
        /// Optional zoom factor.
        zoom: Option<f32>,
    },
    /// External or deep-link URI.
    Uri(String),
}

/// Specification for a PDF `/Link` annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfLinkSpec {
    /// Zero-based source page index.
    pub source_page: usize,
    /// Clickable rectangle in top-left page points.
    pub rect: PageRect,
    /// Where the link navigates.
    pub target: PdfLinkTarget,
}

/// Placement slot for a condensed source slide page on an output page (L.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlideSlot {
    /// Zero-based index of the source slide page.
    pub source_page: usize,
    /// Target rectangle on the output page in PDF bottom-left coordinates `[x0, y0, x1, y1]`.
    pub target_pdf_rect: [f32; 4],
}

/// Builder for creating fresh standards-compliant PDF 1.7 files.
#[derive(Debug, Default)]
pub struct PdfBuilder {
    pages: Vec<PdfPageInput>,
    outlines: Vec<OutlineEntrySpec>,
    links: Vec<PdfLinkSpec>,
    attachments: Vec<(String, Vec<u8>)>,
    include_standard_fonts: bool,
}

impl PdfBuilder {
    /// Creates an empty PDF builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a page to the PDF.
    pub fn add_page(&mut self, page: PdfPageInput) -> &mut Self {
        self.pages.push(page);
        self
    }

    /// Enables standard Type1 fonts (`/F1 Helvetica`, `/F2 Courier`, `/F3 Times-Roman`) in page resources.
    pub fn enable_standard_fonts(&mut self) -> &mut Self {
        self.include_standard_fonts = true;
        self
    }

    /// Adds a bookmark outline entry.
    pub fn add_outline(&mut self, entry: OutlineEntrySpec) -> &mut Self {
        self.outlines.push(entry);
        self
    }

    /// Adds a link annotation.
    pub fn add_link(&mut self, link: PdfLinkSpec) -> &mut Self {
        self.links.push(link);
        self
    }

    /// Adds an embedded file attachment (`/EmbeddedFiles`).
    pub fn add_attachment(&mut self, name: impl Into<String>, bytes: Vec<u8>) -> &mut Self {
        self.attachments.push((name.into(), bytes));
        self
    }

    /// Builds a valid PDF 1.7 byte buffer.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if the builder has zero pages.
    pub fn build(&self) -> Result<Vec<u8>> {
        if self.pages.is_empty() {
            return Err(NotebookError::InvalidFormat(
                "cannot build a PDF with zero pages".into(),
            ));
        }

        let mut writer = RawPdfWriter::new();
        let catalog_id = writer.alloc();
        let pages_id = writer.alloc();

        let font_ids = if self.include_standard_fonts {
            let f1 = writer.alloc();
            let f2 = writer.alloc();
            let f3 = writer.alloc();
            writer.set(
                f1,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                    .to_vec(),
            );
            writer.set(
                f2,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>"
                    .to_vec(),
            );
            writer.set(
                f3,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>"
                    .to_vec(),
            );
            Some((f1, f2, f3))
        } else {
            None
        };

        let page_ids: Vec<u32> = (0..self.pages.len()).map(|_| writer.alloc()).collect();
        let content_ids: Vec<u32> = (0..self.pages.len()).map(|_| writer.alloc()).collect();

        // Group links per page.
        let mut page_annot_ids: Vec<Vec<u32>> = vec![Vec::new(); self.pages.len()];
        for link in &self.links {
            if link.source_page >= self.pages.len() {
                continue;
            }
            let page_h = self.pages[link.source_page].height;
            let annot_id = writer.alloc();
            let annot_body = format_link_annot(link, page_h, &page_ids, &self.pages);
            writer.set(annot_id, annot_body.into_bytes());
            page_annot_ids[link.source_page].push(annot_id);
        }

        // Write page contents and page dicts.
        for (idx, page) in self.pages.iter().enumerate() {
            let stream_bytes = make_stream_object(page.content.as_bytes(), None);
            writer.set(content_ids[idx], stream_bytes);

            let mut dict = format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}]",
                fmt_f32(page.width),
                fmt_f32(page.height)
            );
            if let Some(cb) = page.crop_box {
                let _ = write!(
                    dict,
                    " /CropBox [{} {} {} {}]",
                    fmt_f32(cb[0]),
                    fmt_f32(cb[1]),
                    fmt_f32(cb[2]),
                    fmt_f32(cb[3])
                );
            }
            let _ = write!(dict, " /Contents {} 0 R", content_ids[idx]);
            if let Some((f1, f2, f3)) = font_ids {
                let _ = write!(
                    dict,
                    " /Resources << /Font << /F1 {f1} 0 R /F2 {f2} 0 R /F3 {f3} 0 R >> >>"
                );
            } else {
                dict.push_str(" /Resources << >>");
            }
            if !page_annot_ids[idx].is_empty() {
                dict.push_str(" /Annots [");
                for &aid in &page_annot_ids[idx] {
                    let _ = write!(dict, " {aid} 0 R");
                }
                dict.push_str(" ]");
            }
            dict.push_str(" >>");
            writer.set(page_ids[idx], dict.into_bytes());
        }

        // Write /Pages node.
        let mut kids = String::new();
        for &pid in &page_ids {
            let _ = write!(kids, " {pid} 0 R");
        }
        writer.set(
            pages_id,
            format!(
                "<< /Type /Pages /Kids [{kids} ] /Count {} >>",
                page_ids.len()
            )
            .into_bytes(),
        );

        // Optional /Outlines.
        let outlines_id = write_outlines(&mut writer, &self.outlines, &page_ids, &self.pages);

        // Optional /Names /EmbeddedFiles.
        let names_id = write_embedded_files(&mut writer, &self.attachments);

        // Write /Catalog.
        let mut catalog = format!("<< /Type /Catalog /Pages {pages_id} 0 R");
        if let Some(oid) = outlines_id {
            let _ = write!(catalog, " /Outlines {oid} 0 R /PageMode /UseOutlines");
        }
        if let Some(nid) = names_id {
            let _ = write!(catalog, " /Names {nid} 0 R");
        }
        catalog.push_str(" >>");
        writer.set(catalog_id, catalog.into_bytes());

        Ok(writer.finish(catalog_id))
    }
}

/// Builds a minimal valid blank PDF with the given page dimensions `(width_pt, height_pt)`.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if `pages` is empty.
pub fn build_blank_pdf(pages: &[(f32, f32)]) -> Result<Vec<u8>> {
    let mut builder = PdfBuilder::new();
    for &(width, height) in pages {
        builder.add_page(PdfPageInput {
            width,
            height,
            content: String::new(),
            crop_box: None,
        });
    }
    builder.build()
}

/// Inserts a new page at `at_index` (clamped to `0..=page_count`) into an existing PDF, preserving
/// all existing pages and outline destinations via an incremental PDF revision.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the input PDF cannot be parsed.
pub fn insert_page(
    pdf_bytes: &[u8],
    at_index: usize,
    width: f32,
    height: f32,
    content_stream: &str,
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let insert_at = at_index.min(pages.len());

    let mut next_id = doc.next_obj_id();
    let new_page_id = next_id;
    next_id += 1;
    let new_content_id = next_id;
    next_id += 1;

    let pages_root_id = doc.pages_root_id()?;

    let mut updates: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    updates.insert(
        new_content_id,
        make_stream_object(content_stream.as_bytes(), None),
    );
    let page_dict = format!(
        "<< /Type /Page /Parent {pages_root_id} 0 R /MediaBox [0 0 {} {}] /Contents {new_content_id} 0 R /Resources << >> >>",
        fmt_f32(width),
        fmt_f32(height)
    );
    updates.insert(new_page_id, page_dict.into_bytes());

    // Flatten existing pages directly under pages_root_id so insertion at any index is exact,
    // ensuring every moved page preserves its effective MediaBox/CropBox/Resources and points
    // its /Parent to pages_root_id.
    let mut new_kids = pages.clone();
    new_kids.insert(insert_at, new_page_id);

    for &pid in &pages {
        if let Some(mut dict) = doc.get_dict(pid) {
            let needs_reparent = dict
                .get("Parent")
                .and_then(PdfVal::as_ref_id)
                .is_none_or(|p| p != pages_root_id);
            if needs_reparent {
                // Inherit MediaBox if missing on the leaf page before reparenting.
                if !dict.contains_key("MediaBox")
                    && let Ok(mb) = doc.effective_box(pid, "MediaBox")
                {
                    dict.insert("MediaBox".into(), pdf_rect_val(mb));
                }
                dict.insert("Parent".into(), PdfVal::Ref(pages_root_id, 0));
                updates.insert(pid, serialize_dict(&dict));
            }
        }
    }

    let mut kids_str = String::new();
    for &kid in &new_kids {
        let _ = write!(kids_str, " {kid} 0 R");
    }
    updates.insert(
        pages_root_id,
        format!(
            "<< /Type /Pages /Kids [{kids_str} ] /Count {} >>",
            new_kids.len()
        )
        .into_bytes(),
    );

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        next_id,
    ))
}

/// Expands `/MediaBox` and `/CropBox` of `page_index` on one or more sides (in points), keeping
/// existing page content at its exact coordinates. Stores original boxes in `/EcclesiaPdfOrigBoxes`
/// so [`remove_added_margins`] can restore them exactly (N.3).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if `page_index` is out of range, or
/// [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn expand_margins(
    pdf_bytes: &[u8],
    page_index: usize,
    left_pt: f32,
    right_pt: f32,
    top_pt: f32,
    bottom_pt: f32,
) -> Result<(Vec<u8>, OriginalBoxes)> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &page_id = pages
        .get(page_index)
        .ok_or_else(|| NotebookError::NotFound(format!("page {page_index} out of range")))?;

    let mut dict = doc
        .get_dict(page_id)
        .ok_or_else(|| NotebookError::InvalidFormat("page object is not a dictionary".into()))?;

    let cur_media = doc.effective_box(page_id, "MediaBox")?;
    let cur_crop = doc.effective_box(page_id, "CropBox").unwrap_or(cur_media);

    // Preserve the very first original boxes if already expanded once before.
    let orig = match dict.get(ORIG_BOXES_KEY).and_then(parse_orig_boxes_val) {
        Some(existing) => existing,
        None => OriginalBoxes {
            media_box: cur_media,
            crop_box: cur_crop,
        },
    };

    let new_media = [
        cur_media[0] - left_pt.max(0.0),
        cur_media[1] - bottom_pt.max(0.0),
        cur_media[2] + right_pt.max(0.0),
        cur_media[3] + top_pt.max(0.0),
    ];
    let new_crop = [
        cur_crop[0] - left_pt.max(0.0),
        cur_crop[1] - bottom_pt.max(0.0),
        cur_crop[2] + right_pt.max(0.0),
        cur_crop[3] + top_pt.max(0.0),
    ];

    dict.insert("MediaBox".into(), pdf_rect_val(new_media));
    dict.insert("CropBox".into(), pdf_rect_val(new_crop));
    dict.insert(
        ORIG_BOXES_KEY.into(),
        PdfVal::Array(vec![
            pdf_rect_val(orig.media_box),
            pdf_rect_val(orig.crop_box),
        ]),
    );

    let mut updates = BTreeMap::new();
    updates.insert(page_id, serialize_dict(&dict));

    let out = append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        doc.next_obj_id(),
    );
    Ok((out, orig))
}

/// Restores original `/MediaBox` and `/CropBox` from `/EcclesiaPdfOrigBoxes` on `page_index` (N.3).
/// If the page has no recorded margin expansion, returns a copy of `pdf_bytes` unchanged.
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if `page_index` is out of range, or
/// [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn remove_added_margins(pdf_bytes: &[u8], page_index: usize) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &page_id = pages
        .get(page_index)
        .ok_or_else(|| NotebookError::NotFound(format!("page {page_index} out of range")))?;

    let mut dict = doc
        .get_dict(page_id)
        .ok_or_else(|| NotebookError::InvalidFormat("page object is not a dictionary".into()))?;

    let Some(orig) = dict.get(ORIG_BOXES_KEY).and_then(parse_orig_boxes_val) else {
        return Ok(pdf_bytes.to_vec());
    };

    dict.insert("MediaBox".into(), pdf_rect_val(orig.media_box));
    dict.insert("CropBox".into(), pdf_rect_val(orig.crop_box));
    dict.remove(ORIG_BOXES_KEY);

    let mut updates = BTreeMap::new();
    updates.insert(page_id, serialize_dict(&dict));

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        doc.next_obj_id(),
    ))
}

/// Reads the stored original boxes from `/EcclesiaPdfOrigBoxes` on `page_index`, if present.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn read_original_boxes(pdf_bytes: &[u8], page_index: usize) -> Result<Option<OriginalBoxes>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let Some(&page_id) = pages.get(page_index) else {
        return Ok(None);
    };
    let Some(dict) = doc.get_dict(page_id) else {
        return Ok(None);
    };
    Ok(dict.get(ORIG_BOXES_KEY).and_then(parse_orig_boxes_val))
}

/// Rearranges the pages of `pdf_bytes` according to `new_order` (a permutation of `0..page_count`),
/// preserving indirect object identities so outline destinations follow their pages (N.8).
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if `new_order` is not a valid permutation or the PDF is invalid.
pub fn rearrange_pages(pdf_bytes: &[u8], new_order: &[usize]) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    if new_order.len() != pages.len() {
        return Err(NotebookError::InvalidFormat(format!(
            "rearrange length {} does not match page count {}",
            new_order.len(),
            pages.len()
        )));
    }
    let mut seen = vec![false; pages.len()];
    for &idx in new_order {
        if idx >= pages.len() || seen[idx] {
            return Err(NotebookError::InvalidFormat(
                "rearrange order is not a valid permutation".into(),
            ));
        }
        seen[idx] = true;
    }

    let pages_root_id = doc.pages_root_id()?;
    let mut updates = BTreeMap::new();

    for &pid in &pages {
        if let Some(mut dict) = doc.get_dict(pid)
            && dict
                .get("Parent")
                .and_then(PdfVal::as_ref_id)
                .is_none_or(|p| p != pages_root_id)
        {
            if !dict.contains_key("MediaBox")
                && let Ok(mb) = doc.effective_box(pid, "MediaBox")
            {
                dict.insert("MediaBox".into(), pdf_rect_val(mb));
            }
            dict.insert("Parent".into(), PdfVal::Ref(pages_root_id, 0));
            updates.insert(pid, serialize_dict(&dict));
        }
    }

    let mut kids_str = String::new();
    for &old_idx in new_order {
        let _ = write!(kids_str, " {} 0 R", pages[old_idx]);
    }
    updates.insert(
        pages_root_id,
        format!(
            "<< /Type /Pages /Kids [{kids_str} ] /Count {} >>",
            pages.len()
        )
        .into_bytes(),
    );

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        doc.next_obj_id(),
    ))
}

/// Writes an `/Outlines` bookmark tree into an existing PDF via an incremental update (N.8).
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn set_outlines(pdf_bytes: &[u8], entries: &[OutlineEntrySpec]) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    if entries.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }

    let mut next_id = doc.next_obj_id();
    let mut updates = BTreeMap::new();
    let outlines_root = next_id;
    next_id += 1;

    let valid_entries: Vec<&OutlineEntrySpec> =
        entries.iter().filter(|e| e.page < pages.len()).collect();
    if valid_entries.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }

    let item_ids: Vec<u32> = (0..valid_entries.len())
        .map(|_| {
            let id = next_id;
            next_id += 1;
            id
        })
        .collect();

    for (i, entry) in valid_entries.iter().enumerate() {
        let page_id = pages[entry.page];
        let mb = doc
            .effective_box(page_id, "MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let dest = match entry.y_pt {
            Some(y) => format!(
                "[{page_id} 0 R /XYZ {} {} 0]",
                fmt_f32(mb[0]),
                fmt_f32(mb[3] - y)
            ),
            None => format!("[{page_id} 0 R /Fit]"),
        };
        let mut dict = format!(
            "<< /Title ({}) /Parent {outlines_root} 0 R /Dest {dest}",
            escape_pdf_string(&entry.title)
        );
        if i > 0 {
            let _ = write!(dict, " /Prev {} 0 R", item_ids[i - 1]);
        }
        if i + 1 < item_ids.len() {
            let _ = write!(dict, " /Next {} 0 R", item_ids[i + 1]);
        }
        dict.push_str(" >>");
        updates.insert(item_ids[i], dict.into_bytes());
    }

    updates.insert(
        outlines_root,
        format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            item_ids[0],
            item_ids[item_ids.len() - 1],
            item_ids.len()
        )
        .into_bytes(),
    );

    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| NotebookError::InvalidFormat("catalog is not a dictionary".into()))?;
    catalog.insert("Outlines".into(), PdfVal::Ref(outlines_root, 0));
    catalog.insert("PageMode".into(), PdfVal::Name("UseOutlines".into()));
    updates.insert(doc.root_id, serialize_dict(&catalog));

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        next_id,
    ))
}

/// Appends `/Link` annotations onto pages of an existing PDF via an incremental update (N.9).
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn add_links(pdf_bytes: &[u8], links: &[PdfLinkSpec]) -> Result<Vec<u8>> {
    if links.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let mut next_id = doc.next_obj_id();
    let mut updates = BTreeMap::new();

    let mut per_page: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    for link in links {
        if link.source_page >= pages.len() {
            continue;
        }
        let src_mb = doc
            .effective_box(pages[link.source_page], "MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let x0 = src_mb[0] + link.rect.x0;
        let x1 = src_mb[0] + link.rect.x1;
        let y0 = src_mb[3] - link.rect.y1;
        let y1 = src_mb[3] - link.rect.y0;

        let target_part = match &link.target {
            PdfLinkTarget::InternalPage { page, point, zoom } => {
                let Some(&dest_page_id) = pages.get(*page) else {
                    continue;
                };
                let dest_mb = doc
                    .effective_box(dest_page_id, "MediaBox")
                    .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let dest = match point {
                    Some(pt) => format!(
                        "[{dest_page_id} 0 R /XYZ {} {} {}]",
                        fmt_f32(dest_mb[0] + pt.x),
                        fmt_f32(dest_mb[3] - pt.y),
                        fmt_f32(zoom.unwrap_or(0.0))
                    ),
                    None => format!("[{dest_page_id} 0 R /Fit]"),
                };
                format!("/Dest {dest}")
            }
            PdfLinkTarget::Uri(uri) => {
                format!("/A << /S /URI /URI ({}) >>", escape_pdf_string(uri))
            }
        };

        let annot_id = next_id;
        next_id += 1;
        let annot = format!(
            "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] {target_part} >>",
            fmt_f32(x0),
            fmt_f32(y0),
            fmt_f32(x1),
            fmt_f32(y1)
        );
        updates.insert(annot_id, annot.into_bytes());
        per_page.entry(link.source_page).or_default().push(annot_id);
    }

    for (page_idx, new_annots) in per_page {
        let page_id = pages[page_idx];
        let mut dict = doc.get_dict(page_id).ok_or_else(|| {
            NotebookError::InvalidFormat("page object is not a dictionary".into())
        })?;
        let mut annots = match dict.get("Annots") {
            Some(PdfVal::Array(arr)) => arr.clone(),
            Some(PdfVal::Ref(r, _)) => match doc.resolve_obj(*r) {
                Some(PdfVal::Array(arr)) => arr,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        for aid in new_annots {
            annots.push(PdfVal::Ref(aid, 0));
        }
        dict.insert("Annots".into(), PdfVal::Array(annots));
        updates.insert(page_id, serialize_dict(&dict));
    }

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        next_id,
    ))
}

/// Attaches files to `/Names /EmbeddedFiles` in an existing PDF via an incremental update (N.4 / O.25).
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn attach_files(pdf_bytes: &[u8], files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let mut next_id = doc.next_obj_id();
    let mut updates = BTreeMap::new();

    let mut name_pairs = Vec::new();
    for &(name, data) in files {
        let ef_stream_id = next_id;
        next_id += 1;
        let filespec_id = next_id;
        next_id += 1;

        updates.insert(
            ef_stream_id,
            make_stream_object(data, Some("/Type /EmbeddedFile")),
        );
        let esc = escape_pdf_string(name);
        updates.insert(
            filespec_id,
            format!(
                "<< /Type /Filespec /F ({esc}) /UF ({esc}) /EF << /F {ef_stream_id} 0 R /UF {ef_stream_id} 0 R >> >>"
            )
            .into_bytes(),
        );
        name_pairs.push((name.to_owned(), filespec_id));
    }
    name_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let ef_tree_id = next_id;
    next_id += 1;
    let mut names_arr = String::new();
    for (name, fsid) in &name_pairs {
        let _ = write!(names_arr, " ({}) {fsid} 0 R", escape_pdf_string(name));
    }
    updates.insert(
        ef_tree_id,
        format!("<< /Names [{names_arr} ] >>").into_bytes(),
    );

    let names_dict_id = next_id;
    next_id += 1;
    updates.insert(
        names_dict_id,
        format!("<< /EmbeddedFiles {ef_tree_id} 0 R >>").into_bytes(),
    );

    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| NotebookError::InvalidFormat("catalog is not a dictionary".into()))?;
    catalog.insert("Names".into(), PdfVal::Ref(names_dict_id, 0));
    updates.insert(doc.root_id, serialize_dict(&catalog));

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        next_id,
    ))
}

/// Prepends a vector content stream (e.g., procedural paper template) UNDER existing page content on
/// `page_index`, wrapping the underlay in `q ... Q` so graphics state is isolated (N.1 / N.4).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if `page_index` is out of range, or
/// [`NotebookError::InvalidFormat`] if the PDF cannot be parsed.
pub fn prepend_page_content(
    pdf_bytes: &[u8],
    page_index: usize,
    underlay_stream: &str,
) -> Result<Vec<u8>> {
    if underlay_stream.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &page_id = pages
        .get(page_index)
        .ok_or_else(|| NotebookError::NotFound(format!("page {page_index} out of range")))?;

    let mut dict = doc
        .get_dict(page_id)
        .ok_or_else(|| NotebookError::InvalidFormat("page object is not a dictionary".into()))?;

    let mut next_id = doc.next_obj_id();
    let underlay_id = next_id;
    next_id += 1;

    let wrapped = format!("q\n{underlay_stream}\nQ\n");
    let mut updates = BTreeMap::new();
    updates.insert(underlay_id, make_stream_object(wrapped.as_bytes(), None));

    let new_contents = match dict.get("Contents") {
        Some(PdfVal::Ref(r, g)) => {
            PdfVal::Array(vec![PdfVal::Ref(underlay_id, 0), PdfVal::Ref(*r, *g)])
        }
        Some(PdfVal::Array(arr)) => {
            let mut next = Vec::with_capacity(arr.len() + 1);
            next.push(PdfVal::Ref(underlay_id, 0));
            next.extend(arr.iter().cloned());
            PdfVal::Array(next)
        }
        _ => PdfVal::Ref(underlay_id, 0),
    };
    dict.insert("Contents".into(), new_contents);
    updates.insert(page_id, serialize_dict(&dict));

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.xref_offset,
        next_id,
    ))
}

/// Condenses source slide pages into vector output note pages (L.3).
///
/// Each referenced source slide page is embedded as a vector PDF Form XObject (`/Subtype /Form`)
/// preserving its `/Resources`, fonts, vector paths, and text operators so text stays 100%
/// selectable and searchable in every PDF reader.
///
/// Each tuple in `output_pages` is `(width_pt, height_pt, note_underlay_stream, slide_slots)`.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidFormat`] if `source_pdf_bytes` cannot be parsed or `output_pages` is empty.
pub fn condense_pages_vector(
    source_pdf_bytes: &[u8],
    output_pages: &[(f32, f32, String, Vec<SlideSlot>)],
) -> Result<Vec<u8>> {
    if output_pages.is_empty() {
        return Err(NotebookError::InvalidFormat(
            "condensed output must have at least one page".into(),
        ));
    }

    let src = ParsedPdf::parse(source_pdf_bytes)?;
    let src_pages = src.page_objects()?;

    let mut writer = RawPdfWriter::new();
    let catalog_id = writer.alloc();
    let pages_id = writer.alloc();

    // Standard font for optional slide labels/headers.
    let helv_id = writer.alloc();
    writer.set(
        helv_id,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );

    // Map source object ID -> cloned destination object ID (shared across Form XObjects).
    let mut cloned_objs: HashMap<u32, u32> = HashMap::new();
    // Map source page index -> (form_xobject_id, source_bbox).
    let mut form_cache: HashMap<usize, (u32, [f32; 4])> = HashMap::new();

    let mut out_page_ids = Vec::with_capacity(output_pages.len());

    for (out_w, out_h, underlay, slots) in output_pages {
        let mut xobject_refs = Vec::new();
        let mut content = String::new();
        if !underlay.is_empty() {
            content.push_str("q\n");
            content.push_str(underlay);
            content.push_str("\nQ\n");
        }

        for (slot_idx, slot) in slots.iter().enumerate() {
            let Some(&src_page_id) = src_pages.get(slot.source_page) else {
                continue;
            };
            let (form_id, bbox) = if let Some(&cached) = form_cache.get(&slot.source_page) {
                cached
            } else {
                let created =
                    build_form_xobject_for_page(&src, src_page_id, &mut writer, &mut cloned_objs)?;
                form_cache.insert(slot.source_page, created);
                created
            };

            let xobj_name = format!("Slide{slot_idx}");
            xobject_refs.push((xobj_name.clone(), form_id));

            let src_w = (bbox[2] - bbox[0]).max(1.0);
            let src_h = (bbox[3] - bbox[1]).max(1.0);
            let dst_w = (slot.target_pdf_rect[2] - slot.target_pdf_rect[0]).max(1.0);
            let dst_h = (slot.target_pdf_rect[3] - slot.target_pdf_rect[1]).max(1.0);

            let scale = (dst_w / src_w).min(dst_h / src_h);
            let drawn_w = src_w * scale;
            let drawn_h = src_h * scale;
            let tx = slot.target_pdf_rect[0] + (dst_w - drawn_w) * 0.5 - bbox[0] * scale;
            let ty = slot.target_pdf_rect[1] + (dst_h - drawn_h) * 0.5 - bbox[1] * scale;

            // Clip to slot, apply affine transform, invoke vector Form XObject, and draw thin frame.
            let _ = writeln!(
                content,
                "q\n{} {} {} {} re W n\n{} 0 0 {} {} {} cm\n/{xobj_name} Do\nQ",
                fmt_f32(slot.target_pdf_rect[0]),
                fmt_f32(slot.target_pdf_rect[1]),
                fmt_f32(dst_w),
                fmt_f32(dst_h),
                fmt_f32(scale),
                fmt_f32(scale),
                fmt_f32(tx),
                fmt_f32(ty)
            );
            let _ = writeln!(
                content,
                "q 0.72 0.74 0.78 RG 0.6 w {} {} {} {} re S Q",
                fmt_f32(slot.target_pdf_rect[0]),
                fmt_f32(slot.target_pdf_rect[1]),
                fmt_f32(dst_w),
                fmt_f32(dst_h)
            );
        }

        let content_id = writer.alloc();
        writer.set(content_id, make_stream_object(content.as_bytes(), None));

        let mut xobj_dict = String::new();
        for (name, fid) in &xobject_refs {
            let _ = write!(xobj_dict, " /{name} {fid} 0 R");
        }
        let page_id = writer.alloc();
        let page_dict = format!(
            "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Contents {content_id} 0 R /Resources << /Font << /F1 {helv_id} 0 R >> /XObject <<{xobj_dict} >> >> >>",
            fmt_f32(*out_w),
            fmt_f32(*out_h)
        );
        writer.set(page_id, page_dict.into_bytes());
        out_page_ids.push(page_id);
    }

    let mut kids = String::new();
    for &pid in &out_page_ids {
        let _ = write!(kids, " {pid} 0 R");
    }
    writer.set(
        pages_id,
        format!(
            "<< /Type /Pages /Kids [{kids} ] /Count {} >>",
            out_page_ids.len()
        )
        .into_bytes(),
    );
    writer.set(
        catalog_id,
        format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes(),
    );

    Ok(writer.finish(catalog_id))
}

fn build_form_xobject_for_page(
    src: &ParsedPdf<'_>,
    src_page_id: u32,
    writer: &mut RawPdfWriter,
    cloned_objs: &mut HashMap<u32, u32>,
) -> Result<(u32, [f32; 4])> {
    let bbox = src
        .effective_box(src_page_id, "CropBox")
        .or_else(|_| src.effective_box(src_page_id, "MediaBox"))?;

    let page_dict = src
        .get_dict(src_page_id)
        .ok_or_else(|| NotebookError::InvalidFormat("source slide page is not a dict".into()))?;

    // Resolve and deep-clone /Resources (inheriting from parent if needed).
    let res_val = src
        .effective_resources(src_page_id)
        .unwrap_or_else(|| PdfVal::Dict(BTreeMap::new()));
    let remapped_res = clone_pdf_val(src, &res_val, writer, cloned_objs)?;

    // Gather content stream(s).
    let (stream_bytes, extra_filter) = extract_page_content_bytes(src, &page_dict)?;

    let form_id = writer.alloc();
    let mut form_dict: BTreeMap<String, PdfVal> = BTreeMap::new();
    form_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
    form_dict.insert("Subtype".into(), PdfVal::Name("Form".into()));
    form_dict.insert("FormType".into(), PdfVal::Int(1));
    form_dict.insert("BBox".into(), pdf_rect_val(bbox));
    form_dict.insert("Resources".into(), remapped_res);
    form_dict.insert("Length".into(), PdfVal::Int(stream_bytes.len() as i64));
    if let Some(filter_val) = extra_filter {
        form_dict.insert("Filter".into(), filter_val);
    }

    let mut raw = serialize_dict(&form_dict);
    raw.extend_from_slice(b"\nstream\n");
    raw.extend_from_slice(&stream_bytes);
    raw.extend_from_slice(b"\nendstream");
    writer.set(form_id, raw);

    Ok((form_id, bbox))
}

fn extract_page_content_bytes(
    src: &ParsedPdf<'_>,
    page_dict: &BTreeMap<String, PdfVal>,
) -> Result<(Vec<u8>, Option<PdfVal>)> {
    let Some(contents) = page_dict.get("Contents") else {
        return Ok((Vec::new(), None));
    };
    match contents {
        PdfVal::Ref(id, _) => match src.resolve_obj(*id) {
            Some(PdfVal::Stream { dict, data }) => Ok((data, dict.get("Filter").cloned())),
            Some(PdfVal::Array(arr)) => concat_content_streams(src, &arr),
            _ => Ok((Vec::new(), None)),
        },
        PdfVal::Array(arr) => concat_content_streams(src, arr),
        _ => Ok((Vec::new(), None)),
    }
}

fn concat_content_streams(
    src: &ParsedPdf<'_>,
    arr: &[PdfVal],
) -> Result<(Vec<u8>, Option<PdfVal>)> {
    if arr.len() == 1
        && let Some(id) = arr[0].as_ref_id()
        && let Some(PdfVal::Stream { dict, data }) = src.resolve_obj(id)
    {
        return Ok((data, dict.get("Filter").cloned()));
    }
    // Concatenate uncompressed streams; if a stream is Flate-encoded, keep single stream if possible.
    let mut combined = Vec::new();
    for item in arr {
        if let Some(id) = item.as_ref_id()
            && let Some(PdfVal::Stream { dict, data }) = src.resolve_obj(id)
        {
            if dict.contains_key("Filter") && combined.is_empty() && arr.len() == 1 {
                return Ok((data, dict.get("Filter").cloned()));
            }
            if !combined.is_empty() {
                combined.push(b'\n');
            }
            combined.extend_from_slice(&data);
        }
    }
    Ok((combined, None))
}

fn clone_pdf_val(
    src: &ParsedPdf<'_>,
    val: &PdfVal,
    writer: &mut RawPdfWriter,
    cloned: &mut HashMap<u32, u32>,
) -> Result<PdfVal> {
    match val {
        PdfVal::Ref(src_id, _) => {
            if let Some(&dst_id) = cloned.get(src_id) {
                return Ok(PdfVal::Ref(dst_id, 0));
            }
            let dst_id = writer.alloc();
            cloned.insert(*src_id, dst_id);
            if let Some(obj) = src.resolve_obj(*src_id) {
                let remapped = clone_pdf_val(src, &obj, writer, cloned)?;
                writer.set(dst_id, serialize_val(&remapped));
            } else {
                writer.set(dst_id, b"null".to_vec());
            }
            Ok(PdfVal::Ref(dst_id, 0))
        }
        PdfVal::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(clone_pdf_val(src, item, writer, cloned)?);
            }
            Ok(PdfVal::Array(out))
        }
        PdfVal::Dict(map) => {
            let mut out = BTreeMap::new();
            for (k, v) in map {
                out.insert(k.clone(), clone_pdf_val(src, v, writer, cloned)?);
            }
            Ok(PdfVal::Dict(out))
        }
        PdfVal::Stream { dict, data } => {
            let mut out = BTreeMap::new();
            for (k, v) in dict {
                out.insert(k.clone(), clone_pdf_val(src, v, writer, cloned)?);
            }
            out.insert("Length".into(), PdfVal::Int(data.len() as i64));
            Ok(PdfVal::Stream {
                dict: out,
                data: data.clone(),
            })
        }
        other => Ok(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// Low-level PDF writer helpers
// ---------------------------------------------------------------------------

struct RawPdfWriter {
    next_id: u32,
    objects: BTreeMap<u32, Vec<u8>>,
}

impl RawPdfWriter {
    fn new() -> Self {
        Self {
            next_id: 1,
            objects: BTreeMap::new(),
        }
    }

    fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn set(&mut self, id: u32, body: Vec<u8>) {
        self.objects.insert(id, body);
    }

    fn finish(&self, root_id: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

        let total = self.next_id as usize;
        let mut offsets = vec![0usize; total];
        for (&id, body) in &self.objects {
            offsets[id as usize] = out.len();
            let _ = writeln!(out, "{id} 0 obj");
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }

        let xref_pos = out.len();
        let _ = writeln!(out, "xref\n0 {total}");
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            if *offset == 0 {
                out.extend_from_slice(b"0000000000 65535 f \n");
            } else {
                let _ = writeln!(out, "{offset:010} 00000 n ");
            }
        }
        let _ = writeln!(
            out,
            "trailer\n<< /Size {total} /Root {root_id} 0 R >>\nstartxref\n{xref_pos}\n%%EOF"
        );
        out
    }
}

fn append_incremental(
    base_pdf: &[u8],
    updates: &BTreeMap<u32, Vec<u8>>,
    root_id: u32,
    prev_xref: usize,
    new_size: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(base_pdf.len() + updates.len() * 128 + 256);
    out.extend_from_slice(base_pdf);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }

    let mut offsets: BTreeMap<u32, usize> = BTreeMap::new();
    for (&id, body) in updates {
        offsets.insert(id, out.len());
        let _ = writeln!(out, "{id} 0 obj");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n");
    for (&id, &offset) in &offsets {
        let _ = writeln!(out, "{id} 1");
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }

    let _ = writeln!(
        out,
        "trailer\n<< /Size {new_size} /Root {root_id} 0 R /Prev {prev_xref} >>\nstartxref\n{xref_pos}\n%%EOF"
    );
    out
}

fn write_outlines(
    writer: &mut RawPdfWriter,
    entries: &[OutlineEntrySpec],
    page_ids: &[u32],
    pages: &[PdfPageInput],
) -> Option<u32> {
    let valid: Vec<&OutlineEntrySpec> =
        entries.iter().filter(|e| e.page < page_ids.len()).collect();
    if valid.is_empty() {
        return None;
    }
    let root = writer.alloc();
    let item_ids: Vec<u32> = (0..valid.len()).map(|_| writer.alloc()).collect();
    for (i, entry) in valid.iter().enumerate() {
        let pid = page_ids[entry.page];
        let dest = match entry.y_pt {
            Some(y) => format!(
                "[{pid} 0 R /XYZ 0 {} 0]",
                fmt_f32(pages[entry.page].height - y)
            ),
            None => format!("[{pid} 0 R /Fit]"),
        };
        let mut dict = format!(
            "<< /Title ({}) /Parent {root} 0 R /Dest {dest}",
            escape_pdf_string(&entry.title)
        );
        if i > 0 {
            let _ = write!(dict, " /Prev {} 0 R", item_ids[i - 1]);
        }
        if i + 1 < item_ids.len() {
            let _ = write!(dict, " /Next {} 0 R", item_ids[i + 1]);
        }
        dict.push_str(" >>");
        writer.set(item_ids[i], dict.into_bytes());
    }
    writer.set(
        root,
        format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            item_ids[0],
            item_ids[item_ids.len() - 1],
            item_ids.len()
        )
        .into_bytes(),
    );
    Some(root)
}

fn write_embedded_files(writer: &mut RawPdfWriter, files: &[(String, Vec<u8>)]) -> Option<u32> {
    if files.is_empty() {
        return None;
    }
    let mut pairs = Vec::with_capacity(files.len());
    for (name, data) in files {
        let ef_id = writer.alloc();
        let fs_id = writer.alloc();
        writer.set(ef_id, make_stream_object(data, Some("/Type /EmbeddedFile")));
        let esc = escape_pdf_string(name);
        writer.set(
            fs_id,
            format!(
                "<< /Type /Filespec /F ({esc}) /UF ({esc}) /EF << /F {ef_id} 0 R /UF {ef_id} 0 R >> >>"
            )
            .into_bytes(),
        );
        pairs.push((name.clone(), fs_id));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let ef_tree_id = writer.alloc();
    let mut arr = String::new();
    for (name, fs_id) in &pairs {
        let _ = write!(arr, " ({}) {fs_id} 0 R", escape_pdf_string(name));
    }
    writer.set(ef_tree_id, format!("<< /Names [{arr} ] >>").into_bytes());

    let names_id = writer.alloc();
    writer.set(
        names_id,
        format!("<< /EmbeddedFiles {ef_tree_id} 0 R >>").into_bytes(),
    );
    Some(names_id)
}

fn format_link_annot(
    link: &PdfLinkSpec,
    page_height: f32,
    page_ids: &[u32],
    pages: &[PdfPageInput],
) -> String {
    let x0 = link.rect.x0;
    let x1 = link.rect.x1;
    let y0 = page_height - link.rect.y1;
    let y1 = page_height - link.rect.y0;
    let target = match &link.target {
        PdfLinkTarget::InternalPage { page, point, zoom } => {
            if let Some(&dest_id) = page_ids.get(*page) {
                let dest_h = pages.get(*page).map_or(792.0, |p| p.height);
                match point {
                    Some(pt) => format!(
                        "/Dest [{dest_id} 0 R /XYZ {} {} {}]",
                        fmt_f32(pt.x),
                        fmt_f32(dest_h - pt.y),
                        fmt_f32(zoom.unwrap_or(0.0))
                    ),
                    None => format!("/Dest [{dest_id} 0 R /Fit]"),
                }
            } else {
                String::new()
            }
        }
        PdfLinkTarget::Uri(uri) => format!("/A << /S /URI /URI ({}) >>", escape_pdf_string(uri)),
    };
    format!(
        "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] {target} >>",
        fmt_f32(x0),
        fmt_f32(y0),
        fmt_f32(x1),
        fmt_f32(y1)
    )
}

fn make_stream_object(data: &[u8], extra_entries: Option<&str>) -> Vec<u8> {
    let header = match extra_entries {
        Some(extra) => format!("<< {extra} /Length {} >>\nstream\n", data.len()),
        None => format!("<< /Length {} >>\nstream\n", data.len()),
    };
    let mut buf = Vec::with_capacity(header.len() + data.len() + 12);
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(data);
    buf.extend_from_slice(b"\nendstream");
    buf
}

fn fmt_f32(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Escapes a string for use inside PDF literal parentheses `(...)`.
#[must_use]
pub fn escape_pdf_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii() => out.push(c),
            c => {
                // Emit UTF-8 bytes as octal escapes or keep ASCII fallback.
                let mut buf = [0u8; 4];
                for &b in c.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "\\{b:03o}");
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal PDF 1.7 parser (handles standard xref tables & incremental /Prev)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum PdfVal {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    HexStr(Vec<u8>),
    Array(Vec<PdfVal>),
    Dict(BTreeMap<String, PdfVal>),
    Ref(u32, u16),
    Stream {
        dict: BTreeMap<String, PdfVal>,
        data: Vec<u8>,
    },
}

impl PdfVal {
    fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Int(n) => Some(*n as f32),
            Self::Real(f) => Some(*f as f32),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(s) => Some(s),
            _ => None,
        }
    }

    fn as_ref_id(&self) -> Option<u32> {
        match self {
            Self::Ref(id, _) => Some(*id),
            _ => None,
        }
    }

    fn as_rect(&self) -> Option<[f32; 4]> {
        let Self::Array(arr) = self else {
            return None;
        };
        if arr.len() != 4 {
            return None;
        }
        Some([
            arr[0].as_f32()?,
            arr[1].as_f32()?,
            arr[2].as_f32()?,
            arr[3].as_f32()?,
        ])
    }
}

fn pdf_rect_val(r: [f32; 4]) -> PdfVal {
    PdfVal::Array(r.into_iter().map(|v| PdfVal::Real(f64::from(v))).collect())
}

fn parse_orig_boxes_val(val: &PdfVal) -> Option<OriginalBoxes> {
    let PdfVal::Array(arr) = val else {
        return None;
    };
    if arr.len() != 2 {
        return None;
    }
    Some(OriginalBoxes {
        media_box: arr[0].as_rect()?,
        crop_box: arr[1].as_rect()?,
    })
}

fn serialize_val(val: &PdfVal) -> Vec<u8> {
    let mut out = Vec::new();
    write_val(val, &mut out);
    out
}

fn serialize_dict(dict: &BTreeMap<String, PdfVal>) -> Vec<u8> {
    let mut out = Vec::new();
    write_dict(dict, &mut out);
    out
}

fn write_dict(dict: &BTreeMap<String, PdfVal>, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (k, v) in dict {
        out.extend_from_slice(b" /");
        out.extend_from_slice(k.as_bytes());
        out.push(b' ');
        write_val(v, out);
    }
    out.extend_from_slice(b" >>");
}

fn write_val(val: &PdfVal, out: &mut Vec<u8>) {
    match val {
        PdfVal::Null => out.extend_from_slice(b"null"),
        PdfVal::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        PdfVal::Int(n) => {
            let _ = write!(out, "{n}");
        }
        PdfVal::Real(f) => {
            out.extend_from_slice(fmt_f32(*f as f32).as_bytes());
        }
        PdfVal::Name(n) => {
            out.push(b'/');
            out.extend_from_slice(n.as_bytes());
        }
        PdfVal::Str(bytes) => {
            out.push(b'(');
            for &b in bytes {
                match b {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'(' => out.extend_from_slice(b"\\("),
                    b')' => out.extend_from_slice(b"\\)"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    other => out.push(other),
                }
            }
            out.push(b')');
        }
        PdfVal::HexStr(bytes) => {
            out.push(b'<');
            for &b in bytes {
                let _ = write!(out, "{b:02X}");
            }
            out.push(b'>');
        }
        PdfVal::Array(items) => {
            out.push(b'[');
            for item in items {
                out.push(b' ');
                write_val(item, out);
            }
            out.extend_from_slice(b" ]");
        }
        PdfVal::Dict(map) => write_dict(map, out),
        PdfVal::Ref(id, generation) => {
            let _ = write!(out, "{id} {generation} R");
        }
        PdfVal::Stream { dict, data } => {
            write_dict(dict, out);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream");
        }
    }
}

struct ParsedPdf<'a> {
    data: &'a [u8],
    xref: BTreeMap<u32, usize>,
    root_id: u32,
    size: u32,
    xref_offset: usize,
}

impl<'a> ParsedPdf<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        let startxref_pos = find_last_subslice(data, b"startxref")
            .ok_or_else(|| NotebookError::InvalidFormat("PDF missing startxref marker".into()))?;
        let mut cur = Cursor::new(data, startxref_pos + b"startxref".len());
        cur.skip_ws();
        let xref_offset = usize::try_from(
            cur.read_int()
                .ok_or_else(|| NotebookError::InvalidFormat("invalid startxref offset".into()))?,
        )
        .map_err(|_| NotebookError::InvalidFormat("negative startxref offset".into()))?;

        let mut xref: BTreeMap<u32, usize> = BTreeMap::new();
        let mut root_id: Option<u32> = None;
        let mut max_size: u32 = 1;
        let mut next_xref = Some(xref_offset);
        let mut guard = 0usize;

        while let Some(offset) = next_xref {
            guard += 1;
            if guard > 128 || offset >= data.len() {
                break;
            }
            let mut c = Cursor::new(data, offset);
            c.skip_ws();
            if c.starts_with(b"xref") {
                c.pos += 4;
                loop {
                    c.skip_ws();
                    if c.starts_with(b"trailer") || c.is_eof() {
                        break;
                    }
                    let Some(first) = c.read_int().and_then(|v| u32::try_from(v).ok()) else {
                        break;
                    };
                    c.skip_ws();
                    let Some(count) = c.read_int().and_then(|v| u32::try_from(v).ok()) else {
                        break;
                    };
                    for i in 0..count {
                        c.skip_ws();
                        let Some(obj_off) = c.read_int().and_then(|v| usize::try_from(v).ok())
                        else {
                            break;
                        };
                        c.skip_ws();
                        let _gen = c.read_int();
                        c.skip_ws();
                        let flag = c.next_byte().unwrap_or(b'f');
                        if flag == b'n' {
                            xref.entry(first + i).or_insert(obj_off);
                        }
                    }
                }
                c.skip_ws();
                if c.starts_with(b"trailer") {
                    c.pos += 7;
                    c.skip_ws();
                    if let Some(PdfVal::Dict(trailer)) = c.parse_val() {
                        if root_id.is_none() {
                            root_id = trailer.get("Root").and_then(PdfVal::as_ref_id);
                        }
                        if let Some(s) = trailer.get("Size").and_then(PdfVal::as_i64)
                            && let Ok(s32) = u32::try_from(s)
                        {
                            max_size = max_size.max(s32);
                        }
                        next_xref = trailer
                            .get("Prev")
                            .and_then(PdfVal::as_i64)
                            .and_then(|p| usize::try_from(p).ok());
                    } else {
                        next_xref = None;
                    }
                } else {
                    next_xref = None;
                }
            } else {
                // Fallback: scan indirect objects directly if xref stream is present.
                scan_indirect_objects(data, &mut xref, &mut root_id, &mut max_size);
                break;
            }
        }

        let root_id = root_id.ok_or_else(|| {
            NotebookError::InvalidFormat("PDF trailer missing /Root reference".into())
        })?;
        let highest = xref.keys().copied().max().unwrap_or(0) + 1;
        Ok(Self {
            data,
            xref,
            root_id,
            size: max_size.max(highest),
            xref_offset,
        })
    }

    fn next_obj_id(&self) -> u32 {
        self.size
    }

    fn resolve_obj(&self, id: u32) -> Option<PdfVal> {
        let &offset = self.xref.get(&id)?;
        let mut c = Cursor::new(self.data, offset);
        c.skip_ws();
        let _obj_num = c.read_int()?;
        c.skip_ws();
        let _gen_num = c.read_int()?;
        c.skip_ws();
        if !c.starts_with(b"obj") {
            return None;
        }
        c.pos += 3;
        c.skip_ws();
        let val = c.parse_val()?;
        if let PdfVal::Dict(dict) = val {
            c.skip_ws();
            if c.starts_with(b"stream") {
                c.pos += 6;
                if c.peek() == Some(b'\r') {
                    c.pos += 1;
                }
                if c.peek() == Some(b'\n') {
                    c.pos += 1;
                }
                let start = c.pos;
                let length = dict.get("Length").and_then(|v| match v {
                    PdfVal::Int(n) => usize::try_from(*n).ok(),
                    PdfVal::Ref(r, _) => self
                        .resolve_obj(*r)?
                        .as_i64()
                        .and_then(|n| usize::try_from(n).ok()),
                    _ => None,
                });
                let data = if let Some(len) = length
                    && start + len <= self.data.len()
                {
                    self.data[start..start + len].to_vec()
                } else if let Some(rel) = find_first_subslice(&self.data[start..], b"endstream") {
                    let mut slice = &self.data[start..start + rel];
                    while slice.ends_with(b"\n") || slice.ends_with(b"\r") {
                        slice = &slice[..slice.len() - 1];
                    }
                    slice.to_vec()
                } else {
                    Vec::new()
                };
                return Some(PdfVal::Stream { dict, data });
            }
            return Some(PdfVal::Dict(dict));
        }
        Some(val)
    }

    fn get_dict(&self, id: u32) -> Option<BTreeMap<String, PdfVal>> {
        match self.resolve_obj(id)? {
            PdfVal::Dict(d) | PdfVal::Stream { dict: d, .. } => Some(d),
            _ => None,
        }
    }

    fn pages_root_id(&self) -> Result<u32> {
        let catalog = self
            .get_dict(self.root_id)
            .ok_or_else(|| NotebookError::InvalidFormat("PDF /Root catalog not found".into()))?;
        catalog
            .get("Pages")
            .and_then(PdfVal::as_ref_id)
            .ok_or_else(|| NotebookError::InvalidFormat("PDF /Catalog missing /Pages".into()))
    }

    fn page_objects(&self) -> Result<Vec<u32>> {
        let pages_root = self.pages_root_id()?;
        let mut out = Vec::new();
        self.collect_pages(pages_root, &mut out, 0)?;
        Ok(out)
    }

    fn collect_pages(&self, node_id: u32, out: &mut Vec<u32>, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(NotebookError::InvalidFormat(
                "page tree recursion limit".into(),
            ));
        }
        let dict = self
            .get_dict(node_id)
            .ok_or_else(|| NotebookError::InvalidFormat(format!("missing page node {node_id}")))?;
        match dict.get("Type").and_then(PdfVal::as_name) {
            Some("Page") => {
                out.push(node_id);
                Ok(())
            }
            Some("Pages") => {
                if let Some(kids_val) = dict.get("Kids") {
                    let kids = match kids_val {
                        PdfVal::Array(arr) => arr.clone(),
                        PdfVal::Ref(r, _) => match self.resolve_obj(*r) {
                            Some(PdfVal::Array(arr)) => arr,
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };
                    for kid in kids {
                        if let Some(kid_id) = kid.as_ref_id() {
                            self.collect_pages(kid_id, out, depth + 1)?;
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn effective_box(&self, page_id: u32, key: &str) -> Result<[f32; 4]> {
        let mut cur = Some(page_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(dict) = self.get_dict(id) else {
                break;
            };
            if let Some(val) = dict.get(key) {
                let resolved = match val {
                    PdfVal::Ref(r, _) => self.resolve_obj(*r),
                    other => Some(other.clone()),
                };
                if let Some(rect) = resolved.and_then(|v| v.as_rect()) {
                    return Ok(rect);
                }
            }
            cur = dict.get("Parent").and_then(PdfVal::as_ref_id);
        }
        Err(NotebookError::InvalidFormat(format!(
            "page {page_id} missing {key}"
        )))
    }

    fn effective_resources(&self, page_id: u32) -> Option<PdfVal> {
        let mut cur = Some(page_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let dict = self.get_dict(id)?;
            if let Some(res) = dict.get("Resources") {
                return Some(res.clone());
            }
            cur = dict.get("Parent").and_then(PdfVal::as_ref_id);
        }
        None
    }
}

fn scan_indirect_objects(
    data: &[u8],
    xref: &mut BTreeMap<u32, usize>,
    root_id: &mut Option<u32>,
    max_size: &mut u32,
) {
    let mut i = 0;
    while i < data.len() {
        // Look for `<num> <gen> obj` at start of line.
        if (i == 0 || data[i - 1] == b'\n' || data[i - 1] == b'\r') && data[i].is_ascii_digit() {
            let mut c = Cursor::new(data, i);
            if let Some(num) = c.read_int().and_then(|v| u32::try_from(v).ok()) {
                c.skip_ws();
                if c.read_int().is_some() {
                    c.skip_ws();
                    if c.starts_with(b"obj")
                        && data
                            .get(c.pos + 3)
                            .is_none_or(|b| is_ws(*b) || *b == b'<' || *b == b'[')
                    {
                        xref.insert(num, i);
                        *max_size = (*max_size).max(num + 1);
                        c.pos += 3;
                        c.skip_ws();
                        if let Some(PdfVal::Dict(d)) = c.parse_val()
                            && d.get("Type").and_then(PdfVal::as_name) == Some("Catalog")
                        {
                            *root_id = Some(num);
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn starts_with(&self, pat: &[u8]) -> bool {
        self.data[self.pos..].starts_with(pat)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_ws(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(cb) = self.next_byte() {
                    if cb == b'\n' || cb == b'\r' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn read_int(&mut self) -> Option<i64> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == digit_start {
            self.pos = start;
            return None;
        }
        std::str::from_utf8(&self.data[start..self.pos])
            .ok()?
            .parse::<i64>()
            .ok()
    }

    fn parse_val(&mut self) -> Option<PdfVal> {
        self.skip_ws();
        let b = self.peek()?;
        match b {
            b'<' if self.starts_with(b"<<") => {
                self.pos += 2;
                let mut map = BTreeMap::new();
                loop {
                    self.skip_ws();
                    if self.starts_with(b">>") {
                        self.pos += 2;
                        break;
                    }
                    if self.is_eof() {
                        break;
                    }
                    let PdfVal::Name(key) = self.parse_val()? else {
                        break;
                    };
                    let val = self.parse_val()?;
                    map.insert(key, val);
                }
                Some(PdfVal::Dict(map))
            }
            b'<' => {
                self.pos += 1;
                let mut hex = Vec::new();
                while let Some(ch) = self.next_byte() {
                    if ch == b'>' {
                        break;
                    }
                    if ch.is_ascii_hexdigit() {
                        hex.push(ch);
                    }
                }
                if hex.len() % 2 == 1 {
                    hex.push(b'0');
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                for pair in hex.as_chunks::<2>().0 {
                    if let Ok(s) = std::str::from_utf8(pair)
                        && let Ok(byte) = u8::from_str_radix(s, 16)
                    {
                        bytes.push(byte);
                    }
                }
                Some(PdfVal::HexStr(bytes))
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                    if self.is_eof() {
                        break;
                    }
                    items.push(self.parse_val()?);
                }
                Some(PdfVal::Array(items))
            }
            b'(' => {
                self.pos += 1;
                let mut bytes = Vec::new();
                let mut depth = 1usize;
                while let Some(ch) = self.next_byte() {
                    match ch {
                        b'\\' => {
                            let Some(esc) = self.next_byte() else {
                                break;
                            };
                            match esc {
                                b'n' => bytes.push(b'\n'),
                                b'r' => bytes.push(b'\r'),
                                b't' => bytes.push(b'\t'),
                                b'b' => bytes.push(0x08),
                                b'f' => bytes.push(0x0C),
                                b'(' | b')' | b'\\' => bytes.push(esc),
                                b'0'..=b'7' => {
                                    let mut val = esc - b'0';
                                    for _ in 0..2 {
                                        if let Some(d @ b'0'..=b'7') = self.peek() {
                                            self.pos += 1;
                                            val = val.wrapping_mul(8).wrapping_add(d - b'0');
                                        } else {
                                            break;
                                        }
                                    }
                                    bytes.push(val);
                                }
                                other => bytes.push(other),
                            }
                        }
                        b'(' => {
                            depth += 1;
                            bytes.push(b'(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            bytes.push(b')');
                        }
                        other => bytes.push(other),
                    }
                }
                Some(PdfVal::Str(bytes))
            }
            b'/' => {
                self.pos += 1;
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if is_ws(ch) || is_delim(ch) {
                        break;
                    }
                    self.pos += 1;
                }
                let name = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
                Some(PdfVal::Name(name))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.pos;
                let mut is_real = false;
                if matches!(b, b'+' | b'-') {
                    self.pos += 1;
                }
                while let Some(ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        self.pos += 1;
                    } else if ch == b'.' {
                        is_real = true;
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let token = std::str::from_utf8(&self.data[start..self.pos]).ok()?;
                if is_real {
                    Some(PdfVal::Real(token.parse::<f64>().ok()?))
                } else {
                    let n = token.parse::<i64>().ok()?;
                    // Look ahead for indirect reference `N G R`.
                    let saved = self.pos;
                    self.skip_ws();
                    if let Some(generation) = self.read_int().and_then(|g| u16::try_from(g).ok()) {
                        self.skip_ws();
                        if self.peek() == Some(b'R')
                            && self
                                .data
                                .get(self.pos + 1)
                                .is_none_or(|&c| is_ws(c) || is_delim(c))
                            && let Ok(obj_id) = u32::try_from(n)
                        {
                            self.pos += 1;
                            return Some(PdfVal::Ref(obj_id, generation));
                        }
                    }
                    self.pos = saved;
                    Some(PdfVal::Int(n))
                }
            }
            _ => {
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if is_ws(ch) || is_delim(ch) {
                        break;
                    }
                    self.pos += 1;
                }
                match &self.data[start..self.pos] {
                    b"true" => Some(PdfVal::Bool(true)),
                    b"false" => Some(PdfVal::Bool(false)),
                    b"null" => Some(PdfVal::Null),
                    _ => None,
                }
            }
        }
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0x00 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn find_last_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn find_first_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_mupdf::{Document, PageBox};

    #[test]
    fn builder_creates_valid_pdf_with_outlines_links_and_attachments() {
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 18 Tf 72 720 Td (Hello Page One) Tj ET".into(),
            crop_box: None,
        });
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 18 Tf 72 720 Td (Hello Page Two) Tj ET".into(),
            crop_box: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: "Chapter 1".into(),
            page: 0,
            y_pt: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: "Chapter 2".into(),
            page: 1,
            y_pt: None,
        });
        builder.add_link(PdfLinkSpec {
            source_page: 0,
            rect: PageRect {
                x0: 72.0,
                y0: 50.0,
                x1: 200.0,
                y1: 80.0,
            },
            target: PdfLinkTarget::InternalPage {
                page: 1,
                point: None,
                zoom: None,
            },
        });
        builder.add_attachment("lecture.wav", b"RIFFtest".to_vec());

        let pdf = builder.build().expect("build pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open in mupdf");
        assert_eq!(doc.page_count().expect("count"), 2);
        let outline = doc.outline().expect("outline");
        assert_eq!(outline.len(), 2);
        assert_eq!(outline[0].title, "Chapter 1");
        assert_eq!(outline[0].page, Some(0));
        assert_eq!(outline[1].title, "Chapter 2");
        assert_eq!(outline[1].page, Some(1));

        let page0 = doc.load_page(0).expect("page0");
        let text = page0.text(None).expect("text").text().expect("extract");
        assert!(text.contains("Hello Page One"), "got: {text}");
        let links = page0.links().expect("links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target.map(|t| t.page), Some(1));

        let attachments = doc.attachments().expect("attachments");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "lecture.wav");
    }

    #[test]
    fn insert_page_preserves_incremental_prefix_and_outline_targets() {
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 700 Td (Original One) Tj ET".into(),
            crop_box: None,
        });
        builder.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 700 Td (Original Two) Tj ET".into(),
            crop_box: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: "Second Original".into(),
            page: 1,
            y_pt: None,
        });
        let base = builder.build().expect("base");

        // Insert a new page at index 1 (between original 0 and original 1).
        let updated = insert_page(&base, 1, 595.28, 841.89, "").expect("insert");
        assert!(
            updated.starts_with(&base),
            "page insertion keeps incremental update prefix"
        );

        let doc = Document::from_bytes(&updated, "application/pdf").expect("mupdf");
        assert_eq!(doc.page_count().expect("count"), 3);
        let outline = doc.outline().expect("outline");
        assert_eq!(outline.len(), 1);
        assert_eq!(
            outline[0].page,
            Some(2),
            "outline entry for original page 1 now points to page 2"
        );
    }

    #[test]
    fn expand_and_remove_margins_restores_exact_boxes() {
        let base = build_blank_pdf(&[(612.0, 792.0)]).expect("base");
        let (expanded, orig) = expand_margins(&base, 0, 100.0, 144.0, 36.0, 24.0).expect("expand");
        assert_eq!(orig.media_box, [0.0, 0.0, 612.0, 792.0]);
        assert_eq!(orig.crop_box, [0.0, 0.0, 612.0, 792.0]);

        let doc = Document::from_bytes(&expanded, "application/pdf").expect("open expanded");
        let mb = doc.page_box(0, PageBox::Media).expect("media");
        let cb = doc.page_box(0, PageBox::Crop).expect("crop");
        assert!(((mb.x1 - mb.x0) - (612.0 + 244.0)).abs() < 0.1, "{mb:?}");
        assert!(((mb.y1 - mb.y0) - (792.0 + 60.0)).abs() < 0.1, "{mb:?}");
        assert!(((cb.x1 - cb.x0) - (612.0 + 244.0)).abs() < 0.1, "{cb:?}");

        // Second expansion must keep the ORIGINAL pre-expansion boxes.
        let (expanded2, orig2) =
            expand_margins(&expanded, 0, 20.0, 0.0, 0.0, 0.0).expect("expand2");
        assert_eq!(orig2, orig);

        let restored = remove_added_margins(&expanded2, 0).expect("restore");
        let doc2 = Document::from_bytes(&restored, "application/pdf").expect("open restored");
        let mb2 = doc2.page_box(0, PageBox::Media).expect("media2");
        let cb2 = doc2.page_box(0, PageBox::Crop).expect("crop2");
        assert!(((mb2.x1 - mb2.x0) - 612.0).abs() < 0.01, "{mb2:?}");
        assert!(((mb2.y1 - mb2.y0) - 792.0).abs() < 0.01, "{mb2:?}");
        assert!(((cb2.x1 - cb2.x0) - 612.0).abs() < 0.01, "{cb2:?}");
        assert!(read_original_boxes(&restored, 0).expect("read").is_none());
    }
}
