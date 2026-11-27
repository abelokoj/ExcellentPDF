//! Page organization, split/merge, thumbnail drag reordering, and page cropping (plan O.1, R.18, O.2, O.3, D3, D4).
//!
//! Provides an undoable command model ([`PageOrganizer`]) for reordering, rotating, inserting,
//! duplicating, deleting, and extracting pages; `/PageLabels` editing and outline destination
//! remapping; multi-file merge with outline synthesis and multi-strategy split ([`split_pdf`],
//! [`merge_pdfs`]); and `/CropBox`/`/TrimBox`/`/BleedBox` cropping plus content-bounds auto-crop.
//! All PDF mutations route through MuPDF 1.28.5 via `ecclesia-mupdf`.

pub(crate) mod pdf_doc;

use std::collections::BTreeSet;

use ecclesia_mupdf::{
    Background, Document, Matrix, MergeOutlineMode, MergeSourceBytes, PageBox, PageRect,
    PdfPageBoxEdit, PdfPageBoxKind, PdfPageLabelRange, insert_blank_page_bytes, merge_pdf_bytes,
    read_pdf_page_labels, rearrange_pdf_bytes, rotate_pages_pdf_bytes, set_pdf_page_boxes_bytes,
    set_pdf_page_labels_bytes, uncrop_pdf_pages_bytes,
};

pub use pdf_doc::PdfDocError;

/// Convenience result alias for page operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Rotation delta in clockwise 90-degree increments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PageRotationDelta {
    /// Rotate 90 degrees clockwise.
    Clockwise90,
    /// Rotate 180 degrees.
    Rotate180,
    /// Rotate 270 degrees clockwise (90 degrees counter-clockwise).
    CounterClockwise90,
}

impl PageRotationDelta {
    /// Degrees added clockwise.
    #[must_use]
    pub const fn degrees(self) -> i64 {
        match self {
            Self::Clockwise90 => 90,
            Self::Rotate180 => 180,
            Self::CounterClockwise90 => 270,
        }
    }
}

/// Numbering style for PDF `/PageLabels` entries (ISO 32000-1 §12.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PageLabelStyle {
    /// Decimal Arabic numerals (`1, 2, 3`).
    #[default]
    Decimal,
    /// Uppercase Roman numerals (`I, II, III`).
    UpperRoman,
    /// Lowercase Roman numerals (`i, ii, iii`).
    LowerRoman,
    /// Uppercase Latin letters (`A, B, C`).
    UpperAlpha,
    /// Lowercase Latin letters (`a, b, c`).
    LowerAlpha,
    /// Prefix only, no numeric portion.
    NoNumber,
}

impl PageLabelStyle {
    const fn pdf_char(self) -> Option<char> {
        match self {
            Self::Decimal => Some('D'),
            Self::UpperRoman => Some('R'),
            Self::LowerRoman => Some('r'),
            Self::UpperAlpha => Some('A'),
            Self::LowerAlpha => Some('a'),
            Self::NoNumber => None,
        }
    }

    const fn from_pdf_char(ch: Option<char>) -> Self {
        match ch {
            Some('D') => Self::Decimal,
            Some('R') => Self::UpperRoman,
            Some('r') => Self::LowerRoman,
            Some('A') => Self::UpperAlpha,
            Some('a') => Self::LowerAlpha,
            _ => Self::NoNumber,
        }
    }
}

/// One `/PageLabels` numbering range starting at zero-based page `start_page` (plan O.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PageLabelRange {
    /// Zero-based page index where this numbering range begins.
    pub start_page: usize,
    /// Numbering style for pages in this range.
    pub style: PageLabelStyle,
    /// Optional prefix string prepended to each label (e.g. `"A-"`).
    pub prefix: String,
    /// Starting numeric value (`>= 1`, default `1`).
    pub start_number: u32,
}

impl Default for PageLabelRange {
    fn default() -> Self {
        Self {
            start_page: 0,
            style: PageLabelStyle::Decimal,
            prefix: String::new(),
            start_number: 1,
        }
    }
}

/// Target PDF page geometry box (plan O.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CropBoxTarget {
    /// Visible page region (`/CropBox`).
    #[default]
    Crop,
    /// Intended finished trim size (`/TrimBox`).
    Trim,
    /// Extended print bleed boundary (`/BleedBox`).
    Bleed,
    /// Meaningful artwork boundary (`/ArtBox`).
    Art,
}

impl CropBoxTarget {
    const fn mupdf_kind(self) -> PdfPageBoxKind {
        match self {
            Self::Crop => PdfPageBoxKind::Crop,
            Self::Trim => PdfPageBoxKind::Trim,
            Self::Bleed => PdfPageBoxKind::Bleed,
            Self::Art => PdfPageBoxKind::Art,
        }
    }
}

/// Which pages an operation applies to (plan O.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PageApplyScope {
    /// One zero-based page index.
    Single(usize),
    /// Every page in the document.
    All,
    /// Zero-based even indices (displayed 1, 3, 5...).
    OddPrintedPages,
    /// Zero-based odd indices (displayed 2, 4, 6...).
    EvenPrintedPages,
    /// Explicit zero-based page indices.
    Explicit(Vec<usize>),
}

