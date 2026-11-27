//! The macOS print dialog and queue: `NSPrintPanel` over the app's shared `NSPrintInfo` (so the printer and paper
//! chosen stay chosen, as in other Mac apps), then PrintCore sends the spool to that printer with those settings.
//!
//! AppKit wants both on the main thread; the job's thread waits on the main queue for them.

use std::ffi::c_void;
use std::io;
use std::path::Path;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSPrintInfo, NSPrintPanel, NSPrintPanelOptions};
use objc2_foundation::{NSString, NSURL};

use super::spool::Sheet;
use super::spooled::{Owner, Printer};
use super::{Choice, Range, failed};

/// PrintCore's status: 0 is success.
type OsStatus = i32;

/// `kPMPrintAllPages`: "to the last page".
const ALL_PAGES: u32 = u32::MAX;

/// `NSModalResponseOK`: the panel's Print button.
const PRINT: isize = 1;

/// The resolution bands are drawn at when the printer doesn't say, and the most the spool takes: higher only makes
/// the spool larger, not the print sharper on office printers.
const DEFAULT_DPI: f32 = 300.0;
const MAX_DPI: f32 = 600.0;

#[repr(C)]
#[derive(Debug, Default)]
struct PmResolution {
    horizontal: f64,
    vertical: f64,
}

// PrintCore (ApplicationServices): the C side of `NSPrintInfo`. Its session, settings, page format and printer are
// opaque pointers that `NSPrintInfo` owns.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn PMSessionGetCurrentPrinter(session: *mut c_void, printer: *mut *mut c_void) -> OsStatus;
    fn PMGetFirstPage(settings: *mut c_void, first: *mut u32) -> OsStatus;
    fn PMGetLastPage(settings: *mut c_void, last: *mut u32) -> OsStatus;
    fn PMSetFirstPage(settings: *mut c_void, first: u32, lock: u8) -> OsStatus;
    fn PMSetLastPage(settings: *mut c_void, last: u32, lock: u8) -> OsStatus;
    fn PMPrintSettingsSetJobName(settings: *mut c_void, name: *const c_void) -> OsStatus;
    fn PMPrinterGetOutputResolution(
        printer: *mut c_void,
        settings: *mut c_void,
        resolution: *mut PmResolution,
    ) -> OsStatus;
    fn PMPrinterPrintWithFile(
        printer: *mut c_void,
        settings: *mut c_void,
        format: *mut c_void,
        mime_type: *const c_void,
        file: *const c_void,
    ) -> OsStatus;
}

/// The print queue the panel chose: the shared print info's printer, read again when the job is sent.
#[derive(Debug)]
pub(super) struct Queue;

impl Queue {
    /// Sends the PDF at `path`, named `title`, to the chosen printer.
    pub(super) fn send(&self, path: &Path, title: &str) -> io::Result<()> {
        let mut sent = None;
        DispatchQueue::main().exec_sync(|| sent = Some(print_file(path, title)));
        sent.unwrap_or_else(|| Err(failed("the main queue didn't send the print job")))
    }
}

/// Shows the system print dialog for a document of `pages` pages. `None` when the reader cancels. Never call it on
/// the main thread: it waits for the main thread to show the panel.
///
/// # Errors
///
/// The panel's printer couldn't be read.
pub fn ask(_owner: Owner, pages: u32) -> io::Result<Option<Choice>> {
    let pages = pages.max(1);
    let mut answer = None;
    DispatchQueue::main().exec_sync(|| answer = Some(show(pages)));
    let chosen =
        answer.unwrap_or_else(|| Err(failed("the main queue didn't show the print panel")))?;
    Ok(chosen.map(|(sheet, range)| Choice {
        printer: Printer::new(Queue, sheet),
        range,
    }))
}

