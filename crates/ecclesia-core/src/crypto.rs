//! PDF permissions enforcement, password/RC4/AES-256 security descriptors, and document
//! sanitization (plan O.19).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sha2::{Digest, Sha256};

use crate::page_ops::pdf_doc::{
    ParsedPdf, PdfDocError, PdfVal, RawPdfWriter, rewrite_clean, serialize_val,
};

/// Result alias for security and sanitization operations.
pub type Result<T> = std::result::Result<T, PdfCryptoError>;

/// Error returned by PDF encryption, permission enforcement, or sanitization.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum PdfCryptoError {
    /// Underlying PDF parsing or structure error.
    Pdf(PdfDocError),
    /// Supplied password did not match user or owner credentials.
    WrongPassword,
    /// Document already has an encryption dictionary.
    AlreadyEncrypted,
    /// Document is not encrypted when decryption was requested.
    NotEncrypted,
    /// Requested action is forbidden by the document's `/P` permission flags.
    PermissionDenied(DocumentAction),
}

impl fmt::Display for PdfCryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pdf(err) => write!(f, "{err}"),
            Self::WrongPassword => f.write_str("incorrect PDF password"),
            Self::AlreadyEncrypted => f.write_str("document is already encrypted"),
            Self::NotEncrypted => f.write_str("document is not encrypted"),
            Self::PermissionDenied(action) => {
                write!(f, "document permissions disallow {action}")
            }
        }
    }
}

impl std::error::Error for PdfCryptoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pdf(err) => Some(err),
            _ => None,
        }
    }
}

impl From<PdfDocError> for PdfCryptoError {
    fn from(err: PdfDocError) -> Self {
        Self::Pdf(err)
    }
}

/// User actions governed by ISO 32000 PDF `/P` permission flags (plan O.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DocumentAction {
    /// Print at low resolution.
    PrintLowRes,
    /// Print at high resolution.
    PrintHighRes,
    /// Modify document contents (pages, text, images).
    ModifyContents,
    /// Copy text and graphics.
    CopyContent,
    /// Add or modify annotations and fill forms.
    ModifyAnnotations,
    /// Fill interactive form fields.
    FillForms,
    /// Extract text/graphics for accessibility.
    ExtractAccessibility,
    /// Insert, rotate, or delete pages and create bookmarks.
    AssembleDocument,
}

impl fmt::Display for DocumentAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::PrintLowRes => "printing",
            Self::PrintHighRes => "high-resolution printing",
            Self::ModifyContents => "modifying document contents",
            Self::CopyContent => "copying text or graphics",
            Self::ModifyAnnotations => "modifying annotations",
            Self::FillForms => "filling form fields",
            Self::ExtractAccessibility => "accessibility extraction",
            Self::AssembleDocument => "assembling or reordering pages",
        };
        f.write_str(label)
    }
}

/// Whether a specific permission flag is granted or denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PermissionGrant {
    /// Action is permitted.
    #[default]
    Allow,
    /// Action is prohibited.
    Deny,
}

/// PDF Standard Security Handler `/P` permissions bitmask (ISO 32000-1 Table 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PdfPermissions {
    /// Bit 3: Print document.
    pub print_low_res: PermissionGrant,
    /// Bit 12: High-resolution printing.
    pub print_high_res: PermissionGrant,
    /// Bit 4: Modify contents other than annotations/forms.
    pub modify_contents: PermissionGrant,
    /// Bit 5: Copy or extract text and graphics.
    pub copy_content: PermissionGrant,
    /// Bit 6: Add or modify annotations and fill form fields.
    pub modify_annotations: PermissionGrant,
    /// Bit 9: Fill existing interactive form fields.
    pub fill_forms: PermissionGrant,
    /// Bit 10: Extract text/graphics for accessibility.
    pub extract_accessibility: PermissionGrant,
    /// Bit 11: Assemble (insert, rotate, delete pages, create bookmarks).
    pub assemble_document: PermissionGrant,
}

impl Default for PdfPermissions {
    fn default() -> Self {
        Self::allow_all()
    }
}

impl PdfPermissions {
    /// Grants every permission bit.
    #[must_use]
    pub const fn allow_all() -> Self {
        Self {
            print_low_res: PermissionGrant::Allow,
            print_high_res: PermissionGrant::Allow,
            modify_contents: PermissionGrant::Allow,
            copy_content: PermissionGrant::Allow,
            modify_annotations: PermissionGrant::Allow,
            fill_forms: PermissionGrant::Allow,
            extract_accessibility: PermissionGrant::Allow,
            assemble_document: PermissionGrant::Allow,
        }
    }

