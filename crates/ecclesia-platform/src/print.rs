//! Printing (plan O.22), the operating system's side: the system print dialog and the printer it returns, drawn on
//! in bands of RGB pixels. What to draw is the app's (`ecclesia_core::print`).
//!
//! The same calls on every desktop:
//!
//! - Windows: `PrintDlgEx` and a GDI printer device context, drawn on directly.
//! - macOS: `NSPrintPanel` over the shared `NSPrintInfo`; the bands are spooled to a PDF that PrintCore sends to the
//!   printer the panel chose, with its settings.
//! - Linux: GTK's `GtkPrintDialog` (the print portal where there is one); the same spool, sent with the setup the
//!   reader chose.
//!
//! The dialog is modal to the app's window but [`ask`] blocks the calling thread until it is answered: call it, and
//! print, from a thread of your own (on macOS and Linux never the UI thread, which runs the dialog).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Owner, Printer, ask, owner};

#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod spool;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod spooled;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use spooled::{Owner, Printer, owner};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::ask;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::ask;

/// The pages the reader chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Range {
    /// Every page.
    All,
    /// The page shown.
    Current,
    /// Pages from 1, both ends kept, as typed ("1-3, 7").
    Pages(Vec<(u32, u32)>),
}

/// What the dialog answered with Print: the printer, set up as chosen (copies, collation, duplex and colour are in
/// its settings), and the pages.
#[derive(Debug)]
pub struct Choice {
    /// The printer to draw on.
    pub printer: Printer,
    /// The pages chosen.
    pub range: Range,
}

/// The printable area of the paper: its size in device pixels and the printer's resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
    /// Horizontal dots per inch.
    pub dpi_x: f32,
    /// Vertical dots per inch.
    pub dpi_y: f32,
}

/// An I/O error carrying `message`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn failed(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}
