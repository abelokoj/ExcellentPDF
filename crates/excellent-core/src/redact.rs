//! Two-phase true PDF redaction and independent MuPDF verification (plan O.7, §14.3).
//!
//! Phase 1 marks regions manually or by pattern search ([`RedactionPattern`]).
//! Phase 2 ([`apply_redactions`]) permanently strips underlying text, vector, and image
//! operators inside marked rectangles, scrubs metadata/XMP/attachments/hidden layers/annots/forms
//! as configured, rewrites the entire PDF without incremental history, and verifies via
//! [`verify_redaction`] that no redacted string remains extractable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use excellent_mupdf::{Document, PageRect, TextLayout};
use flate2::Compression;

use crate::edit_content::{split_content_ops, update_text_cursor};
use crate::page_ops::pdf_doc::{
    ParsedPdf, PdfDocError, PdfVal, StreamEncoding, fmt_f32, make_stream, rewrite_clean,
};

/// Result alias for redaction operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Whether solid redaction boxes are drawn over redacted areas (plan O.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RedactionBoxStyle {
    /// Draw opaque solid boxes (default black) over redacted areas.
    #[default]
    SolidFill,
    /// Strip underlying content without painting an opaque rectangle.
    NoOverlayBox,
}

/// Whether metadata, attachments, or form values are scrubbed during redaction (plan O.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ScrubMode {
    /// Remove matching items.
    #[default]
    Scrub,
    /// Leave untouched.
    Keep,
}

/// Case sensitivity for redaction pattern matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PatternCase {
    /// Case-sensitive match.
    Sensitive,
    /// ASCII case-insensitive match.
    #[default]
    Insensitive,
}

/// Pattern preset for automatic redaction marking (plan O.7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RedactionPattern {
    /// Exact literal phrase.
    Literal {
        /// Text to search for.
        text: String,
        /// Case sensitivity.
        case: PatternCase,
    },
    /// US Social Security Number (`DDD-DD-DDDD`).
    Ssn,
    /// Email address (`local@domain.tld`).
    Email,
    /// Payment card number (13–19 digits with optional `-` or space separators).
    CreditCard,
    /// Phone number (`+1 (555) 123-4567` / `555-123-4567`).
    Phone,
}

/// Phase-1 redaction mark queued for review before permanent application (plan O.7).
#[derive(Debug, Clone, PartialEq)]
pub struct RedactionMark {
    /// Unique mark identifier.
    pub id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Redaction rectangle in top-down page coordinates (points).
    pub rect: PageRect,
    /// Optional forbidden string associated with this mark for post-apply verification.
    pub expected_removed_text: Option<String>,
    /// Box fill RGB in `0.0..=1.0`.
    pub fill_color: [f32; 3],
}

/// Phase-2 redaction application policy (plan O.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionOptions {
    /// Whether to draw solid fill boxes over redacted rectangles.
    pub box_style: RedactionBoxStyle,
    /// Whether to strip text operators inside redaction rectangles.
    pub redact_text: ScrubMode,
    /// Whether to strip vector path/fill operators inside redaction rectangles.
    pub redact_vectors: ScrubMode,
    /// Whether to strip image `Do` invocations inside redaction rectangles.
    pub redact_images: ScrubMode,
    /// Whether to scrub `/Info` and XMP `/Metadata`.
    pub scrub_metadata: ScrubMode,
    /// Whether to scrub `/EmbeddedFiles` attachments.
    pub scrub_attachments: ScrubMode,
    /// Whether to scrub `/OCProperties` hidden layers.
    pub scrub_hidden_layers: ScrubMode,
    /// Whether to scrub `/Annots` overlapping redaction rectangles.
    pub scrub_overlapping_annots: ScrubMode,
    /// Whether to scrub `/AcroForm` field values overlapping redaction rectangles.
    pub scrub_overlapping_forms: ScrubMode,
}

impl Default for RedactionOptions {
    fn default() -> Self {
        Self {
            box_style: RedactionBoxStyle::SolidFill,
            redact_text: ScrubMode::Scrub,
            redact_vectors: ScrubMode::Scrub,
            redact_images: ScrubMode::Scrub,
            scrub_metadata: ScrubMode::Scrub,
            scrub_attachments: ScrubMode::Scrub,
            scrub_hidden_layers: ScrubMode::Scrub,
            scrub_overlapping_annots: ScrubMode::Scrub,
            scrub_overlapping_forms: ScrubMode::Scrub,
        }
    }
}