    /// Read-only permissions: printing and accessibility allowed, editing and copying denied.
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            print_low_res: PermissionGrant::Allow,
            print_high_res: PermissionGrant::Deny,
            modify_contents: PermissionGrant::Deny,
            copy_content: PermissionGrant::Deny,
            modify_annotations: PermissionGrant::Deny,
            fill_forms: PermissionGrant::Deny,
            extract_accessibility: PermissionGrant::Allow,
            assemble_document: PermissionGrant::Deny,
        }
    }

    /// Encodes permissions into the signed 32-bit PDF `/P` integer.
    #[must_use]
    pub const fn to_pdf_p(self) -> i32 {
        // Required reserved bits 1-2 = 0, bits 7-8 = 1, bits 13-32 = 1 => base 0xFFFF_F0C0u32
        let mut bits: u32 = 0xFFFF_F0C0;
        if matches!(self.print_low_res, PermissionGrant::Allow) {
            bits |= 1 << 2;
        }
        if matches!(self.modify_contents, PermissionGrant::Allow) {
            bits |= 1 << 3;
        }
        if matches!(self.copy_content, PermissionGrant::Allow) {
            bits |= 1 << 4;
        }
        if matches!(self.modify_annotations, PermissionGrant::Allow) {
            bits |= 1 << 5;
        }
        if matches!(self.fill_forms, PermissionGrant::Allow) {
            bits |= 1 << 8;
        }
        if matches!(self.extract_accessibility, PermissionGrant::Allow) {
            bits |= 1 << 9;
        }
        if matches!(self.assemble_document, PermissionGrant::Allow) {
            bits |= 1 << 10;
        }
        if matches!(self.print_high_res, PermissionGrant::Allow) {
            bits |= 1 << 11;
        }
        bits as i32
    }

    /// Decodes a signed 32-bit PDF `/P` integer into [`PdfPermissions`].
    #[must_use]
    pub const fn from_pdf_p(p: i32) -> Self {
        const fn bit(bits: u32, n: u32) -> PermissionGrant {
            if (bits & (1 << n)) != 0 {
                PermissionGrant::Allow
            } else {
                PermissionGrant::Deny
            }
        }
        let bits = p as u32;
        Self {
            print_low_res: bit(bits, 2),
            modify_contents: bit(bits, 3),
            copy_content: bit(bits, 4),
            modify_annotations: bit(bits, 5),
            fill_forms: bit(bits, 8),
            extract_accessibility: bit(bits, 9),
            assemble_document: bit(bits, 10),
            print_high_res: bit(bits, 11),
        }
    }

    /// Checks whether `action` is permitted.
    #[must_use]
    pub const fn allows(self, action: DocumentAction) -> bool {
        match action {
            DocumentAction::PrintLowRes => matches!(self.print_low_res, PermissionGrant::Allow),
            DocumentAction::PrintHighRes => {
                matches!(self.print_low_res, PermissionGrant::Allow)
                    && matches!(self.print_high_res, PermissionGrant::Allow)
            }
            DocumentAction::ModifyContents => {
                matches!(self.modify_contents, PermissionGrant::Allow)
            }
            DocumentAction::CopyContent => matches!(self.copy_content, PermissionGrant::Allow),
            DocumentAction::ModifyAnnotations => {
                matches!(self.modify_annotations, PermissionGrant::Allow)
            }
            DocumentAction::FillForms => matches!(self.fill_forms, PermissionGrant::Allow),
            DocumentAction::ExtractAccessibility => {
                matches!(self.extract_accessibility, PermissionGrant::Allow)
            }
            DocumentAction::AssembleDocument => {
                matches!(self.assemble_document, PermissionGrant::Allow)
            }
        }
    }
}

/// Checks `permissions` for `action` and returns a clear user-facing error if denied (plan O.19).
///
/// # Errors
///
/// Returns [`PdfCryptoError::PermissionDenied`] if the permission is denied.
pub fn enforce_permission(permissions: PdfPermissions, action: DocumentAction) -> Result<()> {
    if permissions.allows(action) {
        Ok(())
    } else {
        Err(PdfCryptoError::PermissionDenied(action))
    }
}

/// PDF encryption cipher profile (plan O.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum EncryptionAlgorithm {
    /// RC4 128-bit (`/V 2 /R 3`).
    Rc4_128,
    /// AES 256-bit (`/V 5 /R 6`, ISO 32000-2).
    #[default]
    Aes256,
}

/// Whether XMP `/Metadata` stream is encrypted alongside strings/streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MetadataEncryption {
    /// Encrypt metadata stream as well.
    #[default]
    Encrypted,
    /// Keep metadata plaintext for search indexers.
    Plaintext,
}

/// Encryption policy applied by [`encrypt_pdf`] (plan O.19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPolicy {
    /// Cipher algorithm.
    pub algorithm: EncryptionAlgorithm,
    /// Password required to open the document (may be empty for permission-only owner lock).
    pub user_password: String,
    /// Owner password required to change permissions or remove encryption.
    pub owner_password: String,
    /// Permissions granted to non-owner users.
    pub permissions: PdfPermissions,
    /// Whether metadata is encrypted.
    pub encrypt_metadata: MetadataEncryption,
}

/// Parsed summary of a PDF's `/Encrypt` dictionary (plan O.19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionDescriptor {
    /// Security handler version (`/V`).
    pub version: i64,
    /// Revision (`/R`).
    pub revision: i64,
    /// Key length in bits.
    pub key_bits: u32,
    /// Algorithm classification.
    pub algorithm: EncryptionAlgorithm,
    /// Decoded `/P` permissions.
    pub permissions: PdfPermissions,
    /// Whether metadata is encrypted (`/EncryptMetadata`).
    pub metadata_encrypted: MetadataEncryption,
}

/// Inspects a PDF's `/Encrypt` dictionary without decrypting streams (plan O.19).
///
/// # Errors
///
/// Returns [`PdfCryptoError::Pdf`] if the PDF cannot be parsed.
pub fn inspect_encryption(pdf_bytes: &[u8]) -> Result<Option<EncryptionDescriptor>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let Some(enc_id) = doc.encrypt_id else {
        return Ok(None);
    };
    let Some(dict) = doc.get_dict(enc_id) else {
        return Ok(None);
    };
    let version = dict.get("V").and_then(PdfVal::as_i64).unwrap_or(0);
    let revision = dict.get("R").and_then(PdfVal::as_i64).unwrap_or(0);
    let key_bits = dict
        .get("Length")
        .and_then(PdfVal::as_i64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(if version >= 5 { 256 } else { 128 });
    let p_val = dict.get("P").and_then(PdfVal::as_i64).unwrap_or(-1) as i32;
    let algorithm = if version >= 5 {
        EncryptionAlgorithm::Aes256
    } else {
        EncryptionAlgorithm::Rc4_128
    };
    let metadata_encrypted = match dict.get("EncryptMetadata") {
        Some(PdfVal::Bool(false)) => MetadataEncryption::Plaintext,
        _ => MetadataEncryption::Encrypted,
    };
    Ok(Some(EncryptionDescriptor {
        version,
        revision,
        key_bits,
        algorithm,
        permissions: PdfPermissions::from_pdf_p(p_val),
        metadata_encrypted,
    }))
}

