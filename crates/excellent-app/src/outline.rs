//! The outline (plan §11.14), the view: the sidebar's Outline tab, the document's table of contents as a tree. A
//! press (or the arrows) on an entry goes to its page; the entry the reader is in is selected and kept in view as
//! pages turn. The outline is read on the render workers when the tab first shows, never at startup. Editing (B.1–B.5)
//! comes with the annotation and save work it needs (K.14, K.15).

use excellent_core::outline::{self, OutlineItem};
use excellent_core::shared::{pool, registry};
use excellent_features::outline::{self as model, Entry};
use nsh_base::{Tree, TreeApi, TreeEntry, TreeItem, theme, tree};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use crate::goto::PageState;
use crate::tabs::TabState;

/// How far each level of the outline is indented, in points.
const INDENT: f32 = 14.0;

/// The outline and what the reader did with it.
#[derive(Clone, Copy)]
pub(crate) struct OutlineState {
    /// The entries, once read; empty when the document has none.
    entries: Signal<Option<Vec<Entry>>>,
    /// The entries as the tree shows them.
    items: Signal<Vec<TreeItem>>,
    /// The expanded entries' ids.
    expanded: Signal<Vec<SharedString>>,
    /// The entry the reader last chose, which stays selected while it opens the page being read.
    chosen: Signal<Option<String>>,
    /// The read under way: dropping it cancels it.
    reading: Stored<Option<Spawned<()>>>,
    tabs: TabState,
}

impl OutlineState {
    pub(crate) fn new(tabs: TabState) -> Self {
        Self {
            entries: signal(None),
            items: signal(Vec::new()),
            expanded: signal(Vec::new()),
            chosen: signal(None),
            reading: stored(None),
            tabs,
        }
    }

    /// Reads the shown document's outline on the render workers.
    fn load(self) {
        let Some(path) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        let mupdf = match registry().open(&path).and_then(|document| document.mupdf()) {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::warn!("outline: cannot open {}: {err}", path.display());
                return;
            }
        };
        let job = outline::load(pool(), mupdf);
        let task = spawn(async move {
            match job.await {
                Ok(items) => self.show(items.iter().map(entry).collect()),
                Err(err) if !err.is_aborted() => log::warn!("outline: {err}"),
                Err(_) => {}
            }
        });
        self.reading.with(|reading| *reading = Some(task));
    }

    /// Shows `entries`, expanded as the file says.
    fn show(self, entries: Vec<Entry>) {
        let open = model::open_ids(&entries);
        self.expanded
            .set(open.into_iter().map(SharedString::from).collect());
        self.items.set(tree_items(&entries, &mut Vec::new()));
        self.chosen.set(None);
        self.entries.set(Some(entries));
    }

    /// Whether the document was read and has no outline.
    fn is_empty(self) -> bool {
        self.entries
            .with(|entries| entries.as_ref().is_some_and(Vec::is_empty))
    }

    fn has_entries(self) -> bool {
        self.entries
            .with(|entries| entries.as_ref().is_some_and(|entries| !entries.is_empty()))
    }

    /// The page entry `id` opens.
    fn page_of(self, id: &str) -> Option<usize> {
        self.entries.with(|entries| {
            let entries = entries.as_deref()?;
            model::find(entries, id)?.page
        })
    }

    /// The entry the reader of page `page` is in.
    fn reading_entry(self, page: usize) -> Option<SharedString> {
        self.entries.with(|entries| {
            let entries = entries.as_deref()?;
            self.chosen
                .with(|chosen| model::reading(entries, chosen.as_deref(), page))
                .map(SharedString::from)
        })
    }
}

/// An outline item as the model holds it.
fn entry(item: &OutlineItem) -> Entry {
    Entry {
        title: item.title.clone(),
        page: item.page,
        open: item.open,
        children: item.children.iter().map(entry).collect(),
    }
}

