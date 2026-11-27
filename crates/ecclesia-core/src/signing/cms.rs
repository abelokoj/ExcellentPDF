//! RFC 5652 CMS/PKCS#7 `SignedData` builder and detached verifier with PAdES `signing-certificate-v2`
//! and RFC 3161 `id-aa-timeStampToken` attributes (plan O.14, O.15, O.20).

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use super::der::{
    AlgParams, ChainTrust, HashAlgorithm, OID_CONTENT_TYPE, OID_EC_PUBLIC_KEY,
    OID_ECDSA_WITH_SHA256, OID_ECDSA_WITH_SHA384, OID_ECDSA_WITH_SHA512, OID_ED25519,
    OID_MESSAGE_DIGEST, OID_PKCS7_DATA, OID_PKCS7_SIGNED_DATA, OID_RSA_ENCRYPTION, OID_RSASSA_PSS,
    OID_SHA256, OID_SHA256_WITH_RSA, OID_SHA384, OID_SHA384_WITH_RSA, OID_SHA512,
    OID_SHA512_WITH_RSA, OID_SIGNING_CERT_V2, OID_SIGNING_TIME, OID_TIMESTAMP_TOKEN, SigningError,
    TAG_INTEGER, TAG_OCTET_STRING, TAG_SEQUENCE, TAG_SET, TrustStore, X509Certificate,
    encode_alg_id, encode_context_constructed, encode_octet_string, encode_oid, encode_sequence,
    encode_set, encode_tlv, encode_u64, encode_utc_time, parse_der_prefix,
    verify_signature_over_slices,
};

/// Parameters for constructing a detached CMS/PKCS#7 `SignedData` signature.
pub(crate) struct CmsBuildParams<'a> {
    pub(crate) signer_cert: &'a X509Certificate,
    pub(crate) chain_certs: &'a [X509Certificate],
    pub(crate) signing_key: &'a SigningKey,
    pub(crate) signing_time: i64,
    pub(crate) timestamp_token_der: Option<&'a [u8]>,
}

/// Parsed fields from a CMS/PKCS#7 `SignedData` container.
#[derive(Debug, Clone)]
pub(crate) struct ParsedCmsSignedData {
    pub(crate) certificates: Vec<X509Certificate>,
    pub(crate) signer_issuer_der: Vec<u8>,
    pub(crate) signer_serial: Vec<u8>,
    pub(crate) signer_skid: Option<Vec<u8>>,
    pub(crate) digest_algorithm_oid: Vec<u32>,
    pub(crate) signature_algorithm_oid: Vec<u32>,
    pub(crate) signature_algorithm_params_der: Option<Vec<u8>>,
    pub(crate) message_digest: Vec<u8>,
    pub(crate) signing_time: Option<i64>,
    pub(crate) signing_cert_v2_hash: Option<(HashAlgorithm, Vec<u8>)>,
    pub(crate) signed_attrs_set_der: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    pub(crate) timestamp_tokens: Vec<Vec<u8>>,
}

/// Outcome of verifying a detached CMS signature against content slices and [`TrustStore`].
#[derive(Debug, Clone)]
pub(crate) struct CmsVerification {
    pub(crate) signer_cert: Option<X509Certificate>,
    pub(crate) chain_trust: ChainTrust,
    pub(crate) digest_valid: bool,
    pub(crate) signature_valid: bool,
    pub(crate) signing_time: Option<i64>,
    pub(crate) has_signing_cert_v2: bool,
    pub(crate) timestamp_tokens: Vec<Vec<u8>>,
    pub(crate) digest_algorithm_name: String,
    pub(crate) signature_algorithm_name: String,
    pub(crate) failure_reason: Option<String>,
}

