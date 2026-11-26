//! Render workers (plan K.4, §5.3, §7.4): a pool of threads that build display lists, render tiles and extract text,
//! highest priority first. Every job carries a [`Cookie`], so it can be cancelled before or while it runs.
//!
//! Each worker uses its own MuPDF context, cloned on first use by `excellent-mupdf`. A [`Document`] is not thread-safe,
//! so jobs that load pages share it behind a `Mutex`; display lists are immutable and are rendered by several workers
//! at once.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};

use excellent_mupdf::{
    AnnotKind, Background, Cookie, DeviceRect, DisplayList, Document, Link, Matrix, PageRect,
    Pixmap, Result, TextLayout,
};

use crate::annot::{self, Mark};
use crate::darkmode::{self, PageColors, PageImages, Pictures};

/// An annotation with text on a page, as a screen reader reads it (U.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// What kind of annotation it is.
    pub kind: AnnotKind,
    /// Where it is, in page points.
    pub rect: PageRect,
    /// Its text (`/Contents`).
    pub contents: String,
}

/// What smart dark mode needs to recolour a page's tiles (R.14).
#[derive(Debug, Clone)]
pub struct Recolor {
    /// The colours to draw in.
    pub colors: PageColors,
    /// The page's pictures, shared by its tiles.
    pub images: Arc<PageImages>,
}

impl Recolor {
    /// Recolours `tile`, which covers `area` of the page `list` draws through `ctm`, keeping the page's picture
    /// region as it is when the colours say so.
    fn apply(
        &self,
        list: &DisplayList,
        ctm: &Matrix,
        area: DeviceRect,
        tile: &mut Pixmap,
        cookie: &Cookie,
    ) -> Result<()> {
        let keep = match self.colors.pictures {
            Pictures::Keep => {
                let scale = (ctm.a * ctm.d - ctm.b * ctm.c).abs().sqrt();
                self.images
                    .kept_region(list, scale, cookie)?
                    .and_then(|region| in_tile(ctm.apply_rect(&region), area))
            }
            Pictures::Recolor => None,
        };
        let (width, height) = (tile.width() as usize, tile.height() as usize);
        let (stride, components) = (tile.stride(), tile.components());
        darkmode::recolor(
            tile.samples_mut(),
            width,
            height,
            stride,
            components,
            &self.colors,
            keep,
        );
        Ok(())
    }
}

/// `region` (device pixels) in the tile covering `area`, in the tile's own pixels, grown by 3 pixels so the
/// picture's anti-aliased edge stays too, as Enhanced does. `None` when it misses the tile.
fn in_tile(region: DeviceRect, area: DeviceRect) -> Option<DeviceRect> {
    let local = DeviceRect {
        x0: (region.x0 - area.x0 - 3).max(0),
        y0: (region.y0 - area.y0 - 3).max(0),
        x1: (region.x1 - area.x0 + 3).min(area.x1 - area.x0),
        y1: (region.y1 - area.y0 + 3).min(area.y1 - area.y0),
    };
    (!local.is_empty()).then_some(local)
}

/// What a job is for (plan §7.4, item 5). Workers take the highest-priority job waiting, and jobs of equal priority in
/// the order they were submitted. Declared lowest first, so `Visible > Bulk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// Bulk jobs, such as export and print.
    Bulk,
    /// Background work: indexing, OCR, embeddings.
    Background,
    /// Thumbnails on screen.
    Thumbnail,
    /// The next or previous screen.
    Nearby,
    /// The current page's text.
    PageText,
    /// Tiles on screen.
    Visible,
}

/// A submitted job. Dropping it cancels it, since no one can receive its result.
#[derive(Debug)]
pub struct Job<T> {
    cookie: Arc<Cookie>,
    result: Receiver<Result<T>>,
    done: Arc<Done>,
}

impl<T> Job<T> {
    /// Cancels the job: it won't start, or stops soon if it has. Its result is then an error for which
    /// `MuError::is_aborted` is true.
    pub fn cancel(&self) {
        self.cookie.abort();
    }

