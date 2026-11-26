//! PDF ByteRange `/Sig` container builder, visual signature appearance generator, and
//! cryptographic signature verifier (plan O.14, O.15).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ed25519_dalek::SigningKey;
use excellent_mupdf::{PagePoint, PageRect};

use crate::forms::pdf_codec::{
    IncrementalPdfWriter, PdfIndex, PdfVal, encode_pdf_literal, format_pdf_val,
};

use super::cms::{CmsBuildParams, build_cms_signed_data, hash_pdf_byte_range, verify_cms_detached};
use super::der::{ChainTrust, SigningError, TrustStore, X509Certificate, hex_upper};
use super::ltv::parse_and_verify_timestamp_token;
use super::mdp::DocMdpPermission;

/// Fixed byte budget reserved for the DER-encoded CMS/PKCS#7 `/Contents` payload (8 KiB binary = 16 KiB hex).
const SIGNATURE_HEX_CAPACITY: usize = 16_384;

/// Fixed placeholder literal used in `/ByteRange` so in-place patching never shifts byte offsets.
const BYTE_RANGE_PLACEHOLDER: &str = "[0 0000000000 0000000000 0000000000]";

/// Trust outcome for a PDF digital signature (plan O.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustStatus {
    /// Cryptographic digest, CMS signature, and certificate chain to a trust anchor all verified.
    Trusted,
    /// Digest and CMS signature verified, but the certificate issuer is not in the trust store.
    UnknownIssuer,
    /// Document bytes inside the signed range or the CMS/certificate signature was tampered with.
    Tampered,
    /// Signature `/ByteRange` is malformed, truncated, or does not cover the signed revision cleanly.
    ByteRangeIncomplete,
}

/// Visual content drawn inside a visible signature widget (plan O.14).
///
/// Note: A visual image or typed name alone is never treated as a cryptographic signature; it is
/// strictly the visual appearance paired with an Ed25519 CMS `/Sig` container.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum VisibleSignatureContent {
    /// Typed signature text rendered in the signature box.
    Typed {
        /// Display name text.
        text: String,
        /// Font resource name (`"Helv"`, `"TiRo"`, `"Cour"`).
        font_name: String,
    },
    /// Freehand handwritten signature strokes in normalized widget coordinates (`0.0..=1.0`).
    Handwritten {
        /// Polyline strokes in normalized `[0, 1] x [0, 1]` coordinates inside the signature box.
        strokes: Vec<Vec<PagePoint>>,
        /// Stroke width in points.
        stroke_width: f32,
    },
    /// Raster RGB image drawn into the signature box.
    Image {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Raw 8-bit RGB pixel data (`width * height * 3` bytes).
        rgb_bytes: Vec<u8>,
    },
}

/// Visual placement and content of a visible signature widget.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleAppearance {
    /// Zero-based page index where the signature widget is placed.
    pub page: usize,
    /// Bounding box in top-down page points.
    pub rect: PageRect,
    /// Visual mark drawn inside the widget.
    pub content: VisibleSignatureContent,
    /// Optional metadata line (signer, reason, date) shown beneath the mark.
    pub caption: Option<String>,
}

/// Whether a signature is visible on a page or invisible (certification / approval only).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SignatureAppearance {
    /// Invisible signature widget (`[0 0 0 0]` rect on page 0).
    #[default]
    Invisible,
    /// Visible widget on the specified page with a Form XObject appearance stream.
    Visible(VisibleAppearance),
}

/// Parameters for cryptographically signing a PDF document (plan O.14).
pub struct SignDocumentRequest<'a> {
    /// Signer X.509 certificate.
    pub signer_cert: &'a X509Certificate,
    /// Intermediate CA certificates to embed in CMS `SignedData`.
    pub chain_certs: &'a [X509Certificate],
    /// Ed25519 private signing key matching `signer_cert`.
    pub signing_key: &'a SigningKey,
    /// Signature form field name (e.g. `"Signature1"`).
    pub field_name: String,
    /// Optional signing reason (`/Reason`).
    pub reason: Option<String>,
    /// Optional signing location (`/Location`).
    pub location: Option<String>,
    /// Signing time as Unix UTC seconds.
    pub signing_time: i64,
    /// Visible or invisible widget appearance.
    pub appearance: SignatureAppearance,
    /// Optional DocMDP certification permission (`/P 1|2|3`).
    pub certification: Option<DocMdpPermission>,
    /// Optional DER-encoded RFC 3161 `TimeStampToken` to embed as an unsigned attribute (PAdES B-T).
    pub timestamp_token_der: Option<&'a [u8]>,
}

