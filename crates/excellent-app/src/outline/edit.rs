//! Editing the outline (B.1–B.5): what each outline command does to the document, one step of undo each. The model
//! plans the paths (`excellent_features::outline`); this turns them into changes, and writes the open state the
//! reader gave entries with every edit (B.5 step 2).
//!
//! Every edit reads the outline from the document first, so its paths are right even while the shown outline is
//! still being read again after the last edit.

use excellent_core::edit::{Change, Operation};
use excellent_core::outline::{Wanted, find_headings};
use excellent_core::shared::pool;
use excellent_features::outline::{self as model, Entry, Restyle, Shift};
use nsh_core::JsonValue;
use nsh_core::keymap::Modifiers;
use nsh_core::prelude::*;

use super::OutlineState;

/// Something done to the outline (B.1–B.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutlineAction {
    /// A new entry opening at the reader's place, after the selected entry (B.1).
    Add,
    /// Edit the selected entry's title (B.2).
    Rename,
    /// Delete the marked entries, or the selected one, with the entries under them (B.2, B.5).
    Delete,
    /// Move the marked entries, or the selected one, one place (B.3, B.5).
    Move(Shift),
    /// The selected entry opens at the reader's place (B.4).
    SetDestination,
    /// The marked entries, or the selected one, drawn bold, italic or in a colour (B.7).
    Style(Restyle),
    /// Says which entries open nowhere (B.9).
    Check,
    /// Points each entry that opens nowhere at the heading with its title, or removes it when there is none (B.9).
    Fix,
    /// Puts the selected entry's children, or its level, in page order (B.10).
    Sort,
    /// A personal bookmark with the selected entry's title and destination; the entry stays (B.10). The shell runs it,
    /// as it needs the bookmarks.
    ToBookmark,
}

/// What the place the pages report next is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Awaiting {
    /// A new entry.
    Add,
    /// The entry of this id, repointed.
    Repoint(String),
}

/// Where the reader is, as the pages reported it: the link to it and the text selected there.
struct Place {
    page: usize,
    uri: String,
    title: Option<String>,
}

impl OutlineState {
    /// Does `action` to the shown document's outline; `canvas` is asked where the reader is for the actions that
    /// need it.
    pub(crate) fn run(self, action: OutlineAction, canvas: ElementRef) {
        if action == OutlineAction::Check {
            self.check();
            return;
        }
        if !self.editing.can_edit() {
            crate::editing::show_message(t!("outline.read_only"), t!("outline.read_only_detail"));
            return;
        }
        match action {
            OutlineAction::Add => self.ask_place(Awaiting::Add, canvas),
            OutlineAction::SetDestination => {
                if let Some(id) = self.selected_now() {
                    self.ask_place(Awaiting::Repoint(id), canvas);
                }
            }
            OutlineAction::Rename => {
                if let Some(id) = self.selected_now() {
                    self.renaming.set(Some(id));
                }
            }
            OutlineAction::Delete => self.delete(),
            OutlineAction::Move(shift) => self.shift(shift),
            OutlineAction::Style(how) => self.restyle(how),
            OutlineAction::Fix => self.fix(),
            OutlineAction::Sort => self.sort(),
            OutlineAction::Check | OutlineAction::ToBookmark => {}
        }
    }