    /// The result, if the job is done; `None` while it runs. The UI thread polls with this after
    /// [`Job::when_done`] wakes it. Once it has returned the result, drop the job.
    ///
    /// # Panics
    ///
    /// The worker running the job panicked, or the result was already taken.
    pub fn try_take(&mut self) -> Option<Result<T>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                panic!("a render worker panicked while running this job, or its result was taken")
            }
        }
    }

    /// Calls `notify` on the worker's thread once the job is done (finished, failed or cancelled), or right away if
    /// it already is. A later call replaces an earlier one that hasn't run. `notify` should only wake the UI thread
    /// (post it a message); the result is then read there with [`Job::try_take`].
    pub fn when_done(&self, notify: impl FnOnce() + Send + 'static) {
        let mut state = self.done.lock();
        if state.finished {
            drop(state);
            notify();
        } else {
            state.notify = Some(Box::new(notify));
        }
    }

    /// Blocks until the job is done. Never call this on the UI thread (plan §5.3).
    ///
    /// # Errors
    ///
    /// The job failed or was cancelled.
    ///
    /// # Panics
    ///
    /// The worker running the job panicked.
    pub fn wait(self) -> Result<T> {
        self.result
            .recv()
            .expect("a render worker panicked while running this job")
    }
}

/// Awaiting a job (in an app task) gives its result: the worker wakes the task when the job is done.
impl<T> Future for Job<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let job = self.get_mut();
        if let Some(result) = job.try_take() {
            return Poll::Ready(result);
        }
        let waker = cx.waker().clone();
        job.when_done(move || waker.wake());
        // It may have finished between the two looks: take it now rather than wait for the wake.
        job.try_take().map_or(Poll::Pending, Poll::Ready)
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cookie.abort();
    }
}

/// The render worker pool.
#[derive(Debug)]
pub struct RenderPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Default for RenderPool {
    /// Bounded worker pool leaving room for the UI thread and avoiding excessive per-thread MuPDF
    /// context overhead on high-core-count machines (plan §5.3).
    fn default() -> Self {
        let cores = thread::available_parallelism().map_or(2, usize::from);
        Self::new(cores.saturating_sub(1).clamp(2, 6))
    }
}

impl RenderPool {
    /// A pool of `workers` threads.
    ///
    /// # Panics
    ///
    /// `workers` is zero, or the OS can't start a thread.
    pub fn new(workers: usize) -> Self {
        assert!(workers > 0, "a render pool needs at least one worker");
        let shared = Arc::new(Shared::default());
        let workers = (0..workers)
            .map(|index| {
                let shared = Arc::clone(&shared);
                thread::Builder::new()
                    .name(format!("render-{index}"))
                    .spawn(move || shared.work())
                    .expect("the OS starts a render worker thread")
            })
            .collect();
        Self { shared, workers }
    }

    /// Records page `page` of `document` into a display list, without the annotations whose ids are in `hidden`
    /// (A.19).
    pub fn build_display_list(
        &self,
        document: &Arc<Mutex<Document>>,
        page: usize,
        hidden: Arc<[String]>,
        priority: Priority,
    ) -> Job<DisplayList> {
        let document = Arc::clone(document);
        self.submit(priority, move |cookie| {
            let document = document.lock().expect("document lock poisoned");
            document
                .load_page(page)?
                .display_list_without(&hidden, Some(cookie))
        })
    }

    /// Renders `area` of `list` as `ctm` maps it onto the device, on white: one tile. With `recolor`, the tile is
    /// then drawn in the theme's colours (smart dark mode, R.14).
    pub fn render_tile(
        &self,
        list: &Arc<DisplayList>,
        ctm: Matrix,
        area: DeviceRect,
        priority: Priority,
        recolor: Option<Recolor>,
    ) -> Job<Pixmap> {
        let list = Arc::clone(list);
        self.submit(priority, move |cookie| {
            let mut tile = list.render_area(&ctm, area, Background::White, Some(cookie))?;
            if let Some(recolor) = recolor {
                recolor.apply(&list, &ctm, area, &mut tile, cookie)?;
            }
            Ok(tile)
        })
    }

    /// Reads the links of page `page` of `document` (R.7).
    pub fn load_links(
        &self,
        document: &Arc<Mutex<Document>>,
        page: usize,
        priority: Priority,
    ) -> Job<Vec<Link>> {
        let document = Arc::clone(document);
        self.submit(priority, move |_| {
            let document = document.lock().expect("document lock poisoned");
            document.load_page(page)?.links()
        })
    }

