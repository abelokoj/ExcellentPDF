//! What the canvas reads per page for the pages on and near the screen: links (plan R.7) and text (K.13). Read on
//! the render workers, kept while their page stays near.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use ecclesia_core::render::Job;

/// Pages this far from the requested ones keep what was read, so scrolling back doesn't read it again.
const REACH: usize = 8;

/// One page's value: being read as a `J`, or read and kept as a `T`.
enum Entry<J, T> {
    Loading(Job<J>),
    Ready(T),
}

/// One kind of per-page value for the pages near the screen: the workers read a `J`, kept as a `T` made from it.
pub(crate) struct Near<J, T = J> {
    /// What it is, for the log: "links", "text".
    what: &'static str,
    pages: HashMap<usize, Entry<J, T>>,
}

impl<J, T> std::fmt::Debug for Near<J, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Near")
            .field("what", &self.what)
            .field("pages", &self.pages.len())
            .finish()
    }
}

impl<J, T: Default + From<J>> Near<J, T> {
    /// Nothing read yet.
    pub(crate) fn new(what: &'static str) -> Self {
        Self {
            what,
            pages: HashMap::new(),
        }
    }

    /// Starts reading pages `pages` with `load` (it starts a job) and forgets those far from them, cancelling their
    /// reads.
    pub(crate) fn request(&mut self, pages: RangeInclusive<usize>, load: impl Fn(usize) -> Job<J>) {
        let (first, last) = (*pages.start(), *pages.end());
        self.pages.retain(|page, entry| {
            let near = *page + REACH >= first && *page <= last + REACH;
            if !near && let Entry::Loading(job) = entry {
                job.cancel();
            }
            near
        });
        for page in pages {
            self.pages
                .entry(page)
                .or_insert_with(|| Entry::Loading(load(page)));
        }
    }

    /// Takes the reads that finished; whether any did.
    pub(crate) fn take_done(&mut self) -> bool {
        let mut done = false;
        for (page, entry) in &mut self.pages {
            let Entry::Loading(job) = entry else {
                continue;
            };
            let value = match job.try_take() {
                None => continue,
                Some(Ok(value)) => T::from(value),
                Some(Err(err)) => {
                    // A page whose links or text can't be read has none; its content still shows.
                    if !err.is_aborted() {
                        log::warn!("cannot read the {} of page {page}: {err}", self.what);
                    }
                    T::default()
                }
            };
            *entry = Entry::Ready(value);
            done = true;
        }
        done
    }

    /// Page `page`'s value, once it is read.
    pub(crate) fn get(&self, page: usize) -> Option<&T> {
        match self.pages.get(&page)? {
            Entry::Ready(value) => Some(value),
            Entry::Loading(_) => None,
        }
    }

    /// Forgets every page's value, cancelling the reads.
    pub(crate) fn clear(&mut self) {
        for entry in self.pages.values() {
            if let Entry::Loading(job) = entry {
                job.cancel();
            }
        }
        self.pages.clear();
    }
}