/// Builds a detached CMS `ContentInfo` wrapping `SignedData` (`ETSI.CAdES.detached` compatible).
pub(crate) fn build_cms_signed_data(
    digest_sha256: &[u8; 32],
    params: &CmsBuildParams<'_>,
) -> Result<Vec<u8>, SigningError> {
    let cert_hash = params.signer_cert.sha256_digest();
    // ESSCertIDv2: SEQUENCE { certHash OCTET STRING } (default SHA-256 per RFC 5035 §4)
    let ess_cert_id_v2 = encode_sequence(&[encode_octet_string(&cert_hash)]);
    let signing_cert_v2 = encode_sequence(&[encode_sequence(&[ess_cert_id_v2])]);

    let attr_content_type = encode_sequence(&[
        encode_oid(OID_CONTENT_TYPE),
        encode_set(&[encode_oid(OID_PKCS7_DATA)]),
    ]);
    let attr_message_digest = encode_sequence(&[
        encode_oid(OID_MESSAGE_DIGEST),
        encode_set(&[encode_octet_string(digest_sha256)]),
    ]);
    let attr_signing_time = encode_sequence(&[
        encode_oid(OID_SIGNING_TIME),
        encode_set(&[encode_utc_time(params.signing_time)]),
    ]);
    let attr_signing_cert = encode_sequence(&[
        encode_oid(OID_SIGNING_CERT_V2),
        encode_set(&[signing_cert_v2]),
    ]);

    let mut attrs = vec![
        attr_content_type,
        attr_message_digest,
        attr_signing_time,
        attr_signing_cert,
    ];
    attrs.sort();
    let signed_attrs_for_signing = encode_set(&attrs);
    // Replace outer SET (0x31) tag with [0] IMPLICIT (0xA0) for SignerInfo encoding.
    let mut signed_attrs_implicit = signed_attrs_for_signing.clone();
    signed_attrs_implicit[0] = 0xA0;

    let sig = params.signing_key.sign(&signed_attrs_for_signing);
    let sid = encode_sequence(&[
        params.signer_cert.issuer_der.clone(),
        encode_tlv(TAG_INTEGER, &params.signer_cert.serial),
    ]);

    let mut signer_info_parts = vec![
        encode_u64(1),
        sid,
        encode_alg_id(OID_SHA256, AlgParams::Null),
        signed_attrs_implicit,
        encode_alg_id(OID_ED25519, AlgParams::None),
        encode_octet_string(&sig.to_bytes()),
    ];

    if let Some(tst_der) = params.timestamp_token_der {
        let attr_tst = encode_sequence(&[
            encode_oid(OID_TIMESTAMP_TOKEN),
            encode_set(&[tst_der.to_vec()]),
        ]);
        signer_info_parts.push(encode_context_constructed(1, &[attr_tst]));
    }

    let signer_info = encode_sequence(&signer_info_parts);

    let mut cert_ders = vec![params.signer_cert.raw_der.clone()];
    for c in params.chain_certs {
        if c.raw_der != params.signer_cert.raw_der {
            cert_ders.push(c.raw_der.clone());
        }
    }
    let certs_implicit = encode_context_constructed(0, &cert_ders);

    let signed_data = encode_sequence(&[
        encode_u64(1),
        encode_set(&[encode_alg_id(OID_SHA256, AlgParams::Null)]),
        encode_sequence(&[encode_oid(OID_PKCS7_DATA)]),
        certs_implicit,
        encode_set(&[signer_info]),
    ]);

    let content_info = encode_sequence(&[
        encode_oid(OID_PKCS7_SIGNED_DATA),
        encode_context_constructed(0, &[signed_data]),
    ]);
    Ok(content_info)
}