    /// Reads the annotation marks (R.17) of every page in `pages` of `document`, a list per page in order. A
    /// page that can't be read has none (logged), so one bad page doesn't hide the rest.
    pub fn load_marks(
        &self,
        document: &Arc<Mutex<Document>>,
        pages: std::ops::Range<usize>,
        priority: Priority,
    ) -> Job<Vec<Vec<Mark>>> {
        let document = Arc::clone(document);
        self.submit(priority, move |cookie| {
            let mut marks = Vec::with_capacity(pages.len());
            for page in pages {
                cookie.check()?;
                let document = document.lock().expect("document lock poisoned");
                marks.push(annot::page_marks(&document, page).unwrap_or_else(|err| {
                    log::warn!("cannot read the annotations of page {page}: {err}");
                    Vec::new()
                }));
            }
            Ok(marks)
        })
    }

    /// Reads the notes of page `page` of `document`: its annotations that carry text, for screen readers (U.2).
    /// Links (R.7 reads them) and pop-ups (the window of another annotation) are left out.
    pub fn load_notes(
        &self,
        document: &Arc<Mutex<Document>>,
        page: usize,
        priority: Priority,
    ) -> Job<Vec<Note>> {
        let document = Arc::clone(document);
        self.submit(priority, move |_| {
            let document = document.lock().expect("document lock poisoned");
            let page = document.load_page(page)?;
            let mut notes = Vec::new();
            for annot in page.annots()? {
                let kind = annot.kind()?;
                if matches!(kind, AnnotKind::Link | AnnotKind::Popup) {
                    continue;
                }
                let contents = annot.contents()?;
                if !contents.trim().is_empty() {
                    notes.push(Note {
                        kind,
                        rect: annot.rect()?,
                        contents,
                    });
                }
            }
            Ok(notes)
        })
    }

    /// Extracts the plain text of page `page` of `document`.
    pub fn extract_text(
        &self,
        document: &Arc<Mutex<Document>>,
        page: usize,
        priority: Priority,
    ) -> Job<String> {
        let document = Arc::clone(document);
        self.submit(priority, move |cookie| {
            let document = document.lock().expect("document lock poisoned");
            document.load_page(page)?.text(Some(cookie))?.text()
        })
    }

    /// Reads the text of page `page` of `document` with its layout, for the text layer (K.13) and find (F.1).
    pub fn read_text(
        &self,
        document: &Arc<Mutex<Document>>,
        page: usize,
        priority: Priority,
    ) -> Job<TextLayout> {
        let document = Arc::clone(document);
        self.submit(priority, move |cookie| {
            let document = document.lock().expect("document lock poisoned");
            Ok(document.load_page(page)?.text(Some(cookie))?.layout())
        })
    }

    pub(crate) fn submit<T: Send + 'static>(
        &self,
        priority: Priority,
        work: impl FnOnce(&Cookie) -> Result<T> + Send + 'static,
    ) -> Job<T> {
        let cookie = Arc::new(Cookie::new());
        let (sender, result) = mpsc::sync_channel(1);
        let done = Arc::new(Done::default());
        let finished = Arc::clone(&done);
        let run = Box::new(move |cookie: &Cookie| {
            let outcome = cookie.check().and_then(|()| work(cookie));
            // Fails only if the job was dropped, and then no one wants the result.
            let _ = sender.send(outcome);
            finished.finish();
        });
        {
            let mut queue = self.shared.lock();
            let order = queue.submitted;
            queue.submitted += 1;
            queue.waiting.push(Queued {
                priority,
                order,
                cookie: Arc::clone(&cookie),
                run,
            });
        }
        self.shared.wake.notify_one();
        Job {
            cookie,
            result,
            done,
        }
    }
}

/// Whether a job is done, and who to tell when it is.
#[derive(Default)]
struct Done {
    state: Mutex<DoneState>,
}

#[derive(Default)]
struct DoneState {
    finished: bool,
    notify: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for Done {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Done")
            .field("finished", &self.lock().finished)
            .finish_non_exhaustive()
    }
}

impl Done {
    fn lock(&self) -> std::sync::MutexGuard<'_, DoneState> {
        self.state.lock().expect("job state lock poisoned")
    }

    /// The result was sent: run the notification, outside the lock.
    fn finish(&self) {
        let notify = {
            let mut state = self.lock();
            state.finished = true;
            state.notify.take()
        };
        if let Some(notify) = notify {
            notify();
        }
    }
}