/// Extracted signer metadata for display in the signature inspector (plan O.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerDetails {
    /// Signer common name (`CN`).
    pub common_name: String,
    /// Signer organization (`O`).
    pub organization: Option<String>,
    /// Signer country (`C`).
    pub country: Option<String>,
    /// Issuer common name (`CN`).
    pub issuer_common_name: String,
    /// Certificate serial number in uppercase hex.
    pub serial_hex: String,
    /// Signing reason (`/Reason`).
    pub reason: Option<String>,
    /// Signing location (`/Location`).
    pub location: Option<String>,
    /// Signing timestamp in Unix UTC seconds.
    pub signing_time: Option<i64>,
    /// PDF `/SubFilter` (e.g. `"ETSI.CAdES.detached"` or `"ETSI.RFC3161"`).
    pub sub_filter: String,
    /// Digest algorithm name (e.g. `"SHA-256"`).
    pub digest_algorithm: String,
    /// Signature algorithm name (e.g. `"Ed25519"`).
    pub signature_algorithm: String,
    /// Whether this entry is a standalone RFC 3161 `/DocTimeStamp`.
    pub is_document_timestamp: bool,
}

/// Full verification report for one signature in a PDF (plan O.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureVerification {
    /// Associated signature field name (`/T`), if present.
    pub field_name: String,
    /// Evaluated trust status.
    pub trust_status: TrustStatus,
    /// Signer metadata and algorithms.
    pub signer: SignerDetails,
    /// Raw `/ByteRange` array `[offset1, len1, offset2, len2]`.
    pub byte_range: [usize; 4],
    /// Byte offset where the signed revision ends (`offset2 + len2`).
    pub covered_revision_end: usize,
    /// Whether incremental updates exist after this signature's revision.
    pub modified_after_signing: bool,
    /// DocMDP permission level if this signature certifies the document.
    pub doc_mdp: Option<DocMdpPermission>,
    /// Whether a verified RFC 3161 timestamp is present.
    pub has_timestamp: bool,
    /// Timestamp generation time in Unix UTC seconds, if present.
    pub timestamp_gen_time: Option<i64>,
    /// Whether `signing-certificate-v2` / PAdES-B-B attributes are present.
    pub has_signing_cert_v2: bool,
    /// SHA-1 uppercase hex key of `/Contents` bytes (used for `/DSS /VRI` lookup).
    pub vri_key: String,
    /// Diagnostic failure detail when not [`TrustStatus::Trusted`].
    pub failure_reason: Option<String>,
}

