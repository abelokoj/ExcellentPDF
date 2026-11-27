//! The printer on macOS and Linux: the job's bands go to a PDF spool in the temporary folder, which the platform's
//! print queue takes when the job finishes. A job dropped before [`Printer::finish`] sends nothing; its spool is
//! deleted either way.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
use super::linux::Queue;
#[cfg(target_os = "macos")]
use super::macos::Queue;
use super::spool::{Sheet, Spool};
use super::{Area, failed};

/// The window a dialog belongs to. On macOS and Linux the dialog finds the app's active window itself, on the UI
/// thread, so there is nothing to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner;

/// The calling thread's active window: call it on the UI thread, before handing the job to another.
pub fn owner() -> Owner {
    Owner
}

/// A printer chosen in the dialog: start a document, draw its pages a band at a time, then finish. Dropped before
/// [`Printer::finish`], nothing is printed.
#[derive(Debug)]
pub struct Printer {
    queue: Queue,
    sheet: Sheet,
    spool: Option<Spool<BufWriter<File>>>,
    path: PathBuf,
    title: String,
}

impl Printer {
    /// The printer `queue` leads to, its paper `sheet`.
    pub(super) fn new(queue: Queue, sheet: Sheet) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "ecclesiapdf-print-{}-{}.pdf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            queue,
            sheet,
            spool: None,
            path: std::env::temp_dir().join(name),
            title: String::new(),
        }
    }

    /// The printable area, from the printer's settings.
    pub fn area(&self) -> Area {
        self.sheet.area()
    }

    /// Starts the printer's document, named `title` in the print queue.
    ///
    /// # Errors
    ///
    /// The spool couldn't be created.
    pub fn start(&mut self, title: &str) -> io::Result<()> {
        // A new file only: the temporary folder is shared, so a file planted at this name is refused, not followed.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)?;
        self.spool = Some(Spool::new(BufWriter::new(file), self.sheet)?);
        self.title = title.to_owned();
        Ok(())
    }

    /// Starts a sheet.
    ///
    /// # Errors
    ///
    /// The document wasn't started, or the spool couldn't be written.
    pub fn begin_page(&mut self) -> io::Result<()> {
        self.spool()?.begin_page()
    }

    /// Draws `width` × `height` pixels of RGB (3 bytes each, rows `stride` bytes apart) with the top-left corner at
    /// `(x, y)` of the printable area, one to one.
    ///
    /// # Errors
    ///
    /// The document wasn't started, or the spool couldn't be written.
    ///
    /// # Panics
    ///
    /// `rgb` is shorter than `height` rows of `stride` bytes, or a row shorter than `width` pixels.
    pub fn draw_rgb(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        stride: usize,
        rgb: &[u8],
    ) -> io::Result<()> {
        self.spool()?.band(x, y, width, height, stride, rgb)
    }

    /// Ends the sheet.
    ///
    /// # Errors
    ///
    /// The document wasn't started, or the spool couldn't be written.
    pub fn end_page(&mut self) -> io::Result<()> {
        self.spool()?.end_page()
    }

    /// Ends the document: the spool goes to the print queue, and the printer prints it.
    ///
    /// # Errors
    ///
    /// The document wasn't started, the spool couldn't be written, or the print queue refused it.
    pub fn finish(mut self) -> io::Result<()> {
        let spool = self
            .spool
            .take()
            .ok_or_else(|| failed("the print job was never started"))?;
        let file = spool
            .finish()?
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        file.sync_all()?;
        drop(file);
        self.queue.send(&self.path, &self.title)
    }

    fn spool(&mut self) -> io::Result<&mut Spool<BufWriter<File>>> {
        self.spool
            .as_mut()
            .ok_or_else(|| failed("the print job was never started"))
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        // The queue has its own copy once sent; a job not sent never prints.
        self.spool = None;
        let _ = fs::remove_file(&self.path);
    }
}
