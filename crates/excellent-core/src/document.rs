//! One open file (plan K.5): the MuPDF document, its display lists, each page's revision, and its edits. Every view
//! of a file shares one [`Document`] (through [`crate::registry::Registry`]), so the file is parsed once and its
//! caches serve every view.
//!
//! A PDF can be edited (K.14, K.15): each [`Operation`] is applied inside one MuPDF operation and written to the
//! operation log before it counts, undo and redo walk MuPDF's journal and are logged too, and [`Document::save`]
//! writes the file atomically. Edits a run didn't save are offered again when the file next opens
//! ([`Document::recover`]).
//!
//! shortcut: the annotation model (K.14 step 1) joins `Document` when annotations are edited (Phase 3).

mod identity;

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use excellent_mupdf::{DisplayList, MuError, Reflow, Save, UndoState};

pub use identity::FileId;

use crate::edit::Operation;
use crate::oplog::{self, Log, Record};

/// A MuPDF document as the render pool takes it: one thread uses it at a time.
pub type SharedMupdf = Arc<Mutex<excellent_mupdf::Document>>;

/// Why a document couldn't be opened, woken, hibernated, edited or saved.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file or the operation log couldn't be read or written.
    Io(std::io::Error),
    /// MuPDF couldn't open, read, edit or write it.
    Mupdf(MuError),
    /// The document has unsaved changes, so it stays open.
    UnsavedChanges,
    /// Only a PDF can be edited and saved.
    ReadOnly,
    /// The file written didn't read back with the document's pages, so it didn't replace the old one.
    Unverified,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot read or write the file: {err}"),
            Self::Mupdf(err) => write!(f, "{err}"),
            Self::UnsavedChanges => f.write_str("the document has unsaved changes"),
            Self::ReadOnly => f.write_str("only a PDF can be edited"),
            Self::Unverified => f.write_str("the saved file did not read back the same"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Mupdf(err) => Some(err),
            Self::UnsavedChanges | Self::ReadOnly | Self::Unverified => None,
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
    /// The path it was opened by, to open it again after hibernating and to save over.
    path: PathBuf,
    /// Its operation log's file; `None` when it can't be edited (not a PDF).
    log_path: Option<PathBuf>,
    state: Mutex<State>,
}

struct State {
    /// Which version of the file it is; a save makes a new one.
    id: FileId,
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
    /// The operation log and the undo history it records; `None` when the document can't be edited.
    log: Option<Log>,
    /// Edits an earlier run logged and never saved, until they are recovered or discarded.
    recovery: Option<oplog::Read>,
    /// Goes up with every operation applied, undone or redone, and every recovery.
    edits: u64,
}

impl State {
    fn unsaved(&self) -> bool {
        self.log.as_ref().is_some_and(|log| log.history().unsaved())
    }

    /// Every page may have changed: every list goes and every revision goes up.
    /// Page `page` changed: its revision goes up and its display list goes.
    fn page_changed(&mut self, page: usize) {
        *self.revisions.entry(page).or_insert(0) += 1;
        self.lists.remove(&page);
    }

    fn all_pages_changed(&mut self) {
        self.layouts += 1;
        self.lists.clear();
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("Document")
            .field("id", &state.id)
            .field("hibernating", &state.mupdf.is_none())
            .field("lists", &state.lists.len())
            .field("unsaved", &state.unsaved())
            .finish()
    }
}