/// Cryptographically signs `pdf_bytes` using an incremental update, `/ByteRange` digest,
/// visual appearance stream, and DER-encoded CMS/PKCS#7 Ed25519 `SignedData` (plan O.14).
///
/// # Errors
///
/// Returns [`SigningError`] if the PDF is malformed, the target page is out of range, or the CMS
/// signature exceeds the reserved `/Contents` container capacity.
pub fn sign_pdf_bytes(
    pdf_bytes: &[u8],
    request: &SignDocumentRequest<'_>,
) -> Result<Vec<u8>, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let pages = index.collect_pages()?;
    let target_page = match &request.appearance {
        SignatureAppearance::Invisible => 0,
        SignatureAppearance::Visible(vis) => vis.page,
    };
    let &(page_obj_id, page_gen) = pages.get(target_page).ok_or_else(|| {
        SigningError::PdfStructure(format!("signature page {target_page} out of range"))
    })?;

    let (root_id, root_gen, mut catalog) = index.catalog()?;
    let page_val = index.read_value(page_obj_id)?;
    let mut page_dict = page_val
        .as_dict()
        .cloned()
        .ok_or_else(|| SigningError::PdfStructure("target page has no dict".to_owned()))?;
    let page_height = page_dict
        .get("CropBox")
        .or_else(|| page_dict.get("MediaBox"))
        .and_then(PdfVal::as_array)
        .and_then(|arr| arr.get(3).and_then(PdfVal::as_f64))
        .unwrap_or(792.0) as f32;

    let mut writer = IncrementalPdfWriter::from_index(&index)?;

    // Allocate signature value object first so we know its byte range in the finished buffer.
    let sig_id = writer.alloc_id();
    let widget_id = writer.alloc_id();

    let mut sig_dict = format!(
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.CAdES.detached /ByteRange {BYTE_RANGE_PLACEHOLDER} /Contents <{}>",
        "0".repeat(SIGNATURE_HEX_CAPACITY)
    );
    let _ = write!(
        sig_dict,
        " /Name {}",
        encode_pdf_literal(&request.signer_cert.subject.common_name)
    );
    if let Some(ref reason) = request.reason {
        let _ = write!(sig_dict, " /Reason {}", encode_pdf_literal(reason));
    }
    if let Some(ref location) = request.location {
        let _ = write!(sig_dict, " /Location {}", encode_pdf_literal(location));
    }
    let pdf_date = format_pdf_date(request.signing_time);
    let _ = write!(sig_dict, " /M {}", encode_pdf_literal(&pdf_date));

    if let Some(mdp) = request.certification {
        let _ = write!(
            sig_dict,
            " /Reference [ << /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type /TransformParams /P {} /V /1.2 >> >> ]",
            mdp.code()
        );
    }
    sig_dict.push_str(" >>");
    let sig_obj_offset = writer.write_object(sig_id, 0, &sig_dict);

    // Visual appearance Form XObject if visible.
    let (pdf_rect, ap_ref_entry) = match &request.appearance {
        SignatureAppearance::Invisible => ([0.0f32, 0.0, 0.0, 0.0], None),
        SignatureAppearance::Visible(vis) => {
            let llx = vis.rect.x0.min(vis.rect.x1);
            let urx = vis.rect.x0.max(vis.rect.x1);
            let lly = (page_height - vis.rect.y1.max(vis.rect.y0)).max(0.0);
            let ury = (page_height - vis.rect.y0.min(vis.rect.y1)).max(lly + 10.0);
            let ap_id = write_visual_sig_appearance(
                &mut writer,
                urx - llx,
                ury - lly,
                vis,
                &request.signer_cert.subject.common_name,
            );
            ([llx, lly, urx, ury], Some(ap_id))
        }
    };

    let mut widget_body = format!(
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T {} /Rect [{:.2} {:.2} {:.2} {:.2}] /F 132 /P {} {} R /V {} 0 R",
        encode_pdf_literal(&request.field_name),
        pdf_rect[0],
        pdf_rect[1],
        pdf_rect[2],
        pdf_rect[3],
        page_obj_id,
        page_gen,
        sig_id
    );
    if let Some(ap_id) = ap_ref_entry {
        let _ = write!(widget_body, " /AP << /N {ap_id} 0 R >>");
    }
    widget_body.push_str(" >>");
    writer.write_object(widget_id, 0, &widget_body);

    // Attach widget to page /Annots.
    let mut annots = if let Some(ann_val) = page_dict.get("Annots") {
        let mut owned = None;
        index
            .resolve(ann_val, &mut owned)?
            .as_array()
            .map(<[PdfVal]>::to_vec)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    annots.push(PdfVal::Ref(widget_id, 0));
    page_dict.insert("Annots".to_owned(), PdfVal::Array(annots));
    writer.write_object(
        page_obj_id,
        page_gen,
        &format_pdf_val(&PdfVal::Dict(page_dict)),
    );

    // Attach widget to /AcroForm /Fields and set /SigFlags 3 (SignaturesExist | AppendOnly).
    let (acro_ref, mut acro_dict) = if let Some(acro_val) = catalog.get("AcroForm").cloned() {
        let aref = acro_val.as_ref();
        let mut owned = None;
        let dict = index
            .resolve(&acro_val, &mut owned)?
            .as_dict()
            .cloned()
            .unwrap_or_default();
        (aref, dict)
    } else {
        (None, BTreeMap::new())
    };
    let mut fields = if let Some(fval) = acro_dict.get("Fields") {
        let mut owned = None;
        index
            .resolve(fval, &mut owned)?
            .as_array()
            .map(<[PdfVal]>::to_vec)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    fields.push(PdfVal::Ref(widget_id, 0));
    acro_dict.insert("Fields".to_owned(), PdfVal::Array(fields));
    acro_dict.insert("SigFlags".to_owned(), PdfVal::Int(3));
    if let Some((aid, agen)) = acro_ref {
        writer.write_object(aid, agen, &format_pdf_val(&PdfVal::Dict(acro_dict)));
    } else {
        let aid = writer.alloc_id();
        writer.write_object(aid, 0, &format_pdf_val(&PdfVal::Dict(acro_dict)));
        catalog.insert("AcroForm".to_owned(), PdfVal::Ref(aid, 0));
        writer.write_object(root_id, root_gen, &format_pdf_val(&PdfVal::Dict(catalog)));
    }

    let mut out = writer.finish();
    patch_byte_range_and_contents(&mut out, sig_obj_offset, |digest| {
        build_cms_signed_data(
            &digest,
            &CmsBuildParams {
                signer_cert: request.signer_cert,
                chain_certs: request.chain_certs,
                signing_key: request.signing_key,
                signing_time: request.signing_time,
                timestamp_token_der: request.timestamp_token_der,
            },
        )
    })?;
    Ok(out)
}

/// Shared ByteRange `/Contents` reserve-patch-hash-fill helper used by both `/Sig` and `/DocTimeStamp`.
pub(crate) fn patch_byte_range_and_contents(
    pdf_bytes: &mut [u8],
    sig_obj_offset: usize,
    build_payload: impl FnOnce([u8; 32]) -> Result<Vec<u8>, SigningError>,
) -> Result<(), SigningError> {
    let tail = &pdf_bytes[sig_obj_offset..];
    let br_rel = find_subslice(tail, BYTE_RANGE_PLACEHOLDER.as_bytes()).ok_or_else(|| {
        SigningError::PdfStructure("missing /ByteRange placeholder in signature object".to_owned())
    })?;
    let br_pos = sig_obj_offset + br_rel;

    let contents_marker = b"/Contents <";
    let c_rel = find_subslice(tail, contents_marker).ok_or_else(|| {
        SigningError::PdfStructure("missing /Contents placeholder in signature object".to_owned())
    })?;
    let lt_pos = sig_obj_offset + c_rel + b"/Contents ".len();
    let gt_rel = tail[c_rel + contents_marker.len()..]
        .iter()
        .position(|&b| b == b'>')
        .ok_or_else(|| {
            SigningError::PdfStructure("unterminated /Contents hex placeholder".to_owned())
        })?;
    let gt_end = sig_obj_offset + c_rel + contents_marker.len() + gt_rel + 1;

    let br = [0usize, lt_pos, gt_end, pdf_bytes.len() - gt_end];
    let formatted_br = format!("[0 {:010} {:010} {:010}]", br[1], br[2], br[3]);
    if formatted_br.len() != BYTE_RANGE_PLACEHOLDER.len() {
        return Err(SigningError::PdfStructure(
            "ByteRange formatted length mismatch".to_owned(),
        ));
    }
    pdf_bytes[br_pos..br_pos + formatted_br.len()].copy_from_slice(formatted_br.as_bytes());

    let digest = hash_pdf_byte_range(pdf_bytes, br)
        .ok_or_else(|| SigningError::PdfStructure("computed ByteRange out of bounds".to_owned()))?;
    let der = build_payload(digest)?;
    let hex_payload = hex_upper(&der);
    let hex_slot_len = gt_end - 1 - (lt_pos + 1);
    if hex_payload.len() > hex_slot_len {
        return Err(SigningError::PdfStructure(format!(
            "CMS/TSA payload ({} hex chars) exceeds reserved /Contents slot ({hex_slot_len})",
            hex_payload.len()
        )));
    }
    pdf_bytes[lt_pos + 1..lt_pos + 1 + hex_payload.len()].copy_from_slice(hex_payload.as_bytes());
    Ok(())
}

pub(crate) const fn signature_hex_capacity() -> usize {
    SIGNATURE_HEX_CAPACITY
}

pub(crate) const fn byte_range_placeholder() -> &'static str {
    BYTE_RANGE_PLACEHOLDER
}

