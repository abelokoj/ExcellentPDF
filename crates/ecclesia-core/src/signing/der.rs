//! ASN.1 DER encoder/decoder, X.509 v3 certificate parser/verifier (RSA PKCS#1 v1.5, RSA-PSS,
//! ECDSA P-256/P-384, and Ed25519), and trust store (plan O.14, O.15, O.20).

use std::cmp::Ordering;
use std::fmt;
use std::io;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::forms::pdf_codec::PdfCodecError;

/// DER universal tags.
pub(crate) const TAG_BOOLEAN: u8 = 0x01;
pub(crate) const TAG_INTEGER: u8 = 0x02;
pub(crate) const TAG_BIT_STRING: u8 = 0x03;
pub(crate) const TAG_OCTET_STRING: u8 = 0x04;
pub(crate) const TAG_NULL: u8 = 0x05;
pub(crate) const TAG_OID: u8 = 0x06;
pub(crate) const TAG_UTF8_STRING: u8 = 0x0C;
pub(crate) const TAG_PRINTABLE_STRING: u8 = 0x13;
pub(crate) const TAG_UTC_TIME: u8 = 0x17;
pub(crate) const TAG_GENERALIZED_TIME: u8 = 0x18;
pub(crate) const TAG_SEQUENCE: u8 = 0x30;
pub(crate) const TAG_SET: u8 = 0x31;

/// Common OIDs used by X.509, CMS/PKCS#7, PAdES, and RFC 3161.
pub(crate) const OID_SHA256: &[u32] = &[2, 16, 840, 1, 101, 3, 4, 2, 1];
pub(crate) const OID_SHA384: &[u32] = &[2, 16, 840, 1, 101, 3, 4, 2, 2];
pub(crate) const OID_SHA512: &[u32] = &[2, 16, 840, 1, 101, 3, 4, 2, 3];
pub(crate) const OID_ED25519: &[u32] = &[1, 3, 101, 112];
pub(crate) const OID_RSA_ENCRYPTION: &[u32] = &[1, 2, 840, 113_549, 1, 1, 1];
pub(crate) const OID_MGF1: &[u32] = &[1, 2, 840, 113_549, 1, 1, 8];
pub(crate) const OID_RSASSA_PSS: &[u32] = &[1, 2, 840, 113_549, 1, 1, 10];
pub(crate) const OID_SHA256_WITH_RSA: &[u32] = &[1, 2, 840, 113_549, 1, 1, 11];
pub(crate) const OID_SHA384_WITH_RSA: &[u32] = &[1, 2, 840, 113_549, 1, 1, 12];
pub(crate) const OID_SHA512_WITH_RSA: &[u32] = &[1, 2, 840, 113_549, 1, 1, 13];
pub(crate) const OID_EC_PUBLIC_KEY: &[u32] = &[1, 2, 840, 10_045, 2, 1];
pub(crate) const OID_SECP256R1: &[u32] = &[1, 2, 840, 10_045, 3, 1, 7];
pub(crate) const OID_SECP384R1: &[u32] = &[1, 3, 132, 0, 34];
pub(crate) const OID_ECDSA_WITH_SHA256: &[u32] = &[1, 2, 840, 10_045, 4, 3, 2];
pub(crate) const OID_ECDSA_WITH_SHA384: &[u32] = &[1, 2, 840, 10_045, 4, 3, 3];
pub(crate) const OID_ECDSA_WITH_SHA512: &[u32] = &[1, 2, 840, 10_045, 4, 3, 4];
pub(crate) const OID_PKCS7_DATA: &[u32] = &[1, 2, 840, 113_549, 1, 7, 1];
pub(crate) const OID_PKCS7_SIGNED_DATA: &[u32] = &[1, 2, 840, 113_549, 1, 7, 2];
pub(crate) const OID_CONTENT_TYPE: &[u32] = &[1, 2, 840, 113_549, 1, 9, 3];
pub(crate) const OID_MESSAGE_DIGEST: &[u32] = &[1, 2, 840, 113_549, 1, 9, 4];
pub(crate) const OID_SIGNING_TIME: &[u32] = &[1, 2, 840, 113_549, 1, 9, 5];
pub(crate) const OID_SIGNING_CERT_V2: &[u32] = &[1, 2, 840, 113_549, 1, 9, 16, 2, 47];
pub(crate) const OID_TIMESTAMP_TOKEN: &[u32] = &[1, 2, 840, 113_549, 1, 9, 16, 2, 14];
pub(crate) const OID_TST_INFO: &[u32] = &[1, 2, 840, 113_549, 1, 9, 16, 1, 4];
pub(crate) const OID_TSA_POLICY_DEFAULT: &[u32] = &[1, 3, 6, 1, 4, 1, 55555, 1, 1];
pub(crate) const OID_CN: &[u32] = &[2, 5, 4, 3];
pub(crate) const OID_ORG: &[u32] = &[2, 5, 4, 10];
pub(crate) const OID_COUNTRY: &[u32] = &[2, 5, 4, 6];
pub(crate) const OID_BASIC_CONSTRAINTS: &[u32] = &[2, 5, 29, 19];
pub(crate) const OID_KEY_USAGE: &[u32] = &[2, 5, 29, 15];
pub(crate) const OID_EXT_KEY_USAGE: &[u32] = &[2, 5, 29, 37];
pub(crate) const OID_TIME_STAMPING: &[u32] = &[1, 3, 6, 1, 5, 5, 7, 3, 8];
pub(crate) const OID_SUBJECT_KEY_ID: &[u32] = &[2, 5, 29, 14];

/// Errors produced during PDF signing, CMS/DER parsing, verification, or PAdES LTV operations.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum SigningError {
    /// Underlying I/O error.
    Io(io::Error),
    /// MuPDF engine error.
    Mupdf(ecclesia_mupdf::MuError),
    /// Malformed PDF structure or ByteRange layout.
    PdfStructure(String),
    /// Malformed ASN.1 DER, CMS, X.509, or RFC 3161 payload.
    DerFormat(String),
    /// Cryptographic signature or digest error.
    Crypto(String),
    /// Network or remote TSA error.
    Network(String),
    /// Requested action requires explicit user consent.
    ConsentRequired,
}

impl fmt::Display for SigningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "signing I/O error: {err}"),
            Self::Mupdf(err) => write!(f, "signing engine error: {err}"),
            Self::PdfStructure(msg) => write!(f, "invalid PDF signature structure: {msg}"),
            Self::DerFormat(msg) => write!(f, "invalid DER/CMS structure: {msg}"),
            Self::Crypto(msg) => write!(f, "cryptographic error: {msg}"),
            Self::Network(msg) => write!(f, "timestamp network error: {msg}"),
            Self::ConsentRequired => f.write_str("online timestamping requires user consent"),
        }
    }
}

impl std::error::Error for SigningError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Mupdf(err) => Some(err),
            Self::PdfStructure(_)
            | Self::DerFormat(_)
            | Self::Crypto(_)
            | Self::Network(_)
            | Self::ConsentRequired => None,
        }
    }
}

impl From<io::Error> for SigningError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ecclesia_mupdf::MuError> for SigningError {
    fn from(err: ecclesia_mupdf::MuError) -> Self {
        Self::Mupdf(err)
    }
}

impl From<PdfCodecError> for SigningError {
    fn from(err: PdfCodecError) -> Self {
        Self::PdfStructure(err.0)
    }
}

// ---- ASN.1 DER encoder ----

pub(crate) fn encode_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    encode_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

fn encode_length(len: usize, out: &mut Vec<u8>) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let mut buf = [0u8; 8];
        let mut n = len;
        let mut count = 0usize;
        while n > 0 {
            buf[7 - count] = (n & 0xFF) as u8;
            n >>= 8;
            count += 1;
        }
        out.push(0x80 | (count as u8));
        out.extend_from_slice(&buf[8 - count..]);
    }
}

pub(crate) fn encode_sequence(items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.iter().flat_map(|v| v.iter().copied()).collect();
    encode_tlv(TAG_SEQUENCE, &body)
}

pub(crate) fn encode_set(items: &[Vec<u8>]) -> Vec<u8> {
    // DER SET elements are sorted lexicographically by encoded bytes.
    let mut sorted = items.to_vec();
    sorted.sort();
    let body: Vec<u8> = sorted.into_iter().flatten().collect();
    encode_tlv(TAG_SET, &body)
}

pub(crate) fn encode_context_constructed(index: u8, items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.iter().flat_map(|v| v.iter().copied()).collect();
    encode_tlv(0xA0 | (index & 0x1F), &body)
}

pub(crate) fn encode_u64(val: u64) -> Vec<u8> {
    if val == 0 {
        return encode_tlv(TAG_INTEGER, &[0]);
    }
    let bytes = val.to_be_bytes();
    let first_non_zero = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    let slice = &bytes[first_non_zero..];
    if slice[0] & 0x80 != 0 {
        let mut padded = Vec::with_capacity(slice.len() + 1);
        padded.push(0);
        padded.extend_from_slice(slice);
        encode_tlv(TAG_INTEGER, &padded)
    } else {
        encode_tlv(TAG_INTEGER, slice)
    }
}

pub(crate) fn encode_boolean_true() -> Vec<u8> {
    encode_tlv(TAG_BOOLEAN, &[0xFF])
}

pub(crate) fn encode_null() -> Vec<u8> {
    encode_tlv(TAG_NULL, &[])
}

pub(crate) fn encode_octet_string(bytes: &[u8]) -> Vec<u8> {
    encode_tlv(TAG_OCTET_STRING, bytes)
}

pub(crate) fn encode_bit_string(bytes: &[u8], unused_bits: u8) -> Vec<u8> {
    let mut content = Vec::with_capacity(bytes.len() + 1);
    content.push(unused_bits);
    content.extend_from_slice(bytes);
    encode_tlv(TAG_BIT_STRING, &content)
}

pub(crate) fn encode_utf8_string(text: &str) -> Vec<u8> {
    encode_tlv(TAG_UTF8_STRING, text.as_bytes())
}

pub(crate) fn encode_printable_string(text: &str) -> Vec<u8> {
    encode_tlv(TAG_PRINTABLE_STRING, text.as_bytes())
}

pub(crate) fn encode_oid(arcs: &[u32]) -> Vec<u8> {
    if arcs.len() < 2 {
        return encode_tlv(TAG_OID, &[]);
    }
    let mut out = Vec::new();
    out.push((arcs[0] * 40 + arcs[1]) as u8);
    for &arc in &arcs[2..] {
        encode_base128(arc, &mut out);
    }
    encode_tlv(TAG_OID, &out)
}

fn encode_base128(mut v: u32, out: &mut Vec<u8>) {
    let mut buf = [0u8; 5];
    let mut i = 5usize;
    loop {
        i -= 1;
        buf[i] = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for &b in buf.iter().take(4).skip(i) {
        out.push(b | 0x80);
    }
    out.push(buf[4]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlgParams {
    None,
    Null,
}

pub(crate) fn encode_alg_id(oid: &[u32], params: AlgParams) -> Vec<u8> {
    match params {
        AlgParams::Null => encode_sequence(&[encode_oid(oid), encode_null()]),
        AlgParams::None => encode_sequence(&[encode_oid(oid)]),
    }
}

pub(crate) fn encode_utc_time(unix_secs: i64) -> Vec<u8> {
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(unix_secs);
    let yy = year.rem_euclid(100);
    let s = format!("{yy:02}{month:02}{day:02}{hour:02}{min:02}{sec:02}Z");
    encode_tlv(TAG_UTC_TIME, s.as_bytes())
}

pub(crate) fn encode_generalized_time(unix_secs: i64) -> Vec<u8> {
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(unix_secs);
    let s = format!("{year:04}{month:02}{day:02}{hour:02}{min:02}{sec:02}Z");
    encode_tlv(TAG_GENERALIZED_TIME, s.as_bytes())
}

// ---- ASN.1 DER decoder ----

#[derive(Debug, Clone, Copy)]
pub(crate) struct DerNode<'a> {
    pub(crate) tag: u8,
    pub(crate) full: &'a [u8],
    pub(crate) content: &'a [u8],
}

pub(crate) fn parse_der(bytes: &[u8]) -> Result<DerNode<'_>, SigningError> {
    let (node, consumed) = parse_der_prefix(bytes)?;
    if consumed != bytes.len() {
        return Err(SigningError::DerFormat(format!(
            "trailing DER bytes ({} remaining)",
            bytes.len() - consumed
        )));
    }
    Ok(node)
}

pub(crate) fn parse_der_prefix(bytes: &[u8]) -> Result<(DerNode<'_>, usize), SigningError> {
    let &tag = bytes
        .first()
        .ok_or_else(|| SigningError::DerFormat("empty DER input".to_owned()))?;
    let &len_byte = bytes
        .get(1)
        .ok_or_else(|| SigningError::DerFormat("truncated DER length".to_owned()))?;
    let (len, hdr_len) = if len_byte < 0x80 {
        (usize::from(len_byte), 2usize)
    } else {
        let num_octets = usize::from(len_byte & 0x7F);
        if num_octets == 0 || num_octets > 4 || bytes.len() < 2 + num_octets {
            return Err(SigningError::DerFormat(
                "invalid DER length octets".to_owned(),
            ));
        }
        let mut len = 0usize;
        for &b in &bytes[2..2 + num_octets] {
            len = (len << 8) | usize::from(b);
        }
        (len, 2 + num_octets)
    };
    let end = hdr_len
        .checked_add(len)
        .ok_or_else(|| SigningError::DerFormat("DER length overflow".to_owned()))?;
    if end > bytes.len() {
        return Err(SigningError::DerFormat("truncated DER value".to_owned()));
    }
    Ok((
        DerNode {
            tag,
            full: &bytes[..end],
            content: &bytes[hdr_len..end],
        },
        end,
    ))
}

impl<'a> DerNode<'a> {
    pub(crate) fn children(&self) -> Result<Vec<DerNode<'a>>, SigningError> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < self.content.len() {
            let (child, used) = parse_der_prefix(&self.content[pos..])?;
            out.push(child);
            pos += used;
        }
        Ok(out)
    }

    pub(crate) fn as_oid(&self) -> Result<Vec<u32>, SigningError> {
        if self.tag != TAG_OID || self.content.is_empty() {
            return Err(SigningError::DerFormat("expected OID".to_owned()));
        }
        let first = u32::from(self.content[0]);
        let mut arcs = vec![first / 40, first % 40];
        let mut val = 0u32;
        for &b in &self.content[1..] {
            let shifted = val
                .checked_shl(7)
                .ok_or_else(|| SigningError::DerFormat("OID arc overflow".to_owned()))?;
            val = shifted | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                arcs.push(val);
                val = 0;
            }
        }
        Ok(arcs)
    }

    pub(crate) fn as_integer_bytes(&self) -> Result<&'a [u8], SigningError> {
        if self.tag != TAG_INTEGER {
            return Err(SigningError::DerFormat("expected INTEGER".to_owned()));
        }
        Ok(self.content)
    }

    pub(crate) fn as_bit_string_bytes(&self) -> Result<&'a [u8], SigningError> {
        if self.tag != TAG_BIT_STRING || self.content.is_empty() {
            return Err(SigningError::DerFormat("expected BIT STRING".to_owned()));
        }
        Ok(&self.content[1..])
    }

    pub(crate) fn as_text(&self) -> Result<String, SigningError> {
        Ok(String::from_utf8_lossy(self.content).into_owned())
    }

    pub(crate) fn as_unix_time(&self) -> Result<i64, SigningError> {
        let s = std::str::from_utf8(self.content)
            .map_err(|_| SigningError::DerFormat("invalid time ASCII".to_owned()))?;
        if !s.is_ascii() {
            return Err(SigningError::DerFormat("non-ASCII time value".to_owned()));
        }
        let s = s.strip_suffix('Z').unwrap_or(s);
        let (year, rest) = if self.tag == TAG_UTC_TIME && s.len() >= 12 {
            let yy: i32 = s[..2]
                .parse()
                .map_err(|_| SigningError::DerFormat("bad UTCTime year".to_owned()))?;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (year, &s[2..])
        } else if self.tag == TAG_GENERALIZED_TIME && s.len() >= 14 {
            let year: i32 = s[..4]
                .parse()
                .map_err(|_| SigningError::DerFormat("bad GeneralizedTime year".to_owned()))?;
            (year, &s[4..])
        } else {
            return Err(SigningError::DerFormat(format!("unsupported time {s:?}")));
        };
        let month: u32 = rest[0..2].parse().unwrap_or(1);
        let day: u32 = rest[2..4].parse().unwrap_or(1);
        let hour: u32 = rest[4..6].parse().unwrap_or(0);
        let min: u32 = rest[6..8].parse().unwrap_or(0);
        let sec: u32 = rest[8..10].parse().unwrap_or(0);
        Ok(ymdhms_to_unix(year, month, day, hour, min, sec))
    }
}

