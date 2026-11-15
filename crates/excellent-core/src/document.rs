//! One open file (plan K.5): the MuPDF document, its display lists, each page's revision, and whether it has unsaved
//! changes. Every view of a file shares one [`Document`] (through [`crate::registry::Registry`]), so the file is parsed
//! once and its caches serve every view.
//!
//! shortcut: the annotation model (K.13) and the operation log (K.14) join `Document` when they are built.

mod identity;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use excellent_mupdf::{DisplayList, MuError, Reflow};

pub use identity::FileId;

/// A MuPDF document as the render pool takes it: one thread uses it at a time.
pub type SharedMupdf = Arc<Mutex<excellent_mupdf::Document>>;

/// Why a document couldn't be opened, woken or hibernated.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file couldn't be read.
    Io(std::io::Error),
    /// MuPDF couldn't open or read it.
    Mupdf(MuError),
    /// The document has unsaved changes, so it stays open.
    UnsavedChanges,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot read the file: {err}"),
            Self::Mupdf(err) => write!(f, "{err}"),
            Self::UnsavedChanges => f.write_str("the document has unsaved changes"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Mupdf(err) => Some(err),
            Self::UnsavedChanges => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<MuError> for Error {
    fn from(err: MuError) -> Self {
        Self::Mupdf(err)
    }
}

/// The result of a document operation.
pub type Result<T> = std::result::Result<T, Error>;

/// One open file, shared by its views.
pub struct Document {
    id: FileId,
    /// The path it was opened by, to open it again after hibernating.
    path: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// `None` while hibernating.
    mupdf: Option<SharedMupdf>,
    /// Finished display lists by page, each recorded at the page's current revision.
    lists: HashMap<usize, Arc<DisplayList>>,
    /// Each changed page's revision; a page never changed is at 0.
    revisions: HashMap<usize, u64>,
    /// Goes up with every layout of a reflowable document, which changes every page: it adds to each page's
    /// revision.
    layouts: u64,
    /// The pages a reflowable document is laid out on, to lay it out the same way after hibernating.
    reflow: Option<Reflow>,
    unsaved: bool,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("Document")
            .field("id", &self.id)
            .field("hibernating", &state.mupdf.is_none())
            .field("lists", &state.lists.len())
            .field("unsaved", &state.unsaved)
            .finish()
    }
}