impl PageApplyScope {
    fn resolve(&self, page_count: usize) -> Result<Vec<usize>> {
        match self {
            Self::Single(idx) => {
                if *idx >= page_count {
                    return Err(PdfDocError::OutOfRange(format!(
                        "page {idx} out of range (count {page_count})"
                    )));
                }
                Ok(vec![*idx])
            }
            Self::All => Ok((0..page_count).collect()),
            Self::OddPrintedPages => Ok((0..page_count).filter(|i| i % 2 == 0).collect()),
            Self::EvenPrintedPages => Ok((0..page_count).filter(|i| i % 2 == 1).collect()),
            Self::Explicit(indices) => {
                for &idx in indices {
                    if idx >= page_count {
                        return Err(PdfDocError::OutOfRange(format!(
                            "page {idx} out of range (count {page_count})"
                        )));
                    }
                }
                Ok(indices.clone())
            }
        }
    }
}

/// An undoable page-organizer command (plan O.1, R.18, O.3).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PageCommand {
    /// Reorders pages according to a permutation of `0..page_count`.
    Reorder(Vec<usize>),
    /// Drags selected thumbnail pages to `drop_before` index (plan R.18).
    DragReorder {
        /// Selected zero-based page indices to move together.
        selected: Vec<usize>,
        /// Target zero-based insertion slot before moving (`0..=page_count`).
        drop_before: usize,
    },
    /// Rotates target pages clockwise by `delta`.
    Rotate {
        /// Zero-based page indices to rotate.
        pages: Vec<usize>,
        /// Rotation increment.
        delta: PageRotationDelta,
    },
    /// Inserts a blank page at `at_index`.
    InsertBlank {
        /// Zero-based insertion index (`0..=page_count`).
        at_index: usize,
        /// Page width in points.
        width_pt: f32,
        /// Page height in points.
        height_pt: f32,
    },
    /// Duplicates the specified zero-based pages right after each original.
    Duplicate(Vec<usize>),
    /// Deletes the specified zero-based pages.
    Delete(Vec<usize>),
    /// Replaces `/PageLabels` ranges.
    SetPageLabels(Vec<PageLabelRange>),
    /// Sets a page box on the matching scope.
    SetCropBox {
        /// Target PDF box dictionary key.
        target: CropBoxTarget,
        /// Box in PDF user-space `[x0, y0, x1, y1]` (points).
        pdf_rect: [f32; 4],
        /// Pages to update.
        scope: PageApplyScope,
    },
    /// Restores `/CropBox` to `/MediaBox` on the matching scope.
    Uncrop(PageApplyScope),
}

/// Undoable page-organizer session keeping document snapshots for one-step undo/redo (plan O.1, R.18).
#[derive(Debug, Clone)]
pub struct PageOrganizer {
    current: Vec<u8>,
    undo_stack: Vec<(String, Vec<u8>)>,
    redo_stack: Vec<(String, Vec<u8>)>,
}

fn pdf_page_count(pdf_bytes: &[u8]) -> Result<usize> {
    let doc = Document::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    doc.page_count()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))
}

fn map_mu_error(err: ecclesia_mupdf::MuError) -> PdfDocError {
    let msg = err.message();
    if msg.contains("out of range") || msg.contains("exceeds page count") {
        PdfDocError::OutOfRange(err.to_string())
    } else if msg.contains("must be positive")
        || msg.contains("must not be empty")
        || msg.contains("zero pages")
        || msg.contains("must be >= 1")
    {
        PdfDocError::InvalidOperation(err.to_string())
    } else {
        PdfDocError::InvalidPdf(err.to_string())
    }
}

