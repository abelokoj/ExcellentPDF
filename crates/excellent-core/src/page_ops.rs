//! Page organization, split/merge, thumbnail drag reordering, and page cropping (plan O.1, R.18, O.2, O.3).
//!
//! Provides an undoable command model ([`PageOrganizer`]) for reordering, rotating, inserting,
//! duplicating, deleting, and extracting pages; `/PageLabels` editing and outline destination
//! remapping; multi-file merge with outline synthesis and multi-strategy split ([`split_pdf`],
//! [`merge_pdfs`]); and `/CropBox`/`/TrimBox`/`/BleedBox` cropping plus content-bounds auto-crop.

pub(crate) mod pdf_doc;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use excellent_mupdf::{Background, Document, Matrix, PageRect};

use pdf_doc::{
    ParentLinkMode, ParsedPdf, PdfVal, RawPdfWriter, StreamEncoding, append_incremental,
    clone_val_into, escape_pdf_string, fmt_f32, make_stream, rect_val, rewrite_clean,
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
    fn pdf_name(self) -> Option<&'static str> {
        match self {
            Self::Decimal => Some("D"),
            Self::UpperRoman => Some("R"),
            Self::LowerRoman => Some("r"),
            Self::UpperAlpha => Some("A"),
            Self::LowerAlpha => Some("a"),
            Self::NoNumber => None,
        }
    }

    fn from_pdf_name(name: Option<&str>) -> Self {
        match name {
            Some("D") => Self::Decimal,
            Some("R") => Self::UpperRoman,
            Some("r") => Self::LowerRoman,
            Some("A") => Self::UpperAlpha,
            Some("a") => Self::LowerAlpha,
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
    fn pdf_key(self) -> &'static str {
        match self {
            Self::Crop => "CropBox",
            Self::Trim => "TrimBox",
            Self::Bleed => "BleedBox",
            Self::Art => "ArtBox",
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

impl PageOrganizer {
    /// Starts an organizer session from raw PDF bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PdfDocError::InvalidPdf`] if `pdf_bytes` is not a valid PDF.
    pub fn new(pdf_bytes: Vec<u8>) -> Result<Self> {
        let _ = ParsedPdf::parse(&pdf_bytes)?;
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
            let doc = ParsedPdf::parse(pdf_bytes)?;
            let count = doc.page_objects()?.len();
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    if new_order.len() != pages.len() {
        return Err(PdfDocError::InvalidOperation(format!(
            "permutation length {} does not match page count {}",
            new_order.len(),
            pages.len()
        )));
    }
    let mut seen = vec![false; pages.len()];
    for &idx in new_order {
        if idx >= pages.len() || seen[idx] {
            return Err(PdfDocError::InvalidOperation(
                "reorder target is not a valid permutation".into(),
            ));
        }
        seen[idx] = true;
    }
    let mut mapping = HashMap::with_capacity(pages.len());
    for (new_idx, &old_idx) in new_order.iter().enumerate() {
        mapping.insert(pages[old_idx], new_idx);
    }
    let mut overrides = BTreeMap::new();
    let pages_root_id = doc.pages_root_id()?;
    reparent_pages(&doc, &pages, pages_root_id, &mut overrides)?;
    let kids: Vec<PdfVal> = new_order
        .iter()
        .map(|&old_idx| PdfVal::Ref(pages[old_idx], 0))
        .collect();
    let mut root_pages = doc.get_dict(pages_root_id).unwrap_or_default();
    root_pages.insert("Type".into(), PdfVal::Name("Pages".into()));
    root_pages.insert("Kids".into(), PdfVal::Array(kids));
    root_pages.insert("Count".into(), PdfVal::Int(pages.len() as i64));
    overrides.insert(pages_root_id, PdfVal::Dict(root_pages));

    Ok(rewrite_clean(
        &doc,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let page_ids = doc.page_objects()?;
    let mut updates = BTreeMap::new();
    for &idx in pages {
        let &pid = page_ids.get(idx).ok_or_else(|| {
            PdfDocError::OutOfRange(format!(
                "page {idx} out of range (count {})",
                page_ids.len()
            ))
        })?;
        let mut dict = doc
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dictionary missing".into()))?;
        let current = doc.effective_rotate(pid);
        let next = (current + delta.degrees()).rem_euclid(360);
        dict.insert("Rotate".into(), PdfVal::Int(next));
        updates.insert(pid, pdf_doc::serialize_dict(&dict));
    }
    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.info_id,
        doc.xref_offset,
        doc.next_obj_id(),
    ))
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    if at_index > pages.len() {
        return Err(PdfDocError::OutOfRange(format!(
            "insert index {at_index} exceeds page count {}",
            pages.len()
        )));
    }
    let pages_root_id = doc.pages_root_id()?;
    let next_id = doc.next_obj_id();
    let new_page_id = next_id;
    let content_id = next_id + 1;

    let mut overrides = BTreeMap::new();
    reparent_pages(&doc, &pages, pages_root_id, &mut overrides)?;

    let content_stream = make_stream(BTreeMap::new(), b"", StreamEncoding::Raw)?;
    let mut extra = BTreeMap::new();
    extra.insert(content_id, content_stream);

    let mut page_dict = BTreeMap::new();
    page_dict.insert("Type".into(), PdfVal::Name("Page".into()));
    page_dict.insert("Parent".into(), PdfVal::Ref(pages_root_id, 0));
    page_dict.insert("MediaBox".into(), rect_val([0.0, 0.0, width_pt, height_pt]));
    page_dict.insert("Contents".into(), PdfVal::Ref(content_id, 0));
    page_dict.insert("Resources".into(), PdfVal::Dict(BTreeMap::new()));
    extra.insert(new_page_id, PdfVal::Dict(page_dict));

    let mut new_kids = pages;
    new_kids.insert(at_index, new_page_id);
    let mut root_pages = doc.get_dict(pages_root_id).unwrap_or_default();
    root_pages.insert("Type".into(), PdfVal::Name("Pages".into()));
    root_pages.insert(
        "Kids".into(),
        PdfVal::Array(new_kids.iter().map(|&id| PdfVal::Ref(id, 0)).collect()),
    );
    root_pages.insert("Count".into(), PdfVal::Int(new_kids.len() as i64));
    overrides.insert(pages_root_id, PdfVal::Dict(root_pages));

    Ok(rewrite_clean(&doc, &overrides, &extra, &BTreeSet::new()))
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
    let target = ParsedPdf::parse(target_pdf)?;
    let target_pages = target.page_objects()?;
    if at_index > target_pages.len() {
        return Err(PdfDocError::OutOfRange(format!(
            "insert index {at_index} exceeds target page count {}",
            target_pages.len()
        )));
    }
    let src = ParsedPdf::parse(source_pdf)?;
    let src_pages = src.page_objects()?;
    let chosen: Vec<usize> = if source_indices.is_empty() {
        (0..src_pages.len()).collect()
    } else {
        for &idx in source_indices {
            if idx >= src_pages.len() {
                return Err(PdfDocError::OutOfRange(format!(
                    "source page {idx} out of range (count {})",
                    src_pages.len()
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
    if at_index < target_pages.len() {
        merge_sources.push(MergeSource {
            title: String::new(),
            bytes: target_pdf.to_vec(),
            page_indices: Some((at_index..target_pages.len()).collect()),
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let dup_set: BTreeSet<usize> = indices.iter().copied().collect();
    for &idx in &dup_set {
        if idx >= pages.len() {
            return Err(PdfDocError::OutOfRange(format!(
                "duplicate page {idx} out of range (count {})",
                pages.len()
            )));
        }
    }
    let mut selection = Vec::with_capacity(pages.len() + dup_set.len());
    for i in 0..pages.len() {
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let del_set: BTreeSet<usize> = indices.iter().copied().collect();
    for &idx in &del_set {
        if idx >= pages.len() {
            return Err(PdfDocError::OutOfRange(format!(
                "delete page {idx} out of range (count {})",
                pages.len()
            )));
        }
    }
    let kept: Vec<usize> = (0..pages.len()).filter(|i| !del_set.contains(i)).collect();
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
    let src = ParsedPdf::parse(pdf_bytes)?;
    let src_pages = src.page_objects()?;
    for &idx in indices {
        if idx >= src_pages.len() {
            return Err(PdfDocError::OutOfRange(format!(
                "extract page {idx} out of range (count {})",
                src_pages.len()
            )));
        }
    }
    let source = MergeSource {
        title: String::new(),
        bytes: pdf_bytes.to_vec(),
        page_indices: Some(indices.to_vec()),
    };
    merge_pdfs(&[source], OutlineSynthesis::PreserveExisting)
}

/// Reads `/PageLabels` ranges from a PDF, returning an empty vector when none exist (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be parsed.
pub fn read_page_labels(pdf_bytes: &[u8]) -> Result<Vec<PageLabelRange>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let Some(catalog) = doc.get_dict(doc.root_id) else {
        return Ok(Vec::new());
    };
    let Some(labels_val) = catalog.get("PageLabels") else {
        return Ok(Vec::new());
    };
    let Some(PdfVal::Dict(labels_dict)) = doc.resolve_val(labels_val) else {
        return Ok(Vec::new());
    };
    let mut ranges = Vec::new();
    collect_page_label_nums(&doc, &labels_dict, &mut ranges);
    ranges.sort_by_key(|r| r.start_page);
    Ok(ranges)
}

fn collect_page_label_nums(
    doc: &ParsedPdf,
    node: &BTreeMap<String, PdfVal>,
    out: &mut Vec<PageLabelRange>,
) {
    if let Some(nums_val) = node.get("Nums")
        && let Some(PdfVal::Array(arr)) = doc.resolve_val(nums_val)
    {
        for pair in arr.as_chunks::<2>().0 {
            let Some(start_page) = pair[0].as_i64().and_then(|v| usize::try_from(v).ok()) else {
                continue;
            };
            let Some(PdfVal::Dict(entry)) = doc.resolve_val(&pair[1]) else {
                continue;
            };
            let style = PageLabelStyle::from_pdf_name(
                entry
                    .get("S")
                    .and_then(|v| doc.resolve_val(v))
                    .as_ref()
                    .and_then(PdfVal::as_name),
            );
            let prefix = entry
                .get("P")
                .and_then(|v| doc.resolve_val(v))
                .and_then(|v| v.as_text_string())
                .unwrap_or_default();
            let start_number = entry
                .get("St")
                .and_then(|v| doc.resolve_val(v))
                .and_then(|v| v.as_i64())
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(1);
            out.push(PageLabelRange {
                start_page,
                style,
                prefix,
                start_number,
            });
        }
    }
    if let Some(kids_val) = node.get("Kids")
        && let Some(PdfVal::Array(kids)) = doc.resolve_val(kids_val)
    {
        for kid in kids {
            if let Some(PdfVal::Dict(kd)) = doc.resolve_val(&kid) {
                collect_page_label_nums(doc, &kd, out);
            }
        }
    }
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| r.start_page);

    let mut nums = Vec::with_capacity(sorted.len() * 2);
    for range in sorted {
        if range.start_number == 0 {
            return Err(PdfDocError::InvalidOperation(
                "page label start_number must be >= 1".into(),
            ));
        }
        nums.push(PdfVal::Int(range.start_page as i64));
        let mut entry = BTreeMap::new();
        if let Some(style_name) = range.style.pdf_name() {
            entry.insert("S".into(), PdfVal::Name(style_name.into()));
        }
        if !range.prefix.is_empty() {
            entry.insert("P".into(), PdfVal::Str(range.prefix.into_bytes()));
        }
        if range.start_number != 1 {
            entry.insert("St".into(), PdfVal::Int(i64::from(range.start_number)));
        }
        nums.push(PdfVal::Dict(entry));
    }

    let mut labels_dict = BTreeMap::new();
    labels_dict.insert("Nums".into(), PdfVal::Array(nums));
    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("catalog missing".into()))?;
    catalog.insert("PageLabels".into(), PdfVal::Dict(labels_dict));

    let mut overrides = BTreeMap::new();
    overrides.insert(doc.root_id, PdfVal::Dict(catalog));
    Ok(rewrite_clean(
        &doc,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
}

/// Removes `/PageLabels` from the PDF catalog (plan O.1).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be parsed.
pub fn delete_page_labels(pdf_bytes: &[u8]) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("catalog missing".into()))?;
    catalog.remove("PageLabels");
    let mut overrides = BTreeMap::new();
    overrides.insert(doc.root_id, PdfVal::Dict(catalog));
    Ok(rewrite_clean(
        &doc,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
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

/// Outline node used internally while remapping / synthesizing outlines across merge & split.
#[derive(Debug, Clone)]
struct OutlineNodeSpec {
    title: String,
    dest_page: Option<usize>,
    y_pt: Option<f32>,
    uri: Option<String>,
    children: Vec<OutlineNodeSpec>,
}

/// Merges `sources` into one PDF, preserving annotations, internal links, and synthesizing or
/// remapping `/Outlines` (plan O.2).
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

    let mut writer = RawPdfWriter::new();
    let catalog_id = writer.alloc();
    let pages_root_id = writer.alloc();

    let mut merged_page_ids: Vec<u32> = Vec::new();
    let mut merged_page_boxes: Vec<[f32; 4]> = Vec::new();
    let mut merged_outlines: Vec<OutlineNodeSpec> = Vec::new();

    for source in sources {
        let src = ParsedPdf::parse(&source.bytes)?;
        let src_pages = src.page_objects()?;
        let chosen: Vec<usize> = match &source.page_indices {
            Some(list) => {
                for &idx in list {
                    if idx >= src_pages.len() {
                        return Err(PdfDocError::OutOfRange(format!(
                            "merge source page {idx} out of range (count {})",
                            src_pages.len()
                        )));
                    }
                }
                list.clone()
            }
            None => (0..src_pages.len()).collect(),
        };
        if chosen.is_empty() {
            continue;
        }

        let mut cloned_objs: HashMap<u32, u32> = HashMap::new();
        let mut src_pid_to_first_merged_idx: HashMap<u32, usize> = HashMap::new();
        let start_merged_idx = merged_page_ids.len();

        // First pass: allocate destination page IDs so cross-page /Link destinations resolve.
        let mut dst_page_ids = Vec::with_capacity(chosen.len());
        for &src_idx in &chosen {
            let src_pid = src_pages[src_idx];
            let dst_pid = writer.alloc();
            // Do not insert into cloned_objs before cloning when a page is duplicated; track first mapping.
            let merged_idx = merged_page_ids.len();
            src_pid_to_first_merged_idx
                .entry(src_pid)
                .or_insert(merged_idx);
            merged_page_ids.push(dst_pid);
            let mb = src
                .effective_box(src_pid, "MediaBox")
                .unwrap_or([0.0, 0.0, 612.0, 792.0]);
            merged_page_boxes.push(mb);
            dst_page_ids.push((src_idx, src_pid, dst_pid));
        }
        for (_, src_pid, dst_pid) in &dst_page_ids {
            cloned_objs.entry(*src_pid).or_insert(*dst_pid);
        }

        // Second pass: clone each selected page dictionary and its dependencies.
        for (_src_idx, src_pid, dst_pid) in dst_page_ids {
            let mut page_dict = src
                .get_dict(src_pid)
                .ok_or_else(|| PdfDocError::InvalidPdf("source page dict missing".into()))?;
            if !page_dict.contains_key("MediaBox")
                && let Ok(mb) = src.effective_box(src_pid, "MediaBox")
            {
                page_dict.insert("MediaBox".into(), rect_val(mb));
            }
            if !page_dict.contains_key("CropBox")
                && let Ok(cb) = src.effective_box(src_pid, "CropBox")
            {
                page_dict.insert("CropBox".into(), rect_val(cb));
            }
            if !page_dict.contains_key("Resources") {
                let res = src
                    .effective_resources(src_pid)
                    .unwrap_or_else(|| PdfVal::Dict(BTreeMap::new()));
                page_dict.insert("Resources".into(), res);
            }
            if !page_dict.contains_key("Rotate") {
                let rot = src.effective_rotate(src_pid);
                if rot != 0 {
                    page_dict.insert("Rotate".into(), PdfVal::Int(rot));
                }
            }
            // Remove structure parent link and clone remaining entries cleanly.
            page_dict.remove("Parent");
            page_dict.remove("StructParents");
            page_dict.remove("B");
            let mut remapped = BTreeMap::new();
            for (k, v) in &page_dict {
                remapped.insert(
                    k.clone(),
                    clone_val_into(
                        &src,
                        v,
                        &mut writer,
                        &mut cloned_objs,
                        ParentLinkMode::SkipPageParent,
                    )?,
                );
            }
            remapped.insert("Type".into(), PdfVal::Name("Page".into()));
            remapped.insert("Parent".into(), PdfVal::Ref(pages_root_id, 0));
            writer.set_val(dst_pid, &PdfVal::Dict(remapped));
        }

        // Collect & remap existing outlines for this source.
        if synthesis != OutlineSynthesis::Omit {
            let src_outline =
                read_pdf_outline_nodes(&src, &src_pages, &src_pid_to_first_merged_idx);
            match synthesis {
                OutlineSynthesis::PerFile => {
                    let title = if source.title.trim().is_empty() {
                        format!("Document {}", start_merged_idx + 1)
                    } else {
                        source.title.clone()
                    };
                    merged_outlines.push(OutlineNodeSpec {
                        title,
                        dest_page: Some(start_merged_idx),
                        y_pt: None,
                        uri: None,
                        children: src_outline,
                    });
                }
                OutlineSynthesis::PreserveExisting => {
                    merged_outlines.extend(src_outline);
                }
                OutlineSynthesis::Omit => {}
            }
        }
    }

    if merged_page_ids.is_empty() {
        return Err(PdfDocError::InvalidOperation(
            "merged PDF contains zero pages".into(),
        ));
    }

    let mut pages_dict = BTreeMap::new();
    pages_dict.insert("Type".into(), PdfVal::Name("Pages".into()));
    pages_dict.insert(
        "Kids".into(),
        PdfVal::Array(
            merged_page_ids
                .iter()
                .map(|&id| PdfVal::Ref(id, 0))
                .collect(),
        ),
    );
    pages_dict.insert("Count".into(), PdfVal::Int(merged_page_ids.len() as i64));
    writer.set_val(pages_root_id, &PdfVal::Dict(pages_dict));

    let outlines_id = write_outline_tree(
        &mut writer,
        &merged_outlines,
        &merged_page_ids,
        &merged_page_boxes,
    );

    let mut catalog = BTreeMap::new();
    catalog.insert("Type".into(), PdfVal::Name("Catalog".into()));
    catalog.insert("Pages".into(), PdfVal::Ref(pages_root_id, 0));
    if let Some(oid) = outlines_id {
        catalog.insert("Outlines".into(), PdfVal::Ref(oid, 0));
        catalog.insert("PageMode".into(), PdfVal::Name("UseOutlines".into()));
    }
    writer.set_val(catalog_id, &PdfVal::Dict(catalog));

    Ok(writer.finish(catalog_id, None))
}

/// Splits `pdf_bytes` according to `strategy` into standalone [`SplitPart`] documents (plan O.2).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF is invalid or the split parameters are invalid.
pub fn split_pdf(pdf_bytes: &[u8], strategy: &SplitStrategy) -> Result<Vec<SplitPart>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let page_count = doc.page_objects()?.len();
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
    items: &[excellent_mupdf::OutlineItem],
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
/// (plan O.3). Stores the original `/CropBox` in `/ExcellentPdfOrigCrop` on first crop so
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
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let indices = scope.resolve(pages.len())?;
    let mut updates = BTreeMap::new();

    for idx in indices {
        let pid = pages[idx];
        let mut dict = doc
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
        if target == CropBoxTarget::Crop && !dict.contains_key("ExcellentPdfOrigCrop") {
            let orig = doc
                .effective_box(pid, "CropBox")
                .or_else(|_| doc.effective_box(pid, "MediaBox"))
                .unwrap_or(pdf_rect);
            dict.insert("ExcellentPdfOrigCrop".into(), rect_val(orig));
        }
        dict.insert(target.pdf_key().into(), rect_val(pdf_rect));
        updates.insert(pid, pdf_doc::serialize_dict(&dict));
    }

    Ok(append_incremental(
        pdf_bytes,
        &updates,
        doc.root_id,
        doc.info_id,
        doc.xref_offset,
        doc.next_obj_id(),
    ))
}

/// Restores `/CropBox` to its pre-crop state (or removes `/CropBox` so `/MediaBox` is visible)
/// across `scope` (plan O.3).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any page index is out of range or the PDF is invalid.
pub fn uncrop_pages(pdf_bytes: &[u8], scope: &PageApplyScope) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let indices = scope.resolve(pages.len())?;
    let mut overrides = BTreeMap::new();

    for idx in indices {
        let pid = pages[idx];
        let mut dict = doc
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
        if let Some(orig) = dict.remove("ExcellentPdfOrigCrop") {
            dict.insert("CropBox".into(), orig);
        } else {
            dict.remove("CropBox");
        }
        overrides.insert(pid, PdfVal::Dict(dict));
    }

    Ok(rewrite_clean(
        &doc,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
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
    let parsed = ParsedPdf::parse(pdf_bytes)?;
    let page_ids = parsed.page_objects()?;
    let indices = scope.resolve(page_ids.len())?;

    let mupdf_doc = Document::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;

    let mut overrides = BTreeMap::new();
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

        let pid = page_ids[idx];
        let media = parsed.effective_box(pid, "MediaBox").unwrap_or([
            0.0,
            0.0,
            bounds.width(),
            bounds.height(),
        ]);
        // Convert top-down page coordinates to PDF bottom-up user space.
        let pdf_x0 = (media[0] + content_rect.x0 - pad).max(media[0]);
        let pdf_x1 = (media[0] + content_rect.x1 + pad).min(media[2]);
        let pdf_y0 = (media[3] - content_rect.y1 - pad).max(media[1]);
        let pdf_y1 = (media[3] - content_rect.y0 + pad).min(media[3]);
        if pdf_x1 <= pdf_x0 || pdf_y1 <= pdf_y0 {
            continue;
        }
        let mut dict = parsed
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
        if !dict.contains_key("ExcellentPdfOrigCrop") {
            let orig = parsed.effective_box(pid, "CropBox").unwrap_or(media);
            dict.insert("ExcellentPdfOrigCrop".into(), rect_val(orig));
        }
        dict.insert("CropBox".into(), rect_val([pdf_x0, pdf_y0, pdf_x1, pdf_y1]));
        overrides.insert(pid, PdfVal::Dict(dict));
    }

    Ok(rewrite_clean(
        &parsed,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
}

fn detect_non_white_rect(pix: &excellent_mupdf::Pixmap, bounds: PageRect) -> PageRect {
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

fn reparent_pages(
    doc: &ParsedPdf,
    pages: &[u32],
    pages_root_id: u32,
    overrides: &mut BTreeMap<u32, PdfVal>,
) -> Result<()> {
    for &pid in pages {
        let mut dict = doc
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page object missing".into()))?;
        if !dict.contains_key("MediaBox")
            && let Ok(mb) = doc.effective_box(pid, "MediaBox")
        {
            dict.insert("MediaBox".into(), rect_val(mb));
        }
        if !dict.contains_key("CropBox")
            && let Ok(cb) = doc.effective_box(pid, "CropBox")
        {
            dict.insert("CropBox".into(), rect_val(cb));
        }
        if !dict.contains_key("Resources")
            && let Some(res) = doc.effective_resources(pid)
        {
            dict.insert("Resources".into(), res);
        }
        let rot = doc.effective_rotate(pid);
        if rot != 0 && !dict.contains_key("Rotate") {
            dict.insert("Rotate".into(), PdfVal::Int(rot));
        }
        dict.insert("Parent".into(), PdfVal::Ref(pages_root_id, 0));
        overrides.insert(pid, PdfVal::Dict(dict));
    }
    Ok(())
}

fn read_pdf_outline_nodes(
    src: &ParsedPdf,
    src_pages: &[u32],
    src_pid_to_merged_idx: &HashMap<u32, usize>,
) -> Vec<OutlineNodeSpec> {
    let Some(catalog) = src.get_dict(src.root_id) else {
        return Vec::new();
    };
    let Some(outlines_val) = catalog.get("Outlines") else {
        return Vec::new();
    };
    let Some(PdfVal::Dict(outlines_dict)) = src.resolve_val(outlines_val) else {
        return Vec::new();
    };
    let mut name_dests: HashMap<String, usize> = HashMap::new();
    collect_named_dests(src, &catalog, &mut name_dests);

    let first_id = outlines_dict.get("First").and_then(PdfVal::as_ref_id);
    walk_outline_chain(
        src,
        first_id,
        src_pages,
        src_pid_to_merged_idx,
        &name_dests,
        0,
    )
}

fn collect_named_dests(
    src: &ParsedPdf,
    catalog: &BTreeMap<String, PdfVal>,
    out: &mut HashMap<String, usize>,
) {
    if let Some(names_val) = catalog.get("Names")
        && let Some(PdfVal::Dict(names_dict)) = src.resolve_val(names_val)
        && let Some(dests_val) = names_dict.get("Dests")
        && let Some(PdfVal::Dict(dests_tree)) = src.resolve_val(dests_val)
    {
        walk_dest_name_tree(src, &dests_tree, out, 0);
    }
}

fn walk_dest_name_tree(
    src: &ParsedPdf,
    node: &BTreeMap<String, PdfVal>,
    out: &mut HashMap<String, usize>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    if let Some(names_val) = node.get("Names")
        && let Some(PdfVal::Array(arr)) = src.resolve_val(names_val)
    {
        for pair in arr.as_chunks::<2>().0 {
            if let Some(name) = pair[0].as_text_string()
                && let Some(PdfVal::Array(dest_arr)) = src.resolve_val(&pair[1])
                && let Some(pid) = dest_arr.first().and_then(PdfVal::as_ref_id)
            {
                out.insert(name, pid as usize);
            }
        }
    }
    if let Some(kids_val) = node.get("Kids")
        && let Some(PdfVal::Array(kids)) = src.resolve_val(kids_val)
    {
        for kid in kids {
            if let Some(PdfVal::Dict(kd)) = src.resolve_val(&kid) {
                walk_dest_name_tree(src, &kd, out, depth + 1);
            }
        }
    }
}

fn walk_outline_chain(
    src: &ParsedPdf,
    mut current: Option<u32>,
    src_pages: &[u32],
    pid_map: &HashMap<u32, usize>,
    name_dests: &HashMap<String, usize>,
    depth: usize,
) -> Vec<OutlineNodeSpec> {
    if depth > 32 {
        return Vec::new();
    }
    let mut nodes = Vec::new();
    let mut visited = BTreeSet::new();
    while let Some(item_id) = current {
        if !visited.insert(item_id) || visited.len() > 10_000 {
            break;
        }
        let Some(dict) = src.get_dict(item_id) else {
            break;
        };
        let title = dict
            .get("Title")
            .and_then(|v| src.resolve_val(v))
            .and_then(|v| v.as_text_string())
            .unwrap_or_default();

        let (dest_page, y_pt, uri) =
            resolve_outline_target(src, &dict, src_pages, pid_map, name_dests);
        let children = walk_outline_chain(
            src,
            dict.get("First").and_then(PdfVal::as_ref_id),
            src_pages,
            pid_map,
            name_dests,
            depth + 1,
        );
        if dest_page.is_some() || uri.is_some() || !children.is_empty() {
            nodes.push(OutlineNodeSpec {
                title,
                dest_page,
                y_pt,
                uri,
                children,
            });
        }
        current = dict.get("Next").and_then(PdfVal::as_ref_id);
    }
    nodes
}

fn resolve_outline_target(
    src: &ParsedPdf,
    dict: &BTreeMap<String, PdfVal>,
    src_pages: &[u32],
    pid_map: &HashMap<u32, usize>,
    name_dests: &HashMap<String, usize>,
) -> (Option<usize>, Option<f32>, Option<String>) {
    if let Some(dest) = dict.get("Dest")
        && let Some((p, y)) = resolve_dest_val(src, dest, src_pages, pid_map, name_dests)
    {
        return (Some(p), y, None);
    }
    if let Some(action_val) = dict.get("A")
        && let Some(PdfVal::Dict(action)) = src.resolve_val(action_val)
    {
        match action.get("S").and_then(PdfVal::as_name) {
            Some("GoTo") => {
                if let Some(d) = action.get("D")
                    && let Some((p, y)) = resolve_dest_val(src, d, src_pages, pid_map, name_dests)
                {
                    return (Some(p), y, None);
                }
            }
            Some("URI") => {
                if let Some(uri) = action
                    .get("URI")
                    .and_then(|v| src.resolve_val(v))
                    .and_then(|v| v.as_text_string())
                {
                    return (None, None, Some(uri));
                }
            }
            _ => {}
        }
    }
    (None, None, None)
}

fn resolve_dest_val(
    src: &ParsedPdf,
    dest: &PdfVal,
    src_pages: &[u32],
    pid_map: &HashMap<u32, usize>,
    name_dests: &HashMap<String, usize>,
) -> Option<(usize, Option<f32>)> {
    let resolved = src.resolve_val(dest)?;
    match resolved {
        PdfVal::Array(arr) => {
            let page_ref = arr.first()?;
            let src_pid = match page_ref {
                PdfVal::Ref(id, _) => *id,
                PdfVal::Int(idx) => *src_pages.get(usize::try_from(*idx).ok()?)?,
                _ => return None,
            };
            let merged_idx = *pid_map.get(&src_pid)?;
            let y_pt = if arr.get(1).and_then(PdfVal::as_name) == Some("XYZ") {
                let top = arr.get(3).and_then(PdfVal::as_f32);
                let mb = src
                    .effective_box(src_pid, "MediaBox")
                    .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                top.map(|t| mb[3] - t)
            } else {
                None
            };
            Some((merged_idx, y_pt))
        }
        PdfVal::Str(_) | PdfVal::HexStr(_) | PdfVal::Name(_) => {
            let name = resolved
                .as_name()
                .map(str::to_owned)
                .or_else(|| resolved.as_text_string())?;
            let &src_pid = name_dests.get(&name)?;
            let merged_idx = *pid_map.get(&(src_pid as u32))?;
            Some((merged_idx, None))
        }
        _ => None,
    }
}

fn write_outline_tree(
    writer: &mut RawPdfWriter,
    nodes: &[OutlineNodeSpec],
    page_ids: &[u32],
    page_boxes: &[[f32; 4]],
) -> Option<u32> {
    if nodes.is_empty() {
        return None;
    }
    let root_id = writer.alloc();
    let (first_id, last_id, count) =
        write_outline_level(writer, root_id, nodes, page_ids, page_boxes)?;
    writer.set(
        root_id,
        format!("<< /Type /Outlines /First {first_id} 0 R /Last {last_id} 0 R /Count {count} >>")
            .into_bytes(),
    );
    Some(root_id)
}

fn write_outline_level(
    writer: &mut RawPdfWriter,
    parent_id: u32,
    nodes: &[OutlineNodeSpec],
    page_ids: &[u32],
    page_boxes: &[[f32; 4]],
) -> Option<(u32, u32, usize)> {
    if nodes.is_empty() {
        return None;
    }
    let ids: Vec<u32> = (0..nodes.len()).map(|_| writer.alloc()).collect();
    let mut total_count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let child_info = write_outline_level(writer, ids[i], &node.children, page_ids, page_boxes);
        if let Some((_, _, child_count)) = child_info {
            total_count += child_count;
        }
        let mut dict = format!(
            "<< /Title ({}) /Parent {parent_id} 0 R",
            escape_pdf_string(&node.title)
        );
        if i > 0 {
            let _ = write!(dict, " /Prev {} 0 R", ids[i - 1]);
        }
        if i + 1 < ids.len() {
            let _ = write!(dict, " /Next {} 0 R", ids[i + 1]);
        }
        if let Some((c_first, c_last, c_count)) = child_info {
            let _ = write!(
                dict,
                " /First {c_first} 0 R /Last {c_last} 0 R /Count {c_count}"
            );
        }
        if let Some(p) = node.dest_page
            && let Some(&pid) = page_ids.get(p)
        {
            let mb = page_boxes
                .get(p)
                .copied()
                .unwrap_or([0.0, 0.0, 612.0, 792.0]);
            match node.y_pt {
                Some(y) => {
                    let _ = write!(
                        dict,
                        " /Dest [{pid} 0 R /XYZ {} {} 0]",
                        fmt_f32(mb[0]),
                        fmt_f32(mb[3] - y)
                    );
                }
                None => {
                    let _ = write!(dict, " /Dest [{pid} 0 R /Fit]");
                }
            }
        } else if let Some(uri) = &node.uri {
            let _ = write!(dict, " /A << /S /URI /URI ({}) >>", escape_pdf_string(uri));
        }
        dict.push_str(" >>");
        writer.set(ids[i], dict.into_bytes());
    }
    Some((ids[0], ids[ids.len() - 1], total_count))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use excellent_mupdf::{Document, PageBox};

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