// ---- X.509 v3 Certificate model ----

/// X.509 Distinguished Name (`CN`, `O`, `C`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct DistinguishedName {
    /// Common Name (`CN`).
    pub common_name: String,
    /// Organization (`O`).
    pub organization: Option<String>,
    /// Country code (`C`).
    pub country: Option<String>,
}

impl DistinguishedName {
    /// Creates a name with only a Common Name.
    #[must_use]
    pub fn with_common_name(cn: impl Into<String>) -> Self {
        Self {
            common_name: cn.into(),
            organization: None,
            country: None,
        }
    }

    /// Encodes the X.509 `Name` (`SEQUENCE OF RelativeDistinguishedName`) in DER.
    #[must_use]
    pub fn to_der(&self) -> Vec<u8> {
        let mut rdns = Vec::new();
        if let Some(ref c) = self.country {
            rdns.push(encode_set(&[encode_sequence(&[
                encode_oid(OID_COUNTRY),
                encode_printable_string(c),
            ])]));
        }
        if let Some(ref o) = self.organization {
            rdns.push(encode_set(&[encode_sequence(&[
                encode_oid(OID_ORG),
                encode_utf8_string(o),
            ])]));
        }
        rdns.push(encode_set(&[encode_sequence(&[
            encode_oid(OID_CN),
            encode_utf8_string(&self.common_name),
        ])]));
        encode_sequence(&rdns)
    }

    fn from_der_node(node: DerNode<'_>) -> Result<Self, SigningError> {
        let mut dn = Self::default();
        for rdn in node.children()? {
            for atv in rdn.children()? {
                let parts = atv.children()?;
                if parts.len() >= 2
                    && let Ok(oid) = parts[0].as_oid()
                {
                    let val = parts[1].as_text()?;
                    if oid == OID_CN {
                        dn.common_name = val;
                    } else if oid == OID_ORG {
                        dn.organization = Some(val);
                    } else if oid == OID_COUNTRY {
                        dn.country = Some(val);
                    }
                }
            }
        }
        Ok(dn)
    }
}

/// Role of a generated X.509 v3 certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CertRole {
    /// End-entity document signing certificate.
    #[default]
    EndEntity,
    /// Certificate authority capable of signing child certificates.
    CertificateAuthority,
    /// RFC 3161 Time-Stamping Authority (`id-kp-timeStamping`).
    TimestampAuthority,
}

/// Validity window in Unix seconds (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CertValidity {
    /// Earliest valid Unix timestamp (seconds).
    pub not_before: i64,
    /// Latest valid Unix timestamp (seconds).
    pub not_after: i64,
}

impl Default for CertValidity {
    fn default() -> Self {
        // 2025-01-01 to 2035-01-01
        Self {
            not_before: 1_735_689_600,
            not_after: 2_051_222_400,
        }
    }
}

/// Parsed X.509 v3 certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X509Certificate {
    /// Full DER-encoded certificate bytes.
    pub raw_der: Vec<u8>,
    /// Raw DER-encoded `TBSCertificate` bytes covered by the signature.
    pub tbs_der: Vec<u8>,
    /// Raw DER `Name` bytes of the issuer (used for CMS `IssuerAndSerialNumber`).
    pub issuer_der: Vec<u8>,
    /// Raw DER `Name` bytes of the subject (used for exact issuer/subject chain matching).
    pub(crate) subject_der: Vec<u8>,
    /// Big-endian serial number bytes.
    pub serial: Vec<u8>,
    /// Parsed issuer distinguished name.
    pub issuer: DistinguishedName,
    /// Parsed subject distinguished name.
    pub subject: DistinguishedName,
    /// Validity start (Unix UTC seconds).
    pub not_before: i64,
    /// Validity end (Unix UTC seconds).
    pub not_after: i64,
    /// SubjectPublicKeyInfo algorithm OID.
    pub public_key_oid: Vec<u32>,
    /// Raw DER parameters from `SubjectPublicKeyInfo.algorithm`, if present (e.g., EC named curve OID).
    pub(crate) public_key_params_der: Option<Vec<u8>>,
    /// Subject public key bytes.
    pub public_key_bytes: Vec<u8>,
    /// Certificate signature algorithm OID.
    pub signature_algorithm_oid: Vec<u32>,
    /// Raw DER parameters from `signatureAlgorithm`, if present (e.g., `RSASSA-PSS-params`).
    pub(crate) signature_algorithm_params_der: Option<Vec<u8>>,
    /// Certificate signature bytes.
    pub signature_bytes: Vec<u8>,
    /// SubjectKeyIdentifier octets from certificate extensions, if present.
    pub(crate) subject_key_id: Option<Vec<u8>>,
    /// Whether `BasicConstraints` sets `cA = TRUE`.
    pub is_ca: bool,
    /// Whether `ExtendedKeyUsage` includes `id-kp-timeStamping`.
    pub is_tsa: bool,
}

impl X509Certificate {
    /// Parses a DER-encoded X.509 certificate.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::DerFormat`] if the bytes are not a valid X.509 certificate.
    pub fn from_der(der: &[u8]) -> Result<Self, SigningError> {
        let root = parse_der(der)?;
        let top = root.children()?;
        if top.len() < 3 {
            return Err(SigningError::DerFormat(
                "X.509 certificate requires 3 elements".to_owned(),
            ));
        }
        let tbs = top[0];
        let sig_alg_children = top[1].children()?;
        let signature_algorithm_oid = sig_alg_children
            .first()
            .ok_or_else(|| SigningError::DerFormat("missing sig algorithm OID".to_owned()))?
            .as_oid()?;
        let signature_algorithm_params_der = sig_alg_children.get(1).map(|n| n.full.to_vec());
        let signature_bytes = top[2].as_bit_string_bytes()?.to_vec();

        let tbs_children = tbs.children()?;
        let mut idx = 0usize;
        if tbs_children.first().is_some_and(|n| n.tag == 0xA0) {
            idx += 1;
        }
        if tbs_children.len() < idx + 6 {
            return Err(SigningError::DerFormat(
                "truncated TBSCertificate".to_owned(),
            ));
        }
        let serial = tbs_children[idx].as_integer_bytes()?.to_vec();
        let issuer_node = tbs_children[idx + 2];
        let issuer_der = issuer_node.full.to_vec();
        let issuer = DistinguishedName::from_der_node(issuer_node)?;

        let validity_children = tbs_children[idx + 3].children()?;
        if validity_children.len() < 2 {
            return Err(SigningError::DerFormat(
                "invalid certificate validity".to_owned(),
            ));
        }
        let not_before = validity_children[0].as_unix_time()?;
        let not_after = validity_children[1].as_unix_time()?;

        let subject_node = tbs_children[idx + 4];
        let subject_der = subject_node.full.to_vec();
        let subject = DistinguishedName::from_der_node(subject_node)?;

        let spki_children = tbs_children[idx + 5].children()?;
        if spki_children.len() < 2 {
            return Err(SigningError::DerFormat(
                "invalid SubjectPublicKeyInfo".to_owned(),
            ));
        }
        let spki_alg_children = spki_children[0].children()?;
        let public_key_oid = spki_alg_children
            .first()
            .ok_or_else(|| SigningError::DerFormat("missing SPKI OID".to_owned()))?
            .as_oid()?;
        let public_key_params_der = spki_alg_children.get(1).map(|n| n.full.to_vec());
        let public_key_bytes = spki_children[1].as_bit_string_bytes()?.to_vec();

        let mut is_ca = false;
        let mut is_tsa = false;
        let mut subject_key_id = None;
        for node in &tbs_children[idx + 6..] {
            if node.tag == 0xA3
                && let Ok(exts_seq) = node.children()
                && let Some(seq) = exts_seq.first()
            {
                for ext in seq.children()? {
                    let ext_parts = ext.children()?;
                    if ext_parts.is_empty() {
                        continue;
                    }
                    let oid = ext_parts[0].as_oid()?;
                    let octets = ext_parts
                        .last()
                        .ok_or_else(|| SigningError::DerFormat("empty extension".to_owned()))?
                        .content;
                    if oid == OID_BASIC_CONSTRAINTS
                        && let Ok(bc) = parse_der(octets)
                        && let Ok(items) = bc.children()
                        && items.first().is_some_and(|b| {
                            b.tag == TAG_BOOLEAN && b.content.first() == Some(&0xFF)
                        })
                    {
                        is_ca = true;
                    } else if oid == OID_EXT_KEY_USAGE
                        && let Ok(eku) = parse_der(octets)
                    {
                        for u in eku.children()? {
                            if u.as_oid().ok().as_deref() == Some(OID_TIME_STAMPING) {
                                is_tsa = true;
                            }
                        }
                    } else if oid == OID_SUBJECT_KEY_ID
                        && let Ok(skid_node) = parse_der(octets)
                        && skid_node.tag == TAG_OCTET_STRING
                    {
                        subject_key_id = Some(skid_node.content.to_vec());
                    }
                }
            }
        }

        Ok(Self {
            raw_der: der.to_vec(),
            tbs_der: tbs.full.to_vec(),
            issuer_der,
            subject_der,
            serial,
            issuer,
            subject,
            not_before,
            not_after,
            public_key_oid,
            public_key_params_der,
            public_key_bytes,
            signature_algorithm_oid,
            signature_algorithm_params_der,
            signature_bytes,
            subject_key_id,
            is_ca,
            is_tsa,
        })
    }