impl PageOrganizer {
    /// Starts an organizer session from raw PDF bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PdfDocError::InvalidPdf`] if `pdf_bytes` is not a valid PDF.
    pub fn new(pdf_bytes: Vec<u8>) -> Result<Self> {
        let _ = pdf_page_count(&pdf_bytes)?;
        Ok(Self {
            current: pdf_bytes,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    /// Current PDF bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.current
    }

    /// Consumes the organizer and returns the current PDF bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.current
    }

    /// Number of undoable commands in the history.
    #[must_use]
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Number of redoable commands in the history.
    #[must_use]
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Whether an undo step is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether a redo step is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Executes one [`PageCommand`] as a single undoable step.
    ///
    /// # Errors
    ///
    /// Returns [`PdfDocError`] if the command is invalid or the PDF cannot be updated.
    pub fn execute(&mut self, label: &str, command: PageCommand) -> Result<()> {
        let next = apply_command(&self.current, command)?;
        let prev = std::mem::replace(&mut self.current, next);
        self.undo_stack.push((label.to_owned(), prev));
        self.redo_stack.clear();
        Ok(())
    }

    /// Undoes the most recent page command, returning its label if any.
    #[must_use]
    pub fn undo(&mut self) -> Option<String> {
        let (label, prev) = self.undo_stack.pop()?;
        let cur = std::mem::replace(&mut self.current, prev);
        self.redo_stack.push((label.clone(), cur));
        Some(label)
    }

    /// Redoes the most recently undone page command, returning its label if any.
    #[must_use]
    pub fn redo(&mut self) -> Option<String> {
        let (label, next) = self.redo_stack.pop()?;
        let cur = std::mem::replace(&mut self.current, next);
        self.undo_stack.push((label.clone(), cur));
        Some(label)
    }
}

fn apply_command(pdf_bytes: &[u8], command: PageCommand) -> Result<Vec<u8>> {
    match command {
        PageCommand::Reorder(order) => reorder_pages(pdf_bytes, &order),
        PageCommand::DragReorder {
            selected,
            drop_before,
        } => {
            let count = pdf_page_count(pdf_bytes)?;
            let order = plan_thumbnail_drag_reorder(count, &selected, drop_before)?;
            reorder_pages(pdf_bytes, &order)
        }
        PageCommand::Rotate { pages, delta } => rotate_pages(pdf_bytes, &pages, delta),
        PageCommand::InsertBlank {
            at_index,
            width_pt,
            height_pt,
        } => insert_blank_page(pdf_bytes, at_index, width_pt, height_pt),
        PageCommand::Duplicate(pages) => duplicate_pages(pdf_bytes, &pages),
        PageCommand::Delete(pages) => delete_pages(pdf_bytes, &pages),
        PageCommand::SetPageLabels(ranges) => set_page_labels(pdf_bytes, &ranges),
        PageCommand::SetCropBox {
            target,
            pdf_rect,
            scope,
        } => set_page_box(pdf_bytes, target, pdf_rect, &scope),
        PageCommand::Uncrop(scope) => uncrop_pages(pdf_bytes, &scope),
    }
}

/// Computes the resulting page permutation after dragging `selected` thumbnails before `drop_before`
/// (plan R.18). Preserves relative order of the dragged selection.
///
/// # Errors
///
/// Returns [`PdfDocError::OutOfRange`] if any selected index exceeds `page_count` or `drop_before > page_count`.
pub fn plan_thumbnail_drag_reorder(
    page_count: usize,
    selected: &[usize],
    drop_before: usize,
) -> Result<Vec<usize>> {
    if drop_before > page_count {
        return Err(PdfDocError::OutOfRange(format!(
            "drop index {drop_before} exceeds page count {page_count}"
        )));
    }
    let mut is_selected = vec![false; page_count];
    for &idx in selected {
        if idx >= page_count {
            return Err(PdfDocError::OutOfRange(format!(
                "selected page {idx} out of range (count {page_count})"
            )));
        }
        is_selected[idx] = true;
    }
    let moved: Vec<usize> = (0..page_count).filter(|&i| is_selected[i]).collect();
    let mut remaining_before = Vec::new();
    let mut remaining_after = Vec::new();
    for (i, &selected) in is_selected.iter().enumerate().take(page_count) {
        if !selected {
            if i < drop_before {
                remaining_before.push(i);
            } else {
                remaining_after.push(i);
            }
        }
    }
    let mut order = Vec::with_capacity(page_count);
    order.extend(remaining_before);
    order.extend(moved);
    order.extend(remaining_after);
    Ok(order)
}

/// Reorders pages according to `new_order` (a permutation of `0..page_count`), preserving indirect
/// page object IDs so `/Outlines` and internal `/Link` destinations continue to target their pages
/// (plan O.1, R.18).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `new_order` is not a valid permutation or the PDF is malformed.
pub fn reorder_pages(pdf_bytes: &[u8], new_order: &[usize]) -> Result<Vec<u8>> {
    let page_count = pdf_page_count(pdf_bytes)?;
    if new_order.len() != page_count {
        return Err(PdfDocError::InvalidOperation(format!(
            "permutation length {} does not match page count {page_count}",
            new_order.len(),
        )));
    }
    let mut seen = vec![false; page_count];
    for &idx in new_order {
        if idx >= page_count || seen[idx] {
            return Err(PdfDocError::InvalidOperation(
                "reorder target is not a valid permutation".into(),
            ));
        }
        seen[idx] = true;
    }
    rearrange_pdf_bytes(pdf_bytes, new_order).map_err(map_mu_error)
}

/// Rotates `pages` clockwise by `delta` (accumulating onto existing `/Rotate`) (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any index is out of bounds or the PDF is malformed.
pub fn rotate_pages(
    pdf_bytes: &[u8],
    pages: &[usize],
    delta: PageRotationDelta,
) -> Result<Vec<u8>> {
    rotate_pages_pdf_bytes(pdf_bytes, pages, delta.degrees() as i32).map_err(map_mu_error)
}

/// Inserts a blank page of `width_pt` × `height_pt` at `at_index` (`0..=page_count`) (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if dimensions are non-positive or `at_index > page_count`.
pub fn insert_blank_page(
    pdf_bytes: &[u8],
    at_index: usize,
    width_pt: f32,
    height_pt: f32,
) -> Result<Vec<u8>> {
    if width_pt <= 0.0 || height_pt <= 0.0 {
        return Err(PdfDocError::InvalidOperation(
            "blank page dimensions must be positive".into(),
        ));
    }
    insert_blank_page_bytes(pdf_bytes, at_index, width_pt, height_pt).map_err(map_mu_error)
}

/// Inserts pages from `source_pdf` (`source_indices` or all pages if empty) into `target_pdf` at
/// `at_index` (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if either PDF is malformed or any index is out of range.
pub fn insert_pages_from(
    target_pdf: &[u8],
    source_pdf: &[u8],
    source_indices: &[usize],
    at_index: usize,
) -> Result<Vec<u8>> {
    let target_count = pdf_page_count(target_pdf)?;
    if at_index > target_count {
        return Err(PdfDocError::OutOfRange(format!(
            "insert index {at_index} exceeds target page count {target_count}"
        )));
    }
    let src_count = pdf_page_count(source_pdf)?;
    let chosen: Vec<usize> = if source_indices.is_empty() {
        (0..src_count).collect()
    } else {
        for &idx in source_indices {
            if idx >= src_count {
                return Err(PdfDocError::OutOfRange(format!(
                    "source page {idx} out of range (count {src_count})"
                )));
            }
        }
        source_indices.to_vec()
    };

    let mut merge_sources = Vec::new();
    if at_index > 0 {
        merge_sources.push(MergeSource {
            title: String::new(),
            bytes: target_pdf.to_vec(),
            page_indices: Some((0..at_index).collect()),
        });
    }
    merge_sources.push(MergeSource {
        title: String::new(),
        bytes: source_pdf.to_vec(),
        page_indices: Some(chosen),
    });
    if at_index < target_count {
        merge_sources.push(MergeSource {
            title: String::new(),
            bytes: target_pdf.to_vec(),
            page_indices: Some((at_index..target_count).collect()),
        });
    }
    merge_pdfs(&merge_sources, OutlineSynthesis::PreserveExisting)
}

/// Duplicates the specified zero-based pages (placing each copy right after the selected page) (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any page index is out of bounds.
pub fn duplicate_pages(pdf_bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>> {
    let page_count = pdf_page_count(pdf_bytes)?;
    let dup_set: BTreeSet<usize> = indices.iter().copied().collect();
    for &idx in &dup_set {
        if idx >= page_count {
            return Err(PdfDocError::OutOfRange(format!(
                "duplicate page {idx} out of range (count {page_count})"
            )));
        }
    }
    let mut selection = Vec::with_capacity(page_count + dup_set.len());
    for i in 0..page_count {
        selection.push(i);
        if dup_set.contains(&i) {
            selection.push(i);
        }
    }
    extract_pages(pdf_bytes, &selection)
}

/// Deletes the specified zero-based pages, remapping `/Outlines` entries whose destinations remain
/// valid and dropping entries targeting deleted pages (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if deleting would leave zero pages or an index is out of bounds.
pub fn delete_pages(pdf_bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>> {
    let page_count = pdf_page_count(pdf_bytes)?;
    let del_set: BTreeSet<usize> = indices.iter().copied().collect();
    for &idx in &del_set {
        if idx >= page_count {
            return Err(PdfDocError::OutOfRange(format!(
                "delete page {idx} out of range (count {page_count})"
            )));
        }
    }
    let kept: Vec<usize> = (0..page_count).filter(|i| !del_set.contains(i)).collect();
    if kept.is_empty() {
        return Err(PdfDocError::InvalidOperation(
            "cannot delete all pages from a PDF".into(),
        ));
    }
    extract_pages(pdf_bytes, &kept)
}

/// Extracts `indices` (zero-based, duplicates allowed) into a standalone PDF while preserving
/// `/Resources`, `/Annots`, `/Link` annotations, and remapped `/Outlines` (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `indices` is empty or any index is out of range.
pub fn extract_pages(pdf_bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>> {
    if indices.is_empty() {
        return Err(PdfDocError::InvalidOperation(
            "cannot extract zero pages".into(),
        ));
    }
    let page_count = pdf_page_count(pdf_bytes)?;
    for &idx in indices {
        if idx >= page_count {
            return Err(PdfDocError::OutOfRange(format!(
                "extract page {idx} out of range (count {page_count})"
            )));
        }
    }
    rearrange_pdf_bytes(pdf_bytes, indices).map_err(map_mu_error)
}

/// Reads `/PageLabels` ranges from a PDF, returning an empty vector when none exist (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be parsed.
pub fn read_page_labels(pdf_bytes: &[u8]) -> Result<Vec<PageLabelRange>> {
    let raw = read_pdf_page_labels(pdf_bytes).map_err(map_mu_error)?;
    Ok(raw
        .into_iter()
        .map(|r| PageLabelRange {
            start_page: r.start_page,
            style: PageLabelStyle::from_pdf_char(r.style),
            prefix: r.prefix,
            start_number: r.start_number,
        })
        .collect())
}

/// Writes `/PageLabels` ranges into the PDF catalog (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be parsed or a range has `start_number == 0`.
pub fn set_page_labels(pdf_bytes: &[u8], ranges: &[PageLabelRange]) -> Result<Vec<u8>> {
    if ranges.is_empty() {
        return delete_page_labels(pdf_bytes);
    }
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| r.start_page);
    let mut mapped = Vec::with_capacity(sorted.len());
    for range in sorted {
        if range.start_number == 0 {
            return Err(PdfDocError::InvalidOperation(
                "page label start_number must be >= 1".into(),
            ));
        }
        mapped.push(PdfPageLabelRange {
            start_page: range.start_page,
            style: range.style.pdf_char(),
            start_number: range.start_number,
            prefix: range.prefix,
        });
    }
    set_pdf_page_labels_bytes(pdf_bytes, &mapped).map_err(map_mu_error)
}

/// Removes `/PageLabels` from the PDF catalog (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be parsed.
pub fn delete_page_labels(pdf_bytes: &[u8]) -> Result<Vec<u8>> {
    set_pdf_page_labels_bytes(pdf_bytes, &[]).map_err(map_mu_error)
}

/// How [`merge_pdfs`] builds the merged document's `/Outlines` tree (plan O.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum OutlineSynthesis {
    /// Wrap each input source's existing outline under a top-level file bookmark.
    #[default]
    PerFile,
    /// Merge existing outlines directly at the top level without wrapper entries.
    PreserveExisting,
    /// Omit outlines in the merged PDF.
    Omit,
}

/// One input document (and optional subset of zero-based page indices) for [`merge_pdfs`] (plan O.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSource {
    /// Bookmark title used when [`OutlineSynthesis::PerFile`] is active.
    pub title: String,
    /// Raw PDF bytes.
    pub bytes: Vec<u8>,
    /// Zero-based pages to include; `None` includes every page in order.
    pub page_indices: Option<Vec<usize>>,
}

/// How [`split_pdf`] partitions a PDF into output documents (plan O.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SplitStrategy {
    /// Explicit zero-based inclusive page ranges `(start, end)`.
    ByRanges(Vec<(usize, usize)>),
    /// Fixed batch size of `N` pages per output document.
    EveryN(usize),
    /// Split at each bookmark of depth `level` (`1` = top-level chapter bookmarks).
    ByOutlineLevel(usize),
    /// Split when accumulated byte estimate reaches `max_bytes`.
    ByMaxBytes(usize),
}