/// Parses a DER-encoded CMS `ContentInfo` containing `SignedData`.
pub(crate) fn parse_cms_signed_data(der: &[u8]) -> Result<ParsedCmsSignedData, SigningError> {
    let (root, _) = parse_der_prefix(der)?;
    let top = root.children()?;
    if top.len() < 2 {
        return Err(SigningError::DerFormat(
            "CMS ContentInfo too short".to_owned(),
        ));
    }
    if top[0].as_oid()? != OID_PKCS7_SIGNED_DATA {
        return Err(SigningError::DerFormat(
            "CMS ContentInfo is not SignedData".to_owned(),
        ));
    }
    let sd_wrapper = top[1].children()?;
    let sd_node = sd_wrapper
        .first()
        .ok_or_else(|| SigningError::DerFormat("missing SignedData body".to_owned()))?;
    let sd_fields = sd_node.children()?;
    if sd_fields.len() < 4 {
        return Err(SigningError::DerFormat("truncated SignedData".to_owned()));
    }

    let mut certificates = Vec::new();
    let mut signer_infos_node = None;
    for node in &sd_fields[3..] {
        if node.tag == 0xA0 {
            for cert_node in node.children()? {
                if let Ok(cert) = X509Certificate::from_der(cert_node.full) {
                    certificates.push(cert);
                }
            }
        } else if node.tag == TAG_SET {
            signer_infos_node = Some(*node);
        }
    }

    let si_set = signer_infos_node
        .ok_or_else(|| SigningError::DerFormat("SignedData missing signerInfos".to_owned()))?;
    let si_list = si_set.children()?;
    let si = si_list
        .first()
        .ok_or_else(|| SigningError::DerFormat("SignedData has empty signerInfos".to_owned()))?;
    let si_fields = si.children()?;
    if si_fields.len() < 5 {
        return Err(SigningError::DerFormat("truncated SignerInfo".to_owned()));
    }

    let (signer_issuer_der, signer_serial, signer_skid) = if si_fields[1].tag == 0x80 {
        (Vec::new(), Vec::new(), Some(si_fields[1].content.to_vec()))
    } else {
        let sid_children = si_fields[1].children()?;
        if sid_children.len() < 2 {
            return Err(SigningError::DerFormat(
                "SignerIdentifier must be IssuerAndSerialNumber or SubjectKeyIdentifier".to_owned(),
            ));
        }
        (
            sid_children[0].full.to_vec(),
            sid_children[1].as_integer_bytes()?.to_vec(),
            None,
        )
    };

    let digest_algorithm_oid = si_fields[2]
        .children()?
        .first()
        .ok_or_else(|| SigningError::DerFormat("missing digestAlgorithm".to_owned()))?
        .as_oid()?;

    let mut idx = 3usize;
    let mut message_digest = Vec::new();
    let mut signing_time = None;
    let mut signing_cert_v2_hash = None;
    let mut signed_attrs_set_der = Vec::new();

    if si_fields[idx].tag == 0xA0 {
        let sa_node = si_fields[idx];
        let mut rebuilt = sa_node.full.to_vec();
        rebuilt[0] = TAG_SET;
        signed_attrs_set_der = rebuilt;
        for attr in sa_node.children()? {
            let parts = attr.children()?;
            if parts.len() < 2 {
                continue;
            }
            let oid = parts[0].as_oid()?;
            let values = parts[1].children()?;
            let Some(&first_val) = values.first() else {
                continue;
            };
            if oid == OID_MESSAGE_DIGEST && first_val.tag == TAG_OCTET_STRING {
                message_digest = first_val.content.to_vec();
            } else if oid == OID_SIGNING_TIME {
                signing_time = first_val.as_unix_time().ok();
            } else if oid == OID_SIGNING_CERT_V2
                && first_val.tag == TAG_SEQUENCE
                && let Ok(sc_fields) = first_val.children()
                && let Some(certs_seq) = sc_fields.first()
                && let Ok(ess_list) = certs_seq.children()
                && let Some(ess) = ess_list.first()
                && let Ok(ess_parts) = ess.children()
                && let Some(first_ess) = ess_parts.first()
            {
                if first_ess.tag == TAG_OCTET_STRING {
                    signing_cert_v2_hash =
                        Some((HashAlgorithm::Sha256, first_ess.content.to_vec()));
                } else if first_ess.tag == TAG_SEQUENCE
                    && let Ok(alg_children) = first_ess.children()
                    && let Some(alg_oid_node) = alg_children.first()
                    && let Ok(alg_oid) = alg_oid_node.as_oid()
                    && let Some(alg) = HashAlgorithm::from_oid(&alg_oid)
                    && let Some(hash_node) = ess_parts.get(1)
                    && hash_node.tag == TAG_OCTET_STRING
                {
                    signing_cert_v2_hash = Some((alg, hash_node.content.to_vec()));
                }
            }
        }
        idx += 1;
    }

    let sig_alg_children = si_fields
        .get(idx)
        .ok_or_else(|| SigningError::DerFormat("missing signatureAlgorithm".to_owned()))?
        .children()?;
    let signature_algorithm_oid = sig_alg_children
        .first()
        .ok_or_else(|| SigningError::DerFormat("missing signatureAlgorithm OID".to_owned()))?
        .as_oid()?;
    let signature_algorithm_params_der = sig_alg_children.get(1).map(|n| n.full.to_vec());
    idx += 1;

    let signature_node = si_fields
        .get(idx)
        .ok_or_else(|| SigningError::DerFormat("missing signature value".to_owned()))?;
    let signature = signature_node.content.to_vec();
    idx += 1;

    let mut timestamp_tokens = Vec::new();
    if let Some(unsigned) = si_fields.get(idx)
        && unsigned.tag == 0xA1
    {
        for attr in unsigned.children()? {
            let parts = attr.children()?;
            if parts.len() >= 2 && parts[0].as_oid().ok().as_deref() == Some(OID_TIMESTAMP_TOKEN) {
                for val in parts[1].children()? {
                    timestamp_tokens.push(val.full.to_vec());
                }
            }
        }
    }

    Ok(ParsedCmsSignedData {
        certificates,
        signer_issuer_der,
        signer_serial,
        signer_skid,
        digest_algorithm_oid,
        signature_algorithm_oid,
        signature_algorithm_params_der,
        message_digest,
        signing_time,
        signing_cert_v2_hash,
        signed_attrs_set_der,
        signature,
        timestamp_tokens,
    })
}