/// Encrypts `pdf_bytes` with Standard Security Handler (`/V 2 /R 3` RC4-128 or `/V 5 /R 6` AES-256)
/// (plan O.19).
///
/// # Errors
///
/// Returns [`PdfCryptoError::AlreadyEncrypted`] if already encrypted, or [`PdfCryptoError::Pdf`] on
/// parse errors.
pub fn encrypt_pdf(pdf_bytes: &[u8], policy: &SecurityPolicy) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    if doc.encrypt_id.is_some() {
        return Err(PdfCryptoError::AlreadyEncrypted);
    }

    let file_id: [u8; 16] = Sha256::digest(pdf_bytes)[..16]
        .try_into()
        .expect("16 bytes");
    let p_int = policy.permissions.to_pdf_p();
    let owner_pw = if policy.owner_password.is_empty() {
        policy.user_password.as_str()
    } else {
        policy.owner_password.as_str()
    };

    let mut writer = RawPdfWriter::new();
    let enc_id = doc.next_obj_id();

    let (enc_dict, key, algo) = match policy.algorithm {
        EncryptionAlgorithm::Rc4_128 => {
            let (o_val, u_val, key) = derive_rc4_128_keys(
                policy.user_password.as_bytes(),
                owner_pw.as_bytes(),
                p_int,
                &file_id,
            );
            let mut d = BTreeMap::new();
            d.insert("Filter".into(), PdfVal::Name("Standard".into()));
            d.insert("V".into(), PdfVal::Int(2));
            d.insert("R".into(), PdfVal::Int(3));
            d.insert("Length".into(), PdfVal::Int(128));
            d.insert("P".into(), PdfVal::Int(i64::from(p_int)));
            d.insert("O".into(), PdfVal::HexStr(o_val.to_vec()));
            d.insert("U".into(), PdfVal::HexStr(u_val.to_vec()));
            (d, key.to_vec(), EncryptionAlgorithm::Rc4_128)
        }
        EncryptionAlgorithm::Aes256 => {
            let file_key: [u8; 32] =
                Sha256::digest([b"ExPdfAes256".as_slice(), &file_id, owner_pw.as_bytes()].concat())
                    .into();
            let entries = build_aes256_entries(
                policy.user_password.as_bytes(),
                owner_pw.as_bytes(),
                p_int,
                &file_key,
                policy.encrypt_metadata == MetadataEncryption::Encrypted,
            );
            let mut cf_entry = BTreeMap::new();
            cf_entry.insert("AuthEvent".into(), PdfVal::Name("DocOpen".into()));
            cf_entry.insert("CFM".into(), PdfVal::Name("AESV3".into()));
            cf_entry.insert("Length".into(), PdfVal::Int(32));
            let mut cf = BTreeMap::new();
            cf.insert("StdCF".into(), PdfVal::Dict(cf_entry));

            let mut d = BTreeMap::new();
            d.insert("Filter".into(), PdfVal::Name("Standard".into()));
            d.insert("V".into(), PdfVal::Int(5));
            d.insert("R".into(), PdfVal::Int(6));
            d.insert("Length".into(), PdfVal::Int(256));
            d.insert("CF".into(), PdfVal::Dict(cf));
            d.insert("StmF".into(), PdfVal::Name("StdCF".into()));
            d.insert("StrF".into(), PdfVal::Name("StdCF".into()));
            d.insert("P".into(), PdfVal::Int(i64::from(p_int)));
            d.insert("O".into(), PdfVal::HexStr(entries.o_val.to_vec()));
            d.insert("U".into(), PdfVal::HexStr(entries.u_val.to_vec()));
            d.insert("OE".into(), PdfVal::HexStr(entries.oe_val.to_vec()));
            d.insert("UE".into(), PdfVal::HexStr(entries.ue_val.to_vec()));
            d.insert("Perms".into(), PdfVal::HexStr(entries.perms_val.to_vec()));
            if policy.encrypt_metadata == MetadataEncryption::Plaintext {
                d.insert("EncryptMetadata".into(), PdfVal::Bool(false));
            }
            (d, file_key.to_vec(), EncryptionAlgorithm::Aes256)
        }
    };

    for id in doc.object_ids() {
        let Some(val) = doc.resolve_obj(id) else {
            continue;
        };
        let is_meta_stm = matches!(
            &val,
            PdfVal::Stream { dict, .. }
                if matches!(dict.get("Type").and_then(PdfVal::as_name), Some("XRef" | "ObjStm"))
        );
        if is_meta_stm {
            continue;
        }
        let encrypted_val = encrypt_pdf_val(&val, id, 0, &key, algo, policy.encrypt_metadata);
        writer.set(id, serialize_val(&encrypted_val));
    }
    writer.set(enc_id, serialize_val(&PdfVal::Dict(enc_dict)));
    Ok(writer.finish_with_encrypt(doc.root_id, doc.info_id, Some(enc_id), Some(file_id)))
}

/// Removes encryption from `pdf_bytes` when `password` matches the user or owner password
/// (plan O.19).
///
/// # Errors
///
/// Returns [`PdfCryptoError::NotEncrypted`] if not encrypted or [`PdfCryptoError::WrongPassword`]
/// if `password` is invalid.
pub fn decrypt_pdf(pdf_bytes: &[u8], password: &str) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let enc_id = doc.encrypt_id.ok_or(PdfCryptoError::NotEncrypted)?;
    let enc_dict = doc
        .get_dict(enc_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("Encrypt dictionary missing".into()))?;
    let desc = inspect_encryption(pdf_bytes)?.ok_or(PdfCryptoError::NotEncrypted)?;

    let file_id = extract_trailer_id(pdf_bytes).unwrap_or([0u8; 16]);
    let key = match desc.algorithm {
        EncryptionAlgorithm::Rc4_128 => {
            authenticate_rc4_128(&enc_dict, password.as_bytes(), &file_id)?.to_vec()
        }
        EncryptionAlgorithm::Aes256 => {
            authenticate_aes256(&enc_dict, password.as_bytes())?.to_vec()
        }
    };

    let mut overrides = BTreeMap::new();
    let mut dropped = BTreeSet::new();
    dropped.insert(enc_id);

    for id in doc.object_ids() {
        if id == enc_id {
            continue;
        }
        let Some(val) = doc.resolve_obj(id) else {
            continue;
        };
        let decrypted = decrypt_pdf_val(&val, id, 0, &key, desc.algorithm, desc.metadata_encrypted);
        overrides.insert(id, decrypted);
    }

    Ok(rewrite_clean(&doc, &overrides, &BTreeMap::new(), &dropped))
}

