//! ASN.1 DER encoder/decoder, X.509 v3 Ed25519 certificate parser/builder, and trust store
//! (plan O.14, O.15, O.20).

use std::fmt;
use std::io;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

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
pub(crate) const OID_ED25519: &[u32] = &[1, 3, 101, 112];
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
    Mupdf(excellent_mupdf::MuError),
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

impl From<excellent_mupdf::MuError> for SigningError {
    fn from(err: excellent_mupdf::MuError) -> Self {
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
            val = (val << 7) | u32::from(b & 0x7F);
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
    /// Subject public key bytes.
    pub public_key_bytes: Vec<u8>,
    /// Certificate signature algorithm OID.
    pub signature_algorithm_oid: Vec<u32>,
    /// Certificate signature bytes.
    pub signature_bytes: Vec<u8>,
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

        let subject = DistinguishedName::from_der_node(tbs_children[idx + 4])?;

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
        let public_key_bytes = spki_children[1].as_bit_string_bytes()?.to_vec();

        let mut is_ca = false;
        let mut is_tsa = false;
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
                    }
                }
            }
        }

        Ok(Self {
            raw_der: der.to_vec(),
            tbs_der: tbs.full.to_vec(),
            issuer_der,
            serial,
            issuer,
            subject,
            not_before,
            not_after,
            public_key_oid,
            public_key_bytes,
            signature_algorithm_oid,
            signature_bytes,
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
    /// # Errors
    ///
    /// Returns [`SigningError::Crypto`] if the issuer mismatch, unsupported key, or invalid signature is detected.
    pub fn verify_issued_by(&self, issuer: &Self) -> Result<(), SigningError> {
        if self.issuer_der != issuer.issuer_der_for_subject() && self.issuer != issuer.subject {
            return Err(SigningError::Crypto(
                "certificate issuer does not match parent subject".to_owned(),
            ));
        }
        if self.signature_algorithm_oid != OID_ED25519 || issuer.public_key_oid != OID_ED25519 {
            return Err(SigningError::Crypto(
                "unsupported non-Ed25519 certificate algorithm".to_owned(),
            ));
        }
        let vk = VerifyingKey::try_from(issuer.public_key_bytes.as_slice())
            .map_err(|err| SigningError::Crypto(format!("invalid issuer public key: {err}")))?;
        let sig = Signature::from_slice(&self.signature_bytes)
            .map_err(|err| SigningError::Crypto(format!("invalid certificate signature: {err}")))?;
        vk.verify(&self.tbs_der, &sig)
            .map_err(|err| SigningError::Crypto(format!("certificate signature failed: {err}")))
    }

    fn issuer_der_for_subject(&self) -> Vec<u8> {
        self.subject.to_der()
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
