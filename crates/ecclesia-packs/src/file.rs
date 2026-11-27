//! A pack file (`.expack`): a 64-byte Ed25519 signature, then a zip archive it signs. The archive holds
//! `manifest.json` (`{id, version, title}`) and the pack's files. Signed whole, a pack checks itself, so one copied in
//! from a disk (offline) is as trusted as one downloaded.

use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use serde_json::Value;

use crate::error::Error;

/// The signature's length at the start of a pack file.
pub const SIGNATURE_LEN: usize = 64;

/// The manifest's name inside the archive.
pub const MANIFEST: &str = "manifest.json";

/// What a pack says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Its id, which names its folder: lower-case letters, digits, `-` and `.`.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Its name for people.
    pub title: String,
}

impl Manifest {
    /// The manifest `bytes` hold.
    ///
    /// # Errors
    ///
    /// It isn't JSON, lacks an id or version, or its id isn't a safe folder name.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|err| Error::Format(err.to_string()))?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let id = text("id").ok_or_else(|| Error::Format("the manifest has no id".into()))?;
        if !is_id(&id) {
            return Err(Error::Format(format!("{id:?} is not a pack id")));
        }
        Ok(Self {
            version: text("version")
                .ok_or_else(|| Error::Format("the manifest has no version".into()))?,
            title: text("title").unwrap_or_else(|| id.clone()),
            id,
        })
    }
}

/// Whether `id` is a pack id: 1 to 64 lower-case letters, digits, `-` and `.`, not starting with a dot.
#[must_use]
pub fn is_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// The archive inside the pack file `bytes`, once its signature verifies with `key`.
///
/// # Errors
///
/// [`Error::Signature`] when it is too short to hold one, or the signature doesn't verify: a byte changed anywhere.
pub fn verified<'a>(bytes: &'a [u8], key: &VerifyingKey) -> Result<&'a [u8], Error> {
    if bytes.len() < SIGNATURE_LEN {
        return Err(Error::Signature);
    }
    let (signature, archive) = bytes.split_at(SIGNATURE_LEN);
    let signature = Signature::from_slice(signature).map_err(|_| Error::Signature)?;
    key.verify_strict(archive, &signature)
        .map_err(|_| Error::Signature)?;
    Ok(archive)
}

/// A pack file of `archive`, signed with `key`: what the release tools write.
#[must_use]
pub fn sign(archive: &[u8], key: &SigningKey) -> Vec<u8> {
    use ed25519_dalek::Signer;
    let mut out = key.sign(archive).to_bytes().to_vec();
    out.extend_from_slice(archive);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tampered_pack_is_rejected() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let file = sign(b"the archive", &key);
        assert_eq!(
            verified(&file, &key.verifying_key()).expect("verifies"),
            b"the archive"
        );
        for at in [0, SIGNATURE_LEN, file.len() - 1] {
            let mut changed = file.clone();
            changed[at] ^= 1;
            assert!(
                matches!(
                    verified(&changed, &key.verifying_key()),
                    Err(Error::Signature)
                ),
                "byte {at}"
            );
        }
        let other = SigningKey::from_bytes(&[8; 32]);
        assert!(
            matches!(
                verified(&file, &other.verifying_key()),
                Err(Error::Signature)
            ),
            "another key"
        );
        assert!(matches!(
            verified(&file[..10], &key.verifying_key()),
            Err(Error::Signature)
        ));
    }

    #[test]
    fn ids_are_safe_folder_names() {
        assert!(is_id("dict-en-gb") && is_id("fonts.noto-cjk") && is_id("ocr2"));
        for bad in [
            "",
            "..",
            ".hidden",
            "a/b",
            "a\\b",
            "Caps",
            "space here",
            &"x".repeat(65),
        ] {
            assert!(!is_id(bad), "{bad:?}");
        }
        assert!(Manifest::parse(br#"{"id": "../x", "version": "1"}"#).is_err());
        let manifest =
            Manifest::parse(br#"{"id": "dict-en", "version": "2026.1"}"#).expect("parses");
        assert_eq!(manifest.title, "dict-en", "the id when it has no title");
    }
}
