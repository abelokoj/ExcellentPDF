//! Invisible PDF text-layer (`3 Tr`) placement planner and vector-preserving incremental PDF writer.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use excellent_mupdf::{Background, Document, Matrix, PageRect};
use flate2::read::ZlibDecoder;

use super::engine::OcrEngine;
use super::model::{
    ExistingTextPolicy, OcrConfig, OcrError, OcrPageResult, PageImageInput, PageScanStatus,
    detect_scanned_page,
};

/// Resource name given to the Helvetica font used by OCR invisible text streams.
const OCR_FONT_NAME: &str = "ExOcrF1";

/// Approximate average advance width of Helvetica glyphs as a fraction of font size.
const HELVETICA_AVG_ADVANCE: f32 = 0.52;

/// One word positioned in PDF user space (y-up, points) for invisible text rendering (`3 Tr`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextPlacement {
    /// Recognized word string.
    pub word: String,
    /// Baseline x-origin in PDF points.
    pub x_pts: f32,
    /// Baseline y-origin in PDF points (y-up).
    pub y_pts: f32,
    /// Font size in PDF points.
    pub font_size_pts: f32,
    /// Horizontal scaling factor percentage (`Tz`, 100.0 = unscaled).
    pub scale_x_pct: f32,
    /// Bounding box in top-down page points.
    pub box_pts: PageRect,
}

/// Complete invisible text-layer plan for one PDF page.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayerPlan {
    /// Zero-based page index.
    pub page_index: usize,
    /// Page width in PDF points.
    pub page_width_pts: f32,
    /// Page height in PDF points.
    pub page_height_pts: f32,
    /// Individual word placements.
    pub placements: Vec<TextPlacement>,
    /// PDF content stream operators (`Q\nq\nBT 3 Tr ... ET\nQ`).
    pub content_stream_ops: String,
}

/// Per-page summary returned by [`ocr_pdf_bytes`] and [`ocr_pdf_file`].
#[derive(Debug, Clone, PartialEq)]
pub struct OcrPageReport {
    /// Zero-based page index.
    pub page_index: usize,
    /// Whether OCR ran on this page (`false` when skipped due to existing text or empty page).
    pub ocred: bool,
    /// Number of words placed into the invisible text layer.
    pub words_placed: usize,
    /// Mean confidence (`0.0..=100.0`) when OCR ran.
    pub mean_confidence: f32,
    /// Estimated/applied skew angle in degrees.
    pub skew_degrees: f32,
    /// Extracted/recognized text preview.
    pub text: String,
}

/// Summary of OCR over an entire PDF document.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrDocumentReport {
    /// Total number of pages in the document.
    pub page_count: usize,
    /// Number of pages that received an invisible OCR text layer.
    pub pages_ocred: usize,
    /// Total words added across all OCR'd pages.
    pub total_words: usize,
    /// Per-page reports in page order.
    pub pages: Vec<OcrPageReport>,
}

/// Converts pixel-space OCR results into a PDF invisible-text (`3 Tr`) content stream plan.
#[must_use]
pub fn build_text_layer_plan(result: &OcrPageResult, page_bounds: PageRect) -> TextLayerPlan {
    let page_width_pts = page_bounds.width().max(1.0);
    let page_height_pts = page_bounds.height().max(1.0);
    let img_w = (result.image_width.max(1)) as f32;
    let img_h = (result.image_height.max(1)) as f32;
    let sx = page_width_pts / img_w;
    let sy = page_height_pts / img_h;

    let mut placements = Vec::with_capacity(result.words.len());
    // Leading `Q` closes the `q` wrapper prepended around original content streams so existing CTMs never skew OCR text.
    let mut ops = String::from("Q\nq\nBT\n3 Tr\n");

    for word in &result.words {
        if word.text.trim().is_empty() {
            continue;
        }
        let left_pts = page_bounds.x0 + word.left_px * sx;
        let right_pts = page_bounds.x0 + word.right_px.max(word.left_px + 1.0) * sx;
        let top_pts = page_bounds.y0 + word.top_px * sy;
        let bottom_pts = page_bounds.y0 + word.bottom_px.max(word.top_px + 1.0) * sy;

        let box_h = (bottom_pts - top_pts).max(4.0);
        let box_w = (right_pts - left_pts).max(2.0);
        let font_size_pts = (box_h * 0.85).clamp(4.0, 200.0);

        let baseline_top_down = word
            .baseline_px
            .map_or(bottom_pts - box_h * 0.15, |b| page_bounds.y0 + b * sy);
        // Convert y-down page points into y-up PDF user space relative to page origin.
        let y_pts = (page_bounds.y1 - baseline_top_down).clamp(0.0, page_height_pts);
        let x_pts = left_pts;

        let char_count = word.text.chars().count().max(1) as f32;
        let est_unscaled_w = (char_count * font_size_pts * HELVETICA_AVG_ADVANCE).max(1.0);
        let scale_x_pct = ((box_w / est_unscaled_w) * 100.0).clamp(25.0, 400.0);

        let escaped = escape_pdf_literal(&word.text);
        let _ = writeln!(
            ops,
            "/{OCR_FONT_NAME} {font_size_pts:.2} Tf {scale_x_pct:.2} Tz 1 0 0 1 {x_pts:.2} {y_pts:.2} Tm ({escaped}) Tj"
        );
        placements.push(TextPlacement {
            word: word.text.clone(),
            x_pts,
            y_pts,
            font_size_pts,
            scale_x_pct,
            box_pts: PageRect {
                x0: left_pts,
                y0: top_pts,
                x1: right_pts,
                y1: bottom_pts,
            },
        });
    }

    ops.push_str("ET\nQ\n");
    TextLayerPlan {
        page_index: result.page_index,
        page_width_pts,
        page_height_pts,
        placements,
        content_stream_ops: ops,
    }
}

