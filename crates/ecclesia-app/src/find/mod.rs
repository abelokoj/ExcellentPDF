//! Find in document (plan F.1): each pane's find bar and the hits its canvas reports. The search itself runs in the
//! page canvas (`ecclesia-canvas-win`'s `canvas::find`, on the render workers), which draws the hits and sends them
//! here with the `found` event; this keeps the query, the options, the count and the result list, and sends the
//! canvas `find`, `findNext`, `findPrevious`, `findShow` and `findClear`.

mod view;

use ecclesia_features::search::find::{Case, Marks, Options, Words};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

pub(crate) use view::{find_bar, results_panel};

/// A find command (F.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindAction {
    /// Opens the find bar, or takes the keyboard back to it (Ctrl+F).
    Open,
    /// The next hit (F3); opens the bar while there is nothing to find.
    Next,
    /// The previous hit (Shift+F3).
    Previous,
    /// Closes the bar and takes the hits off the pages (Escape in the bar).
    Close,
}

/// One option of the bar's three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toggle {
    Case,
    Words,
    Marks,
}

impl Toggle {
    pub(crate) const ALL: [Self; 3] = [Self::Case, Self::Words, Self::Marks];

    /// Whether it is on in `options`.
    pub(crate) fn is_on(self, options: Options) -> bool {
        match self {
            Self::Case => options.case == Case::Match,
            Self::Words => options.words == Words::Whole,
            Self::Marks => options.marks == Marks::Match,
        }
    }

    /// `options` with it turned on or off.
    pub(crate) fn set(self, options: Options, on: bool) -> Options {
        match self {
            Self::Case => Options {
                case: if on { Case::Match } else { Case::Ignore },
                ..options
            },
            Self::Words => Options {
                words: if on { Words::Whole } else { Words::Anywhere },
                ..options
            },
            Self::Marks => Options {
                marks: if on { Marks::Match } else { Marks::Ignore },
                ..options
            },
        }
    }
}

/// A hit as the result list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) page: usize,
    /// Its place among its page's hits.
    pub(crate) nth: usize,
    pub(crate) before: String,
    pub(crate) hit: String,
    pub(crate) after: String,
}

impl Hit {
    fn from_json(value: &JsonValue) -> Option<Self> {
        let index = |key: &str| usize::try_from(value.get(key)?.as_u64()?).ok();
        let text = |key: &str| {
            value
                .get(key)
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Some(Self {
            page: index("page")?,
            nth: index("nth")?,
            before: text("before"),
            hit: text("hit"),
            after: text("after"),
        })
    }

    fn key(&self) -> (usize, usize) {
        (self.page, self.nth)
    }
}

/// Where a search is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    /// Pages read so far, and in all.
    pub(crate) searched: usize,
    pub(crate) pages: usize,
    /// Hits found so far.
    pub(crate) total: usize,
    /// The reader's hit, in document order from 0.
    pub(crate) current: Option<usize>,
}

impl Progress {
    /// Whether every page has been read.
    pub(crate) fn is_done(self) -> bool {
        self.searched >= self.pages
    }
}

/// What the bar says about the search: "3 of 120", "No results", or nothing while the query is empty.
pub(crate) fn status(query: &str, progress: Progress) -> String {
    if query.trim().is_empty() {
        String::new()
    } else if progress.total == 0 && progress.is_done() {
        t!("find.none")
    } else if progress.total == 0 {
        t!("find.searching")
    } else {
        let total = if progress.is_done() {
            progress.total.to_string()
        } else {
            format!("{}+", progress.total)
        };
        match progress.current {
            Some(current) => t!("find.count", n = current + 1, total = total),
            None => t!("find.total", total = total),
        }
    }
}

/// Adds `added` to `list`, kept in document order.
fn merge(list: &mut Vec<Hit>, added: Vec<Hit>) {
    for hit in added {
        let at = list.partition_point(|other| other.key() < hit.key());
        if list.get(at).is_none_or(|other| other.key() != hit.key()) {
            list.insert(at, hit);
        }
    }
}

/// A pane's find bar and hits.
#[derive(Clone, Copy)]
pub(crate) struct FindState {
    open: Signal<bool>,
    query: Signal<String>,
    options: Signal<Options>,
    /// The number of the search the canvas runs, so the hits of one replaced while it ran are ignored.
    id: Stored<u64>,
    progress: Signal<Progress>,
    hits: Signal<Vec<Hit>>,
    /// The bar's text field.
    input: ElementRef,
    canvas: ElementRef,
    /// The view that holds the keyboard for the pages.
    pages_focus: ElementRef,
}

impl FindState {
    /// The find state of the pane with `canvas`, whose pages take the keyboard through `pages_focus`.
    pub(crate) fn new(canvas: ElementRef, pages_focus: ElementRef) -> Self {
        Self {
            open: signal(false),
            query: signal(String::new()),
            options: signal(Options::default()),
            id: stored(0),
            progress: signal(Progress::default()),
            hits: signal(Vec::new()),
            input: element_ref(),
            canvas,
            pages_focus,
        }
    }

    pub(crate) fn is_open(self) -> bool {
        self.open.get()
    }

    pub(crate) fn query(self) -> String {
        self.query.get()
    }

    pub(crate) fn options(self) -> Options {
        self.options.get()
    }

    pub(crate) fn progress(self) -> Progress {
        self.progress.get()
    }

    /// The hits found so far, in document order.
    pub(crate) fn hits(self) -> Vec<Hit> {
        self.hits.get()
    }

    pub(crate) fn input(self) -> ElementRef {
        self.input
    }

