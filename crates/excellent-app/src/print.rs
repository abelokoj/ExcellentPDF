//! Printing (plan O.22), the app's part: the Print command (Ctrl+P) opens the system print dialog, the shown document
//! prints on a thread of its own, and a strip at the window's foot shows how far it is, with Cancel.
//!
//! The system dialog chooses the printer, the pages, copies, duplex and colour. How pages are sized on the paper
//! (`print_scaling`) and whether annotations print (`print_annotations`) are settings, in the settings window's
//! Printing group (DD6 in `dev notes/dev_decisions.md`). One job at a time per window.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use excellent_core::print::{Progress, Scaling};
use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::settings::Live;

/// How pages are sized on the paper: `shrink` (the default), `fit` or `actual`.
pub(crate) const PRINT_SCALING: &str = "print_scaling";
/// Whether annotations print; on by default.
pub(crate) const PRINT_ANNOTATIONS: &str = "print_annotations";

/// How often the strip reads the job's progress.
const POLL: Duration = Duration::from_millis(200);
/// How long "Sent to the printer" and "Cancelled" stay.
const NOTICE: Duration = Duration::from_secs(6);
/// The progress bar's width.
const BAR_WIDTH: f32 = 160.0;

/// What the strip shows.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    /// The system dialog is open.
    Asking,
    /// `done` of `total` pages have gone to the printer.
    Printing { done: usize, total: usize },
    /// Every page went to the printer.
    Printed(usize),
    /// The reader cancelled the job.
    Cancelled,
    /// The job stopped: the reason, for the reader.
    Failed(String),
}

impl Status {
    /// Whether a job is under way, so another can't start and Cancel shows.
    fn is_busy(&self) -> bool {
        matches!(self, Self::Asking | Self::Printing { .. })
    }
}

/// A window's print job and its strip.
#[derive(Clone, Copy)]
pub(crate) struct Printing {
    status: Signal<Option<Status>>,
    /// The file printing, for the strip.
    name: Signal<String>,
    progress: Stored<Option<Arc<Progress>>>,
    /// The job and the notice's timer; dropped with the window.
    tasks: Stored<Vec<Spawned<()>>>,
}

impl Printing {
    pub(crate) fn new() -> Self {
        Self {
            status: signal(None),
            name: signal(String::new()),
            progress: stored(None),
            tasks: stored(Vec::new()),
        }
    }

    /// Asks where and what to print of the file at `path` (`pages` pages, the reader on `current`), then prints it.
    /// Does nothing while a job is under way.
    pub(crate) fn start(self, path: PathBuf, pages: usize, current: usize, live: Live) {
        if untracked(|| {
            self.status
                .with(|status| status.as_ref().is_some_and(Status::is_busy))
        }) {
            return;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let job = untracked(|| {
            live.read(|s| Options {
                scaling: Scaling::parse(s.text(PRINT_SCALING)),
                annotations: s.flag(PRINT_ANNOTATIONS).unwrap_or(true),
            })
        });
        self.name.set(name.clone());
        self.status.set(Some(Status::Asking));
        let task = spawn(async move {
            let status = self.run(path, name, pages, current, job).await;
            self.finish(status);
        });
        self.tasks.with(|tasks| {
            tasks.clear();
            tasks.push(task);
        });
    }

    /// Stops the job under way: the page being drawn is dropped and the printer's document cancelled.
    pub(crate) fn cancel(self) {
        self.progress.with(|progress| {
            if let Some(progress) = progress {
                progress.cancel();
            }
        });
    }

    /// Takes the strip away once the job is over.
    fn dismiss(self) {
        self.status.set(None);
    }

    /// Shows how the job ended (`None`: the dialog was cancelled, so nothing shows), and says it to screen readers.
    fn finish(self, status: Option<Status>) {
        self.progress.replace(None);
        if let Some(status) = &status {
            announce_for_accessibility(message(&untracked(|| self.name.get()), status));
        }
        let fades = matches!(status, Some(Status::Printed(_) | Status::Cancelled));
        self.status.set(status);
        if fades {
            let timer = set_timeout(NOTICE, move || self.dismiss());
            self.tasks.with(|tasks| tasks.push(timer));
        }
    }

    /// The dialog, then the job, each on a thread of its own; how it ended.
    #[cfg(windows)]
    async fn run(
        self,
        path: PathBuf,
        name: String,
        pages: usize,
        current: usize,
        options: Options,
    ) -> Option<Status> {
        use excellent_core::print::{self, Job};
        use excellent_platform::print::{self as platform, Range};

        let owner = platform::owner();
        let count = u32::try_from(pages).unwrap_or(u32::MAX);
        let choice = match blocking(move || platform::ask(owner, count)).await {
            Ok(Ok(Some(choice))) => choice,
            Ok(Ok(None)) => return None,
            Ok(Err(err)) => return Some(Status::Failed(err.to_string())),
            Err(err) => return Some(Status::Failed(err.to_string())),
        };
        let chosen = match choice.range {
            Range::All => (0..pages).collect(),
            Range::Current => vec![current.min(pages.saturating_sub(1))],
            Range::Pages(ranges) => print::pages_in(&ranges, pages),
        };
        let total = chosen.len();
        if total == 0 {
            return Some(Status::Failed(t!("print.no_pages").to_string()));
        }
        let job = Job {
            pages: chosen,
            scaling: options.scaling,
            annotations: options.annotations,
        };
        let progress = Progress::new();
        self.progress.replace(Some(Arc::clone(&progress)));
        self.status.set(Some(Status::Printing { done: 0, total }));
        let watched = Arc::clone(&progress);
        let ticker = set_interval(POLL, move || {
            let done = watched.printed();
            self.status.set(Some(Status::Printing { done, total }));
        });
        let printer = choice.printer;
        let sent = blocking(move || job::send(&path, &name, printer, &job, &progress)).await;
        drop(ticker);
        Some(sent.unwrap_or_else(|err| Status::Failed(err.to_string())))
    }

    /// Printing comes to macOS and Linux in Phase 2 (plan O.22).
    #[cfg(not(windows))]
    async fn run(
        self,
        _path: PathBuf,
        _name: String,
        _pages: usize,
        _current: usize,
        _options: Options,
    ) -> Option<Status> {
        Some(Status::Failed(t!("print.unsupported").to_string()))
    }
}

/// What the settings say a job prints.
#[derive(Debug, Clone, Copy)]
struct Options {
    scaling: Scaling,
    annotations: bool,
}

/// The job itself, on its own thread.
#[cfg(windows)]
mod job {
    use std::io;
    use std::path::Path;