/// Options for [`sanitize_pdf`] (plan O.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeOptions {
    /// Remove `/Info` and XMP `/Metadata`.
    pub strip_metadata: SanitizeAction,
    /// Remove `/EmbeddedFiles` attachments.
    pub strip_attachments: SanitizeAction,
    /// Remove `/Names /JavaScript`, `/OpenAction`, and `/AA` scripts/actions.
    pub strip_scripts_and_actions: SanitizeAction,
    /// Remove `/OCProperties` hidden layers.
    pub strip_hidden_layers: SanitizeAction,
    /// Remove page `/Thumb` thumbnail streams.
    pub strip_thumbnails: SanitizeAction,
    /// Remove `/Annots` comments/markups (preserving `/Link` unless desired).
    pub strip_comments: SanitizeAction,
}

/// Whether a sanitization category is stripped or retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SanitizeAction {
    /// Strip this category.
    #[default]
    Strip,
    /// Keep this category intact.
    Keep,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self {
            strip_metadata: SanitizeAction::Strip,
            strip_attachments: SanitizeAction::Strip,
            strip_scripts_and_actions: SanitizeAction::Strip,
            strip_hidden_layers: SanitizeAction::Strip,
            strip_thumbnails: SanitizeAction::Strip,
            strip_comments: SanitizeAction::Strip,
        }
    }
}

/// Sanitizes `pdf_bytes` by stripping metadata, scripts, attachments, comments, thumbnails, and
/// hidden layers and rewriting cleanly (plan O.19).
///
/// # Errors
///
/// Returns [`PdfCryptoError::Pdf`] if `pdf_bytes` cannot be parsed.
pub fn sanitize_pdf(pdf_bytes: &[u8], options: SanitizeOptions) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let mut overrides = BTreeMap::new();
    let mut dropped = BTreeSet::new();

    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("catalog missing".into()))?;

    if options.strip_metadata == SanitizeAction::Strip {
        if let Some(meta_id) = catalog.remove("Metadata").and_then(|v| v.as_ref_id()) {
            dropped.insert(meta_id);
        }
        if let Some(info_id) = doc.info_id {
            dropped.insert(info_id);
        }
    }

    if options.strip_scripts_and_actions == SanitizeAction::Strip {
        catalog.remove("OpenAction");
        catalog.remove("AA");
    }

    if (options.strip_attachments == SanitizeAction::Strip
        || options.strip_scripts_and_actions == SanitizeAction::Strip)
        && let Some(names_val) = catalog.get("Names").cloned()
        && let Some(PdfVal::Dict(mut names)) = doc.resolve_val(&names_val)
    {
        if options.strip_attachments == SanitizeAction::Strip {
            names.remove("EmbeddedFiles");
        }
        if options.strip_scripts_and_actions == SanitizeAction::Strip {
            names.remove("JavaScript");
        }
        catalog.insert("Names".into(), PdfVal::Dict(names));
    }

    if options.strip_hidden_layers == SanitizeAction::Strip {
        catalog.remove("OCProperties");
    }
    overrides.insert(doc.root_id, PdfVal::Dict(catalog));

    let pages = doc.page_objects()?;
    for pid in pages {
        let Some(mut pdict) = doc.get_dict(pid) else {
            continue;
        };
        let mut changed = false;
        if options.strip_thumbnails == SanitizeAction::Strip
            && let Some(tid) = pdict.remove("Thumb").and_then(|v| v.as_ref_id())
        {
            dropped.insert(tid);
            changed = true;
        }
        if options.strip_scripts_and_actions == SanitizeAction::Strip
            && pdict.remove("AA").is_some()
        {
            changed = true;
        }
        if options.strip_comments == SanitizeAction::Strip
            && let Some(annots_val) = pdict.get("Annots").cloned()
            && let Some(PdfVal::Array(annots)) = doc.resolve_val(&annots_val)
        {
            let mut kept = Vec::new();
            for item in annots {
                let keep = item
                    .as_ref_id()
                    .and_then(|aid| doc.get_dict(aid))
                    .is_some_and(|ad| ad.get("Subtype").and_then(PdfVal::as_name) == Some("Link"));
                if keep {
                    kept.push(item);
                } else if let Some(aid) = item.as_ref_id() {
                    dropped.insert(aid);
                }
            }
            pdict.insert("Annots".into(), PdfVal::Array(kept));
            changed = true;
        }
        if changed {
            overrides.insert(pid, PdfVal::Dict(pdict));
        }
    }

    Ok(rewrite_clean(&doc, &overrides, &BTreeMap::new(), &dropped))
}

// ---------------------------------------------------------------------------
// RC4-128 & AES-256-CBC pure-Rust cryptographic primitives
// ---------------------------------------------------------------------------

const PDF_PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

fn pad_password(pw: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    if n < 32 {
        out[n..].copy_from_slice(&PDF_PADDING[..32 - n]);
    }
    out
}

