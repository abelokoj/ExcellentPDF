//! Digital signing, signature inspection, DocMDP edit guard, and PAdES LTV feature facade
//! (plan O.14, O.15, O.16, O.20).

mod panel;

pub use panel::{
    EditGuardDecision, LtvRevocationEvidence, OnlineTimestampConsent, SignaturePanelModel,
    SignatureSummary, SigningWorkflow, TimestampConfig, evaluate_edit_guard,
    extract_signed_revision,
};

#[cfg(test)]
mod tests {
    use ecclesia_core::signing::{
        CertRole, CertValidity, DistinguishedName, DocMdpPermission, DocumentEditKind,
        LocalTimestampAuthority, PadesLevel, SignatureAppearance, TrustStatus, TrustStore,
        X509Certificate,
    };
    use ecclesia_mupdf::Document;
    use ed25519_dalek::SigningKey;

    use super::*;

    fn sample_pdf() -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");
        let mut push_obj = |id: u32, body: &str| {
            offsets.push((id, out.len()));
            out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
        };
        push_obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
        push_obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        push_obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>");
        let xref_off = out.len();
        out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for (_, off) in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_off}\n%%EOF\n")
                .as_bytes(),
        );
        out
    }

    #[test]
    fn workflow_signs_lta_inspections_extracts_revision_and_guards_edits() {
        let ca_key = SigningKey::from_bytes(&[11; 32]);
        let ca_cert = X509Certificate::issue_ed25519(
            &DistinguishedName::with_common_name("Features Root CA"),
            &ca_key.verifying_key(),
            None,
            &ca_key,
            CertValidity::default(),
            CertRole::CertificateAuthority,
            1,
        )
        .expect("ca");

        let signer_key = SigningKey::from_bytes(&[12; 32]);
        let signer_cert = X509Certificate::issue_ed25519(
            &DistinguishedName::with_common_name("Grace Hopper"),
            &signer_key.verifying_key(),
            Some(&ca_cert),
            &ca_key,
            CertValidity::default(),
            CertRole::EndEntity,
            2,
        )
        .expect("signer");

        let tsa_key = SigningKey::from_bytes(&[13; 32]);
        let tsa_cert = X509Certificate::issue_ed25519(
            &DistinguishedName::with_common_name("Features TSA"),
            &tsa_key.verifying_key(),
            Some(&ca_cert),
            &ca_key,
            CertValidity::default(),
            CertRole::TimestampAuthority,
            3,
        )
        .expect("tsa");
        let authority = LocalTimestampAuthority::new(tsa_cert, vec![ca_cert.clone()], tsa_key);

        let pdf = sample_pdf();
        let workflow = SigningWorkflow {
            signer_cert: &signer_cert,
            chain_certs: std::slice::from_ref(&ca_cert),
            signing_key: &signer_key,
            field_name: "ContractSig".to_owned(),
            reason: Some("Final approval".to_owned()),
            location: Some("New York".to_owned()),
            signing_time: 1_760_100_000,
            appearance: SignatureAppearance::Invisible,
            certification: Some(DocMdpPermission::FormFillAndSign),
            timestamp: TimestampConfig::Local {
                authority: Box::new(authority),
                gen_time: 1_760_100_050,
                serial: 900,
            },
            ltv_revocation: Some(LtvRevocationEvidence {
                ocsps: vec![b"OCSP-DATA".to_vec()],
                crls: Vec::new(),
            }),
        };

        let signed = workflow.execute(&pdf).expect("execute signing workflow");
        let store = TrustStore::with_anchors([ca_cert]);
        let panel = SignaturePanelModel::inspect(&signed, &store).expect("inspect panel");
        assert_eq!(panel.pades_level(), PadesLevel::LTA);
        assert_eq!(panel.signatures[0].trust_status, TrustStatus::Trusted);
        assert_eq!(panel.signatures[0].signer_name, "Grace Hopper");

        // "View signed version" extracts a valid PDF revision at the first signature.
        let rev0 = extract_signed_revision(&signed, 0).expect("extract signed revision");
        let doc = Document::from_bytes(&rev0, "application/pdf").expect("open signed revision");
        assert_eq!(doc.page_count().expect("pages"), 1);

        // Edit guard allows form fill under P2, warns on content edit.
        assert_eq!(
            evaluate_edit_guard(
                &signed,
                &DocumentEditKind::FormFill {
                    field_name: Some("amount".to_owned()),
                },
            )
            .expect("guard form"),
            EditGuardDecision::ProceedSilently
        );
        assert!(matches!(
            evaluate_edit_guard(&signed, &DocumentEditKind::ContentEdit).expect("guard content"),
            EditGuardDecision::WarnUser { .. }
        ));
    }
}