/// Extracts and verifies every digital signature and document timestamp in `pdf_bytes` against `trust_store` (plan O.15).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed as a PDF.
pub fn verify_pdf_signatures(
    pdf_bytes: &[u8],
    trust_store: &TrustStore,
) -> Result<Vec<SignatureVerification>, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let mut results = Vec::new();
    let mut seen_sig_ids = std::collections::BTreeSet::new();

    // Map signature object id -> field name from /AcroForm /Fields.
    let mut sig_field_names: BTreeMap<u32, String> = BTreeMap::new();
    if let Ok((_, _, catalog)) = index.catalog()
        && let Some(acro_val) = catalog.get("AcroForm")
    {
        let mut owned_acro = None;
        if let Some(acro_dict) = index.resolve(acro_val, &mut owned_acro)?.as_dict()
            && let Some(fields_val) = acro_dict.get("Fields")
        {
            let mut owned_fields = None;
            if let Some(arr) = index.resolve(fields_val, &mut owned_fields)?.as_array() {
                collect_sig_field_names(&index, arr, "", &mut sig_field_names);
            }
        }
    }

    let obj_ids: Vec<u32> = index.objects.keys().copied().collect();
    for obj_id in obj_ids {
        let Ok(val) = index.read_value(obj_id) else {
            continue;
        };
        let Some(dict) = val.as_dict() else {
            continue;
        };
        let typ = dict.get("Type").and_then(PdfVal::as_name);
        let is_sig =
            typ == Some("Sig") || (dict.contains_key("ByteRange") && dict.contains_key("Contents"));
        let is_doc_ts = typ == Some("DocTimeStamp");
        if (!is_sig && !is_doc_ts) || !seen_sig_ids.insert(obj_id) {
            continue;
        }

        let field_name = sig_field_names
            .get(&obj_id)
            .cloned()
            .or_else(|| dict.get("Name").and_then(PdfVal::as_text))
            .unwrap_or_else(|| format!("Signature_{obj_id}"));
        let verification =
            verify_single_signature_dict(pdf_bytes, &field_name, dict, is_doc_ts, trust_store);
        results.push(verification);
    }

    results.sort_by_key(|v| v.covered_revision_end);
    Ok(results)
}