fn derive_rc4_128_keys(
    user_pw: &[u8],
    owner_pw: &[u8],
    p_int: i32,
    file_id: &[u8; 16],
) -> ([u8; 32], [u8; 32], [u8; 16]) {
    let mut owner_key = md5(&pad_password(owner_pw));
    for _ in 0..50 {
        owner_key = md5(&owner_key);
    }
    let mut o_entry = pad_password(user_pw);
    for round in 0..20u8 {
        let rk: [u8; 16] = owner_key.map(|b| b ^ round);
        o_entry = rc4_apply_fixed(&rk, &o_entry);
    }

    let enc_key = compute_rc4_128_enc_key(user_pw, &o_entry, p_int, file_id);
    let mut u_hash_in = Vec::with_capacity(48);
    u_hash_in.extend_from_slice(&PDF_PADDING);
    u_hash_in.extend_from_slice(file_id);
    let mut u_digest = md5(&u_hash_in);
    for round in 0..20u8 {
        let rk: [u8; 16] = enc_key.map(|b| b ^ round);
        u_digest = rc4_apply_fixed(&rk, &u_digest);
    }
    let mut u_entry = [0u8; 32];
    u_entry[..16].copy_from_slice(&u_digest);
    (o_entry, u_entry, enc_key)
}

fn compute_rc4_128_enc_key(
    user_pw: &[u8],
    o_entry: &[u8; 32],
    p_int: i32,
    file_id: &[u8; 16],
) -> [u8; 16] {
    let mut buf = Vec::with_capacity(84);
    buf.extend_from_slice(&pad_password(user_pw));
    buf.extend_from_slice(o_entry);
    buf.extend_from_slice(&p_int.to_le_bytes());
    buf.extend_from_slice(file_id);
    let mut digest = md5(&buf);
    for _ in 0..50 {
        digest = md5(&digest);
    }
    digest
}

fn authenticate_rc4_128(
    enc_dict: &BTreeMap<String, PdfVal>,
    password: &[u8],
    file_id: &[u8; 16],
) -> Result<[u8; 16]> {
    let o_bytes = enc_dict
        .get("O")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /O in /Encrypt".into()))?;
    let u_bytes = enc_dict
        .get("U")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /U in /Encrypt".into()))?;
    if o_bytes.len() < 32 || u_bytes.len() < 16 {
        return Err(PdfCryptoError::WrongPassword);
    }
    let mut o_arr = [0u8; 32];
    o_arr.copy_from_slice(&o_bytes[..32]);
    let p_int = enc_dict.get("P").and_then(PdfVal::as_i64).unwrap_or(-1) as i32;

    let check_user = |candidate_pw: &[u8]| -> Option<[u8; 16]> {
        let key = compute_rc4_128_enc_key(candidate_pw, &o_arr, p_int, file_id);
        let mut u_hash_in = Vec::with_capacity(48);
        u_hash_in.extend_from_slice(&PDF_PADDING);
        u_hash_in.extend_from_slice(file_id);
        let mut d = md5(&u_hash_in);
        for round in 0..20u8 {
            let rk: [u8; 16] = key.map(|b| b ^ round);
            d = rc4_apply_fixed(&rk, &d);
        }
        (d == u_bytes[..16]).then_some(key)
    };

    if let Some(key) = check_user(password) {
        return Ok(key);
    }

    // Try owner password recovery.
    let mut owner_key = md5(&pad_password(password));
    for _ in 0..50 {
        owner_key = md5(&owner_key);
    }
    let mut recovered_user = o_arr;
    for round in (0..20u8).rev() {
        let rk: [u8; 16] = owner_key.map(|b| b ^ round);
        recovered_user = rc4_apply_fixed(&rk, &recovered_user);
    }
    check_user(&recovered_user).ok_or(PdfCryptoError::WrongPassword)
}

struct Aes256Entries {
    u_val: [u8; 48],
    ue_val: [u8; 32],
    o_val: [u8; 48],
    oe_val: [u8; 32],
    perms_val: [u8; 16],
}

fn build_aes256_entries(
    user_pw: &[u8],
    owner_pw: &[u8],
    p_int: i32,
    file_key: &[u8; 32],
    encrypt_metadata: bool,
) -> Aes256Entries {
    let u_val_salt: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let u_key_salt: [u8; 8] = [0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x01];
    let u_hash = sha256_iterated(user_pw, &u_val_salt, &[]);
    let mut u_entry = [0u8; 48];
    u_entry[..32].copy_from_slice(&u_hash);
    u_entry[32..40].copy_from_slice(&u_val_salt);
    u_entry[40..48].copy_from_slice(&u_key_salt);

    let ue_key = sha256_iterated(user_pw, &u_key_salt, &[]);
    let ue_entry = aes256_cbc_encrypt_no_pad(&ue_key, &[0u8; 16], file_key);
    let mut ue_arr = [0u8; 32];
    ue_arr.copy_from_slice(&ue_entry[..32]);

    let o_val_salt: [u8; 8] = [0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98];
    let o_key_salt: [u8; 8] = [0xA9, 0xBA, 0xCB, 0xDC, 0xED, 0xFE, 0x0F, 0x10];
    let o_hash = sha256_iterated(owner_pw, &o_val_salt, &u_entry);
    let mut o_entry = [0u8; 48];
    o_entry[..32].copy_from_slice(&o_hash);
    o_entry[32..40].copy_from_slice(&o_val_salt);
    o_entry[40..48].copy_from_slice(&o_key_salt);

    let oe_key = sha256_iterated(owner_pw, &o_key_salt, &u_entry);
    let oe_entry = aes256_cbc_encrypt_no_pad(&oe_key, &[0u8; 16], file_key);
    let mut oe_arr = [0u8; 32];
    oe_arr.copy_from_slice(&oe_entry[..32]);

    let mut perms_plain = [0u8; 16];
    let p_bytes = (p_int as u32).to_le_bytes();
    perms_plain[..4].copy_from_slice(&p_bytes);
    perms_plain[4..8].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    perms_plain[8] = if encrypt_metadata { b'T' } else { b'F' };
    perms_plain[9..12].copy_from_slice(b"adb");
    let perms_enc = aes256_ecb_encrypt_block(file_key, &perms_plain);

    Aes256Entries {
        u_val: u_entry,
        ue_val: ue_arr,
        o_val: o_entry,
        oe_val: oe_arr,
        perms_val: perms_enc,
    }
}

