//! The outline (plan §11.14), the view: the sidebar's Outline tab, the document's table of contents as a tree. A
//! press (or the arrows) on an entry goes to its page; the entry the reader is in is selected and kept in view as
//! pages turn. The outline is read on the render workers when the tab first shows, never at startup, and again after
//! every edit.
//!
//! Editing (B.1–B.5, [`edit`]): each row has a context menu (the secondary button, Shift+F10 or the Menu key) with
//! every outline command; Ctrl+press and Shift+press mark several entries; F2 edits a title in place. While the
//! outline has the keyboard focus, F2, Delete, Alt+Up and Alt+Down, Tab and Shift+Tab act on it, and Escape gives the
//! focus back to the pages (Tab moves entries in, so it doesn't leave the tree).

mod edit;

use std::path::PathBuf;

use excellent_core::outline::{self, OutlineItem};
use excellent_core::shared::{pool, registry};
use excellent_features::outline::{self as model, Entry, Shift};
use nsh_base::{
    ContextMenuElement, MenuItemElement, Tree, TreeApi, TreeEntry, TreeItem, button, context_menu,
    input, menu_item, menu_separator, theme, tree,
};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

pub(crate) use edit::OutlineAction;

use crate::commands::{self, CommandId};
use crate::editing::Editing;
use crate::goto::PageState;
use crate::tabs::TabState;
use edit::Awaiting;

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
    /// The entries marked to act on together (B.5), in the order they were marked.
    marked: Signal<Vec<String>>,
    /// The entry last pressed, where a Shift+press run of marks starts.
    anchor: Stored<Option<String>>,
    /// The entry whose title is being edited in place (B.2).
    renaming: Signal<Option<String>>,
    /// What the place the pages report next is for (B.1, B.4).
    awaiting: Stored<Option<Awaiting>>,
    /// Whether the Outline tab is showing.
    shown: Signal<bool>,
    /// Whether the outline has the keyboard focus: its own keys work only then.
    focused: Signal<bool>,
    /// Whether a row's context menu is open: the pages, a window over the app, hide meanwhile.
    menu_open: Signal<bool>,
    /// The file the entries are from.
    loaded: Stored<Option<PathBuf>>,
    /// The read under way: dropping it cancels it.
    reading: Stored<Option<Spawned<()>>>,
    tree: ComponentRef<Tree>,
    tabs: TabState,
    pages: PageState,
    editing: Editing,
}

impl OutlineState {
    pub(crate) fn new(tabs: TabState, pages: PageState, editing: Editing) -> Self {
        Self {
            entries: signal(None),
            items: signal(Vec::new()),
            expanded: signal(Vec::new()),
            chosen: signal(None),
            marked: signal(Vec::new()),
            anchor: stored(None),
            renaming: signal(None),
            awaiting: stored(None),
            shown: signal(false),
            focused: signal(false),
            menu_open: signal(false),
            loaded: stored(None),
            reading: stored(None),
            tree: component_ref::<Tree>(),
            tabs,
            pages,
            editing,
        }
    }

    /// Whether a row's context menu is open (tracked).
    pub(crate) fn is_menu_open(self) -> bool {
        self.menu_open.get()
    }