impl Document {
    /// Opens the file at `path`.
    ///
    /// # Errors
    ///
    /// The file can't be read, or MuPDF can't open it.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_as(FileId::of(path)?, path)
    }

    /// Opens the file at `path`, already known as `id`.
    pub(crate) fn open_as(id: FileId, path: &Path) -> Result<Self> {
        let mupdf = excellent_mupdf::Document::open(path)?;
        Ok(Self {
            id,
            path: path.to_owned(),
            state: Mutex::new(State {
                mupdf: Some(Arc::new(Mutex::new(mupdf))),
                ..State::default()
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("document lock poisoned")
    }

    /// Which file it is.
    pub fn id(&self) -> &FileId {
        &self.id
    }

    /// The path it was opened by.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The MuPDF document, opened again first if the document was hibernating.
    ///
    /// # Errors
    ///
    /// Waking failed: the file is gone or no longer opens.
    pub fn mupdf(&self) -> Result<SharedMupdf> {
        let mut state = self.lock();
        if let Some(mupdf) = &state.mupdf {
            return Ok(Arc::clone(mupdf));
        }
        // shortcut: a file changed on disk while hibernating is simply opened again; K.18 notices changes.
        let mut document = excellent_mupdf::Document::open(&self.path)?;
        if let Some(reflow) = state.reflow {
            document.reflow(reflow, None)?;
        }
        let mupdf = Arc::new(Mutex::new(document));
        state.mupdf = Some(Arc::clone(&mupdf));
        Ok(mupdf)
    }

    /// Whether it is hibernating: its MuPDF document is closed until a view needs it.
    pub fn is_hibernating(&self) -> bool {
        self.lock().mupdf.is_none()
    }

    /// Frees its memory: drops the display lists, then closes the MuPDF document (once render jobs still using it
    /// finish). Views drop their own tiles.
    ///
    /// # Errors
    ///
    /// [`Error::UnsavedChanges`]: a document with unsaved changes never hibernates.
    pub fn hibernate(&self) -> Result<()> {
        let mut state = self.lock();
        if state.unsaved {
            return Err(Error::UnsavedChanges);
        }
        state.lists.clear();
        state.mupdf = None;
        Ok(())
    }

    /// Page `page`'s display list, if a view recorded one at its current revision.
    pub fn display_list(&self, page: usize) -> Option<Arc<DisplayList>> {
        self.lock().lists.get(&page).cloned()
    }

    /// Keeps `list`, recorded from page `page` at `revision`, for every view; a list from an older revision is
    /// dropped.
    pub fn keep_display_list(&self, page: usize, revision: u64, list: Arc<DisplayList>) {
        let mut state = self.lock();
        let current = state.layouts + state.revisions.get(&page).copied().unwrap_or(0);
        if revision == current && state.mupdf.is_some() {
            state.lists.insert(page, list);
        }
    }

    /// Page `page`'s revision: it goes up with every change to the page, and with every layout of a reflowable
    /// document.
    pub fn revision(&self, page: usize) -> u64 {
        let state = self.lock();
        state.layouts + state.revisions.get(&page).copied().unwrap_or(0)
    }

    /// Lays a reflowable document out on `reflow`'s pages (R.28) and returns where the text page `keep` started
    /// with is now (see [`excellent_mupdf::Document::reflow`]). Every page changes: the display lists go and every
    /// revision goes up, so views draw the pages again. A document with fixed pages is left as it is, and `keep`
    /// comes back unchanged.
    ///
    /// # Errors
    ///
    /// Waking failed, or MuPDF can't lay the document out.
    pub fn reflow(&self, reflow: Reflow, keep: Option<usize>) -> Result<Option<usize>> {
        if self.lock().reflow == Some(reflow) {
            return Ok(keep);
        }
        let mupdf = self.mupdf()?;
        let mut mupdf = mupdf.lock().expect("document lock poisoned");
        if !mupdf.is_reflowable()? {
            return Ok(keep);
        }
        let kept = mupdf.reflow(reflow, keep)?;
        let mut state = self.lock();
        state.reflow = Some(reflow);
        state.layouts += 1;
        state.lists.clear();
        Ok(kept)
    }

    /// Page `page` changed: its revision goes up, its display list goes, and the document has unsaved changes.
    pub fn page_changed(&self, page: usize) {
        let mut state = self.lock();
        *state.revisions.entry(page).or_insert(0) += 1;
        state.lists.remove(&page);
        state.unsaved = true;
    }

    /// Whether it has changes not yet saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.lock().unsaved
    }

    /// Its changes were saved.
    pub fn saved(&self) {
        self.lock().unsaved = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name)
    }

    fn page_count(document: &Document) -> usize {
        let mupdf = document.mupdf().expect("awake");
        let mupdf = mupdf.lock().expect("lock");
        mupdf.page_count().expect("count")
    }

    #[test]
    fn hibernates_only_without_unsaved_changes() {
        let document = Document::open(&corpus("zlib-manual.pdf")).expect("open");
        let pages = page_count(&document);

        document.page_changed(0);
        assert!(matches!(document.hibernate(), Err(Error::UnsavedChanges)));
        assert!(!document.is_hibernating(), "unsaved changes keep it open");

        document.saved();
        document.hibernate().expect("hibernate");
        assert!(document.is_hibernating());
        assert_eq!(page_count(&document), pages, "a view wakes it");
        assert!(!document.is_hibernating());
    }

    #[test]
    fn a_changed_page_drops_its_list_and_refuses_stale_ones() {
        let document = Document::open(&corpus("zlib-manual.pdf")).expect("open");
        let mupdf = document.mupdf().expect("awake");
        let list = Arc::new(
            mupdf
                .lock()
                .expect("lock")
                .load_page(0)
                .expect("page")
                .display_list(None)
                .expect("list"),
        );
        document.keep_display_list(0, 0, Arc::clone(&list));
        assert!(document.display_list(0).is_some());

        document.page_changed(0);
        assert!(document.display_list(0).is_none());
        assert_eq!(document.revision(0), 1);
        document.keep_display_list(0, 0, Arc::clone(&list));
        assert!(
            document.display_list(0).is_none(),
            "recorded before the change"
        );
        document.keep_display_list(0, 1, list);
        assert!(document.display_list(0).is_some());
    }

    #[test]
    fn a_reflow_changes_every_page_and_survives_hibernating() {
        let document = Document::open(&corpus("formats.epub")).expect("open");
        let small = Reflow {
            width: 420.0,
            height: 595.0,
            font_size: 9.0,
        };
        document.reflow(small, None).expect("reflow");
        let pages = page_count(&document);
        let list = Arc::new(
            document
                .mupdf()
                .expect("awake")
                .lock()
                .expect("lock")
                .load_page(1)
                .expect("page")
                .display_list(None)
                .expect("list"),
        );
        let revision = document.revision(1);
        document.keep_display_list(1, revision, Arc::clone(&list));
        assert!(document.display_list(1).is_some());

        let large = Reflow {
            font_size: 18.0,
            ..small
        };
        let kept = document.reflow(large, Some(1)).expect("reflow");
        assert!(kept.is_some_and(|page| page >= 1));
        assert!(document.display_list(1).is_none(), "the old list is gone");
        assert!(document.revision(1) > revision);
        document.keep_display_list(1, revision, list);
        assert!(
            document.display_list(1).is_none(),
            "laid out before the reflow"
        );
        assert!(!document.has_unsaved_changes(), "a layout is no edit");

        let reflowed = page_count(&document);
        assert!(reflowed > pages);
        document.hibernate().expect("hibernate");
        assert_eq!(
            page_count(&document),
            reflowed,
            "laid out the same way on waking"
        );

        // Fixed pages: nothing changes.
        let pdf = Document::open(&corpus("zlib-manual.pdf")).expect("open");
        assert_eq!(pdf.reflow(small, Some(1)).expect("reflow"), Some(1));
        assert_eq!(pdf.revision(1), 0);
    }
}