/// Verifies a detached CMS `SignedData` payload against `content_slices` and `trust_store`.
///
/// Supports Ed25519, RSA PKCS#1 v1.5 (SHA-256 / SHA-384 / SHA-512), RSA-PSS (MGF1-SHA256 /
/// SHA-384 / SHA-512), and ECDSA (P-256 / P-384), both with and without `signedAttrs`.
pub(crate) fn verify_cms_detached_slices(
    der: &[u8],
    content_slices: &[&[u8]],
    trust_store: &TrustStore,
    check_time: Option<i64>,
) -> Result<CmsVerification, SigningError> {
    let parsed = parse_cms_signed_data(der)?;
    let digest_algorithm_name = oid_display_name(&parsed.digest_algorithm_oid);
    let signature_algorithm_name = oid_display_name(&parsed.signature_algorithm_oid);
    let has_signing_cert_v2 = parsed.signing_cert_v2_hash.is_some();

    let signer_cert = if let Some(ref skid) = parsed.signer_skid {
        parsed
            .certificates
            .iter()
            .find(|c| c.subject_key_id.as_deref() == Some(skid.as_slice()))
            .cloned()
    } else {
        parsed
            .certificates
            .iter()
            .find(|c| c.issuer_der == parsed.signer_issuer_der && c.serial == parsed.signer_serial)
            .cloned()
    }
    .or_else(|| parsed.certificates.first().cloned());

    let digest_alg = HashAlgorithm::from_oid(&parsed.digest_algorithm_oid);
    let has_signed_attrs = !parsed.signed_attrs_set_der.is_empty();
    let digest_valid = if has_signed_attrs {
        digest_alg.is_some_and(|alg| {
            let expected = alg.hash_slices(content_slices);
            parsed.message_digest.as_slice() == expected.as_slice()
        })
    } else {
        digest_alg.is_some()
    };

    let Some(ref cert) = signer_cert else {
        return Ok(CmsVerification {
            signer_cert: None,
            chain_trust: ChainTrust::UnknownIssuer,
            digest_valid,
            signature_valid: false,
            signing_time: parsed.signing_time,
            has_signing_cert_v2,
            timestamp_tokens: parsed.timestamp_tokens,
            digest_algorithm_name,
            signature_algorithm_name,
            failure_reason: Some("CMS SignedData contains no signer certificate".to_owned()),
        });
    };

    if let Some((cert_hash_alg, ref cert_hash)) = parsed.signing_cert_v2_hash
        && cert_hash.as_slice() != cert_hash_alg.hash(&cert.raw_der).as_slice()
    {
        return Ok(CmsVerification {
            signer_cert: Some(cert.clone()),
            chain_trust: ChainTrust::Tampered(
                "signing-certificate-v2 digest does not match signer certificate".to_owned(),
            ),
            digest_valid,
            signature_valid: false,
            signing_time: parsed.signing_time,
            has_signing_cert_v2,
            timestamp_tokens: parsed.timestamp_tokens,
            digest_algorithm_name,
            signature_algorithm_name,
            failure_reason: Some("signing-certificate-v2 mismatch".to_owned()),
        });
    }

    let mut signature_valid = false;
    let mut failure_reason = if digest_valid {
        None
    } else if digest_alg.is_none() {
        Some(format!(
            "unsupported CMS digest algorithm {digest_algorithm_name}"
        ))
    } else {
        Some(format!(
            "PDF ByteRange {digest_algorithm_name} digest mismatch"
        ))
    };

    let signed_attrs_slice = [parsed.signed_attrs_set_der.as_slice()];
    let sig_input_slices: &[&[u8]] = if has_signed_attrs {
        &signed_attrs_slice
    } else {
        content_slices
    };

    match verify_signature_over_slices(
        cert,
        &parsed.signature_algorithm_oid,
        parsed.signature_algorithm_params_der.as_deref(),
        digest_alg,
        sig_input_slices,
        &parsed.signature,
    ) {
        Ok(()) => {
            signature_valid = true;
        }
        Err(err) => {
            failure_reason = Some(err.to_string());
        }
    }

    let chain_trust = trust_store.evaluate_chain(cert, &parsed.certificates, check_time);

    Ok(CmsVerification {
        signer_cert: Some(cert.clone()),
        chain_trust,
        digest_valid,
        signature_valid,
        signing_time: parsed.signing_time,
        has_signing_cert_v2,
        timestamp_tokens: parsed.timestamp_tokens,
        digest_algorithm_name,
        signature_algorithm_name,
        failure_reason,
    })
}