fn authenticate_aes256(enc_dict: &BTreeMap<String, PdfVal>, password: &[u8]) -> Result<[u8; 32]> {
    let u = enc_dict
        .get("U")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /U".into()))?;
    let ue = enc_dict
        .get("UE")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /UE".into()))?;
    let o = enc_dict
        .get("O")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /O".into()))?;
    let oe = enc_dict
        .get("OE")
        .and_then(PdfVal::as_bytes)
        .ok_or_else(|| PdfDocError::InvalidPdf("missing /OE".into()))?;
    if u.len() < 48 || ue.len() < 32 || o.len() < 48 || oe.len() < 32 {
        return Err(PdfCryptoError::WrongPassword);
    }

    let u_check = sha256_iterated(password, &u[32..40], &[]);
    if u_check == u[..32] {
        let key = sha256_iterated(password, &u[40..48], &[]);
        let fk = aes256_cbc_decrypt_no_pad(&key, &[0u8; 16], &ue[..32]);
        let mut out = [0u8; 32];
        out.copy_from_slice(&fk[..32]);
        return Ok(out);
    }

    let o_check = sha256_iterated(password, &o[32..40], &u[..48]);
    if o_check == o[..32] {
        let key = sha256_iterated(password, &o[40..48], &u[..48]);
        let fk = aes256_cbc_decrypt_no_pad(&key, &[0u8; 16], &oe[..32]);
        let mut out = [0u8; 32];
        out.copy_from_slice(&fk[..32]);
        return Ok(out);
    }

    Err(PdfCryptoError::WrongPassword)
}

fn sha256_iterated(password: &[u8], salt: &[u8], u_bytes: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(password.len() + salt.len() + u_bytes.len());
    input.extend_from_slice(password);
    input.extend_from_slice(salt);
    input.extend_from_slice(u_bytes);
    Sha256::digest(&input).into()
}

fn obj_rc4_key(file_key: &[u8], obj_id: u32, gen_num: u16) -> [u8; 16] {
    let mut buf = Vec::with_capacity(file_key.len() + 5);
    buf.extend_from_slice(file_key);
    buf.extend_from_slice(&obj_id.to_le_bytes()[..3]);
    buf.extend_from_slice(&gen_num.to_le_bytes());
    md5(&buf)
}

fn encrypt_pdf_val(
    val: &PdfVal,
    obj_id: u32,
    gen_num: u16,
    key: &[u8],
    algo: EncryptionAlgorithm,
    meta_mode: MetadataEncryption,
) -> PdfVal {
    match val {
        PdfVal::Str(bytes) => {
            let enc = encrypt_bytes(bytes, obj_id, gen_num, key, algo);
            PdfVal::HexStr(enc)
        }
        PdfVal::HexStr(bytes) => {
            let enc = encrypt_bytes(bytes, obj_id, gen_num, key, algo);
            PdfVal::HexStr(enc)
        }
        PdfVal::Array(arr) => PdfVal::Array(
            arr.iter()
                .map(|v| encrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode))
                .collect(),
        ),
        PdfVal::Dict(d) => PdfVal::Dict(
            d.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        encrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode),
                    )
                })
                .collect(),
        ),
        PdfVal::Stream { dict, data } => {
            let skip_meta = meta_mode == MetadataEncryption::Plaintext
                && dict.get("Type").and_then(PdfVal::as_name) == Some("Metadata");
            let new_data = if skip_meta {
                data.clone()
            } else {
                encrypt_bytes(data, obj_id, gen_num, key, algo)
            };
            let mut new_dict = BTreeMap::new();
            for (k, v) in dict {
                new_dict.insert(
                    k.clone(),
                    encrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode),
                );
            }
            new_dict.insert("Length".into(), PdfVal::Int(new_data.len() as i64));
            PdfVal::Stream {
                dict: new_dict,
                data: new_data,
            }
        }
        other => other.clone(),
    }
}

fn decrypt_pdf_val(
    val: &PdfVal,
    obj_id: u32,
    gen_num: u16,
    key: &[u8],
    algo: EncryptionAlgorithm,
    meta_mode: MetadataEncryption,
) -> PdfVal {
    match val {
        PdfVal::Str(bytes) | PdfVal::HexStr(bytes) => {
            let dec = decrypt_bytes(bytes, obj_id, gen_num, key, algo);
            PdfVal::Str(dec)
        }
        PdfVal::Array(arr) => PdfVal::Array(
            arr.iter()
                .map(|v| decrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode))
                .collect(),
        ),
        PdfVal::Dict(d) => PdfVal::Dict(
            d.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        decrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode),
                    )
                })
                .collect(),
        ),
        PdfVal::Stream { dict, data } => {
            let skip_meta = meta_mode == MetadataEncryption::Plaintext
                && dict.get("Type").and_then(PdfVal::as_name) == Some("Metadata");
            let new_data = if skip_meta {
                data.clone()
            } else {
                decrypt_bytes(data, obj_id, gen_num, key, algo)
            };
            let mut new_dict = BTreeMap::new();
            for (k, v) in dict {
                new_dict.insert(
                    k.clone(),
                    decrypt_pdf_val(v, obj_id, gen_num, key, algo, meta_mode),
                );
            }
            new_dict.insert("Length".into(), PdfVal::Int(new_data.len() as i64));
            PdfVal::Stream {
                dict: new_dict,
                data: new_data,
            }
        }
        other => other.clone(),
    }
}

