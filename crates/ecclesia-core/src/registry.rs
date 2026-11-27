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

    /// Generates a minimal valid multi-page PDF with `pages` letter-sized pages and valid xref.
    fn minimal_multipage_pdf(pages: usize) -> Vec<u8> {
        use std::fmt::Write as _;
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets: Vec<usize> = Vec::with_capacity(pages + 3);
        offsets.push(0); // object 0 (free)
        // Object 1: Catalog
        offsets.push(out.len());
        out.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        // Object 2: Pages root
        offsets.push(out.len());
        out.push_str("2 0 obj\n<< /Type /Pages /Kids [");
        for i in 0..pages {
            let _ = write!(out, "{} 0 R ", i + 3);
        }
        let _ = writeln!(out, "] /Count {pages} >>\nendobj");
        // Objects 3..=pages+2: individual pages
        for i in 0..pages {
            offsets.push(out.len());
            let _ = writeln!(
                out,
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj",
                i + 3
            );
        }
        let xref_pos = out.len();
        let _ = writeln!(out, "xref\n0 {}", pages + 3);
        out.push_str("0000000000 65535 f \n");
        for &offset in offsets.iter().skip(1) {
            let _ = writeln!(out, "{offset:010} 00000 n ");
        }
        let _ = write!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            pages + 3
        );
        out.into_bytes()
    }

    #[test]
    fn stress_50_pdfs_with_2000_pages_each_hibernated_and_concurrent() {
        use std::time::Instant;

        const FILE_COUNT: usize = 50;
        const PAGE_COUNT: usize = 2000;

        let dir = std::env::temp_dir().join(format!(
            "ecclesia-core-stress-50x2000-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("stress folder");

        let template = minimal_multipage_pdf(PAGE_COUNT);
        let paths: Vec<PathBuf> = (0..FILE_COUNT)
            .map(|i| {
                let p = dir.join(format!("book-{i:02}.pdf"));
                std::fs::write(&p, &template).expect("write stress pdf");
                p
            })
            .collect();

        let registry = Registry::with_log_dir(dir.join("logs"));

        // 1. Real EcclesiaPDF tab-hibernation model: only the active tab holds an Arc<Document>.
        let hibernated_start = Instant::now();
        for path in &paths {
            let doc = registry.open(path).expect("open hibernated stress pdf");
            assert_eq!(registry.len(), 1, "only the active tab stays parsed");
            let mupdf = doc.mupdf().expect("awake");
            let guard = mupdf.lock().expect("lock");
            assert_eq!(guard.page_count().expect("page_count"), PAGE_COUNT);
            let first = guard
                .page_box(0, ecclesia_mupdf::PageBox::Media)
                .expect("first page");
            let last = guard
                .page_box(PAGE_COUNT - 1, ecclesia_mupdf::PageBox::Media)
                .expect("last page");
            assert!((first.x1 - 612.0).abs() < 1.0 && (last.y1 - 792.0).abs() < 1.0);
            drop(guard);
            drop(mupdf);
            drop(doc);
            assert!(registry.is_empty(), "inactive tab releases MuPDF handle");
        }
        let hibernated_ms = hibernated_start.elapsed().as_secs_f64() * 1000.0;

        // 2. Worst-case concurrent load: all 50 x 2,000-page PDFs (100,000 pages) open at once.
        let concurrent_start = Instant::now();
        let mut open_docs = Vec::with_capacity(FILE_COUNT);
        for path in &paths {
            let doc = registry.open(path).expect("open concurrent stress pdf");
            let mupdf = doc.mupdf().expect("awake");
            let count = mupdf.lock().expect("lock").page_count().expect("count");
            assert_eq!(count, PAGE_COUNT);
            open_docs.push(doc);
        }
        let concurrent_ms = concurrent_start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(registry.len(), FILE_COUNT);

        // Measure full page-box scan of one 2,000-page document (what PageCanvas::open runs).
        let boxes_start = Instant::now();
        {
            let mupdf = open_docs[0].mupdf().expect("awake");
            let guard = mupdf.lock().expect("lock");
            for page in 0..PAGE_COUNT {
                let _ = guard
                    .page_box(page, ecclesia_mupdf::PageBox::Media)
                    .expect("page_box");
            }
        }
        let boxes_ms = boxes_start.elapsed().as_secs_f64() * 1000.0;

        drop(open_docs);
        assert!(
            registry.is_empty(),
            "all 50 documents cleanly freed on drop"
        );
        let _ = std::fs::remove_dir_all(&dir);

        eprintln!(
            "STRESS 50x2000 (100,000 pages): hibernated_cycle={hibernated_ms:.1}ms, all_50_open={concurrent_ms:.1}ms, 2000_page_bounds_scan={boxes_ms:.1}ms"
        );
    }
}
