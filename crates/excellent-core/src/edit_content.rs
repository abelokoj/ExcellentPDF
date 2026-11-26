//! In-place PDF text-run editing with box reflow and embedded image replacement/masking (plan O.5, O.6).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use excellent_mupdf::{PageRect, Quad};
use flate2::Compression;

use crate::page_ops::pdf_doc::{
    ParsedPdf, PdfDocError, PdfVal, StreamEncoding, decode_stream_data, escape_pdf_string, fmt_f32,
    make_stream, rewrite_clean,
};

/// Result alias for content-editing operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Policy for editing a PDF that contains digital signature fields (plan O.5, O.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SignatureGuard {
    /// Refuse to edit if the PDF contains `/Sig` or `/ByteRange` signatures.
    #[default]
    BlockIfSigned,
    /// Allow editing even if existing digital signatures will be invalidated.
    AllowInvalidatingSignatures,
}

/// Horizontal text alignment inside the reflow edit box (plan O.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TextBoxAlign {
    /// Flush left.
    #[default]
    Left,
    /// Centered horizontally.
    Center,
    /// Flush right.
    Right,
}

/// Standard PDF font family used when substituting missing or subsetted fonts (plan O.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StandardPdfFont {
    /// Helvetica (`/Helvetica`).
    #[default]
    Helvetica,
    /// Times Roman (`/Times-Roman`).
    TimesRoman,
    /// Courier (`/Courier`).
    Courier,
}

impl StandardPdfFont {
    fn base_font_name(self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::TimesRoman => "Times-Roman",
            Self::Courier => "Courier",
        }
    }

    fn from_hint(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.contains("cour") || lower.contains("mono") || lower.contains("consola") {
            Self::Courier
        } else if lower.contains("times") || lower.contains("serif") || lower.contains("garamond") {
            Self::TimesRoman
        } else {
            Self::Helvetica
        }
    }
}

/// Warning emitted when a requested font cannot be matched directly and a fallback was used (plan O.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontMatchWarning {
    /// Original font name requested.
    pub requested_font: String,
    /// Substitute font applied in the PDF resource dictionary.
    pub substituted_font: String,
    /// Human-readable reason.
    pub reason: String,
}

/// Specification for editing and reflowing a text run within a bounding box (plan O.5).
#[derive(Debug, Clone, PartialEq)]
pub struct TextRunEdit {
    /// Zero-based page index.
    pub page: usize,
    /// Target character/line quads of the original text run to remove.
    pub target_quads: Vec<Quad>,
    /// Optional literal text substring to remove if stream coordinates are indirect.
    pub original_text: String,
    /// Replacement text to wrap and reflow inside `box_rect`.
    pub replacement_text: String,
    /// Preferred font name (e.g. `"Helvetica"`, `"Times-Roman"`, `"Courier"`, or embedded name).
    pub font_name: String,
    /// Font size in points.
    pub font_size_pt: f32,
    /// RGB text color in `0.0..=1.0`.
    pub color_rgb: [f32; 3],
    /// Reflow box in top-down page coordinates (points).
    pub box_rect: PageRect,
    /// Alignment within `box_rect`.
    pub align: TextBoxAlign,
}

/// Outcome of [`edit_text_run`].
#[derive(Debug, Clone, PartialEq)]
pub struct TextEditOutcome {
    /// Updated PDF bytes.
    pub bytes: Vec<u8>,
    /// Wrapped lines emitted into the box.
    pub wrapped_lines: Vec<String>,
    /// Optional font substitution warning.
    pub warning: Option<FontMatchWarning>,
}