impl Document {
    /// Opens the file at `path`, logging edits in the default folder (`registry::default_log_dir`).
    ///
    /// # Errors
    ///
    /// The file can't be read, or MuPDF can't open it.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_as(FileId::of(path)?, path, &crate::registry::default_log_dir())
    }

    /// Opens the file at `path`, already known as `id`. A PDF logs its edits in `log_dir`; edits left there by an
    /// earlier run are found ([`Document::is_recoverable`]).
    pub(crate) fn open_as(id: FileId, path: &Path, log_dir: &Path) -> Result<Self> {
        let pdf = is_pdf_file(path)?;
        let mupdf = open_mupdf(path, pdf)?;
        let log_path = pdf.then(|| log_dir.join(format!("{:016x}.oplog", id.path_key())));
        let recovery = log_path.as_deref().and_then(|log| recovery(log, &id));
        let log = log_path.clone().map(|log| Log::new(log, id.fingerprint()));
        Ok(Self {
            path: path.to_owned(),
            log_path,
            state: Mutex::new(State {
                id,
                mupdf: Some(Arc::new(Mutex::new(mupdf))),
                lists: HashMap::new(),
                revisions: HashMap::new(),
                layouts: 0,
                reflow: None,
                log,
                recovery,
                edits: 0,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("document lock poisoned")
    }

    /// Which file it is, as last opened or saved.
    pub fn id(&self) -> FileId {
        self.lock().id.clone()
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
        let id = FileId::of(&self.path)?;
        let mut document = open_mupdf(&self.path, self.log_path.is_some())?;
        if let Some(reflow) = state.reflow {
            document.reflow(reflow, None)?;
        }
        // A new journal: the log starts again from the file as it is now.
        if let Some(log) = state.log.as_mut() {
            log.restart(id.fingerprint());
        }
        state.id = id;
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
        if state.unsaved() {
            return Err(Error::UnsavedChanges);
        }
        state.lists.clear();
        state.mupdf = None;
        // Everything is saved, and the journal goes with the document.
        if let Some(log) = state.log.as_mut() {
            log.remove();
        }
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
        state.all_pages_changed();
        Ok(kept)
    }

    /// Page `page` changed: its revision goes up and its display list goes.
    pub fn page_changed(&self, page: usize) {
        self.lock().page_changed(page);
    }

    /// Whether it can be edited: it is a PDF.
    pub fn is_editable(&self) -> bool {
        self.log_path.is_some()
    }

    /// A number that goes up with every edit applied, undone, redone or recovered: a view of what edits change
    /// (the outline) reads it again when it moves.
    pub fn edits(&self) -> u64 {
        self.lock().edits
    }

    /// The link to (`x`, `y`), in page points, on page `page` (counting from zero) at `zoom` (1 is 100 %; `None` keeps
    /// the reader's), as an outline entry takes it (B.1, B.4): `#page=4&zoom=150,72,92`.
    ///
    /// # Errors
    ///
    /// The document doesn't wake, or MuPDF can't make the link.
    pub fn link_uri(&self, page: usize, x: f32, y: f32, zoom: Option<f32>) -> Result<String> {
        let shared = self.mupdf()?;
        let mupdf = shared.lock().expect("document lock poisoned");
        Ok(mupdf.link_uri(page, excellent_mupdf::PagePoint { x, y }, zoom)?)
    }

    /// Applies `operation` as one step of undo, logged before it counts. An operation without changes does nothing.
    ///
    /// # Errors
    ///
    /// [`Error::ReadOnly`] for a document that isn't a PDF; MuPDF couldn't make a change, or the log couldn't be
    /// written. Either way nothing changed.
    pub fn apply(&self, operation: &Operation) -> Result<()> {
        if !self.is_editable() {
            return Err(Error::ReadOnly);
        }
        if operation.changes.is_empty() {
            return Ok(());
        }
        // The MuPDF document, then the state: the order every method takes them in.
        let shared = self.mupdf()?;
        let mut mupdf = shared.lock().expect("document lock poisoned");
        let mut state = self.lock();
        let log = state.log.as_mut().ok_or(Error::ReadOnly)?;
        operation.apply(&mut mupdf)?;
        if let Err(err) = log.operation(operation) {
            // An edit the log can't recover isn't kept.
            let _ = mupdf.undo();
            return Err(err.into());
        }
        // A new log replaced the earlier run's.
        state.recovery = None;
        for page in operation.pages() {
            state.page_changed(page);
        }
        state.edits += 1;
        Ok(())
    }

    /// Takes back the last operation; `false` when there is none.
    ///
    /// # Errors
    ///
    /// MuPDF couldn't undo it, or the log couldn't be written (then it stays done).
    pub fn undo(&self) -> Result<bool> {
        self.step(Direction::Back)
    }

    /// Does the last undone operation again; `false` when there is none.
    ///
    /// # Errors
    ///
    /// MuPDF couldn't redo it, or the log couldn't be written (then it stays undone).
    pub fn redo(&self) -> Result<bool> {
        self.step(Direction::Forward)
    }

    fn step(&self, direction: Direction) -> Result<bool> {
        let shared = self.mupdf()?;
        let mut mupdf = shared.lock().expect("document lock poisoned");
        let mut state = self.lock();
        let Some(log) = state.log.as_mut() else {
            return Ok(false);
        };
        let undo = log.history().state();
        let pages = match direction {
            Direction::Back => log.history().undo_pages(),
            Direction::Forward => log.history().redo_pages(),
        }
        .to_vec();
        match direction {
            Direction::Back if undo.can_undo() => {
                mupdf.undo()?;
                if let Err(err) = log.undo() {
                    let _ = mupdf.redo();
                    return Err(err.into());
                }
            }
            Direction::Forward if undo.can_redo() => {
                mupdf.redo()?;
                if let Err(err) = log.redo() {
                    let _ = mupdf.undo();
                    return Err(err.into());
                }
            }
            Direction::Back | Direction::Forward => return Ok(false),
        }
        for page in pages {
            state.page_changed(page);
        }
        state.edits += 1;
        Ok(true)
    }

    /// Where the document is in its undo history.
    pub fn undo_state(&self) -> UndoState {
        self.lock()
            .log
            .as_ref()
            .map(|log| log.history().state())
            .unwrap_or_default()
    }

    /// Whether it has changes not yet saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.lock().unsaved()
    }

    /// Writes its edits over its file (K.15): beside it first, read back, then moved over it. Undo goes on working
    /// after a save, except when the file had to be rewritten whole (a file MuPDF repaired): then the document is
    /// opened again from the new file and its history starts there.
    ///
    /// # Errors
    ///
    /// The file couldn't be written or verified (the old one stays as it was), or the log couldn't record the
    /// save.
    pub fn save(&self) -> Result<()> {
        let Some(shared) = self.lock().mupdf.clone() else {
            return Ok(()); // Hibernating: nothing is unsaved.
        };
        let mut mupdf = shared.lock().expect("document lock poisoned");
        if !self.has_unsaved_changes() {
            return Ok(());
        }
        let save = crate::save::replace(&mut mupdf, &self.path)?;
        let id = FileId::of(&self.path)?;
        let mut state = self.lock();
        state.id = id.clone();
        match save {
            Save::Incremental => {
                if let Some(log) = state.log.as_mut() {
                    log.saved(id.fingerprint())?;
                }
            }
            Save::Rewritten => {
                // The log's operations apply to bytes the new file no longer starts with: start both again from it.
                *mupdf = excellent_mupdf::Document::open_pdf(&self.path)?;
                if let Some(log) = state.log.as_mut() {
                    log.restart(id.fingerprint());
                }
                state.all_pages_changed();
                state.edits += 1;
            }
        }
        Ok(())
    }

    /// Whether an earlier run left edits to this file that it never saved.
    pub fn is_recoverable(&self) -> bool {
        self.lock().recovery.is_some()
    }

    /// Brings back the edits an earlier run never saved, with their undo history: its log replayed over the file as
    /// the log started. They are unsaved until the next save. Returns the names of the edits the saved file doesn't
    /// have, in the order they were made (A.15's "Recovered changes" bar lists them).
    ///
    /// # Errors
    ///
    /// The file as the log started can't be had back, or the log doesn't replay; the document stays as it was.
    pub fn recover(&self) -> Result<Vec<String>> {
        let (Some(log_path), shared) = (self.log_path.clone(), self.mupdf()?) else {
            return Ok(Vec::new());
        };
        let mut mupdf = shared.lock().expect("document lock poisoned");
        let mut state = self.lock();
        let Some(read) = state.recovery.take() else {
            return Ok(Vec::new());
        };
        let mut document = replay(&self.path, &log_path, &read)?;
        std::mem::swap(&mut *mupdf, &mut document);
        state.log = Some(Log::resume(log_path, &read)?);
        state.all_pages_changed();
        state.edits += 1;
        Ok(read.restored())
    }

    /// Drops every unsaved edit (A.15's Discard): the document is its file as last saved again, its undo history
    /// starts again and its log goes.
    ///
    /// # Errors
    ///
    /// MuPDF can't open the file again; the document stays as it was.
    pub fn revert(&self) -> Result<()> {
        if !self.is_editable() {
            return Ok(());
        }
        let shared = self.mupdf()?;
        let mut mupdf = shared.lock().expect("document lock poisoned");
        let mut state = self.lock();
        let mut reopened = excellent_mupdf::Document::open_pdf(&self.path)?;
        std::mem::swap(&mut *mupdf, &mut reopened);
        let base = state.id.fingerprint();
        if let Some(log) = state.log.as_mut() {
            log.restart(base);
        }
        state.recovery = None;
        state.all_pages_changed();
        state.edits += 1;
        Ok(())
    }

    /// Forgets the edits an earlier run never saved: the file stays as it is.
    pub fn discard_recovery(&self) {
        let mut state = self.lock();
        if state.recovery.take().is_some()
            && let Some(log_path) = &self.log_path
            && let Err(err) = fs::remove_file(log_path)
        {
            log::warn!("oplog: can't remove {}: {err}", log_path.display());
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        // A log with nothing unsaved has nothing to recover.
        if let Ok(state) = self.state.get_mut()
            && !state.unsaved()
            && let Some(log) = state.log.as_mut()
        {
            log.remove();
        }
    }
}

/// Which way [`Document::step`] goes through the history.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Back,
    Forward,
}

/// Whether the file at `path` is a PDF, by its first bytes.
fn is_pdf_file(path: &Path) -> io::Result<bool> {
    let mut head = Vec::with_capacity(1024);
    File::open(path)?.take(1024).read_to_end(&mut head)?;
    Ok(crate::formats::is_pdf(&head))
}

/// Opens the file at `path`: a PDF so it can be edited and saved over.
fn open_mupdf(path: &Path, pdf: bool) -> Result<excellent_mupdf::Document> {
    Ok(if pdf {
        excellent_mupdf::Document::open_pdf(path)?
    } else {
        excellent_mupdf::Document::open(path)?
    })
}

/// The unsaved edits the log at `log` holds for the file `id`. A log with nothing unsaved is removed; a log of
/// another version of the file is left, and can't be replayed.
fn recovery(log: &Path, id: &FileId) -> Option<oplog::Read> {
    match oplog::read(log) {
        Ok(Some(read)) if read.history.unsaved() && read.last == id.fingerprint() => Some(read),
        Ok(Some(read)) if read.history.unsaved() => {
            // shortcut: edits to a file changed since are dropped; K.15 step 5 offers "keep both" for this.
            log::warn!(
                "oplog: {} changed since its unsaved edits were logged; they can't be replayed",
                id.path().display()
            );
            None
        }
        Ok(_) => {
            let _ = fs::remove_file(log);
            None
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => {
            log::warn!("oplog: can't read {}: {err}", log.display());
            None
        }
    }
}

/// The document `read` describes: the file at `path` as the log started (its first bytes: every save since was
/// incremental), with every logged operation, undo and redo replayed.
fn replay(path: &Path, log_path: &Path, read: &oplog::Read) -> Result<excellent_mupdf::Document> {
    let base = log_path.with_extension("base.pdf");
    let copied = io::copy(
        &mut File::open(path)?.take(read.base.size),
        &mut File::create(&base)?,
    );
    let document = copied.map_err(Error::from).and_then(|copied| {
        if copied != read.base.size || !FileId::of(&base)?.same_bytes(read.base) {
            return Err(Error::Unverified);
        }
        let mut document = excellent_mupdf::Document::open_pdf(&base)?;
        for record in &read.records {
            match record {
                Record::Operation { operation, .. } => operation.apply(&mut document)?,
                Record::Undo => document.undo()?,
                Record::Redo => document.redo()?,
                Record::Start { .. } | Record::Saved { .. } => {}
            }
        }
        Ok(document)
    });
    // The document goes on reading the copy it opened (its handle allows deleting it).
    let _ = fs::remove_file(&base);
    document
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{PagePoint, PageRect};

    use super::*;
    use crate::edit::Change;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name)
    }

    /// A copy of a corpus file and a log folder of their own, removed when the test ends.
    struct Scratch {
        dir: PathBuf,
        file: PathBuf,
        logs: PathBuf,
    }

    impl Scratch {
        fn new(test: &str, from: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("excellent-core-doc-{test}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch folder");
            let file = dir.join(from);
            fs::copy(corpus(from), &file).expect("copy");
            let logs = dir.join("logs");
            Self { dir, file, logs }
        }

        fn open(&self) -> Document {
            Document::open_as(FileId::of(&self.file).expect("id"), &self.file, &self.logs)
                .expect("open")
        }

        fn logs(&self) -> usize {
            fs::read_dir(&self.logs).map_or(0, Iterator::count)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn page_count(document: &Document) -> usize {
        let mupdf = document.mupdf().expect("awake");
        let mupdf = mupdf.lock().expect("lock");
        mupdf.page_count().expect("count")
    }

    fn titles(document: &Document) -> Vec<String> {
        let mupdf = document.mupdf().expect("awake");
        let mupdf = mupdf.lock().expect("lock");
        let outline = mupdf.outline().expect("outline");
        outline.into_iter().map(|item| item.title).collect()
    }

    fn add(document: &Document, title: &str, at: usize) -> Operation {
        let uri = document
            .mupdf()
            .expect("awake")
            .lock()
            .expect("lock")
            .link_uri(1, PagePoint { x: 0.0, y: 0.0 }, None)
            .expect("uri");
        Operation::new(
            "Add outline entry",
            vec![Change::InsertOutlineEntry {
                at: vec![at],
                title: title.into(),
                uri,
            }],
        )
    }

    #[test]
    fn hibernates_only_without_unsaved_changes() {
        let scratch = Scratch::new("hibernate", "base14-outline-annot.pdf");
        let document = scratch.open();
        let pages = page_count(&document);

        document.apply(&add(&document, "New", 0)).expect("apply");
        assert!(matches!(document.hibernate(), Err(Error::UnsavedChanges)));
        assert!(!document.is_hibernating(), "unsaved changes keep it open");

        document.save().expect("save");
        document.hibernate().expect("hibernate");
        assert!(document.is_hibernating());
        assert_eq!(page_count(&document), pages, "a view wakes it");
        assert!(!document.is_hibernating());
        assert_eq!(titles(&document), ["New", "Page one", "Page two"]);
        assert!(
            !document.undo_state().can_undo(),
            "the history starts again"
        );
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
    fn an_annotation_edit_changes_its_page_and_its_marks() {
        use crate::annot::{self, Annotation, Boxed, Shape, Style};

        let scratch = Scratch::new("marks", "base14-outline-annot.pdf");
        let document = scratch.open();
        let marks = |page| {
            let mupdf = document.mupdf().expect("awake");
            let mupdf = mupdf.lock().expect("lock");
            annot::page_marks(&mupdf, page).expect("marks")
        };
        let before = marks(1);
        let rect = PageRect {
            x0: 10.0,
            y0: 20.0,
            x1: 30.0,
            y1: 40.0,
        };
        let note = Annotation::new(1, Shape::Boxed(Boxed::Note, rect), Style::default(), "Ada");
        let create = Operation::new("Add note", vec![Change::CreateAnnotation(note)]);
        assert_eq!(create.pages(), [1]);

        document.apply(&create).expect("apply");
        assert_eq!((document.revision(0), document.revision(1)), (0, 1));
        assert_eq!(marks(1).len(), before.len() + 1, "the new note is marked");
        assert!(document.undo().expect("undo"));
        assert_eq!(document.revision(1), 2, "undo draws it again");
        assert_eq!(marks(1), before);
        assert!(document.redo().expect("redo"));
        assert_eq!(document.revision(1), 3, "and so does redo");
        assert_eq!(document.revision(0), 0, "the other page is untouched");
    }

    #[test]
    fn edits_are_logged_undone_and_saved_over_the_file() {
        let scratch = Scratch::new("save", "base14-outline-annot.pdf");
        let document = scratch.open();
        assert!(document.is_editable());
        assert!(!document.has_unsaved_changes());
        assert_eq!(scratch.logs(), 0, "nothing is written until an edit");

        document.apply(&add(&document, "Added", 2)).expect("apply");
        assert!(document.has_unsaved_changes());
        assert_eq!(document.edits(), 1);
        assert_eq!(scratch.logs(), 1);
        let before = document.id();
        document.save().expect("save");
        assert!(!document.has_unsaved_changes());
        assert_ne!(document.id(), before, "a saved file is a new version");
        let reopened = Document::open(&scratch.file).expect("reopen");
        assert_eq!(titles(&reopened), ["Page one", "Page two", "Added"]);

        // Undo goes on across the save; the saved state is reached again by redo.
        assert!(document.undo().expect("undo"));
        assert!(document.has_unsaved_changes());
        assert_eq!(titles(&document), ["Page one", "Page two"]);
        assert!(document.redo().expect("redo"));
        assert!(!document.has_unsaved_changes());
        assert!(!document.redo().expect("redo"), "nothing more to redo");

        drop(document);
        assert_eq!(scratch.logs(), 0, "a saved document leaves no log");
    }

    #[test]
    fn a_failed_edit_changes_nothing_and_is_not_logged() {
        let scratch = Scratch::new("failed", "base14-outline-annot.pdf");
        let document = scratch.open();
        let delete = Operation::new("Delete", vec![Change::DeleteOutlineEntry { at: vec![9] }]);
        assert!(document.apply(&delete).is_err());
        assert!(!document.has_unsaved_changes());
        assert_eq!(document.edits(), 0);
        assert_eq!(scratch.logs(), 0);
        document
            .apply(&Operation::new("Nothing", Vec::new()))
            .expect("nothing to do");
        assert!(!document.undo_state().can_undo(), "and no step to undo");
    }

    #[test]
    fn unsaved_edits_come_back_after_a_crash() {
        let scratch = Scratch::new("recover", "base14-outline-annot.pdf");
        {
            let document = scratch.open();
            document.apply(&add(&document, "Saved", 0)).expect("apply");
            document.save().expect("save");
            document.apply(&add(&document, "Kept", 0)).expect("apply");
            document.apply(&add(&document, "Undone", 0)).expect("apply");
            assert!(document.undo().expect("undo"));
            // The process dies: nothing more is written.
        }
        let document = scratch.open();
        assert!(document.is_recoverable());
        assert_eq!(titles(&document), ["Saved", "Page one", "Page two"]);
        assert_eq!(
            document.recover().expect("recover"),
            ["Add outline entry"],
            "the edit the saved file lacks; the undone one isn't listed"
        );
        assert!(!document.is_recoverable());
        assert!(document.has_unsaved_changes());
        assert_eq!(titles(&document), ["Kept", "Saved", "Page one", "Page two"]);
        assert_eq!(
            document.undo_state(),
            UndoState {
                position: 2,
                steps: 3
            },
            "the history comes back too"
        );
        assert!(document.redo().expect("redo"));
        assert_eq!(titles(&document)[0], "Undone");
        document.save().expect("save");
        drop(document);

        let document = scratch.open();
        assert!(!document.is_recoverable(), "all of it was saved");
        assert_eq!(titles(&document).len(), 5);
    }

    #[test]
    fn ten_annotations_drawn_before_a_crash_all_come_back() {
        use crate::annot::{Annotation, Boxed, Shape, Style, page_annotations};

        let scratch = Scratch::new("ten", "base14-outline-annot.pdf");
        let count = |document: &Document| {
            let mupdf = document.mupdf().expect("awake");
            let mupdf = mupdf.lock().expect("lock");
            page_annotations(&mupdf, 0).expect("annotations").len()
        };
        let before = count(&scratch.open());
        {
            let document = scratch.open();
            for n in 0..10u8 {
                let at = 20.0 + f32::from(n) * 30.0;
                let note = Annotation::new(
                    0,
                    Shape::Boxed(
                        Boxed::Note,
                        PageRect {
                            x0: at,
                            y0: 40.0,
                            x1: at + 20.0,
                            y1: 60.0,
                        },
                    ),
                    Style::default(),
                    "Ada",
                );
                let operation = Operation::new("Sticky note", vec![Change::CreateAnnotation(note)]);
                document.apply(&operation).expect("apply");
            }
            // The app is killed: nothing is saved.
        }
        let document = scratch.open();
        assert_eq!(count(&document), before, "the file has none of them");
        let restored = document.recover().expect("recover");
        assert_eq!(restored.len(), 10);
        assert_eq!(count(&document), before + 10, "all ten come back");

        document.revert().expect("revert");
        assert_eq!(count(&document), before, "discarded, the file is as it was");
        assert!(!document.has_unsaved_changes());
        assert!(!document.undo_state().can_undo());
        drop(document);
        assert_eq!(scratch.logs(), 0, "and no log is left");
    }

    #[test]
    fn discarded_edits_are_gone() {
        let scratch = Scratch::new("discard", "base14-outline-annot.pdf");
        {
            let document = scratch.open();
            document.apply(&add(&document, "Lost", 0)).expect("apply");
        }
        let document = scratch.open();
        assert!(document.is_recoverable());
        document.discard_recovery();
        assert!(!document.is_recoverable());
        assert_eq!(scratch.logs(), 0);
        assert_eq!(titles(&document), ["Page one", "Page two"]);
    }

    #[test]
    fn only_a_pdf_is_edited() {
        let document = Document::open(&corpus("formats.epub")).expect("open");
        assert!(!document.is_editable());
        let delete = Operation::new("Delete", vec![Change::DeleteOutlineEntry { at: vec![0] }]);
        assert!(matches!(document.apply(&delete), Err(Error::ReadOnly)));
        assert!(!document.undo().expect("undo"));
        document.save().expect("nothing to save");
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
