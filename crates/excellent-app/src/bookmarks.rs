//! Personal bookmarks (plan B.6), the view: the sidebar's Bookmarks tab, the places the reader saved in the shown
//! file, searchable by title, note and tag. A press on one goes to its place, at its zoom. Ctrl+D (⌘D) bookmarks the
//! place being read, titled with the selected text or the page, in a dialog that also takes a note, a colour and
//! tags; the row's context menu (or F2 and Delete while the list has the keyboard) edits and deletes.
//!
//! Bookmarks live in the library (`excellent_store::bookmarks`), not in the file, so every format has them; a file
//! renamed or moved finds its own again by its content when it is next shown.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use excellent_core::shared::registry;
use excellent_features::bookmarks as model;
use excellent_features::outline::title_from_selection;
use excellent_store::bookmarks::{self as library, Bookmark, Details, Place};
use nsh_base::{alert_dialog, button, context_menu, input, theme, toggle};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::commands::{self, CommandId};
use crate::goto::PageState;
use crate::list_dialog::{card, list_dialog};
use crate::outline::menu_entry;
use crate::tabs::{GroupColor, TabState, Tone, action_button, color_name};

/// What a bookmark command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookmarkAction {
    /// Bookmarks the place being read.
    Add,
    /// Edits the selected bookmark's title, note, colour and tags.
    Edit,
    /// Deletes the selected bookmark, once the reader confirms.
    Delete,
}

/// What the editor dialog saves to.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// A new bookmark at `place`, with the text selected there.
    New { place: Place, quote: Option<String> },
    /// The saved bookmark of this id.
    Saved(i64),
}

/// The bookmark being made or edited.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    target: Target,
    details: Details,
}

/// The shown file's bookmarks and what the reader is doing with them.
#[derive(Clone, Copy)]
pub(crate) struct BookmarkState {
    /// The bookmarks of [`Self::loaded`], in reading order.
    list: Signal<Vec<Bookmark>>,
    /// The file the list is from.
    loaded: Signal<Option<PathBuf>>,
    /// What the search field holds.
    query: Signal<String>,
    /// The bookmark last pressed: the one Edit and Delete act on.
    selected: Signal<Option<i64>>,
    /// The bookmark in the editor dialog.
    draft: Signal<Option<Draft>>,
    /// Whether the place the pages report next is for a new bookmark.
    awaiting: Stored<bool>,
    /// Whether the list has the keyboard focus: its own keys work only then.
    focused: Signal<bool>,
    /// Whether a row's context menu is open: the pages, a window over the app, hide meanwhile.
    menu_open: Signal<bool>,
    tabs: TabState,
    pages: PageState,
}

impl BookmarkState {
    pub(crate) fn new(tabs: TabState, pages: PageState) -> Self {
        Self {
            list: signal(Vec::new()),
            loaded: signal(None),
            query: signal(String::new()),
            selected: signal(None),
            draft: signal(None),
            awaiting: stored(false),
            focused: signal(false),
            menu_open: signal(false),
            tabs,
            pages,
        }
    }

    /// Whether the editor dialog or a row's context menu is open (tracked): the pages hide meanwhile.
    pub(crate) fn covers_pages(self) -> bool {
        self.draft.with(Option::is_some) || self.menu_open.get()
    }

    /// Whether the editor dialog is open (tracked).
    pub(crate) fn is_editing(self) -> bool {
        self.draft.with(Option::is_some)
    }

    /// Runs `action`; `canvas` is asked where the reader is for a new bookmark.
    pub(crate) fn run(self, action: BookmarkAction, canvas: ElementRef) {
        match action {
            BookmarkAction::Add => {
                self.awaiting.replace(true);
                canvas.command("readerPlace", JsonValue::Null);
            }
            BookmarkAction::Edit => {
                if let Some(bookmark) = self.selected_bookmark() {
                    self.draft.set(Some(Draft {
                        target: Target::Saved(bookmark.id),
                        details: bookmark.details,
                    }));
                }
            }
            BookmarkAction::Delete => {
                if let Some(bookmark) = self.selected_bookmark() {
                    self.confirm_delete(bookmark);
                }
            }
        }
    }

