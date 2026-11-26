//! Signature inspector panel model, "View signed version" revision extractor, DocMDP edit
//! guard, and end-to-end PAdES signing workflow (plan O.14, O.15, O.16, O.20).

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use excellent_core::signing::{
    DocMdpPermission, DocumentEditKind, DocumentSecurityStore, EditSafetyVerdict,
    LocalTimestampAuthority, PadesConformanceReport, PadesLevel, SignDocumentRequest,
    SignatureAppearance, SignatureVerification, SigningError, TimeStampRequest, TrustStatus,
    TrustStore, VriEntry, append_document_timestamp, check_pades_conformance, embed_dss,
    fetch_tsa_response, inspect_edit_safety, sign_pdf_bytes, verify_pdf_signatures,
};

/// Consent setting for contacting a remote RFC 3161 TSA server over HTTPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnlineTimestampConsent {
    /// Offline only: do not make network requests.
    #[default]
    Denied,
    /// User explicitly consented to sending the digest to the configured TSA URL.
    Allowed,
}

/// Optional timestamping configuration for [`SigningWorkflow`].
#[derive(Clone)]
pub enum TimestampConfig {
    /// No timestamp (PAdES-B-B).
    None,
    /// Local offline TSA signer (deterministic tests and local timestamping).
    Local {
        /// Local TSA authority.
        authority: Box<LocalTimestampAuthority>,
        /// Generation timestamp (Unix UTC seconds).
        gen_time: i64,
        /// Monotonic serial number.
        serial: u64,
    },
    /// Remote RFC 3161 TSA endpoint over HTTPS, gated by user consent.
    Remote {
        /// HTTP(S) URL of the TSA endpoint.
        url: String,
        /// Explicit user consent.
        consent: OnlineTimestampConsent,
    },
}

/// Revocation evidence embedded into `/DSS` for PAdES B-LT / B-LTA upgrades.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LtvRevocationEvidence {
    /// DER-encoded OCSP responses.
    pub ocsps: Vec<Vec<u8>>,
    /// DER-encoded Certificate Revocation Lists.
    pub crls: Vec<Vec<u8>>,
}

/// UI-ready summary row for a digital signature or `/DocTimeStamp` in the signature panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureSummary {
    /// Form field name.
    pub field_name: String,
    /// Signer display name.
    pub signer_name: String,
    /// Issuer display name.
    pub issuer_name: String,
    /// Certificate serial hex.
    pub serial_hex: String,
    /// Signing time (Unix UTC seconds), if available.
    pub signing_time: Option<i64>,
    /// Reason string.
    pub reason: Option<String>,
    /// Location string.
    pub location: Option<String>,
    /// Cryptographic and trust status.
    pub trust_status: TrustStatus,
    /// DocMDP permission level if certified.
    pub doc_mdp: Option<DocMdpPermission>,
    /// Whether edits occurred after this signature's revision.
    pub modified_after_signing: bool,
    /// Whether a verified timestamp is attached.
    pub has_timestamp: bool,
    /// Timestamp generation time (Unix UTC seconds), if verified.
    pub timestamp_gen_time: Option<i64>,
    /// Byte length of the PDF revision covered by this signature ("View signed version").
    pub covered_revision_len: usize,
    /// Optional diagnostic failure reason.
    pub failure_reason: Option<String>,
}

impl From<&SignatureVerification> for SignatureSummary {
    fn from(v: &SignatureVerification) -> Self {
        Self {
            field_name: v.field_name.clone(),
            signer_name: v.signer.common_name.clone(),
            issuer_name: v.signer.issuer_common_name.clone(),
            serial_hex: v.signer.serial_hex.clone(),
            signing_time: v.signer.signing_time,
            reason: v.signer.reason.clone(),
            location: v.signer.location.clone(),
            trust_status: v.trust_status,
            doc_mdp: v.doc_mdp,
            modified_after_signing: v.modified_after_signing,
            has_timestamp: v.has_timestamp,
            timestamp_gen_time: v.timestamp_gen_time,
            covered_revision_len: v.covered_revision_end,
            failure_reason: v.failure_reason.clone(),
        }
    }
}

