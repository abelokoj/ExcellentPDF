//! The Windows printer: `PrintDlgEx` and a GDI printer device context.

use std::io;

use windows::Win32::Foundation::{GlobalFree, HWND};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HDC, HORZRES,
    LOGPIXELSX, LOGPIXELSY, SRCCOPY, StretchDIBits, VERTRES,
};
use windows::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Controls::Dialogs::{
    PD_CURRENTPAGE, PD_NOSELECTION, PD_PAGENUMS, PD_RESULT_PRINT, PD_RETURNDC,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, PRINTPAGERANGE, PrintDlgExW, START_PAGE_GENERAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::core::PCWSTR;

use super::{Area, Choice, Range};

/// How many page ranges the dialog takes ("1-3, 7, 9-12" is three).
const MAX_RANGES: usize = 32;

/// The window a dialog belongs to, as a plain number so it can go to the dialog's thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner(usize);

/// The calling thread's active window: call it on the UI thread, before handing the job to another.
pub fn owner() -> Owner {
    // SAFETY: no arguments; it reads this thread's window state.
    let window = unsafe { GetActiveWindow() };
    Owner(window.0 as usize)
}

/// Shows the system print dialog for a document of `pages` pages over `owner`. `None` when the reader cancels.
///
/// # Errors
///
/// The dialog couldn't be shown, or the printer couldn't be opened.
pub fn ask(owner: Owner, pages: u32) -> io::Result<Option<Choice>> {
    // SAFETY: COM for this thread, undone below when this call started it.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let answer = show(owner, pages.max(1));
    if com {
        // SAFETY: matches the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    answer
}

fn show(owner: Owner, pages: u32) -> io::Result<Option<Choice>> {
    let mut ranges = [PRINTPAGERANGE::default(); MAX_RANGES];
    let mut dialog = PRINTDLGEXW {
        lStructSize: size_of::<PRINTDLGEXW>() as u32,
        hwndOwner: HWND(owner.0 as *mut _),
        Flags: PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE,
        nMaxPageRanges: MAX_RANGES as u32,
        lpPageRanges: ranges.as_mut_ptr(),
        nMinPage: 1,
        nMaxPage: pages,
        nCopies: 1,
        nStartPage: START_PAGE_GENERAL,
        ..Default::default()
    };
    // SAFETY: `dialog` is filled as PrintDlgEx documents, and `ranges` outlives the call.
    let shown = unsafe { PrintDlgExW(&mut dialog) };
    // SAFETY: the dialog's settings handles are ours to free, whatever the answer; null ones are skipped.
    unsafe {
        if !dialog.hDevMode.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevMode));
        }
        if !dialog.hDevNames.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevNames));
        }
    }
    let printer = (!dialog.hDC.is_invalid()).then(|| Printer {
        dc: dialog.hDC,
        started: false,
    });
    shown.map_err(io::Error::other)?;
    if dialog.dwResultAction != PD_RESULT_PRINT {
        return Ok(None);
    }
    let printer = printer.ok_or_else(|| io::Error::other("the print dialog gave no printer"))?;
    let range = if dialog.Flags.contains(PD_CURRENTPAGE) {
        Range::Current
    } else if dialog.Flags.contains(PD_PAGENUMS) {
        let chosen = &ranges[..(dialog.nPageRanges as usize).min(MAX_RANGES)];
        Range::Pages(
            chosen
                .iter()
                .map(|range| (range.nFromPage, range.nToPage))
                .collect(),
        )
    } else {
        Range::All
    };
    Ok(Some(Choice { printer, range }))
}

/// A printer chosen in the dialog: start a document, draw its pages a band at a time, then finish. Dropped before
/// [`Printer::finish`], the document is cancelled at the printer.
#[derive(Debug)]
pub struct Printer {
    dc: HDC,
    started: bool,
}

// SAFETY: a printer device context may be used from any thread, one at a time; `Printer` owns it and is not `Sync`.
unsafe impl Send for Printer {}

impl Printer {
    /// The printable area, from the printer's settings.
    pub fn area(&self) -> Area {
        // SAFETY: the device context is ours and live.
        let cap = |index| unsafe { GetDeviceCaps(Some(self.dc), index) };
        Area {
            width: cap(HORZRES).max(1) as u32,
            height: cap(VERTRES).max(1) as u32,
            dpi_x: cap(LOGPIXELSX).max(1) as f32,
            dpi_y: cap(LOGPIXELSY).max(1) as f32,
        }
    }

    /// Starts the printer's document, named `title` in the print queue.
    ///
    /// # Errors
    ///
    /// The printer refused.
    pub fn start(&mut self, title: &str) -> io::Result<()> {
        let name: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let info = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        // SAFETY: the context is live; `info` and the name it points to outlive the call.
        refused(unsafe { StartDocW(self.dc, &info) })?;
        self.started = true;
        Ok(())
    }

    /// Starts a sheet.
    ///
    /// # Errors
    ///
    /// The printer refused.
    pub fn begin_page(&mut self) -> io::Result<()> {
        // SAFETY: the context is live, its document started.
        refused(unsafe { StartPage(self.dc) })
    }

    /// Draws `width` × `height` pixels of RGB (3 bytes each, rows `stride` bytes apart) with the top-left corner at
    /// `(x, y)` of the printable area, one to one.
    ///
    /// # Errors
    ///
    /// The printer refused.
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
        let (w, h) = (width as usize, height as usize);
        assert!(
            stride >= w * 3 && rgb.len() >= stride * h.saturating_sub(1) + w * 3,
            "band of {w}×{h} pixels doesn't fit {} bytes at stride {stride}",
            rgb.len()
        );
        // A DIB's rows are BGR, padded to four bytes.
        let row = (w * 3).next_multiple_of(4);
        let mut bgr = vec![0u8; row * h];
        for (from, to) in rgb.chunks(stride).zip(bgr.chunks_mut(row)) {
            let (pixels, _) = from[..w * 3].as_chunks::<3>();
            let (outs, _) = to.as_chunks_mut::<3>();
            for (&[r, g, b], out) in pixels.iter().zip(outs) {
                *out = [b, g, r];
            }
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative: rows from the top.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let (w, h) = (width as i32, height as i32);
        // SAFETY: the context is live in a started page; `bgr` holds `h` rows as `info` describes.
        let drawn = unsafe {
            StretchDIBits(
                self.dc,
                x,
                y,
                w,
                h,
                0,
                0,
                w,
                h,
                Some(bgr.as_ptr().cast()),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if drawn == 0 {
            return Err(io::Error::other(
                "the printer did not take the page's pixels",
            ));
        }
        Ok(())
    }

    /// Ends the sheet.
    ///
    /// # Errors
    ///
    /// The printer refused.
    pub fn end_page(&mut self) -> io::Result<()> {
        // SAFETY: the context is live in a started page.
        refused(unsafe { EndPage(self.dc) })
    }

    /// Ends the document: the printer prints it.
    ///
    /// # Errors
    ///
    /// The printer refused.
    pub fn finish(mut self) -> io::Result<()> {
        self.started = false;
        // SAFETY: the context is live, its document started.
        refused(unsafe { EndDoc(self.dc) })
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        // SAFETY: we own the context; a document not finished is cancelled before it goes.
        unsafe {
            if self.started {
                AbortDoc(self.dc);
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

/// A GDI print call's answer as a result: zero or less is a refusal.
fn refused(answer: i32) -> io::Result<()> {
    if answer > 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