/// One resulting document produced by [`split_pdf`] (plan O.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPart {
    /// Suggested file stem or section label.
    pub label: String,
    /// Zero-based source page indices included in this part.
    pub page_indices: Vec<usize>,
    /// Valid standalone PDF bytes.
    pub bytes: Vec<u8>,
}

/// Merges `sources` into one PDF through MuPDF grafting, preserving annotations, internal links,
/// and synthesizing or remapping `/Outlines` (plan O.2, D3, D4).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `sources` is empty, produces zero pages, or any input PDF is invalid.
pub fn merge_pdfs(sources: &[MergeSource], synthesis: OutlineSynthesis) -> Result<Vec<u8>> {
    if sources.is_empty() {
        return Err(PdfDocError::InvalidOperation(
            "merge requires at least one source PDF".into(),
        ));
    }
    let mode = match synthesis {
        OutlineSynthesis::Omit => MergeOutlineMode::Omit,
        OutlineSynthesis::PreserveExisting => MergeOutlineMode::PreserveExisting,
        OutlineSynthesis::PerFile => MergeOutlineMode::PerFile,
    };
    let raw_sources: Vec<MergeSourceBytes<'_>> = sources
        .iter()
        .map(|s| MergeSourceBytes {
            bytes: &s.bytes,
            title: if s.title.trim().is_empty() {
                None
            } else {
                Some(s.title.as_str())
            },
            pages: s.page_indices.as_deref(),
        })
        .collect();
    merge_pdf_bytes(&raw_sources, mode).map_err(map_mu_error)
}