fn escape_pdf_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            // Map common Latin-1 / Windows-1252 printable characters cleanly for WinAnsiEncoding; replace others with '?'.
            c if (0xA0..=0xFF).contains(&(c as u32)) => {
                let byte = c as u8;
                let _ = write!(out, "\\{byte:03o}");
            }
            _ => out.push('?'),
        }
    }
    out
}

/// Runs OCR over `input_pdf` and writes a searchable PDF to `output_pdf` with invisible `3 Tr` text layers,
/// preserving all original vector graphics, images, annotations, and outlines.
///
/// # Errors
///
/// Returns [`OcrError`] if reading, rendering, OCR, or PDF updating fails.
pub fn ocr_pdf_file(
    input_pdf: &Path,
    output_pdf: &Path,
    engine: &dyn OcrEngine,
    config: &OcrConfig,
) -> Result<OcrDocumentReport, OcrError> {
    let bytes = fs::read(input_pdf)?;
    let (out_bytes, report) = ocr_pdf_bytes(&bytes, engine, config)?;
    if let Some(parent) = output_pdf.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_pdf, &out_bytes)?;
    Ok(report)
}

/// Runs OCR over an in-memory PDF byte slice and returns the updated searchable PDF bytes and report.
///
/// # Errors
///
/// Returns [`OcrError`] if MuPDF cannot open/render the PDF, the OCR engine fails, or the PDF update fails.
pub fn ocr_pdf_bytes(
    pdf_bytes: &[u8],
    engine: &dyn OcrEngine,
    config: &OcrConfig,
) -> Result<(Vec<u8>, OcrDocumentReport), OcrError> {
    let doc = Document::from_bytes(pdf_bytes, "application/pdf")?;
    let page_count = doc.page_count()?;
    let mut plans = Vec::new();
    let mut pages = Vec::with_capacity(page_count);
    let mut pages_ocred = 0usize;
    let mut total_words = 0usize;

    for page_index in 0..page_count {
        let should_skip = if config.existing_text == ExistingTextPolicy::SkipPagesWithText {
            matches!(
                detect_scanned_page(&doc, page_index)?,
                PageScanStatus::HasSelectableText { .. }
            )
        } else {
            false
        };

        if should_skip {
            let existing_text = doc.load_page(page_index)?.text(None)?.text()?;
            pages.push(OcrPageReport {
                page_index,
                ocred: false,
                words_placed: 0,
                mean_confidence: 100.0,
                skew_degrees: 0.0,
                text: existing_text,
            });
            continue;
        }

        let page = doc.load_page(page_index)?;
        let bounds = page.bounds()?;
        let scale = (config.dpi.max(72) as f32) / 72.0;
        let pixmap = page.render(&Matrix::scale(scale, scale), Background::White)?;
        let img = PageImageInput::from_mupdf_pixmap(page_index, &pixmap, bounds, config.dpi)?;
        let ocr_res = engine.recognize_page(&img, config)?;
        let plan = build_text_layer_plan(&ocr_res, bounds);

        let word_count = plan.placements.len();
        if word_count > 0 {
            plans.push(plan);
            pages_ocred += 1;
            total_words += word_count;
        }
        pages.push(OcrPageReport {
            page_index,
            ocred: word_count > 0,
            words_placed: word_count,
            mean_confidence: ocr_res.mean_confidence,
            skew_degrees: ocr_res.skew_degrees,
            text: ocr_res.full_text,
        });
    }

    let out_bytes = if plans.is_empty() {
        pdf_bytes.to_vec()
    } else {
        apply_text_layer_to_pdf_bytes(pdf_bytes, &plans)?
    };
    Ok((
        out_bytes,
        OcrDocumentReport {
            page_count,
            pages_ocred,
            total_words,
            pages,
        },
    ))
}