    /// Issues an Ed25519 X.509 v3 certificate signed by `issuer_key` (self-signed if `issuer_cert` is `None`).
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Crypto`] if building or encoding fails.
    pub fn issue_ed25519(
        subject: &DistinguishedName,
        subject_key: &VerifyingKey,
        issuer_cert: Option<&Self>,
        issuer_key: &SigningKey,
        validity: CertValidity,
        role: CertRole,
        serial: u64,
    ) -> Result<Self, SigningError> {
        let issuer_dn = issuer_cert.map_or(subject, |c| &c.subject);
        let version = encode_context_constructed(0, &[encode_u64(2)]);
        let serial_der = encode_u64(serial);
        let sig_alg = encode_alg_id(OID_ED25519, AlgParams::None);
        let issuer_der = issuer_dn.to_der();
        let validity_der = encode_sequence(&[
            encode_utc_time(validity.not_before),
            encode_utc_time(validity.not_after),
        ]);
        let subject_der = subject.to_der();
        let spki = encode_sequence(&[
            encode_alg_id(OID_ED25519, AlgParams::None),
            encode_bit_string(subject_key.as_bytes(), 0),
        ]);

        let mut extensions = Vec::new();
        let bc_val = if role == CertRole::CertificateAuthority {
            encode_sequence(&[encode_boolean_true()])
        } else {
            encode_sequence(&[])
        };
        extensions.push(encode_sequence(&[
            encode_oid(OID_BASIC_CONSTRAINTS),
            encode_boolean_true(),
            encode_octet_string(&bc_val),
        ]));

        let key_usage_byte = match role {
            CertRole::CertificateAuthority => 0b0000_0110, // keyCertSign | cRLSign
            CertRole::EndEntity | CertRole::TimestampAuthority => 0b1100_0000, // digitalSignature | nonRepudiation
        };
        extensions.push(encode_sequence(&[
            encode_oid(OID_KEY_USAGE),
            encode_boolean_true(),
            encode_octet_string(&encode_bit_string(&[key_usage_byte], 0)),
        ]));

        if role == CertRole::TimestampAuthority {
            let eku = encode_sequence(&[encode_oid(OID_TIME_STAMPING)]);
            extensions.push(encode_sequence(&[
                encode_oid(OID_EXT_KEY_USAGE),
                encode_boolean_true(),
                encode_octet_string(&eku),
            ]));
        }

        let skid: [u8; 32] = Sha256::digest(subject_key.as_bytes()).into();
        extensions.push(encode_sequence(&[
            encode_oid(OID_SUBJECT_KEY_ID),
            encode_octet_string(&encode_octet_string(&skid[..20])),
        ]));

        let exts_wrapper = encode_context_constructed(3, &[encode_sequence(&extensions)]);

        let tbs_der = encode_sequence(&[
            version,
            serial_der,
            sig_alg.clone(),
            issuer_der,
            validity_der,
            subject_der,
            spki,
            exts_wrapper,
        ]);

        let sig = issuer_key.sign(&tbs_der);
        let cert_der = encode_sequence(&[tbs_der, sig_alg, encode_bit_string(&sig.to_bytes(), 0)]);
        Self::from_der(&cert_der)
    }

    /// SHA-256 fingerprint of the DER-encoded certificate.
    #[must_use]
    pub fn sha256_digest(&self) -> [u8; 32] {
        Sha256::digest(&self.raw_der).into()
    }

    /// Big-endian hexadecimal serial number.
    #[must_use]
    pub fn serial_hex(&self) -> String {
        hex_upper(&self.serial)
    }

    /// Verifies that `self`'s signature was produced by `issuer`'s public key.
    ///
    /// Supports RSA PKCS#1 v1.5 (SHA-256 / SHA-384 / SHA-512), RSA-PSS (MGF1-SHA256 / SHA-384 /
    /// SHA-512), ECDSA (P-256 / P-384), and Ed25519.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Crypto`] if the issuer mismatch, unsupported key, or invalid signature is detected.
    pub fn verify_issued_by(&self, issuer: &Self) -> Result<(), SigningError> {
        if self.issuer_der != issuer.subject_der && self.issuer != issuer.subject {
            return Err(SigningError::Crypto(
                "certificate issuer does not match parent subject".to_owned(),
            ));
        }
        verify_signature_over_slices(
            issuer,
            &self.signature_algorithm_oid,
            self.signature_algorithm_params_der.as_deref(),
            None,
            &[&self.tbs_der],
            &self.signature_bytes,
        )
    }
}

/// Outcome of evaluating a certificate chain against a [`TrustStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainTrust {
    /// Chain verified up to a trusted root anchor.
    Trusted {
        /// Subject common name of the matched trust anchor.
        anchor_common_name: String,
    },
    /// Signatures are intact, but no trusted root anchor was found.
    UnknownIssuer,
    /// A certificate signature failed or a certificate is outside its validity window.
    Tampered(String),
}

/// Collection of trusted root / CA certificates for signature validation (plan O.15).
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    anchors: Vec<X509Certificate>,
}

impl TrustStore {
    /// Creates an empty [`TrustStore`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a [`TrustStore`] pre-populated with `anchors`.
    #[must_use]
    pub fn with_anchors(anchors: impl IntoIterator<Item = X509Certificate>) -> Self {
        Self {
            anchors: anchors.into_iter().collect(),
        }
    }

    /// Adds a trusted certificate anchor.
    pub fn add_anchor(&mut self, cert: X509Certificate) {
        self.anchors.push(cert);
    }

    /// All configured trust anchors.
    #[must_use]
    pub fn anchors(&self) -> &[X509Certificate] {
        &self.anchors
    }

    /// Evaluates whether `leaf` chains to any anchor in `self` via `intermediates`.
    #[must_use]
    pub fn evaluate_chain(
        &self,
        leaf: &X509Certificate,
        intermediates: &[X509Certificate],
        check_time: Option<i64>,
    ) -> ChainTrust {
        if let Some(t) = check_time
            && (t < leaf.not_before || t > leaf.not_after)
        {
            return ChainTrust::Tampered("signer certificate outside validity window".to_owned());
        }

        let mut current = leaf;
        for _hop in 0..8 {
            if let Some(anchor) = self.anchors.iter().find(|a| a.raw_der == current.raw_der) {
                return ChainTrust::Trusted {
                    anchor_common_name: anchor.subject.common_name.clone(),
                };
            }
            if let Some(anchor) = self.anchors.iter().find(|a| a.subject == current.issuer) {
                return match current.verify_issued_by(anchor) {
                    Ok(()) => {
                        if let Some(t) = check_time
                            && (t < anchor.not_before || t > anchor.not_after)
                        {
                            return ChainTrust::Tampered(
                                "trust anchor outside validity window".to_owned(),
                            );
                        }
                        ChainTrust::Trusted {
                            anchor_common_name: anchor.subject.common_name.clone(),
                        }
                    }
                    Err(err) => ChainTrust::Tampered(err.to_string()),
                };
            }
            if current.issuer == current.subject {
                // Self-signed and not in anchors.
                if let Err(err) = current.verify_issued_by(current) {
                    return ChainTrust::Tampered(err.to_string());
                }
                return ChainTrust::UnknownIssuer;
            }
            let Some(next) = intermediates
                .iter()
                .find(|c| c.subject == current.issuer && c.raw_der != current.raw_der)
            else {
                return ChainTrust::UnknownIssuer;
            };
            if let Err(err) = current.verify_issued_by(next) {
                return ChainTrust::Tampered(err.to_string());
            }
            if let Some(t) = check_time
                && (t < next.not_before || t > next.not_after)
            {
                return ChainTrust::Tampered(
                    "intermediate certificate outside validity window".to_owned(),
                );
            }
            current = next;
        }
        ChainTrust::UnknownIssuer
    }
}

pub(crate) fn hex_upper(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02X}");
    }
    out
}

fn unix_to_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    // Civil-from-days algorithm (Howard Hinnant).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = (y + i64::from(m <= 2)) as i32;
    (year, m, d, hour, min, sec)
}

fn ymdhms_to_unix(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m_adj = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m_adj + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + i64::from(doe) - 719_468;
    days * 86_400 + i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec)
}

// ---- Pure-Rust cryptographic signature verification (RSA PKCS#1 v1.5, RSA-PSS, ECDSA P-256/P-384, Ed25519) ----

/// Supported SHA-2 digest algorithms for X.509, CMS, RSA, and ECDSA verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HashAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlgorithm {
    pub(crate) fn from_oid(oid: &[u32]) -> Option<Self> {
        if oid == OID_SHA256 {
            Some(Self::Sha256)
        } else if oid == OID_SHA384 {
            Some(Self::Sha384)
        } else if oid == OID_SHA512 {
            Some(Self::Sha512)
        } else {
            None
        }
    }

    pub(crate) const fn digest_len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    pub(crate) fn hash(self, data: &[u8]) -> Vec<u8> {
        self.hash_slices(&[data])
    }

    pub(crate) fn hash_slices(self, slices: &[&[u8]]) -> Vec<u8> {
        match self {
            Self::Sha256 => {
                let mut h = Sha256::new();
                for s in slices {
                    h.update(s);
                }
                h.finalize().to_vec()
            }
            Self::Sha384 => {
                let mut h = Sha384::new();
                for s in slices {
                    h.update(s);
                }
                h.finalize().to_vec()
            }
            Self::Sha512 => {
                let mut h = Sha512::new();
                for s in slices {
                    h.update(s);
                }
                h.finalize().to_vec()
            }
        }
    }
}

/// Verifies `signature` over the concatenation of `message_slices` under `cert`'s public key.
///
/// `cms_digest_hint` supplies the `SignerInfo.digestAlgorithm` hash when verifying CMS
/// signatures whose `signatureAlgorithm` uses bare `rsaEncryption` or `id-ecPublicKey`.
/// Pass `None` for X.509 certificate verification.
pub(crate) fn verify_signature_over_slices(
    cert: &X509Certificate,
    sig_alg_oid: &[u32],
    sig_alg_params_der: Option<&[u8]>,
    cms_digest_hint: Option<HashAlgorithm>,
    message_slices: &[&[u8]],
    signature: &[u8],
) -> Result<(), SigningError> {
    if sig_alg_oid == OID_ED25519 {
        if cert.public_key_oid != OID_ED25519 {
            return Err(SigningError::Crypto(
                "Ed25519 signature does not match certificate key type".to_owned(),
            ));
        }
        let vk = VerifyingKey::try_from(cert.public_key_bytes.as_slice())
            .map_err(|err| SigningError::Crypto(format!("invalid Ed25519 public key: {err}")))?;
        let sig = Signature::from_slice(signature)
            .map_err(|err| SigningError::Crypto(format!("invalid Ed25519 signature: {err}")))?;
        if message_slices.len() == 1 {
            return vk.verify(message_slices[0], &sig).map_err(|err| {
                SigningError::Crypto(format!("Ed25519 verification failed: {err}"))
            });
        }
        let total: usize = message_slices.iter().map(|s| s.len()).sum();
        let mut combined = Vec::with_capacity(total);
        for s in message_slices {
            combined.extend_from_slice(s);
        }
        return vk
            .verify(&combined, &sig)
            .map_err(|err| SigningError::Crypto(format!("Ed25519 verification failed: {err}")));
    }

    if sig_alg_oid == OID_SHA256_WITH_RSA
        || sig_alg_oid == OID_SHA384_WITH_RSA
        || sig_alg_oid == OID_SHA512_WITH_RSA
        || sig_alg_oid == OID_RSA_ENCRYPTION
    {
        if cert.public_key_oid != OID_RSA_ENCRYPTION {
            return Err(SigningError::Crypto(
                "RSA PKCS#1 v1.5 signature does not match certificate key type".to_owned(),
            ));
        }
        let hash_alg = if sig_alg_oid == OID_SHA256_WITH_RSA {
            HashAlgorithm::Sha256
        } else if sig_alg_oid == OID_SHA384_WITH_RSA {
            HashAlgorithm::Sha384
        } else if sig_alg_oid == OID_SHA512_WITH_RSA {
            HashAlgorithm::Sha512
        } else {
            cms_digest_hint.ok_or_else(|| {
                SigningError::Crypto(
                    "bare rsaEncryption requires a CMS digest algorithm".to_owned(),
                )
            })?
        };
        let key = parse_rsa_public_key(&cert.public_key_bytes)?;
        return verify_rsa_pkcs1_v15(&key, hash_alg, message_slices, signature);
    }

    if sig_alg_oid == OID_RSASSA_PSS {
        if cert.public_key_oid != OID_RSA_ENCRYPTION && cert.public_key_oid != OID_RSASSA_PSS {
            return Err(SigningError::Crypto(
                "RSA-PSS signature does not match certificate key type".to_owned(),
            ));
        }
        let key = parse_rsa_public_key(&cert.public_key_bytes)?;
        let params_der = sig_alg_params_der.or(cert.public_key_params_der.as_deref());
        let pss_params = parse_pss_params(params_der)?;
        return verify_rsa_pss(&key, pss_params, message_slices, signature);
    }

    if sig_alg_oid == OID_ECDSA_WITH_SHA256
        || sig_alg_oid == OID_ECDSA_WITH_SHA384
        || sig_alg_oid == OID_ECDSA_WITH_SHA512
        || sig_alg_oid == OID_EC_PUBLIC_KEY
    {
        if cert.public_key_oid != OID_EC_PUBLIC_KEY {
            return Err(SigningError::Crypto(
                "ECDSA signature does not match certificate key type".to_owned(),
            ));
        }
        let hash_alg = if sig_alg_oid == OID_ECDSA_WITH_SHA256 {
            HashAlgorithm::Sha256
        } else if sig_alg_oid == OID_ECDSA_WITH_SHA384 {
            HashAlgorithm::Sha384
        } else if sig_alg_oid == OID_ECDSA_WITH_SHA512 {
            HashAlgorithm::Sha512
        } else {
            cms_digest_hint.ok_or_else(|| {
                SigningError::Crypto(
                    "bare id-ecPublicKey requires a CMS digest algorithm".to_owned(),
                )
            })?
        };
        let curve_params = cert.public_key_params_der.as_deref().ok_or_else(|| {
            SigningError::Crypto("EC public key missing namedCurve parameters".to_owned())
        })?;
        let curve_oid = parse_der(curve_params)?.as_oid()?;
        return verify_ecdsa(
            &curve_oid,
            &cert.public_key_bytes,
            hash_alg,
            message_slices,
            signature,
        );
    }

    Err(SigningError::Crypto(format!(
        "unsupported signature algorithm OID {}",
        sig_alg_oid
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".")
    )))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn der_positive_uint(bytes: &[u8]) -> Result<&[u8], SigningError> {
    let &first = bytes
        .first()
        .ok_or_else(|| SigningError::DerFormat("empty DER INTEGER".to_owned()))?;
    if first & 0x80 != 0 {
        return Err(SigningError::DerFormat(
            "negative DER INTEGER where unsigned expected".to_owned(),
        ));
    }
    let pos = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    Ok(&bytes[pos..])
}

// ---- RSA big-integer Montgomery arithmetic & PKCS#1 v1.5 / PSS verification ----