/// Splits `pdf_bytes` according to `strategy` into standalone [`SplitPart`] documents (plan O.2).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF is invalid or the split parameters are invalid.
pub fn split_pdf(pdf_bytes: &[u8], strategy: &SplitStrategy) -> Result<Vec<SplitPart>> {
    let page_count = pdf_page_count(pdf_bytes)?;
    if page_count == 0 {
        return Err(PdfDocError::InvalidOperation(
            "cannot split an empty PDF".into(),
        ));
    }

    let groups: Vec<(String, Vec<usize>)> = match strategy {
        SplitStrategy::ByRanges(ranges) => {
            if ranges.is_empty() {
                return Err(PdfDocError::InvalidOperation(
                    "split ranges cannot be empty".into(),
                ));
            }
            let mut out = Vec::with_capacity(ranges.len());
            for (idx, &(start, end)) in ranges.iter().enumerate() {
                if start > end || end >= page_count {
                    return Err(PdfDocError::OutOfRange(format!(
                        "range {start}..={end} invalid for {page_count} pages"
                    )));
                }
                out.push((format!("part-{}", idx + 1), (start..=end).collect()));
            }
            out
        }
        SplitStrategy::EveryN(n) => {
            let chunk = (*n).max(1);
            (0..page_count)
                .collect::<Vec<_>>()
                .chunks(chunk)
                .enumerate()
                .map(|(i, c)| (format!("part-{}", i + 1), c.to_vec()))
                .collect()
        }
        SplitStrategy::ByOutlineLevel(level) => {
            let target_level = (*level).max(1);
            let mupdf_doc = Document::from_bytes(pdf_bytes, "application/pdf")
                .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
            let outline = mupdf_doc
                .outline()
                .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
            let mut split_points: Vec<(usize, String)> = Vec::new();
            collect_outline_splits(&outline, 1, target_level, &mut split_points);
            split_points.sort_by_key(|(p, _)| *p);
            split_points.dedup_by_key(|(p, _)| *p);
            if split_points.is_empty() {
                vec![("part-1".to_owned(), (0..page_count).collect())]
            } else {
                let mut out = Vec::new();
                if split_points[0].0 > 0 {
                    out.push(("front-matter".to_owned(), (0..split_points[0].0).collect()));
                }
                for (i, (start, title)) in split_points.iter().enumerate() {
                    let end = split_points
                        .get(i + 1)
                        .map_or(page_count, |(next_p, _)| *next_p);
                    if *start < end {
                        out.push((title.clone(), (*start..end).collect()));
                    }
                }
                out
            }
        }
        SplitStrategy::ByMaxBytes(max_bytes) => {
            let budget = (*max_bytes).max(2048);
            let mut parts = Vec::new();
            let mut current: Vec<usize> = Vec::new();
            for page_idx in 0..page_count {
                current.push(page_idx);
                let candidate = extract_pages(pdf_bytes, &current)?;
                if candidate.len() > budget && current.len() > 1 {
                    let overflow = current.pop();
                    parts.push((format!("part-{}", parts.len() + 1), current));
                    current = overflow.into_iter().collect();
                }
            }
            if !current.is_empty() {
                parts.push((format!("part-{}", parts.len() + 1), current));
            }
            return parts
                .into_iter()
                .map(|(label, page_indices)| {
                    let bytes = extract_pages(pdf_bytes, &page_indices)?;
                    Ok(SplitPart {
                        label,
                        page_indices,
                        bytes,
                    })
                })
                .collect();
        }
    };

    groups
        .into_iter()
        .map(|(label, page_indices)| {
            let bytes = extract_pages(pdf_bytes, &page_indices)?;
            Ok(SplitPart {
                label,
                page_indices,
                bytes,
            })
        })
        .collect()
}