/// Appends invisible `3 Tr` text-layer streams to `pdf_bytes` via a vector-preserving incremental PDF update.
///
/// # Errors
///
/// Returns [`OcrError::InvalidInput`] if `pdf_bytes` is not a readable PDF or a target page index is out of range.
pub fn apply_text_layer_to_pdf_bytes(
    pdf_bytes: &[u8],
    plans: &[TextLayerPlan],
) -> Result<Vec<u8>, OcrError> {
    if plans.is_empty() {
        return Ok(pdf_bytes.to_vec());
    }

    let parsed = PdfIndex::parse(pdf_bytes)?;
    let pages = parsed.collect_pages()?;
    for plan in plans {
        if plan.page_index >= pages.len() {
            return Err(OcrError::InvalidInput(format!(
                "page index {} out of range ({} pages)",
                plan.page_index,
                pages.len()
            )));
        }
    }

    let mut out = pdf_bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }

    let mut next_id = parsed.size;
    let mut new_entries: BTreeMap<u32, usize> = BTreeMap::new();

    // Shared Helvetica Type1 font object.
    let font_id = next_id;
    next_id += 1;
    let font_offset = out.len();
    let font_obj = format!(
        "{font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n"
    );
    out.extend_from_slice(font_obj.as_bytes());
    new_entries.insert(font_id, font_offset);

    // Shared `q` graphics-state save stream prepended before existing page contents.
    let q_id = next_id;
    next_id += 1;
    let q_offset = out.len();
    let q_obj = format!("{q_id} 0 obj\n<< /Length 2 >>\nstream\nq\nendstream\nendobj\n");
    out.extend_from_slice(q_obj.as_bytes());
    new_entries.insert(q_id, q_offset);

    for plan in plans {
        let page_ref = pages[plan.page_index];
        let page_body = parsed.read_object(page_ref.0)?;
        let dict_inner = extract_dict_inner(&page_body).ok_or_else(|| {
            OcrError::InvalidInput(format!("page object {} has no dictionary", page_ref.0))
        })?;

        // Emit invisible text overlay stream.
        let text_stream_id = next_id;
        next_id += 1;
        let stream_bytes = plan.content_stream_ops.as_bytes();
        let text_offset = out.len();
        let header = format!(
            "{text_stream_id} 0 obj\n<< /Length {} >>\nstream\n",
            stream_bytes.len()
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(stream_bytes);
        out.extend_from_slice(b"endstream\nendobj\n");
        new_entries.insert(text_stream_id, text_offset);

        // Rebuild /Contents array: [ q_id 0 R <existing...> text_stream_id 0 R ]
        let new_contents = build_updated_contents(dict_inner, q_id, text_stream_id);

        // Update /Resources (or indirect /Resources dict) with /Font << ... /ExOcrF1 font_id 0 R >>.
        let updated_page_dict = update_page_dict(
            &parsed,
            dict_inner,
            &new_contents,
            font_id,
            &mut next_id,
            &mut out,
            &mut new_entries,
        )?;

        let page_offset = out.len();
        let page_obj = format!(
            "{} {} obj\n<< {} >>\nendobj\n",
            page_ref.0, page_ref.1, updated_page_dict
        );
        out.extend_from_slice(page_obj.as_bytes());
        new_entries.insert(page_ref.0, page_offset);
    }

    if parsed.uses_xref_stream {
        write_incremental_xref_stream(&mut out, &new_entries, next_id, &parsed)?;
    } else {
        write_incremental_xref_table(&mut out, &new_entries, next_id, &parsed)?;
    }

    Ok(out)
}

fn write_incremental_xref_table(
    out: &mut Vec<u8>,
    entries: &BTreeMap<u32, usize>,
    total_size: u32,
    parsed: &PdfIndex<'_>,
) -> Result<(), OcrError> {
    let xref_offset = out.len();
    out.extend_from_slice(b"xref\n");
    for (&obj_id, &offset) in entries {
        let line = format!("{obj_id} 1\n{offset:010} 00000 n \n");
        out.extend_from_slice(line.as_bytes());
    }
    let root = parsed
        .root
        .ok_or_else(|| OcrError::InvalidInput("pdf trailer missing /Root".to_owned()))?;
    let mut trailer = format!(
        "trailer\n<< /Size {total_size} /Root {} {} R /Prev {}",
        root.0, root.1, parsed.last_startxref
    );
    if let Some(info) = parsed.info {
        let _ = write!(trailer, " /Info {} {} R", info.0, info.1);
    }
    trailer.push_str(" >>\nstartxref\n");
    let _ = writeln!(trailer, "{xref_offset}\n%%EOF");
    out.extend_from_slice(trailer.as_bytes());
    Ok(())
}

fn write_incremental_xref_stream(
    out: &mut Vec<u8>,
    entries: &BTreeMap<u32, usize>,
    next_id: u32,
    parsed: &PdfIndex<'_>,
) -> Result<(), OcrError> {
    let xref_obj_id = next_id;
    let total_size = next_id + 1;
    let xref_offset = out.len();

    let mut full = entries.clone();
    full.insert(xref_obj_id, xref_offset);

    let mut index_arr = String::from("[");
    let mut stream = Vec::with_capacity(full.len() * 7);
    for (&obj_id, &offset) in &full {
        let _ = write!(index_arr, " {obj_id} 1");
        stream.push(1u8);
        stream.extend_from_slice(&(offset as u32).to_be_bytes());
        stream.extend_from_slice(&0u16.to_be_bytes());
    }
    index_arr.push_str(" ]");

    let root = parsed
        .root
        .ok_or_else(|| OcrError::InvalidInput("pdf trailer missing /Root".to_owned()))?;
    let mut dict = format!(
        "{xref_obj_id} 0 obj\n<< /Type /XRef /Size {total_size} /W [1 4 2] /Index {index_arr} /Root {} {} R /Prev {} /Length {} >>\nstream\n",
        root.0,
        root.1,
        parsed.last_startxref,
        stream.len()
    );
    out.extend_from_slice(dict.as_bytes());
    out.extend_from_slice(&stream);
    dict.clear();
    let _ = writeln!(dict, "\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF");
    out.extend_from_slice(dict.as_bytes());
    Ok(())
}

// ---- Minimal PDF cross-reference & dictionary inspector ----

