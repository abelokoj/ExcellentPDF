//! Markdown notes with `[[wiki-links]]`, backlink graph, and FTS5 indexing (plan F.12).

pub mod export;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use excellent_store::fts::{self, FtsEntryInput, SourceKind};
use excellent_store::notes::{
    self, LinkTarget, Note, NoteLink, extract_wiki_links, parse_link_target,
};
use excellent_store::{Result as StoreResult, Store};

pub use excellent_store::notes::{Note as NoteRecord, NoteLink as NoteLinkRecord};
pub use export::{
    DeepLink, HighlightExport, ObsidianVaultReport, export_document_highlights_markdown,
    export_note_markdown, export_obsidian_vault,
};

/// Resolved graph over all notes in the library (plan F.12).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BacklinkGraph {
    /// Outgoing links keyed by source note id.
    pub outgoing: BTreeMap<String, Vec<NoteLink>>,
    /// Incoming backlinks keyed by target note id.
    pub incoming: BTreeMap<String, Vec<NoteLink>>,
    /// Links whose target note or file does not yet exist.
    pub unresolved: Vec<NoteLink>,
    /// Note ids that have neither incoming nor outgoing links.
    pub orphans: Vec<String>,
}

/// High-level Markdown note manager over [`Store`] (plan F.12).
#[derive(Debug)]
pub struct Notes<'a> {
    store: &'a Store,
}

impl<'a> Notes<'a> {
    /// Wraps `store` as a [`Notes`] view.
    #[must_use]
    pub const fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Saves `note`, updates the `[[wiki-link]]` graph, and indexes its body in SQLite FTS5.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn save(&self, note: &Note) -> StoreResult<()> {
        let note = note.clone();
        let fts_entry = FtsEntryInput {
            path: note.file.clone(),
            source_kind: SourceKind::Note,
            source_id: note.id.clone(),
            page: note.page.unwrap_or(0),
            author: String::new(),
            color: String::new(),
            tags: note.tags.clone(),
            created_at: note.modified_at,
            quads_json: "[]".to_owned(),
            title: note.title.clone(),
            body: note.body.clone(),
        };
        self.store
            .write(move |tx| {
                notes::upsert_note(tx, &note)?;
                fts::delete_by_source(tx, SourceKind::Note, &note.id)?;
                fts::insert_entry(tx, &fts_entry)?;
                Ok(())
            })
            .wait()
    }

    /// Renames `note_id` to `new_title` and updates every `[[link]]` pointing to it across all notes
    /// (plan F.12), refreshing FTS5 entries for all updated notes.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn rename(&self, note_id: &str, new_title: &str, modified_at: i64) -> StoreResult<usize> {
        let note_id = note_id.to_owned();
        let new_title = new_title.to_owned();
        self.store
            .write(move |tx| {
                let updated = notes::rename_note(tx, &note_id, &new_title, modified_at)?;
                // Refresh FTS entries for all notes so renamed titles/bodies stay searchable.
                for note in notes::list_notes(tx)? {
                    fts::delete_by_source(tx, SourceKind::Note, &note.id)?;
                    fts::insert_entry(
                        tx,
                        &FtsEntryInput {
                            path: note.file,
                            source_kind: SourceKind::Note,
                            source_id: note.id,
                            page: note.page.unwrap_or(0),
                            author: String::new(),
                            color: String::new(),
                            tags: note.tags,
                            created_at: note.modified_at,
                            quads_json: "[]".to_owned(),
                            title: note.title,
                            body: note.body,
                        },
                    )?;
                }
                Ok(updated)
            })
            .wait()
    }

    /// Deletes a note and its FTS5 entry.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn delete(&self, note_id: &str) -> StoreResult<bool> {
        let note_id = note_id.to_owned();
        self.store
            .write(move |tx| {
                fts::delete_by_source(tx, SourceKind::Note, &note_id)?;
                notes::delete_note(tx, &note_id)
            })
            .wait()
    }

    /// Lists all notes.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn list(&self) -> StoreResult<Vec<Note>> {
        self.store.read(notes::list_notes)
    }

    /// Lists notes anchored to `path`.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn for_file(&self, path: &Path) -> StoreResult<Vec<Note>> {
        self.store.read(|conn| notes::notes_for_file(conn, path))
    }

    /// Returns incoming backlinks for `note_id`.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn backlinks(&self, note_id: &str) -> StoreResult<Vec<NoteLink>> {
        self.store
            .read(|conn| notes::backlinks_for_note(conn, note_id))
    }

    /// Builds the complete [`BacklinkGraph`] across all stored notes.
    ///
    /// # Errors
    ///
    /// SQLite failed.
    pub fn graph(&self) -> StoreResult<BacklinkGraph> {
        self.store.read(|conn| {
            let all_notes = notes::list_notes(conn)?;
            let mut graph = BacklinkGraph::default();
            let mut connected = BTreeSet::new();

            for note in &all_notes {
                let outgoing = notes::outgoing_links(conn, &note.id)?;
                if !outgoing.is_empty() {
                    connected.insert(note.id.clone());
                }
                for link in &outgoing {
                    if let Some(target_id) = &link.target_note {
                        connected.insert(target_id.clone());
                        graph
                            .incoming
                            .entry(target_id.clone())
                            .or_default()
                            .push(link.clone());
                    } else if link.target_file.is_none() {
                        graph.unresolved.push(link.clone());
                    }
                }
                graph.outgoing.insert(note.id.clone(), outgoing);
            }

            for note in all_notes {
                if !connected.contains(&note.id) {
                    graph.orphans.push(note.id);
                }
            }
            Ok(graph)
        })
    }
}

/// Extracts parsed [`LinkTarget`]s from a Markdown string without touching SQLite.
#[must_use]
pub fn parse_links_in_markdown(body: &str) -> Vec<LinkTarget> {
    extract_wiki_links(body)
        .into_iter()
        .map(|raw| parse_link_target(&raw))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("excellent-notes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn renaming_note_updates_links_and_fts_index() {
        let dir = temp_dir("rename");
        let store = Store::open(&dir.join("library.db")).expect("open");
        let manager = Notes::new(&store);

        manager
            .save(&Note {
                id: "n1".to_owned(),
                title: "Zettelkasten".to_owned(),
                body: "Builds on [[Evergreen Notes]] and [[paper.pdf#page=4]].".to_owned(),
                file: None,
                page: None,
                annotation_id: None,
                tags: vec!["pkm".to_owned()],
                created_at: 10,
                modified_at: 10,
            })
            .expect("save n1");
        manager
            .save(&Note {
                id: "n2".to_owned(),
                title: "Evergreen Notes".to_owned(),
                body: "See [[Zettelkasten]] and [[Unwritten Note]].".to_owned(),
                file: None,
                page: None,
                annotation_id: None,
                tags: Vec::new(),
                created_at: 11,
                modified_at: 11,
            })
            .expect("save n2");

        let graph = manager.graph().expect("graph");
        assert_eq!(graph.incoming["n2"].len(), 1);
        assert_eq!(graph.unresolved.len(), 2); // paper.pdf#page=4 (unregistered file) + Unwritten Note

        let updated = manager.rename("n2", "Atomic Notes", 20).expect("rename");
        assert_eq!(updated, 1);

        let hits = store
            .read(|conn| {
                fts::search_fts(conn, "\"Atomic Notes\"", &fts::SearchFilters::default(), 10)
            })
            .expect("fts");
        assert_eq!(hits.len(), 2, "both note title and updated link match FTS");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
