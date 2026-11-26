//! PAdES Long-Term Validation (plan O.20): Document Security Store (`/DSS` with `/Certs`,
//! `/OCSPs`, `/CRLs`, `/VRI`), RFC 3161 TSA request/response builder and verifier, `/DocTimeStamp`,
//! and PAdES B-B / B-T / B-LT / B-LTA conformance checking.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::forms::pdf_codec::{IncrementalPdfWriter, PdfIndex, PdfVal, format_pdf_val};

use super::cms::{build_cms_signed_data, parse_cms_signed_data, verify_cms_detached};
use super::der::{
    AlgParams, ChainTrust, OID_SHA256, OID_TSA_POLICY_DEFAULT, OID_TST_INFO, SigningError,
    TAG_INTEGER, TAG_OCTET_STRING, TrustStore, X509Certificate, encode_alg_id, encode_boolean_true,
    encode_context_constructed, encode_generalized_time, encode_octet_string, encode_oid,
    encode_sequence, encode_u64, parse_der_prefix,
};
use super::pdf_sig::{
    TrustStatus, byte_range_placeholder, patch_byte_range_and_contents, signature_hex_capacity,
    verify_pdf_signatures,
};

/// PAdES conformance profile achieved by a signed PDF (ETSI EN 319 142-1, plan O.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PadesLevel {
    /// No valid PAdES signature present.
    None,
    /// PAdES-B-B: baseline detached signature with `signing-certificate-v2`.
    B,
    /// PAdES-B-T: B-B plus verified RFC 3161 timestamp token.
    T,
    /// PAdES-B-LT: B-T plus embedded `/DSS` validation data (certificates, OCSP/CRL, `/VRI`).
    LT,
    /// PAdES-B-LTA: B-LT plus a document-level `/DocTimeStamp` covering the `/DSS`.
    LTA,
}

/// Consent switch for making outbound HTTP requests to an RFC 3161 Time-Stamping Authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TsaConsent {
    /// Offline only: refuse outbound network connections.
    #[default]
    Denied,
    /// User explicitly consented to contacting the configured TSA URL.
    Allowed,
}

/// Parsed RFC 3161 `TimeStampReq` (RFC 3161 §2.4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeStampRequest {
    /// Hash algorithm OID (e.g. SHA-256).
    pub digest_algorithm_oid: Vec<u32>,
    /// Message imprint digest bytes.
    pub hashed_message: Vec<u8>,
    /// Optional nonce integer bytes.
    pub nonce: Option<Vec<u8>>,
    /// Whether `certReq` is true.
    pub cert_req: bool,
}

impl TimeStampRequest {
    /// Builds a DER-encoded RFC 3161 `TimeStampReq` for a 32-byte SHA-256 digest.
    #[must_use]
    pub fn encode_sha256(digest: &[u8; 32], nonce: Option<u64>) -> Vec<u8> {
        let message_imprint = encode_sequence(&[
            encode_alg_id(OID_SHA256, AlgParams::Null),
            encode_octet_string(digest),
        ]);
        let mut parts = vec![encode_u64(1), message_imprint];
        if let Some(n) = nonce {
            parts.push(encode_u64(n));
        }
        parts.push(encode_boolean_true()); // certReq = TRUE
        encode_sequence(&parts)
    }

    /// Parses a DER-encoded RFC 3161 `TimeStampReq`.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::DerFormat`] if the request is malformed.
    pub fn from_der(der: &[u8]) -> Result<Self, SigningError> {
        let (root, _) = parse_der_prefix(der)?;
        let fields = root.children()?;
        if fields.len() < 2 {
            return Err(SigningError::DerFormat("TimeStampReq too short".to_owned()));
        }
        let imprint = fields[1].children()?;
        if imprint.len() < 2 {
            return Err(SigningError::DerFormat("invalid MessageImprint".to_owned()));
        }
        let digest_algorithm_oid = imprint[0]
            .children()?
            .first()
            .ok_or_else(|| SigningError::DerFormat("missing imprint hash OID".to_owned()))?
            .as_oid()?;
        let hashed_message = imprint[1].content.to_vec();
        let mut nonce = None;
        let mut cert_req = false;
        for node in &fields[2..] {
            if node.tag == TAG_INTEGER {
                nonce = Some(node.content.to_vec());
            } else if node.tag == 0x01 {
                cert_req = node.content.first() == Some(&0xFF);
            }
        }
        Ok(Self {
            digest_algorithm_oid,
            hashed_message,
            nonce,
            cert_req,
        })
    }
}

