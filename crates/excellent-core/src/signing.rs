//! PDF digital signing, signature verification, DocMDP signature-safe editing, and PAdES LTV
//! (plan O.14, O.15, O.16, O.20).

mod cms;
mod der;
mod ltv;
mod mdp;
mod pdf_sig;

pub use der::{
    CertRole, CertValidity, ChainTrust, DistinguishedName, SigningError, TrustStore,
    X509Certificate,
};
pub use ltv::{
    DocumentSecurityStore, LocalTimestampAuthority, PadesConformanceReport, PadesLevel,
    TimeStampRequest, TsaConsent, VerifiedTimestamp, VriEntry, append_document_timestamp,
    check_pades_conformance, embed_dss, extract_dss, fetch_tsa_response,
    parse_and_verify_timestamp_response, parse_and_verify_timestamp_token,
};
pub use mdp::{
    DocMdpPermission, DocumentEditKind, EditSafetyReport, EditSafetyVerdict, FieldMdpRule,
    enforce_incremental_save, inspect_edit_safety,
};
pub use pdf_sig::{
    SignDocumentRequest, SignatureAppearance, SignatureVerification, SignerDetails, TrustStatus,
    VisibleAppearance, VisibleSignatureContent, sign_pdf_bytes, verify_pdf_signatures,
};

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use excellent_mupdf::{Document, PagePoint, PageRect};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::forms::{AcroForm, FieldValue, FormField};

    fn minimal_pdf() -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");
        let mut push_obj = |id: u32, body: &str| {
            offsets.push((id, out.len()));
            out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
        };
        push_obj(1, "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>");
        push_obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        push_obj(
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] >>",
        );
        push_obj(4, "<< /Fields [5 0 R] >>");
        push_obj(
            5,
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (notes) /Rect [50 700 200 724] /DA (/Helv 12 Tf 0 g) /V (initial) /P 3 0 R >>",
        );
        let xref_off = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for (_, off) in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_off}\n%%EOF\n")
                .as_bytes(),
        );
        out
    }

    fn key_from_seed(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn test_pki() -> (
        X509Certificate,
        SigningKey,
        X509Certificate,
        SigningKey,
        X509Certificate,
        SigningKey,
    ) {
        let ca_key = key_from_seed(1);
        let ca_dn = DistinguishedName {
            common_name: "ExcellentPDF Root CA".to_owned(),
            organization: Some("ExcellentPDF".to_owned()),
            country: Some("US".to_owned()),
        };
        let ca_cert = X509Certificate::issue_ed25519(
            &ca_dn,
            &ca_key.verifying_key(),
            None,
            &ca_key,
            CertValidity::default(),
            CertRole::CertificateAuthority,
            1,
        )
        .expect("root ca cert");

        let signer_key = key_from_seed(2);
        let signer_dn = DistinguishedName {
            common_name: "Ada Lovelace".to_owned(),
            organization: Some("Analytical Engine Lab".to_owned()),
            country: Some("GB".to_owned()),
        };
        let signer_cert = X509Certificate::issue_ed25519(
            &signer_dn,
            &signer_key.verifying_key(),
            Some(&ca_cert),
            &ca_key,
            CertValidity::default(),
            CertRole::EndEntity,
            42,
        )
        .expect("signer cert");

        let tsa_key = key_from_seed(3);
        let tsa_dn = DistinguishedName::with_common_name("ExcellentPDF TSA");
        let tsa_cert = X509Certificate::issue_ed25519(
            &tsa_dn,
            &tsa_key.verifying_key(),
            Some(&ca_cert),
            &ca_key,
            CertValidity::default(),
            CertRole::TimestampAuthority,
            99,
        )
        .expect("tsa cert");

        (ca_cert, ca_key, signer_cert, signer_key, tsa_cert, tsa_key)
    }

    #[test]
    fn signs_verifies_trust_and_detects_tampering() {
        let (ca_cert, _, signer_cert, signer_key, _, _) = test_pki();
        let pdf = minimal_pdf();

        let signed = sign_pdf_bytes(
            &pdf,
            &SignDocumentRequest {
                signer_cert: &signer_cert,
                chain_certs: std::slice::from_ref(&ca_cert),
                signing_key: &signer_key,
                field_name: "ApprovalSig".to_owned(),
                reason: Some("Approved for release".to_owned()),
                location: Some("London, UK".to_owned()),
                signing_time: 1_760_000_000,
                appearance: SignatureAppearance::Visible(VisibleAppearance {
                    page: 0,
                    rect: PageRect {
                        x0: 72.0,
                        y0: 600.0,
                        x1: 272.0,
                        y1: 660.0,
                    },
                    content: VisibleSignatureContent::Handwritten {
                        strokes: vec![vec![
                            PagePoint { x: 0.1, y: 0.7 },
                            PagePoint { x: 0.4, y: 0.3 },
                            PagePoint { x: 0.85, y: 0.65 },
                        ]],
                        stroke_width: 2.0,
                    },
                    caption: Some("Signed by Ada Lovelace".to_owned()),
                }),
                certification: Some(DocMdpPermission::FormFillAndSign),
                timestamp_token_der: None,
            },
        )
        .expect("sign pdf");

        // Signed PDF must still open cleanly in MuPDF.
        let doc = Document::from_bytes(&signed, "application/pdf").expect("mupdf opens signed pdf");
        assert_eq!(doc.page_count().expect("count"), 1);

        // Trusted when root CA is in TrustStore.
        let trusted_store = TrustStore::with_anchors([ca_cert.clone()]);
        let verified = verify_pdf_signatures(&signed, &trusted_store).expect("verify trusted");
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].trust_status, TrustStatus::Trusted);
        assert_eq!(verified[0].signer.common_name, "Ada Lovelace");
        assert_eq!(
            verified[0].signer.issuer_common_name,
            "ExcellentPDF Root CA"
        );
        assert_eq!(verified[0].doc_mdp, Some(DocMdpPermission::FormFillAndSign));
        assert!(!verified[0].modified_after_signing);

        // UnknownIssuer when trust store is empty.
        let empty_store = TrustStore::new();
        let unknown = verify_pdf_signatures(&signed, &empty_store).expect("verify unknown");
        assert_eq!(unknown[0].trust_status, TrustStatus::UnknownIssuer);

        // Tamper one byte inside the signed revision -> Tampered!
        let mut tampered = signed.clone();
        let pos = tampered
            .windows(7)
            .position(|w| w == b"initial")
            .expect("find initial");
        tampered[pos] = b'X';
        let after_tamper =
            verify_pdf_signatures(&tampered, &trusted_store).expect("verify tampered");
        assert_eq!(after_tamper[0].trust_status, TrustStatus::Tampered);

        // Corrupt the %%EOF marker inside the covered revision -> ByteRangeIncomplete!
        let mut bad_eof = signed;
        let end = verified[0].covered_revision_end;
        bad_eof[end - 3] = b'X';
        let bad_res = verify_pdf_signatures(&bad_eof, &trusted_store).expect("verify bad eof");
        assert_eq!(bad_res[0].trust_status, TrustStatus::ByteRangeIncomplete);
    }

    #[test]
    fn doc_mdp_permits_or_blocks_edits_and_enforces_incremental_save() {
        let (ca_cert, _, signer_cert, signer_key, _, _) = test_pki();
        let pdf = minimal_pdf();

        let certified = sign_pdf_bytes(
            &pdf,
            &SignDocumentRequest {
                signer_cert: &signer_cert,
                chain_certs: std::slice::from_ref(&ca_cert),
                signing_key: &signer_key,
                field_name: "CertSig".to_owned(),
                reason: None,
                location: None,
                signing_time: 1_760_000_000,
                appearance: SignatureAppearance::Invisible,
                certification: Some(DocMdpPermission::FormFillAndSign),
                timestamp_token_der: None,
            },
        )
        .expect("certify");

        let form_ok = inspect_edit_safety(
            &certified,
            &DocumentEditKind::FormFill {
                field_name: Some("notes".to_owned()),
            },
        )
        .expect("check form fill");
        assert_eq!(form_ok.verdict, EditSafetyVerdict::Allowed);
        assert!(form_ok.requires_incremental_save);

        let annot_blocked =
            inspect_edit_safety(&certified, &DocumentEditKind::Annotation).expect("check annot");
        assert!(matches!(
            annot_blocked.verdict,
            EditSafetyVerdict::WouldInvalidate { .. }
        ));

        let content_blocked =
            inspect_edit_safety(&certified, &DocumentEditKind::ContentEdit).expect("check content");
        assert!(matches!(
            content_blocked.verdict,
            EditSafetyVerdict::WouldInvalidate { .. }
        ));

        // Filling a form incrementally keeps the certification signature cryptographically valid!
        let mut form = AcroForm::from_pdf_bytes(&certified).expect("parse form");
        form.set_value("notes", FieldValue::Text("filled safely".into()))
            .expect("fill notes");
        let updated = form.to_pdf_bytes(&certified).expect("incremental save");
        let safe = enforce_incremental_save(&certified, &updated).expect("incremental invariant");

        let store = TrustStore::with_anchors([ca_cert]);
        let sigs = verify_pdf_signatures(&safe, &store).expect("verify after form fill");
        assert_eq!(sigs[0].trust_status, TrustStatus::Trusted);
        assert!(sigs[0].modified_after_signing);

        // Non-incremental rewrite is rejected.
        let mut rewritten = safe;
        rewritten[0] = b' ';
        assert!(enforce_incremental_save(&certified, &rewritten).is_err());
        let _ = FormField::text("unused", "");
    }

    #[test]
    fn pades_ltv_upgrades_from_bb_to_bt_blt_and_blta() {
        let (ca_cert, _, signer_cert, signer_key, tsa_cert, tsa_key) = test_pki();
        let store = TrustStore::with_anchors([ca_cert.clone()]);
        let tsa = LocalTimestampAuthority::new(tsa_cert.clone(), vec![ca_cert.clone()], tsa_key);
        let pdf = minimal_pdf();

        // 1. PAdES-B-B
        let bb = sign_pdf_bytes(
            &pdf,
            &SignDocumentRequest {
                signer_cert: &signer_cert,
                chain_certs: std::slice::from_ref(&ca_cert),
                signing_key: &signer_key,
                field_name: "SigBB".to_owned(),
                reason: Some("LTV test".to_owned()),
                location: None,
                signing_time: 1_760_000_000,
                appearance: SignatureAppearance::Invisible,
                certification: None,
                timestamp_token_der: None,
            },
        )
        .expect("sign B-B");
        let rep_bb = check_pades_conformance(&bb, &store).expect("conformance B-B");
        assert_eq!(rep_bb.achieved_level, PadesLevel::B);

        // 2. PAdES-B-T via RFC 3161 TSA token over a digest
        let sample_digest: [u8; 32] = Sha256::digest(b"pades-bt-test").into();
        let tsa_req = TimeStampRequest::encode_sha256(&sample_digest, Some(77));
        let tsa_resp = tsa
            .issue_response(&tsa_req, 1_760_000_100, 501)
            .expect("tsa resp");
        let verified_ts =
            parse_and_verify_timestamp_response(&tsa_resp, &sample_digest, Some(&store))
                .expect("verify tsa resp");
        assert!(verified_ts.imprint_valid && verified_ts.signature_valid);
        assert_eq!(verified_ts.gen_time, 1_760_000_100);

        // Embed a B-T signature with a Document Timestamp or TSA token -> B-LT -> B-LTA.
        let bt = append_document_timestamp(&bb, "SigTS", |req_der| {
            tsa.issue_token(req_der, 1_760_000_200, 502)
        })
        .expect("append timestamp");
        let rep_bt = check_pades_conformance(&bt, &store).expect("conformance B-T");
        assert_eq!(rep_bt.achieved_level, PadesLevel::T);

        // 3. PAdES-B-LT: embed /DSS with /Certs, /OCSPs, /CRLs, /VRI
        let sigs = verify_pdf_signatures(&bt, &store).expect("sigs");
        let mut dss = DocumentSecurityStore {
            certs: vec![
                signer_cert.raw_der.clone(),
                tsa_cert.raw_der.clone(),
                ca_cert.raw_der.clone(),
            ],
            ocsps: vec![b"DER-OCSP-RESPONSE-STUB".to_vec()],
            crls: vec![b"DER-CRL-STUB".to_vec()],
            vri: Default::default(),
        };
        dss.vri.insert(
            sigs[0].vri_key.clone(),
            VriEntry {
                certs: vec![signer_cert.raw_der],
                ocsps: vec![b"DER-OCSP-RESPONSE-STUB".to_vec()],
                crls: Vec::new(),
            },
        );
        let blt = embed_dss(&bt, &dss).expect("embed dss");
        let extracted = extract_dss(&blt)
            .expect("extract dss")
            .expect("dss present");
        assert_eq!(extracted.certs.len(), 3);
        assert_eq!(extracted.ocsps.len(), 1);
        assert_eq!(extracted.crls.len(), 1);
        assert!(extracted.vri.contains_key(&sigs[0].vri_key));

        let rep_blt = check_pades_conformance(&blt, &store).expect("conformance B-LT");
        assert_eq!(rep_blt.achieved_level, PadesLevel::LT);

        // 4. PAdES-B-LTA: append final /DocTimeStamp sealing /DSS
        let blta = append_document_timestamp(&blt, "LtvDocTimeStamp", |req_der| {
            tsa.issue_token(req_der, 1_760_000_300, 503)
        })
        .expect("append LTA timestamp");
        let rep_blta = check_pades_conformance(&blta, &store).expect("conformance B-LTA");
        assert_eq!(rep_blta.achieved_level, PadesLevel::LTA);
        assert!(rep_blta.missing_for_next_level.is_empty());

        // Offline TSA consent enforcement check.
        assert!(matches!(
            fetch_tsa_response("https://tsa.example.invalid", &tsa_req, TsaConsent::Denied),
            Err(SigningError::ConsentRequired)
        ));
    }
}