/// Complete signature panel inspection state for an open PDF (plan O.15, O.20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignaturePanelModel {
    /// Per-signature verification summaries in revision order.
    pub signatures: Vec<SignatureSummary>,
    /// Overall PAdES LTV conformance assessment.
    pub pades: PadesConformanceReport,
}

impl SignaturePanelModel {
    /// Inspects `pdf_bytes` against `trust_store` and constructs a [`SignaturePanelModel`].
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed as a PDF.
    pub fn inspect(pdf_bytes: &[u8], trust_store: &TrustStore) -> Result<Self, SigningError> {
        let verified = verify_pdf_signatures(pdf_bytes, trust_store)?;
        let pades = check_pades_conformance(pdf_bytes, trust_store)?;
        let signatures = verified.iter().map(SignatureSummary::from).collect();
        Ok(Self { signatures, pades })
    }

    /// Achieved PAdES conformance level.
    #[must_use]
    pub const fn pades_level(&self) -> PadesLevel {
        self.pades.achieved_level
    }
}

/// Extracts the exact PDF revision bytes covered by signature `signature_index` so the reader can
/// open "View signed version" (plan O.15).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `signature_index` is out of bounds or the signature's
/// `/ByteRange` does not delimit a readable revision.
pub fn extract_signed_revision(
    pdf_bytes: &[u8],
    signature_index: usize,
) -> Result<Vec<u8>, SigningError> {
    let sigs = verify_pdf_signatures(pdf_bytes, &TrustStore::new())?;
    let sig = sigs.get(signature_index).ok_or_else(|| {
        SigningError::PdfStructure(format!("signature index {signature_index} out of range"))
    })?;
    if sig.covered_revision_end == 0 || sig.covered_revision_end > pdf_bytes.len() {
        return Err(SigningError::PdfStructure(
            "signature covered revision end is out of bounds".to_owned(),
        ));
    }
    let mut rev = pdf_bytes[..sig.covered_revision_end].to_vec();
    // Blank out the signature's own /Contents hex payload? No — in PDF, the signed revision already
    // includes the /Contents placeholder/value at that revision; returning the exact revision prefix
    // ending at %%EOF opens identically in MuPDF and Acrobat.
    if !rev.ends_with(b"\n") {
        rev.push(b'\n');
    }
    Ok(rev)
}

/// UI action decision returned before applying an edit to a potentially signed PDF (plan O.16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditGuardDecision {
    /// Document is unsigned or edit is explicitly allowed by active DocMDP / FieldMDP policies.
    ProceedSilently,
    /// Edit would invalidate one or more existing signatures; warn the user first.
    WarnUser {
        /// Names of affected signatures.
        affected_signatures: Vec<String>,
        /// Human-readable reason.
        reason: String,
    },
}

/// Evaluates whether `edit` can be applied to `pdf_bytes` without invalidating existing signatures (plan O.16).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed.
pub fn evaluate_edit_guard(
    pdf_bytes: &[u8],
    edit: &DocumentEditKind,
) -> Result<EditGuardDecision, SigningError> {
    let report = inspect_edit_safety(pdf_bytes, edit)?;
    match report.verdict {
        EditSafetyVerdict::Allowed => Ok(EditGuardDecision::ProceedSilently),
        EditSafetyVerdict::WouldInvalidate {
            affected_signatures,
            reason,
        } => Ok(EditGuardDecision::WarnUser {
            affected_signatures,
            reason,
        }),
    }
}