/// Verified RFC 3161 `TSTInfo` + CMS signature result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTimestamp {
    /// Policy OID declared in `TSTInfo`.
    pub policy_oid: Vec<u32>,
    /// Timestamp generation time in Unix UTC seconds.
    pub gen_time: i64,
    /// Serial number bytes.
    pub serial: Vec<u8>,
    /// Whether `TSTInfo.messageImprint` equals the expected SHA-256 digest.
    pub imprint_valid: bool,
    /// Whether the TSA CMS signature over `TSTInfo` is valid.
    pub signature_valid: bool,
    /// Chain trust status of the TSA certificate against the [`TrustStore`].
    pub chain_trust: ChainTrust,
    /// TSA certificate common name, if embedded.
    pub tsa_common_name: Option<String>,
    /// Failure description when verification did not succeed cleanly.
    pub failure_reason: Option<String>,
}

/// Local RFC 3161 Time-Stamping Authority backed by an Ed25519 TSA certificate (for deterministic
/// offline signing/testing) plus consent-gated HTTP TSA client.
#[derive(Debug, Clone)]
pub struct LocalTimestampAuthority {
    tsa_cert: X509Certificate,
    chain_certs: Vec<X509Certificate>,
    signing_key: SigningKey,
}

impl LocalTimestampAuthority {
    /// Creates a local RFC 3161 TSA signer.
    #[must_use]
    pub fn new(
        tsa_cert: X509Certificate,
        chain_certs: Vec<X509Certificate>,
        signing_key: SigningKey,
    ) -> Self {
        Self {
            tsa_cert,
            chain_certs,
            signing_key,
        }
    }

    /// Issues a DER-encoded RFC 3161 `TimeStampResp` (`PKIStatus = granted (0)`) for `req_der`.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] if `req_der` is malformed or CMS signing fails.
    pub fn issue_response(
        &self,
        req_der: &[u8],
        gen_time: i64,
        serial: u64,
    ) -> Result<Vec<u8>, SigningError> {
        let token_der = self.issue_token(req_der, gen_time, serial)?;
        let status_info = encode_sequence(&[encode_u64(0)]);
        Ok(encode_sequence(&[status_info, token_der]))
    }

    /// Issues a DER-encoded RFC 3161 `TimeStampToken` (`ContentInfo` wrapping CMS `SignedData`
    /// with `encapContentInfo = TSTInfo`).
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] if `req_der` is malformed or CMS signing fails.
    pub fn issue_token(
        &self,
        req_der: &[u8],
        gen_time: i64,
        serial: u64,
    ) -> Result<Vec<u8>, SigningError> {
        let req = TimeStampRequest::from_der(req_der)?;
        let message_imprint = encode_sequence(&[
            encode_alg_id(&req.digest_algorithm_oid, AlgParams::Null),
            encode_octet_string(&req.hashed_message),
        ]);
        let mut tst_parts = vec![
            encode_u64(1),
            encode_oid(OID_TSA_POLICY_DEFAULT),
            message_imprint,
            encode_u64(serial),
            encode_generalized_time(gen_time),
        ];
        if let Some(ref nonce) = req.nonce {
            tst_parts.push(super::der::encode_tlv(TAG_INTEGER, nonce));
        }
        let tst_info_der = encode_sequence(&tst_parts);
        let tst_digest: [u8; 32] = Sha256::digest(&tst_info_der).into();

        // Build CMS SignedData where encapContentInfo is id-ct-TSTInfo carrying tst_info_der.
        let base_cms = build_cms_signed_data(
            &tst_digest,
            &super::cms::CmsBuildParams {
                signer_cert: &self.tsa_cert,
                chain_certs: &self.chain_certs,
                signing_key: &self.signing_key,
                signing_time: gen_time,
                timestamp_token_der: None,
            },
        )?;
        // Wrap tst_info_der as encapsulated content inside SignedData by re-encoding ContentInfo:
        rewrite_cms_encap_content(&base_cms, OID_TST_INFO, &tst_info_der)
    }
}