/// Checks whether a PDF contains any `/Sig` dictionary or `/ByteRange` signature entries.
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be parsed.
pub fn has_digital_signatures(pdf_bytes: &[u8]) -> Result<bool> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    for id in doc.object_ids() {
        if let Some(dict) = doc.get_dict(id) {
            let is_sig = dict.get("Type").and_then(PdfVal::as_name) == Some("Sig")
                || dict.get("FT").and_then(PdfVal::as_name) == Some("Sig")
                || dict.contains_key("ByteRange");
            if is_sig {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Wraps `text` into lines fitting within `max_width_pt` at `font_size_pt` (plan O.5).
#[must_use]
pub fn reflow_text_in_box(text: &str, max_width_pt: f32, font_size_pt: f32) -> Vec<String> {
    let width = max_width_pt.max(font_size_pt);
    let avg_char_w = (font_size_pt * 0.52).max(1.0);
    let max_chars = ((width / avg_char_w).floor() as usize).max(1);

    let mut lines = Vec::new();
    for para in text.lines() {
        let words: Vec<&str> = para.split_whitespace().collect();
        if words.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in words {
            let word_units = approx_display_units(word);
            let cur_units = approx_display_units(&current);
            if current.is_empty() {
                current.push_str(word);
            } else if cur_units + 1 + word_units <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_owned();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn approx_display_units(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// Replaces a text run on `edit.page`, preserving images and vector graphics, and reflows
/// `edit.replacement_text` inside `edit.box_rect` (plan O.5).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidOperation`] if the document is signed and `guard` is
/// [`SignatureGuard::BlockIfSigned`], or [`PdfDocError::OutOfRange`] if `edit.page` is invalid.
pub fn edit_text_run(
    pdf_bytes: &[u8],
    edit: &TextRunEdit,
    guard: SignatureGuard,
) -> Result<TextEditOutcome> {
    if guard == SignatureGuard::BlockIfSigned && has_digital_signatures(pdf_bytes)? {
        return Err(PdfDocError::InvalidOperation(
            "document has digital signatures; confirm signature invalidation before editing text"
                .into(),
        ));
    }
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &page_id = pages.get(edit.page).ok_or_else(|| {
        PdfDocError::OutOfRange(format!(
            "page {} out of range (count {})",
            edit.page,
            pages.len()
        ))
    })?;
    let mut page_dict = doc
        .get_dict(page_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
    let media = doc
        .effective_box(page_id, "MediaBox")
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let page_height = media[3] - media[1];

    // Match existing font resource or create standard substitute.
    let font_match = ensure_font_resource(&doc, page_id, &edit.font_name, &edit.replacement_text);
    let res_dict = font_match.res_dict;
    let font_res_name = font_match.res_key;
    let warning = font_match.warning;
    let extra_objs = font_match.extra;
    let mut next_id = font_match.next_id;

    // Strip matching text run from decoded content streams while keeping vector & image ops intact.
    let streams = doc.page_decoded_content_streams(page_id)?;
    let mut cleaned_stream = String::new();
    for stm in streams {
        let s = String::from_utf8_lossy(&stm);
        let stripped = strip_matching_text_ops(
            &s,
            &edit.original_text,
            &edit.target_quads,
            page_height,
            media[0],
            media[3],
        );
        cleaned_stream.push_str(&stripped);
        if !cleaned_stream.ends_with('\n') {
            cleaned_stream.push('\n');
        }
    }

    let wrapped = reflow_text_in_box(
        &edit.replacement_text,
        edit.box_rect.width(),
        edit.font_size_pt.max(4.0),
    );
    let leading = edit.font_size_pt.max(4.0) * 1.25;
    let [r, g, b] = edit.color_rgb.map(|v| v.clamp(0.0, 1.0));
    let mut added_bt = String::from("q\nBT\n");
    let _ = writeln!(
        added_bt,
        "/{font_res_name} {} Tf\n{} {} {} rg",
        fmt_f32(edit.font_size_pt.max(4.0)),
        fmt_f32(r),
        fmt_f32(g),
        fmt_f32(b)
    );
    for (idx, line) in wrapped.iter().enumerate() {
        let est_w = approx_display_units(line) as f32 * edit.font_size_pt * 0.52;
        let x_offset = match edit.align {
            TextBoxAlign::Left => 0.0,
            TextBoxAlign::Center => ((edit.box_rect.width() - est_w) * 0.5).max(0.0),
            TextBoxAlign::Right => (edit.box_rect.width() - est_w).max(0.0),
        };
        let pdf_x = media[0] + edit.box_rect.x0 + x_offset;
        let pdf_y = media[3] - edit.box_rect.y0 - edit.font_size_pt - (idx as f32 * leading);
        let _ = writeln!(
            added_bt,
            "1 0 0 1 {} {} Tm\n({}) Tj",
            fmt_f32(pdf_x),
            fmt_f32(pdf_y),
            escape_pdf_string(line)
        );
    }
    added_bt.push_str("ET\nQ\n");
    cleaned_stream.push_str(&added_bt);

    let new_content_id = next_id;
    next_id += 1;
    let _ = next_id;
    let mut extra = extra_objs;
    extra.insert(
        new_content_id,
        make_stream(
            BTreeMap::new(),
            cleaned_stream.as_bytes(),
            StreamEncoding::Flate(Compression::default()),
        )?,
    );
    page_dict.insert("Contents".into(), PdfVal::Ref(new_content_id, 0));
    page_dict.insert("Resources".into(), PdfVal::Dict(res_dict));

    let mut overrides = BTreeMap::new();
    overrides.insert(page_id, PdfVal::Dict(page_dict));

    Ok(TextEditOutcome {
        bytes: rewrite_clean(&doc, &overrides, &extra, &BTreeSet::new()),
        wrapped_lines: wrapped,
        warning,
    })
}

struct FontResourceMatch {
    res_dict: BTreeMap<String, PdfVal>,
    res_key: String,
    warning: Option<FontMatchWarning>,
    extra: BTreeMap<u32, PdfVal>,
    next_id: u32,
}

fn ensure_font_resource(
    doc: &ParsedPdf,
    page_id: u32,
    requested_font: &str,
    replacement_text: &str,
) -> FontResourceMatch {
    let mut res_dict = doc
        .effective_resources(page_id)
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();
    let mut fonts_dict = res_dict
        .get("Font")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();

    let has_non_ascii = !replacement_text.is_ascii();
    let req_lower = requested_font.trim().to_ascii_lowercase();

    for (key, val) in &fonts_dict {
        if let Some(PdfVal::Dict(fd)) = doc.resolve_val(val)
            && let Some(base) = fd.get("BaseFont").and_then(PdfVal::as_name)
        {
            let base_lower = base.to_ascii_lowercase();
            let is_subset = base.len() > 7 && base.as_bytes().get(6) == Some(&b'+');
            let matches_name = !req_lower.is_empty()
                && (base_lower == req_lower || base_lower.ends_with(&req_lower));
            if matches_name && !is_subset && !has_non_ascii {
                return FontResourceMatch {
                    res_dict,
                    res_key: key.clone(),
                    warning: None,
                    extra: BTreeMap::new(),
                    next_id: doc.next_obj_id(),
                };
            }
        }
    }

    let mut next_id = doc.next_obj_id();
    let font_id = next_id;
    next_id += 1;
    let subst = StandardPdfFont::from_hint(requested_font);
    let mut new_font = BTreeMap::new();
    new_font.insert("Type".into(), PdfVal::Name("Font".into()));
    new_font.insert("Subtype".into(), PdfVal::Name("Type1".into()));
    new_font.insert(
        "BaseFont".into(),
        PdfVal::Name(subst.base_font_name().into()),
    );
    new_font.insert("Encoding".into(), PdfVal::Name("WinAnsiEncoding".into()));

    let mut res_key = "FEx1".to_owned();
    let mut suffix = 1usize;
    while fonts_dict.contains_key(&res_key) {
        suffix += 1;
        res_key = format!("FEx{suffix}");
    }
    fonts_dict.insert(res_key.clone(), PdfVal::Ref(font_id, 0));
    res_dict.insert("Font".into(), PdfVal::Dict(fonts_dict));

    let mut extra = BTreeMap::new();
    extra.insert(font_id, PdfVal::Dict(new_font));

    let warning = if requested_font.eq_ignore_ascii_case(subst.base_font_name()) && !has_non_ascii {
        None
    } else {
        Some(FontMatchWarning {
            requested_font: requested_font.to_owned(),
            substituted_font: subst.base_font_name().to_owned(),
            reason: if has_non_ascii {
                "embedded subset or standard font substituted for non-ASCII run".into()
            } else {
                "embedded subset font lacked editable full glyph set; substituted standard Type1 font"
                    .into()
            },
        })
    };

    FontResourceMatch {
        res_dict,
        res_key,
        warning,
        extra,
        next_id,
    }
}

pub(crate) fn split_content_ops(stream: &str) -> Vec<String> {
    let bytes = stream.as_bytes();
    let len = bytes.len();
    let mut ops = Vec::new();
    let mut i = 0usize;
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut hex_or_dict = 0usize;

    while i < len {
        let b = bytes[i];
        if paren_depth > 0 {
            if b == b'\\' {
                i = (i + 2).min(len);
                continue;
            }
            if b == b'(' {
                paren_depth += 1;
            } else if b == b')' {
                paren_depth -= 1;
            }
            i += 1;
            continue;
        }
        if hex_or_dict > 0 {
            if bytes[i..].starts_with(b"<<") {
                hex_or_dict += 1;
                i += 2;
                continue;
            }
            if bytes[i..].starts_with(b">>") {
                hex_or_dict = hex_or_dict.saturating_sub(1);
                i += 2;
                continue;
            }
            if b == b'>' && hex_or_dict == 1 {
                hex_or_dict = 0;
            }
            i += 1;
            continue;
        }
        if b == b'%' {
            while i < len && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        if b == b'(' {
            paren_depth = 1;
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"<<") {
            hex_or_dict += 1;
            i += 2;
            continue;
        }
        if b == b'<' {
            hex_or_dict = 1;
            i += 1;
            continue;
        }
        if b == b'[' {
            bracket_depth += 1;
            i += 1;
            continue;
        }
        if b == b']' {
            bracket_depth = bracket_depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' {
            i += 1;
            while i < len && !bytes[i].is_ascii_whitespace() && !b"()<>[]{}/%".contains(&bytes[i]) {
                i += 1;
            }
            continue;
        }

        // Read bare token (either number/boolean operand or operator keyword).
        let tok_start = i;
        while i < len && !bytes[i].is_ascii_whitespace() && !b"()<>[]{}/%".contains(&bytes[i]) {
            i += 1;
        }
        if bracket_depth == 0 {
            let tok = &stream[tok_start..i];
            let is_num = tok.parse::<f32>().is_ok();
            let is_kw_operand = matches!(tok, "true" | "false" | "null");
            if !tok.is_empty() && !is_num && !is_kw_operand {
                let stmt = stream[start..i].trim();
                if !stmt.is_empty() {
                    ops.push(stmt.to_owned());
                }
                start = i;
            }
        }
    }
    let tail = stream[start..].trim();
    if !tail.is_empty() {
        ops.push(tail.to_owned());
    }
    ops
}

pub(crate) fn update_text_cursor(
    stmt: &str,
    cur_x: &mut f32,
    cur_y: &mut f32,
    font_size: &mut f32,
) {
    let tokens: Vec<&str> = stmt.split_whitespace().collect();
    let Some(&op) = tokens.last() else {
        return;
    };
    match op {
        "BT" => {
            *cur_x = 0.0;
            *cur_y = 0.0;
        }
        "Tf" if tokens.len() >= 3 => {
            if let Ok(sz) = tokens[tokens.len() - 2].parse::<f32>()
                && sz.is_finite()
                && sz > 0.5
            {
                *font_size = sz;
            }
        }
        "Tm" if tokens.len() >= 7 => {
            if let (Ok(e), Ok(f)) = (
                tokens[tokens.len() - 3].parse::<f32>(),
                tokens[tokens.len() - 2].parse::<f32>(),
            ) {
                *cur_x = e;
                *cur_y = f;
            }
        }
        "Td" | "TD" if tokens.len() >= 3 => {
            if let (Ok(tx), Ok(ty)) = (
                tokens[tokens.len() - 3].parse::<f32>(),
                tokens[tokens.len() - 2].parse::<f32>(),
            ) {
                *cur_x += tx;
                *cur_y += ty;
            }
        }
        _ => {}
    }
}

pub(crate) fn strip_matching_text_ops(
    stream: &str,
    needle_text: &str,
    target_quads: &[Quad],
    _page_height: f32,
    media_x0: f32,
    media_y1: f32,
) -> String {
    let mut out = String::with_capacity(stream.len());
    let mut cur_x = 0.0f32;
    let mut cur_y = 0.0f32;
    let mut font_size = 12.0f32;

    for stmt in split_content_ops(stream) {
        update_text_cursor(&stmt, &mut cur_x, &mut cur_y, &mut font_size);
        let is_text_show = stmt.ends_with("Tj")
            || stmt.ends_with("TJ")
            || stmt.ends_with('\'')
            || stmt.ends_with('"');
        if is_text_show {
            let page_x = cur_x - media_x0;
            let page_y = media_y1 - cur_y;
            let inside_quad = target_quads.iter().any(|q| {
                let r = q.bounds();
                page_x >= r.x0 - 8.0
                    && page_x <= r.x1 + 8.0
                    && page_y >= r.y0 - 8.0
                    && page_y <= r.y1 + 8.0
            });
            let matches_literal = !needle_text.is_empty()
                && (stmt.contains(&escape_pdf_string(needle_text)) || stmt.contains(needle_text));
            if inside_quad || matches_literal {
                continue;
            }
        }
        out.push_str(&stmt);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// O.6: Embedded Image Inspection, Extraction, Replacement, Masking
// ---------------------------------------------------------------------------

/// Encoding of an extracted or replacement PDF image stream (plan O.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImageStreamFormat {
    /// Baseline/progressive JPEG (`/DCTDecode`).
    Jpeg,
    /// Raw 8-bit RGB pixel stream compressed with `/FlateDecode`.
    FlateRgb,
    /// Raw 8-bit grayscale pixel stream compressed with `/FlateDecode`.
    FlateGray,
    /// Other PDF image stream filter.
    Other,
}

/// Metadata describing an embedded image XObject on a page (plan O.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedImageInfo {
    /// Indirect PDF object ID of the image XObject.
    pub object_id: u32,
    /// Resource name inside `/Resources /XObject` (without leading `/`).
    pub resource_name: String,
    /// Native pixel width.
    pub width: u32,
    /// Native pixel height.
    pub height: u32,
    /// PDF color space name (e.g. `"DeviceRGB"`, `"DeviceGray"`).
    pub color_space: String,
    /// Bits per color component.
    pub bits_per_component: u8,
    /// Stream filter name, if any.
    pub filter: Option<String>,
}

/// Extracted native image payload from a PDF XObject (plan O.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImage {
    /// Image stream format.
    pub format: ImageStreamFormat,
    /// Native pixel width.
    pub width: u32,
    /// Native pixel height.
    pub height: u32,
    /// Raw stream bytes (valid JPEG for [`ImageStreamFormat::Jpeg`], or decoded RGB/Gray samples).
    pub bytes: Vec<u8>,
}

/// Replacement image payload for [`replace_image`] (plan O.6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplacementImage {
    /// Already-encoded JPEG bytes (`/DCTDecode`).
    Jpeg {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Valid JFIF/JPEG bytes.
        bytes: Vec<u8>,
    },
    /// Raw 8-bit RGB samples (`width * height * 3` bytes).
    Rgb8 {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Interleaved RGB bytes.
        samples: Vec<u8>,
    },
    /// Raw 8-bit RGBA samples (`width * height * 4` bytes); splits alpha into `/SMask`.
    Rgba8 {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Interleaved RGBA bytes.
        samples: Vec<u8>,
    },
}

/// Lists all embedded image XObjects referenced in `page`'s `/Resources /XObject` dictionary (plan O.6).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `page` is out of range or the PDF is invalid.
pub fn list_page_images(pdf_bytes: &[u8], page: usize) -> Result<Vec<EmbeddedImageInfo>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &page_id = pages.get(page).ok_or_else(|| {
        PdfDocError::OutOfRange(format!("page {page} out of range (count {})", pages.len()))
    })?;
    let Some(res) = doc.effective_resources(page_id) else {
        return Ok(Vec::new());
    };
    let Some(res_dict) = res.as_dict() else {
        return Ok(Vec::new());
    };
    let Some(xobj_val) = res_dict.get("XObject").and_then(|v| doc.resolve_val(v)) else {
        return Ok(Vec::new());
    };
    let Some(xobjs) = xobj_val.as_dict() else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (name, val) in xobjs {
        let Some(obj_id) = val.as_ref_id() else {
            continue;
        };
        let Some(PdfVal::Stream { dict, .. }) = doc.resolve_obj(obj_id) else {
            continue;
        };
        if dict.get("Subtype").and_then(PdfVal::as_name) != Some("Image") {
            continue;
        }
        let width = dict
            .get("Width")
            .and_then(PdfVal::as_i64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let height = dict
            .get("Height")
            .and_then(PdfVal::as_i64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let color_space = dict
            .get("ColorSpace")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_name().map(str::to_owned))
            .unwrap_or_else(|| "DeviceRGB".into());
        let bpc = dict
            .get("BitsPerComponent")
            .and_then(PdfVal::as_i64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(8);
        let filter = dict
            .get("Filter")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_name().map(str::to_owned));
        out.push(EmbeddedImageInfo {
            object_id: obj_id,
            resource_name: name.clone(),
            width,
            height,
            color_space,
            bits_per_component: bpc,
            filter,
        });
    }
    Ok(out)
}

/// Extracts an embedded image XObject by `object_id` at native resolution (plan O.6).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the object is missing or is not a readable image stream.
pub fn extract_image(pdf_bytes: &[u8], object_id: u32) -> Result<ExtractedImage> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let Some(PdfVal::Stream { dict, data }) = doc.resolve_obj(object_id) else {
        return Err(PdfDocError::OutOfRange(format!(
            "image object {object_id} not found"
        )));
    };
    if dict.get("Subtype").and_then(PdfVal::as_name) != Some("Image") {
        return Err(PdfDocError::InvalidOperation(format!(
            "object {object_id} is not an Image XObject"
        )));
    }
    let width = dict
        .get("Width")
        .and_then(PdfVal::as_i64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let height = dict
        .get("Height")
        .and_then(PdfVal::as_i64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let filter = dict.get("Filter").and_then(PdfVal::as_name);
    let cs = dict.get("ColorSpace").and_then(PdfVal::as_name);

    match filter {
        Some("DCTDecode") => Ok(ExtractedImage {
            format: ImageStreamFormat::Jpeg,
            width,
            height,
            bytes: data,
        }),
        Some("FlateDecode") | None => {
            let decoded = decode_stream_data(&dict, &data)?;
            let format = if cs == Some("DeviceGray") {
                ImageStreamFormat::FlateGray
            } else {
                ImageStreamFormat::FlateRgb
            };
            Ok(ExtractedImage {
                format,
                width,
                height,
                bytes: decoded,
            })
        }
        Some(_) => Ok(ExtractedImage {
            format: ImageStreamFormat::Other,
            width,
            height,
            bytes: data,
        }),
    }
}

/// Replaces image XObject `resource_name` on `page` with `replacement`, preserving the existing
/// placement matrix in the page content stream (plan O.6).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the page or image resource is missing or pixel buffer sizes mismatch.
pub fn replace_image(
    pdf_bytes: &[u8],
    page: usize,
    resource_name: &str,
    replacement: &ReplacementImage,
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let images = list_page_images(pdf_bytes, page)?;
    let target = images
        .iter()
        .find(|img| img.resource_name == resource_name)
        .ok_or_else(|| {
            PdfDocError::OutOfRange(format!(
                "image resource '{resource_name}' not found on page {page}"
            ))
        })?;

    let mut overrides = BTreeMap::new();
    let mut extra = BTreeMap::new();
    let mut next_id = doc.next_obj_id();

    let mut dict = BTreeMap::new();
    dict.insert("Type".into(), PdfVal::Name("XObject".into()));
    dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
    dict.insert("BitsPerComponent".into(), PdfVal::Int(8));

    let stream = match replacement {
        ReplacementImage::Jpeg {
            width,
            height,
            bytes,
        } => {
            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            dict.insert("Filter".into(), PdfVal::Name("DCTDecode".into()));
            make_stream(dict, bytes, StreamEncoding::Raw)?
        }
        ReplacementImage::Rgb8 {
            width,
            height,
            samples,
        } => {
            let expected = (*width as usize) * (*height as usize) * 3;
            if samples.len() != expected {
                return Err(PdfDocError::InvalidOperation(format!(
                    "expected {expected} RGB bytes, got {}",
                    samples.len()
                )));
            }
            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            make_stream(dict, samples, StreamEncoding::Flate(Compression::default()))?
        }
        ReplacementImage::Rgba8 {
            width,
            height,
            samples,
        } => {
            let pixels = (*width as usize) * (*height as usize);
            if samples.len() != pixels * 4 {
                return Err(PdfDocError::InvalidOperation(format!(
                    "expected {} RGBA bytes, got {}",
                    pixels * 4,
                    samples.len()
                )));
            }
            let mut rgb = Vec::with_capacity(pixels * 3);
            let mut alpha = Vec::with_capacity(pixels);
            for px in samples.as_chunks::<4>().0 {
                rgb.extend_from_slice(&px[..3]);
                alpha.push(px[3]);
            }
            let smask_id = next_id;
            next_id += 1;
            let _ = next_id;
            let mut smask_dict = BTreeMap::new();
            smask_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            smask_dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            smask_dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            smask_dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            smask_dict.insert("ColorSpace".into(), PdfVal::Name("DeviceGray".into()));
            smask_dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            extra.insert(
                smask_id,
                make_stream(
                    smask_dict,
                    &alpha,
                    StreamEncoding::Flate(Compression::default()),
                )?,
            );

            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            dict.insert("SMask".into(), PdfVal::Ref(smask_id, 0));
            make_stream(dict, &rgb, StreamEncoding::Flate(Compression::default()))?
        }
    };

    overrides.insert(target.object_id, stream);
    Ok(rewrite_clean(&doc, &overrides, &extra, &BTreeSet::new()))
}

/// Attaches or updates an 8-bit grayscale `/SMask` transparency mask on embedded image
/// `resource_name` on `page` (plan O.6).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the image is not found or `alpha_samples.len() != width * height`.
pub fn apply_image_mask(
    pdf_bytes: &[u8],
    page: usize,
    resource_name: &str,
    alpha_samples: &[u8],
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let images = list_page_images(pdf_bytes, page)?;
    let target = images
        .iter()
        .find(|img| img.resource_name == resource_name)
        .ok_or_else(|| {
            PdfDocError::OutOfRange(format!(
                "image resource '{resource_name}' not found on page {page}"
            ))
        })?;
    let expected = (target.width as usize) * (target.height as usize);
    if alpha_samples.len() != expected {
        return Err(PdfDocError::InvalidOperation(format!(
            "expected {expected} alpha mask bytes, got {}",
            alpha_samples.len()
        )));
    }
    let Some(PdfVal::Stream { mut dict, data }) = doc.resolve_obj(target.object_id) else {
        return Err(PdfDocError::InvalidPdf("image stream missing".into()));
    };
    let smask_id = doc.next_obj_id();
    let mut smask_dict = BTreeMap::new();
    smask_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
    smask_dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
    smask_dict.insert("Width".into(), PdfVal::Int(i64::from(target.width)));
    smask_dict.insert("Height".into(), PdfVal::Int(i64::from(target.height)));
    smask_dict.insert("ColorSpace".into(), PdfVal::Name("DeviceGray".into()));
    smask_dict.insert("BitsPerComponent".into(), PdfVal::Int(8));

    let mut extra = BTreeMap::new();
    extra.insert(
        smask_id,
        make_stream(
            smask_dict,
            alpha_samples,
            StreamEncoding::Flate(Compression::default()),
        )?,
    );

    dict.insert("SMask".into(), PdfVal::Ref(smask_id, 0));
    let mut overrides = BTreeMap::new();
    overrides.insert(target.object_id, PdfVal::Stream { dict, data });
    Ok(rewrite_clean(&doc, &overrides, &extra, &BTreeSet::new()))
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{Document, PagePoint};

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    fn sample_pdf_with_text_and_image() -> Vec<u8> {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 720 Td (Original Heading) Tj ET".into(),
            crop_box: None,
        });
        let base = b.build().expect("base");
        // Embed an image XObject /Im1 via clean rewrite.
        let doc = ParsedPdf::parse(&base).expect("parse");
        let pages = doc.page_objects().expect("pages");
        let pid = pages[0];
        let mut pdict = doc.get_dict(pid).expect("dict");

        let img_id = doc.next_obj_id();
        let content_id = img_id + 1;
        let mut img_dict = BTreeMap::new();
        img_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
        img_dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
        img_dict.insert("Width".into(), PdfVal::Int(2));
        img_dict.insert("Height".into(), PdfVal::Int(2));
        img_dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
        img_dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
        let rgb_samples = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];

        let mut extra = BTreeMap::new();
        extra.insert(
            img_id,
            make_stream(
                img_dict,
                &rgb_samples,
                StreamEncoding::Flate(Compression::default()),
            )
            .expect("stream"),
        );
        let content = b"BT /F1 14 Tf 1 0 0 1 72 720 Tm (Original Heading) Tj ET\nq 100 0 0 100 72 500 cm /Im1 Do Q\n";
        extra.insert(
            content_id,
            make_stream(BTreeMap::new(), content, StreamEncoding::Raw).expect("content"),
        );

        let mut res = doc
            .effective_resources(pid)
            .and_then(|v| v.as_dict().cloned())
            .unwrap_or_default();
        let mut xobjs = BTreeMap::new();
        xobjs.insert("Im1".into(), PdfVal::Ref(img_id, 0));
        res.insert("XObject".into(), PdfVal::Dict(xobjs));
        pdict.insert("Resources".into(), PdfVal::Dict(res));
        pdict.insert("Contents".into(), PdfVal::Ref(content_id, 0));

        let mut overrides = BTreeMap::new();
        overrides.insert(pid, PdfVal::Dict(pdict));
        rewrite_clean(&doc, &overrides, &extra, &BTreeSet::new())
    }

    #[test]
    fn edit_text_run_reflows_and_preserves_images() {
        let pdf = sample_pdf_with_text_and_image();
        let edit = TextRunEdit {
            page: 0,
            target_quads: vec![Quad {
                ul: PagePoint { x: 70.0, y: 55.0 },
                ur: PagePoint { x: 220.0, y: 55.0 },
                ll: PagePoint { x: 70.0, y: 76.0 },
                lr: PagePoint { x: 220.0, y: 76.0 },
            }],
            original_text: "Original Heading".into(),
            replacement_text: "Updated Title That Wraps Cleanly".into(),
            font_name: "Helvetica".into(),
            font_size_pt: 14.0,
            color_rgb: [0.1, 0.2, 0.7],
            box_rect: PageRect {
                x0: 72.0,
                y0: 55.0,
                x1: 200.0,
                y1: 120.0,
            },
            align: TextBoxAlign::Left,
        };
        let outcome = edit_text_run(&pdf, &edit, SignatureGuard::BlockIfSigned).expect("edit");
        assert!(outcome.wrapped_lines.len() >= 2, "reflowed into >=2 lines");
        let doc = Document::from_bytes(&outcome.bytes, "application/pdf").expect("mupdf");
        let extracted = doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("stext")
            .text()
            .expect("text");
        assert!(
            !extracted.contains("Original Heading"),
            "old text gone: {extracted}"
        );
        assert!(
            extracted.contains("Updated Title"),
            "new text present: {extracted}"
        );
        assert_eq!(list_page_images(&outcome.bytes, 0).expect("imgs").len(), 1);
    }

    #[test]
    fn replace_extract_and_mask_embedded_image() {
        let pdf = sample_pdf_with_text_and_image();
        let imgs = list_page_images(&pdf, 0).expect("list");
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].resource_name, "Im1");
        let extracted = extract_image(&pdf, imgs[0].object_id).expect("extract");
        assert_eq!((extracted.width, extracted.height), (2, 2));
        assert_eq!(extracted.bytes.len(), 12);

        let replaced = replace_image(
            &pdf,
            0,
            "Im1",
            &ReplacementImage::Rgb8 {
                width: 3,
                height: 2,
                samples: vec![42; 18],
            },
        )
        .expect("replace");
        let imgs2 = list_page_images(&replaced, 0).expect("list2");
        assert_eq!((imgs2[0].width, imgs2[0].height), (3, 2));

        let masked =
            apply_image_mask(&replaced, 0, "Im1", &[255, 128, 0, 255, 128, 0]).expect("mask");
        let doc = Document::from_bytes(&masked, "application/pdf").expect("open masked");
        assert_eq!(doc.page_count().expect("c"), 1);
    }
}
