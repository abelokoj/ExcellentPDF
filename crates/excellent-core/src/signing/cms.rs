//! RFC 5652 CMS/PKCS#7 `SignedData` builder and detached verifier with PAdES `signing-certificate-v2`
//! and RFC 3161 `id-aa-timeStampToken` attributes (plan O.14, O.15, O.20).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use super::der::{
    AlgParams, ChainTrust, OID_CONTENT_TYPE, OID_ED25519, OID_MESSAGE_DIGEST, OID_PKCS7_DATA,
    OID_PKCS7_SIGNED_DATA, OID_SHA256, OID_SIGNING_CERT_V2, OID_SIGNING_TIME, OID_TIMESTAMP_TOKEN,
    SigningError, TAG_INTEGER, TAG_OCTET_STRING, TAG_SEQUENCE, TAG_SET, TrustStore,
    X509Certificate, encode_alg_id, encode_context_constructed, encode_octet_string, encode_oid,
    encode_sequence, encode_set, encode_tlv, encode_u64, encode_utc_time, parse_der_prefix,
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
    pub(crate) digest_algorithm_oid: Vec<u32>,
    pub(crate) signature_algorithm_oid: Vec<u32>,
    pub(crate) message_digest: Vec<u8>,
    pub(crate) signing_time: Option<i64>,
    pub(crate) signing_cert_v2_hash: Option<Vec<u8>>,
    pub(crate) signed_attrs_set_der: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    pub(crate) timestamp_tokens: Vec<Vec<u8>>,
}

/// Outcome of verifying a detached CMS signature against a digest and [`TrustStore`].
#[derive(Debug, Clone)]
pub(crate) struct CmsVerification {
    pub(crate) signer_cert: Option<X509Certificate>,
    pub(crate) chain_trust: ChainTrust,
    pub(crate) digest_valid: bool,
    pub(crate) signature_valid: bool,
    pub(crate) signing_time: Option<i64>,
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

    let sid_children = si_fields[1].children()?;
    if sid_children.len() < 2 {
        return Err(SigningError::DerFormat(
            "SignerIdentifier must be IssuerAndSerialNumber".to_owned(),
        ));
    }
    let signer_issuer_der = sid_children[0].full.to_vec();
    let signer_serial = sid_children[1].as_integer_bytes()?.to_vec();

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
                && let Some(hash_node) = ess_parts.first()
            {
                signing_cert_v2_hash = Some(hash_node.content.to_vec());
            }
        }
        idx += 1;
    }

    let signature_algorithm_oid = si_fields
        .get(idx)
        .ok_or_else(|| SigningError::DerFormat("missing signatureAlgorithm".to_owned()))?
        .children()?
        .first()
        .ok_or_else(|| SigningError::DerFormat("missing signatureAlgorithm OID".to_owned()))?
        .as_oid()?;
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
        digest_algorithm_oid,
        signature_algorithm_oid,
        message_digest,
        signing_time,
        signing_cert_v2_hash,
        signed_attrs_set_der,
        signature,
        timestamp_tokens,
    })
}

/// Verifies a detached CMS `SignedData` payload against `expected_digest_sha256` and `trust_store`.
pub(crate) fn verify_cms_detached(
    der: &[u8],
    expected_digest_sha256: &[u8; 32],
    trust_store: &TrustStore,
    check_time: Option<i64>,
) -> Result<CmsVerification, SigningError> {
    let parsed = parse_cms_signed_data(der)?;
    let digest_algorithm_name = oid_display_name(&parsed.digest_algorithm_oid);
    let signature_algorithm_name = oid_display_name(&parsed.signature_algorithm_oid);

    let signer_cert = parsed
        .certificates
        .iter()
        .find(|c| c.issuer_der == parsed.signer_issuer_der && c.serial == parsed.signer_serial)
        .cloned()
        .or_else(|| parsed.certificates.first().cloned());

    let digest_valid = parsed.digest_algorithm_oid == OID_SHA256
        && parsed.message_digest.as_slice() == expected_digest_sha256;

    let Some(ref cert) = signer_cert else {
        return Ok(CmsVerification {
            signer_cert: None,
            chain_trust: ChainTrust::UnknownIssuer,
            digest_valid,
            signature_valid: false,
            signing_time: parsed.signing_time,
            timestamp_tokens: parsed.timestamp_tokens,
            digest_algorithm_name,
            signature_algorithm_name,
            failure_reason: Some("CMS SignedData contains no signer certificate".to_owned()),
        });
    };

    if let Some(ref cert_hash) = parsed.signing_cert_v2_hash
        && cert_hash.as_slice() != cert.sha256_digest()
    {
        return Ok(CmsVerification {
            signer_cert: Some(cert.clone()),
            chain_trust: ChainTrust::Tampered(
                "signing-certificate-v2 digest does not match signer certificate".to_owned(),
            ),
            digest_valid,
            signature_valid: false,
            signing_time: parsed.signing_time,
            timestamp_tokens: parsed.timestamp_tokens,
            digest_algorithm_name,
            signature_algorithm_name,
            failure_reason: Some("signing-certificate-v2 mismatch".to_owned()),
        });
    }

    let mut signature_valid = false;
    let mut failure_reason = if digest_valid {
        None
    } else {
        Some("PDF ByteRange SHA-256 digest mismatch".to_owned())
    };

    if parsed.signature_algorithm_oid == OID_ED25519 && cert.public_key_oid == OID_ED25519 {
        let vk = VerifyingKey::try_from(cert.public_key_bytes.as_slice())
            .map_err(|err| SigningError::Crypto(format!("invalid signer public key: {err}")))?;
        if let Ok(sig) = Signature::from_slice(&parsed.signature) {
            if vk.verify(&parsed.signed_attrs_set_der, &sig).is_ok() {
                signature_valid = true;
            } else {
                failure_reason = Some("Ed25519 CMS signature verification failed".to_owned());
            }
        } else {
            failure_reason = Some("malformed Ed25519 signature bytes".to_owned());
        }
    } else {
        failure_reason = Some("unsupported CMS signature algorithm".to_owned());
    }

    let chain_trust = trust_store.evaluate_chain(cert, &parsed.certificates, check_time);

    Ok(CmsVerification {
        signer_cert: Some(cert.clone()),
        chain_trust,
        digest_valid,
        signature_valid,
        signing_time: parsed.signing_time,
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
    } else if oid == OID_ED25519 {
        "Ed25519".to_owned()
    } else {
        oid.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
    }
}