/// Sends an RFC 3161 `TimeStampReq` over HTTP (`application/timestamp-query`) when `consent` is
/// [`TsaConsent::Allowed`].
///
/// # Errors
///
/// Returns [`SigningError::ConsentRequired`] if `consent` is [`TsaConsent::Denied`], or
/// [`SigningError::Network`] if the HTTP request fails.
pub fn fetch_tsa_response(
    tsa_url: &str,
    req_der: &[u8],
    consent: TsaConsent,
) -> Result<Vec<u8>, SigningError> {
    if consent != TsaConsent::Allowed {
        return Err(SigningError::ConsentRequired);
    }
    let mut response = ureq::post(tsa_url)
        .header("Content-Type", "application/timestamp-query")
        .send(req_der)
        .map_err(|err| SigningError::Network(format!("TSA request failed: {err}")))?;
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(|err| SigningError::Network(format!("TSA body read failed: {err}")))?;
    Ok(body)
}

/// Parses an RFC 3161 `TimeStampResp`, checks `PKIStatus`, extracts the `TimeStampToken`, and
/// verifies its `messageImprint`, CMS signature, and TSA trust chain.
///
/// # Errors
///
/// Returns [`SigningError::DerFormat`] if the response or token is malformed.
pub fn parse_and_verify_timestamp_response(
    resp_der: &[u8],
    expected_digest_sha256: &[u8; 32],
    trust_store: Option<&TrustStore>,
) -> Result<VerifiedTimestamp, SigningError> {
    let (root, _) = parse_der_prefix(resp_der)?;
    let fields = root.children()?;
    if fields.len() < 2 {
        return Err(SigningError::DerFormat(
            "TimeStampResp missing TimeStampToken".to_owned(),
        ));
    }
    let status_children = fields[0].children()?;
    let status_code = status_children
        .first()
        .and_then(|n| n.as_integer_bytes().ok())
        .and_then(|b| b.last().copied())
        .unwrap_or(255);
    if status_code != 0 && status_code != 1 {
        return Err(SigningError::DerFormat(format!(
            "TSA rejected request with PKIStatus {status_code}"
        )));
    }
    parse_and_verify_timestamp_token(fields[1].full, expected_digest_sha256, trust_store)
}

/// Parses and verifies an RFC 3161 `TimeStampToken` (`ContentInfo`) against `expected_digest_sha256`.
///
/// # Errors
///
/// Returns [`SigningError::DerFormat`] if the token structure is malformed.
pub fn parse_and_verify_timestamp_token(
    token_der: &[u8],
    expected_digest_sha256: &[u8; 32],
    trust_store: Option<&TrustStore>,
) -> Result<VerifiedTimestamp, SigningError> {
    let (root, _) = parse_der_prefix(token_der)?;
    let top = root.children()?;
    if top.len() < 2 {
        return Err(SigningError::DerFormat(
            "TimeStampToken too short".to_owned(),
        ));
    }
    let sd_node = top[1]
        .children()?
        .into_iter()
        .next()
        .ok_or_else(|| SigningError::DerFormat("missing SignedData".to_owned()))?;
    let sd_fields = sd_node.children()?;
    let encap_children = sd_fields
        .get(2)
        .ok_or_else(|| SigningError::DerFormat("missing encapContentInfo".to_owned()))?
        .children()?;
    let econtent_wrapper = encap_children
        .get(1)
        .ok_or_else(|| SigningError::DerFormat("TimeStampToken missing eContent".to_owned()))?;
    let tst_octets = econtent_wrapper
        .children()?
        .into_iter()
        .next()
        .ok_or_else(|| SigningError::DerFormat("missing TSTInfo OCTET STRING".to_owned()))?;
    if tst_octets.tag != TAG_OCTET_STRING {
        return Err(SigningError::DerFormat(
            "TSTInfo must be OCTET STRING".to_owned(),
        ));
    }
    let tst_info_bytes = tst_octets.content;
    let tst_digest: [u8; 32] = Sha256::digest(tst_info_bytes).into();

    let (tst_node, _) = parse_der_prefix(tst_info_bytes)?;
    let tst_fields = tst_node.children()?;
    if tst_fields.len() < 5 {
        return Err(SigningError::DerFormat("truncated TSTInfo".to_owned()));
    }
    let policy_oid = tst_fields[1].as_oid()?;
    let imprint_parts = tst_fields[2].children()?;
    let hashed_msg = imprint_parts
        .get(1)
        .ok_or_else(|| SigningError::DerFormat("TSTInfo missing hashedMessage".to_owned()))?
        .content;
    let serial = tst_fields[3].as_integer_bytes()?.to_vec();
    let gen_time = tst_fields[4].as_unix_time()?;
    let imprint_valid = hashed_msg == expected_digest_sha256;

    let empty_store = TrustStore::new();
    let store = trust_store.unwrap_or(&empty_store);
    let cms_res = verify_cms_detached(token_der, &tst_digest, store, Some(gen_time))?;
    let signature_valid = cms_res.digest_valid && cms_res.signature_valid;
    let tsa_common_name = cms_res
        .signer_cert
        .as_ref()
        .map(|c| c.subject.common_name.clone());
    let failure_reason = if !imprint_valid {
        Some("TSA messageImprint does not match signed digest".to_owned())
    } else {
        cms_res.failure_reason
    };

    Ok(VerifiedTimestamp {
        policy_oid,
        gen_time,
        serial,
        imprint_valid,
        signature_valid,
        chain_trust: cms_res.chain_trust,
        tsa_common_name,
        failure_reason,
    })
}