    /// The pages reported where the reader is (`readerPlace`): a new bookmark there goes to the editor.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name != "readerPlace" || !self.awaiting.replace(false) {
            return;
        }
        let Some((place, quote)) = place(&event.data) else {
            log::warn!("bookmarks: the pages reported no place: {}", event.data);
            return;
        };
        let title = quote
            .as_deref()
            .and_then(title_from_selection)
            .unwrap_or_else(|| t!("bookmark.page_title", page = self.pages.label(place.page)));
        self.draft.set(Some(Draft {
            target: Target::New { place, quote },
            details: Details {
                title,
                color: GroupColor::ALL[0].id().to_owned(),
                ..Details::default()
            },
        }));
    }

    /// Reads the shown file's bookmarks, first taking those it had under another name if it was renamed or moved.
    fn load(self) {
        let Some(path) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        if untracked(|| self.loaded.get()).as_ref() != Some(&path) {
            self.selected.set(None);
            self.query.set(String::new());
        }
        // The document is open already: its identity is known without reading the file again.
        let content = registry()
            .open(&path)
            .map(|document| document.id().content_key())
            .inspect_err(|err| log::warn!("bookmarks: cannot open {}: {err}", path.display()))
            .ok();
        let read = path.clone();
        let list = change(move |tx| {
            if let Some(content) = content
                && library::find_again(tx, &read, content, Path::exists)?
            {
                log::info!("bookmarks: found again for {}", read.display());
            }
            library::list(tx, &read)
        })
        .unwrap_or_default();
        self.selected.update(|selected| {
            if selected.is_some_and(|id| !list.iter().any(|bookmark| bookmark.id == id)) {
                *selected = None;
            }
        });
        self.list.set(list);
        self.loaded.set(Some(path));
    }

    /// The bookmarks the search finds (tracked).
    fn found(self) -> Vec<Bookmark> {
        let query = self.query.get();
        self.list.with(|list| {
            list.iter()
                .filter(|bookmark| {
                    let details = &bookmark.details;
                    model::matches(&query, &details.title, &details.note, &details.tags)
                })
                .cloned()
                .collect()
        })
    }

    fn selected_bookmark(self) -> Option<Bookmark> {
        let id = untracked(|| self.selected.get())?;
        untracked(|| {
            self.list
                .with(|list| list.iter().find(|bookmark| bookmark.id == id).cloned())
        })
    }

    /// Saves the editor's bookmark with `details`, and shows it selected.
    fn save(self, draft: Draft) {
        let Some(path) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        let Draft { target, details } = draft;
        let saved = change(move |tx| match target {
            Target::New { place, quote } => {
                library::add(tx, &path, place, quote.as_deref(), &details, now())
            }
            Target::Saved(id) => library::update(tx, id, &details).map(|()| id),
        });
        if let Some(id) = saved {
            self.load();
            self.selected.set(Some(id));
        }
    }

    /// Asks before deleting `bookmark`: its note and tags go with it.
    fn confirm_delete(self, bookmark: Bookmark) {
        let answer = alert_dialog()
            .title(t!("bookmark.delete_title", title = bookmark.details.title))
            .description(t!("bookmark.delete_detail"))
            .cancel("cancel", t!("bookmark.cancel"))
            .destructive("delete", t!("bookmark.delete"))
            .show();
        spawn(async move {
            if answer
                .await
                .is_ok_and(|answer| answer.action.as_deref() == Some("delete"))
                && change(move |tx| library::delete(tx, bookmark.id)).is_some()
            {
                self.load();
            }
        });
    }
}

/// The place in a `readerPlace` event, with the text selected there.
fn place(data: &JsonValue) -> Option<(Place, Option<String>)> {
    let number = |key: &str| data.get(key).and_then(JsonValue::as_f64).map(|n| n as f32);
    let place = Place {
        page: usize::try_from(data.get("page")?.as_u64()?).ok()?,
        x: number("x")?,
        y: number("y")?,
        zoom: number("zoom"),
    };
    let quote = data
        .get("title")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    Some((place, quote))
}

