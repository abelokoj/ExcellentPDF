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
    use ecclesia_mupdf::{Document, PagePoint, PageRect};
    use ed25519_dalek::SigningKey;
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
            common_name: "EcclesiaPDF Root CA".to_owned(),
            organization: Some("EcclesiaPDF".to_owned()),
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
        let tsa_dn = DistinguishedName::with_common_name("EcclesiaPDF TSA");
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
        assert_eq!(verified[0].signer.issuer_common_name, "EcclesiaPDF Root CA");
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

    fn sign_pdf_with_custom_cms(
        pdf_bytes: &[u8],
        field_name: &str,
        build_cms: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Vec<u8> {
        use crate::forms::pdf_codec::{IncrementalPdfWriter, PdfIndex, encode_pdf_literal};
        let index = PdfIndex::parse(pdf_bytes).expect("parse pdf");
        let pages = index.collect_pages().expect("pages");
        let (page_obj_id, page_gen) = pages[0];
        let mut writer = IncrementalPdfWriter::from_index(&index).expect("writer");
        let sig_id = writer.alloc_id();
        let widget_id = writer.alloc_id();
        let placeholder = pdf_sig::byte_range_placeholder();
        let hex_cap = pdf_sig::signature_hex_capacity();
        let sig_dict = format!(
            "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.CAdES.detached /ByteRange {placeholder} /Contents <{}> >>",
            "0".repeat(hex_cap)
        );
        let sig_offset = writer.write_object(sig_id, 0, &sig_dict);
        let widget_body = format!(
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T {} /Rect [0 0 0 0] /F 132 /P {page_obj_id} {page_gen} R /V {sig_id} 0 R >>",
            encode_pdf_literal(field_name)
        );
        writer.write_object(widget_id, 0, &widget_body);
        let mut out = writer.finish();

        let tail = &out[sig_offset..];
        let br_rel = tail
            .windows(placeholder.len())
            .position(|w| w == placeholder.as_bytes())
            .expect("placeholder");
        let br_pos = sig_offset + br_rel;
        let marker = b"/Contents <";
        let c_rel = tail
            .windows(marker.len())
            .position(|w| w == marker)
            .expect("contents");
        let lt_pos = sig_offset + c_rel + b"/Contents ".len();
        let gt_rel = tail[c_rel + marker.len()..]
            .iter()
            .position(|&b| b == b'>')
            .expect("closing >");
        let gt_end = sig_offset + c_rel + marker.len() + gt_rel + 1;

        let tail_len = out.len() - gt_end;
        let formatted_br = format!("[0 {lt_pos:010} {gt_end:010} {tail_len:010}]");
        out[br_pos..br_pos + formatted_br.len()].copy_from_slice(formatted_br.as_bytes());

        let mut covered = Vec::with_capacity(lt_pos + (out.len() - gt_end));
        covered.extend_from_slice(&out[..lt_pos]);
        covered.extend_from_slice(&out[gt_end..]);
        let cms_der = build_cms(&covered);
        let hex_cms = der::hex_upper(&cms_der);
        out[lt_pos + 1..lt_pos + 1 + hex_cms.len()].copy_from_slice(hex_cms.as_bytes());
        out
    }

    #[test]
    fn verifies_rsa2048_ecdsa_p256_and_ecdsa_p384_signed_pdfs_end_to_end() {
        use cms::build_custom_cms_for_test;
        use der::test_support::{
            TEST_RSA_D_HEX, TEST_RSA_E_HEX, TEST_RSA_N_HEX, ecdsa_p256_pubkey, ecdsa_p256_sign,
            ecdsa_p384_pubkey, ecdsa_p384_sign, encode_rsa_spki, hex_decode, issue_custom_cert,
            rsa_pkcs1_sign,
        };
        use der::{
            AlgParams, HashAlgorithm, OID_EC_PUBLIC_KEY, OID_ECDSA_WITH_SHA256,
            OID_ECDSA_WITH_SHA384, OID_SECP256R1, OID_SECP384R1, OID_SHA256_WITH_RSA,
            encode_alg_id, encode_bit_string, encode_oid, encode_sequence,
        };

        let pdf = minimal_pdf();

        // 1. RSA-2048 PKCS#1 v1.5 (SHA-256)
        let n = hex_decode(TEST_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let d = hex_decode(TEST_RSA_D_HEX);
        let rsa_spki = encode_rsa_spki(&n, &e);
        let rsa_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("PDF RSA Root CA"),
            &rsa_spki,
            None,
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::CertificateAuthority,
            501,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa ca");
        let rsa_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("Grace Hopper (RSA)"),
            &rsa_spki,
            Some(&rsa_ca),
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::EndEntity,
            502,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa leaf");
        let rsa_pdf = sign_pdf_with_custom_cms(&pdf, "RsaSig", |covered| {
            build_custom_cms_for_test(
                covered,
                &rsa_leaf,
                &rsa_ca,
                HashAlgorithm::Sha256,
                encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
                |sa| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, sa),
            )
        });
        let rsa_store = TrustStore::with_anchors([rsa_ca]);
        let rsa_res = verify_pdf_signatures(&rsa_pdf, &rsa_store).expect("verify rsa pdf");
        assert_eq!(rsa_res.len(), 1);
        assert_eq!(rsa_res[0].trust_status, TrustStatus::Trusted);
        assert_eq!(rsa_res[0].signer.common_name, "Grace Hopper (RSA)");
        assert_eq!(rsa_res[0].signer.signature_algorithm, "RSA-SHA256");

        let mut tampered_rsa = rsa_pdf;
        let pos = tampered_rsa
            .windows(7)
            .position(|w| w == b"initial")
            .expect("find initial");
        tampered_rsa[pos] = b'Z';
        let bad_rsa =
            verify_pdf_signatures(&tampered_rsa, &rsa_store).expect("verify tampered rsa");
        assert_eq!(bad_rsa[0].trust_status, TrustStatus::Tampered);

        // 2. ECDSA P-256 (SHA-256)
        let d256 = [
            0xCAFE_BABE_1111_2222u64,
            0x3333_4444_5555_6666,
            0x7777,
            0x0001,
        ];
        let k256 = [
            0x8888_9999_AAAA_BBBBu64,
            0xCCCC_DDDD_EEEE_FFFF,
            0x1234,
            0x0002,
        ];
        let spki_256 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP256R1)]),
            encode_bit_string(&ecdsa_p256_pubkey(d256), 0),
        ]);
        let p256_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("PDF P-256 Root CA"),
            &spki_256,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::CertificateAuthority,
            601,
            |tbs| ecdsa_p256_sign(d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 ca");
        let p256_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("Alan Turing (P-256)"),
            &spki_256,
            Some(&p256_ca),
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::EndEntity,
            602,
            |tbs| ecdsa_p256_sign(d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 leaf");
        let p256_pdf = sign_pdf_with_custom_cms(&pdf, "EcdsaP256Sig", |covered| {
            build_custom_cms_for_test(
                covered,
                &p256_leaf,
                &p256_ca,
                HashAlgorithm::Sha256,
                encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
                |sa| ecdsa_p256_sign(d256, k256, HashAlgorithm::Sha256, sa),
            )
        });
        let p256_store = TrustStore::with_anchors([p256_ca]);
        let p256_res = verify_pdf_signatures(&p256_pdf, &p256_store).expect("verify p256 pdf");
        assert_eq!(p256_res[0].trust_status, TrustStatus::Trusted);
        assert_eq!(p256_res[0].signer.signature_algorithm, "ECDSA-SHA256");

        // 3. ECDSA P-384 (SHA-384)
        let d384 = [
            0x2468_ACE0_1357_9BDFu64,
            0x0102_0304_0506_0708,
            0x090A_0B0C_0D0E_0F10,
            0x1112_1314_1516_1718,
            0x191A,
            0x0001,
        ];
        let k384 = [
            0xF0E0_D0C0_B0A0_9080u64,
            0x7060_5040_3020_1000,
            0xAA55_AA55_AA55_AA55,
            0x55AA_55AA_55AA_55AA,
            0x4321,
            0x0002,
        ];
        let spki_384 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP384R1)]),
            encode_bit_string(&ecdsa_p384_pubkey(d384), 0),
        ]);
        let p384_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("PDF P-384 Root CA"),
            &spki_384,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::CertificateAuthority,
            701,
            |tbs| ecdsa_p384_sign(d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 ca");
        let p384_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("Katherine Johnson (P-384)"),
            &spki_384,
            Some(&p384_ca),
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::EndEntity,
            702,
            |tbs| ecdsa_p384_sign(d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 leaf");
        let p384_pdf = sign_pdf_with_custom_cms(&pdf, "EcdsaP384Sig", |covered| {
            build_custom_cms_for_test(
                covered,
                &p384_leaf,
                &p384_ca,
                HashAlgorithm::Sha384,
                encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
                |sa| ecdsa_p384_sign(d384, k384, HashAlgorithm::Sha384, sa),
            )
        });
        let p384_store = TrustStore::with_anchors([p384_ca]);
        let p384_res = verify_pdf_signatures(&p384_pdf, &p384_store).expect("verify p384 pdf");
        assert_eq!(p384_res[0].trust_status, TrustStatus::Trusted);
        assert_eq!(p384_res[0].signer.digest_algorithm, "SHA-384");
        assert_eq!(p384_res[0].signer.signature_algorithm, "ECDSA-SHA384");
        let unknown_384 =
            verify_pdf_signatures(&p384_pdf, &TrustStore::new()).expect("verify unknown p384");
        assert_eq!(unknown_384[0].trust_status, TrustStatus::UnknownIssuer);
    }
}