fn rewrite_cms_encap_content(
    cms_der: &[u8],
    econtent_oid: &[u32],
    econtent_bytes: &[u8],
) -> Result<Vec<u8>, SigningError> {
    let _parsed = parse_cms_signed_data(cms_der)?;
    let (root, _) = parse_der_prefix(cms_der)?;
    let top = root.children()?;
    let sd_node = top[1]
        .children()?
        .into_iter()
        .next()
        .ok_or_else(|| SigningError::DerFormat("missing SignedData".to_owned()))?;
    let mut sd_parts: Vec<Vec<u8>> = sd_node
        .children()?
        .into_iter()
        .map(|n| n.full.to_vec())
        .collect();
    if sd_parts.len() < 4 {
        return Err(SigningError::DerFormat("invalid SignedData".to_owned()));
    }
    sd_parts[2] = encode_sequence(&[
        encode_oid(econtent_oid),
        encode_context_constructed(0, &[encode_octet_string(econtent_bytes)]),
    ]);
    let new_sd = encode_sequence(&sd_parts);
    Ok(encode_sequence(&[
        top[0].full.to_vec(),
        encode_context_constructed(0, &[new_sd]),
    ]))
}

/// Validation-related entry for one signature inside `/DSS /VRI` (ISO 32000-2 §12.8.4.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VriEntry {
    /// DER-encoded certificates specific to this signature.
    pub certs: Vec<Vec<u8>>,
    /// DER-encoded OCSP responses specific to this signature.
    pub ocsps: Vec<Vec<u8>>,
    /// DER-encoded CRLs specific to this signature.
    pub crls: Vec<Vec<u8>>,
}

/// PDF Document Security Store (`/DSS`, ISO 32000-2 §12.8.4.3, plan O.20).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentSecurityStore {
    /// Document-wide validation certificates (`/Certs`).
    pub certs: Vec<Vec<u8>>,
    /// Document-wide OCSP responses (`/OCSPs`).
    pub ocsps: Vec<Vec<u8>>,
    /// Document-wide CRLs (`/CRLs`).
    pub crls: Vec<Vec<u8>>,
    /// Per-signature validation-related info keyed by uppercase hex SHA-1 of `/Contents`.
    pub vri: BTreeMap<String, VriEntry>,
}