fn encrypt_bytes(
    data: &[u8],
    obj_id: u32,
    gen_num: u16,
    key: &[u8],
    algo: EncryptionAlgorithm,
) -> Vec<u8> {
    match algo {
        EncryptionAlgorithm::Rc4_128 => {
            let ok = obj_rc4_key(key, obj_id, gen_num);
            rc4_apply(&ok, data)
        }
        EncryptionAlgorithm::Aes256 => {
            let iv: [u8; 16] = Sha256::digest(
                [&obj_id.to_le_bytes()[..], &gen_num.to_le_bytes(), b"ExIV"].concat(),
            )[..16]
                .try_into()
                .expect("16 bytes");
            let mut k = [0u8; 32];
            k.copy_from_slice(&key[..32]);
            let mut out = iv.to_vec();
            out.extend_from_slice(&aes256_cbc_encrypt_pkcs7(&k, &iv, data));
            out
        }
    }
}

fn decrypt_bytes(
    data: &[u8],
    obj_id: u32,
    gen_num: u16,
    key: &[u8],
    algo: EncryptionAlgorithm,
) -> Vec<u8> {
    match algo {
        EncryptionAlgorithm::Rc4_128 => {
            let ok = obj_rc4_key(key, obj_id, gen_num);
            rc4_apply(&ok, data)
        }
        EncryptionAlgorithm::Aes256 => {
            if data.len() < 16 {
                return data.to_vec();
            }
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&data[..16]);
            let mut k = [0u8; 32];
            k.copy_from_slice(&key[..32]);
            aes256_cbc_decrypt_pkcs7(&k, &iv, &data[16..])
        }
    }
}

fn extract_trailer_id(data: &[u8]) -> Option<[u8; 16]> {
    let pos = crate::page_ops::pdf_doc::find_last_subslice(data, b"/ID")?;
    let slice = &data[pos..];
    let lt = slice.iter().position(|&b| b == b'<')?;
    let gt = slice[lt + 1..].iter().position(|&b| b == b'>')?;
    let hex_bytes = &slice[lt + 1..lt + 1 + gt];
    if hex_bytes.len() < 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in hex_bytes[..32].as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

fn rc4_apply_fixed<const N: usize>(key: &[u8], data: &[u8; N]) -> [u8; N] {
    let v = rc4_apply(key, data);
    let mut out = [0u8; N];
    out.copy_from_slice(&v);
    out
}

fn rc4_apply(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s = [0u8; 256];
    for (i, slot) in s.iter_mut().enumerate() {
        *slot = i as u8;
    }
    let mut j: u8 = 0;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, usize::from(j));
    }
    let mut out = Vec::with_capacity(data.len());
    let (mut i, mut j) = (0u8, 0u8);
    for &b in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(s[usize::from(i)]);
        s.swap(usize::from(i), usize::from(j));
        let k = s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))];
        out.push(b ^ k);
    }
    out
}

/// Minimal RFC 1321 MD5 for PDF RC4-128 key derivation.
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301_u32,
        0xefcd_ab89_u32,
        0x98ba_dcfe_u32,
        0x1032_5476_u32,
    );
    for chunk in msg.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        for (i, w) in chunk.as_chunks::<4>().0.iter().enumerate() {
            m[i] = u32::from_le_bytes(*w);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

// Minimal FIPS 197 AES-256 (ECB/CBC) in pure Rust.
const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

fn aes256_expand_key(key: &[u8; 32]) -> [[u8; 16]; 15] {
    const RCON: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];
    let mut w = [[0u8; 4]; 60];
    for i in 0..8 {
        w[i] = [key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]];
    }
    for i in 8..60 {
        let mut temp = w[i - 1];
        if i % 8 == 0 {
            temp = [
                AES_SBOX[usize::from(temp[1])] ^ RCON[i / 8 - 1],
                AES_SBOX[usize::from(temp[2])],
                AES_SBOX[usize::from(temp[3])],
                AES_SBOX[usize::from(temp[0])],
            ];
        } else if i % 8 == 4 {
            temp = temp.map(|b| AES_SBOX[usize::from(b)]);
        }
        for c in 0..4 {
            w[i][c] = w[i - 8][c] ^ temp[c];
        }
    }
    let mut rkeys = [[0u8; 16]; 15];
    for r in 0..15 {
        for c in 0..4 {
            rkeys[r][4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
        }
    }
    rkeys
}

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut res = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            res ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1B;
        }
        b >>= 1;
    }
    res
}

fn aes256_ecb_encrypt_block(key: &[u8; 32], block: &[u8; 16]) -> [u8; 16] {
    let rk = aes256_expand_key(key);
    let mut s = *block;
    for (sb, rb) in s.iter_mut().zip(&rk[0]) {
        *sb ^= rb;
    }
    for (round, round_key) in rk.iter().enumerate().skip(1) {
        for b in &mut s {
            *b = AES_SBOX[usize::from(*b)];
        }
        s = [
            s[0], s[5], s[10], s[15], s[4], s[9], s[14], s[3], s[8], s[13], s[2], s[7], s[12],
            s[1], s[6], s[11],
        ];
        if round < 14 {
            for col in s.as_chunks_mut::<4>().0 {
                let (a0, a1, a2, a3) = (col[0], col[1], col[2], col[3]);
                col[0] = gf_mul(a0, 2) ^ gf_mul(a1, 3) ^ a2 ^ a3;
                col[1] = a0 ^ gf_mul(a1, 2) ^ gf_mul(a2, 3) ^ a3;
                col[2] = a0 ^ a1 ^ gf_mul(a2, 2) ^ gf_mul(a3, 3);
                col[3] = gf_mul(a0, 3) ^ a1 ^ a2 ^ gf_mul(a3, 2);
            }
        }
        for (sb, rb) in s.iter_mut().zip(round_key) {
            *sb ^= rb;
        }
    }
    s
}

