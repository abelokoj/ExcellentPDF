//! The Linux print dialog and queue: GTK's `GtkPrintDialog` (GTK 4.14), which uses the desktop's print portal where
//! there is one (Flatpak, Snap) and the GTK dialog otherwise, then sends the spool with the setup the reader chose.
//!
//! GTK wants both on its main thread; the job's thread waits on a channel for them. The setup itself, a GTK object,
//! stays on the main thread, under the job's id.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};

use nsh_platform_linux::gtk4 as gtk;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::spool::Sheet;
use super::spooled::{Owner, Printer};
use super::{Choice, Range, failed};

/// The resolution bands are drawn at when the printer doesn't say, and the most the spool takes: higher only makes
/// the spool larger, not the print sharper on office printers.
const DEFAULT_DPI: f32 = 300.0;
const MAX_DPI: f32 = 600.0;

thread_local! {
    /// The setups chosen in the dialog, by job, on the main thread.
    static SETUPS: RefCell<HashMap<u64, gtk::PrintSetup>> = RefCell::new(HashMap::new());
}

/// The print queue the dialog chose: its setup, kept on the main thread under this id.
#[derive(Debug)]
pub(super) struct Queue {
    id: u64,
}

impl Queue {
    /// Sends the PDF at `path` to the chosen printer.
    pub(super) fn send(&self, path: &Path, _title: &str) -> io::Result<()> {
        // shortcut: the print queue names the job after the spool's file name, not the document's title; name the
        // spool after the title if readers ask.
        let (reply, sent) = mpsc::channel();
        let (id, path) = (self.id, path.to_owned());
        glib::MainContext::default().invoke(move || print_file(id, path, reply));
        sent.recv()
            .map_err(|_| failed("the print queue closed without an answer"))?
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        let id = self.id;
        glib::MainContext::default().invoke(move || {
            SETUPS.with_borrow_mut(|setups| setups.remove(&id));
        });
    }
}

/// Shows the system print dialog for a document of `pages` pages. `None` when the reader cancels. Never call it on
/// GTK's main thread: it waits for that thread to show the dialog.
///
/// # Errors
///
/// The dialog failed.
pub fn ask(_owner: Owner, pages: u32) -> io::Result<Option<Choice>> {
    let pages = pages.max(1);
    let (reply, answer) = mpsc::channel();
    glib::MainContext::default().invoke(move || show(pages, reply));
    let chosen = answer
        .recv()
        .map_err(|_| failed("the print dialog closed without an answer"))??;
    Ok(chosen.map(|(id, sheet, range)| Choice {
        printer: Printer::new(Queue { id }, sheet),
        range,
    }))
}

type Answer = io::Result<Option<(u64, Sheet, Range)>>;

fn show(pages: u32, reply: Sender<Answer>) {
    let dialog = gtk::PrintDialog::builder().modal(true).build();
    dialog.setup(
        active_window().as_ref(),
        None::<&gio::Cancellable>,
        move |result| {
            let answer = match result {
                Ok(setup) => Ok(Some(chosen(setup, pages))),
                Err(err)
                    if err.matches(gtk::DialogError::Dismissed)
                        || err.matches(gtk::DialogError::Cancelled) =>
                {
                    Ok(None)
                }
                Err(err) => Err(failed(err.to_string())),
            };
            let _ = reply.send(answer);
        },
    );
}

/// The pages and the paper `setup` names, the setup kept for the job.
fn chosen(setup: gtk::PrintSetup, pages: u32) -> (u64, Sheet, Range) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let settings = setup.print_settings();
    let range = match settings.print_pages() {
        gtk::PrintPages::Current => Range::Current,
        gtk::PrintPages::Ranges => Range::Pages(
            settings
                .page_ranges()
                .iter()
                // GTK counts from 0.
                .map(|range| (from_zero(range.start()), from_zero(range.end()).min(pages)))
                .collect(),
        ),
        _ => Range::All,
    };
    // The spool holds only the pages chosen: the queue prints all of it.
    settings.set_print_pages(gtk::PrintPages::All);
    let page = setup.page_setup();
    let points = gtk::Unit::Points;
    let dpi = match settings.resolution() {
        dpi if dpi > 0 => (dpi as f32).min(MAX_DPI),
        _ => DEFAULT_DPI,
    };
    // All follow the chosen orientation.
    let sheet = Sheet {
        paper_width: page.paper_width(points) as f32,
        paper_height: page.paper_height(points) as f32,
        left: page.left_margin(points) as f32,
        top: page.top_margin(points) as f32,
        width: page.page_width(points) as f32,
        height: page.page_height(points) as f32,
        dpi,
    };
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    SETUPS.with_borrow_mut(|setups| setups.insert(id, setup));
    (id, sheet, range)
}

fn print_file(id: u64, path: PathBuf, reply: Sender<io::Result<()>>) {
    let Some(setup) = SETUPS.with_borrow(|setups| setups.get(&id).cloned()) else {
        let _ = reply.send(Err(failed("the print setup is gone")));
        return;
    };
    let dialog = gtk::PrintDialog::builder().modal(true).build();
    dialog.print_file(
        active_window().as_ref(),
        Some(&setup),
        &gio::File::for_path(path),
        None::<&gio::Cancellable>,
        move |result| {
            let _ = reply.send(result.map_err(|err| failed(err.to_string())));
        },
    );
}

/// The app's active window, the dialog's parent.
fn active_window() -> Option<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.is_active())
}

/// A page number from 0, as GTK gives it, from 1.
fn from_zero(page: i32) -> u32 {
    u32::try_from(page).unwrap_or(0).saturating_add(1)
}