/// Embeds `dss` into `pdf_bytes` as an incremental update attaching `/DSS` to the PDF catalog
/// (plan O.20, PAdES-B-LT).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed or updated.
pub fn embed_dss(pdf_bytes: &[u8], dss: &DocumentSecurityStore) -> Result<Vec<u8>, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let (root_id, root_gen, mut catalog) = index.catalog()?;
    let mut writer = IncrementalPdfWriter::from_index(&index)?;

    let mut dss_dict = String::from("<< /Type /DSS");

    if !dss.certs.is_empty() {
        let mut refs = String::new();
        for cert in &dss.certs {
            let id = writer.alloc_id();
            writer.write_stream_object(id, 0, "", cert);
            let _ = write!(refs, " {id} 0 R");
        }
        let _ = write!(dss_dict, " /Certs [{refs} ]");
    }
    if !dss.ocsps.is_empty() {
        let mut refs = String::new();
        for ocsp in &dss.ocsps {
            let id = writer.alloc_id();
            writer.write_stream_object(id, 0, "", ocsp);
            let _ = write!(refs, " {id} 0 R");
        }
        let _ = write!(dss_dict, " /OCSPs [{refs} ]");
    }
    if !dss.crls.is_empty() {
        let mut refs = String::new();
        for crl in &dss.crls {
            let id = writer.alloc_id();
            writer.write_stream_object(id, 0, "", crl);
            let _ = write!(refs, " {id} 0 R");
        }
        let _ = write!(dss_dict, " /CRLs [{refs} ]");
    }

    if !dss.vri.is_empty() {
        let mut vri_dict = String::from("<<");
        for (key, entry) in &dss.vri {
            let mut entry_str = String::from("<<");
            if !entry.certs.is_empty() {
                let mut refs = String::new();
                for c in &entry.certs {
                    let id = writer.alloc_id();
                    writer.write_stream_object(id, 0, "", c);
                    let _ = write!(refs, " {id} 0 R");
                }
                let _ = write!(entry_str, " /Cert [{refs} ]");
            }
            if !entry.ocsps.is_empty() {
                let mut refs = String::new();
                for o in &entry.ocsps {
                    let id = writer.alloc_id();
                    writer.write_stream_object(id, 0, "", o);
                    let _ = write!(refs, " {id} 0 R");
                }
                let _ = write!(entry_str, " /OCSP [{refs} ]");
            }
            if !entry.crls.is_empty() {
                let mut refs = String::new();
                for crl in &entry.crls {
                    let id = writer.alloc_id();
                    writer.write_stream_object(id, 0, "", crl);
                    let _ = write!(refs, " {id} 0 R");
                }
                let _ = write!(entry_str, " /CRL [{refs} ]");
            }
            entry_str.push_str(" >>");
            let entry_id = writer.alloc_id();
            writer.write_object(entry_id, 0, &entry_str);
            let _ = write!(vri_dict, " /{key} {entry_id} 0 R");
        }
        vri_dict.push_str(" >>");
        let vri_id = writer.alloc_id();
        writer.write_object(vri_id, 0, &vri_dict);
        let _ = write!(dss_dict, " /VRI {vri_id} 0 R");
    }

    dss_dict.push_str(" >>");
    let dss_id = writer.alloc_id();
    writer.write_object(dss_id, 0, &dss_dict);
    catalog.insert("DSS".to_owned(), PdfVal::Ref(dss_id, 0));
    writer.write_object(root_id, root_gen, &format_pdf_val(&PdfVal::Dict(catalog)));

    Ok(writer.finish())
}