/// Computes SHA-256 over the two PDF `/ByteRange` slices `[a..a+b]` and `[c..c+d]`.
pub(crate) fn hash_pdf_byte_range(pdf_bytes: &[u8], br: [usize; 4]) -> Option<[u8; 32]> {
    let [a, b, c, d] = br;
    let end1 = a.checked_add(b)?;
    let end2 = c.checked_add(d)?;
    if end1 > c || end2 > pdf_bytes.len() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(&pdf_bytes[a..end1]);
    hasher.update(&pdf_bytes[c..end2]);
    Some(hasher.finalize().into())
}

pub(crate) fn oid_display_name(oid: &[u32]) -> String {
    if oid == OID_SHA256 {
        "SHA-256".to_owned()
    } else if oid == OID_SHA384 {
        "SHA-384".to_owned()
    } else if oid == OID_SHA512 {
        "SHA-512".to_owned()
    } else if oid == OID_ED25519 {
        "Ed25519".to_owned()
    } else if oid == OID_RSA_ENCRYPTION {
        "RSA".to_owned()
    } else if oid == OID_SHA256_WITH_RSA {
        "RSA-SHA256".to_owned()
    } else if oid == OID_SHA384_WITH_RSA {
        "RSA-SHA384".to_owned()
    } else if oid == OID_SHA512_WITH_RSA {
        "RSA-SHA512".to_owned()
    } else if oid == OID_RSASSA_PSS {
        "RSA-PSS".to_owned()
    } else if oid == OID_EC_PUBLIC_KEY {
        "ECDSA".to_owned()
    } else if oid == OID_ECDSA_WITH_SHA256 {
        "ECDSA-SHA256".to_owned()
    } else if oid == OID_ECDSA_WITH_SHA384 {
        "ECDSA-SHA384".to_owned()
    } else if oid == OID_ECDSA_WITH_SHA512 {
        "ECDSA-SHA512".to_owned()
    } else {
        oid.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
    }
}

#[cfg(test)]
pub(crate) fn build_custom_cms_for_test(
    content: &[u8],
    signer_cert: &X509Certificate,
    ca_cert: &X509Certificate,
    digest_alg: HashAlgorithm,
    sig_alg_der: Vec<u8>,
    sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
) -> Vec<u8> {
    let digest_oid = match digest_alg {
        HashAlgorithm::Sha256 => OID_SHA256,
        HashAlgorithm::Sha384 => OID_SHA384,
        HashAlgorithm::Sha512 => OID_SHA512,
    };
    let digest = digest_alg.hash(content);
    let cert_hash = digest_alg.hash(&signer_cert.raw_der);
    let ess_cert_id_v2 = if digest_alg == HashAlgorithm::Sha256 {
        encode_sequence(&[encode_octet_string(&cert_hash)])
    } else {
        encode_sequence(&[
            encode_alg_id(digest_oid, AlgParams::Null),
            encode_octet_string(&cert_hash),
        ])
    };
    let signing_cert_v2 = encode_sequence(&[encode_sequence(&[ess_cert_id_v2])]);

    let attr_content_type = encode_sequence(&[
        encode_oid(OID_CONTENT_TYPE),
        encode_set(&[encode_oid(OID_PKCS7_DATA)]),
    ]);
    let attr_message_digest = encode_sequence(&[
        encode_oid(OID_MESSAGE_DIGEST),
        encode_set(&[encode_octet_string(&digest)]),
    ]);
    let attr_signing_time = encode_sequence(&[
        encode_oid(OID_SIGNING_TIME),
        encode_set(&[encode_utc_time(1_760_000_000)]),
    ]);
    let attr_signing_cert = encode_sequence(&[
        encode_oid(OID_SIGNING_CERT_V2),
        encode_set(&[signing_cert_v2]),
    ]);

    let mut attrs = vec![
        attr_content_type,
        attr_message_digest,
        attr_signing_time,
        attr_signing_cert,
    ];
    attrs.sort();
    let signed_attrs_set = encode_set(&attrs);
    let mut signed_attrs_implicit = signed_attrs_set.clone();
    signed_attrs_implicit[0] = 0xA0;

    let sig = sign_fn(&signed_attrs_set);
    let sid = encode_sequence(&[
        signer_cert.issuer_der.clone(),
        encode_tlv(TAG_INTEGER, &signer_cert.serial),
    ]);
    let signer_info = encode_sequence(&[
        encode_u64(1),
        sid,
        encode_alg_id(digest_oid, AlgParams::Null),
        signed_attrs_implicit,
        sig_alg_der,
        encode_octet_string(&sig),
    ]);

    let certs_implicit =
        encode_context_constructed(0, &[signer_cert.raw_der.clone(), ca_cert.raw_der.clone()]);
    let signed_data = encode_sequence(&[
        encode_u64(1),
        encode_set(&[encode_alg_id(digest_oid, AlgParams::Null)]),
        encode_sequence(&[encode_oid(OID_PKCS7_DATA)]),
        certs_implicit,
        encode_set(&[signer_info]),
    ]);
    encode_sequence(&[
        encode_oid(OID_PKCS7_SIGNED_DATA),
        encode_context_constructed(0, &[signed_data]),
    ])
}