/// DER-encoded `DigestInfo` prefixes for EMSA-PKCS1-v1_5 (RFC 8017 §9.2 Note 1).
const DIGEST_INFO_SHA256: &[u8] = &[
    0x30, 0x31, 0x30, 0x0D, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];
const DIGEST_INFO_SHA384: &[u8] = &[
    0x30, 0x41, 0x30, 0x0D, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05,
    0x00, 0x04, 0x30,
];
const DIGEST_INFO_SHA512: &[u8] = &[
    0x30, 0x51, 0x30, 0x0D, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05,
    0x00, 0x04, 0x40,
];

/// Parsed RSA public key `(n, e)` in little-endian `u64` limbs.
struct RsaPublicKey {
    modulus: Vec<u64>,
    exponent: Vec<u64>,
    modulus_bytes: usize,
    modulus_bits: usize,
}

/// Parsed `RSASSA-PSS-params` (RFC 4055 §3.1).
#[derive(Debug, Clone, Copy)]
struct RsaPssParams {
    hash_alg: HashAlgorithm,
    mgf_hash_alg: HashAlgorithm,
    salt_len: usize,
}

fn parse_rsa_public_key(spki_key_bytes: &[u8]) -> Result<RsaPublicKey, SigningError> {
    let seq = parse_der(spki_key_bytes)?;
    if seq.tag != TAG_SEQUENCE {
        return Err(SigningError::Crypto(
            "RSA public key is not a DER SEQUENCE".to_owned(),
        ));
    }
    let parts = seq.children()?;
    if parts.len() != 2 {
        return Err(SigningError::Crypto(
            "RSA public key must have modulus and publicExponent".to_owned(),
        ));
    }
    let n_bytes = der_positive_uint(parts[0].as_integer_bytes()?)?;
    let e_bytes = der_positive_uint(parts[1].as_integer_bytes()?)?;
    if n_bytes.is_empty() || e_bytes.is_empty() {
        return Err(SigningError::Crypto(
            "RSA modulus or exponent is zero".to_owned(),
        ));
    }
    if n_bytes.last().copied().unwrap_or(0) & 1 == 0 {
        return Err(SigningError::Crypto("RSA modulus must be odd".to_owned()));
    }
    let limb_count = n_bytes.len().div_ceil(8);
    let modulus = bytes_to_limbs_le(n_bytes, limb_count);
    let modulus_bits = limbs_bit_len(&modulus);
    if !(1024..=8192).contains(&modulus_bits) {
        return Err(SigningError::Crypto(format!(
            "unsupported RSA key size ({modulus_bits} bits)"
        )));
    }
    if e_bytes.len() > n_bytes.len() {
        return Err(SigningError::Crypto(
            "RSA public exponent exceeds modulus size".to_owned(),
        ));
    }
    let exp_limbs = bytes_to_limbs_le(e_bytes, e_bytes.len().div_ceil(8));
    let e_is_odd = exp_limbs.first().copied().unwrap_or(0) & 1 == 1;
    let e_ge_3 = exp_limbs.len() > 1 || exp_limbs.first().copied().unwrap_or(0) >= 3;
    if !e_is_odd || !e_ge_3 || cmp_limbs_le(&exp_limbs, &modulus) != Ordering::Less {
        return Err(SigningError::Crypto(
            "invalid RSA public exponent".to_owned(),
        ));
    }
    Ok(RsaPublicKey {
        modulus,
        exponent: exp_limbs,
        modulus_bytes: n_bytes.len(),
        modulus_bits,
    })
}

fn rsa_public_op(key: &RsaPublicKey, signature: &[u8]) -> Result<Vec<u8>, SigningError> {
    if signature.len() != key.modulus_bytes {
        return Err(SigningError::Crypto(format!(
            "RSA signature length {} does not match modulus length {}",
            signature.len(),
            key.modulus_bytes
        )));
    }
    let s = bytes_to_limbs_le(signature, key.modulus.len());
    if cmp_limbs_le(&s, &key.modulus) != Ordering::Less {
        return Err(SigningError::Crypto(
            "RSA signature representative out of range".to_owned(),
        ));
    }
    let m = mod_pow(&s, &key.exponent, &key.modulus);
    Ok(limbs_le_to_bytes_be(&m, key.modulus_bytes))
}

fn verify_rsa_pkcs1_v15(
    key: &RsaPublicKey,
    hash_alg: HashAlgorithm,
    message_slices: &[&[u8]],
    signature: &[u8],
) -> Result<(), SigningError> {
    let em = rsa_public_op(key, signature)?;
    let prefix = match hash_alg {
        HashAlgorithm::Sha256 => DIGEST_INFO_SHA256,
        HashAlgorithm::Sha384 => DIGEST_INFO_SHA384,
        HashAlgorithm::Sha512 => DIGEST_INFO_SHA512,
    };
    let digest = hash_alg.hash_slices(message_slices);
    let t_len = prefix.len() + digest.len();
    if key.modulus_bytes < t_len + 11 {
        return Err(SigningError::Crypto(
            "RSA modulus too short for EMSA-PKCS1-v1_5 encoding".to_owned(),
        ));
    }
    let ps_len = key.modulus_bytes - t_len - 3;
    let mut expected = Vec::with_capacity(key.modulus_bytes);
    expected.push(0x00);
    expected.push(0x01);
    expected.resize(2 + ps_len, 0xFF);
    expected.push(0x00);
    expected.extend_from_slice(prefix);
    expected.extend_from_slice(&digest);

    if ct_eq(&em, &expected) {
        Ok(())
    } else {
        Err(SigningError::Crypto(
            "RSA PKCS#1 v1.5 signature verification failed".to_owned(),
        ))
    }
}

fn parse_pss_params(params_der: Option<&[u8]>) -> Result<RsaPssParams, SigningError> {
    let raw = params_der.ok_or_else(|| {
        SigningError::Crypto(
            "RSA-PSS parameters required (SHA-1 default is unsupported)".to_owned(),
        )
    })?;
    let seq = parse_der(raw)?;
    if seq.tag != TAG_SEQUENCE {
        return Err(SigningError::Crypto(
            "RSASSA-PSS-params must be a DER SEQUENCE".to_owned(),
        ));
    }
    let mut hash_alg: Option<HashAlgorithm> = None;
    let mut mgf_hash_alg: Option<HashAlgorithm> = None;
    let mut salt_len: usize = 20;
    let mut trailer_field: u64 = 1;

    for child in seq.children()? {
        match child.tag {
            0xA0 => {
                let inner = child.children()?;
                let alg_seq = inner.first().ok_or_else(|| {
                    SigningError::Crypto("empty RSA-PSS hashAlgorithm".to_owned())
                })?;
                let oid = alg_seq
                    .children()?
                    .first()
                    .ok_or_else(|| SigningError::Crypto("missing RSA-PSS hash OID".to_owned()))?
                    .as_oid()?;
                hash_alg = Some(HashAlgorithm::from_oid(&oid).ok_or_else(|| {
                    SigningError::Crypto("unsupported RSA-PSS hash algorithm".to_owned())
                })?);
            }
            0xA1 => {
                let inner = child.children()?;
                let mgf_seq = inner.first().ok_or_else(|| {
                    SigningError::Crypto("empty RSA-PSS maskGenAlgorithm".to_owned())
                })?;
                let mgf_parts = mgf_seq.children()?;
                let mgf_oid = mgf_parts
                    .first()
                    .ok_or_else(|| SigningError::Crypto("missing RSA-PSS MGF OID".to_owned()))?
                    .as_oid()?;
                if mgf_oid != OID_MGF1 {
                    return Err(SigningError::Crypto(
                        "unsupported RSA-PSS mask generation function".to_owned(),
                    ));
                }
                let mgf_hash_seq = mgf_parts.get(1).ok_or_else(|| {
                    SigningError::Crypto("missing MGF1 hash algorithm parameter".to_owned())
                })?;
                let mgf_hash_oid = mgf_hash_seq
                    .children()?
                    .first()
                    .ok_or_else(|| SigningError::Crypto("missing MGF1 hash OID".to_owned()))?
                    .as_oid()?;
                mgf_hash_alg = Some(HashAlgorithm::from_oid(&mgf_hash_oid).ok_or_else(|| {
                    SigningError::Crypto("unsupported RSA-PSS MGF1 hash algorithm".to_owned())
                })?);
            }
            0xA2 => {
                let inner = child.children()?;
                let int_node = inner
                    .first()
                    .ok_or_else(|| SigningError::Crypto("empty RSA-PSS saltLength".to_owned()))?;
                let bytes = der_positive_uint(int_node.as_integer_bytes()?)?;
                if bytes.len() > 4 {
                    return Err(SigningError::Crypto(
                        "RSA-PSS saltLength too large".to_owned(),
                    ));
                }
                let mut val = 0usize;
                for &b in bytes {
                    val = (val << 8) | usize::from(b);
                }
                salt_len = val;
            }
            0xA3 => {
                let inner = child.children()?;
                let int_node = inner
                    .first()
                    .ok_or_else(|| SigningError::Crypto("empty RSA-PSS trailerField".to_owned()))?;
                let bytes = der_positive_uint(int_node.as_integer_bytes()?)?;
                if bytes.len() > 8 {
                    return Err(SigningError::Crypto(
                        "invalid RSA-PSS trailerField".to_owned(),
                    ));
                }
                let mut val = 0u64;
                for &b in bytes {
                    val = (val << 8) | u64::from(b);
                }
                trailer_field = val;
            }
            _ => {
                return Err(SigningError::Crypto(
                    "unexpected tag in RSASSA-PSS-params".to_owned(),
                ));
            }
        }
    }

    if trailer_field != 1 {
        return Err(SigningError::Crypto(
            "unsupported RSA-PSS trailerField (only 0xBC / 1 supported)".to_owned(),
        ));
    }
    let hash_alg = hash_alg.ok_or_else(|| {
        SigningError::Crypto("RSA-PSS requires explicit SHA-256/384/512 hashAlgorithm".to_owned())
    })?;
    let mgf_hash_alg = mgf_hash_alg.ok_or_else(|| {
        SigningError::Crypto(
            "RSA-PSS requires explicit MGF1 SHA-256/384/512 maskGenAlgorithm".to_owned(),
        )
    })?;
    Ok(RsaPssParams {
        hash_alg,
        mgf_hash_alg,
        salt_len,
    })
}

fn mgf1(hash_alg: HashAlgorithm, seed: &[u8], mask_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(mask_len + hash_alg.digest_len());
    let mut counter = 0u32;
    while out.len() < mask_len {
        let c_bytes = counter.to_be_bytes();
        let block = hash_alg.hash_slices(&[seed, &c_bytes]);
        out.extend_from_slice(&block);
        counter = counter.wrapping_add(1);
    }
    out.truncate(mask_len);
    out
}

fn verify_rsa_pss(
    key: &RsaPublicKey,
    params: RsaPssParams,
    message_slices: &[&[u8]],
    signature: &[u8],
) -> Result<(), SigningError> {
    let em_full = rsa_public_op(key, signature)?;
    let em_bits = key.modulus_bits - 1;
    let em_len = em_bits.div_ceil(8);
    if em_len > em_full.len() {
        return Err(SigningError::Crypto(
            "RSA-PSS encoded message length exceeds modulus".to_owned(),
        ));
    }
    let lead = em_full.len() - em_len;
    if em_full[..lead].iter().any(|&b| b != 0) {
        return Err(SigningError::Crypto(
            "RSA-PSS signature representative overflows emLen".to_owned(),
        ));
    }
    let em = &em_full[lead..];
    let h_len = params.hash_alg.digest_len();
    let s_len = params.salt_len;
    if em_len < h_len + s_len + 2 {
        return Err(SigningError::Crypto(
            "RSA-PSS modulus too small for hash and salt length".to_owned(),
        ));
    }
    if em[em_len - 1] != 0xBC {
        return Err(SigningError::Crypto(
            "RSA-PSS trailer byte mismatch".to_owned(),
        ));
    }

    let db_len = em_len - h_len - 1;
    let masked_db = &em[..db_len];
    let h = &em[db_len..db_len + h_len];

    let top_zero_bits = 8 * em_len - em_bits;
    if top_zero_bits > 0 {
        let top_mask = 0xFFu8 << (8 - top_zero_bits);
        if (masked_db[0] & top_mask) != 0 {
            return Err(SigningError::Crypto(
                "RSA-PSS leftmost padding bits non-zero".to_owned(),
            ));
        }
    }

    let db_mask = mgf1(params.mgf_hash_alg, h, db_len);
    let mut db: Vec<u8> = masked_db
        .iter()
        .zip(db_mask.iter())
        .map(|(&a, &b)| a ^ b)
        .collect();
    if top_zero_bits > 0 {
        db[0] &= 0xFFu8 >> top_zero_bits;
    }

    let ps_len = db_len - s_len - 1;
    let mut pad_diff = 0u8;
    for &b in &db[..ps_len] {
        pad_diff |= b;
    }
    pad_diff |= db[ps_len] ^ 0x01;
    if pad_diff != 0 {
        return Err(SigningError::Crypto(
            "RSA-PSS DB padding verification failed".to_owned(),
        ));
    }

    let salt = &db[ps_len + 1..];
    let m_hash = params.hash_alg.hash_slices(message_slices);
    let h_prime = params.hash_alg.hash_slices(&[&[0u8; 8], &m_hash, salt]);
    if ct_eq(h, &h_prime) {
        Ok(())
    } else {
        Err(SigningError::Crypto(
            "RSA-PSS digest verification failed".to_owned(),
        ))
    }
}

fn mont_inv_u64(n0: u64) -> u64 {
    // Computes -n0^{-1} mod 2^64 via Newton's method (n0 must be odd).
    let mut inv = 1u64;
    for _ in 0..6 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(n0.wrapping_mul(inv)));
    }
    inv.wrapping_neg()
}