fn collect_sig_field_names(
    index: &PdfIndex<'_>,
    arr: &[PdfVal],
    prefix: &str,
    out: &mut BTreeMap<u32, String>,
) {
    for item in arr {
        let Some((fid, _)) = item.as_ref() else {
            continue;
        };
        let Ok(val) = index.read_value(fid) else {
            continue;
        };
        let Some(dict) = val.as_dict() else {
            continue;
        };
        let part = dict.get("T").and_then(PdfVal::as_text).unwrap_or_default();
        let full = if prefix.is_empty() {
            part
        } else if part.is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}.{part}")
        };
        if let Some((sig_id, _)) = dict.get("V").and_then(PdfVal::as_ref) {
            out.insert(sig_id, full.clone());
        }
        if let Some(kids_val) = dict.get("Kids") {
            let mut owned = None;
            if let Ok(resolved) = index.resolve(kids_val, &mut owned)
                && let Some(kids) = resolved.as_array()
            {
                collect_sig_field_names(index, kids, &full, out);
            }
        }
    }
}

fn verify_single_signature_dict(
    pdf_bytes: &[u8],
    field_name: &str,
    dict: &BTreeMap<String, PdfVal>,
    is_doc_ts: bool,
    trust_store: &TrustStore,
) -> SignatureVerification {
    let sub_filter = dict
        .get("SubFilter")
        .and_then(PdfVal::as_name)
        .unwrap_or("adbe.pkcs7.detached")
        .to_owned();
    let reason = dict.get("Reason").and_then(PdfVal::as_text);
    let location = dict.get("Location").and_then(PdfVal::as_text);
    let declared_name = dict
        .get("Name")
        .and_then(PdfVal::as_text)
        .unwrap_or_else(|| field_name.to_owned());
    let doc_mdp = parse_doc_mdp_from_sig_dict(dict);

    let br_opt = dict
        .get("ByteRange")
        .and_then(PdfVal::as_array)
        .and_then(|arr| {
            if arr.len() != 4 {
                return None;
            }
            Some([
                usize::try_from(arr[0].as_int()?).ok()?,
                usize::try_from(arr[1].as_int()?).ok()?,
                usize::try_from(arr[2].as_int()?).ok()?,
                usize::try_from(arr[3].as_int()?).ok()?,
            ])
        });

    let raw_contents = dict
        .get("Contents")
        .and_then(PdfVal::as_bytes)
        .unwrap_or(&[]);
    let vri_key = sha1_upper_hex(raw_contents);

    let default_signer = SignerDetails {
        common_name: declared_name,
        organization: None,
        country: None,
        issuer_common_name: String::new(),
        serial_hex: String::new(),
        reason: reason.clone(),
        location: location.clone(),
        signing_time: None,
        sub_filter: sub_filter.clone(),
        digest_algorithm: "SHA-256".to_owned(),
        signature_algorithm: "Ed25519".to_owned(),
        is_document_timestamp: is_doc_ts,
    };

    let Some(br) = br_opt else {
        return SignatureVerification {
            field_name: field_name.to_owned(),
            trust_status: TrustStatus::ByteRangeIncomplete,
            signer: default_signer,
            byte_range: [0; 4],
            covered_revision_end: 0,
            modified_after_signing: false,
            doc_mdp,
            has_timestamp: false,
            timestamp_gen_time: None,
            has_signing_cert_v2: false,
            vri_key,
            failure_reason: Some("missing or malformed /ByteRange".to_owned()),
        };
    };

    let covered_end = br[2].saturating_add(br[3]);
    let range_sane = br[0] == 0
        && br[1] > 0
        && br[2] > br[1]
        && covered_end <= pdf_bytes.len()
        && pdf_bytes.get(br[1]) == Some(&b'<')
        && pdf_bytes.get(br[2] - 1) == Some(&b'>');

    if !range_sane {
        return SignatureVerification {
            field_name: field_name.to_owned(),
            trust_status: TrustStatus::ByteRangeIncomplete,
            signer: default_signer,
            byte_range: br,
            covered_revision_end: covered_end.min(pdf_bytes.len()),
            modified_after_signing: covered_end < pdf_bytes.len(),
            doc_mdp,
            has_timestamp: false,
            timestamp_gen_time: None,
            has_signing_cert_v2: false,
            vri_key,
            failure_reason: Some("ByteRange does not bracket /Contents hex bounds".to_owned()),
        };
    }

    // Check whether trailing bytes after `covered_end` exist, and whether the covered slice ended on a valid `%%EOF`.
    let covered_tail_trimmed = pdf_bytes[..covered_end].trim_ascii_end();
    if !covered_tail_trimmed.ends_with(b"%%EOF") {
        return SignatureVerification {
            field_name: field_name.to_owned(),
            trust_status: TrustStatus::ByteRangeIncomplete,
            signer: default_signer,
            byte_range: br,
            covered_revision_end: covered_end,
            modified_after_signing: covered_end < pdf_bytes.len(),
            doc_mdp,
            has_timestamp: false,
            timestamp_gen_time: None,
            has_signing_cert_v2: false,
            vri_key,
            failure_reason: Some("ByteRange does not end at a PDF %%EOF marker".to_owned()),
        };
    }

    let modified_after_signing = covered_end < pdf_bytes.len();
    let Some(digest) = hash_pdf_byte_range(pdf_bytes, br) else {
        return SignatureVerification {
            field_name: field_name.to_owned(),
            trust_status: TrustStatus::ByteRangeIncomplete,
            signer: default_signer,
            byte_range: br,
            covered_revision_end: covered_end,
            modified_after_signing,
            doc_mdp,
            has_timestamp: false,
            timestamp_gen_time: None,
            has_signing_cert_v2: false,
            vri_key,
            failure_reason: Some("failed to hash ByteRange".to_owned()),
        };
    };

    if is_doc_ts || sub_filter == "ETSI.RFC3161" {
        match parse_and_verify_timestamp_token(raw_contents, &digest, Some(trust_store)) {
            Ok(ts) => {
                let mut signer = default_signer;
                signer.common_name = ts
                    .tsa_common_name
                    .clone()
                    .unwrap_or_else(|| "Time-Stamping Authority".to_owned());
                signer.signing_time = Some(ts.gen_time);
                let trust_status = match ts.chain_trust {
                    ChainTrust::Trusted { .. } => TrustStatus::Trusted,
                    ChainTrust::UnknownIssuer => TrustStatus::UnknownIssuer,
                    ChainTrust::Tampered(_) => TrustStatus::Tampered,
                };
                return SignatureVerification {
                    field_name: field_name.to_owned(),
                    trust_status,
                    signer,
                    byte_range: br,
                    covered_revision_end: covered_end,
                    modified_after_signing,
                    doc_mdp,
                    has_timestamp: true,
                    timestamp_gen_time: Some(ts.gen_time),
                    has_signing_cert_v2: true,
                    vri_key,
                    failure_reason: ts.failure_reason,
                };
            }
            Err(err) => {
                return SignatureVerification {
                    field_name: field_name.to_owned(),
                    trust_status: TrustStatus::Tampered,
                    signer: default_signer,
                    byte_range: br,
                    covered_revision_end: covered_end,
                    modified_after_signing,
                    doc_mdp,
                    has_timestamp: false,
                    timestamp_gen_time: None,
                    has_signing_cert_v2: false,
                    vri_key,
                    failure_reason: Some(err.to_string()),
                };
            }
        }
    }

    match verify_cms_detached(raw_contents, &digest, trust_store, None) {
        Ok(cms) => {
            let mut signer = default_signer;
            signer.digest_algorithm = cms.digest_algorithm_name;
            signer.signature_algorithm = cms.signature_algorithm_name;
            signer.signing_time = cms.signing_time;
            if let Some(ref cert) = cms.signer_cert {
                signer.common_name = cert.subject.common_name.clone();
                signer.organization = cert.subject.organization.clone();
                signer.country = cert.subject.country.clone();
                signer.issuer_common_name = cert.issuer.common_name.clone();
                signer.serial_hex = cert.serial_hex();
            }

            let mut timestamp_gen_time = None;
            let mut has_timestamp = false;
            if let Some(tst_der) = cms.timestamp_tokens.first()
                && let Ok(sig_hash) = sha256_of_cms_signature(raw_contents)
                && let Ok(ts) =
                    parse_and_verify_timestamp_token(tst_der, &sig_hash, Some(trust_store))
                && ts.imprint_valid
                && ts.signature_valid
            {
                has_timestamp = true;
                timestamp_gen_time = Some(ts.gen_time);
            }

            let trust_status = if !cms.digest_valid || !cms.signature_valid {
                TrustStatus::Tampered
            } else {
                match &cms.chain_trust {
                    ChainTrust::Trusted { .. } => TrustStatus::Trusted,
                    ChainTrust::UnknownIssuer => TrustStatus::UnknownIssuer,
                    ChainTrust::Tampered(_) => TrustStatus::Tampered,
                }
            };

            let failure_reason = cms.failure_reason.or_else(|| match &cms.chain_trust {
                ChainTrust::Tampered(msg) => Some(msg.clone()),
                ChainTrust::UnknownIssuer => {
                    Some("signer certificate issuer is not in the trust store".to_owned())
                }
                ChainTrust::Trusted { .. } => None,
            });

            SignatureVerification {
                field_name: field_name.to_owned(),
                trust_status,
                signer,
                byte_range: br,
                covered_revision_end: covered_end,
                modified_after_signing,
                doc_mdp,
                has_timestamp,
                timestamp_gen_time,
                has_signing_cert_v2: sub_filter == "ETSI.CAdES.detached",
                vri_key,
                failure_reason,
            }
        }
        Err(err) => SignatureVerification {
            field_name: field_name.to_owned(),
            trust_status: TrustStatus::Tampered,
            signer: default_signer,
            byte_range: br,
            covered_revision_end: covered_end,
            modified_after_signing,
            doc_mdp,
            has_timestamp: false,
            timestamp_gen_time: None,
            has_signing_cert_v2: false,
            vri_key,
            failure_reason: Some(err.to_string()),
        },
    }
}

