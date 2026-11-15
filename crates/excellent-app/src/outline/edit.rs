//! Editing the outline (B.1–B.5): what each outline command does to the document, one step of undo each. The model
//! plans the paths (`excellent_features::outline`); this turns them into changes, and writes the open state the
//! reader gave entries with every edit (B.5 step 2).
//!
//! Every edit reads the outline from the document first, so its paths are right even while the shown outline is
//! still being read again after the last edit.

use excellent_core::edit::{Change, Operation};
use excellent_features::outline::{self as model, Entry, Shift};
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
        }
    }

    /// The pages reported where the reader is (`outlineTarget`): the action waiting for it goes on.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        if &*event.name != "outlineTarget" {
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

    /// Asks the pages where the reader is; the answer comes as `outlineTarget`.
    fn ask_place(self, awaiting: Awaiting, canvas: ElementRef) {
        self.awaiting.replace(Some(awaiting));
        canvas.command("outlineTarget", JsonValue::Null);
    }

    /// The place in an `outlineTarget` event, with the link to it.
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