    /// Puts the selected entry's children, or the entries on its level, in page order (B.10); one step of undo.
    fn sort(self) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let Some(level) = self
            .selected_in(&entries)
            .and_then(|id| model::sort_level(&entries, &id))
        else {
            return;
        };
        let changes: Vec<Change> = model::plan_sort(&entries, &level)
            .into_iter()
            .map(|(from, to)| Change::MoveOutlineEntry { from, to })
            .collect();
        if !changes.is_empty() && self.edit(&entries, t!("outline.op.sort"), changes) {
            self.marked.set(Vec::new());
        }
    }

    /// The selected entry's title, page and spot, for a bookmark made from it (B.10); `None` when it opens no page.
    pub(crate) fn selected_place(self) -> Option<(String, usize, model::Spot)> {
        let entries = self.read_now()?;
        let id = self.selected_in(&entries)?;
        let entry = model::find(&entries, &id)?;
        let page = entry.page?;
        Some((entry.title.clone(), page, model::spot(entry.uri.as_deref())))
    }

    /// A new entry titled `title` opening at `spot` on page `page`, from a personal bookmark (B.10): after the entry
    /// that page is in, selected.
    pub(crate) fn add_from_bookmark(self, title: &str, page: usize, spot: model::Spot) {
        if !self.editing.can_edit() {
            crate::editing::show_message(t!("outline.read_only"), t!("outline.read_only_detail"));
            return;
        }
        let Some(entries) = self.read_now() else {
            return;
        };
        let Some(document) = self.editing.document() else {
            return;
        };
        let uri = match document.link_uri(page, spot.x, spot.y, spot.zoom) {
            Ok(uri) => uri,
            Err(err) => {
                log::warn!("outline: no link to page {page}: {err}");
                return;
            }
        };
        let after = model::reading(&entries, None, page);
        let at = model::insert_at(&entries, after.as_deref());
        let change = Change::InsertOutlineEntry {
            at: at.clone(),
            title: title.to_owned(),
            uri,
        };
        if self.edit(&entries, t!("outline.op.add"), vec![change]) {
            self.marked.set(Vec::new());
            self.chosen.set(Some(model::id(&at)));
        }
    }

    /// Says which entries open nowhere (B.9); the rows mark them all the time, this counts them. Reading needs no
    /// editing, so it runs on any document.
    pub(crate) fn check(self) {
        let entries = untracked(|| self.entries.get()).unwrap_or_default();
        let broken = model::check(&entries, untracked(|| self.pages.count()));
        if broken.is_empty() {
            crate::editing::show_message(
                t!("outline.check.ok_title"),
                t!("outline.check.ok", count = model::count(&entries)),
            );
        } else {
            crate::editing::show_message(
                t!("outline.check.found_title", count = broken.len()),
                t!("outline.check.found"),
            );
        }
    }

    /// Looks on the workers for the heading of each entry that opens nowhere, then points the entries whose heading
    /// is found at it and removes the rest, as one step of undo (B.9).
    fn fix(self) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let count = untracked(|| self.pages.count());
        let broken = model::check(&entries, count);
        if broken.is_empty() {
            self.check();
            return;
        }
        let Some(mupdf) = self.editing.document().and_then(|document| {
            document
                .mupdf()
                .inspect_err(|err| log::warn!("outline: {err}"))
                .ok()
        }) else {
            return;
        };
        let wanted: Vec<Wanted> = broken
            .iter()
            .filter_map(|(id, _)| {
                Some(Wanted {
                    title: model::find(&entries, id)?.title.clone(),
                    from: model::search_from(&entries, id, count),
                })
            })
            .collect();
        let job = find_headings(pool(), mupdf, wanted, model::heading_top);
        let ids: Vec<String> = broken.into_iter().map(|(id, _)| id).collect();
        let task = spawn(async move {
            match job.await {
                Ok(found) => self.fixed(&entries, &ids, &found),
                Err(err) if !err.is_aborted() => log::warn!("outline: looking for headings: {err}"),
                Err(_) => {}
            }
        });
        self.fixing.with(|fixing| *fixing = Some(task));
    }

    /// The headings of the entries of ids `ids` were looked for in the outline `before`: `found` holds each one's page
    /// and top. Makes the changes if the outline is still as it was.
    fn fixed(self, before: &[Entry], ids: &[String], found: &[Option<(usize, f32)>]) {
        let Some(entries) = self.read_now() else {
            return;
        };
        if entries != before {
            log::info!("outline: changed while its headings were looked for; nothing fixed");
            return;
        }
        let Some(document) = self.editing.document() else {
            return;
        };
        let mut changes = Vec::new();
        let mut gone = Vec::new();
        for (id, place) in ids.iter().zip(found) {
            let uri = place.and_then(|(page, top)| {
                document
                    .link_uri(page, 0.0, top, None)
                    .inspect_err(|err| log::warn!("outline: no link to page {page}: {err}"))
                    .ok()
            });
            match (uri, model::path(id)) {
                (Some(uri), Some(at)) => changes.push(Change::UpdateOutlineEntry {
                    at,
                    title: None,
                    uri: Some(uri),
                    open: self.open_state(&entries, id),
                }),
                (None, _) => gone.push(id.clone()),
                (Some(_), None) => {}
            }
        }
        let repointed = changes.len();
        // Repoints change no paths, so the removals' paths are planned on the outline as it is.
        changes.extend(
            model::plan_delete(&entries, &gone)
                .into_iter()
                .map(|at| Change::DeleteOutlineEntry { at }),
        );
        if changes.is_empty() || !self.edit(&entries, t!("outline.op.fix"), changes) {
            return;
        }
        self.marked.set(Vec::new());
        if !gone.is_empty() {
            self.chosen.set(None);
        }
        crate::editing::show_message(
            t!("outline.fix.done_title"),
            t!(
                "outline.fix.done",
                repointed = repointed,
                removed = gone.len()
            ),
        );
    }

    /// The pages reported where the reader is (`readerPlace`): the action waiting for it goes on.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name != "readerPlace" {
            return;
        }
        let Some(awaiting) = self.awaiting.replace(None) else {
            return;
        };
        let Some(place) = self.place(&event.data) else {
            log::warn!("outline: the pages reported no place: {}", event.data);
            return;
        };
        match awaiting {
            Awaiting::Add => self.add(place),
            Awaiting::Repoint(id) => self.repoint(&id, place.uri),
        }
    }

    /// A press on the row of entry `id` with `modifiers` held (B.5): Ctrl (⌘ on a Mac) marks or unmarks it, Shift
    /// marks the entries shown from the last one pressed to it, and a plain press selects it and goes to its page,
    /// opening or closing it when it has entries under it, as the tree's own press does.
    pub(super) fn pressed(self, id: &str, folder: bool, modifiers: Modifiers, go: impl Fn(usize)) {
        if modifiers.ctrl || modifiers.cmd {
            self.marked.update(|marked| {
                if marked.is_empty()
                    && let Some(selected) = self.selected_shown()
                {
                    // The selected entry is the first of a group the reader starts with Ctrl.
                    marked.push(selected);
                }
                match marked.iter().position(|marked| marked == id) {
                    Some(at) => {
                        marked.remove(at);
                    }
                    None => marked.push(id.to_owned()),
                }
            });
            self.anchor.replace(Some(id.to_owned()));
            return;
        }
        if modifiers.shift {
            let from = self
                .anchor
                .with(|anchor| anchor.clone())
                .or_else(|| self.selected_shown());
            let shown = self.entries.with(|entries| {
                let entries = entries.as_deref().unwrap_or_default();
                self.expanded
                    .with(|expanded| model::shown_ids(entries, expanded))
            });
            let at = |id: &str| shown.iter().position(|shown| shown == id);
            if let (Some(start), Some(end)) = (from.as_deref().and_then(at), at(id)) {
                let (first, last) = (start.min(end), start.max(end));
                self.marked.set(shown[first..=last].to_vec());
            }
            return;
        }
        self.marked.set(Vec::new());
        self.anchor.replace(Some(id.to_owned()));
        self.chosen.set(Some(id.to_owned()));
        if let Some(page) = self.page_of(id) {
            go(page);
        }
        if folder {
            let id = SharedString::from(id.to_owned());
            self.expanded.update(
                |expanded| match expanded.iter().position(|open| *open == id) {
                    Some(at) => {
                        expanded.remove(at);
                    }
                    None => expanded.push(id),
                },
            );
        }
    }

    /// The reader typed `title` for the entry of id `id` (B.2): kept unless it is empty or unchanged.
    pub(super) fn rename_to(self, id: &str, title: &str) {
        let title = title.trim();
        let Some(entries) = self.read_now() else {
            return;
        };
        let Some((at, entry)) = model::path(id).zip(model::find(&entries, id)) else {
            return;
        };
        if title.is_empty() || entry.title == title {
            return;
        }
        let change = Change::UpdateOutlineEntry {
            at,
            title: Some(title.to_owned()),
            uri: None,
            open: self.open_state(&entries, id),
        };
        self.edit(&entries, t!("outline.op.rename"), vec![change]);
    }

    /// Asks the pages where the reader is; the answer comes as `readerPlace`.
    fn ask_place(self, awaiting: Awaiting, canvas: ElementRef) {
        self.awaiting.replace(Some(awaiting));
        canvas.command("readerPlace", JsonValue::Null);
    }

    /// The place in an `readerPlace` event, with the link to it.
    fn place(self, data: &JsonValue) -> Option<Place> {
        let page = usize::try_from(data.get("page")?.as_u64()?).ok()?;
        let number = |key: &str| data.get(key).and_then(JsonValue::as_f64).map(|n| n as f32);
        let document = self.editing.document()?;
        let uri = document
            .link_uri(page, number("x")?, number("y")?, number("zoom"))
            .inspect_err(|err| log::warn!("outline: no link to page {page}: {err}"))
            .ok()?;
        let title = data
            .get("title")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        Some(Place { page, uri, title })
    }

    /// A new entry at `place`, after the selected entry (B.1); titled with the selected text, or with the page and
    /// ready for the reader to type over.
    fn add(self, place: Place) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let after = self.selected_in(&entries);
        let at = model::insert_at(&entries, after.as_deref());
        let typed = place.title.as_deref().and_then(model::title_from_selection);
        let title = typed
            .clone()
            .unwrap_or_else(|| t!("outline.page_entry", page = self.pages.label(place.page)));
        let change = Change::InsertOutlineEntry {
            at: at.clone(),
            title,
            uri: place.uri,
        };
        if self.edit(&entries, t!("outline.op.add"), vec![change]) {
            let id = model::id(&at);
            self.marked.set(Vec::new());
            self.chosen.set(Some(id.clone()));
            if typed.is_none() && untracked(|| self.shown.get()) {
                self.renaming.set(Some(id));
            }
        }
    }

    /// The entry of id `id` opens at `uri` (B.4).
    fn repoint(self, id: &str, uri: String) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let Some(at) = model::find(&entries, id).and(model::path(id)) else {
            return;
        };
        let change = Change::UpdateOutlineEntry {
            at,
            title: None,
            uri: Some(uri),
            open: self.open_state(&entries, id),
        };
        self.edit(&entries, t!("outline.op.set_destination"), vec![change]);
    }

    /// Deletes the marked entries, or the selected one, with the entries under them (B.2, B.5).
    fn delete(self) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let targets = self.targets_in(&entries);
        let changes: Vec<Change> = model::plan_delete(&entries, &targets)
            .into_iter()
            .map(|at| Change::DeleteOutlineEntry { at })
            .collect();
        if !changes.is_empty() && self.edit(&entries, t!("outline.op.delete"), changes) {
            self.marked.set(Vec::new());
            self.chosen.set(None);
        }
    }

    /// Draws the marked entries, or the selected one, as `how` says (B.7); one step of undo.
    fn restyle(self, how: Restyle) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let targets = self.targets_in(&entries);
        let changes: Vec<Change> = model::plan_style(&entries, &targets, how)
            .into_iter()
            .map(|(at, style)| Change::StyleOutlineEntry { at, style })
            .collect();
        if !changes.is_empty() {
            self.edit(&entries, t!("outline.op.style"), changes);
        }
    }

    /// Moves the marked entries, or the selected one, one place `shift` (B.3, B.5); they stay selected.
    fn shift(self, shift: Shift) {
        let Some(entries) = self.read_now() else {
            return;
        };
        let targets = self.targets_in(&entries);
        let moves = model::plan_shift(&entries, &targets, shift);
        if moves.steps.is_empty() {
            return;
        }
        let mut changes: Vec<Change> = moves
            .steps
            .iter()
            .map(|(from, to)| Change::MoveOutlineEntry {
                from: from.clone(),
                to: to.clone(),
            })
            .collect();
        if shift == Shift::In {
            // The entries they went under open, so they still show.
            let mut parents: Vec<Vec<usize>> = moves
                .moved
                .iter()
                .filter_map(|id| {
                    let mut path = model::path(id)?;
                    path.pop();
                    Some(path)
                })
                .collect();
            parents.dedup();
            changes.extend(parents.into_iter().map(|at| Change::UpdateOutlineEntry {
                at,
                title: None,
                uri: None,
                open: true,
            }));
        }
        let several = untracked(|| self.marked.with(|marked| !marked.is_empty()));
        if self.edit(&entries, t!("outline.op.move"), changes) {
            if several {
                self.marked.set(moves.moved);
            } else {
                self.chosen.set(moves.moved.into_iter().next());
            }
        }
    }

    /// Makes `changes` to the outline `entries` (as the document has it now) as one operation named `name`, the
    /// reader's open states written first, and shows the outline it makes. Returns whether it was made.
    fn edit(self, entries: &[Entry], name: String, changes: Vec<Change>) -> bool {
        let open = untracked(|| {
            self.expanded
                .with(|expanded| model::open_changes(entries, expanded))
        });
        let mut all: Vec<Change> = open
            .into_iter()
            .map(|(at, open)| Change::UpdateOutlineEntry {
                at,
                title: None,
                uri: None,
                open,
            })
            .collect();
        all.extend(changes);
        if !self.editing.apply(&Operation::new(name, all)) {
            return false;
        }
        if let Some(entries) = self.read_now() {
            self.show(entries);
        }
        true
    }

    /// The outline as the document has it now.
    // shortcut: read on the UI thread, waiting for a page being drawn; move it to the workers if a long outline
    // makes edits stall (K.24's 10,000-entry check).
    fn read_now(self) -> Option<Vec<Entry>> {
        let document = self.editing.document()?;
        let mupdf = document
            .mupdf()
            .inspect_err(|err| log::warn!("outline: {err}"))
            .ok()?;
        let outline = mupdf.lock().expect("document lock poisoned").outline();
        outline
            .inspect_err(|err| log::warn!("outline: {err}"))
            .ok()
            .map(|items| items.iter().map(super::entry).collect())
    }

    /// The open state to write for the entry of id `id`: the reader's when it has entries under it, else its own.
    fn open_state(self, entries: &[Entry], id: &str) -> bool {
        match model::find(entries, id) {
            Some(entry) if entry.children.is_empty() => entry.open,
            Some(_) => self.is_expanded(id),
            None => false,
        }
    }

    fn is_expanded(self, id: &str) -> bool {
        untracked(|| {
            self.expanded
                .with(|expanded| expanded.iter().any(|open| open.as_ref() == id))
        })
    }

    /// The selected entry in the outline as the document has it now.
    fn selected_in(self, entries: &[Entry]) -> Option<String> {
        let page = untracked(|| self.pages.current());
        let chosen = untracked(|| self.chosen.get());
        model::reading(entries, chosen.as_deref(), page)
    }

    /// The selected entry, read from the document now.
    fn selected_now(self) -> Option<String> {
        self.selected_in(&self.read_now()?)
    }

    /// The selected entry of the shown outline.
    fn selected_shown(self) -> Option<String> {
        untracked(|| self.reading_entry(self.pages.current())).map(|id| id.to_string())
    }

    /// What an action on several entries acts on: the marked ones, else the selected one.
    fn targets_in(self, entries: &[Entry]) -> Vec<String> {
        let marked = untracked(|| self.marked.get());
        if marked.is_empty() {
            self.selected_in(entries).into_iter().collect()
        } else {
            marked
        }
    }
}