impl Drop for RenderPool {
    /// Cancels the jobs still waiting (their results are errors), lets running ones finish, and joins the workers.
    fn drop(&mut self) {
        {
            let mut queue = self.shared.lock();
            queue.closing = true;
            for job in &queue.waiting {
                job.cookie.abort();
            }
        }
        self.shared.wake.notify_all();
        for worker in self.workers.drain(..) {
            // A worker that panicked has already reported it; there is nothing more to do here.
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Default)]
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().expect("render queue lock poisoned")
    }

    /// A worker's loop: run the best waiting job; sleep when there is none; stop when the pool closes and the queue
    /// is empty. Cancelled jobs still run, to report that they were cancelled, which takes no time.
    fn work(&self) {
        loop {
            let job = {
                let mut queue = self.lock();
                loop {
                    if let Some(job) = queue.waiting.pop() {
                        break job;
                    }
                    if queue.closing {
                        return;
                    }
                    queue = self.wake.wait(queue).expect("render queue lock poisoned");
                }
            };
            (job.run)(&job.cookie);
        }
    }
}

#[derive(Debug, Default)]
struct Queue {
    waiting: BinaryHeap<Queued>,
    submitted: u64,
    closing: bool,
}

/// A job waiting for a worker, ordered by priority, then first submitted first.
struct Queued {
    priority: Priority,
    order: u64,
    cookie: Arc<Cookie>,
    run: Box<dyn FnOnce(&Cookie) + Send>,
}

impl std::fmt::Debug for Queued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queued")
            .field("priority", &self.priority)
            .field("order", &self.order)
            .finish_non_exhaustive()
    }
}

// Equality and order compare `priority` and `order` only; `order` is unique, so no two queued jobs are equal.
impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Queued {}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Queued {
    /// `BinaryHeap` pops the greatest: the highest priority, then the lowest (earliest) order.
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.order.cmp(&self.order))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_priority_first_and_cancelled_jobs_never_run() {
        let pool = RenderPool::new(1);
        // Hold the only worker until every job is queued.
        let (open_gate, gate) = mpsc::channel::<()>();
        let blocker = pool.submit(Priority::Visible, move |_| {
            gate.recv().expect("the test opens the gate");
            Ok(())
        });

        let ran = Arc::new(Mutex::new(Vec::new()));
        let job = |name: &'static str| {
            let ran = Arc::clone(&ran);
            move |_: &Cookie| {
                ran.lock().expect("log lock").push(name);
                Ok(name)
            }
        };
        let bulk = pool.submit(Priority::Bulk, job("bulk"));
        let first_thumbnail = pool.submit(Priority::Thumbnail, job("thumbnail 1"));
        let second_thumbnail = pool.submit(Priority::Thumbnail, job("thumbnail 2"));
        let visible = pool.submit(Priority::Visible, job("visible"));
        let cancelled = pool.submit(Priority::Visible, job("cancelled"));
        cancelled.cancel();
        open_gate.send(()).expect("the blocker is waiting");

        blocker.wait().expect("blocker");
        assert!(cancelled.wait().expect_err("cancelled").is_aborted());
        for job in [bulk, first_thumbnail, second_thumbnail, visible] {
            job.wait().expect("ran");
        }
        assert_eq!(
            *ran.lock().expect("log lock"),
            ["visible", "thumbnail 1", "thumbnail 2", "bulk"]
        );
    }

    #[test]
    fn a_finished_job_notifies_and_its_result_can_be_taken_without_blocking() {
        let pool = RenderPool::new(1);
        let (open_gate, gate) = mpsc::channel::<()>();
        let mut job = pool.submit(Priority::Visible, move |_| {
            gate.recv().expect("the test opens the gate");
            Ok(7)
        });
        assert!(job.try_take().is_none(), "still running");
        let (notified, woken) = mpsc::channel();
        job.when_done(move || notified.send(()).expect("the test listens"));
        open_gate.send(()).expect("the job is waiting");
        woken.recv().expect("the job notifies when done");
        assert_eq!(job.try_take().expect("done").expect("ran"), 7);

        // Asked after it finished: notified at once.
        let (notified, woken) = mpsc::channel();
        job.when_done(move || notified.send(()).expect("the test listens"));
        woken.try_recv().expect("notified right away");
    }

    #[test]
    fn default_pool_has_a_worker() {
        assert!(!RenderPool::default().workers.is_empty());
    }
}