#[derive(Debug, Clone, Copy)]
enum XrefLoc {
    Direct(usize),
    Compressed { stm_obj: u32, index: usize },
}

struct PdfIndex<'a> {
    bytes: &'a [u8],
    objects: BTreeMap<u32, XrefLoc>,
    size: u32,
    root: Option<(u32, u16)>,
    info: Option<(u32, u16)>,
    last_startxref: usize,
    uses_xref_stream: bool,
}

impl<'a> PdfIndex<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, OcrError> {
        let last_startxref = find_last_startxref(bytes)?;
        let mut idx = Self {
            bytes,
            objects: BTreeMap::new(),
            size: 1,
            root: None,
            info: None,
            last_startxref,
            uses_xref_stream: false,
        };
        let mut visited = Vec::new();
        let mut cursor = Some(last_startxref);
        let mut first = true;
        while let Some(offset) = cursor {
            if visited.contains(&offset) || offset >= bytes.len() {
                break;
            }
            visited.push(offset);
            let slice = skip_ws(&bytes[offset..]);
            if slice.starts_with(b"xref") {
                cursor = idx.parse_classic_xref(offset)?;
            } else {
                if first {
                    idx.uses_xref_stream = true;
                }
                cursor = idx.parse_xref_stream(offset)?;
            }
            first = false;
        }
        let max_id = idx.objects.keys().next_back().copied().unwrap_or(0);
        if idx.size <= max_id {
            idx.size = max_id + 1;
        }
        Ok(idx)
    }

    fn parse_classic_xref(&mut self, offset: usize) -> Result<Option<usize>, OcrError> {
        let text = String::from_utf8_lossy(&self.bytes[offset..]);
        let after_xref = text
            .strip_prefix("xref")
            .ok_or_else(|| OcrError::InvalidInput("expected xref keyword".to_owned()))?;
        let trailer_pos = after_xref
            .find("trailer")
            .ok_or_else(|| OcrError::InvalidInput("expected trailer after xref".to_owned()))?;
        let table = &after_xref[..trailer_pos];
        let mut tokens = table.split_whitespace();
        while let Some(tok) = tokens.next() {
            let start_id: u32 = tok
                .parse()
                .map_err(|_| OcrError::InvalidInput("invalid xref start id".to_owned()))?;
            let count: u32 = tokens
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| OcrError::InvalidInput("invalid xref count".to_owned()))?;
            for i in 0..count {
                let off_tok = tokens
                    .next()
                    .ok_or_else(|| OcrError::InvalidInput("truncated xref entry".to_owned()))?;
                let _gen_tok = tokens
                    .next()
                    .ok_or_else(|| OcrError::InvalidInput("truncated xref gen".to_owned()))?;
                let flag = tokens
                    .next()
                    .ok_or_else(|| OcrError::InvalidInput("truncated xref flag".to_owned()))?;
                if flag == "n"
                    && let Ok(obj_off) = off_tok.parse::<usize>()
                {
                    self.objects
                        .entry(start_id + i)
                        .or_insert(XrefLoc::Direct(obj_off));
                }
            }
        }
        let trailer_slice = &after_xref[trailer_pos..];
        let dict_inner = extract_dict_inner(trailer_slice)
            .ok_or_else(|| OcrError::InvalidInput("missing trailer dict".to_owned()))?;
        if let Some(size) = parse_int_field(dict_inner, "Size") {
            self.size = self.size.max(size as u32);
        }
        if self.root.is_none() {
            self.root = parse_ref_field(dict_inner, "Root");
        }
        if self.info.is_none() {
            self.info = parse_ref_field(dict_inner, "Info");
        }
        Ok(parse_int_field(dict_inner, "Prev").and_then(|p| usize::try_from(p).ok()))
    }

    fn parse_xref_stream(&mut self, offset: usize) -> Result<Option<usize>, OcrError> {
        let body = read_indirect_object_bytes(self.bytes, offset)?;
        let header_end = find_subslice(body, b"stream").ok_or_else(|| {
            OcrError::InvalidInput("xref stream missing stream keyword".to_owned())
        })?;
        let header_str = String::from_utf8_lossy(&body[..header_end]);
        let dict_inner = extract_dict_inner(&header_str)
            .ok_or_else(|| OcrError::InvalidInput("xref stream missing dict".to_owned()))?;
        if let Some(size) = parse_int_field(dict_inner, "Size") {
            self.size = self.size.max(size as u32);
        }
        if self.root.is_none() {
            self.root = parse_ref_field(dict_inner, "Root");
        }
        if self.info.is_none() {
            self.info = parse_ref_field(dict_inner, "Info");
        }
        let w_arr = parse_int_array_field(dict_inner, "W")
            .filter(|w| w.len() == 3)
            .ok_or_else(|| OcrError::InvalidInput("xref stream missing /W [3]".to_owned()))?;
        let (w1, w2, w3) = (w_arr[0] as usize, w_arr[1] as usize, w_arr[2] as usize);
        let row_len = w1 + w2 + w3;

        let raw_stream = extract_stream_bytes(body, header_end)?;
        let decoded = if dict_inner.contains("/FlateDecode") {
            let mut dec = ZlibDecoder::new(raw_stream);
            let mut buf = Vec::new();
            dec.read_to_end(&mut buf)
                .map_err(|err| OcrError::InvalidInput(format!("xref zlib decode failed: {err}")))?;
            // Handle PNG predictor (/Predictor 12 etc.) if present.
            if let Some(pred) = parse_int_field(dict_inner, "Predictor")
                && pred >= 10
            {
                let cols =
                    parse_int_field(dict_inner, "Columns").unwrap_or(row_len as i64) as usize;
                unfilter_png_up(&buf, cols)?
            } else {
                buf
            }
        } else {
            raw_stream.to_vec()
        };

        let index_pairs = parse_int_array_field(dict_inner, "Index")
            .unwrap_or_else(|| vec![0, i64::from(self.size)]);
        let mut cursor = 0usize;
        for &[start_raw, count_raw] in index_pairs.as_chunks::<2>().0 {
            let start = start_raw as u32;
            let count = count_raw as u32;
            for i in 0..count {
                if cursor + row_len > decoded.len() {
                    break;
                }
                let row = &decoded[cursor..cursor + row_len];
                cursor += row_len;
                let typ = if w1 == 0 { 1 } else { read_be_uint(&row[..w1]) };
                let f2 = read_be_uint(&row[w1..w1 + w2]);
                let f3 = read_be_uint(&row[w1 + w2..row_len]);
                match typ {
                    1 => {
                        self.objects
                            .entry(start + i)
                            .or_insert(XrefLoc::Direct(f2 as usize));
                    }
                    2 => {
                        self.objects
                            .entry(start + i)
                            .or_insert(XrefLoc::Compressed {
                                stm_obj: f2 as u32,
                                index: f3 as usize,
                            });
                    }
                    _ => {}
                }
            }
        }

        Ok(parse_int_field(dict_inner, "Prev").and_then(|p| usize::try_from(p).ok()))
    }

    fn read_object(&self, obj_id: u32) -> Result<String, OcrError> {
        let Some(&loc) = self.objects.get(&obj_id) else {
            return Err(OcrError::InvalidInput(format!(
                "missing pdf object {obj_id}"
            )));
        };
        match loc {
            XrefLoc::Direct(offset) => {
                let raw = read_indirect_object_bytes(self.bytes, offset)?;
                Ok(String::from_utf8_lossy(raw).into_owned())
            }
            XrefLoc::Compressed { stm_obj, index } => {
                let Some(XrefLoc::Direct(stm_off)) = self.objects.get(&stm_obj) else {
                    return Err(OcrError::InvalidInput(format!(
                        "missing object stream {stm_obj}"
                    )));
                };
                let raw = read_indirect_object_bytes(self.bytes, *stm_off)?;
                let hdr_end = find_subslice(raw, b"stream").ok_or_else(|| {
                    OcrError::InvalidInput("object stream missing stream keyword".to_owned())
                })?;
                let hdr = String::from_utf8_lossy(&raw[..hdr_end]);
                let dict = extract_dict_inner(&hdr).unwrap_or("");
                let first = parse_int_field(dict, "First").unwrap_or(0) as usize;
                let n = parse_int_field(dict, "N").unwrap_or(0) as usize;
                let raw_stm = extract_stream_bytes(raw, hdr_end)?;
                let decoded = if dict.contains("/FlateDecode") {
                    let mut dec = ZlibDecoder::new(raw_stm);
                    let mut buf = Vec::new();
                    dec.read_to_end(&mut buf).map_err(|err| {
                        OcrError::InvalidInput(format!("objstm decode error: {err}"))
                    })?;
                    buf
                } else {
                    raw_stm.to_vec()
                };
                if index >= n || first > decoded.len() {
                    return Err(OcrError::InvalidInput(format!(
                        "object index {index} out of bounds in objstm {stm_obj}"
                    )));
                }
                let header_tokens: Vec<usize> = String::from_utf8_lossy(&decoded[..first])
                    .split_whitespace()
                    .filter_map(|t| t.parse::<usize>().ok())
                    .collect();
                let start = first + header_tokens.get(index * 2 + 1).copied().unwrap_or(0);
                let end = header_tokens
                    .get((index + 1) * 2 + 1)
                    .map_or(decoded.len(), |&off| first + off)
                    .min(decoded.len());
                Ok(String::from_utf8_lossy(&decoded[start..end]).into_owned())
            }
        }
    }

    fn collect_pages(&self) -> Result<Vec<(u32, u16)>, OcrError> {
        let root = self
            .root
            .ok_or_else(|| OcrError::InvalidInput("pdf missing /Root".to_owned()))?;
        let catalog = self.read_object(root.0)?;
        let catalog_dict = extract_dict_inner(&catalog)
            .ok_or_else(|| OcrError::InvalidInput("catalog missing dict".to_owned()))?;
        let pages_root = parse_ref_field(catalog_dict, "Pages")
            .ok_or_else(|| OcrError::InvalidInput("catalog missing /Pages".to_owned()))?;
        let mut out = Vec::new();
        let mut visited = Vec::new();
        self.walk_pages_node(pages_root, &mut visited, &mut out)?;
        Ok(out)
    }

    fn walk_pages_node(
        &self,
        node: (u32, u16),
        visited: &mut Vec<u32>,
        out: &mut Vec<(u32, u16)>,
    ) -> Result<(), OcrError> {
        if visited.contains(&node.0) {
            return Ok(());
        }
        visited.push(node.0);
        let obj = self.read_object(node.0)?;
        let dict = extract_dict_inner(&obj).unwrap_or("");
        if let Some(kids) = parse_ref_array_field(dict, "Kids") {
            for kid in kids {
                self.walk_pages_node(kid, visited, out)?;
            }
        } else {
            out.push(node);
        }
        Ok(())
    }
}

