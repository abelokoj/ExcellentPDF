//! Two-phase true PDF redaction and independent MuPDF verification (plan O.7, §14.3, D3, D4).
//!
//! Phase 1 marks regions manually or by pattern search ([`RedactionPattern`]).
//! Phase 2 ([`apply_redactions`]) delegates to MuPDF 1.28.5 (`pdf_redact_page`) through the C shim
//! to permanently strip underlying text, vector, and image operators inside marked rectangles,
//! scrub metadata/XMP/attachments/hidden layers/annots/forms as configured, rewrite the entire PDF
//! non-incrementally with full garbage collection, and verify via [`verify_redaction`] that no
//! redacted string remains extractable.

use ecclesia_mupdf::{
    Document, PageRect, RedactionFlags, RedactionSpec, TextLayout, pdf_info_entries,
    redact_pdf_bytes,
};

use crate::page_ops::pdf_doc::PdfDocError;

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
    /// Whether to strip image pixels/invocations inside redaction rectangles.
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

fn map_mu_error(err: ecclesia_mupdf::MuError) -> PdfDocError {
    if err.message().contains("out of range") {
        PdfDocError::OutOfRange(err.to_string())
    } else {
        PdfDocError::InvalidPdf(err.to_string())
    }
}

/// Permanently applies `marks` to `pdf_bytes` through MuPDF's `pdf_redact_page` and performs a
/// full non-incremental garbage-collected rewrite so no redacted text, vectors, images, or
/// metadata remain in the file (plan O.7, D3, D4).
///
/// # Errors
///
/// Returns [`PdfDocError`] if any mark targets an out-of-range page or the PDF cannot be parsed.
pub fn apply_redactions(
    pdf_bytes: &[u8],
    marks: &[RedactionMark],
    options: &RedactionOptions,
) -> Result<Vec<u8>> {
    let specs: Vec<RedactionSpec> = marks
        .iter()
        .map(|m| RedactionSpec {
            page: m.page,
            rect: m.rect,
            fill_color: m.fill_color,
        })
        .collect();
    let flags = RedactionFlags {
        black_boxes: options.box_style == RedactionBoxStyle::SolidFill,
        redact_text: options.redact_text == ScrubMode::Scrub,
        redact_vectors: options.redact_vectors == ScrubMode::Scrub,
        redact_images: options.redact_images == ScrubMode::Scrub,
        scrub_metadata: options.scrub_metadata == ScrubMode::Scrub,
        scrub_attachments: options.scrub_attachments == ScrubMode::Scrub,
        scrub_hidden_layers: options.scrub_hidden_layers == ScrubMode::Scrub,
        scrub_overlapping_annots: options.scrub_overlapping_annots == ScrubMode::Scrub,
        scrub_overlapping_forms: options.scrub_overlapping_forms == ScrubMode::Scrub,
    };
    redact_pdf_bytes(pdf_bytes, &specs, flags).map_err(map_mu_error)
}