fn sha256_of_cms_signature(cms_der: &[u8]) -> Result<[u8; 32], SigningError> {
    use sha2::{Digest, Sha256};
    let parsed = super::cms::parse_cms_signed_data(cms_der)?;
    Ok(Sha256::digest(&parsed.signature).into())
}

pub(crate) fn parse_doc_mdp_from_sig_dict(
    dict: &BTreeMap<String, PdfVal>,
) -> Option<DocMdpPermission> {
    let refs = dict.get("Reference").and_then(PdfVal::as_array)?;
    for item in refs {
        let Some(rdict) = item.as_dict() else {
            continue;
        };
        if rdict.get("TransformMethod").and_then(PdfVal::as_name) == Some("DocMDP")
            && let Some(tp) = rdict.get("TransformParams").and_then(PdfVal::as_dict)
        {
            let p = tp.get("P").and_then(PdfVal::as_int).unwrap_or(2);
            return DocMdpPermission::from_code(p);
        }
    }
    None
}

fn write_visual_sig_appearance(
    writer: &mut IncrementalPdfWriter,
    width: f32,
    height: f32,
    vis: &VisibleAppearance,
    signer_name: &str,
) -> u32 {
    let w = width.max(40.0);
    let h = height.max(20.0);
    let helv_id = writer.alloc_id();
    writer.write_object(
        helv_id,
        0,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );

    let mut ops = format!(
        "0.97 0.98 1.0 rg 0 0 {w:.2} {h:.2} re f 0.2 0.35 0.65 RG 1 w 0.5 0.5 {:.2} {:.2} re S\n",
        w - 1.0,
        h - 1.0
    );
    let mut xobject_res = String::new();

    match &vis.content {
        VisibleSignatureContent::Typed { text, font_name } => {
            let label = if text.is_empty() {
                signer_name
            } else {
                text.as_str()
            };
            let _ = writeln!(
                ops,
                "BT /{} 13 Tf 0.08 0.12 0.2 rg 8 {:.2} Td {} Tj ET",
                font_name,
                (h * 0.55).max(10.0),
                encode_pdf_literal(label)
            );
        }
        VisibleSignatureContent::Handwritten {
            strokes,
            stroke_width,
        } => {
            let _ = writeln!(ops, "0.05 0.1 0.35 RG {stroke_width:.2} w 1 J 1 j");
            for stroke in strokes {
                if let Some((first, rest)) = stroke.split_first() {
                    let _ = writeln!(ops, "{:.2} {:.2} m", first.x * w, (1.0 - first.y) * h);
                    for pt in rest {
                        let _ = writeln!(ops, "{:.2} {:.2} l", pt.x * w, (1.0 - pt.y) * h);
                    }
                    ops.push_str("S\n");
                }
            }
        }
        VisibleSignatureContent::Image {
            width: iw,
            height: ih,
            rgb_bytes,
        } => {
            if *iw > 0 && *ih > 0 && !rgb_bytes.is_empty() {
                let img_id = writer.alloc_id();
                let img_dict = format!(
                    "/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8"
                );
                writer.write_stream_object(img_id, 0, &img_dict, rgb_bytes);
                let _ = write!(xobject_res, " /XObject << /SigImg {img_id} 0 R >>");
                let _ = writeln!(ops, "q {w:.2} 0 0 {h:.2} 0 0 cm /SigImg Do Q");
            }
        }
    }

    if let Some(ref caption) = vis.caption {
        let _ = writeln!(
            ops,
            "BT /Helv 7.5 Tf 0.25 0.25 0.3 rg 8 5 Td {} Tj ET",
            encode_pdf_literal(caption)
        );
    }

    let font_key = match &vis.content {
        VisibleSignatureContent::Typed { font_name, .. } => font_name.as_str(),
        _ => "Helv",
    };
    let dict = format!(
        "/Type /XObject /Subtype /Form /BBox [0 0 {w:.2} {h:.2}] /Resources << /Font << /{font_key} {helv_id} 0 R >>{xobject_res} >>"
    );
    let ap_id = writer.alloc_id();
    writer.write_stream_object(ap_id, 0, &dict, ops.as_bytes());
    ap_id
}

fn format_pdf_date(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let rem = unix_secs.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    format!("D:{year:04}{m:02}{d:02}{hour:02}{min:02}{sec:02}Z")
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Minimal FIPS 180-4 SHA-1 used exclusively for PDF `/DSS /VRI` dictionary keys (ISO 32000-2 §12.8.4.3).
pub(crate) fn sha1_upper_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    hex_upper(&digest)
}
