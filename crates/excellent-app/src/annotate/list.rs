//! The annotation list (A.14): a panel with every annotation of the shown document, one row each (its kind's mark in
//! its colour, its note or the text it marks, and its kind and category), under a heading per page. Kind, colour,
//! category (A.17), tag, author and date (A.19) chips and a search field filter it as the reader types; "Show only
//! matching" leaves the others out of the pages too. Each file's filter is kept in `library.db` and comes back with
//! it. A row's press shows its page and selects the annotation (A.13). The annotations are read on the render
//! workers, again after every edit; the model is `excellent-features`' `annotate::list`.

use std::path::{Path, PathBuf};

use excellent_core::annot::load_all;
use excellent_core::shared::pool;
use excellent_features::annotate::colors::hex;
use excellent_features::annotate::list::{self, Entry, Filter, Kind, Row, Since};
use excellent_store::annotation_filters;
use nsh_base::{button, input, theme};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use super::AnnotateState;
use super::inspector::category_chip;
use crate::bookmarks::{caption, field};
use crate::goto::PageState;
use crate::settings_window::{on_off, segment, segment_of, segmented};

/// What the list holds: the rows read, and the filter.
#[derive(Clone, Copy)]
pub(crate) struct Listing {
    /// The shown document's rows; `None` until they are read.
    rows: Signal<Option<Vec<Row>>>,
    filter: Signal<Filter>,
    /// The read under way: dropping it cancels it.
    reading: Stored<Option<Spawned<()>>>,
    /// The file the rows and the filter are of.
    path: Stored<Option<PathBuf>>,
    /// The filter as last kept for that file, so only a change is written.
    kept: Stored<Filter>,
}

impl Listing {
    pub(crate) fn new() -> Self {
        Self {
            rows: signal(None),
            filter: signal(Filter::default()),
            reading: stored(None),
            path: stored(None),
            kept: stored(Filter::default()),
        }
    }

    /// Brings back `path`'s filter when another file is shown; its rows are read again.
    fn show_file(self, path: &Path) {
        if self.path.with(|shown| shown.as_deref() == Some(path)) {
            return;
        }
        let filter = kept_filter(path);
        self.path.with(|shown| *shown = Some(path.to_path_buf()));
        self.kept.with(|kept| kept.clone_from(&filter));
        self.rows.set(None);
        self.filter.set(filter);
    }

    /// Keeps the filter with its file when it changed.
    fn keep(self) {
        let filter = self.filter.get_untracked();
        let changed = self.kept.with(|kept| {
            let changed = *kept != filter;
            kept.clone_from(&filter);
            changed
        });
        if let Some(path) = self.path.with(|path| path.clone()).filter(|_| changed) {
            keep_filter(path, &filter);
        }
    }
}

/// The filter kept for the file at `path`; the empty filter when none was, or the library can't be read.
fn kept_filter(path: &Path) -> Filter {
    let Some(store) = crate::library::store() else {
        return Filter::default();
    };
    match store.read(|conn| annotation_filters::load(conn, path)) {
        Ok(kept) => kept.map_or_else(Filter::default, |text| Filter::from_json(&text)),
        Err(err) => {
            log::warn!("annotation filter: {err}");
            Filter::default()
        }
    }
}

/// Keeps `filter` for the file at `path`, on the library's writer; the empty filter is forgotten.
fn keep_filter(path: PathBuf, filter: &Filter) {
    let Some(store) = crate::library::store() else {
        return;
    };
    let text = if *filter == Filter::default() {
        String::new()
    } else {
        filter.to_json()
    };
    let _ = store.write(move |tx| {
        let result = annotation_filters::save(tx, &path, &text);
        if let Err(err) = &result {
            log::warn!("annotation filter: {err}");
        }
        result
    });
}