/// Makes a change in the library and gives its result; `None` (logged) without a library or when it fails.
///
/// shortcut: waits on the library's writer, well under a millisecond while it is idle; make `Pending` awaitable if a
/// write ever queues behind a long one.
fn change<T: Send + 'static>(
    f: impl FnOnce(&excellent_store::rusqlite::Transaction<'_>) -> excellent_store::Result<T>
    + Send
    + 'static,
) -> Option<T> {
    let store = crate::library::store()?;
    store
        .write(f)
        .wait()
        .inspect_err(|err| log::warn!("bookmarks: the library failed: {err}"))
        .ok()
}

/// Seconds since 1970.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The Bookmarks tab. `canvas` shows a bookmark's place; `run` runs a command; `pages_focus` holds the keyboard focus
/// for the pages.
pub(crate) fn bookmark_panel(
    state: BookmarkState,
    canvas: ElementRef,
    run: impl Fn(CommandId) + Copy + 'static,
    pages_focus: ElementRef,
) -> impl IntoElement {
    let pages = state.pages;
    cleanup(move || state.focused.set(false));
    // Read when the tab shows and again when another document is shown.
    effect(move || {
        if pages.count() > 0 {
            state.tabs.shown_path();
            untracked(|| state.load());
        }
    });
    let holder = element_ref();
    holder.on_focus_in(move || state.focused.set(true));
    holder.on_focus_out(move || state.focused.set(false));
    effect(move || {
        if !state.focused.get() || state.is_editing() {
            return;
        }
        let act = move |action: BookmarkAction| move |_: &str| run(CommandId::Bookmark(action));
        on_key("f2", act(BookmarkAction::Edit));
        on_key("delete", act(BookmarkAction::Delete));
        on_key("escape", move |_: &str| pages_focus.focus());
    });
    let has_any = move || state.list.with(|list| !list.is_empty());
    view().flex_1().element_ref(&holder).child(
        when(has_any, move || {
            view()
                .flex_1()
                .child(search_field(state))
                .child(bookmark_list(state, canvas, run))
        })
        .otherwise(move || no_bookmarks(state, run)),
    )
}

/// The search field over the list.
fn search_field(state: BookmarkState) -> impl IntoElement {
    view().p_2().pb(px(4.0)).child(
        input()
            .value(move || state.query.get())
            .on_change(move |text: &str| state.query.set(text.to_owned()))
            .placeholder(t!("bookmark.search"))
            .label(t!("bookmark.search"))
            .h(px(30.0))
            .px_3()
            .rounded(px(8.0))
            .border_1()
            .border_color(move || theme().input)
            .focus(|s| s.border_color(move || theme().ring))
            .bg(move || theme().background)
            .text_size(px(13.0))
            .text_color(move || theme().foreground),
    )
}

/// The bookmarks the search finds, or a line saying none does.
fn bookmark_list(
    state: BookmarkState,
    canvas: ElementRef,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let found = move || state.found();
    let none_found = move || found().is_empty();
    when(none_found, move || {
        view().p_4().child(
            text(move || t!("bookmark.no_match"))
                .text_size(px(13.0))
                .text_center()
                .text_color(move || theme().muted_foreground),
        )
    })
    .otherwise(move || {
        scroll()
            .flex_1()
            .child(view().p_2().pt(px(0.0)).gap(px(2.0)).child(each(
                found,
                |bookmark: &Bookmark| bookmark.id,
                move |bookmark, _| bookmark_row(state, bookmark, canvas, run),
            )))
    })
}