fn find_last_startxref(bytes: &[u8]) -> Result<usize, OcrError> {
    let needle = b"startxref";
    let pos = bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .ok_or_else(|| OcrError::InvalidInput("missing startxref in pdf".to_owned()))?;
    let tail = String::from_utf8_lossy(&bytes[pos + needle.len()..]);
    tail.split_whitespace()
        .next()
        .and_then(|num| num.parse::<usize>().ok())
        .ok_or_else(|| OcrError::InvalidInput("invalid startxref offset".to_owned()))
}

fn skip_ws(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if b.is_ascii_whitespace() {
            s = &s[1..];
        } else {
            break;
        }
    }
    s
}

fn read_indirect_object_bytes(bytes: &[u8], offset: usize) -> Result<&[u8], OcrError> {
    if offset >= bytes.len() {
        return Err(OcrError::InvalidInput(
            "object offset out of bounds".to_owned(),
        ));
    }
    let slice = &bytes[offset..];
    let obj_pos = find_subslice(slice, b"obj")
        .ok_or_else(|| OcrError::InvalidInput("missing obj keyword".to_owned()))?;
    let after_obj = &slice[obj_pos + 3..];
    let end_pos = find_subslice(after_obj, b"endobj")
        .ok_or_else(|| OcrError::InvalidInput("missing endobj keyword".to_owned()))?;
    Ok(&after_obj[..end_pos])
}