impl AnnotateState {
    /// Reads the shown document's annotations on the render workers, into the list.
    fn read_list(self) {
        let listing = self.listing;
        let Some(document) = self.editing.document() else {
            listing.rows.set(Some(Vec::new()));
            return;
        };
        listing.show_file(document.path());
        let mupdf = match document.mupdf() {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::warn!("annotation list: cannot read the document: {err}");
                return;
            }
        };
        let job = load_all(pool(), mupdf);
        let task = spawn(async move {
            match job.await {
                Ok(annotations) => listing.rows.set(Some(list::rows(&annotations))),
                Err(err) if !err.is_aborted() => log::warn!("annotation list: {err}"),
                Err(_) => {}
            }
        });
        listing.reading.with(|reading| *reading = Some(task));
    }

    /// Shows `row`'s page and selects its annotation, its note open.
    fn choose_row(self, row: &Row, go: impl Fn(usize)) {
        go(row.target.page);
        self.selected.set(vec![row.target.clone()]);
        self.open_note(&row.target);
    }
}

/// The annotation list panel. `go` shows a page.
pub(crate) fn annotation_list(
    state: AnnotateState,
    pages: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let listing = state.listing;
    // Read when the panel shows, again when another document is shown, and after every edit.
    effect(move || {
        state.editing.revision();
        if pages.count() > 0 {
            untracked(|| state.read_list());
        }
    });
    // Each change of the filter is kept with the file.
    effect(move || {
        listing.filter.with(|_| ());
        untracked(|| listing.keep());
    });
    // "Show only matching" leaves the other annotations out of the pages; drawing them again reads the rows again,
    // which hides the same ones, so this settles.
    effect(move || {
        let hidden = listing.filter.with(|filter| {
            listing
                .rows
                .with(|rows| Some(filter.hidden(rows.as_deref()?)))
        });
        let Some(hidden) = hidden else {
            return;
        };
        untracked(|| {
            let document = state.editing.document();
            if document.is_some_and(|document| document.hide_annotations(hidden)) {
                state.editing.redraw();
            }
        });
    });
    let shown = computed(move || {
        let filter = listing.filter.get();
        listing.rows.with(|rows| {
            let rows = rows.as_deref().unwrap_or_default();
            list::entries(rows, &filter.apply(rows))
        })
    });
    let has_rows = move || {
        listing
            .rows
            .with(|rows| rows.as_ref().is_some_and(|r| !r.is_empty()))
    };
    let read = move || listing.rows.with(Option::is_some);
    view()
        .flex_1()
        .child(when(has_rows, move || filters(listing)))
        .child(
            when(
                move || !shown.with(Vec::is_empty),
                move || {
                    virtual_list(
                        move || shown.with(Vec::len),
                        move |at: Position| entry(state, listing, pages, shown, at, go),
                    )
                    .label(t!("annotations.title"))
                    .pb_2()
                    .flex_1()
                },
            )
            .otherwise(move || {
                let hint = move || {
                    if !read() {
                        t!("annotations.reading")
                    } else if has_rows() {
                        t!("annotations.none_match")
                    } else {
                        t!("annotations.empty")
                    }
                };
                text(hint)
                    .p_3()
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground)
            }),
        )
}