/// One bookmark: its colour, title, note and tags, and the label of its page. A press selects it and goes there;
/// its context menu edits and deletes it.
fn bookmark_row(
    state: BookmarkState,
    bookmark: ReadSignal<Bookmark>,
    canvas: ElementRef,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let pages = state.pages;
    let id = bookmark.with_untracked(|bookmark| bookmark.id);
    let selected = move || state.selected.get() == Some(id);
    let title = move || bookmark.with(|bookmark| bookmark.details.title.clone());
    let note = move || bookmark.with(|bookmark| bookmark.details.note.clone());
    let tags = move || bookmark.with(|bookmark| model::tags_text(&bookmark.details.tags));
    let page = move || bookmark.with(|bookmark| pages.label(bookmark.place.page));
    let color = move || {
        let color = bookmark.with(|bookmark| GroupColor::parse(&bookmark.details.color));
        Hsla::parse_hex(color.swatch().0).expect("a built-in colour")
    };
    let go = move || {
        state.selected.set(Some(id));
        let place = bookmark.with_untracked(|bookmark| bookmark.place);
        canvas.command(
            "goToPoint",
            serde_json::json!({ "page": place.page, "x": place.x, "y": place.y, "zoom": place.zoom }),
        );
    };
    let content = button()
        .accessibility_label(move || {
            Some(SharedString::from(t!(
                "bookmark.row",
                title = title(),
                page = page()
            )))
        })
        .on_press(go)
        .flex_row()
        .items_start()
        .gap_2()
        .px_2()
        .py(px(6.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .style_when(selected, |s| s.bg(move || theme().sidebar_accent))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            // The colour's dot, level with the title's first line.
            view()
                .w(px(8.0))
                .h(px(8.0))
                .mt(px(5.0))
                .flex_shrink_0()
                .rounded_full()
                .bg(color),
        )
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text(title)
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .text_color(move || theme().sidebar_foreground),
                )
                .child(when(
                    move || !note().is_empty(),
                    move || {
                        text(note)
                            .text_size(px(12.0))
                            .truncate()
                            .text_color(move || theme().muted_foreground)
                    },
                ))
                .child(when(
                    move || !tags().is_empty(),
                    move || {
                        text(tags)
                            .text_size(px(11.0))
                            .truncate()
                            .text_color(move || theme().muted_foreground)
                    },
                )),
        )
        .child(
            text(page)
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        );
    // The menu acts on the row under the pointer.
    let item = move |action: BookmarkAction| {
        menu_entry(CommandId::Bookmark(action), move || {
            state.selected.set(Some(id));
            run(CommandId::Bookmark(action));
        })
    };
    context_menu()
        .trigger(content)
        .on_open(move || state.menu_open.set(true))
        .on_close(move || state.menu_open.set(false))
        .popup_class("w-56 p-1 rounded-lg border border-border bg-popover shadow-lg")
        .child(item(BookmarkAction::Edit))
        .child(item(BookmarkAction::Delete))
}

/// A file without bookmarks: what they are, and the first one to add.
fn no_bookmarks(
    state: BookmarkState,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let add = CommandId::Bookmark(BookmarkAction::Add);
    crate::shell::empty_message(
        || t!("panel.bookmarks"),
        move || t!("bookmark.none", keys = commands::shortcut_label(add)),
    )
    .child(when(
        move || state.loaded.with(Option::is_some),
        move || {
            view().mt(px(8.0)).child(crate::shell::primary_button(
                "bookmark.add_here",
                move || {
                    run(add);
                },
            ))
        },
    ))
}

/// The editor of the bookmark being made or edited: its title, note, tags and colour. Return in the title saves;
/// Escape or a press outside drops the change.
pub(crate) fn bookmark_dialog(state: BookmarkState) -> impl IntoElement {
    let close = move || state.draft.set(None);
    let content = match untracked(|| state.draft.get()) {
        Some(draft) => form(state, draft, close),
        None => view(),
    };
    list_dialog(t!("bookmark.dialog"), close, card(content))
}

