//! The open documents (plan K.5, item 3): a file opened twice is parsed once, and its [`Document`] lives while any
//! view holds it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::document::{self, Document, FileId};

/// The open documents, by file identity.
#[derive(Debug)]
pub struct Registry {
    open: Mutex<HashMap<FileId, Weak<Document>>>,
    /// Where the documents' operation logs are kept (K.14).
    log_dir: PathBuf,
}

/// Where operation logs are kept when the app names no folder: the system's temporary folder (the tests, the
/// command line).
pub(crate) fn default_log_dir() -> PathBuf {
    std::env::temp_dir().join("excellentpdf-oplog")
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_log_dir(default_log_dir())
    }
}

impl Registry {
    /// An empty registry, keeping operation logs in the system's temporary folder.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty registry, keeping operation logs in `log_dir` (made when the first edit is logged).
    pub fn with_log_dir(log_dir: PathBuf) -> Self {
        Self {
            open: Mutex::new(HashMap::new()),
            log_dir,
        }
    }

    /// The document for the file at `path`: the one already open, or the file parsed now.
    ///
    /// # Errors
    ///
    /// The file can't be read, or MuPDF can't open it.
    pub fn open(&self, path: &Path) -> document::Result<Arc<Document>> {
        let id = FileId::of(path)?;
        {
            let mut open = self.open.lock().expect("registry lock poisoned");
            if let Some(document) = open.get(&id).and_then(Weak::upgrade) {
                return Ok(document);
            }
            // A document saved since it opened is a new version of its file, which its key doesn't show.
            let saved = open
                .values()
                .filter_map(Weak::upgrade)
                .find(|document| document.id() == id);
            if let Some(document) = saved {
                open.insert(id, Arc::downgrade(&document));
                return Ok(document);
            }
        }
        // Parse outside the registry mutex so opening one large file never blocks another tab or
        // thread checking already-open documents.
        let parsed = Arc::new(Document::open_as(id.clone(), path, &self.log_dir)?);
        let mut open = self.open.lock().expect("registry lock poisoned");
        if let Some(existing) = open.get(&id).and_then(Weak::upgrade) {
            return Ok(existing);
        }
        open.retain(|_, held| held.strong_count() > 0);
        open.insert(id, Arc::downgrade(&parsed));
        Ok(parsed)
    }

    /// How many documents are open.
    pub fn len(&self) -> usize {
        let open = self.open.lock().expect("registry lock poisoned");
        open.values().filter(|held| held.strong_count() > 0).count()
    }

    /// Whether no document is open.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name)
    }

    #[test]
    fn a_file_opened_twice_is_parsed_once_and_shared() {
        let registry = Registry::new();
        let path = corpus("zlib-manual.pdf");
        let first = registry.open(&path).expect("open");
        // Another spelling of the same path.
        let again = path.parent().expect("folder").join("./zlib-manual.pdf");
        let second = registry.open(&again).expect("open");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.len(), 1);

        // Two views share one parse and its caches.
        let a = first.mupdf().expect("awake");
        let b = second.mupdf().expect("awake");
        assert!(Arc::ptr_eq(&a, &b));

        drop((first, second, a, b));
        assert!(registry.is_empty(), "closed when the last view lets go");
    }
}