#[cfg(test)]
mod tests {
    use super::super::der::test_support::{
        TEST_RSA_D_HEX, TEST_RSA_E_HEX, TEST_RSA_N_HEX, ecdsa_p256_pubkey, ecdsa_p256_sign,
        ecdsa_p384_pubkey, ecdsa_p384_sign, encode_pss_params_der, encode_rsa_spki, hex_decode,
        issue_custom_cert, rsa_pkcs1_sign, rsa_pss_sign,
    };
    use super::super::der::{
        CertRole, DistinguishedName, OID_EC_PUBLIC_KEY, OID_ECDSA_WITH_SHA256,
        OID_ECDSA_WITH_SHA384, OID_RSA_ENCRYPTION, OID_RSASSA_PSS, OID_SECP256R1, OID_SECP384R1,
        OID_SHA256_WITH_RSA, OID_SHA384_WITH_RSA, OID_SHA512_WITH_RSA, encode_bit_string,
    };
    use super::*;

    #[test]
    fn verifies_rsa_pkcs1_cms_with_combined_and_bare_oids_and_rejects_tampering() {
        let n = hex_decode(TEST_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let d = hex_decode(TEST_RSA_D_HEX);
        let spki = encode_rsa_spki(&n, &e);
        let ca_dn = DistinguishedName::with_common_name("Test RSA Root CA");
        let leaf_dn = DistinguishedName::with_common_name("Test RSA Signer");

        let ca_cert = issue_custom_cert(
            &ca_dn,
            &spki,
            None,
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::CertificateAuthority,
            1,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa ca cert");
        let leaf_cert = issue_custom_cert(
            &leaf_dn,
            &spki,
            Some(&ca_cert),
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::EndEntity,
            2,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa leaf cert");

        let store = TrustStore::with_anchors([ca_cert.clone()]);
        let payload = b"CMS detached PDF byte range payload";

        // 1. sha256WithRSAEncryption
        let cms_sha256 = build_custom_cms_for_test(
            payload,
            &leaf_cert,
            &ca_cert,
            HashAlgorithm::Sha256,
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            |sa| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, sa),
        );
        let res = verify_cms_detached_slices(&cms_sha256, &[payload], &store, None)
            .expect("verify rsa sha256");
        assert!(res.digest_valid, "digest should match");
        assert!(res.signature_valid, "rsa-sha256 signature should verify");
        assert!(matches!(res.chain_trust, ChainTrust::Trusted { .. }));
        assert_eq!(res.signature_algorithm_name, "RSA-SHA256");

        // 2. Bare rsaEncryption OID in SignerInfo with SHA-384 digestAlgorithm
        let cms_bare_384 = build_custom_cms_for_test(
            payload,
            &leaf_cert,
            &ca_cert,
            HashAlgorithm::Sha384,
            encode_alg_id(OID_RSA_ENCRYPTION, AlgParams::Null),
            |sa| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha384, sa),
        );
        let res384 = verify_cms_detached_slices(&cms_bare_384, &[payload], &store, None)
            .expect("verify bare rsa sha384");
        assert!(res384.digest_valid);
        assert!(res384.signature_valid);
        assert_eq!(res384.digest_algorithm_name, "SHA-384");

        // 3. sha512WithRSAEncryption
        let cms_512 = build_custom_cms_for_test(
            payload,
            &leaf_cert,
            &ca_cert,
            HashAlgorithm::Sha512,
            encode_alg_id(OID_SHA512_WITH_RSA, AlgParams::Null),
            |sa| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha512, sa),
        );
        let res512 = verify_cms_detached_slices(&cms_512, &[payload], &store, None)
            .expect("verify rsa sha512");
        assert!(res512.digest_valid && res512.signature_valid);
        assert_eq!(res512.signature_algorithm_name, "RSA-SHA512");

        // 4. sha384WithRSAEncryption
        let cms_384 = build_custom_cms_for_test(
            payload,
            &leaf_cert,
            &ca_cert,
            HashAlgorithm::Sha384,
            encode_alg_id(OID_SHA384_WITH_RSA, AlgParams::Null),
            |sa| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha384, sa),
        );
        let res384_comb = verify_cms_detached_slices(&cms_384, &[payload], &store, None)
            .expect("verify rsa sha384 combined");
        assert!(res384_comb.digest_valid && res384_comb.signature_valid);