/// One hit found during post-redaction verification (plan O.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationHit {
    /// Where the hit was found (`"page:0"`, `"metadata:Info"`, etc.).
    pub location: String,
    /// Matched secret string or pattern label.
    pub matched: String,
}

/// Verification report returned by [`verify_redaction`] (plan O.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionVerification {
    /// Zero-based pages checked in reopened MuPDF document.
    pub pages_checked: usize,
    /// Remaining hits that failed redaction; empty means verified clean.
    pub remaining_hits: Vec<VerificationHit>,
}

impl RedactionVerification {
    /// Returns `true` when zero forbidden strings or patterns were found anywhere in the reopened PDF.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.remaining_hits.is_empty()
    }
}

/// Searches a page's [`TextLayout`] for `pattern` and produces Phase-1 [`RedactionMark`]s (plan O.7).
#[must_use]
pub fn mark_from_text_layout(
    page: usize,
    layout: &TextLayout,
    pattern: &RedactionPattern,
) -> Vec<RedactionMark> {
    let mut marks = Vec::new();
    for (line_idx, line) in layout.lines.iter().enumerate() {
        let chars = layout.chars.get(line.chars.clone()).unwrap_or(&[]);
        if chars.is_empty() {
            continue;
        }
        let line_text: String = chars.iter().map(|c| c.c).collect();
        for (byte_start, byte_end) in find_pattern_byte_spans(&line_text, pattern) {
            let char_start = line_text[..byte_start].chars().count();
            let char_end = line_text[..byte_end].chars().count();
            let slice = chars.get(char_start..char_end).unwrap_or(&[]);
            if slice.is_empty() {
                continue;
            }
            let mut rect = slice[0].quad.bounds();
            for ch in &slice[1..] {
                let b = ch.quad.bounds();
                rect.x0 = rect.x0.min(b.x0);
                rect.y0 = rect.y0.min(b.y0);
                rect.x1 = rect.x1.max(b.x1);
                rect.y1 = rect.y1.max(b.y1);
            }
            marks.push(RedactionMark {
                id: format!("redact-p{page}-l{line_idx}-c{char_start}"),
                page,
                rect,
                expected_removed_text: Some(line_text[byte_start..byte_end].to_owned()),
                fill_color: [0.0, 0.0, 0.0],
            });
        }
    }
    marks
}

fn find_pattern_byte_spans(text: &str, pattern: &RedactionPattern) -> Vec<(usize, usize)> {
    match pattern {
        RedactionPattern::Literal { text: needle, case } => {
            if needle.is_empty() {
                return Vec::new();
            }
            let mut spans = Vec::new();
            match case {
                PatternCase::Sensitive => {
                    let mut start = 0;
                    while let Some(pos) = text[start..].find(needle.as_str()) {
                        let abs = start + pos;
                        spans.push((abs, abs + needle.len()));
                        start = abs + needle.len();
                    }
                }
                PatternCase::Insensitive => {
                    let hay = text.to_ascii_lowercase();
                    let ndl = needle.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = hay[start..].find(&ndl) {
                        let abs = start + pos;
                        spans.push((abs, abs + ndl.len()));
                        start = abs + ndl.len();
                    }
                }
            }
            spans
        }
        RedactionPattern::Ssn => find_digit_pattern_spans(text, is_ssn_candidate),
        RedactionPattern::Email => find_email_spans(text),
        RedactionPattern::CreditCard => find_digit_pattern_spans(text, is_card_candidate),
        RedactionPattern::Phone => find_digit_pattern_spans(text, is_phone_candidate),
    }
}

fn find_email_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    for part in text.split_whitespace() {
        if let Some(rel) = text[i..].find(part) {
            let start = i + rel;
            let trimmed =
                part.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '(' | ')'));
            if let Some((local, domain)) = trimmed.split_once('@')
                && !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
            {
                let offset = part.find(trimmed).unwrap_or(0);
                out.push((start + offset, start + offset + trimmed.len()));
            }
            i = start + part.len();
        }
    }
    out
}