    /// Runs `action`.
    pub(crate) fn run(self, action: FindAction) {
        match action {
            FindAction::Open => self.open_bar(),
            FindAction::Next | FindAction::Previous => {
                if !self.open.get_untracked() || self.query.with_untracked(|q| q.trim().is_empty())
                {
                    self.open_bar();
                } else if action == FindAction::Next {
                    self.canvas.command("findNext", JsonValue::Null);
                } else {
                    self.canvas.command("findPrevious", JsonValue::Null);
                }
            }
            FindAction::Close => self.close(),
        }
    }

    fn open_bar(self) {
        let was_open = self.open.get_untracked();
        self.open.set(true);
        // Reopened with its text kept: find it again on the page shown now.
        if !was_open && self.query.with_untracked(|q| !q.trim().is_empty()) {
            self.search();
        }
        let input = self.input;
        spawn(async move {
            // The bar mounts with this change; its field takes the keyboard once it is there.
            next_frame().await;
            input.focus();
        });
    }

    fn close(self) {
        self.open.set(false);
        self.canvas.command("findClear", JsonValue::Null);
        self.hits.set(Vec::new());
        self.progress.set(Progress::default());
        self.pages_focus.focus();
    }

    /// The reader typed `text`: it is found at once.
    pub(crate) fn set_query(self, text: &str) {
        self.query.set(text.to_owned());
        self.search();
    }

    /// The reader turned `toggle` on or off.
    pub(crate) fn set_toggle(self, toggle: Toggle, on: bool) {
        self.options
            .update(|options| *options = toggle.set(*options, on));
        self.search();
    }

    /// Starts the search again with the query and options as they are.
    fn search(self) {
        let id = self.id.with(|id| {
            *id += 1;
            *id
        });
        self.hits.set(Vec::new());
        self.progress.set(Progress::default());
        let options = self.options.get_untracked();
        let text = self.query.get_untracked();
        self.canvas.command(
            "find",
            serde_json::json!({
                "id": id,
                "text": text,
                "matchCase": options.case == Case::Match,
                "wholeWord": options.words == Words::Whole,
                "matchMarks": options.marks == Marks::Match,
            }),
        );
    }

    /// Goes to `hit`, from the result list.
    pub(crate) fn show(self, hit: &Hit) {
        self.canvas.command(
            "findShow",
            serde_json::json!({ "page": hit.page, "nth": hit.nth }),
        );
    }

    /// Takes the canvas's `found` events, and searches a newly shown document while the bar is open.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        match &*event.name {
            "found" => self.found(&event.data),
            "pages" => {
                let count = event.data.get("count").and_then(JsonValue::as_u64);
                if count.is_some_and(|count| count > 0)
                    && self.open.get_untracked()
                    && self.query.with_untracked(|q| !q.trim().is_empty())
                {
                    self.search();
                } else {
                    self.hits.set(Vec::new());
                    self.progress.set(Progress::default());
                }
            }
            _ => {}
        }
    }

    fn found(self, data: &JsonValue) {
        let id = data.get("id").and_then(JsonValue::as_u64);
        if id != Some(self.id.get()) {
            return;
        }
        let count = |key: &str| {
            data.get(key)
                .and_then(JsonValue::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        let progress = Progress {
            searched: count("searched"),
            pages: count("pages"),
            total: count("total"),
            current: data
                .get("current")
                .and_then(JsonValue::as_u64)
                .and_then(|n| usize::try_from(n).ok()),
        };
        let added: Vec<Hit> = data
            .get("hits")
            .and_then(JsonValue::as_array)
            .map(|hits| hits.iter().filter_map(Hit::from_json).collect())
            .unwrap_or_default();
        batch(move || {
            // A search the canvas ran again (a book laid out again) starts over.
            if progress.searched == 0 {
                self.hits.set(Vec::new());
            }
            if !added.is_empty() {
                self.hits.update(|hits| merge(hits, added));
            }
            self.progress.set(progress);
        });
    }

    /// The hit the reader is on, in the result list.
    pub(crate) fn current_key(self) -> Option<(usize, usize)> {
        let current = self.progress.get().current?;
        self.hits.with(|hits| hits.get(current).map(Hit::key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(page: usize, nth: usize) -> Hit {
        Hit {
            page,
            nth,
            before: String::new(),
            hit: "x".to_owned(),
            after: String::new(),
        }
    }

    #[test]
    fn hits_arriving_out_of_order_are_kept_in_document_order() {
        let mut list = vec![hit(4, 0), hit(4, 1)];
        merge(&mut list, vec![hit(9, 0), hit(1, 0), hit(4, 1)]);
        let keys: Vec<_> = list.iter().map(Hit::key).collect();
        assert_eq!(keys, [(1, 0), (4, 0), (4, 1), (9, 0)]);
    }

    #[test]
    fn the_toggles_turn_their_option_on_and_off() {
        let options = Options::default();
        for toggle in Toggle::ALL {
            assert!(!toggle.is_on(options));
            let on = toggle.set(options, true);
            assert!(toggle.is_on(on));
            assert!(Toggle::ALL.iter().filter(|t| t.is_on(on)).count() == 1);
            assert_eq!(toggle.set(on, false), options);
        }
    }

    #[test]
    fn a_search_is_done_once_every_page_is_read() {
        let mut progress = Progress {
            searched: 3,
            pages: 10,
            total: 0,
            current: None,
        };
        assert!(!progress.is_done());
        progress.searched = 10;
        assert!(progress.is_done());
    }
}