        // 5. Tampered content fails digest check
        let tampered_payload = b"CMS detached PDF byte range payloae";
        let bad_digest = verify_cms_detached_slices(&cms_sha256, &[tampered_payload], &store, None)
            .expect("verify tampered digest");
        assert!(!bad_digest.digest_valid);

        // 6. Tampered signature byte fails signature check
        let mut tampered_cms = cms_sha256;
        let last = tampered_cms.len() - 1;
        tampered_cms[last] ^= 0x01;
        let bad_sig = verify_cms_detached_slices(&tampered_cms, &[payload], &store, None)
            .expect("verify tampered sig");
        assert!(!bad_sig.signature_valid);
    }

    #[test]
    fn verifies_rsa_pss_cms_and_rejects_tampered_signature() {
        let n = hex_decode(TEST_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let d = hex_decode(TEST_RSA_D_HEX);
        let spki = encode_rsa_spki(&n, &e);
        let pss_params = encode_pss_params_der(HashAlgorithm::Sha256, 32);
        let pss_alg_id = encode_sequence(&[encode_oid(OID_RSASSA_PSS), pss_params]);
        let ca_dn = DistinguishedName::with_common_name("Test RSA-PSS CA");
        let leaf_dn = DistinguishedName::with_common_name("Test RSA-PSS Signer");
        let salt = [0xA5u8; 32];

        let ca_cert = issue_custom_cert(
            &ca_dn,
            &spki,
            None,
            pss_alg_id.clone(),
            CertRole::CertificateAuthority,
            10,
            |tbs| rsa_pss_sign(&n, &d, HashAlgorithm::Sha256, &salt, tbs),
        )
        .expect("pss ca");
        let leaf_cert = issue_custom_cert(
            &leaf_dn,
            &spki,
            Some(&ca_cert),
            pss_alg_id.clone(),
            CertRole::EndEntity,
            11,
            |tbs| rsa_pss_sign(&n, &d, HashAlgorithm::Sha256, &salt, tbs),
        )
        .expect("pss leaf");

        let store = TrustStore::with_anchors([ca_cert.clone()]);
        let payload = b"RSA-PSS CMS test message";
        let cms = build_custom_cms_for_test(
            payload,
            &leaf_cert,
            &ca_cert,
            HashAlgorithm::Sha256,
            pss_alg_id,
            |sa| rsa_pss_sign(&n, &d, HashAlgorithm::Sha256, &salt, sa),
        );

        let res = verify_cms_detached_slices(&cms, &[payload], &store, None).expect("verify pss");
        assert!(res.digest_valid);
        assert!(res.signature_valid);
        assert!(matches!(res.chain_trust, ChainTrust::Trusted { .. }));
        assert_eq!(res.signature_algorithm_name, "RSA-PSS");

        let mut tampered = cms;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x80;
        let bad = verify_cms_detached_slices(&tampered, &[payload], &store, None)
            .expect("verify bad pss");
        assert!(!bad.signature_valid);
    }

    #[test]
    fn verifies_ecdsa_p256_and_p384_cms_and_rejects_tampering() {
        // P-256
        let ca_d256 = [
            0x1111_2222_3333_4444u64,
            0x5555_6666_7777_8888,
            0x9999,
            0x0001,
        ];
        let leaf_d256 = [
            0xAAAA_BBBB_CCCC_DDDDu64,
            0x1234_5678_9ABC_DEF0,
            0x4242,
            0x0002,
        ];
        let k256 = [
            0x7777_8888_9999_AAAAu64,
            0xBBBB_CCCC_DDDD_EEEE,
            0x1357,
            0x0003,
        ];
        let ca_spki_256 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP256R1)]),
            encode_bit_string(&ecdsa_p256_pubkey(ca_d256), 0),
        ]);
        let leaf_spki_256 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP256R1)]),
            encode_bit_string(&ecdsa_p256_pubkey(leaf_d256), 0),
        ]);
        let ca_256 = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-256 Root CA"),
            &ca_spki_256,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::CertificateAuthority,
            21,
            |tbs| ecdsa_p256_sign(ca_d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 ca");
        let leaf_256 = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-256 Signer"),
            &leaf_spki_256,
            Some(&ca_256),
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::EndEntity,
            22,
            |tbs| ecdsa_p256_sign(ca_d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 leaf");

        let payload = b"ECDSA CMS detached payload";
        let cms_256 = build_custom_cms_for_test(
            payload,
            &leaf_256,
            &ca_256,
            HashAlgorithm::Sha256,
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            |sa| ecdsa_p256_sign(leaf_d256, k256, HashAlgorithm::Sha256, sa),
        );
        let store_256 = TrustStore::with_anchors([ca_256]);
        let res_256 =
            verify_cms_detached_slices(&cms_256, &[payload], &store_256, None).expect("p256 cms");
        assert!(res_256.digest_valid && res_256.signature_valid);
        assert!(matches!(res_256.chain_trust, ChainTrust::Trusted { .. }));
        assert_eq!(res_256.signature_algorithm_name, "ECDSA-SHA256");

        let mut bad_256 = cms_256;
        let last = bad_256.len() - 1;
        bad_256[last] ^= 0x01;
        let bad_res_256 = verify_cms_detached_slices(&bad_256, &[payload], &store_256, None)
            .expect("bad p256 cms");
        assert!(!bad_res_256.signature_valid);

        // P-384
        let ca_d384 = [
            0x1010_2020_3030_4040u64,
            0x5050_6060_7070_8080,
            0x9090_A0A0_B0B0_C0C0,
            0xD0D0_E0E0_F0F0_0101,
            0x2345,
            0x0001,
        ];
        let leaf_d384 = [
            0xDEAD_BEEF_CAFE_BABEu64,
            0x0123_4567_89AB_CDEF,
            0xFEDC_BA98_7654_3210,
            0x1111_2222_3333_4444,
            0x6789,
            0x0002,
        ];
        let k384 = [
            0x3141_5926_5358_9793u64,
            0x2384_6264_3383_2795,
            0x0288_4197_1693_9937,
            0x5105_8209_7494_4592,
            0x1111,
            0x0003,
        ];
        let ca_spki_384 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP384R1)]),
            encode_bit_string(&ecdsa_p384_pubkey(ca_d384), 0),
        ]);
        let leaf_spki_384 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP384R1)]),
            encode_bit_string(&ecdsa_p384_pubkey(leaf_d384), 0),
        ]);
        let ca_384 = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-384 Root CA"),
            &ca_spki_384,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::CertificateAuthority,
            31,
            |tbs| ecdsa_p384_sign(ca_d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 ca");
        let leaf_384 = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-384 Signer"),
            &leaf_spki_384,
            Some(&ca_384),
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::EndEntity,
            32,
            |tbs| ecdsa_p384_sign(ca_d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 leaf");

        let cms_384 = build_custom_cms_for_test(
            payload,
            &leaf_384,
            &ca_384,
            HashAlgorithm::Sha384,
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            |sa| ecdsa_p384_sign(leaf_d384, k384, HashAlgorithm::Sha384, sa),
        );
        let store_384 = TrustStore::with_anchors([ca_384]);
        let res_384 =
            verify_cms_detached_slices(&cms_384, &[payload], &store_384, None).expect("p384 cms");
        assert!(res_384.digest_valid && res_384.signature_valid);
        assert!(matches!(res_384.chain_trust, ChainTrust::Trusted { .. }));
        assert_eq!(res_384.digest_algorithm_name, "SHA-384");
        assert_eq!(res_384.signature_algorithm_name, "ECDSA-SHA384");

        let mut bad_384 = cms_384;
        let last = bad_384.len() - 1;
        bad_384[last] ^= 0x01;
        let bad_res_384 = verify_cms_detached_slices(&bad_384, &[payload], &store_384, None)
            .expect("bad p384 cms");
        assert!(!bad_res_384.signature_valid);
    }
}