    /// Reads the shown document's outline on the render workers.
    fn load(self) {
        let Some(path) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        // Another document: nothing the reader did with the last one's outline carries over.
        if self.loaded.with(|loaded| loaded.as_ref() != Some(&path)) {
            self.loaded.replace(Some(path.clone()));
            self.chosen.set(None);
            self.marked.set(Vec::new());
            self.renaming.set(None);
            self.anchor.replace(None);
        }
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

    /// Shows `entries`, expanded as the file says. The reader's choice and marks stay on the entries still there.
    fn show(self, entries: Vec<Entry>) {
        let open = model::open_ids(&entries);
        self.expanded
            .set(open.into_iter().map(SharedString::from).collect());
        self.items.set(tree_items(&entries, &mut Vec::new()));
        let there = |id: &String| model::find(&entries, id).is_some();
        if untracked(|| self.chosen.get()).is_some_and(|id| !there(&id)) {
            self.chosen.set(None);
        }
        self.marked.update(|marked| marked.retain(there));
        if untracked(|| self.renaming.get()).is_some_and(|id| !there(&id)) {
            self.renaming.set(None);
        }
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

    fn is_marked(self, id: &str) -> bool {
        self.marked
            .with(|marked| marked.iter().any(|marked| marked == id))
    }

    /// The pages take the keyboard back.
    fn leave(self, pages_focus: ElementRef) {
        self.marked.set(Vec::new());
        pages_focus.focus();
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

/// The Outline tab. `go` gets the index of a page to show; `run` runs a command; `pages_focus` holds the keyboard
/// focus for the pages.
pub(crate) fn outline_panel(
    state: OutlineState,
    go: impl Fn(usize) + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
    pages_focus: ElementRef,
) -> impl IntoElement {
    let pages = state.pages;
    state.shown.set(true);
    cleanup(move || {
        state.shown.set(false);
        state.focused.set(false);
    });
    // Read when the tab shows, again when another document is shown, and after every edit.
    effect(move || {
        state.editing.revision();
        if pages.count() > 0 {
            untracked(|| state.load());
        }
    });
    let holder = element_ref();
    holder.on_focus_in(move || state.focused.set(true));
    holder.on_focus_out(move || state.focused.set(false));
    outline_keys(state, run, pages_focus);
    view().flex_1().element_ref(&holder).child(
        when(
            move || state.has_entries(),
            move || outline_tree(state, go, run),
        )
        .otherwise(move || when(move || state.is_empty(), move || no_outline(state, run))),
    )
}

/// The outline's own keys, while it has the keyboard focus and no title is being edited.
fn outline_keys(
    state: OutlineState,
    run: impl Fn(CommandId) + Copy + 'static,
    pages_focus: ElementRef,
) {
    effect(move || {
        if !state.focused.get() || state.renaming.get().is_some() {
            return;
        }
        let act = move |action: OutlineAction| move |_: &str| run(CommandId::Outline(action));
        on_key("f2", act(OutlineAction::Rename));
        on_key("delete", act(OutlineAction::Delete));
        on_key("alt+up", act(OutlineAction::Move(Shift::Up)));
        on_key("alt+down", act(OutlineAction::Move(Shift::Down)));
        on_key("tab", act(OutlineAction::Move(Shift::In)));
        on_key("shift+tab", act(OutlineAction::Move(Shift::Out)));
        on_key("escape", move |_: &str| state.leave(pages_focus));
    });
}

/// A document without an outline: the message, and the first entry to add (B.1 step 4).
fn no_outline(state: OutlineState, run: impl Fn(CommandId) + Copy + 'static) -> impl IntoElement {
    let add = CommandId::Outline(OutlineAction::Add);
    crate::shell::empty_message(|| t!("panel.outline"), || t!("outline.none")).child(when(
        move || state.editing.can_edit(),
        move || {
            button()
                .accessibility_label(commands::label_with_shortcut(add))
                .on_press(move || run(add))
                .mt(px(8.0))
                .h(px(32.0))
                .px_4()
                .rounded(px(8.0))
                .items_center()
                .justify_center()
                .bg(move || theme().primary)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.7))
                .child(
                    text(t!("outline.add_first"))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().primary_foreground),
                )
        },
    ))
}

/// The tree of entries.
fn outline_tree(
    state: OutlineState,
    go: impl Fn(usize) + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let pages = state.pages;
    let tree_ref = state.tree;
    let selected = move || state.reading_entry(pages.current());
    // Keep the reader's entry in view as pages turn (the tree expands what is above it).
    effect(move || {
        if let Some(id) = selected()
            && let Some(api) = untracked(|| tree_ref.api::<TreeApi>())
        {
            api.reveal_item(id.to_string());
        }
    });
    // An edited title gives the keyboard back to the tree.
    effect(move || {
        if state.renaming.get().is_none()
            && untracked(|| state.focused.get())
            && let Some(api) = untracked(|| tree_ref.api::<TreeApi>())
        {
            api.focus();
        }
    });
    tree()
        .items(move || state.items.get())
        .expanded(move || Some(state.expanded.get()))
        .on_expanded_change(move |ids: &Vec<SharedString>| state.expanded.set(ids.clone()))
        .selected(selected)
        .on_selected_change(move |id: &SharedString| {
            state.marked.set(Vec::new());
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
        .render_item(move |row: &TreeEntry| entry_row(row, state, go, run))
        .flex_1()
}

/// One entry's row: its chevron, its title (or the field editing it) and the label of the page it opens, with the
/// outline commands in its context menu. A marked entry has the accent's tint and a bar at its start.
fn entry_row<Go, Run>(
    row: &TreeEntry,
    state: OutlineState,
    go: Go,
    run: Run,
) -> impl IntoElement + use<Go, Run>
where
    Go: Fn(usize) + Copy + 'static,
    Run: Fn(CommandId) + Copy + 'static,
{
    let pages = state.pages;
    let id = row.id.to_string();
    let folder = row.folder;
    let page = state.page_of(&id);
    let reading = {
        let id = row.id.clone();
        move || state.reading_entry(pages.current()).as_ref() == Some(&id)
    };
    let open = {
        let id = row.id.clone();
        move || state.expanded.with(|ids| ids.contains(&id))
    };
    let marked = {
        let id = id.clone();
        move || state.is_marked(&id)
    };
    let renaming = {
        let id = id.clone();
        move || {
            state
                .renaming
                .with(|renaming| renaming.as_deref() == Some(id.as_str()))
        }
    };
    let title = {
        let (label, editing) = (row.label.clone(), id.clone());
        let plain = label.clone();
        when(renaming, move || {
            title_field(state, editing.clone(), label.to_string())
        })
        .otherwise(move || {
            text(plain.clone())
                .flex_1()
                .text_size(px(13.0))
                .truncate()
                // Titles are what the reader scans, so all keep full contrast; the reader's entry is bolder.
                .style_when(reading.clone(), |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || theme().sidebar_foreground)
        })
    };
    let pressed = {
        let id = id.clone();
        move |event: &PressEvent| state.pressed(&id, folder, event.modifiers, go)
    };
    let content = view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap_1()
        .self_stretch()
        .pl(px(4.0 + INDENT * row.depth as f32))
        .rounded(px(6.0))
        .style_when(marked.clone(), |s| s.bg(move || theme().accent))
        .on_press(pressed)
        .child(
            // The mark's bar: marked rows read as a group without relying on the tint alone.
            view()
                .w(px(2.0))
                .h(px(16.0))
                .rounded(px(1.0))
                .bg(move || theme().primary)
                .style_when(move || !marked(), |s| s.opacity(0.0)),
        )
        .child(chevron(folder, open))
        .child(title)
        .child(
            text(move || page.map(|page| pages.label(page)).unwrap_or_default())
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        );
    let menu = context_menu()
        .trigger(content)
        .on_open(move || state.menu_open.set(true))
        .on_close(move || state.menu_open.set(false))
        .popup_class("w-60 p-1 rounded-lg border border-border bg-popover shadow-lg");
    entry_menu(menu, state, id, run)
}

/// `menu` with the outline commands, for the context menu of the entry of id `id`: they act on it (and on the other
/// marked entries when it is one of them).
fn entry_menu(
    menu: ContextMenuElement,
    state: OutlineState,
    id: String,
    run: impl Fn(CommandId) + Copy + 'static,
) -> ContextMenuElement {
    // The menu acts on the entry under the pointer: it becomes the selected one unless it is marked.
    let choose = move || {
        if !untracked(|| state.is_marked(&id)) {
            state.marked.set(Vec::new());
            state.chosen.set(Some(id.clone()));
        }
    };
    let item = move |action: OutlineAction| {
        let choose = choose.clone();
        menu_entry(CommandId::Outline(action), move || {
            choose();
            run(CommandId::Outline(action));
        })
    };
    let separator = || {
        menu_separator()
            .h(px(1.0))
            .my(px(4.0))
            .bg(move || theme().border)
    };
    menu.child(item(OutlineAction::Add))
        .child(item(OutlineAction::Rename))
        .child(item(OutlineAction::SetDestination))
        .child(separator())
        .child(item(OutlineAction::Move(Shift::Up)))
        .child(item(OutlineAction::Move(Shift::Down)))
        .child(item(OutlineAction::Move(Shift::In)))
        .child(item(OutlineAction::Move(Shift::Out)))
        .child(separator())
        .child(item(OutlineAction::Delete))
}

/// A menu item for command `id`: its label and its shortcut, as the palette shows them.
fn menu_entry(id: CommandId, select: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(commands::label_with_shortcut(id))
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(32.0))
        .px_2()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(
            text(commands::label(id))
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(commands::shortcut_label(id))
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
}

/// The field editing the title of the entry of id `id` in place (B.2): Return or leaving it keeps the title, Escape
/// drops it.
fn title_field(state: OutlineState, id: String, title: String) -> impl IntoElement {
    let typed = signal(title.clone());
    let selected = u32::try_from(title.encode_utf16().count()).unwrap_or(u32::MAX);
    let done = move |keep: bool| {
        // Once: Return, then the field going (a blur), would keep it twice.
        if untracked(|| state.renaming.get()).as_deref() != Some(id.as_str()) {
            return;
        }
        state.renaming.set(None);
        if keep {
            state.rename_to(&id, &untracked(|| typed.get()));
        }
    };
    let cancel = done.clone();
    on_key("escape", move |_: &str| cancel(false));
    let submit = done.clone();
    input()
        .value(move || typed.get())
        .on_change(move |text: &str| typed.set(text.to_owned()))
        .on_submit(move |_: &str| submit(true))
        .on_blur(move || done(true))
        .label(t!("outline.title"))
        .auto_focus(true)
        .selected_range((0, selected))
        .flex_1()
        .h(px(26.0))
        .px_2()
        .rounded(px(4.0))
        .border_1()
        .border_color(move || theme().ring)
        .bg(move || theme().background)
        .text_size(px(13.0))
        .text_color(move || theme().foreground)
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
