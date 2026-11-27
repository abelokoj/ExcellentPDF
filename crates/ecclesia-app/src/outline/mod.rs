//! The outline (plan §11.14), the view: the sidebar's Outline tab, the document's table of contents as a tree. A
//! press (or the arrows) on an entry goes to its page; the entry the reader is in is selected and kept in view as
//! pages turn. The outline is read on the render workers when the tab first shows, never at startup, and again after
//! every edit.
//!
//! Editing (B.1–B.5, [`edit`]): each row has a context menu (the secondary button, Shift+F10 or the Menu key) with
//! every outline command; Ctrl+press and Shift+press mark several entries; F2 edits a title in place. While the
//! outline has the keyboard focus, F2, Delete, Alt+Up and Alt+Down, Tab and Shift+Tab act on it, and Escape gives the
//! focus back to the pages (Tab moves entries in, so it doesn't leave the tree).
//!
//! Entries are drawn bold, italic and in their colour as the document has them (B.7), the colour adjusted to read on
//! the theme's sidebar; Ctrl+Shift+B and Ctrl+Shift+I (⌘ on a Mac) toggle bold and italic while the outline has the
//! keyboard, and the context menu has those and a Colour submenu.
//!
//! Entries that open nowhere (B.9) say why in place of their page label; Check destinations counts them and Fix
//! broken entries points each at the heading with its title, or removes it.

mod edit;
pub(crate) mod extra;

use std::path::PathBuf;

use ecclesia_core::outline::{self, OutlineItem, OutlineStyle};
use ecclesia_core::shared::{pool, registry};
use ecclesia_features::outline::{self as model, Broken, Entry, Restyle, Shift, Swatch};
use nsh_base::{
    ContextMenuElement, MenuItemElement, Tree, TreeApi, TreeEntry, TreeItem, button, context_menu,
    input, menu_item, menu_separator, submenu, theme, tree,
};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

pub(crate) use edit::OutlineAction;

