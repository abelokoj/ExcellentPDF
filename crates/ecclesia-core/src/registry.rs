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
    std::env::temp_dir().join("ecclesiapdf-oplog")
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

    /// Reloads any live open [`Document`] for `path` in place from disk and updates its registry
    /// entry so subsequent [`Registry::open`] calls continue sharing the same `Arc<Document>`.
    ///
    /// Returns `Ok(true)` if an open document for `path` was reloaded, or `Ok(false)` if no live
    /// document is currently open for `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file on disk cannot be read or MuPDF cannot open the updated file.
    pub fn reload(&self, path: &Path) -> document::Result<bool> {
        let target_canonical = std::fs::canonicalize(path).ok();
        let matching = {
            let open = self.open.lock().expect("registry lock poisoned");
            open.values().filter_map(Weak::upgrade).find(|doc| {
                doc.path() == path
                    || target_canonical
                        .as_deref()
                        .is_some_and(|canonical| doc.id().path() == canonical)
            })
        };
        let Some(doc) = matching else {
            return Ok(false);
        };
        // Reload outside the registry mutex so disk I/O and MuPDF parsing never block other tabs.
        doc.reload_from_disk()?;
        let new_id = doc.id();
        let mut open = self.open.lock().expect("registry lock poisoned");
        open.retain(|_, held| {
            held.upgrade()
                .is_some_and(|existing| !Arc::ptr_eq(&existing, &doc))
        });
        open.insert(new_id, Arc::downgrade(&doc));
        Ok(true)
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

    #[test]
    fn reloading_an_externally_replaced_pdf_updates_pages_and_keeps_sharing() {
        let dir = std::env::temp_dir().join(format!(
            "ecclesia-core-registry-reload-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch folder");
        let path = dir.join("live.pdf");
        // Start with a 1-page PDF.
        std::fs::copy(corpus("heavy-vector.pdf"), &path).expect("copy 1-page pdf");

        let registry = Registry::with_log_dir(dir.join("logs"));
        let doc = registry.open(&path).expect("open initial");
        let initial_pages = doc
            .mupdf()
            .expect("awake")
            .lock()
            .expect("lock")
            .page_count()
            .expect("count");
        assert_eq!(initial_pages, 1);
        let initial_rev = doc.revision(0);
        let initial_edits = doc.edits();

        // Overwrite the file on disk with a 2-page PDF.
        std::fs::copy(corpus("base14-outline-annot.pdf"), &path)
            .expect("overwrite with 2-page pdf");
        let reloaded = registry.reload(&path).expect("reload");
        assert!(reloaded, "open document should be reloaded");

        let updated_pages = doc
            .mupdf()
            .expect("awake")
            .lock()
            .expect("lock")
            .page_count()
            .expect("count");
        assert_eq!(updated_pages, 2);
        assert!(doc.revision(0) > initial_rev, "page revisions must advance");
        assert_eq!(doc.edits(), initial_edits + 1, "edits counter must advance");

        // A subsequent open returns the exact same shared Arc<Document>.
        let reopened = registry.open(&path).expect("reopen");
        assert!(Arc::ptr_eq(&doc, &reopened));
        assert_eq!(registry.len(), 1);

        // Reloading a path that is not currently open returns Ok(false).
        assert!(
            !registry
                .reload(&corpus("zlib-manual.pdf"))
                .expect("not open")
        );

        drop((doc, reopened));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