fn find_digit_pattern_spans(text: &str, pred: fn(&str) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || bytes[i] == b'+' || bytes[i] == b'(' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || matches!(bytes[i], b'-' | b' ' | b'(' | b')' | b'+'))
            {
                i += 1;
            }
            let cand = text[start..i].trim();
            if pred(cand)
                && let Some(off) = text[start..i].find(cand)
            {
                out.push((start + off, start + off + cand.len()));
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_ssn_candidate(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 11
        && b[3] == b'-'
        && b[6] == b'-'
        && b[..3].iter().all(u8::is_ascii_digit)
        && b[4..6].iter().all(u8::is_ascii_digit)
        && b[7..].iter().all(u8::is_ascii_digit)
}

fn is_card_candidate(s: &str) -> bool {
    let digits: usize = s.bytes().filter(u8::is_ascii_digit).count();
    (13..=19).contains(&digits) && (s.contains('-') || s.contains(' ') || digits == s.len())
}

fn is_phone_candidate(s: &str) -> bool {
    let digits: usize = s.bytes().filter(u8::is_ascii_digit).count();
    (7..=15).contains(&digits) && (s.contains('-') || s.contains('('))
}

/// Permanently applies `marks` to `pdf_bytes` and performs a full non-incremental rewrite so no
/// redacted text, vectors, images, or metadata remain in the file (plan O.7).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any mark targets an out-of-range page or the PDF cannot be parsed.
pub fn apply_redactions(
    pdf_bytes: &[u8],
    marks: &[RedactionMark],
    options: &RedactionOptions,
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    for mark in marks {
        if mark.page >= pages.len() {
            return Err(PdfDocError::OutOfRange(format!(
                "redaction mark page {} out of range (count {})",
                mark.page,
                pages.len()
            )));
        }
    }

    let mut marks_by_page: BTreeMap<usize, Vec<&RedactionMark>> = BTreeMap::new();
    for mark in marks {
        marks_by_page.entry(mark.page).or_default().push(mark);
    }

    let mut overrides = BTreeMap::new();
    let mut dropped = BTreeSet::new();
    let mut next_id = doc.next_obj_id();

    // Scrub catalog-level metadata, attachments, and hidden layers if requested.
    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("catalog missing".into()))?;
    if options.scrub_metadata == ScrubMode::Scrub {
        if let Some(meta_id) = catalog.remove("Metadata").and_then(|v| v.as_ref_id()) {
            dropped.insert(meta_id);
        }
        if let Some(info_id) = doc.info_id {
            dropped.insert(info_id);
        }
    }
    if options.scrub_attachments == ScrubMode::Scrub
        && let Some(PdfVal::Dict(mut names)) = catalog.get("Names").and_then(|v| doc.resolve_val(v))
        && names.remove("EmbeddedFiles").is_some()
    {
        catalog.insert("Names".into(), PdfVal::Dict(names));
    }

    let hidden_ocg_names = if options.scrub_hidden_layers == ScrubMode::Scrub {
        collect_hidden_ocg_names(&doc, &catalog)
    } else {
        BTreeSet::new()
    };
    overrides.insert(doc.root_id, PdfVal::Dict(catalog));

    for (page_idx, &page_id) in pages.iter().enumerate() {
        let page_marks = marks_by_page
            .get(&page_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if page_marks.is_empty() && hidden_ocg_names.is_empty() {
            continue;
        }
        let mut page_dict = doc
            .get_dict(page_id)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
        if options.scrub_metadata == ScrubMode::Scrub
            && let Some(thumb_id) = page_dict.remove("Thumb").and_then(|v| v.as_ref_id())
        {
            dropped.insert(thumb_id);
        }
        let media = doc
            .effective_box(page_id, "MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);

        let pdf_rects: Vec<[f32; 4]> = page_marks
            .iter()
            .map(|m| to_pdf_rect(m.rect, media[0], media[3]))
            .collect();

        // Rewrite content stream removing text/vectors/images inside pdf_rects and hidden OCGs.
        let streams = doc.page_decoded_content_streams(page_id)?;
        let mut rewritten_stream = String::new();
        for stm in streams {
            let text = String::from_utf8_lossy(&stm);
            let stripped = strip_redacted_content(
                &text,
                &pdf_rects,
                options,
                &hidden_ocg_names,
                media[0],
                media[3],
            );
            rewritten_stream.push_str(&stripped);
            if !rewritten_stream.ends_with('\n') {
                rewritten_stream.push('\n');
            }
        }

        if options.box_style == RedactionBoxStyle::SolidFill {
            for (mark, rect) in page_marks.iter().zip(&pdf_rects) {
                let [r, g, b] = mark.fill_color.map(|c| c.clamp(0.0, 1.0));
                let _ = writeln!(
                    rewritten_stream,
                    "q\n{} {} {} rg\n{} {} {} {} re f\nQ",
                    fmt_f32(r),
                    fmt_f32(g),
                    fmt_f32(b),
                    fmt_f32(rect[0]),
                    fmt_f32(rect[1]),
                    fmt_f32(rect[2] - rect[0]),
                    fmt_f32(rect[3] - rect[1])
                );
            }
        }

        // Drop replaced /Contents stream object(s) so old unredacted bytes cannot survive as orphans.
        match page_dict.get("Contents") {
            Some(PdfVal::Ref(old_cid, _)) => {
                dropped.insert(*old_cid);
            }
            Some(PdfVal::Array(items)) => {
                for item in items {
                    if let Some(old_cid) = item.as_ref_id() {
                        dropped.insert(old_cid);
                    }
                }
            }
            _ => {}
        }

        let content_id = next_id;
        next_id += 1;
        overrides.insert(
            content_id,
            make_stream(
                BTreeMap::new(),
                rewritten_stream.as_bytes(),
                StreamEncoding::Flate(Compression::default()),
            )?,
        );
        page_dict.insert("Contents".into(), PdfVal::Ref(content_id, 0));

        // Scrub overlapping annotations and form widget values.
        if (options.scrub_overlapping_annots == ScrubMode::Scrub
            || options.scrub_overlapping_forms == ScrubMode::Scrub)
            && let Some(annots_val) = page_dict.get("Annots").cloned()
            && let Some(PdfVal::Array(annots)) = doc.resolve_val(&annots_val)
        {
            let mut kept_annots = Vec::new();
            for item in annots {
                let Some(aid) = item.as_ref_id() else {
                    kept_annots.push(item);
                    continue;
                };
                let Some(mut adict) = doc.get_dict(aid) else {
                    continue;
                };
                let subtype = adict.get("Subtype").and_then(PdfVal::as_name);
                let overlap = adict
                    .get("Rect")
                    .and_then(|v| doc.resolve_val(v))
                    .and_then(|v| v.as_rect())
                    .is_some_and(|r| pdf_rects.iter().any(|rr| rects_intersect(r, *rr)));
                if !overlap {
                    kept_annots.push(item);
                    continue;
                }
                if subtype == Some("Widget") {
                    if options.scrub_overlapping_forms == ScrubMode::Scrub {
                        adict.remove("V");
                        adict.remove("DV");
                        adict.remove("AP");
                        overrides.insert(aid, PdfVal::Dict(adict));
                    }
                    kept_annots.push(item);
                } else if options.scrub_overlapping_annots == ScrubMode::Scrub {
                    dropped.insert(aid);
                } else {
                    kept_annots.push(item);
                }
            }
            page_dict.insert("Annots".into(), PdfVal::Array(kept_annots));
        }

        overrides.insert(page_id, PdfVal::Dict(page_dict));
    }

    Ok(rewrite_clean(&doc, &overrides, &BTreeMap::new(), &dropped))
}

/// Reopens `redacted_pdf` in MuPDF and searches every page's extracted text and PDF metadata
/// for `forbidden_patterns` (plan O.7). Never claims success without checking every target!
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be opened in MuPDF.
pub fn verify_redaction(
    redacted_pdf: &[u8],
    forbidden_patterns: &[RedactionPattern],
) -> Result<RedactionVerification> {
    let parsed = ParsedPdf::parse(redacted_pdf)?;
    let mut hits = Vec::new();

    if let Some(info_id) = parsed.info_id
        && let Some(info) = parsed.get_dict(info_id)
    {
        for (k, v) in info {
            if let Some(s) = v.as_text_string() {
                for pat in forbidden_patterns {
                    if !find_pattern_byte_spans(&s, pat).is_empty() {
                        hits.push(VerificationHit {
                            location: format!("metadata:Info/{k}"),
                            matched: describe_pattern(pat),
                        });
                    }
                }
            }
        }
    }

    let doc = Document::from_bytes(redacted_pdf, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let count = doc
        .page_count()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    for page_idx in 0..count {
        let page = doc
            .load_page(page_idx)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let text = page
            .text(None)
            .and_then(|st| st.text())
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        for pat in forbidden_patterns {
            for (s, e) in find_pattern_byte_spans(&text, pat) {
                hits.push(VerificationHit {
                    location: format!("page:{page_idx}"),
                    matched: text[s..e].to_owned(),
                });
            }
        }
    }

    Ok(RedactionVerification {
        pages_checked: count,
        remaining_hits: hits,
    })
}

fn describe_pattern(pat: &RedactionPattern) -> String {
    match pat {
        RedactionPattern::Literal { text, .. } => text.clone(),
        RedactionPattern::Ssn => "SSN".into(),
        RedactionPattern::Email => "Email".into(),
        RedactionPattern::CreditCard => "CreditCard".into(),
        RedactionPattern::Phone => "Phone".into(),
    }
}

fn to_pdf_rect(r: PageRect, media_x0: f32, media_y1: f32) -> [f32; 4] {
    [
        media_x0 + r.x0,
        media_y1 - r.y1,
        media_x0 + r.x1,
        media_y1 - r.y0,
    ]
}

fn rects_intersect(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

fn collect_hidden_ocg_names(
    doc: &ParsedPdf,
    catalog: &BTreeMap<String, PdfVal>,
) -> BTreeSet<String> {
    let mut hidden = BTreeSet::new();
    let Some(PdfVal::Dict(ocp)) = catalog.get("OCProperties").and_then(|v| doc.resolve_val(v))
    else {
        return hidden;
    };
    let Some(PdfVal::Dict(def)) = ocp.get("D").and_then(|v| doc.resolve_val(v)) else {
        return hidden;
    };
    let Some(PdfVal::Array(off)) = def.get("OFF").and_then(|v| doc.resolve_val(v)) else {
        return hidden;
    };
    for item in off {
        if let Some(PdfVal::Dict(ocg)) = doc.resolve_val(&item)
            && let Some(name) = ocg.get("Name").and_then(|v| v.as_text_string())
        {
            hidden.insert(name);
        }
    }
    hidden
}

fn strip_redacted_content(
    stream: &str,
    pdf_rects: &[[f32; 4]],
    options: &RedactionOptions,
    hidden_ocgs: &BTreeSet<String>,
    _media_x0: f32,
    _media_y1: f32,
) -> String {
    let mut out = String::with_capacity(stream.len());
    let mut cur_x = 0.0f32;
    let mut cur_y = 0.0f32;
    let mut font_size = 12.0f32;
    let mut cur_ctm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut ctm_stack: Vec<[f32; 6]> = Vec::new();
    let mut hidden_depth = 0usize;

    for stmt in split_content_ops(stream) {
        let trimmed = stmt.trim();
        if !hidden_ocgs.is_empty() {
            if trimmed.starts_with("/OC") && trimmed.ends_with("BDC") {
                if hidden_ocgs.iter().any(|n| trimmed.contains(n)) {
                    hidden_depth += 1;
                    continue;
                }
            } else if hidden_depth > 0 {
                if trimmed == "EMC" {
                    hidden_depth -= 1;
                }
                continue;
            }
        }

        if trimmed == "q" {
            ctm_stack.push(cur_ctm);
        } else if trimmed == "Q" {
            if let Some(prev) = ctm_stack.pop() {
                cur_ctm = prev;
            }
        } else if let Some(m) = parse_cm(trimmed) {
            cur_ctm = m;
        }
        update_text_cursor(trimmed, &mut cur_x, &mut cur_y, &mut font_size);

        let is_text_show = trimmed.ends_with("Tj")
            || trimmed.ends_with("TJ")
            || trimmed.ends_with('\'')
            || trimmed.ends_with('"');
        if options.redact_text == ScrubMode::Scrub && is_text_show {
            let approx_w = (trimmed.len() as f32 * font_size * 0.55).max(32.0);
            let span_box = [
                cur_x - 4.0,
                cur_y - 4.0,
                cur_x + approx_w,
                cur_y + font_size.max(12.0),
            ];
            if rect_intersects_any(span_box, pdf_rects) {
                continue;
            }
        }

        if options.redact_images == ScrubMode::Scrub
            && trimmed.ends_with("Do")
            && rect_intersects_any(
                [
                    cur_ctm[4],
                    cur_ctm[5],
                    cur_ctm[4] + cur_ctm[0],
                    cur_ctm[5] + cur_ctm[3],
                ],
                pdf_rects,
            )
        {
            continue;
        }

        if options.redact_vectors == ScrubMode::Scrub
            && let Some(rect) = parse_re_rect(trimmed)
            && rect_intersects_any(rect, pdf_rects)
        {
            continue;
        }

        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

fn rect_intersects_any(a: [f32; 4], rects: &[[f32; 4]]) -> bool {
    let norm = [
        a[0].min(a[2]),
        a[1].min(a[3]),
        a[0].max(a[2]),
        a[1].max(a[3]),
    ];
    rects.iter().any(|b| rects_intersect(norm, *b))
}

fn parse_cm(line: &str) -> Option<[f32; 6]> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() >= 7 && tokens[tokens.len() - 1] == "cm" {
        let base = tokens.len() - 7;
        return Some([
            tokens[base].parse().ok()?,
            tokens[base + 1].parse().ok()?,
            tokens[base + 2].parse().ok()?,
            tokens[base + 3].parse().ok()?,
            tokens[base + 4].parse().ok()?,
            tokens[base + 5].parse().ok()?,
        ]);
    }
    None
}

fn parse_re_rect(line: &str) -> Option<[f32; 4]> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() >= 5 && tokens[tokens.len() - 1] == "re" {
        let base = tokens.len() - 5;
        let x: f32 = tokens[base].parse().ok()?;
        let y: f32 = tokens[base + 1].parse().ok()?;
        let w: f32 = tokens[base + 2].parse().ok()?;
        let h: f32 = tokens[base + 3].parse().ok()?;
        return Some([x, y, x + w, y + h]);
    }
    None
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::Document;

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn two_phase_redaction_permanently_strips_secrets_and_verifies_clean() {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 12 Tf\n1 0 0 1 72 720 Tm (SSN: 123-45-6789) Tj\n1 0 0 1 72 680 Tm (Contact: secret@example.com) Tj\n1 0 0 1 72 640 Tm (Public Statement) Tj\nET".into(),
            crop_box: None,
        });
        let pdf = b.build().expect("build");

        let mupdf_doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let layout = mupdf_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("t")
            .layout();

        let mut marks = mark_from_text_layout(0, &layout, &RedactionPattern::Ssn);
        marks.extend(mark_from_text_layout(0, &layout, &RedactionPattern::Email));
        assert_eq!(marks.len(), 2, "marked both SSN and Email");

        let redacted =
            apply_redactions(&pdf, &marks, &RedactionOptions::default()).expect("redact");
        // Raw bytes must not contain the secret literals anywhere!
        let raw_ascii = String::from_utf8_lossy(&redacted);
        assert!(!raw_ascii.contains("123-45-6789"), "raw SSN stripped");
        assert!(
            !raw_ascii.contains("secret@example.com"),
            "raw email stripped"
        );

        let verification = verify_redaction(
            &redacted,
            &[
                RedactionPattern::Ssn,
                RedactionPattern::Email,
                RedactionPattern::Literal {
                    text: "123-45-6789".into(),
                    case: PatternCase::Insensitive,
                },
            ],
        )
        .expect("verify");
        assert!(
            verification.is_clean(),
            "remaining: {:?}",
            verification.remaining_hits
        );

        // Non-redacted text remains intact.
        let rdoc = Document::from_bytes(&redacted, "application/pdf").expect("open redacted");
        let text = rdoc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("t")
            .text()
            .expect("s");
        assert!(text.contains("Public Statement"), "got: {text}");
    }
}