/// `entries` as tree items, each with its path id.
fn tree_items(entries: &[Entry], path: &mut Vec<usize>) -> Vec<TreeItem> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            path.push(index);
            let item = TreeItem::new(model::id(path), entry.title.clone())
                .children(tree_items(&entry.children, path));
            path.pop();
            item
        })
        .collect()
}

/// The Outline tab. `go` gets the index of a page to show.
pub(crate) fn outline_panel(
    state: OutlineState,
    pages: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    // Read when the tab shows and again when another document is shown.
    effect(move || {
        if pages.count() > 0 {
            untracked(|| state.load());
        }
    });
    view().flex_1().child(
        when(
            move || state.has_entries(),
            move || outline_tree(state, pages, go),
        )
        .otherwise(move || {
            when(
                move || state.is_empty(),
                || crate::shell::empty_message(|| t!("panel.outline"), || t!("outline.none")),
            )
        }),
    )
}

/// The tree of entries.
fn outline_tree(
    state: OutlineState,
    pages: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let tree_ref = component_ref::<Tree>();
    let selected = move || state.reading_entry(pages.current());
    // Keep the reader's entry in view as pages turn (the tree expands what is above it).
    effect(move || {
        if let Some(id) = selected()
            && let Some(api) = untracked(|| tree_ref.api::<TreeApi>())
        {
            api.reveal_item(id.to_string());
        }
    });
    tree()
        .items(move || state.items.get())
        .expanded(move || Some(state.expanded.get()))
        .on_expanded_change(move |ids: &Vec<SharedString>| state.expanded.set(ids.clone()))
        .selected(selected)
        .on_selected_change(move |id: &SharedString| {
            state.chosen.set(Some(id.to_string()));
            if let Some(page) = state.page_of(id) {
                go(page);
            }
        })
        .accessibility_label(t!("panel.outline"))
        .component_ref(&tree_ref)
        .list_class("p-2")
        .item_class(
            "group flex-row items-center h-8 pr-2 rounded-md hover:bg-sidebar-accent \
             selected:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring",
        )
        .render_item(move |row: &TreeEntry| entry_row(row, state, pages))
        .flex_1()
}

/// One entry's row: its chevron, its title and the label of the page it opens.
fn entry_row(row: &TreeEntry, state: OutlineState, pages: PageState) -> impl IntoElement + use<> {
    let id = row.id.clone();
    let page = state.page_of(&id);
    let reading = {
        let id = id.clone();
        move || state.reading_entry(pages.current()).as_ref() == Some(&id)
    };
    let open = {
        let id = id.clone();
        move || state.expanded.with(|ids| ids.contains(&id))
    };
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap_1()
        .pl(px(4.0 + INDENT * row.depth as f32))
        .child(chevron(row.folder, open))
        .child(
            text(row.label.clone())
                .flex_1()
                .text_size(px(13.0))
                .truncate()
                // Titles are what the reader scans, so all keep full contrast; the reader's entry is bolder.
                .style_when(reading, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || theme().sidebar_foreground),
        )
        .child(
            text(move || page.map(|page| pages.label(page)).unwrap_or_default())
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
}

/// A folder's chevron, pointing right while it is collapsed and down while it is expanded; a leaf keeps the space.
fn chevron(folder: bool, open: impl Fn() -> bool + 'static) -> impl IntoElement {
    let slot = view()
        .w(px(16.0))
        .h(px(16.0))
        .items_center()
        .justify_center();
    if !folder {
        return slot;
    }
    // Two sides of a small square, turned to make the point.
    let mark = view()
        .w(px(6.0))
        .h(px(6.0))
        .border_r(px(1.5))
        .border_b(px(1.5))
        .border_color(move || theme().muted_foreground)
        .rotate(Radians(-std::f32::consts::FRAC_PI_4));
    slot.rotate(move || {
        Radians(if open() {
            std::f32::consts::FRAC_PI_2
        } else {
            0.0
        })
    })
    .transition_transform()
    .child(mark)
}