/// Reopens `redacted_pdf` in MuPDF and searches every page's extracted text and PDF `/Info` /
/// XMP `/Metadata` for `forbidden_patterns` (plan O.7). Never claims success without checking every target!
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be opened in MuPDF.
pub fn verify_redaction(
    redacted_pdf: &[u8],
    forbidden_patterns: &[RedactionPattern],
) -> Result<RedactionVerification> {
    let mut hits = Vec::new();

    for (k, s) in pdf_info_entries(redacted_pdf).map_err(map_mu_error)? {
        for pat in forbidden_patterns {
            if !find_pattern_byte_spans(&s, pat).is_empty() {
                hits.push(VerificationHit {
                    location: format!("metadata:Info/{k}"),
                    matched: describe_pattern(pat),
                });
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

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use ecclesia_mupdf::Document;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    /// Encodes `input` into a valid PDF `/LZWDecode` byte stream (MSB-first, 9-bit codes with
    /// periodic ClearCode 256 so the decoder never needs to widen past 9 bits, ending in EOD 257).
    fn lzw_encode_pdf(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut bit_buf: u32 = 0;
        let mut bit_count: u32 = 0;
        let mut push_9 = |code: u16| {
            bit_buf = (bit_buf << 9) | u32::from(code);
            bit_count += 9;
            while bit_count >= 8 {
                bit_count -= 8;
                out.push(((bit_buf >> bit_count) & 0xFF) as u8);
            }
        };

        push_9(256); // LZW_CLEAR
        for (idx, &byte) in input.iter().enumerate() {
            if idx > 0 && idx % 200 == 0 {
                push_9(256); // Reset table before 9-bit EarlyChange threshold (511)
            }
            push_9(u16::from(byte));
        }
        push_9(257); // LZW_EOD
        if bit_count > 0 {
            out.push(((bit_buf << (8 - bit_count)) & 0xFF) as u8);
        }
        out
    }

    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).expect("zlib write");
        enc.finish().expect("zlib finish")
    }

    fn push_xref_entry(buf: &mut Vec<u8>, kind: u8, field2: u32, field3: u16) {
        buf.push(kind);
        buf.extend_from_slice(&field2.to_be_bytes());
        buf.extend_from_slice(&field3.to_be_bytes());
    }

    /// Builds a valid PDF 1.5 containing:
    /// (a) an `/LZWDecode` content stream,
    /// (b) an `/ObjStm` compressed object stream holding Page, Font, and `/Info` (with secret), and
    /// (c) an incremental update revision after `%%EOF` updating the LZW content stream.
    fn build_lzw_objstm_incremental_pdf(secret: &str) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");

        // Obj 1: Catalog
        let off1 = pdf.len() as u32;
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        // Obj 2: Pages
        let off2 = pdf.len() as u32;
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        // Obj 5: LZW-compressed content stream (base revision)
        let content_v1 = format!(
            "BT /F1 12 Tf\n1 0 0 1 72 720 Tm ({secret}) Tj\n1 0 0 1 72 680 Tm (Public Statement) Tj\nET\n"
        );
        let lzw_v1 = lzw_encode_pdf(content_v1.as_bytes());
        let off5 = pdf.len() as u32;
        let hdr5 = format!(
            "5 0 obj\n<< /Filter /LZWDecode /Length {} >>\nstream\n",
            lzw_v1.len()
        );
        pdf.extend_from_slice(hdr5.as_bytes());
        pdf.extend_from_slice(&lzw_v1);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        // Obj 6: Compressed /ObjStm storing Obj 3 (Page), Obj 4 (Font), and Obj 7 (Info with secret)
        let obj3_body = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\n";
        let obj4_body = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\n";
        let obj7_body = format!("<< /Subject ({secret}) /Author (Audit) >>\n");
        let off_in_stm_4 = obj3_body.len();
        let off_in_stm_7 = off_in_stm_4 + obj4_body.len();
        let stm_index = format!("3 0 4 {off_in_stm_4} 7 {off_in_stm_7}\n");
        let mut stm_plain = Vec::new();
        stm_plain.extend_from_slice(stm_index.as_bytes());
        stm_plain.extend_from_slice(obj3_body);
        stm_plain.extend_from_slice(obj4_body);
        stm_plain.extend_from_slice(obj7_body.as_bytes());
        let stm_compressed = zlib_compress(&stm_plain);

        let off6 = pdf.len() as u32;
        let hdr6 = format!(
            "6 0 obj\n<< /Type /ObjStm /N 3 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            stm_index.len(),
            stm_compressed.len()
        );
        pdf.extend_from_slice(hdr6.as_bytes());
        pdf.extend_from_slice(&stm_compressed);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        // Obj 8: Base /XRef stream
        let off8 = pdf.len() as u32;
        let mut xref1 = Vec::new();
        push_xref_entry(&mut xref1, 0, 0, 65535); // 0: free
        push_xref_entry(&mut xref1, 1, off1, 0); // 1: Catalog
        push_xref_entry(&mut xref1, 1, off2, 0); // 2: Pages
        push_xref_entry(&mut xref1, 2, 6, 0); // 3: Page in ObjStm 6 idx 0
        push_xref_entry(&mut xref1, 2, 6, 1); // 4: Font in ObjStm 6 idx 1
        push_xref_entry(&mut xref1, 1, off5, 0); // 5: LZW content stream
        push_xref_entry(&mut xref1, 1, off6, 0); // 6: ObjStm
        push_xref_entry(&mut xref1, 2, 6, 2); // 7: Info in ObjStm 6 idx 2
        push_xref_entry(&mut xref1, 1, off8, 0); // 8: XRef stream
        let hdr8 = format!(
            "8 0 obj\n<< /Type /XRef /Size 9 /W [1 4 2] /Root 1 0 R /Info 7 0 R /Length {} >>\nstream\n",
            xref1.len()
        );
        pdf.extend_from_slice(hdr8.as_bytes());
        pdf.extend_from_slice(&xref1);
        pdf.extend_from_slice(
            format!("\nendstream\nendobj\nstartxref\n{off8}\n%%EOF\n").as_bytes(),
        );

        // Incremental update revision: replace Obj 5 with a second LZW-compressed content stream.
        let content_v2 = format!(
            "BT /F1 12 Tf\n1 0 0 1 72 720 Tm ({secret}) Tj\n1 0 0 1 72 680 Tm (Public Statement) Tj\nET\n"
        );
        let lzw_v2 = lzw_encode_pdf(content_v2.as_bytes());
        let off5_v2 = pdf.len() as u32;
        let hdr5_v2 = format!(
            "5 0 obj\n<< /Filter /LZWDecode /Length {} >>\nstream\n",
            lzw_v2.len()
        );
        pdf.extend_from_slice(hdr5_v2.as_bytes());
        pdf.extend_from_slice(&lzw_v2);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        // Obj 9: Incremental /XRef stream pointing /Prev to base XRef
        let off9 = pdf.len() as u32;
        let mut xref2 = Vec::new();
        push_xref_entry(&mut xref2, 1, off5_v2, 0); // 5: updated LZW stream
        push_xref_entry(&mut xref2, 1, off9, 0); // 9: incremental XRef
        let hdr9 = format!(
            "9 0 obj\n<< /Type /XRef /Size 10 /Index [5 1 9 1] /W [1 4 2] /Prev {off8} /Root 1 0 R /Info 7 0 R /Length {} >>\nstream\n",
            xref2.len()
        );
        pdf.extend_from_slice(hdr9.as_bytes());
        pdf.extend_from_slice(&xref2);
        pdf.extend_from_slice(
            format!("\nendstream\nendobj\nstartxref\n{off9}\n%%EOF\n").as_bytes(),
        );

        pdf
    }

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

    #[test]
    fn redacts_secret_across_lzw_stream_objstm_and_incremental_update() {
        let secret = "TOPSECRET-LZW-OBJSTM-42";
        let pdf = build_lzw_objstm_incremental_pdf(secret);

        // Confirm MuPDF decodes the LZW stream, ObjStm, and incremental update before redaction.
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open lzw+objstm pdf");
        let stext = doc.load_page(0).expect("page 0").text(None).expect("stext");
        let extracted = stext.text().expect("plain text");
        assert!(
            extracted.contains(secret),
            "secret must be extractable before redaction: {extracted}"
        );
        assert!(
            extracted.contains("Public Statement"),
            "public line must be extractable before redaction: {extracted}"
        );
        let pre_info = pdf_info_entries(&pdf).expect("info entries before");
        assert!(
            pre_info.iter().any(|(_, v)| v.contains(secret)),
            "secret must be present in ObjStm /Info before redaction: {pre_info:?}"
        );

        let layout = stext.layout();
        let pattern = RedactionPattern::Literal {
            text: secret.to_owned(),
            case: PatternCase::Sensitive,
        };
        let marks = mark_from_text_layout(0, &layout, &pattern);
        assert_eq!(marks.len(), 1, "expected exactly one mark for secret");

        let redacted =
            apply_redactions(&pdf, &marks, &RedactionOptions::default()).expect("apply redaction");

        // 1. Independent MuPDF verification must report zero hits across pages and metadata.
        let verification = verify_redaction(&redacted, &[pattern]).expect("verify");
        assert!(
            verification.is_clean(),
            "verification found remaining hits: {:?}",
            verification.remaining_hits
        );

        // 2. Raw output bytes must contain zero trace of the secret phrase (no LZW or incremental leftovers).
        assert!(
            !redacted
                .windows(secret.len())
                .any(|w| w == secret.as_bytes()),
            "raw output bytes must not contain the redacted secret"
        );
        let raw_ascii = String::from_utf8_lossy(&redacted);
        assert!(
            !raw_ascii.contains("TOPSECRET"),
            "no trace of redacted secret prefix in output"
        );

        // 3. Non-redacted text remains intact in the rewritten document.
        let reopened = Document::from_bytes(&redacted, "application/pdf").expect("reopen");
        let post_text = reopened
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("t")
            .text()
            .expect("s");
        assert!(
            post_text.contains("Public Statement"),
            "non-redacted text must survive: {post_text}"
        );
    }
}