fn collect_outline_splits(
    items: &[ecclesia_mupdf::OutlineItem],
    depth: usize,
    target_depth: usize,
    out: &mut Vec<(usize, String)>,
) {
    for item in items {
        if depth == target_depth
            && let Some(page) = item.page
        {
            out.push((page, item.title.clone()));
        }
        if depth < target_depth {
            collect_outline_splits(&item.children, depth + 1, target_depth, out);
        }
    }
}

/// Sets `target` box (`/CropBox`, `/TrimBox`, `/BleedBox`, `/ArtBox`) to `pdf_rect` on `scope`
/// (plan O.3). Stores the original `/CropBox` in `/EcclesiaPdfOrigCrop` on first crop so
/// [`uncrop_pages`] restores it cleanly.
///
/// # Errors
///
/// Returns [`PdfDocError`] if the rectangle is degenerate or any target page is out of range.
pub fn set_page_box(
    pdf_bytes: &[u8],
    target: CropBoxTarget,
    pdf_rect: [f32; 4],
    scope: &PageApplyScope,
) -> Result<Vec<u8>> {
    if pdf_rect[2] <= pdf_rect[0] || pdf_rect[3] <= pdf_rect[1] {
        return Err(PdfDocError::InvalidOperation(
            "crop box must have positive width and height".into(),
        ));
    }
    let page_count = pdf_page_count(pdf_bytes)?;
    let indices = scope.resolve(page_count)?;
    let edits: Vec<PdfPageBoxEdit> = indices
        .into_iter()
        .map(|page| PdfPageBoxEdit {
            page,
            rect: pdf_rect,
        })
        .collect();
    set_pdf_page_boxes_bytes(pdf_bytes, target.mupdf_kind(), &edits).map_err(map_mu_error)
}

/// Restores `/CropBox` to its pre-crop state (or removes `/CropBox` so `/MediaBox` is visible)
/// across `scope` (plan O.3).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any page index is out of range or the PDF is invalid.
pub fn uncrop_pages(pdf_bytes: &[u8], scope: &PageApplyScope) -> Result<Vec<u8>> {
    let page_count = pdf_page_count(pdf_bytes)?;
    let indices = scope.resolve(page_count)?;
    uncrop_pdf_pages_bytes(pdf_bytes, &indices).map_err(map_mu_error)
}

