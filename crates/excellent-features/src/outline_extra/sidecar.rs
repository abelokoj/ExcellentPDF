//! Custom editable sidecar outline store for non-PDF documents (EPUB, CBZ, DjVu, CHM, etc.)
//! (plan B.13).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::outline::Entry;
use crate::outline_extra::io::{export_json, import_json};

/// Errors produced by sidecar outline persistence.
#[derive(Debug)]
pub enum SidecarError {
    /// Filesystem I/O failure.
    Io(std::io::Error),
    /// Outline JSON serialization/deserialization failure.
    Format(String),
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "sidecar outline I/O error: {err}"),
            Self::Format(msg) => write!(f, "sidecar outline format error: {msg}"),
        }
    }
}

impl std::error::Error for SidecarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Format(_) => None,
        }
    }
}

impl From<std::io::Error> for SidecarError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Convenience result alias for sidecar outline persistence.
pub type Result<T> = std::result::Result<T, SidecarError>;

/// Persistent atomic store for custom outlines attached to read-only non-PDF documents (plan B.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarOutlineStore {
    root_dir: PathBuf,
}

impl SidecarOutlineStore {
    /// Creates a store rooted at `root_dir` (created on first write).
    #[must_use]
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
        }
    }

    /// Computes the deterministic sidecar JSON path for a non-PDF document at `doc_path`.
    #[must_use]
    pub fn sidecar_path_for(&self, doc_path: &Path) -> PathBuf {
        let canonical = doc_path.to_string_lossy().replace('\\', "/");
        let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
        self.root_dir
            .join(format!("{}.outline.json", &digest[..24]))
    }

    /// Loads the custom sidecar outline for `doc_path`, returning `Ok(None)` when none exists.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError`] if the file exists but cannot be read or parsed.
    pub fn load(&self, doc_path: &Path) -> Result<Option<Vec<Entry>>> {
        let path = self.sidecar_path_for(doc_path);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let entries = import_json(&raw).map_err(|e| SidecarError::Format(e.to_string()))?;
        Ok(Some(entries))
    }

    /// Saves `entries` atomically as the custom outline for `doc_path` (surviving app restarts).
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError`] if serialization or atomic file replacement fails.
    pub fn save(&self, doc_path: &Path, entries: &[Entry]) -> Result<()> {
        fs::create_dir_all(&self.root_dir)?;
        let target = self.sidecar_path_for(doc_path);
        let payload = export_json(entries).map_err(|e| SidecarError::Format(e.to_string()))?;
        let tmp = target.with_extension("outline.json.part");
        {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(payload.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Deletes any custom sidecar outline for `doc_path`, reverting to the document's built-in outline.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Io`] if removal fails for reasons other than not found.
    pub fn clear(&self, doc_path: &Path) -> Result<()> {
        let target = self.sidecar_path_for(doc_path);
        if target.exists() {
            fs::remove_file(target)?;
        }
        Ok(())
    }
}

/// Returns the custom sidecar outline when present (even if edited), falling back to `builtin`
/// otherwise (plan B.13 step 1).
#[must_use]
pub fn effective_outline<'a>(sidecar: Option<&'a [Entry]>, builtin: &'a [Entry]) -> &'a [Entry] {
    match sidecar {
        Some(custom) => custom,
        None => builtin,
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::OutlineStyle;

    use super::*;

    #[test]
    fn custom_cbz_outline_persists_across_store_reopen() {
        let dir = std::env::temp_dir().join(format!("ex-sidecar-outline-{}", std::process::id()));
        let doc = Path::new("/comics/issue1.cbz");
        let entries = vec![
            Entry {
                title: "Cover".into(),
                page: Some(0),
                uri: Some("#page=1".into()),
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
            Entry {
                title: "Act 1".into(),
                page: Some(4),
                uri: Some("#page=5".into()),
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
        ];

        {
            let store = SidecarOutlineStore::new(&dir);
            assert!(store.load(doc).expect("empty load").is_none());
            store.save(doc, &entries).expect("save sidecar");
        }

        // Reopen store to simulate restart (plan B.13 Done when).
        let reopened = SidecarOutlineStore::new(&dir);
        let loaded = reopened.load(doc).expect("reload").expect("present");
        assert_eq!(loaded, entries);
        assert_eq!(effective_outline(Some(&loaded), &[]), loaded.as_slice());

        reopened.clear(doc).expect("clear");
        assert!(reopened.load(doc).expect("load after clear").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