fn show(pages: u32) -> io::Result<Option<(Sheet, Range)>> {
    let mtm = MainThreadMarker::new().expect("the main queue runs on the main thread");
    let info = NSPrintInfo::sharedPrintInfo();
    let panel = NSPrintPanel::printPanel(mtm);
    panel.setOptions(
        NSPrintPanelOptions::ShowsCopies
            | NSPrintPanelOptions::ShowsPageRange
            | NSPrintPanelOptions::ShowsPaperSize
            | NSPrintPanelOptions::ShowsOrientation,
    );
    if panel.runModalWithPrintInfo(&info) != PRINT {
        return Ok(None);
    }
    let settings = info.PMPrintSettings().as_ptr();
    let (mut first, mut last) = (1, ALL_PAGES);
    // SAFETY: the settings are the print info's, live while `info` is; the out-pointers are ours.
    unsafe {
        PMGetFirstPage(settings, &mut first);
        PMGetLastPage(settings, &mut last);
    }
    let range = if first <= 1 && last >= pages {
        Range::All
    } else {
        Range::Pages(vec![(first.max(1), last.min(pages))])
    };
    let printer = current_printer(&info)?;
    let mut resolution = PmResolution::default();
    // SAFETY: the printer and settings are the print info's; `resolution` is ours.
    let read = unsafe { PMPrinterGetOutputResolution(printer, settings, &mut resolution) };
    let dpi = if read == 0 && resolution.horizontal > 0.0 {
        (resolution.horizontal.min(resolution.vertical) as f32).min(MAX_DPI)
    } else {
        DEFAULT_DPI
    };
    // Both follow the chosen orientation; the imageable bounds are measured up from the paper's foot.
    let paper = info.paperSize();
    let bounds = info.imageablePageBounds();
    let sheet = Sheet {
        paper_width: paper.width as f32,
        paper_height: paper.height as f32,
        left: bounds.origin.x as f32,
        top: (paper.height - bounds.origin.y - bounds.size.height) as f32,
        width: bounds.size.width as f32,
        height: bounds.size.height as f32,
        dpi,
    };
    Ok(Some((sheet, range)))
}

/// Sends the spool, on the main thread. The range the panel chose is already in the spool, so every page of it
/// prints.
fn print_file(path: &Path, title: &str) -> io::Result<()> {
    let info = NSPrintInfo::sharedPrintInfo();
    let settings = info.PMPrintSettings().as_ptr();
    let name = NSString::from_str(title);
    // SAFETY: the settings are the print info's; `name` is an NSString, toll-free bridged with CFString.
    unsafe {
        PMSetFirstPage(settings, 1, 0);
        PMSetLastPage(settings, ALL_PAGES, 0);
        PMPrintSettingsSetJobName(settings, Retained::as_ptr(&name).cast());
    }
    info.updateFromPMPrintSettings();
    let printer = current_printer(&info)?;
    let url =
        NSURL::from_file_path(path).ok_or_else(|| failed("the print spool has no file URL"))?;
    let pdf = NSString::from_str("application/pdf");
    // SAFETY: the printer, settings and page format are the print info's, live while `info` is; the URL and the
    // type are NSURL and NSString, toll-free bridged with CFURL and CFString.
    let status = unsafe {
        PMPrinterPrintWithFile(
            printer,
            settings,
            info.PMPageFormat().as_ptr(),
            Retained::as_ptr(&pdf).cast(),
            Retained::as_ptr(&url).cast(),
        )
    };
    status_ok(status, "the printer refused the job")
}

fn current_printer(info: &NSPrintInfo) -> io::Result<*mut c_void> {
    let mut printer = std::ptr::null_mut();
    // SAFETY: the session is the print info's; `printer` is ours. The printer is not retained: it lives while the
    // session does.
    let status =
        unsafe { PMSessionGetCurrentPrinter(info.PMPrintSession().as_ptr(), &mut printer) };
    status_ok(status, "no printer is chosen")?;
    if printer.is_null() {
        return Err(failed("no printer is chosen"));
    }
    Ok(printer)
}

fn status_ok(status: OsStatus, what: &str) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(failed(format!("{what} (PrintCore {status})")))
    }
}