fn extract_stream_bytes(obj_body: &[u8], stream_kw_pos: usize) -> Result<&[u8], OcrError> {
    let mut start = stream_kw_pos + b"stream".len();
    if obj_body.get(start) == Some(&b'\r') {
        start += 1;
    }
    if obj_body.get(start) == Some(&b'\n') {
        start += 1;
    }
    let end = find_subslice(&obj_body[start..], b"endstream")
        .map(|rel| start + rel)
        .ok_or_else(|| OcrError::InvalidInput("missing endstream".to_owned()))?;
    let mut trimmed = end;
    if trimmed > start && obj_body[trimmed - 1] == b'\n' {
        trimmed -= 1;
    }
    if trimmed > start && obj_body[trimmed - 1] == b'\r' {
        trimmed -= 1;
    }
    Ok(&obj_body[start..trimmed])
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn read_be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

fn unfilter_png_up(raw: &[u8], cols: usize) -> Result<Vec<u8>, OcrError> {
    let stride = cols + 1;
    if stride <= 1 || !raw.len().is_multiple_of(stride) {
        return Err(OcrError::InvalidInput(
            "invalid png-filtered xref stream".to_owned(),
        ));
    }
    let rows = raw.len() / stride;
    let mut out = vec![0u8; rows * cols];
    for r in 0..rows {
        let filter = raw[r * stride];
        let src_row = &raw[r * stride + 1..(r + 1) * stride];
        for c in 0..cols {
            let up = if r > 0 { out[(r - 1) * cols + c] } else { 0 };
            let val = match filter {
                0 => src_row[c],
                2 => src_row[c].wrapping_add(up),
                _ => src_row[c],
            };
            out[r * cols + c] = val;
        }
    }
    Ok(out)
}

fn extract_dict_inner(text: &str) -> Option<&str> {
    let start = text.find("<<")? + 2;
    let mut depth = 1usize;
    let bytes = text.as_bytes();
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'(' {
            // Skip literal string.
            i += 1;
            let mut paren_depth = 1usize;
            while i < bytes.len() && paren_depth > 0 {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'(' {
                    paren_depth += 1;
                } else if bytes[i] == b')' {
                    paren_depth -= 1;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'<' && bytes[i + 1] == b'<' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'>' && bytes[i + 1] == b'>' {
            depth -= 1;
            if depth == 0 {
                return Some(&text[start..i]);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

fn find_key_value_span(dict_inner: &str, key: &str) -> Option<(usize, usize)> {
    let bytes = dict_inner.as_bytes();
    let target = format!("/{key}");
    let target_bytes = target.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b')' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'<' {
            depth += 1;
            i += 2;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'>' && bytes[i + 1] == b'>' {
            depth = depth.saturating_sub(1);
            i += 2;
            continue;
        }
        if depth == 0 && bytes[i..].starts_with(target_bytes) {
            let after_key = i + target_bytes.len();
            if bytes
                .get(after_key)
                .is_some_and(|&b| b.is_ascii_whitespace() || b == b'[' || b == b'<' || b == b'/')
            {
                let mut val_start = after_key;
                while val_start < bytes.len() && bytes[val_start].is_ascii_whitespace() {
                    val_start += 1;
                }
                let val_end = scan_pdf_value_end(dict_inner, val_start);
                return Some((i, val_end));
            }
        }
        i += 1;
    }
    None
}

fn scan_pdf_value_end(s: &str, start: usize) -> usize {
    let bytes = s.as_bytes();
    if start >= bytes.len() {
        return start;
    }
    if bytes[start..].starts_with(b"<<") {
        let mut depth = 0usize;
        let mut i = start;
        while i + 1 < bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'<' {
                depth += 1;
                i += 2;
            } else if bytes[i] == b'>' && bytes[i + 1] == b'>' {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            } else {
                i += 1;
            }
        }
        return bytes.len();
    }
    if bytes[start] == b'[' {
        let mut depth = 0usize;
        let mut i = start;
        while i < bytes.len() {
            if bytes[i] == b'[' {
                depth += 1;
            } else if bytes[i] == b']' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            i += 1;
        }
        return bytes.len();
    }
    // Check for indirect ref `N G R` vs single token.
    let rest = &s[start..];
    let mut parts = rest.split_whitespace();
    if let (Some(a), Some(b), Some(c)) = (parts.next(), parts.next(), parts.next())
        && a.bytes().all(|ch| ch.is_ascii_digit())
        && b.bytes().all(|ch| ch.is_ascii_digit())
        && c.starts_with('R')
    {
        let offset_c = c.as_ptr() as usize - s.as_ptr() as usize;
        return offset_c + 1;
    }
    let mut i = start + 1;
    while i < bytes.len()
        && !bytes[i].is_ascii_whitespace()
        && !matches!(bytes[i], b'/' | b'<' | b'>' | b'[' | b']')
    {
        i += 1;
    }
    i
}

fn key_value_slice<'a>(dict_inner: &'a str, key: &str) -> Option<&'a str> {
    let (key_start, val_end) = find_key_value_span(dict_inner, key)?;
    let val_part = dict_inner[key_start..val_end]
        .strip_prefix(&format!("/{key}"))?
        .trim();
    Some(val_part)
}

fn parse_int_field(dict_inner: &str, key: &str) -> Option<i64> {
    key_value_slice(dict_inner, key)?.parse::<i64>().ok()
}

fn parse_ref_field(dict_inner: &str, key: &str) -> Option<(u32, u16)> {
    let val = key_value_slice(dict_inner, key)?;
    let mut parts = val.split_whitespace();
    let num: u32 = parts.next()?.parse().ok()?;
    let gen_num: u16 = parts.next()?.parse().ok()?;
    if parts.next()? == "R" {
        Some((num, gen_num))
    } else {
        None
    }
}

fn parse_int_array_field(dict_inner: &str, key: &str) -> Option<Vec<i64>> {
    let val = key_value_slice(dict_inner, key)?;
    let inner = val.strip_prefix('[')?.strip_suffix(']')?;
    Some(
        inner
            .split_whitespace()
            .filter_map(|t| t.parse::<i64>().ok())
            .collect(),
    )
}

fn parse_ref_array_field(dict_inner: &str, key: &str) -> Option<Vec<(u32, u16)>> {
    let val = key_value_slice(dict_inner, key)?;
    if let Some(inner) = val.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let toks: Vec<&str> = inner.split_whitespace().collect();
        let mut refs = Vec::new();
        for &[n_tok, g_tok, r_tok] in toks.as_chunks::<3>().0 {
            if r_tok == "R"
                && let (Ok(n), Ok(g)) = (n_tok.parse::<u32>(), g_tok.parse::<u16>())
            {
                refs.push((n, g));
            }
        }
        Some(refs)
    } else {
        None
    }
}

fn replace_or_insert_key(dict_inner: &str, key: &str, new_val: &str) -> String {
    if let Some((start, end)) = find_key_value_span(dict_inner, key) {
        let mut out = String::with_capacity(dict_inner.len() + new_val.len());
        out.push_str(&dict_inner[..start]);
        let _ = write!(out, "/{key} {new_val}");
        out.push_str(&dict_inner[end..]);
        out
    } else {
        format!("{} /{key} {new_val}", dict_inner.trim())
    }
}

fn build_updated_contents(dict_inner: &str, q_id: u32, text_stream_id: u32) -> String {
    match key_value_slice(dict_inner, "Contents") {
        Some(existing) if existing.starts_with('[') && existing.ends_with(']') => {
            let inner = &existing[1..existing.len() - 1];
            format!("[ {q_id} 0 R {} {text_stream_id} 0 R ]", inner.trim())
        }
        Some(existing) => {
            format!("[ {q_id} 0 R {existing} {text_stream_id} 0 R ]")
        }
        None => format!("[ {q_id} 0 R {text_stream_id} 0 R ]"),
    }
}

fn add_font_to_font_dict(font_dict_inner: &str, font_id: u32) -> String {
    replace_or_insert_key(font_dict_inner, OCR_FONT_NAME, &format!("{font_id} 0 R"))
}

fn update_resources_dict(
    parsed: &PdfIndex<'_>,
    res_inner: &str,
    font_id: u32,
    out: &mut Vec<u8>,
    new_entries: &mut BTreeMap<u32, usize>,
) -> Result<String, OcrError> {
    if let Some(font_val) = key_value_slice(res_inner, "Font") {
        if let Some(inner) = extract_dict_inner(font_val) {
            let updated_font = format!("<< {} >>", add_font_to_font_dict(inner, font_id));
            return Ok(replace_or_insert_key(res_inner, "Font", &updated_font));
        }
        if let Some((f_num, f_gen)) = parse_ref_field(res_inner, "Font") {
            let font_obj = parsed.read_object(f_num)?;
            let inner = extract_dict_inner(&font_obj).unwrap_or("");
            let updated_font = add_font_to_font_dict(inner, font_id);
            let off = out.len();
            let obj_str = format!("{f_num} {f_gen} obj\n<< {updated_font} >>\nendobj\n");
            out.extend_from_slice(obj_str.as_bytes());
            new_entries.insert(f_num, off);
            return Ok(res_inner.to_owned());
        }
    }
    Ok(replace_or_insert_key(
        res_inner,
        "Font",
        &format!("<< /{OCR_FONT_NAME} {font_id} 0 R >>"),
    ))
}

fn update_page_dict(
    parsed: &PdfIndex<'_>,
    dict_inner: &str,
    new_contents: &str,
    font_id: u32,
    _next_id: &mut u32,
    out: &mut Vec<u8>,
    new_entries: &mut BTreeMap<u32, usize>,
) -> Result<String, OcrError> {
    let mut page_dict = replace_or_insert_key(dict_inner, "Contents", new_contents);
    if let Some(res_val) = key_value_slice(&page_dict, "Resources") {
        if let Some(res_inner) = extract_dict_inner(res_val) {
            let updated_res = update_resources_dict(parsed, res_inner, font_id, out, new_entries)?;
            page_dict =
                replace_or_insert_key(&page_dict, "Resources", &format!("<< {updated_res} >>"));
        } else if let Some((r_num, r_gen)) = parse_ref_field(&page_dict, "Resources") {
            let res_obj = parsed.read_object(r_num)?;
            let res_inner = extract_dict_inner(&res_obj).unwrap_or("");
            let updated_res = update_resources_dict(parsed, res_inner, font_id, out, new_entries)?;
            let off = out.len();
            let obj_str = format!("{r_num} {r_gen} obj\n<< {updated_res} >>\nendobj\n");
            out.extend_from_slice(obj_str.as_bytes());
            new_entries.insert(r_num, off);
        }
    } else {
        page_dict = replace_or_insert_key(
            &page_dict,
            "Resources",
            &format!("<< /Font << /{OCR_FONT_NAME} {font_id} 0 R >> >>"),
        );
    }
    Ok(page_dict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::engine::DeterministicTestEngine;
    use crate::ocr::model::WordBox;

    fn corpus(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name)
    }

    #[test]
    fn text_layer_plan_emits_invisible_tr3_and_maps_y_up() {
        let res = OcrPageResult::from_words(
            0,
            1000,
            2000,
            vec![WordBox {
                text: "Invoice".to_owned(),
                left_px: 100.0,
                top_px: 200.0,
                right_px: 300.0,
                bottom_px: 260.0,
                confidence: 95.0,
                baseline_px: Some(250.0),
            }],
            0.0,
            "test",
        );
        let bounds = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 500.0,
            y1: 1000.0,
        };
        let plan = build_text_layer_plan(&res, bounds);
        assert!(plan.content_stream_ops.contains("3 Tr"));
        assert!(plan.content_stream_ops.contains("(Invoice) Tj"));
        assert_eq!(plan.placements.len(), 1);
        // top_px 200 out of 2000 -> 100 pts from top -> baseline ~875 pts from bottom.
        assert!((plan.placements[0].x_pts - 50.0).abs() < 0.1);
        assert!((plan.placements[0].y_pts - 875.0).abs() < 1.0);
    }

    #[test]
    fn ocr_adds_searchable_text_to_vector_pdf_preserving_original_graphics() {
        let pdf = fs::read(corpus("heavy-vector.pdf")).expect("read heavy-vector.pdf");
        let before_doc = Document::from_bytes(&pdf, "application/pdf").expect("open original");
        let before_text = before_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("stext")
            .text()
            .expect("text");
        assert!(!before_text.contains("EXCELLENT_OCR_TOKEN"));

        let engine = DeterministicTestEngine::with_text("EXCELLENT_OCR_TOKEN searchable layer");
        let config = OcrConfig {
            existing_text: ExistingTextPolicy::ForceAllPages,
            ..Default::default()
        };
        let (out_bytes, report) = ocr_pdf_bytes(&pdf, &engine, &config).expect("ocr succeeds");
        assert_eq!(report.page_count, 1);
        assert_eq!(report.pages_ocred, 1);
        assert_eq!(report.total_words, 3);
        // Incremental update preserves original bytes verbatim at the start of the output PDF.
        assert!(out_bytes.starts_with(&pdf[..pdf.len().min(512)]));

        let after_doc =
            Document::from_bytes(&out_bytes, "application/pdf").expect("reopen ocr pdf");
        let extracted = after_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("stext")
            .text()
            .expect("text");
        assert!(
            extracted.contains("EXCELLENT_OCR_TOKEN"),
            "extracted text was: {extracted:?}"
        );
        assert!(extracted.contains("searchable"));
        assert!(extracted.contains("layer"));
    }

    #[test]
    fn detect_scanned_distinguishes_text_pdf_from_non_text_pdf() {
        let text_doc = Document::open(corpus("base14-outline-annot.pdf")).expect("open text pdf");
        let status = detect_scanned_page(&text_doc, 0).expect("detect");
        assert!(matches!(
            status,
            PageScanStatus::HasSelectableText { char_count } if char_count > 0
        ));
    }
}