/// Extracts the `/DSS` dictionary from `pdf_bytes`, if present.
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` or its `/DSS` objects cannot be parsed.
pub fn extract_dss(pdf_bytes: &[u8]) -> Result<Option<DocumentSecurityStore>, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let (_, _, catalog) = index.catalog()?;
    let Some(dss_val) = catalog.get("DSS") else {
        return Ok(None);
    };
    let mut owned_dss = None;
    let dss_dict = index
        .resolve(dss_val, &mut owned_dss)?
        .as_dict()
        .ok_or_else(|| SigningError::PdfStructure("/DSS is not a dictionary".to_owned()))?;

    let certs = read_stream_array(&index, dss_dict.get("Certs"))?;
    let ocsps = read_stream_array(&index, dss_dict.get("OCSPs"))?;
    let crls = read_stream_array(&index, dss_dict.get("CRLs"))?;

    let mut vri = BTreeMap::new();
    if let Some(vri_val) = dss_dict.get("VRI") {
        let mut owned_vri = None;
        if let Some(vri_dict) = index.resolve(vri_val, &mut owned_vri)?.as_dict() {
            for (k, v) in vri_dict {
                let mut owned_entry = None;
                if let Some(edict) = index.resolve(v, &mut owned_entry)?.as_dict() {
                    vri.insert(
                        k.clone(),
                        VriEntry {
                            certs: read_stream_array(&index, edict.get("Cert"))?,
                            ocsps: read_stream_array(&index, edict.get("OCSP"))?,
                            crls: read_stream_array(&index, edict.get("CRL"))?,
                        },
                    );
                }
            }
        }
    }

    Ok(Some(DocumentSecurityStore {
        certs,
        ocsps,
        crls,
        vri,
    }))
}

fn read_stream_array(
    index: &PdfIndex<'_>,
    val: Option<&PdfVal>,
) -> Result<Vec<Vec<u8>>, SigningError> {
    let Some(arr_val) = val else {
        return Ok(Vec::new());
    };
    let mut owned = None;
    let Some(arr) = index.resolve(arr_val, &mut owned)?.as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some((oid, _)) = item.as_ref() {
            let (_, bytes) = index.read_stream(oid)?;
            out.push(bytes);
        }
    }
    Ok(out)
}

/// Appends an RFC 3161 `/DocTimeStamp` signature dictionary covering all bytes of `pdf_bytes`
/// (including any embedded `/DSS`), upgrading a PAdES-B-LT PDF to PAdES-B-LTA (plan O.20).
///
/// # Errors
///
/// Returns [`SigningError`] if `pdf_bytes` cannot be parsed or the timestamp callback fails.
pub fn append_document_timestamp(
    pdf_bytes: &[u8],
    field_name: &str,
    issue_token: impl FnOnce(&[u8]) -> Result<Vec<u8>, SigningError>,
) -> Result<Vec<u8>, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let pages = index.collect_pages()?;
    let &(page_obj_id, page_gen) = pages
        .first()
        .ok_or_else(|| SigningError::PdfStructure("PDF has no pages".to_owned()))?;
    let (root_id, root_gen, mut catalog) = index.catalog()?;
    let mut page_dict = index
        .read_value(page_obj_id)?
        .as_dict()
        .cloned()
        .ok_or_else(|| SigningError::PdfStructure("page 0 missing dict".to_owned()))?;

    let mut writer = IncrementalPdfWriter::from_index(&index)?;
    let sig_id = writer.alloc_id();
    let widget_id = writer.alloc_id();

    let sig_dict = format!(
        "<< /Type /DocTimeStamp /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange {} /Contents <{}> >>",
        byte_range_placeholder(),
        "0".repeat(signature_hex_capacity())
    );
    let sig_offset = writer.write_object(sig_id, 0, &sig_dict);

    let widget_body = format!(
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T {} /Rect [0 0 0 0] /F 132 /P {} {} R /V {} 0 R >>",
        crate::forms::pdf_codec::encode_pdf_literal(field_name),
        page_obj_id,
        page_gen,
        sig_id
    );
    writer.write_object(widget_id, 0, &widget_body);

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
    patch_byte_range_and_contents(&mut out, sig_offset, |digest| {
        let req = TimeStampRequest::encode_sha256(&digest, Some(1));
        issue_token(&req)
    })?;
    Ok(out)
}

/// Summary of PAdES B-B / B-T / B-LT / B-LTA conformance for a PDF (plan O.20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadesConformanceReport {
    /// Highest achieved PAdES profile level across the document's signatures.
    pub achieved_level: PadesLevel,
    /// Whether the document contains a `/DSS` dictionary.
    pub has_dss: bool,
    /// Number of embedded certificates in `/DSS`.
    pub dss_cert_count: usize,
    /// Number of embedded OCSP responses in `/DSS`.
    pub dss_ocsp_count: usize,
    /// Number of embedded CRLs in `/DSS`.
    pub dss_crl_count: usize,
    /// Number of `/DocTimeStamp` objects covering the PDF.
    pub doc_timestamp_count: usize,
    /// Gaps preventing a higher PAdES level (empty when `LTA`).
    pub missing_for_next_level: Vec<String>,
}

/// Evaluates PAdES B-B / B-T / B-LT / B-LTA conformance on `pdf_bytes` against `trust_store` (plan O.20).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed.
pub fn check_pades_conformance(
    pdf_bytes: &[u8],
    trust_store: &TrustStore,
) -> Result<PadesConformanceReport, SigningError> {
    let sigs = verify_pdf_signatures(pdf_bytes, trust_store)?;
    let dss = extract_dss(pdf_bytes)?;
    let has_dss = dss.is_some();
    let (dss_cert_count, dss_ocsp_count, dss_crl_count) = dss
        .as_ref()
        .map_or((0, 0, 0), |d| (d.certs.len(), d.ocsps.len(), d.crls.len()));

    let doc_sigs: Vec<_> = sigs
        .iter()
        .filter(|s| !s.signer.is_document_timestamp)
        .collect();
    let doc_timestamps: Vec<_> = sigs
        .iter()
        .filter(|s| s.signer.is_document_timestamp)
        .collect();

    let mut missing = Vec::new();
    let any_intact_bb = doc_sigs.iter().any(|s| {
        matches!(
            s.trust_status,
            TrustStatus::Trusted | TrustStatus::UnknownIssuer
        ) && s.has_signing_cert_v2
    });
    if !any_intact_bb {
        missing.push("valid PAdES-B-B signature with signing-certificate-v2".to_owned());
        return Ok(PadesConformanceReport {
            achieved_level: PadesLevel::None,
            has_dss,
            dss_cert_count,
            dss_ocsp_count,
            dss_crl_count,
            doc_timestamp_count: doc_timestamps.len(),
            missing_for_next_level: missing,
        });
    }

    let has_bt = doc_sigs.iter().any(|s| {
        matches!(
            s.trust_status,
            TrustStatus::Trusted | TrustStatus::UnknownIssuer
        ) && (s.has_timestamp || !doc_timestamps.is_empty())
    });
    if !has_bt {
        missing.push("RFC 3161 signature or document timestamp (B-T)".to_owned());
        return Ok(PadesConformanceReport {
            achieved_level: PadesLevel::B,
            has_dss,
            dss_cert_count,
            dss_ocsp_count,
            dss_crl_count,
            doc_timestamp_count: doc_timestamps.len(),
            missing_for_next_level: missing,
        });
    }

    let dss_populated = dss.as_ref().is_some_and(|d| {
        !d.certs.is_empty() && (!d.ocsps.is_empty() || !d.crls.is_empty() || !d.vri.is_empty())
    });
    if !dss_populated {
        missing.push("/DSS dictionary with /Certs and revocation or /VRI data (B-LT)".to_owned());
        return Ok(PadesConformanceReport {
            achieved_level: PadesLevel::T,
            has_dss,
            dss_cert_count,
            dss_ocsp_count,
            dss_crl_count,
            doc_timestamp_count: doc_timestamps.len(),
            missing_for_next_level: missing,
        });
    }

    let has_valid_doc_ts = doc_timestamps.iter().any(|ts| {
        matches!(
            ts.trust_status,
            TrustStatus::Trusted | TrustStatus::UnknownIssuer
        ) && !ts.modified_after_signing
    });
    if !has_valid_doc_ts {
        missing.push("uncovered /DocTimeStamp sealing /DSS (B-LTA)".to_owned());
        return Ok(PadesConformanceReport {
            achieved_level: PadesLevel::LT,
            has_dss,
            dss_cert_count,
            dss_ocsp_count,
            dss_crl_count,
            doc_timestamp_count: doc_timestamps.len(),
            missing_for_next_level: missing,
        });
    }

    Ok(PadesConformanceReport {
        achieved_level: PadesLevel::LTA,
        has_dss,
        dss_cert_count,
        dss_ocsp_count,
        dss_crl_count,
        doc_timestamp_count: doc_timestamps.len(),
        missing_for_next_level: missing,
    })
}