/// High-level workflow for signing a document and optionally upgrading it to PAdES B-T / B-LT / B-LTA
/// (plan O.14, O.20).
pub struct SigningWorkflow<'a> {
    /// Signer certificate.
    pub signer_cert: &'a excellent_core::signing::X509Certificate,
    /// Intermediate / root chain certificates.
    pub chain_certs: &'a [excellent_core::signing::X509Certificate],
    /// Private Ed25519 signing key.
    pub signing_key: &'a SigningKey,
    /// Signature field name.
    pub field_name: String,
    /// Optional reason.
    pub reason: Option<String>,
    /// Optional location.
    pub location: Option<String>,
    /// Signing time (Unix UTC seconds).
    pub signing_time: i64,
    /// Visual or invisible signature appearance.
    pub appearance: SignatureAppearance,
    /// Optional DocMDP certification permission.
    pub certification: Option<DocMdpPermission>,
    /// Timestamping configuration.
    pub timestamp: TimestampConfig,
    /// Optional LTV revocation evidence (OCSP responses / CRLs) for B-LT / B-LTA upgrade.
    pub ltv_revocation: Option<LtvRevocationEvidence>,
}

impl SigningWorkflow<'_> {
    /// Executes the signing workflow on `pdf_bytes` and returns the signed PDF bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] if signing, timestamping, or DSS embedding fails.
    pub fn execute(&self, pdf_bytes: &[u8]) -> Result<Vec<u8>, SigningError> {
        let mut signed = sign_pdf_bytes(
            pdf_bytes,
            &SignDocumentRequest {
                signer_cert: self.signer_cert,
                chain_certs: self.chain_certs,
                signing_key: self.signing_key,
                field_name: self.field_name.clone(),
                reason: self.reason.clone(),
                location: self.location.clone(),
                signing_time: self.signing_time,
                appearance: self.appearance.clone(),
                certification: self.certification,
                timestamp_token_der: None,
            },
        )?;

        match &self.timestamp {
            TimestampConfig::None => return Ok(signed),
            TimestampConfig::Local {
                authority,
                gen_time,
                serial,
            } => {
                signed = append_document_timestamp(
                    &signed,
                    &format!("{}_TS", self.field_name),
                    |req_der| authority.issue_token(req_der, *gen_time, *serial),
                )?;
            }
            TimestampConfig::Remote { url, consent } => {
                let core_consent = match consent {
                    OnlineTimestampConsent::Denied => excellent_core::signing::TsaConsent::Denied,
                    OnlineTimestampConsent::Allowed => excellent_core::signing::TsaConsent::Allowed,
                };
                if core_consent == excellent_core::signing::TsaConsent::Denied {
                    return Err(SigningError::ConsentRequired);
                }
                signed = append_document_timestamp(
                    &signed,
                    &format!("{}_TS", self.field_name),
                    |req_der| {
                        let resp = fetch_tsa_response(url, req_der, core_consent)?;
                        // Return the raw TimeStampResp token extracted or validated; remote TSA returns TimeStampResp.
                        let _ = TimeStampRequest::from_der(req_der)?;
                        Ok(resp)
                    },
                )?;
            }
        }

        if let Some(evidence) = &self.ltv_revocation {
            let sigs = verify_pdf_signatures(&signed, &TrustStore::new())?;
            let mut certs = vec![self.signer_cert.raw_der.clone()];
            for c in self.chain_certs {
                certs.push(c.raw_der.clone());
            }
            let mut vri = BTreeMap::new();
            if let Some(first) = sigs.first() {
                vri.insert(
                    first.vri_key.clone(),
                    VriEntry {
                        certs: certs.clone(),
                        ocsps: evidence.ocsps.clone(),
                        crls: evidence.crls.clone(),
                    },
                );
            }
            signed = embed_dss(
                &signed,
                &DocumentSecurityStore {
                    certs,
                    ocsps: evidence.ocsps.clone(),
                    crls: evidence.crls.clone(),
                    vri,
                },
            )?;

            if let TimestampConfig::Local {
                authority,
                gen_time,
                serial,
            } = &self.timestamp
            {
                signed = append_document_timestamp(
                    &signed,
                    &format!("{}_LTA_TS", self.field_name),
                    |req_der| authority.issue_token(req_der, *gen_time + 1, *serial + 1),
                )?;
            }
        }

        Ok(signed)
    }
}