/// The search field, then the kinds, colours, categories, tags and authors the document has, as chips that filter
/// the list, how recently they changed, "Show only matching", and a way to clear them all.
fn filters(listing: Listing) -> impl IntoElement {
    let offered = computed(move || {
        listing
            .rows
            .with(|rows| list::offered(rows.as_deref().unwrap_or_default()))
    });
    let search = field(input().value(move || listing.filter.with(|f| f.query.clone())))
        .on_change(move |text: &str| listing.filter.update(|f| text.clone_into(&mut f.query)))
        .placeholder(t!("annotations.search"))
        .label(t!("annotations.search"));
    let kinds = dynamic(move || {
        offered
            .with(|(kinds, _)| kinds.clone())
            .into_iter()
            .fold(segmented(), |group, kind| {
                group.child(segment_of(
                    move || t!(&format!("annotations.kind.{}", kind.name())),
                    move || listing.filter.with(|f| f.kinds.contains(&kind)),
                    move || listing.filter.update(|f| *f = f.toggled_kind(kind)),
                ))
            })
    });
    let colors = dynamic(move || {
        let colors = offered.with(|(_, colors)| colors.clone());
        colors
            .into_iter()
            .fold(view().flex_row().flex_wrap().gap(px(6.0)), |row, color| {
                row.child(color_chip(listing, color))
            })
    });
    let offered_categories = computed(move || {
        listing
            .rows
            .with(|rows| list::categories(rows.as_deref().unwrap_or_default()))
    });
    let categories = move || {
        dynamic(move || {
            offered_categories
                .get()
                .into_iter()
                .fold(segmented(), |group, category| {
                    group.child(category_chip(
                        category,
                        move || listing.filter.with(|f| f.categories.contains(&category)),
                        move || {
                            listing.filter.update(|f| *f = f.toggled_category(category));
                        },
                    ))
                })
        })
    };
    let offered_tags = computed(move || {
        listing
            .rows
            .with(|rows| list::tags(rows.as_deref().unwrap_or_default()))
    });
    let offered_authors = computed(move || {
        listing
            .rows
            .with(|rows| list::authors(rows.as_deref().unwrap_or_default()))
    });
    let tags = move || {
        view()
            .gap_1()
            .child(caption("tags.title"))
            .child(name_chips(
                offered_tags,
                move |tag| {
                    listing
                        .filter
                        .with(|f| f.tags.contains(&tag.to_lowercase()))
                },
                move |tag| listing.filter.update(|f| *f = f.toggled_tag(tag)),
            ))
    };
    let authors = move || {
        view()
            .gap_1()
            .child(caption("annotations.authors"))
            .child(name_chips(
                offered_authors,
                move |author| {
                    listing
                        .filter
                        .with(|f| f.authors.iter().any(|a| a == author))
                },
                move |author| listing.filter.update(|f| *f = f.toggled_author(author)),
            ))
    };
    view()
        .px_3()
        .pt_2()
        .pb_2()
        .gap_2()
        .child(search)
        .child(kinds)
        .child(colors)
        .child(when(
            move || !offered_categories.with(Vec::is_empty),
            categories,
        ))
        .child(when(move || !offered_tags.with(Vec::is_empty), tags))
        .child(when(move || offered_authors.with(Vec::len) > 1, authors))
        .child(
            view()
                .gap_1()
                .child(caption("annotations.since"))
                .child(since_chips(listing)),
        )
        .child(show_only_row(listing))
        .child(when(
            move || listing.filter.with(|f| !f.is_empty()),
            move || clear_button(listing),
        ))
}

/// One chip per name of `names` (tags or authors): `chosen` says whether a name is shown, `choose` shows it or, when
/// it was, not.
fn name_chips(
    names: Memo<Vec<String>>,
    chosen: impl Fn(&str) -> bool + Copy + 'static,
    choose: impl Fn(&str) + Copy + 'static,
) -> impl IntoElement {
    dynamic(move || {
        (0..names.with(Vec::len)).fold(segmented(), |group, at| {
            let name = move || names.with(|names| names.get(at).cloned().unwrap_or_default());
            group.child(segment_of(
                name,
                move || chosen(&name()),
                move || choose(&name()),
            ))
        })
    })
}

/// Any time, or the annotations changed today, this week or this month.
fn since_chips(listing: Listing) -> impl IntoElement {
    let any = segment(
        "annotations.since.any",
        move || listing.filter.with(|f| f.since.is_none()),
        move || listing.filter.update(|f| f.since = None),
    );
    Since::ALL
        .into_iter()
        .fold(segmented().child(any), |group, since| {
            group.child(segment(
                since_key(since),
                move || listing.filter.with(|f| f.since == Some(since)),
                move || listing.filter.update(|f| f.since = Some(since)),
            ))
        })
}

/// `since`'s label key.
fn since_key(since: Since) -> &'static str {
    match since {
        Since::Day => "annotations.since.day",
        Since::Week => "annotations.since.week",
        Since::Month => "annotations.since.month",
    }
}

/// "Show only matching": the pages, too, leave out the annotations the list doesn't show.
fn show_only_row(listing: Listing) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(t!("annotations.show_only"))
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().sidebar_foreground),
        )
        .child(on_off(
            "annotations.show_only",
            move || listing.filter.with(|f| f.show_only),
            move |on| listing.filter.update(|f| f.show_only = on),
        ))
}

