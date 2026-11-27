//! The signed index of packs on offer: JSON (`{"packs": [{id, version, title, size, sha256, url, licence}]}`) and its
//! Ed25519 signature, fetched together. Nothing in it is used until the signature verifies.

use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::Value;

use crate::error::Error;
use crate::file::is_id;

/// One pack on offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Its name for people.
    pub title: String,
    /// Its file's size in bytes.
    pub size: u64,
    /// Its file's SHA-256, lower-case hex.
    pub sha256: String,
    /// Where to download it: HTTPS only.
    pub url: String,
    /// Its licence, as an SPDX expression.
    pub licence: String,
}

/// The index `json`, once `signature` (64 bytes) over it verifies with `key`.
///
/// # Errors
///
/// [`Error::Signature`] when it doesn't verify; [`Error::Format`] when it isn't an index (an entry with a bad id, a
/// size, hash or URL missing, or a URL that isn't HTTPS).
pub fn verified(json: &[u8], signature: &[u8], key: &VerifyingKey) -> Result<Vec<Entry>, Error> {
    let signature = Signature::from_slice(signature).map_err(|_| Error::Signature)?;
    key.verify_strict(json, &signature)
        .map_err(|_| Error::Signature)?;
    let value: Value =
        serde_json::from_slice(json).map_err(|err| Error::Format(err.to_string()))?;
    value
        .get("packs")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Format("the index lists no packs".into()))?
        .iter()
        .map(entry)
        .collect()
}

fn entry(value: &Value) -> Result<Entry, Error> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Format(format!("an entry has no {key}")))
    };
    let id = text("id")?;
    if !is_id(&id) {
        return Err(Error::Format(format!("{id:?} is not a pack id")));
    }
    let url = text("url")?;
    if !url.starts_with("https://") {
        return Err(Error::Format(format!("{id}'s address isn't HTTPS")));
    }
    let sha256 = text("sha256")?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Format(format!("{id}'s hash isn't SHA-256")));
    }
    Ok(Entry {
        version: text("version")?,
        title: text("title").unwrap_or_else(|_| id.clone()),
        size: value
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Format(format!("{id} has no size")))?,
        licence: text("licence").unwrap_or_default(),
        id,
        sha256,
        url,
    })
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    const INDEX: &str = r#"{"packs": [{"id": "dict-en", "version": "1", "title": "English dictionary",
        "size": 1200, "sha256": "ab", "url": "https://packs.example/dict-en-1.expack", "licence": "CC-BY-4.0"}]}"#;

    #[test]
    fn an_index_is_read_only_when_its_signature_verifies() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let good = INDEX.replace("\"ab\"", &format!("\"{}\"", "ab".repeat(32)));
        let signature = key.sign(good.as_bytes()).to_bytes();
        let entries =
            verified(good.as_bytes(), &signature, &key.verifying_key()).expect("verifies");
        assert_eq!(entries[0].id, "dict-en");
        assert_eq!(entries[0].size, 1200);
        let tampered = good.replace("1200", "1201");
        assert!(matches!(
            verified(tampered.as_bytes(), &signature, &key.verifying_key()),
            Err(Error::Signature)
        ));
    }

    #[test]
    fn plain_http_and_bad_hashes_are_refused() {
        let key = SigningKey::from_bytes(&[3; 32]);
        for bad in [INDEX.replace("https://", "http://"), INDEX.to_owned()] {
            let signature = key.sign(bad.as_bytes()).to_bytes();
            assert!(matches!(
                verified(bad.as_bytes(), &signature, &key.verifying_key()),
                Err(Error::Format(_))
            ));
        }
    }
}