fn mont_cond_sub(t: &[u64], n: &[u64]) -> Vec<u64> {
    let k = n.len();
    let mut diff = vec![0u64; k];
    let mut borrow = 0u64;
    for i in 0..k {
        let (d1, b1) = t[i].overflowing_sub(n[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        diff[i] = d2;
        borrow = u64::from(b1) + u64::from(b2);
    }
    let mask = 0u64.wrapping_sub(u64::from(t[k] < borrow));
    let mut out = vec![0u64; k];
    for i in 0..k {
        out[i] = (t[i] & mask) | (diff[i] & !mask);
    }
    out
}

fn mont_mul(a: &[u64], b: &[u64], n: &[u64], n_prime: u64) -> Vec<u64> {
    let k = n.len();
    let mut t = vec![0u64; k + 2];
    for &ai in a.iter().take(k) {
        let mut carry = 0u128;
        for j in 0..k {
            let prod = u128::from(ai) * u128::from(b[j]) + u128::from(t[j]) + carry;
            t[j] = prod as u64;
            carry = prod >> 64;
        }
        let sum = u128::from(t[k]) + carry;
        t[k] = sum as u64;
        t[k + 1] = (u128::from(t[k + 1]) + (sum >> 64)) as u64;

        let m = t[0].wrapping_mul(n_prime);
        let first = u128::from(t[0]) + u128::from(m) * u128::from(n[0]);
        let mut carry = first >> 64;
        for j in 1..k {
            let prod = u128::from(t[j]) + u128::from(m) * u128::from(n[j]) + carry;
            t[j - 1] = prod as u64;
            carry = prod >> 64;
        }
        let sum = u128::from(t[k]) + carry;
        t[k - 1] = sum as u64;
        t[k] = (u128::from(t[k + 1]) + (sum >> 64)) as u64;
        t[k + 1] = 0;
    }
    mont_cond_sub(&t[..=k], n)
}

fn mont_r2(n: &[u64]) -> Vec<u64> {
    let k = n.len();
    let mut acc = vec![0u64; k + 1];
    acc[0] = 1;
    for _ in 0..(128 * k) {
        let mut carry = 0u64;
        for limb in acc.iter_mut().take(k) {
            let next_carry = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next_carry;
        }
        acc[k] = carry;
        let reduced = mont_cond_sub(&acc, n);
        acc[..k].copy_from_slice(&reduced);
        acc[k] = 0;
    }
    acc.truncate(k);
    acc
}

fn mod_pow(base: &[u64], exp: &[u64], n: &[u64]) -> Vec<u64> {
    let k = n.len();
    let n_prime = mont_inv_u64(n[0]);
    let r2 = mont_r2(n);
    let base_mont = mont_mul(base, &r2, n, n_prime);
    let mut one = vec![0u64; k];
    one[0] = 1;
    let mut acc = mont_mul(&one, &r2, n, n_prime);
    let bits = limbs_bit_len(exp);
    for bit_idx in (0..bits).rev() {
        acc = mont_mul(&acc, &acc, n, n_prime);
        if (exp[bit_idx / 64] >> (bit_idx % 64)) & 1 == 1 {
            acc = mont_mul(&acc, &base_mont, n, n_prime);
        }
    }
    mont_mul(&acc, &one, n, n_prime)
}

fn bytes_to_limbs_le(bytes: &[u8], limb_count: usize) -> Vec<u64> {
    let mut limbs = vec![0u64; limb_count];
    for (idx, &b) in bytes.iter().rev().enumerate() {
        let limb_idx = idx / 8;
        if limb_idx < limb_count {
            limbs[limb_idx] |= u64::from(b) << ((idx % 8) * 8);
        }
    }
    limbs
}

fn limbs_le_to_bytes_be(limbs: &[u64], byte_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; byte_len];
    for (i, slot) in out.iter_mut().rev().enumerate() {
        let limb_idx = i / 8;
        let byte_in_limb = i % 8;
        *slot = limbs
            .get(limb_idx)
            .map_or(0, |&w| ((w >> (byte_in_limb * 8)) & 0xFF) as u8);
    }
    out
}

fn limbs_bit_len(limbs: &[u64]) -> usize {
    for (i, &w) in limbs.iter().enumerate().rev() {
        if w != 0 {
            return i * 64 + (64 - w.leading_zeros() as usize);
        }
    }
    0
}

fn cmp_limbs_le(a: &[u64], b: &[u64]) -> Ordering {
    let len = a.len().max(b.len());
    for i in (0..len).rev() {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    Ordering::Equal
}

// ---- Pure-Rust ECDSA P-256 and P-384 verification (short Weierstrass a = -3) ----

#[derive(Clone, Copy)]
struct CurveParams<const N: usize> {
    p: [u64; N],
    b: [u64; N],
    n: [u64; N],
    gx: [u64; N],
    gy: [u64; N],
}

/// NIST FIPS 186-4 P-256 (`secp256r1` / `prime256v1`) domain parameters.
const CURVE_P256: CurveParams<4> = CurveParams {
    p: [
        0xFFFF_FFFF_FFFF_FFFF,
        0x0000_0000_FFFF_FFFF,
        0x0000_0000_0000_0000,
        0xFFFF_FFFF_0000_0001,
    ],
    b: [
        0x3BCE_3C3E_27D2_604B,
        0x651D_06B0_CC53_B0F6,
        0xB3EB_BD55_7698_86BC,
        0x5AC6_35D8_AA3A_93E7,
    ],
    n: [
        0xF3B9_CAC2_FC63_2551,
        0xBCE6_FAAD_A717_9E84,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_0000_0000,
    ],
    gx: [
        0xF4A1_3945_D898_C296,
        0x7703_7D81_2DEB_33A0,
        0xF8BC_E6E5_63A4_40F2,
        0x6B17_D1F2_E12C_4247,
    ],
    gy: [
        0xCBB6_4068_37BF_51F5,
        0x2BCE_3357_6B31_5ECE,
        0x8EE7_EB4A_7C0F_9E16,
        0x4FE3_42E2_FE1A_7F9B,
    ],
};

/// NIST FIPS 186-4 P-384 (`secp384r1`) domain parameters.
const CURVE_P384: CurveParams<6> = CurveParams {
    p: [
        0x0000_0000_FFFF_FFFF,
        0xFFFF_FFFF_0000_0000,
        0xFFFF_FFFF_FFFF_FFFE,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_FFFF_FFFF,
    ],
    b: [
        0x2A85_C8ED_D3EC_2AEF,
        0xC656_398D_8A2E_D19D,
        0x0314_088F_5013_875A,
        0x181D_9C6E_FE81_4112,
        0x988E_056B_E3F8_2D19,
        0xB331_2FA7_E23E_E7E4,
    ],
    n: [
        0xECEC_196A_CCC5_2973,
        0x581A_0DB2_48B0_A77A,
        0xC763_4D81_F437_2DDF,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_FFFF_FFFF,
    ],
    gx: [
        0x3A54_5E38_7276_0AB7,
        0x5502_F25D_BF55_296C,
        0x59F7_41E0_8254_2A38,
        0x6E1D_3B62_8BA7_9B98,
        0x8EB1_C71E_F320_AD74,
        0xAA87_CA22_BE8B_0537,
    ],
    gy: [
        0x7A43_1D7C_90EA_0E5F,
        0x0A60_B1CE_1D7E_819D,
        0xE9DA_3113_B5F0_B8C0,
        0xF8F4_1DBD_289A_147C,
        0x5D9E_98BF_9292_DC29,
        0x3617_DE4A_9626_2C6F,
    ],
};

#[derive(Clone, Copy)]
struct PrimeField<const N: usize> {
    modulus: [u64; N],
    m_prime: u64,
    r2: [u64; N],
    one_mont: [u64; N],
}

impl<const N: usize> PrimeField<N> {
    fn new(modulus: [u64; N]) -> Self {
        let m_prime = mont_inv_u64(modulus[0]);
        let mut r2 = [0u64; N];
        r2[0] = 1;
        for _ in 0..(128 * N) {
            let mut carry = 0u64;
            for limb in &mut r2 {
                let next = *limb >> 63;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            r2 = cond_sub_fixed(&r2, carry, &modulus);
        }
        let mut one = [0u64; N];
        one[0] = 1;
        let one_mont = mont_mul_fixed(&one, &r2, &modulus, m_prime);
        Self {
            modulus,
            m_prime,
            r2,
            one_mont,
        }
    }

    fn encode_mont(&self, a: &[u64; N]) -> [u64; N] {
        mont_mul_fixed(a, &self.r2, &self.modulus, self.m_prime)
    }

    fn decode_mont(&self, a: &[u64; N]) -> [u64; N] {
        let mut one = [0u64; N];
        one[0] = 1;
        mont_mul_fixed(a, &one, &self.modulus, self.m_prime)
    }

    fn mul(&self, a: &[u64; N], b: &[u64; N]) -> [u64; N] {
        mont_mul_fixed(a, b, &self.modulus, self.m_prime)
    }

    fn sq(&self, a: &[u64; N]) -> [u64; N] {
        self.mul(a, a)
    }

    fn add(&self, a: &[u64; N], b: &[u64; N]) -> [u64; N] {
        let mut sum = [0u64; N];
        let mut carry = 0u64;
        for i in 0..N {
            let (s1, c1) = a[i].overflowing_add(b[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            sum[i] = s2;
            carry = u64::from(c1) + u64::from(c2);
        }
        cond_sub_fixed(&sum, carry, &self.modulus)
    }

    fn sub(&self, a: &[u64; N], b: &[u64; N]) -> [u64; N] {
        let mut diff = [0u64; N];
        let mut borrow = 0u64;
        for i in 0..N {
            let (d1, b1) = a[i].overflowing_sub(b[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            diff[i] = d2;
            borrow = u64::from(b1) + u64::from(b2);
        }
        let mask = 0u64.wrapping_sub(borrow);
        let mut out = [0u64; N];
        let mut carry = 0u64;
        for i in 0..N {
            let addend = self.modulus[i] & mask;
            let (s1, c1) = diff[i].overflowing_add(addend);
            let (s2, c2) = s1.overflowing_add(carry);
            out[i] = s2;
            carry = u64::from(c1) + u64::from(c2);
        }
        out
    }

    fn double(&self, a: &[u64; N]) -> [u64; N] {
        self.add(a, a)
    }

    fn pow_mont(&self, base_mont: &[u64; N], exp: &[u64; N]) -> [u64; N] {
        let mut acc = self.one_mont;
        let bits = limbs_bit_len(exp);
        for bit_idx in (0..bits).rev() {
            acc = self.sq(&acc);
            if (exp[bit_idx / 64] >> (bit_idx % 64)) & 1 == 1 {
                acc = self.mul(&acc, base_mont);
            }
        }
        acc
    }

    fn inv_mont(&self, a_mont: &[u64; N]) -> [u64; N] {
        let exp = sub_u64_fixed(&self.modulus, 2);
        self.pow_mont(a_mont, &exp)
    }
}

fn cond_sub_fixed<const N: usize>(t: &[u64; N], t_hi: u64, m: &[u64; N]) -> [u64; N] {
    let mut diff = [0u64; N];
    let mut borrow = 0u64;
    for i in 0..N {
        let (d1, b1) = t[i].overflowing_sub(m[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        diff[i] = d2;
        borrow = u64::from(b1) + u64::from(b2);
    }
    let mask = 0u64.wrapping_sub(u64::from(t_hi < borrow));
    let mut out = [0u64; N];
    for i in 0..N {
        out[i] = (t[i] & mask) | (diff[i] & !mask);
    }
    out
}

fn mont_mul_fixed<const N: usize>(
    a: &[u64; N],
    b: &[u64; N],
    m: &[u64; N],
    m_prime: u64,
) -> [u64; N] {
    let mut t = [0u64; N];
    let mut t_hi = 0u64;
    for &ai in a {
        let mut carry = 0u128;
        for j in 0..N {
            let prod = u128::from(ai) * u128::from(b[j]) + u128::from(t[j]) + carry;
            t[j] = prod as u64;
            carry = prod >> 64;
        }
        let sum = u128::from(t_hi) + carry;
        t_hi = sum as u64;
        let t_top = (sum >> 64) as u64;

        let u = t[0].wrapping_mul(m_prime);
        let first = u128::from(t[0]) + u128::from(u) * u128::from(m[0]);
        let mut carry = first >> 64;
        for j in 1..N {
            let prod = u128::from(t[j]) + u128::from(u) * u128::from(m[j]) + carry;
            t[j - 1] = prod as u64;
            carry = prod >> 64;
        }
        let sum = u128::from(t_hi) + carry;
        t[N - 1] = sum as u64;
        t_hi = (u128::from(t_top) + (sum >> 64)) as u64;
    }
    cond_sub_fixed(&t, t_hi, m)
}

fn sub_u64_fixed<const N: usize>(a: &[u64; N], v: u64) -> [u64; N] {
    let mut out = *a;
    let mut borrow = v;
    for limb in &mut out {
        let (d, b) = limb.overflowing_sub(borrow);
        *limb = d;
        borrow = u64::from(b);
    }
    out
}

fn is_zero_fixed<const N: usize>(a: &[u64; N]) -> bool {
    a.iter().all(|&w| w == 0)
}

fn bytes_to_fixed_le<const N: usize>(bytes: &[u8]) -> Option<[u64; N]> {
    if bytes.len() > N * 8 {
        return None;
    }
    let mut out = [0u64; N];
    for (idx, &b) in bytes.iter().rev().enumerate() {
        out[idx / 8] |= u64::from(b) << ((idx % 8) * 8);
    }
    Some(out)
}

#[derive(Clone, Copy)]
struct JacobianPoint<const N: usize> {
    x: [u64; N],
    y: [u64; N],
    z: [u64; N],
    infinity: bool,
}

impl<const N: usize> JacobianPoint<N> {
    const fn infinity() -> Self {
        Self {
            x: [0u64; N],
            y: [0u64; N],
            z: [0u64; N],
            infinity: true,
        }
    }

    fn from_affine_mont(fp: &PrimeField<N>, x_mont: [u64; N], y_mont: [u64; N]) -> Self {
        Self {
            x: x_mont,
            y: y_mont,
            z: fp.one_mont,
            infinity: false,
        }
    }
}

fn ec_double<const N: usize>(fp: &PrimeField<N>, p: &JacobianPoint<N>) -> JacobianPoint<N> {
    if p.infinity || is_zero_fixed(&p.y) {
        return JacobianPoint::infinity();
    }
    let z2 = fp.sq(&p.z);
    let t1 = fp.sub(&p.x, &z2);
    let t2 = fp.add(&p.x, &z2);
    let t3 = fp.mul(&t1, &t2);
    let m = fp.add(&fp.double(&t3), &t3);

    let y2 = fp.sq(&p.y);
    let xy2 = fp.mul(&p.x, &y2);
    let s = fp.double(&fp.double(&xy2));

    let m2 = fp.sq(&m);
    let x3 = fp.sub(&m2, &fp.double(&s));

    let y4 = fp.sq(&y2);
    let eight_y4 = fp.double(&fp.double(&fp.double(&y4)));
    let y3 = fp.sub(&fp.mul(&m, &fp.sub(&s, &x3)), &eight_y4);
    let z3 = fp.double(&fp.mul(&p.y, &p.z));

    JacobianPoint {
        x: x3,
        y: y3,
        z: z3,
        infinity: false,
    }
}

fn ec_add<const N: usize>(
    fp: &PrimeField<N>,
    p: &JacobianPoint<N>,
    q: &JacobianPoint<N>,
) -> JacobianPoint<N> {
    if p.infinity {
        return *q;
    }
    if q.infinity {
        return *p;
    }
    let z1_2 = fp.sq(&p.z);
    let z2_2 = fp.sq(&q.z);
    let u1 = fp.mul(&p.x, &z2_2);
    let u2 = fp.mul(&q.x, &z1_2);
    let z2_3 = fp.mul(&q.z, &z2_2);
    let z1_3 = fp.mul(&p.z, &z1_2);
    let s1 = fp.mul(&p.y, &z2_3);
    let s2 = fp.mul(&q.y, &z1_3);

    let h = fp.sub(&u2, &u1);
    let r = fp.sub(&s2, &s1);
    if is_zero_fixed(&h) {
        if is_zero_fixed(&r) {
            return ec_double(fp, p);
        }
        return JacobianPoint::infinity();
    }

    let h2 = fp.sq(&h);
    let h3 = fp.mul(&h, &h2);
    let v = fp.mul(&u1, &h2);
    let r2 = fp.sq(&r);
    let x3 = fp.sub(&fp.sub(&r2, &h3), &fp.double(&v));
    let y3 = fp.sub(&fp.mul(&r, &fp.sub(&v, &x3)), &fp.mul(&s1, &h3));
    let z3 = fp.mul(&h, &fp.mul(&p.z, &q.z));

    JacobianPoint {
        x: x3,
        y: y3,
        z: z3,
        infinity: false,
    }
}

fn ec_mul_add<const N: usize>(
    fp: &PrimeField<N>,
    p: &JacobianPoint<N>,
    u1: &[u64; N],
    q: &JacobianPoint<N>,
    u2: &[u64; N],
) -> JacobianPoint<N> {
    let pq = ec_add(fp, p, q);
    let mut acc = JacobianPoint::infinity();
    for bit_idx in (0..N * 64).rev() {
        acc = ec_double(fp, &acc);
        let b1 = (u1[bit_idx / 64] >> (bit_idx % 64)) & 1;
        let b2 = (u2[bit_idx / 64] >> (bit_idx % 64)) & 1;
        match (b1, b2) {
            (1, 0) => acc = ec_add(fp, &acc, p),
            (0, 1) => acc = ec_add(fp, &acc, q),
            (1, 1) => acc = ec_add(fp, &acc, &pq),
            _ => {}
        }
    }
    acc
}

fn hash_to_scalar<const N: usize>(
    hash_alg: HashAlgorithm,
    slices: &[&[u8]],
    order_n: &[u64; N],
) -> [u64; N] {
    let digest = hash_alg.hash_slices(slices);
    let coord_len = N * 8;
    let truncated = if digest.len() > coord_len {
        &digest[..coord_len]
    } else {
        &digest[..]
    };
    let mut z = [0u64; N];
    for (idx, &b) in truncated.iter().rev().enumerate() {
        z[idx / 8] |= u64::from(b) << ((idx % 8) * 8);
    }
    cond_sub_fixed(&z, 0, order_n)
}

fn verify_ecdsa(
    curve_oid: &[u32],
    pub_bytes: &[u8],
    hash_alg: HashAlgorithm,
    message_slices: &[&[u8]],
    sig_der: &[u8],
) -> Result<(), SigningError> {
    if curve_oid == OID_SECP256R1 {
        verify_ecdsa_curve(&CURVE_P256, pub_bytes, hash_alg, message_slices, sig_der)
    } else if curve_oid == OID_SECP384R1 {
        verify_ecdsa_curve(&CURVE_P384, pub_bytes, hash_alg, message_slices, sig_der)
    } else {
        Err(SigningError::Crypto(format!(
            "unsupported EC curve OID {}",
            curve_oid
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(".")
        )))
    }
}

fn verify_ecdsa_curve<const N: usize>(
    curve: &CurveParams<N>,
    pub_bytes: &[u8],
    hash_alg: HashAlgorithm,
    message_slices: &[&[u8]],
    sig_der: &[u8],
) -> Result<(), SigningError> {
    let coord_len = N * 8;
    if pub_bytes.len() != 1 + 2 * coord_len || pub_bytes[0] != 0x04 {
        return Err(SigningError::Crypto(format!(
            "EC public key must be an uncompressed SEC1 point ({} bytes)",
            1 + 2 * coord_len
        )));
    }
    let qx = bytes_to_fixed_le::<N>(&pub_bytes[1..1 + coord_len])
        .ok_or_else(|| SigningError::Crypto("invalid EC public key x-coordinate".to_owned()))?;
    let qy = bytes_to_fixed_le::<N>(&pub_bytes[1 + coord_len..])
        .ok_or_else(|| SigningError::Crypto("invalid EC public key y-coordinate".to_owned()))?;
    if cmp_limbs_le(&qx, &curve.p) != Ordering::Less
        || cmp_limbs_le(&qy, &curve.p) != Ordering::Less
    {
        return Err(SigningError::Crypto(
            "EC public key coordinates out of field range".to_owned(),
        ));
    }

    let fp = PrimeField::new(curve.p);
    let qx_m = fp.encode_mont(&qx);
    let qy_m = fp.encode_mont(&qy);
    let b_m = fp.encode_mont(&curve.b);
    let lhs = fp.sq(&qy_m);
    let qx2 = fp.sq(&qx_m);
    let qx3 = fp.mul(&qx2, &qx_m);
    let three_qx = fp.add(&fp.double(&qx_m), &qx_m);
    let rhs = fp.add(&fp.sub(&qx3, &three_qx), &b_m);
    if lhs != rhs {
        return Err(SigningError::Crypto(
            "EC public key point is not on curve".to_owned(),
        ));
    }

    let sig_seq = parse_der(sig_der)?;
    if sig_seq.tag != TAG_SEQUENCE {
        return Err(SigningError::Crypto(
            "ECDSA signature must be a DER SEQUENCE".to_owned(),
        ));
    }
    let sig_parts = sig_seq.children()?;
    if sig_parts.len() != 2 {
        return Err(SigningError::Crypto(
            "ECDSA signature SEQUENCE must contain r and s".to_owned(),
        ));
    }
    let r_bytes = der_positive_uint(sig_parts[0].as_integer_bytes()?)?;
    let s_bytes = der_positive_uint(sig_parts[1].as_integer_bytes()?)?;
    let r = bytes_to_fixed_le::<N>(r_bytes)
        .ok_or_else(|| SigningError::Crypto("ECDSA r component exceeds curve order".to_owned()))?;
    let s = bytes_to_fixed_le::<N>(s_bytes)
        .ok_or_else(|| SigningError::Crypto("ECDSA s component exceeds curve order".to_owned()))?;
    if is_zero_fixed(&r)
        || is_zero_fixed(&s)
        || cmp_limbs_le(&r, &curve.n) != Ordering::Less
        || cmp_limbs_le(&s, &curve.n) != Ordering::Less
    {
        return Err(SigningError::Crypto(
            "ECDSA signature r or s out of [1, n - 1]".to_owned(),
        ));
    }

    let z = hash_to_scalar(hash_alg, message_slices, &curve.n);
    let fn_field = PrimeField::new(curve.n);
    let r_m = fn_field.encode_mont(&r);
    let s_m = fn_field.encode_mont(&s);
    let z_m = fn_field.encode_mont(&z);
    let w = fn_field.inv_mont(&s_m);
    let u1 = fn_field.decode_mont(&fn_field.mul(&z_m, &w));
    let u2 = fn_field.decode_mont(&fn_field.mul(&r_m, &w));

    let g_pt =
        JacobianPoint::from_affine_mont(&fp, fp.encode_mont(&curve.gx), fp.encode_mont(&curve.gy));
    let q_pt = JacobianPoint::from_affine_mont(&fp, qx_m, qy_m);
    let sum_pt = ec_mul_add(&fp, &g_pt, &u1, &q_pt, &u2);
    if sum_pt.infinity {
        return Err(SigningError::Crypto(
            "ECDSA verification point is at infinity".to_owned(),
        ));
    }
    let z_inv = fp.inv_mont(&sum_pt.z);
    let z_inv2 = fp.sq(&z_inv);
    let x_aff = fp.decode_mont(&fp.mul(&sum_pt.x, &z_inv2));
    let v = cond_sub_fixed(&x_aff, 0, &curve.n);
    if v == r {
        Ok(())
    } else {
        Err(SigningError::Crypto(
            "ECDSA signature verification failed".to_owned(),
        ))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Test-only 2048-bit RSA modulus (generated with Windows CNG for unit testing).
    pub(crate) const TEST_RSA_N_HEX: &str = "b7c5bee55a6c09dd1afa69a8d03efd787b98c695629c8a7a9de6b0bbf0eb3f38df7f8c8bef5f83e16c877bbb17a339084721553c07454000a8a84b409e2d3296ae31d6db3a57bbd80f36bb1517105e30e35463e952c8905333da9b0b4df21c98a38ccdc0f26099a88b9624db08d3e83ad4839d6e4d6fe6fbc8053023c62a6c287bd119d1f96e488b0a61450a98cd518baa156372ca73fdb064f1533bdb19f3c19d052bffc874f3e314257bf1d32c042bae17cb5c70fadad396f0d8f73e5a244c68869e6135d8fabd7e22ca40842eba1af52626b280d7add11189e0ff1ca116d509b050704069c7e406be5a8c858c9551e80eb149699cbce47bdd48464e5bcf01";
    pub(crate) const TEST_RSA_E_HEX: &str = "010001";
    pub(crate) const TEST_RSA_D_HEX: &str = "33a3edb0a568169d7b2742e99d1b163ef4fc495151148c1548a9baa0156305b1059ce1f1b0a2367cf1f260628291eb44d83815ab789f8d0ca258284d50258341b4516cfecdb639a2cd23a40e991c905b4c863a4d266615a84bb9f2378affade9768bea857d96879745223bd910f1fb200cf7af157fcd222b3e59b6dd56a23521f07d6b22b3f52a81044c85705ec108fb9038eeaa1522f19db497a944ee13da96a777fc724d14953348721ccc9add0559959cd0efdddda41edaa8ed7e84a1da649b7e815fae104a6483a2f6e5f35041d335143f434f5a8386997cd170eaae593d0f380cbcc5680de361d82d453b523a8457629b291d6f6120c4f5c47be07a3fc1";

    pub(crate) fn hex_decode(hex: &str) -> Vec<u8> {
        let bytes = hex.as_bytes();
        assert_eq!(bytes.len() % 2, 0, "hex string must have even length");
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.as_chunks::<2>().0 {
            let s = std::str::from_utf8(chunk).expect("ascii hex");
            out.push(u8::from_str_radix(s, 16).expect("valid hex byte"));
        }
        out
    }

    pub(crate) fn encode_unsigned_integer(bytes: &[u8]) -> Vec<u8> {
        let Some(pos) = bytes.iter().position(|&b| b != 0) else {
            return encode_tlv(TAG_INTEGER, &[0]);
        };
        let stripped = &bytes[pos..];
        if stripped[0] & 0x80 != 0 {
            let mut padded = Vec::with_capacity(stripped.len() + 1);
            padded.push(0);
            padded.extend_from_slice(stripped);
            encode_tlv(TAG_INTEGER, &padded)
        } else {
            encode_tlv(TAG_INTEGER, stripped)
        }
    }

    pub(crate) fn encode_rsa_spki(n_be: &[u8], e_be: &[u8]) -> Vec<u8> {
        let rsa_pub =
            encode_sequence(&[encode_unsigned_integer(n_be), encode_unsigned_integer(e_be)]);
        encode_sequence(&[
            encode_alg_id(OID_RSA_ENCRYPTION, AlgParams::Null),
            encode_bit_string(&rsa_pub, 0),
        ])
    }

    pub(crate) fn encode_pss_params_der(hash_alg: HashAlgorithm, salt_len: u64) -> Vec<u8> {
        let hash_oid = match hash_alg {
            HashAlgorithm::Sha256 => OID_SHA256,
            HashAlgorithm::Sha384 => OID_SHA384,
            HashAlgorithm::Sha512 => OID_SHA512,
        };
        let hash_alg_id = encode_alg_id(hash_oid, AlgParams::Null);
        let mgf_alg_id = encode_sequence(&[encode_oid(OID_MGF1), hash_alg_id.clone()]);
        encode_sequence(&[
            encode_context_constructed(0, &[hash_alg_id]),
            encode_context_constructed(1, &[mgf_alg_id]),
            encode_context_constructed(2, &[encode_u64(salt_len)]),
        ])
    }

    pub(crate) fn rsa_pkcs1_sign(
        n_be: &[u8],
        d_be: &[u8],
        hash_alg: HashAlgorithm,
        msg: &[u8],
    ) -> Vec<u8> {
        let prefix = match hash_alg {
            HashAlgorithm::Sha256 => DIGEST_INFO_SHA256,
            HashAlgorithm::Sha384 => DIGEST_INFO_SHA384,
            HashAlgorithm::Sha512 => DIGEST_INFO_SHA512,
        };
        let digest = hash_alg.hash(msg);
        let k_bytes = n_be.len();
        let ps_len = k_bytes - prefix.len() - digest.len() - 3;
        let mut em = Vec::with_capacity(k_bytes);
        em.push(0x00);
        em.push(0x01);
        em.resize(2 + ps_len, 0xFF);
        em.push(0x00);
        em.extend_from_slice(prefix);
        em.extend_from_slice(&digest);

        let limbs = k_bytes.div_ceil(8);
        let n_l = bytes_to_limbs_le(n_be, limbs);
        let d_l = bytes_to_limbs_le(d_be, d_be.len().div_ceil(8));
        let em_l = bytes_to_limbs_le(&em, limbs);
        let sig_l = mod_pow(&em_l, &d_l, &n_l);
        limbs_le_to_bytes_be(&sig_l, k_bytes)
    }

    pub(crate) fn rsa_pss_sign(
        n_be: &[u8],
        d_be: &[u8],
        hash_alg: HashAlgorithm,
        salt: &[u8],
        msg: &[u8],
    ) -> Vec<u8> {
        let k_bytes = n_be.len();
        let limbs = k_bytes.div_ceil(8);
        let n_l = bytes_to_limbs_le(n_be, limbs);
        let mod_bits = limbs_bit_len(&n_l);
        let em_bits = mod_bits - 1;
        let em_len = em_bits.div_ceil(8);
        let h_len = hash_alg.digest_len();
        let db_len = em_len - h_len - 1;

        let m_hash = hash_alg.hash(msg);
        let h = hash_alg.hash_slices(&[&[0u8; 8], &m_hash, salt]);

        let mut db = vec![0u8; db_len - salt.len() - 1];
        db.push(0x01);
        db.extend_from_slice(salt);
        let db_mask = mgf1(hash_alg, &h, db_len);
        let mut masked_db: Vec<u8> = db
            .iter()
            .zip(db_mask.iter())
            .map(|(&a, &b)| a ^ b)
            .collect();
        let top_zero_bits = 8 * em_len - em_bits;
        if top_zero_bits > 0 {
            masked_db[0] &= 0xFFu8 >> top_zero_bits;
        }

        let mut em = Vec::with_capacity(k_bytes);
        em.resize(k_bytes - em_len, 0);
        em.extend_from_slice(&masked_db);
        em.extend_from_slice(&h);
        em.push(0xBC);

        let d_l = bytes_to_limbs_le(d_be, d_be.len().div_ceil(8));
        let em_l = bytes_to_limbs_le(&em, limbs);
        let sig_l = mod_pow(&em_l, &d_l, &n_l);
        limbs_le_to_bytes_be(&sig_l, k_bytes)
    }

    fn ec_pubkey_curve<const N: usize>(curve: &CurveParams<N>, d: [u64; N]) -> Vec<u8> {
        let fp = PrimeField::new(curve.p);
        let g = JacobianPoint::from_affine_mont(
            &fp,
            fp.encode_mont(&curve.gx),
            fp.encode_mont(&curve.gy),
        );
        let q = ec_mul_add(&fp, &g, &d, &JacobianPoint::infinity(), &[0u64; N]);
        let z_inv = fp.inv_mont(&q.z);
        let z_inv2 = fp.sq(&z_inv);
        let z_inv3 = fp.mul(&z_inv2, &z_inv);
        let qx = fp.decode_mont(&fp.mul(&q.x, &z_inv2));
        let qy = fp.decode_mont(&fp.mul(&q.y, &z_inv3));
        let mut out = Vec::with_capacity(1 + N * 16);
        out.push(0x04);
        out.extend_from_slice(&limbs_le_to_bytes_be(&qx, N * 8));
        out.extend_from_slice(&limbs_le_to_bytes_be(&qy, N * 8));
        out
    }

    fn ec_sign_curve<const N: usize>(
        curve: &CurveParams<N>,
        d: [u64; N],
        k: [u64; N],
        hash_alg: HashAlgorithm,
        msg: &[u8],
    ) -> Vec<u8> {
        let fp = PrimeField::new(curve.p);
        let g = JacobianPoint::from_affine_mont(
            &fp,
            fp.encode_mont(&curve.gx),
            fp.encode_mont(&curve.gy),
        );
        let r_pt = ec_mul_add(&fp, &g, &k, &JacobianPoint::infinity(), &[0u64; N]);
        let z_inv = fp.inv_mont(&r_pt.z);
        let z_inv2 = fp.sq(&z_inv);
        let rx = fp.decode_mont(&fp.mul(&r_pt.x, &z_inv2));
        let r = cond_sub_fixed(&rx, 0, &curve.n);

        let z = hash_to_scalar(hash_alg, &[msg], &curve.n);
        let fn_field = PrimeField::new(curve.n);
        let k_inv = fn_field.inv_mont(&fn_field.encode_mont(&k));
        let rd = fn_field.mul(&fn_field.encode_mont(&r), &fn_field.encode_mont(&d));
        let s_m = fn_field.mul(&k_inv, &fn_field.add(&fn_field.encode_mont(&z), &rd));
        let s = fn_field.decode_mont(&s_m);

        let r_be = limbs_le_to_bytes_be(&r, N * 8);
        let s_be = limbs_le_to_bytes_be(&s, N * 8);
        encode_sequence(&[
            encode_unsigned_integer(&r_be),
            encode_unsigned_integer(&s_be),
        ])
    }

    pub(crate) fn ecdsa_p256_pubkey(d: [u64; 4]) -> Vec<u8> {
        ec_pubkey_curve(&CURVE_P256, d)
    }

    pub(crate) fn ecdsa_p256_sign(
        d: [u64; 4],
        k: [u64; 4],
        hash_alg: HashAlgorithm,
        msg: &[u8],
    ) -> Vec<u8> {
        ec_sign_curve(&CURVE_P256, d, k, hash_alg, msg)
    }

    pub(crate) fn ecdsa_p384_pubkey(d: [u64; 6]) -> Vec<u8> {
        ec_pubkey_curve(&CURVE_P384, d)
    }

    pub(crate) fn ecdsa_p384_sign(
        d: [u64; 6],
        k: [u64; 6],
        hash_alg: HashAlgorithm,
        msg: &[u8],
    ) -> Vec<u8> {
        ec_sign_curve(&CURVE_P384, d, k, hash_alg, msg)
    }

    pub(crate) fn issue_custom_cert(
        subject: &DistinguishedName,
        spki_der: &[u8],
        issuer_cert: Option<&X509Certificate>,
        sig_alg_der: Vec<u8>,
        role: CertRole,
        serial: u64,
        sign_tbs: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<X509Certificate, SigningError> {
        let validity = CertValidity::default();
        let issuer_der = issuer_cert.map_or_else(|| subject.to_der(), |c| c.subject_der.clone());
        let version = encode_context_constructed(0, &[encode_u64(2)]);
        let serial_der = encode_u64(serial);
        let validity_der = encode_sequence(&[
            encode_utc_time(validity.not_before),
            encode_utc_time(validity.not_after),
        ]);
        let subject_der = subject.to_der();

        let bc_val = if role == CertRole::CertificateAuthority {
            encode_sequence(&[encode_boolean_true()])
        } else {
            encode_sequence(&[])
        };
        let extensions = vec![encode_sequence(&[
            encode_oid(OID_BASIC_CONSTRAINTS),
            encode_boolean_true(),
            encode_octet_string(&bc_val),
        ])];
        let exts_wrapper = encode_context_constructed(3, &[encode_sequence(&extensions)]);

        let tbs_der = encode_sequence(&[
            version,
            serial_der,
            sig_alg_der.clone(),
            issuer_der,
            validity_der,
            subject_der,
            spki_der.to_vec(),
            exts_wrapper,
        ]);
        let sig = sign_tbs(&tbs_der);
        let cert_der = encode_sequence(&[tbs_der, sig_alg_der, encode_bit_string(&sig, 0)]);
        X509Certificate::from_der(&cert_der)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    /// Independent Windows CNG (`RSACng`) RSA-2048 vectors (PKCS#1 v1.5 SHA-256/384/512 & PSS SHA-256/384).
    const CNG_RSA_N_HEX: &str = "cfb65bb6891bf9824303207fd792005fafb2df5b3bb7f07824b3d58fad52e76c6da91a4174f4ff6178ae20328cac1e3082c333fc659032c70888f1b48ff59c4f138d20219146df85ca3858dbbc95b1b1ca58334719a25a7580d8242c5399299a451be51a9274bfe214d376460ad29a2dbd3c17d3a04173c70d7423b0f79531184077dccc079e30131b56c460f49be7d95303e7a2105844ffe0925b99a2bf4356a0e4a517607dca667c3080dd74af6c273de9967f77f67d32a86312c3db564a904b7d1e1d39e4ad09a4e145b22ba2fe1f2e93fdcd238f67b58854e99cbf82cb44290756abd70fc0a00e6ed87fbd45d219ff540453b988cf786c846eeded8b791d";
    const CNG_RSA_PKCS256_HEX: &str = "83367fa7e3b9cd5236fd1075232a0faf10e11305548607b760058d781dc3ffb063f02e62f30206c7a480273e2f2cb086b26724b77ca6b52f65950c76dce00ede7e8bf589bc0953c8fce45cff58d5e9980f3e14b83143def0a5527352e0eacdca9ef36e0126919d7acfd96cf3185bcd9b66c0c894f41e66837a53c42cac439116c56dde07d7e293eb4cc745833ec4444028f708b4dda4c269f26eddc70d8a1f12c2ed40408f0ff19f9e381df4b74cd8cca7f64a63ad477730250a16e3dacb8ddaf40db0abab682ca11ed1987016500e8c4ccb17e38bb57efed2aa43dee23b7bbb1c0ed8edb897cce5ea38111214c84c6d5ae0ba82d92de9f978eedda481d857fe";
    const CNG_RSA_PKCS384_HEX: &str = "c75c209005b32e9b1c2d052196b5aff25481af42bb77de6b344d0dd55c7c0d29935b6fc8ef437bcef8d4a70ec78086c8483a75d2170910e455ab23eb19ac163742eaae0004250b64d489e236bc6b8b9c1333631b3ac72b1db845a5652f69109d6185867e7117d8206c6e6a7437c369c09c9387ea072e0bfb6f47c12d85576d5e9fd88f09cdb000031290f2c9170b49889a80cd68f6e2dc13074fe27e2899cd5c8c02b426e6f3919b469fd8bf29ad5fb0c1345468ac7b2c9806954f49e9d13fc329ae861d7a38e38173e3d8745c783fd5050969f2cd59ce2306614902f6f33e29d6a0decfa81c8aa4f24e70421c4f2b457f0da44af9ac2b76fbb12d3cf708492c";
    const CNG_RSA_PKCS512_HEX: &str = "748a05812bb8d9d2f8f934d3fddbee58a9bab0b470ee53880134fd70e5dc42ba1b1c4620677585d8875186a4c29618bd827dfa56ea1ab58169629b4b76927a825ae17c5a47de32a9150048c028af4a851f3e28fe8ab1301a1c1a7000c3e636f8ea6a8afdc6e0ac92a818304cf94efb50df0a505e4c07055869c74cc84f06ab434f68744951c1b31437c60e3b5a048492a20f747c1350e1235dd31322fd782b6252135c452c7cb98155705ddd9a8a71b18399753fd271aa800d846eb6d99a6c6a1df170a5a25d83c464d00034fd250208b3ecd0a2a6fc86fd99701c6bd119e88661cd7ea5a30c0541b4bee21b57ed9448b34a16f832d4b00d5529e2320d2b5e07";
    const CNG_RSA_PSS256_HEX: &str = "36108fd87be9cfa731b5b5f529d4345cbf1fecfaf5802a21c60ce1e68d917bde62ff4da78127809beebc2d38f65bfeed46e6e85a47c498b38ec234ed133c246cc9232cf97e70354217bf376548b157b1f3f701d3797917fae0f0981cb406c2f079e846b65060e8e48c7b959e780913f8bdb0c07535d989f66b0e34ee103c2fba7c8d003d93204610949f388c5e60c9459019802cdfb8c4a71644c65b254e0b12c77c3e715b0d9f1b80e0fd31c0030783c39e2a99d5730deb057bc9c5432eb74c472590414d163eb026adad1406cca1da0d1a2b964dc99dbc9fcceafc3228f7c557c1342ce1ceba62616920febe5e693461bbfe172d9a74979ffecc18e8b4cdae";
    const CNG_RSA_PSS384_HEX: &str = "9458c33f8a2311d7cf8d572d840ec568173e1396abc44cd9e2e5ef5f4bd3b649b6b0a47f8f3a9796617fb1ed7d6277c25489f021d97b4348a777f0525dda36ac75eea4afab5fb80eb58717c6918af13bf3db471c397e48b5ccb1f93047cbbc98458d3552877b4165da13637472142b099ba22e683548da6784a56e8191e828ec24275914ffe475a793a58c8b8f3811da41019d1ad0e7590b566236f6ccb895589e03e0cd510debe4643ddfb94df88e73887497d1d95145b3e44125251ad317cadb0cefe15e87e7d534667f65a8efabe019caf6bd054e8ffca3a54b5293b4262a99521708358ede7dd1b24987fc44783154997a69250c6c25fede314691de2de0";

    /// Independent Windows CNG (`ECDsaCng`) P-256 and P-384 vectors.
    const CNG_P256_QX_HEX: &str =
        "f852464f6b5c35093f9545dee03eb1e82e54965b70c06cf7255d9862f6d13df4";
    const CNG_P256_QY_HEX: &str =
        "9d52a713fa86a0d789abab189297f1bb1fdbaaba77f6d2185585453958435c67";
    const CNG_P256_SIG_HEX: &str = "58b577a7d7c73789b995c45ee776836c16ef9b46955c4158a65c93b9d6e3499cdb86370d671cffba5b33ecb14dd5ff5274d2976e154356640212638f1b60baf2";
    const CNG_P384_QX_HEX: &str = "e69db21bab73fc6a7e8a869fa337d4978cf37968dea26b16e57d5528bbfddab2094c118f19b08809cc682fe5777349a6";
    const CNG_P384_QY_HEX: &str = "20dc3489a4f4b24acf72de57979a8cbb88cc2d75a01e5b9cbf53ac5970a27c216c5e7723a80a29a9f33bd72a1610e4c8";
    const CNG_P384_SIG_HEX: &str = "4f89825428db580ca8105451f2801b37afaf1afbdb9539bffd6d7158edcaf0a695e8d99ca4fe59593771ba8174aa47e800a4f09cbbd2206edeffa1959128c48161ef1c983040fa5b5a5ad89d56e736cc4746f128b29d59c125f1f40f5a2ed0da";

    #[test]
    fn cng_rsa2048_pkcs1_sha256_sha384_sha512_vectors_verify_and_reject_tampering() {
        let n = hex_decode(CNG_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let rsa_pub_der =
            encode_sequence(&[encode_unsigned_integer(&n), encode_unsigned_integer(&e)]);
        let key = parse_rsa_public_key(&rsa_pub_der).expect("parse CNG RSA-2048 key");
        let msg = b"EcclesiaPDF RSA-2048 verification test vector";

        for (hash_alg, sig_hex) in [
            (HashAlgorithm::Sha256, CNG_RSA_PKCS256_HEX),
            (HashAlgorithm::Sha384, CNG_RSA_PKCS384_HEX),
            (HashAlgorithm::Sha512, CNG_RSA_PKCS512_HEX),
        ] {
            let sig = hex_decode(sig_hex);
            verify_rsa_pkcs1_v15(&key, hash_alg, &[msg], &sig)
                .expect("genuine CNG PKCS#1 v1.5 signature must verify");

            let mut tampered_sig = sig.clone();
            tampered_sig[0] ^= 0x01;
            assert!(
                verify_rsa_pkcs1_v15(&key, hash_alg, &[msg], &tampered_sig).is_err(),
                "tampered signature must fail"
            );

            let mut tampered_msg = msg.to_vec();
            tampered_msg[0] ^= 0x01;
            assert!(
                verify_rsa_pkcs1_v15(&key, hash_alg, &[&tampered_msg], &sig).is_err(),
                "tampered message must fail"
            );
        }
    }

    #[test]
    fn cng_rsa2048_pss_sha256_sha384_vectors_verify_and_reject_tampering() {
        let n = hex_decode(CNG_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let rsa_pub_der =
            encode_sequence(&[encode_unsigned_integer(&n), encode_unsigned_integer(&e)]);
        let key = parse_rsa_public_key(&rsa_pub_der).expect("parse CNG RSA-2048 key");
        let msg = b"EcclesiaPDF RSA-2048 verification test vector";

        for (hash_alg, salt_len, sig_hex) in [
            (HashAlgorithm::Sha256, 32, CNG_RSA_PSS256_HEX),
            (HashAlgorithm::Sha384, 48, CNG_RSA_PSS384_HEX),
        ] {
            let pss_params_der = encode_pss_params_der(hash_alg, salt_len as u64);
            let params = parse_pss_params(Some(&pss_params_der)).expect("parse PSS params");
            let sig = hex_decode(sig_hex);
            verify_rsa_pss(&key, params, &[msg], &sig)
                .expect("genuine CNG RSA-PSS signature must verify");

            let mut tampered_sig = sig.clone();
            tampered_sig[255] ^= 0x01;
            assert!(verify_rsa_pss(&key, params, &[msg], &tampered_sig).is_err());

            let wrong_salt = RsaPssParams {
                salt_len: salt_len - 1,
                ..params
            };
            assert!(verify_rsa_pss(&key, wrong_salt, &[msg], &sig).is_err());
        }
    }

    #[test]
    fn cng_ecdsa_p256_and_p384_vectors_verify_and_reject_tampering() {
        // P-256
        let mut pub_256 = vec![0x04];
        pub_256.extend_from_slice(&hex_decode(CNG_P256_QX_HEX));
        pub_256.extend_from_slice(&hex_decode(CNG_P256_QY_HEX));
        let raw_sig_256 = hex_decode(CNG_P256_SIG_HEX);
        let der_sig_256 = encode_sequence(&[
            encode_unsigned_integer(&raw_sig_256[..32]),
            encode_unsigned_integer(&raw_sig_256[32..]),
        ]);
        let msg_256 = b"EcclesiaPDF ECDSA P-256 verification test vector";
        verify_ecdsa(
            OID_SECP256R1,
            &pub_256,
            HashAlgorithm::Sha256,
            &[msg_256],
            &der_sig_256,
        )
        .expect("genuine CNG ECDSA P-256 signature must verify");

        // Tampered message rejected
        assert!(
            verify_ecdsa(
                OID_SECP256R1,
                &pub_256,
                HashAlgorithm::Sha256,
                &[b"EcclesiaPDF ECDSA P-256 verification test vectoR"],
                &der_sig_256
            )
            .is_err()
        );

        // Off-curve public key rejected
        let mut off_curve_256 = pub_256.clone();
        off_curve_256[64] ^= 0x01;
        assert!(
            verify_ecdsa(
                OID_SECP256R1,
                &off_curve_256,
                HashAlgorithm::Sha256,
                &[msg_256],
                &der_sig_256
            )
            .is_err()
        );

        // Zero r or s rejected
        let zero_r_sig = encode_sequence(&[encode_u64(0), encode_u64(1)]);
        assert!(
            verify_ecdsa(
                OID_SECP256R1,
                &pub_256,
                HashAlgorithm::Sha256,
                &[msg_256],
                &zero_r_sig
            )
            .is_err()
        );

        // P-384
        let mut pub_384 = vec![0x04];
        pub_384.extend_from_slice(&hex_decode(CNG_P384_QX_HEX));
        pub_384.extend_from_slice(&hex_decode(CNG_P384_QY_HEX));
        let raw_sig_384 = hex_decode(CNG_P384_SIG_HEX);
        let der_sig_384 = encode_sequence(&[
            encode_unsigned_integer(&raw_sig_384[..48]),
            encode_unsigned_integer(&raw_sig_384[48..]),
        ]);
        let msg_384 = b"EcclesiaPDF ECDSA P-384 verification test vector";
        verify_ecdsa(
            OID_SECP384R1,
            &pub_384,
            HashAlgorithm::Sha384,
            &[msg_384],
            &der_sig_384,
        )
        .expect("genuine CNG ECDSA P-384 signature must verify");

        // Tampered P-384 signature rejected
        let mut bad_raw_384 = raw_sig_384;
        bad_raw_384[95] ^= 0x01;
        let bad_der_384 = encode_sequence(&[
            encode_unsigned_integer(&bad_raw_384[..48]),
            encode_unsigned_integer(&bad_raw_384[48..]),
        ]);
        assert!(
            verify_ecdsa(
                OID_SECP384R1,
                &pub_384,
                HashAlgorithm::Sha384,
                &[msg_384],
                &bad_der_384
            )
            .is_err()
        );
    }

    #[test]
    fn x509_chain_verifies_rsa_pkcs1_rsa_pss_ecdsa_p256_and_p384() {
        let n = hex_decode(TEST_RSA_N_HEX);
        let e = hex_decode(TEST_RSA_E_HEX);
        let d = hex_decode(TEST_RSA_D_HEX);
        let rsa_spki = encode_rsa_spki(&n, &e);

        // 1. RSA PKCS#1 v1.5 chain
        let rsa_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("RSA Root CA"),
            &rsa_spki,
            None,
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::CertificateAuthority,
            100,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa ca");
        let rsa_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("RSA Leaf"),
            &rsa_spki,
            Some(&rsa_ca),
            encode_alg_id(OID_SHA256_WITH_RSA, AlgParams::Null),
            CertRole::EndEntity,
            101,
            |tbs| rsa_pkcs1_sign(&n, &d, HashAlgorithm::Sha256, tbs),
        )
        .expect("rsa leaf");
        let rsa_store = TrustStore::with_anchors([rsa_ca.clone()]);
        assert!(matches!(
            rsa_store.evaluate_chain(&rsa_leaf, &[], Some(1_760_000_000)),
            ChainTrust::Trusted { .. }
        ));
        let mut tampered_rsa_leaf = rsa_leaf;
        tampered_rsa_leaf.tbs_der[10] ^= 0xFF;
        assert!(matches!(
            rsa_store.evaluate_chain(&tampered_rsa_leaf, &[], Some(1_760_000_000)),
            ChainTrust::Tampered(_)
        ));

        // 2. RSA-PSS chain
        let pss_alg = encode_sequence(&[
            encode_oid(OID_RSASSA_PSS),
            encode_pss_params_der(HashAlgorithm::Sha256, 32),
        ]);
        let salt = [0x5Au8; 32];
        let pss_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("RSA-PSS Root CA"),
            &rsa_spki,
            None,
            pss_alg.clone(),
            CertRole::CertificateAuthority,
            200,
            |tbs| rsa_pss_sign(&n, &d, HashAlgorithm::Sha256, &salt, tbs),
        )
        .expect("pss ca");
        let pss_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("RSA-PSS Leaf"),
            &rsa_spki,
            Some(&pss_ca),
            pss_alg,
            CertRole::EndEntity,
            201,
            |tbs| rsa_pss_sign(&n, &d, HashAlgorithm::Sha256, &salt, tbs),
        )
        .expect("pss leaf");
        let pss_store = TrustStore::with_anchors([pss_ca]);
        assert!(matches!(
            pss_store.evaluate_chain(&pss_leaf, &[], Some(1_760_000_000)),
            ChainTrust::Trusted { .. }
        ));

        // 3. ECDSA P-256 chain
        let d256 = [
            0x1234_5678_9ABC_DEF0u64,
            0x0FED_CBA9_8765_4321,
            0x1111,
            0x0001,
        ];
        let k256 = [
            0x2222_3333_4444_5555u64,
            0x6666_7777_8888_9999,
            0x2222,
            0x0002,
        ];
        let spki_256 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP256R1)]),
            encode_bit_string(&ecdsa_p256_pubkey(d256), 0),
        ]);
        let ec256_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-256 CA"),
            &spki_256,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::CertificateAuthority,
            300,
            |tbs| ecdsa_p256_sign(d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 ca");
        let ec256_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-256 Leaf"),
            &spki_256,
            Some(&ec256_ca),
            encode_alg_id(OID_ECDSA_WITH_SHA256, AlgParams::None),
            CertRole::EndEntity,
            301,
            |tbs| ecdsa_p256_sign(d256, k256, HashAlgorithm::Sha256, tbs),
        )
        .expect("p256 leaf");
        let ec256_store = TrustStore::with_anchors([ec256_ca]);
        assert!(matches!(
            ec256_store.evaluate_chain(&ec256_leaf, &[], Some(1_760_000_000)),
            ChainTrust::Trusted { .. }
        ));

        // 4. ECDSA P-384 chain
        let d384 = [
            0x1234_5678_9ABC_DEF0u64,
            0x0FED_CBA9_8765_4321,
            0xAAAA_BBBB_CCCC_DDDD,
            0x1111_2222_3333_4444,
            0x5555,
            0x0001,
        ];
        let k384 = [
            0x7777_8888_9999_AAAAu64,
            0xBBBB_CCCC_DDDD_EEEE,
            0x1357_2468_ACE0_BDF1,
            0x9999_8888_7777_6666,
            0x3333,
            0x0002,
        ];
        let spki_384 = encode_sequence(&[
            encode_sequence(&[encode_oid(OID_EC_PUBLIC_KEY), encode_oid(OID_SECP384R1)]),
            encode_bit_string(&ecdsa_p384_pubkey(d384), 0),
        ]);
        let ec384_ca = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-384 CA"),
            &spki_384,
            None,
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::CertificateAuthority,
            400,
            |tbs| ecdsa_p384_sign(d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 ca");
        let ec384_leaf = issue_custom_cert(
            &DistinguishedName::with_common_name("ECDSA P-384 Leaf"),
            &spki_384,
            Some(&ec384_ca),
            encode_alg_id(OID_ECDSA_WITH_SHA384, AlgParams::None),
            CertRole::EndEntity,
            401,
            |tbs| ecdsa_p384_sign(d384, k384, HashAlgorithm::Sha384, tbs),
        )
        .expect("p384 leaf");
        let ec384_store = TrustStore::with_anchors([ec384_ca]);
        assert!(matches!(
            ec384_store.evaluate_chain(&ec384_leaf, &[], Some(1_760_000_000)),
            ChainTrust::Trusted { .. }
        ));
    }
}