/// Shows every annotation again; "Show only matching" stays as it is.
fn clear_button(listing: Listing) -> impl IntoElement {
    button()
        .accessibility_label(t!("annotations.clear"))
        .on_press(move || {
            listing.filter.update(|f| {
                *f = Filter {
                    show_only: f.show_only,
                    ..Filter::default()
                };
            });
        })
        .h(px(28.0))
        .px_2()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(t!("annotations.clear"))
                .text_size(px(13.0))
                .text_color(move || theme().primary),
        )
}

/// A colour to show alone, or all again when it is the one shown; ringed when chosen.
fn color_chip(listing: Listing, color: u32) -> impl IntoElement {
    let chosen = move || listing.filter.with(|f| f.color == Some(color));
    let name = hex(color);
    button()
        .accessibility_label(move || {
            let key = if chosen() {
                "annotations.color_chosen"
            } else {
                "annotations.color"
            };
            Some(SharedString::from(t!(key, color = name.clone())))
        })
        .on_press(move || {
            listing.filter.update(|f| {
                f.color = if f.color == Some(color) {
                    None
                } else {
                    Some(color)
                }
            });
        })
        .size(px(24.0))
        .rounded_full()
        .border_2()
        .border_color(move || {
            if chosen() {
                theme().ring
            } else {
                theme().border
            }
        })
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            view()
                .m(px(3.0))
                .flex_1()
                .rounded_full()
                .bg(Hsla::from(rgb(color))),
        )
}

/// One line of the list: a page's heading, or an annotation's row.
fn entry(
    state: AnnotateState,
    listing: Listing,
    pages: PageState,
    shown: Memo<Vec<Entry>>,
    at: Position,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let line = move || shown.with(|entries| entries.get(at.index()).copied());
    dynamic(move || match line() {
        Some(Entry::Page { page, count }) => text(t!(
            "annotations.page",
            page = pages.label(page),
            count = count.to_string()
        ))
        .px_3()
        .pt_3()
        .pb_1()
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
        .into_element(),
        Some(Entry::Row(index)) => {
            match listing.rows.with(|rows| rows.as_ref()?.get(index).cloned()) {
                Some(row) => row_view(state, pages, row, go).into_element(),
                None => view().into_element(),
            }
        }
        None => view().into_element(),
    })
}

/// An annotation's row: a mark in its colour, its text (or its kind's name when it has none), and its kind.
fn row_view(
    state: AnnotateState,
    pages: PageState,
    row: Row,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let kind = match row.category {
        Some(category) => t!(
            "annotations.kind_category",
            kind = kind_label(row.kind),
            category = t!(&format!("category.{}", category.key()))
        ),
        None => kind_label(row.kind),
    };
    let shown_text = if row.text.is_empty() {
        kind.clone()
    } else {
        row.text.clone()
    };
    let spoken = t!(
        "annotations.row",
        kind = kind.clone(),
        text = row.text.clone(),
        page = pages.label(row.target.page)
    );
    let target = row.target.clone();
    let selected = move || state.selected.with(|s| s.contains(&target));
    let mark = view()
        .w(px(4.0))
        .rounded(px(2.0))
        .bg(move || match row.color {
            Some(color) => rgb(color).into(),
            None => theme().border,
        });
    let chosen = row.clone();
    button()
        .accessibility_label(spoken)
        .on_press(move || state.choose_row(&chosen, go))
        .flex_row()
        .gap_2()
        .mx_2()
        .px_2()
        .py(px(6.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .style_when(selected, |s| s.bg(move || theme().sidebar_accent))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(mark)
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text(shown_text)
                        .number_of_lines(2u32)
                        .text_size(px(13.0))
                        .text_color(move || theme().sidebar_foreground),
                )
                .child(
                    text(kind)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

/// `kind`'s name in the reader's language.
fn kind_label(kind: Kind) -> String {
    t!(&format!("annotations.kind.{}", kind.name()))
}
