//! Document library management: collections, tags, read status, metadata extraction,
//! duplicate detection, and folder watching (plan F.6–F.9).

pub mod metadata;
pub mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ecclesia_store::library::{
    self, Collection, LibraryFilter, LibraryItem, ReadStatus, WatchedFolder,
};
use ecclesia_store::{Result as StoreResult, Store};

pub use ecclesia_store::library::WatchDepth;
pub use metadata::{
    DuplicateKind, DuplicatePair, ExtractedMetadata, MetadataError, OnlineConsent, compute_minhash,
    detect_duplicates, extract_arxiv_id, extract_doi, extract_from_pdf, extract_isbn,
    minhash_similarity, sha256_bytes, sha256_file,
};
pub use watch::{FolderDiff, FolderWatcher, ReconcileReport, WatchEvent};

/// Nested node in a hierarchical library collection tree (plan F.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionNode {
    /// Collection record.
    pub collection: Collection,
    /// Child collections in display order.
    pub children: Vec<Self>,
}

/// High-level library interface over [`Store`] (plan F.6–F.9).
#[derive(Debug)]
pub struct Library<'a> {
    store: &'a Store,
}

impl<'a> Library<'a> {
    /// Wraps `store` as a [`Library`] view.
    #[must_use]
    pub const fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Adds or updates a document in the library, extracting metadata and duplicate hashes from disk
    /// when `extract_metadata` is [`MetadataSource::FromDisk`].
    ///
    /// # Errors
    ///
    /// Returns [`ecclesia_store::Error`] on SQLite or I/O failure.
    pub fn add_document(
        &self,
        path: &Path,
        source: MetadataSource,
        now_secs: i64,
    ) -> StoreResult<LibraryItem> {
        let mut item = match source {
            MetadataSource::FromDisk => {
                metadata::build_library_item(path, now_secs).map_err(ecclesia_store::Error::Io)?
            }
            MetadataSource::Provided(provided) => *provided,
        };
        if item.added_at == 0 {
            item.added_at = now_secs;
        }
        let cloned = item.clone();
        self.store
            .write(move |tx| library::upsert_item(tx, &cloned))
            .wait()?;
        Ok(item)
    }

    /// Lists library items matching `filter`.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn list(&self, filter: &LibraryFilter) -> StoreResult<Vec<LibraryItem>> {
        self.store.read(|conn| library::list_items(conn, filter))
    }

    /// Updates read status and normalized progress (`0.0..=1.0`).
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn set_read_status(
        &self,
        path: &Path,
        status: ReadStatus,
        progress: f64,
    ) -> StoreResult<bool> {
        let path = path.to_path_buf();
        self.store
            .write(move |tx| library::set_read_status(tx, &path, status, progress))
            .wait()
    }

    /// Sets the library tags on `path`.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn set_tags(&self, path: &Path, tags: Vec<String>) -> StoreResult<()> {
        let path = path.to_path_buf();
        self.store
            .write(move |tx| library::set_file_tags(tx, &path, &tags))
            .wait()
    }

    /// Creates a nested collection.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn create_collection(
        &self,
        parent: Option<i64>,
        name: &str,
        icon: Option<&str>,
        color: Option<&str>,
    ) -> StoreResult<i64> {
        let name = name.to_owned();
        let icon = icon.map(str::to_owned);
        let color = color.map(str::to_owned);
        self.store
            .write(move |tx| {
                library::create_collection(tx, parent, &name, icon.as_deref(), color.as_deref())
            })
            .wait()
    }

    /// Adds `path` to `collection_id` preserving insertion order.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn add_to_collection(&self, collection_id: i64, path: &Path) -> StoreResult<()> {
        let path = path.to_path_buf();
        self.store
            .write(move |tx| library::add_to_collection(tx, collection_id, &path))
            .wait()
    }

    /// Builds the full nested collection tree.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn collection_tree(&self) -> StoreResult<Vec<CollectionNode>> {
        let all = self.store.read(library::list_collections)?;
        Ok(build_collection_tree(&all))
    }

    /// Finds exact and near-duplicate pairs across the entire library (plan F.8).
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn find_duplicates(&self, min_similarity: f64) -> StoreResult<Vec<DuplicatePair>> {
        let items = self
            .store
            .read(|conn| library::list_items(conn, &LibraryFilter::default()))?;
        Ok(metadata::detect_duplicates(&items, min_similarity))
    }

    /// Registers a folder to watch for new, modified, and deleted documents (plan F.9).
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn watch_folder(&self, path: &Path, depth: WatchDepth) -> StoreResult<i64> {
        let path = path.to_path_buf();
        self.store
            .write(move |tx| library::add_watched_folder(tx, &path, depth))
            .wait()
    }

    /// Lists all watched folders.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn watched_folders(&self) -> StoreResult<Vec<WatchedFolder>> {
        self.store.read(library::list_watched_folders)
    }

    /// Scans all watched folders and reconciles added/modified/removed files (plan F.9).
    ///
    /// # Errors
    ///
    /// SQLite or filesystem failed.
    pub fn reconcile_watched(&self) -> StoreResult<ReconcileReport> {
        watch::reconcile_all(self.store)
    }

    /// Returns ordered file paths inside `collection_id`.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn collection_paths(&self, collection_id: i64) -> StoreResult<Vec<PathBuf>> {
        self.store
            .read(|conn| library::collection_items(conn, collection_id))
    }
}

/// How [`Library::add_document`] obtains metadata for a document.
#[derive(Debug, Clone)]
pub enum MetadataSource {
    /// Extract metadata, SHA-256, and MinHash directly from the file on disk.
    FromDisk,
    /// Use caller-supplied [`LibraryItem`] metadata.
    Provided(Box<LibraryItem>),
}

/// Builds a tree of [`CollectionNode`]s from a flat list of [`Collection`]s.
#[must_use]
pub fn build_collection_tree(collections: &[Collection]) -> Vec<CollectionNode> {
    let mut by_parent: BTreeMap<Option<i64>, Vec<Collection>> = BTreeMap::new();
    for col in collections {
        by_parent.entry(col.parent).or_default().push(col.clone());
    }
    build_subtree(&by_parent, None)
}

fn build_subtree(
    by_parent: &BTreeMap<Option<i64>, Vec<Collection>>,
    parent: Option<i64>,
) -> Vec<CollectionNode> {
    let Some(children) = by_parent.get(&parent) else {
        return Vec::new();
    };
    children
        .iter()
        .map(|col| CollectionNode {
            collection: col.clone(),
            children: build_subtree(by_parent, Some(col.id)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ecclesia-lib-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn library_collections_tags_and_tree_work_end_to_end() {
        let dir = temp_dir("tree");
        let store = Store::open(&dir.join("library.db")).expect("open");
        let lib = Library::new(&store);

        let root = lib
            .create_collection(None, "Research", Some("📚"), Some("blue"))
            .expect("root");
        let child = lib
            .create_collection(Some(root), "ML", None, None)
            .expect("child");

        let corpus =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let item = lib
            .add_document(&corpus, MetadataSource::FromDisk, 100)
            .expect("add");
        assert!(item.page_count > 0);
        assert!(!item.content_hash.is_empty());

        lib.add_to_collection(child, &corpus).expect("add to col");
        lib.set_read_status(&corpus, ReadStatus::Reading, 0.5)
            .expect("status");
        lib.set_tags(&corpus, vec!["compression".to_owned()])
            .expect("tags");

        let tree = lib.collection_tree().expect("tree");
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].collection.item_count, 1);

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