use crate::commands::{self, CommandId};
use crate::editing::Editing;
use crate::goto::PageState;
use crate::panes::Place;
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
    /// The entry whose context menu invoked the current command, if any.
    menu_target: Stored<Option<String>>,
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
    /// The search for broken entries' headings under way (B.9): dropping it cancels it.
    fixing: Stored<Option<Spawned<()>>>,
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
            menu_target: stored(None),
            marked: signal(Vec::new()),
            anchor: stored(None),
            renaming: signal(None),
            awaiting: stored(None),
            shown: signal(false),
            focused: signal(false),
            menu_open: signal(false),
            loaded: stored(None),
            reading: stored(None),
            fixing: stored(None),
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

    /// Reads the shown document's outline on the render workers, opening the document off the UI
    /// thread so tab opens and switches never block.
    fn load(self) {
        let Some(path) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        // Another document: nothing the reader did with the last one's outline carries over.
        if self.loaded.with(|loaded| loaded.as_ref() != Some(&path)) {
            self.loaded.replace(Some(path.clone()));
            self.chosen.set(None);
            self.menu_target.replace(None);
            self.marked.set(Vec::new());
            self.renaming.set(None);
            self.anchor.replace(None);
        }
        let load_path = path.clone();
        let task = spawn(async move {
            let opened = blocking(move || {
                let resolved = crate::expdf::resolve_open_path(&path);
                registry().open(&resolved).and_then(|doc| doc.mupdf())
            })
            .await;
            let mupdf = match opened {
                Ok(Ok(mupdf)) => mupdf,
                Ok(Err(err)) => {
                    log::warn!("outline: cannot open {}: {err}", load_path.display());
                    return;
                }
                Err(err) => {
                    log::warn!("outline: open task failed: {err}");
                    return;
                }
            };
            match outline::load(pool(), mupdf).await {
                Ok(items) => {
                    let mut entries: Vec<Entry> = items.iter().map(entry).collect();
                    let is_pdf_backed = load_path.extension().is_some_and(|ext| {
                        let ext = ext.to_string_lossy();
                        ext.eq_ignore_ascii_case("pdf") || ext.eq_ignore_ascii_case("expdf")
                    });
                    if !is_pdf_backed
                        && let Ok(Some(sidecar)) = extra::sidecar_store().load(&load_path)
                    {
                        entries = sidecar;
                    }
                    self.show(entries);
                }
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

    /// The title of entry `id`, if present in the loaded outline.
    fn title_of(self, id: &str) -> Option<String> {
        self.entries.with(|entries| {
            let entries = entries.as_deref()?;
            Some(model::find(entries, id)?.title.clone())
        })
    }

    /// The page and spot entry `id` opens (V.9).
    fn place_of(self, id: &str) -> Option<(usize, model::Spot)> {
        self.entries.with_untracked(|entries| {
            let entry = model::find(entries.as_deref()?, id)?;
            Some((entry.page?, model::spot(entry.uri.as_deref())))
        })
    }

    /// Why entry `id` opens nowhere, if it does (B.9).
    fn broken_of(self, id: &str) -> Option<Broken> {
        let count = self.pages.count();
        self.entries.with(|entries| {
            let entry = model::find(entries.as_deref()?, id)?;
            model::broken(entry.uri.as_deref(), entry.page, count)
        })
    }

    /// How entry `id` is drawn (B.7).
    fn style_of(self, id: &str) -> OutlineStyle {
        self.entries.with(|entries| {
            entries
                .as_deref()
                .and_then(|entries| model::find(entries, id))
                .map(|entry| entry.style)
                .unwrap_or_default()
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
        uri: item.uri.clone(),
        open: item.open,
        style: item.style,
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
    elsewhere: impl Fn(Place) + Copy + 'static,
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
            move || {
                view()
                    .flex_1()
                    .child(dynamic(move || {
                        let renaming = state.renaming.get();
                        untracked(move || match renaming {
                            Some(id) => rename_bar(state, id).into_element(),
                            None => view().into_element(),
                        })
                    }))
                    .child(outline_tree(state, go, run, elsewhere))
            },
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
        on_key("mod+shift+b", act(OutlineAction::Style(Restyle::Bold)));
        on_key("mod+shift+i", act(OutlineAction::Style(Restyle::Italic)));
        on_key("mod+enter", act(OutlineAction::OpenElsewhere));
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
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
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
    elsewhere: impl Fn(Place) + Copy + 'static,
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
    // Keep the entry being renamed visible in the tree while the rename bar is open.
    effect(move || {
        if let Some(id) = state.renaming.get()
            && let Some(api) = untracked(|| tree_ref.api::<TreeApi>())
        {
            api.reveal_item(id);
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
        .render_item(move |row: &TreeEntry| entry_row(row, state, go, run, elsewhere))
        .flex_1()
}

/// One entry's row: its chevron, its title and the label of the page it opens, with the outline commands in its
/// context menu. A marked or actively renamed entry has the accent's tint and a bar at its start.
fn entry_row<Go, Run, Elsewhere>(
    row: &TreeEntry,
    state: OutlineState,
    go: Go,
    run: Run,
    elsewhere: Elsewhere,
) -> impl IntoElement + use<Go, Run, Elsewhere>
where
    Go: Fn(usize) + Copy + 'static,
    Run: Fn(CommandId) + Copy + 'static,
    Elsewhere: Fn(Place) + Copy + 'static,
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
        let plain = row.label.clone();
        let style = state.style_of(&id);
        text(plain)
            .flex_1()
            .text_size(px(13.0))
            .truncate()
            // Titles are what the reader scans, so all keep full contrast; the reader's entry is bolder, and
            // the document's own bold is bolder still.
            .style_when(reading, |s| s.font_weight(FontWeight::SEMIBOLD))
            .style_when(move || style.bold, |s| s.font_weight(FontWeight::BOLD))
            .style_when(move || style.italic, |s| s.italic())
            .text_color(move || title_color(style))
    };
    let pressed = {
        let id = id.clone();
        move |event: &PressEvent| state.pressed(&id, folder, event.modifiers, go)
    };
    let highlighted = {
        let marked = marked.clone();
        let renaming = renaming.clone();
        move || marked() || renaming()
    };
    let content = view()
        .flex_1()
        .flex_row()
        .items_center()
        .gap_1()
        .self_stretch()
        .pl(px(4.0 + INDENT * row.depth as f32))
        .rounded(px(6.0))
        .style_when(marked, |s| s.bg(move || theme().accent))
        .style_when(renaming, |s| {
            s.bg(move || theme().accent)
                .border_1()
                .border_color(move || theme().ring)
        })
        .on_press(pressed)
        // V.9: the middle button shows the entry's destination in the other pane (Ctrl+click marks entries).
        .on_mouse_up({
            let id = id.clone();
            move |event: &MouseEvent| {
                if event.button == Some(MouseButton::Middle)
                    && let Some((page, spot)) = state.place_of(&id)
                {
                    elsewhere(Place {
                        page,
                        spot: Some(spot),
                    });
                }
            }
        })
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            // The mark's bar: marked and actively renamed rows read clearly without relying on tint alone.
            view()
                .w(px(2.0))
                .h(px(16.0))
                .rounded(px(1.0))
                .bg(move || theme().primary)
                .style_when(move || !highlighted(), |s| s.opacity(0.0)),
        )
        .child(chevron(folder, open))
        .child(title)
        .child(where_to(state, id.clone(), page));
    let menu = context_menu()
        .trigger(content)
        .on_open(move || state.menu_open.set(true))
        .on_close(move || state.menu_open.set(false))
        .popup_class("w-60 p-1 rounded-lg border border-border bg-popover shadow-lg");
    entry_menu(menu, state, id, run)
}

/// What a row says at its end: the label of the page the entry opens, or why it opens nowhere (B.9), in the
/// destructive colour after a mark so it doesn't rely on colour alone.
fn where_to(state: OutlineState, id: String, page: Option<usize>) -> impl IntoElement {
    let pages = state.pages;
    let broken = move || state.broken_of(&id);
    let why = move |broken: Broken| match broken {
        Broken::NoTarget => t!("outline.broken.no_target"),
        Broken::Unresolved => t!("outline.broken.unresolved"),
        Broken::MissingPage(page) => t!("outline.broken.missing_page", page = page + 1),
    };
    let warn = move || readable_on_rows(theme().destructive);
    when(
        {
            let broken = broken.clone();
            move || broken().is_some()
        },
        move || {
            let broken = broken.clone();
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    text("!")
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(warn),
                )
                .child(
                    text(move || broken().map(why).unwrap_or_default())
                        .text_size(px(12.0))
                        .text_color(warn),
                )
        },
    )
    .otherwise(move || {
        text(move || page.map(|page| pages.label(page)).unwrap_or_default())
            .text_size(px(12.0))
            .tabular_nums()
            .text_color(move || theme().muted_foreground)
    })
}

/// `colour` made to read on every background a row can have (the sidebar, a hovered or selected row, a marked one).
fn readable_on_rows(colour: nsh_core::element::AdaptiveColor) -> nsh_core::element::AdaptiveColor {
    let text = theme().sidebar_foreground;
    let papers = [theme().sidebar, theme().sidebar_accent, theme().accent].map(crate::theme::rgb);
    let [r, g, b] =
        crate::theme::readable(crate::theme::rgb(colour), &papers, crate::theme::rgb(text));
    nsh_core::color::rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)).into()
}

/// The colour of a title drawn with `style`: the theme's, or the entry's own, made to read on every background a row
/// can have (the sidebar, a hovered or selected row, a marked one).
fn title_color(style: OutlineStyle) -> nsh_core::element::AdaptiveColor {
    match style.color {
        Some([r, g, b]) => readable_on_rows(
            nsh_core::color::rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)).into(),
        ),
        None => theme().sidebar_foreground,
    }
}

/// `menu` with the outline commands, for the context menu of the entry of id `id`: they act on it (and on the other
/// marked entries when it is one of them).
fn entry_menu(
    menu: ContextMenuElement,
    state: OutlineState,
    id: String,
    run: impl Fn(CommandId) + Copy + 'static,
) -> ContextMenuElement {
    // The menu always targets the entry under the pointer; marks are cleared unless this row is already marked.
    let act = move |action: OutlineAction| {
        let id = id.clone();
        move || {
            if !untracked(|| state.is_marked(&id)) {
                state.marked.set(Vec::new());
            }
            state.chosen.set(Some(id.clone()));
            state.menu_target.replace(Some(id.clone()));
            run(CommandId::Outline(action));
            state.menu_target.replace(None);
        }
    };
    let item = {
        let act = act.clone();
        move |action: OutlineAction| menu_entry(CommandId::Outline(action), act(action))
    };
    let separator = || {
        menu_separator()
            .h(px(1.0))
            .my(px(4.0))
            .bg(move || theme().border)
    };
    let colors =
        Some(None)
            .into_iter()
            .chain(Swatch::ALL.map(Some))
            .fold(color_menu(), |colors, swatch| {
                let action = OutlineAction::Style(Restyle::Color(swatch));
                colors.child(swatch_entry(
                    CommandId::Outline(action),
                    swatch,
                    act(action),
                ))
            });
    menu.child(item(OutlineAction::Add))
        .child(item(OutlineAction::Rename))
        .child(item(OutlineAction::SetDestination))
        .child(separator())
        .child(item(OutlineAction::Style(Restyle::Bold)))
        .child(item(OutlineAction::Style(Restyle::Italic)))
        .child(colors)
        .child(separator())
        .child(item(OutlineAction::Move(Shift::Up)))
        .child(item(OutlineAction::Move(Shift::Down)))
        .child(item(OutlineAction::Move(Shift::In)))
        .child(item(OutlineAction::Move(Shift::Out)))
        .child(item(OutlineAction::Sort))
        .child(separator())
        .child(item(OutlineAction::OpenElsewhere))
        .child(item(OutlineAction::ToBookmark))
        .child(item(OutlineAction::Check))
        .child(item(OutlineAction::Fix))
        .child(separator())
        .child(item(OutlineAction::Delete))
}

/// The Colour submenu, its trigger a row like the menu's items with a chevron at its end (B.7).
fn color_menu() -> nsh_base::SubmenuElement {
    let row = |label: String| {
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .h(px(32.0))
            .px_2()
            .rounded(px(6.0))
            .hover(|s| s.bg(move || theme().accent))
            .focus(|s| s.bg(move || theme().accent))
            .child(
                text(label)
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(move || theme().popover_foreground),
            )
    };
    submenu()
        .trigger(
            row(t!("outline.color")).child(
                text("›")
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
            ),
        )
        .back(row(t!("outline.color_back")))
}

/// A Colour submenu item: a dot of the swatch (an outlined empty dot for none) and the command's label.
fn swatch_entry(
    id: CommandId,
    swatch: Option<Swatch>,
    select: impl Fn() + 'static,
) -> MenuItemElement {
    let dot = view()
        .w(px(12.0))
        .h(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().border);
    let dot = match swatch {
        Some(swatch) => {
            let [r, g, b] = swatch.rgb();
            let fill = nsh_core::color::rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b));
            dot.bg(move || fill)
        }
        None => dot,
    };
    menu_item()
        .on_select(select)
        .accessibility_label(commands::label(id))
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(32.0))
        .px_2()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(dot)
        .child(
            text(commands::label(id))
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

/// A menu item for command `id`: its label and its shortcut, as the palette shows them.
pub(crate) fn menu_entry(id: CommandId, select: impl Fn() + 'static) -> MenuItemElement {
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

/// The inline rename bar pinned at the top of the outline panel while editing the title of entry `id` (B.2).
/// Rendered outside `nsh_base::tree()` and `context_menu()` so tree key/press handlers and context-menu focus
/// restoration never intercept input or abort the rename; Return or Save keeps the title, Escape or Cancel drops it.
fn rename_bar(state: OutlineState, id: String) -> impl IntoElement {
    let initial = untracked(|| state.title_of(&id)).unwrap_or_default();
    let typed = signal(initial.clone());
    let selected = u32::try_from(initial.encode_utf16().count()).unwrap_or(u32::MAX);
    let field_ref = element_ref();
    // Focus once mounted and again after any open context menu finishes closing and restoring focus.
    effect(move || {
        if !state.menu_open.get() {
            spawn(async move {
                next_frame().await;
                field_ref.focus();
            });
        }
    });
    let done = move |keep: bool| {
        if untracked(|| state.renaming.get()).as_deref() != Some(id.as_str()) {
            return;
        }
        state.renaming.set(None);
        if keep {
            state.rename_to(&id, &untracked(|| typed.get()));
        }
    };
    let in_fields = KeyOptions {
        when: nsh_core::keymap::When::Always,
        ..KeyOptions::default()
    };
    let cancel_key = done.clone();
    on_key_with("escape", in_fields, move |_| cancel_key(false));
    let submit = done.clone();
    let save_btn = done.clone();
    let cancel_btn = done;
    let field = input()
        .element_ref(&field_ref)
        .value(move || typed.get())
        .on_change(move |text: &str| typed.set(text.to_owned()))
        .on_submit(move |_: &str| submit(true))
        .label(t!("outline.title"))
        .auto_focus(true)
        .selected_range((0, selected))
        .h(px(30.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().ring)
        .bg(move || theme().background)
        .text_size(px(13.0))
        .text_color(move || theme().foreground);
    let cancel = button()
        .accessibility_label(t!("bookmark.cancel"))
        .on_press(move || cancel_btn(false))
        .h(px(26.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(t!("bookmark.cancel"))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().sidebar_foreground),
        );
    let save = button()
        .accessibility_label(t!("bookmark.save"))
        .on_press(move || save_btn(true))
        .h(px(26.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .bg(move || theme().primary)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(t!("bookmark.save"))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().primary_foreground),
        );
    view()
        .flex_shrink_0()
        .p_2()
        .gap_2()
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(move || t!("outline.op.rename"))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
        .child(field)
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(cancel)
                .child(save),
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
    // The turn jumps when the reader asked the system for less motion (U.4).
    .duration(move || if reduce_motion() { 0.0_f32 } else { 150.0 })
    .child(mark)
}