/// Maps a top-down page-space rectangle back to PDF user space via the inverse of `page_ctm`.
fn page_rect_to_pdf_space(rect: PageRect, page_ctm: Matrix) -> [f32; 4] {
    let det = page_ctm.a * page_ctm.d - page_ctm.b * page_ctm.c;
    if det.abs() < 1e-6 {
        return [rect.x0, rect.y0, rect.x1, rect.y1];
    }
    let inv = |x: f32, y: f32| -> (f32, f32) {
        let dx = x - page_ctm.e;
        let dy = y - page_ctm.f;
        (
            (page_ctm.d * dx - page_ctm.c * dy) / det,
            (-page_ctm.b * dx + page_ctm.a * dy) / det,
        )
    };
    let corners = [
        inv(rect.x0, rect.y0),
        inv(rect.x1, rect.y0),
        inv(rect.x0, rect.y1),
        inv(rect.x1, rect.y1),
    ];
    let mut x0 = f32::MAX;
    let mut y0 = f32::MAX;
    let mut x1 = f32::MIN;
    let mut y1 = f32::MIN;
    for (px, py) in corners {
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    [x0, y0, x1, y1]
}

/// Auto-crops pages in `scope` to non-white content bounds plus `margin_pt` (plan O.3).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be opened in MuPDF or updated.
pub fn auto_crop_to_content(
    pdf_bytes: &[u8],
    margin_pt: f32,
    scope: &PageApplyScope,
) -> Result<Vec<u8>> {
    let mupdf_doc = Document::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let page_count = mupdf_doc
        .page_count()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let indices = scope.resolve(page_count)?;

    let mut edits = Vec::with_capacity(indices.len());
    let pad = margin_pt.max(0.0);

    for idx in indices {
        let page = mupdf_doc
            .load_page(idx)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let bounds = page
            .bounds()
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let pix = page
            .render(&Matrix::IDENTITY, Background::White)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let content_rect = detect_non_white_rect(&pix, bounds);

        let page_ctm = mupdf_doc
            .page_transform(idx)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let media_page = mupdf_doc.page_box(idx, PageBox::Media).unwrap_or(bounds);
        let media = page_rect_to_pdf_space(media_page, page_ctm);
        let content_pdf = page_rect_to_pdf_space(content_rect, page_ctm);

        let pdf_x0 = (content_pdf[0] - pad).max(media[0]);
        let pdf_y0 = (content_pdf[1] - pad).max(media[1]);
        let pdf_x1 = (content_pdf[2] + pad).min(media[2]);
        let pdf_y1 = (content_pdf[3] + pad).min(media[3]);
        if pdf_x1 <= pdf_x0 || pdf_y1 <= pdf_y0 {
            continue;
        }
        edits.push(PdfPageBoxEdit {
            page: idx,
            rect: [pdf_x0, pdf_y0, pdf_x1, pdf_y1],
        });
    }

    set_pdf_page_boxes_bytes(pdf_bytes, PdfPageBoxKind::Crop, &edits).map_err(map_mu_error)
}

fn detect_non_white_rect(pix: &ecclesia_mupdf::Pixmap, bounds: PageRect) -> PageRect {
    let w = pix.width() as usize;
    let h = pix.height() as usize;
    let stride = pix.stride();
    let comp = pix.components();
    let samples = pix.samples();
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    if comp >= 3 {
        for y in 0..h {
            let row = &samples[y * stride..y * stride + w * comp];
            for (x, px) in row.chunks_exact(comp).enumerate() {
                if px[0] < 248 || px[1] < 248 || px[2] < 248 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + 1);
                    max_y = max_y.max(y + 1);
                }
            }
        }
    }
    if min_x >= max_x || min_y >= max_y {
        return bounds;
    }
    let sx = bounds.width() / w.max(1) as f32;
    let sy = bounds.height() / h.max(1) as f32;
    PageRect {
        x0: bounds.x0 + min_x as f32 * sx,
        y0: bounds.y0 + min_y as f32 * sy,
        x1: bounds.x0 + max_x as f32 * sx,
        y1: bounds.y0 + max_y as f32 * sy,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ecclesia_mupdf::{Document, PageBox};

    use super::*;
    use crate::notebook::pdf_ops::{
        OutlineEntrySpec, PdfBuilder, PdfLinkSpec, PdfLinkTarget, PdfPageInput,
    };

    fn sample_two_page_pdf() -> Vec<u8> {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 16 Tf 72 720 Td (Page Alpha) Tj ET".into(),
            crop_box: None,
        });
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 16 Tf 72 720 Td (Page Beta) Tj ET".into(),
            crop_box: None,
        });
        b.add_outline(OutlineEntrySpec {
            title: "Alpha".into(),
            page: 0,
            y_pt: None,
        });
        b.add_outline(OutlineEntrySpec {
            title: "Beta".into(),
            page: 1,
            y_pt: None,
        });
        b.add_link(PdfLinkSpec {
            source_page: 0,
            rect: PageRect {
                x0: 72.0,
                y0: 100.0,
                x1: 200.0,
                y1: 120.0,
            },
            target: PdfLinkTarget::InternalPage {
                page: 1,
                point: None,
                zoom: None,
            },
        });
        b.build().expect("sample pdf builds")
    }

    #[test]
    fn organizer_undo_redo_and_hundred_page_drag_reorder() {
        let mut big = PdfBuilder::new();
        big.enable_standard_fonts();
        for i in 0..100 {
            big.add_page(PdfPageInput {
                width: 612.0,
                height: 792.0,
                content: format!("BT /F1 12 Tf 72 720 Td (Page {i}) Tj ET"),
                crop_box: None,
            });
            if i % 25 == 0 {
                big.add_outline(OutlineEntrySpec {
                    title: format!("Section {i}"),
                    page: i,
                    y_pt: None,
                });
            }
        }
        let pdf = big.build().expect("100-page pdf");
        let mut org = PageOrganizer::new(pdf).expect("organizer");
        org.execute(
            "Drag reorder",
            PageCommand::DragReorder {
                selected: vec![0, 1],
                drop_before: 10,
            },
        )
        .expect("drag reorder");
        let moved = Document::from_bytes(org.bytes(), "application/pdf").expect("mupdf");
        assert_eq!(moved.page_count().expect("count"), 100);
        let p0 = moved
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("t")
            .text()
            .expect("s");
        assert!(p0.contains("Page 2"), "got {p0}");
        let p8 = moved
            .load_page(8)
            .expect("p8")
            .text(None)
            .expect("t")
            .text()
            .expect("s");
        assert!(p8.contains("Page 0"), "got {p8}");
        let outline = moved.outline().expect("outline");
        assert_eq!(outline[0].page, Some(8), "outline tracks moved page 0 -> 8");

        assert!(org.undo().is_some());
        let restored = Document::from_bytes(org.bytes(), "application/pdf").expect("restored");
        let r0 = restored
            .load_page(0)
            .expect("r0")
            .text(None)
            .expect("t")
            .text()
            .expect("s");
        assert!(r0.contains("Page 0"));
        assert_eq!(restored.outline().expect("outline")[0].page, Some(0));
    }

    #[test]
    fn page_labels_rotate_duplicate_delete_and_extract() {
        let pdf = sample_two_page_pdf();
        let mut org = PageOrganizer::new(pdf).expect("org");
        org.execute(
            "Labels",
            PageCommand::SetPageLabels(vec![
                PageLabelRange {
                    start_page: 0,
                    style: PageLabelStyle::LowerRoman,
                    prefix: String::new(),
                    start_number: 1,
                },
                PageLabelRange {
                    start_page: 1,
                    style: PageLabelStyle::Decimal,
                    prefix: "A-".into(),
                    start_number: 10,
                },
            ]),
        )
        .expect("set labels");
        let doc = Document::from_bytes(org.bytes(), "application/pdf").expect("open");
        assert_eq!(
            doc.page_labels().expect("labels"),
            Some(vec!["i".to_owned(), "A-10".to_owned()])
        );

        org.execute(
            "Rotate page 0",
            PageCommand::Rotate {
                pages: vec![0],
                delta: PageRotationDelta::Clockwise90,
            },
        )
        .expect("rotate");
        let rot_doc = Document::from_bytes(org.bytes(), "application/pdf").expect("rot");
        let b0 = rot_doc.load_page(0).expect("p0").bounds().expect("bounds");
        assert!(b0.width() > b0.height(), "rotated landscape: {b0:?}");

        let extracted = extract_pages(org.bytes(), &[1]).expect("extract");
        let ex_doc = Document::from_bytes(&extracted, "application/pdf").expect("ex");
        assert_eq!(ex_doc.page_count().expect("c"), 1);
        assert_eq!(ex_doc.outline().expect("o")[0].page, Some(0));

        let dup = duplicate_pages(&sample_two_page_pdf(), &[0]).expect("dup");
        assert_eq!(
            Document::from_bytes(&dup, "application/pdf")
                .expect("d")
                .page_count()
                .expect("c"),
            3
        );
        let del = delete_pages(&dup, &[1]).expect("del");
        assert_eq!(
            Document::from_bytes(&del, "application/pdf")
                .expect("d")
                .page_count()
                .expect("c"),
            2
        );
    }

    #[test]
    fn merge_hundred_files_preserves_outlines_and_links_and_split_works() {
        let base = sample_two_page_pdf();
        let sources: Vec<MergeSource> = (0..100)
            .map(|i| MergeSource {
                title: format!("Doc {i}"),
                bytes: base.clone(),
                page_indices: None,
            })
            .collect();
        let merged = merge_pdfs(&sources, OutlineSynthesis::PerFile).expect("merge 100");
        let doc = Document::from_bytes(&merged, "application/pdf").expect("open merged");
        assert_eq!(doc.page_count().expect("pages"), 200);
        let outline = doc.outline().expect("outline");
        assert_eq!(outline.len(), 100);
        assert_eq!(outline[99].page, Some(198));
        assert_eq!(outline[99].children[1].page, Some(199));

        let parts = split_pdf(&base, &SplitStrategy::EveryN(1)).expect("split");
        assert_eq!(parts.len(), 2);
        let by_outline =
            split_pdf(&base, &SplitStrategy::ByOutlineLevel(1)).expect("split outline");
        assert_eq!(by_outline.len(), 2);
    }

    #[test]
    fn crop_uncrop_and_auto_crop_corpus_file() {
        let pdf_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let bytes = std::fs::read(&pdf_path).expect("corpus pdf");
        let cropped = set_page_box(
            &bytes,
            CropBoxTarget::Crop,
            [50.0, 50.0, 400.0, 600.0],
            &PageApplyScope::All,
        )
        .expect("crop");
        let doc = Document::from_bytes(&cropped, "application/pdf").expect("open");
        let cb = doc.page_box(0, PageBox::Crop).expect("crop box");
        assert!((cb.width() - 350.0).abs() < 0.5, "{cb:?}");

        let uncropped = uncrop_pages(&cropped, &PageApplyScope::All).expect("uncrop");
        let udoc = Document::from_bytes(&uncropped, "application/pdf").expect("open");
        let mb = udoc.page_box(0, PageBox::Media).expect("mb");
        let ucb = udoc.page_box(0, PageBox::Crop).expect("ucb");
        assert!((ucb.width() - mb.width()).abs() < 0.5);

        let auto = auto_crop_to_content(&bytes, 10.0, &PageApplyScope::Single(0)).expect("auto");
        let adoc = Document::from_bytes(&auto, "application/pdf").expect("open auto");
        let acb = adoc.page_box(0, PageBox::Crop).expect("auto crop box");
        assert!(acb.width() <= mb.width() && acb.height() <= mb.height());
    }
}