fn aes256_ecb_decrypt_block(key: &[u8; 32], block: &[u8; 16]) -> [u8; 16] {
    let rk = aes256_expand_key(key);
    let mut s = *block;
    for (sb, rb) in s.iter_mut().zip(&rk[14]) {
        *sb ^= rb;
    }
    for round in (0..14).rev() {
        s = [
            s[0], s[13], s[10], s[7], s[4], s[1], s[14], s[11], s[8], s[5], s[2], s[15], s[12],
            s[9], s[6], s[3],
        ];
        for b in &mut s {
            *b = AES_INV_SBOX[usize::from(*b)];
        }
        for (sb, rb) in s.iter_mut().zip(&rk[round]) {
            *sb ^= rb;
        }
        if round > 0 {
            for col in s.as_chunks_mut::<4>().0 {
                let (a0, a1, a2, a3) = (col[0], col[1], col[2], col[3]);
                col[0] = gf_mul(a0, 14) ^ gf_mul(a1, 11) ^ gf_mul(a2, 13) ^ gf_mul(a3, 9);
                col[1] = gf_mul(a0, 9) ^ gf_mul(a1, 14) ^ gf_mul(a2, 11) ^ gf_mul(a3, 13);
                col[2] = gf_mul(a0, 13) ^ gf_mul(a1, 9) ^ gf_mul(a2, 14) ^ gf_mul(a3, 11);
                col[3] = gf_mul(a0, 11) ^ gf_mul(a1, 13) ^ gf_mul(a2, 9) ^ gf_mul(a3, 14);
            }
        }
    }
    s
}

fn aes256_cbc_encrypt_no_pad(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = *iv;
    for chunk in data.as_chunks::<16>().0 {
        let mut blk = [0u8; 16];
        for i in 0..16 {
            blk[i] = chunk[i] ^ prev[i];
        }
        let enc = aes256_ecb_encrypt_block(key, &blk);
        out.extend_from_slice(&enc);
        prev = enc;
    }
    out
}

fn aes256_cbc_decrypt_no_pad(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = *iv;
    for chunk in data.as_chunks::<16>().0 {
        let blk = *chunk;
        let dec = aes256_ecb_decrypt_block(key, &blk);
        for i in 0..16 {
            out.push(dec[i] ^ prev[i]);
        }
        prev = blk;
    }
    out
}

fn aes256_cbc_encrypt_pkcs7(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let pad = 16 - (data.len() % 16);
    let mut buf = Vec::with_capacity(data.len() + pad);
    buf.extend_from_slice(data);
    buf.resize(data.len() + pad, pad as u8);
    aes256_cbc_encrypt_no_pad(key, iv, &buf)
}

fn aes256_cbc_decrypt_pkcs7(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut dec = aes256_cbc_decrypt_no_pad(key, iv, data);
    if let Some(&pad) = dec.last() {
        let p = usize::from(pad);
        if (1..=16).contains(&p) && dec.len() >= p && dec[dec.len() - p..].iter().all(|&b| b == pad)
        {
            dec.truncate(dec.len() - p);
        }
    }
    dec
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::Document;

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn permissions_encode_decode_and_enforce() {
        let ro = PdfPermissions::read_only();
        let decoded = PdfPermissions::from_pdf_p(ro.to_pdf_p());
        assert_eq!(decoded, ro);
        assert!(enforce_permission(decoded, DocumentAction::PrintLowRes).is_ok());
        assert!(enforce_permission(decoded, DocumentAction::CopyContent).is_err());
        assert!(enforce_permission(decoded, DocumentAction::ModifyContents).is_err());
    }

    #[test]
    fn aes256_and_rc4_encrypt_decrypt_round_trip() {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 720 Td (Confidential Payload) Tj ET".into(),
            crop_box: None,
        });
        let plain = b.build().expect("plain");

        for algo in [EncryptionAlgorithm::Aes256, EncryptionAlgorithm::Rc4_128] {
            let policy = SecurityPolicy {
                algorithm: algo,
                user_password: "open-user".into(),
                owner_password: "master-owner".into(),
                permissions: PdfPermissions::read_only(),
                encrypt_metadata: MetadataEncryption::Encrypted,
            };
            let enc = encrypt_pdf(&plain, &policy).expect("encrypt");
            let desc = inspect_encryption(&enc)
                .expect("inspect")
                .expect("encrypted");
            assert_eq!(desc.algorithm, algo);
            assert_eq!(desc.permissions, PdfPermissions::read_only());

            assert!(matches!(
                decrypt_pdf(&enc, "wrong"),
                Err(PdfCryptoError::WrongPassword)
            ));
            let dec = decrypt_pdf(&enc, "master-owner").expect("decrypt owner");
            assert!(inspect_encryption(&dec).expect("inspect2").is_none());

            let doc = Document::from_bytes(&dec, "application/pdf").expect("mupdf opens decrypted");
            let text = doc
                .load_page(0)
                .expect("p0")
                .text(None)
                .expect("s")
                .text()
                .expect("t");
            assert!(text.contains("Confidential Payload"), "got {text}");
        }
    }

    #[test]
    fn sanitize_strips_metadata_and_scripts() {
        let base = crate::notebook::pdf_ops::build_blank_pdf(&[(612.0, 792.0)]).expect("base");
        let parsed = ParsedPdf::parse(&base).expect("parse");
        let mut catalog = parsed.get_dict(parsed.root_id).expect("cat");
        catalog.insert("OpenAction".into(), PdfVal::Str(b"js".to_vec()));
        let mut ov = BTreeMap::new();
        ov.insert(parsed.root_id, PdfVal::Dict(catalog));
        let dirty = rewrite_clean(&parsed, &ov, &BTreeMap::new(), &BTreeSet::new());

        let clean = sanitize_pdf(&dirty, SanitizeOptions::default()).expect("sanitize");
        let p2 = ParsedPdf::parse(&clean).expect("p2");
        let cat2 = p2.get_dict(p2.root_id).expect("cat2");
        assert!(!cat2.contains_key("OpenAction"));
        assert!(!cat2.contains_key("OCProperties"));
    }
}