/// The editor's fields and buttons.
fn form(state: BookmarkState, draft: Draft, close: impl Fn() + Copy + 'static) -> View {
    let is_new = matches!(draft.target, Target::New { .. });
    let title = signal(draft.details.title.clone());
    let note = signal(draft.details.note.clone());
    let tags = signal(model::tags_text(&draft.details.tags));
    let color = signal(GroupColor::parse(&draft.details.color));
    let target = draft.target;
    let first_title = draft.details.title.clone();
    let selected = u32::try_from(first_title.encode_utf16().count()).unwrap_or(u32::MAX);
    let save = move || {
        let typed = untracked(|| title.get());
        let details = Details {
            // An empty title would leave a blank row: the one the bookmark started with stays.
            title: if typed.trim().is_empty() {
                first_title.clone()
            } else {
                typed.trim().to_owned()
            },
            note: untracked(|| note.get()).trim().to_owned(),
            color: untracked(|| color.get()).id().to_owned(),
            tags: model::parse_tags(&untracked(|| tags.get())),
        };
        close();
        state.save(Draft {
            target: target.clone(),
            details,
        });
    };
    let submit = save.clone();
    let title_field = field(input().value(move || title.get()))
        .on_change(move |text: &str| title.set(text.to_owned()))
        .on_submit(move |_: &str| submit())
        .label(t!("bookmark.title"))
        .auto_focus(true)
        .selected_range((0, selected));
    let note_field = field(input().value(move || note.get()))
        .on_change(move |text: &str| note.set(text.to_owned()))
        .multiline(true)
        .min_h(px(72.0))
        .py_2()
        .placeholder(t!("bookmark.note_placeholder"))
        .label(t!("bookmark.note"));
    let tags_field = field(input().value(move || tags.get()))
        .on_change(move |text: &str| tags.set(text.to_owned()))
        .placeholder(t!("bookmark.tags_placeholder"))
        .label(t!("bookmark.tags"));
    let swatches = GroupColor::ALL
        .into_iter()
        .fold(view().flex_row().gap_2(), |row, swatch| {
            row.child(swatch_button(swatch, color))
        });
    view()
        .w(px(380.0))
        .p_4()
        .gap_3()
        .child(
            text(move || {
                t!(if is_new {
                    "bookmark.new"
                } else {
                    "bookmark.edit"
                })
            })
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().popover_foreground),
        )
        .child(title_field)
        .child(note_field)
        .child(tags_field)
        .child(caption("bookmark.color"))
        .child(swatches)
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .gap_2()
                .justify_end()
                .child(action_button("bookmark.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    if is_new {
                        "bookmark.add"
                    } else {
                        "bookmark.save"
                    },
                    save,
                )),
        )
}

/// A text field of the editor, drawn as the app's other dialogs draw theirs.
fn field(input: Input) -> Input {
    input
        .min_h(px(34.0))
        .px_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(14.0))
        .text_color(move || theme().popover_foreground)
}

/// A small label over a group of controls.
fn caption(key: &'static str) -> impl IntoElement {
    text(move || t!(key))
        .text_size(px(12.0))
        .text_color(move || theme().muted_foreground)
}

/// One colour to pick, named for screen readers; the chosen one is ringed.
fn swatch_button(swatch: GroupColor, chosen: Signal<GroupColor>) -> impl IntoElement {
    toggle()
        .pressed(move || chosen.get() == swatch)
        .accessibility_label(color_name(swatch))
        .on_change(move |_: &bool| chosen.set(swatch))
        .size(px(28.0))
        .rounded_full()
        .bg(Hsla::parse_hex(swatch.swatch().0).expect("a built-in colour"))
        .border_2()
        .border_color(move || theme().popover)
        .pressed_style(|s| s.border_color(move || theme().foreground))
        .focus_visible(|s| s.border_color(move || theme().ring))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_place_keeps_its_point_zoom_and_selected_text() {
        let data = serde_json::json!({ "page": 3, "x": 12.5, "y": 40.0, "zoom": null, "title": "a quote" });
        let (place, quote) = place(&data).expect("a place");
        assert_eq!(
            place,
            Place {
                page: 3,
                x: 12.5,
                y: 40.0,
                zoom: None
            }
        );
        assert_eq!(quote.as_deref(), Some("a quote"));
        assert!(
            super::place(&serde_json::json!({ "x": 1.0, "y": 2.0 })).is_none(),
            "no page"
        );
    }
}