    use excellent_core::print::{self, Job, Paper, Pixmap, Progress, Sheets};
    use excellent_core::shared::registry;
    use excellent_platform::print::Printer;

    use super::Status;

    /// The printer as the job draws on it.
    struct Sheet(Printer);

    impl Sheets for Sheet {
        fn begin_page(&mut self) -> io::Result<()> {
            self.0.begin_page()
        }

        fn band(&mut self, x: i32, y: i32, pixmap: &Pixmap) -> io::Result<()> {
            self.0.draw_rgb(
                x,
                y,
                pixmap.width(),
                pixmap.height(),
                pixmap.stride(),
                pixmap.samples(),
            )
        }

        fn end_page(&mut self) -> io::Result<()> {
            self.0.end_page()
        }
    }

    /// Prints `job` of the file at `path` (named `name` in the print queue) on `printer`. The registry's document is
    /// used, so edits not saved yet print too.
    pub(super) fn send(
        path: &Path,
        name: &str,
        mut printer: Printer,
        job: &Job,
        progress: &Progress,
    ) -> Status {
        let opened = registry()
            .open(path)
            .and_then(|document| Ok((document.mupdf()?, document)));
        let (mupdf, document) = match opened {
            Ok(opened) => opened,
            Err(err) => return Status::Failed(err.to_string()),
        };
        let area = printer.area();
        let paper = Paper {
            width: area.width,
            height: area.height,
            dpi_x: area.dpi_x,
            dpi_y: area.dpi_y,
        };
        if let Err(err) = printer.start(name) {
            return Status::Failed(err.to_string());
        }
        let mut sheet = Sheet(printer);
        // Dropping the printer before it finishes cancels its document.
        match print::print(&document, &mupdf, job, paper, &mut sheet, progress) {
            Ok(()) => match sheet.0.finish() {
                Ok(()) => Status::Printed(job.pages.len()),
                Err(err) => Status::Failed(err.to_string()),
            },
            Err(print::Error::Cancelled) => Status::Cancelled,
            Err(err) => Status::Failed(err.to_string()),
        }
    }
}

/// The strip's line for `status` of the file `name`.
fn message(name: &str, status: &Status) -> String {
    match status {
        Status::Asking => t!("print.asking", name = name),
        Status::Printing { done, total } => t!(
            "print.printing",
            name = name,
            page = (done + 1).min(*total),
            total = total
        ),
        Status::Printed(1) => t!("print.printed_one", name = name),
        Status::Printed(pages) => t!("print.printed", name = name, pages = pages),
        Status::Cancelled => t!("print.cancelled", name = name),
        Status::Failed(reason) => t!("print.failed", name = name, reason = reason),
    }
    .to_string()
}

/// The strip at the window's foot while a job is under way or has just ended; nothing otherwise.
pub(crate) fn print_strip(printing: Printing) -> impl IntoElement {
    when(
        move || printing.status.with(Option::is_some),
        move || strip(printing),
    )
}

fn strip(printing: Printing) -> impl IntoElement {
    let status = move || printing.status.get();
    let busy = move || status().is_some_and(|status| status.is_busy());
    let failed = move || matches!(status(), Some(Status::Failed(_)));
    let line = move || {
        status()
            .map(|status| message(&printing.name.get(), &status))
            .unwrap_or_default()
    };
    let fraction = move || match status() {
        Some(Status::Printing { done, total }) if total > 0 => done as f32 / total as f32,
        _ => 0.0,
    };
    view()
        .label(t!("print.strip"))
        .h(px(36.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .bg(move || theme().sidebar)
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(
            text(line)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || {
                    if failed() {
                        theme().destructive
                    } else {
                        theme().sidebar_foreground
                    }
                }),
        )
        .child(when(busy, move || {
            view()
                .w(px(BAR_WIDTH))
                .h(px(4.0))
                .rounded_full()
                .bg(move || theme().muted)
                .child(
                    view()
                        .h_full()
                        .rounded_full()
                        .bg(move || theme().primary)
                        .w(move || px(BAR_WIDTH * fraction())),
                )
        }))
        .child(dynamic(move || {
            if busy() {
                strip_button("print.cancel", move || printing.cancel()).into_element()
            } else {
                strip_button("print.dismiss", move || printing.dismiss()).into_element()
            }
        }))
}

/// A text button of the strip.
pub(crate) fn strip_button(label: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(t!(label))
        .on_press(press)
        .h(px(26.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(move || t!(label))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().sidebar_foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_dialog_and_the_job_are_busy() {
        assert!(Status::Asking.is_busy());
        assert!(Status::Printing { done: 0, total: 3 }.is_busy());
        assert!(!Status::Printed(3).is_busy());
        assert!(!Status::Cancelled.is_busy());
        assert!(!Status::Failed(String::new()).is_busy());
    }
}
