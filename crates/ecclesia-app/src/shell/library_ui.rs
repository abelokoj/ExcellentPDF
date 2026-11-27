//! Library, Global FTS5 Search, Saved Searches, Collections, Citations/Zotero, Markdown Notes,
//! Workspace Templates/Rules, and CLI/MCP Docs UI (plans F.2–F.13, W.10, W.12, E.1–E.4).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_core::anchor::new_id;
use ecclesia_features::citations::zotero::{LOCAL_API_BASE, ZoteroClient, ZoteroSource};
use ecclesia_features::citations::{
    CitationEntry, CitationStyle, export_bibtex, export_csl_json, export_ris, format_citation,
    generate_citation_key, parse_bibtex, parse_csl_json, parse_ris,
};
use ecclesia_features::library::metadata::{
    DuplicateKind, OnlineConsent, extract_from_pdf, fetch_arxiv_metadata, fetch_crossref_metadata,
};
use ecclesia_features::library::{Library, MetadataSource, WatchDepth};
use ecclesia_features::notes::export::{DeepLink, export_note_markdown, export_obsidian_vault};
use ecclesia_features::notes::{Notes, parse_links_in_markdown};
use ecclesia_features::search::global::{IndexStats, index_file_if_needed, search_indexed};
use ecclesia_features::search::query::ParsedQuery;
use ecclesia_features::workspaces_rules::{
    RoutingRule, WorkspaceTemplate, delete_rule, instantiate_template, list_rules, list_templates,
    open_collection_as_workspace, save_rule, save_template, save_workspace_as_collection,
};
use ecclesia_store::fts::{self, FtsHit, SavedSearch, SearchFilters};
use ecclesia_store::library::{
    self as lib_store, Collection, LibraryFilter, LibraryItem, ReadStatus,
};
use ecclesia_store::notes::Note;
use ecclesia_store::workspace::WorkspaceId;
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use crate::bookmarks::{caption, field};
use crate::library as app_library;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, write_atomically};
use crate::settings_window::{segment_of, segment_owned, segmented};
use crate::shell::control_channel;
use crate::tabs::{Tone, action_button};

/// `settings.toml` key gating online DOI / arXiv / Zotero requests behind explicit user consent (F.7).
pub(crate) const ONLINE_METADATA_CONSENT: &str = "online_metadata_consent";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

const DLG_LIBRARY: u8 = 1 << 0;
const DLG_DUPLICATES: u8 = 1 << 1;
const DLG_CITATIONS: u8 = 1 << 2;
const DLG_NOTES: u8 = 1 << 3;
const DLG_RULES: u8 = 1 << 4;
const DLG_CLI_DOCS: u8 = 1 << 5;

/// Shared UI state for Library, Citations, Notes, Workspace Rules, and CLI/MCP dialogs.
/// Uses a single bitmask signal so cold start allocates one reactive node instead of six,
/// and no SQLite queries or dialog state are built until a dialog actually opens.
#[derive(Clone, Copy)]
pub(crate) struct LibraryUiState {
    open_mask: Signal<u8>,
}

impl LibraryUiState {
    /// Creates lightweight dialog visibility state without opening databases or allocating views.
    pub(crate) fn new() -> Self {
        Self {
            open_mask: signal(0),
        }
    }

    fn is_open(self, bit: u8) -> bool {
        (self.open_mask.get() & bit) != 0
    }

    fn set_open(self, bit: u8, open: bool) {
        self.open_mask.update(|mask| {
            if open {
                *mask |= bit;
            } else {
                *mask &= !bit;
            }
        });
    }

    /// True while any Library/Citations/Notes/Rules/CLI-MCP dialog covers the native page canvas.
    pub(crate) fn any_dialog_open(self) -> bool {
        self.open_mask.get() != 0
    }

    /// True while the Library & FTS5 search dialog is open.
    pub(crate) fn is_library_open(self) -> bool {
        self.is_open(DLG_LIBRARY)
    }
    /// True while the Duplicate Finder dialog is open.
    pub(crate) fn is_duplicates_open(self) -> bool {
        self.is_open(DLG_DUPLICATES)
    }
    /// True while the Citations & Zotero dialog is open.
    pub(crate) fn is_citations_open(self) -> bool {
        self.is_open(DLG_CITATIONS)
    }
    /// True while the Markdown Notes & Backlinks dialog is open.
    pub(crate) fn is_notes_open(self) -> bool {
        self.is_open(DLG_NOTES)
    }
    /// True while the Workspace Templates & Rules dialog is open.
    pub(crate) fn is_rules_open(self) -> bool {
        self.is_open(DLG_RULES)
    }
    /// True while the CLI / MCP documentation dialog is open.
    pub(crate) fn is_cli_docs_open(self) -> bool {
        self.is_open(DLG_CLI_DOCS)
    }

    /// Opens the Library & FTS5 search dialog.
    pub(crate) fn open_library(self) {
        self.set_open(DLG_LIBRARY, true);
    }
    /// Opens the Duplicate Finder dialog.
    pub(crate) fn open_duplicates(self) {
        self.set_open(DLG_DUPLICATES, true);
    }
    /// Opens the Citations & Zotero dialog.
    pub(crate) fn open_citations(self) {
        self.set_open(DLG_CITATIONS, true);
    }
    /// Opens the Markdown Notes & Backlinks dialog.
    pub(crate) fn open_notes(self) {
        self.set_open(DLG_NOTES, true);
    }
    /// Opens the Workspace Templates & Rules dialog.
    pub(crate) fn open_rules(self) {
        self.set_open(DLG_RULES, true);
    }
    /// Opens the CLI / MCP documentation dialog.
    pub(crate) fn open_cli_docs(self) {
        self.set_open(DLG_CLI_DOCS, true);
    }
}

/// Library, FTS5 Global Search, Boolean Operators, Saved Searches, Collections & Metadata dialog
/// (F.2–F.7, F.9, W.10).
pub(crate) fn library_dialog(
    state: LibraryUiState,
    active_doc: Option<PathBuf>,
    active_ws: WorkspaceId,
    open_at: impl Fn(PathBuf, usize) + Copy + 'static,
    open_ws: impl Fn(WorkspaceId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.set_open(DLG_LIBRARY, false);
    let query = signal(String::new());
    let save_search_name = signal(String::new());
    let new_col_name = signal(String::new());
    let doi_or_arxiv = signal(String::new());
    let tags_draft = signal(String::new());
    let selected_col = signal(None::<i64>);
    let status_msg = signal(String::new());
    let hits = signal(Vec::<FtsHit>::new());
    let items = signal(Vec::<LibraryItem>::new());
    let collections = signal(Vec::<Collection>::new());
    let saved_searches = signal(Vec::<SavedSearch>::new());
    let consent_on = signal(
        settings::shared()
            .flag(ONLINE_METADATA_CONSENT)
            .unwrap_or(false),
    );
    let active_doc_clone = active_doc.clone();

    let refresh = move || {
        let Some(store) = app_library::store() else {
            return;
        };
        let lib = Library::new(store);
        let filter = LibraryFilter {
            collection_id: untracked(|| selected_col.get()),
            ..LibraryFilter::default()
        };
        if let Ok(list) = lib.list(&filter) {
            items.set(list);
        }
        if let Ok(cols) = store.read(lib_store::list_collections) {
            collections.set(cols);
        }
        if let Ok(searches) = store.read(fts::list_searches) {
            saved_searches.set(searches);
        }
    };
    refresh();

    let run_search = move || {
        let q = untracked(|| query.get());
        let trimmed = q.trim();
        if trimmed.is_empty() {
            hits.set(Vec::new());
            status_msg.set(String::new());
            return;
        }
        if let Err(err) = ParsedQuery::parse(trimmed) {
            status_msg.set(err.to_string());
            return;
        }
        let Some(store) = app_library::store() else {
            return;
        };
        let mut extra = SearchFilters::default();
        if let Some(cid) = untracked(|| selected_col.get()) {
            extra.collection_id = Some(cid);
        }
        match store.read(|conn| {
            search_indexed(conn, trimmed, &extra, 60)
                .map_err(|e| ecclesia_store::Error::Io(std::io::Error::other(e.to_string())))
        }) {
            Ok(found) => {
                status_msg.set(t!("library.hits_count", count = found.len()));
                hits.set(found);
            }
            Err(err) => status_msg.set(err.to_string()),
        }
    };

    let add_active_doc = {
        let doc = active_doc.clone();
        move || {
            let Some(path) = doc.clone() else {
                return;
            };
            let Some(store) = app_library::store() else {
                return;
            };
            let lib = Library::new(store);
            let mut stats = IndexStats::default();
            let _ = lib.add_document(&path, MetadataSource::FromDisk, now_secs());
            let _ = index_file_if_needed(store, &path, &mut stats);
            if let Some(cid) = untracked(|| selected_col.get()) {
                let _ = lib.add_to_collection(cid, &path);
            }
            let tags_str = untracked(|| tags_draft.get());
            let tag_vec: Vec<String> = tags_str
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if !tag_vec.is_empty() {
                let _ = lib.set_tags(&path, tag_vec);
            }
            status_msg.set(t!("library.indexed_active"));
            refresh();
        }
    };

    let index_folder_action = move || {
        let dialog = OpenDialog::folders().title(t!("library.index_folder"));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(folder) = paths.into_iter().next()
                && let Some(store) = app_library::store()
            {
                let lib = Library::new(store);
                let _ = lib.watch_folder(&folder, WatchDepth::Recursive);
                if let Ok(report) = lib.reconcile_watched() {
                    status_msg.set(t!(
                        "library.folder_watched",
                        added = report.added,
                        updated = report.modified
                    ));
                }
                refresh();
            }
        });
    };

    let create_collection_action = move || {
        let name = untracked(|| new_col_name.get());
        if name.trim().is_empty() {
            return;
        }
        let Some(store) = app_library::store() else {
            return;
        };
        let lib = Library::new(store);
        if lib
            .create_collection(untracked(|| selected_col.get()), name.trim(), None, None)
            .is_ok()
        {
            new_col_name.set(String::new());
            refresh();
        }
    };

    let save_search_action = move || {
        let name = untracked(|| save_search_name.get());
        let q = untracked(|| query.get());
        if name.trim().is_empty() || q.trim().is_empty() {
            return;
        }
        let Some(store) = app_library::store() else {
            return;
        };
        let name_owned = name.trim().to_owned();
        let q_owned = q.trim().to_owned();
        if store
            .write(move |tx| fts::save_search(tx, &name_owned, &q_owned, now_secs()))
            .wait()
            .is_ok()
        {
            save_search_name.set(String::new());
            refresh();
        }
    };

    let open_collection_ws = move || {
        let Some(cid) = untracked(|| selected_col.get()) else {
            return;
        };
        let Some(store) = app_library::store() else {
            return;
        };
        if let Ok(Some(ws_id)) = open_collection_as_workspace(store, cid, None) {
            close();
            open_ws(ws_id);
        }
    };

    let save_ws_as_col = move || {
        let Some(store) = app_library::store() else {
            return;
        };
        if let Ok(Some(cid)) = save_workspace_as_collection(store, active_ws, None, None) {
            selected_col.set(Some(cid));
            status_msg.set(t!("library.saved_workspace"));
            refresh();
        }
    };

    let fetch_meta_action = {
        let doc = active_doc.clone();
        move || {
            let ident = untracked(|| doi_or_arxiv.get()).trim().to_owned();
            let allowed = untracked(|| consent_on.get());
            let consent = if allowed {
                OnlineConsent::Allowed
            } else {
                OnlineConsent::Denied
            };
            if ident.is_empty() {
                if let Some(path) = doc.as_ref()
                    && let Ok(meta) = extract_from_pdf(path)
                {
                    let summary = meta
                        .title
                        .or(meta.doi)
                        .or(meta.arxiv_id)
                        .unwrap_or_default();
                    status_msg.set(summary);
                }
                return;
            }
            let res = if ident.contains('/') {
                fetch_crossref_metadata(&ident, consent)
            } else {
                fetch_arxiv_metadata(&ident, consent)
            };
            match res {
                Ok(meta) => {
                    let title = meta.title.unwrap_or_else(|| ident.clone());
                    status_msg.set(title);
                }
                Err(err) => status_msg.set(err.to_string()),
            }
        }
    };

    let body = view()
        .w(px(720.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("library.title"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button(
                    "library.index_folder",
                    Tone::Plain,
                    index_folder_action,
                ))
                .child(action_button(
                    "library.save_ws_collection",
                    Tone::Plain,
                    save_ws_as_col,
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    field(input().value(move || query.get()))
                        .flex_1()
                        .on_change(move |s: &str| query.set(s.to_owned()))
                        .placeholder(t!("library.search.placeholder"))
                        .label(t!("library.search.label"))
                        .auto_focus(true),
                )
                .child(crate::shell::primary_button(
                    "library.search.run",
                    run_search,
                )),
        )
        .child(
            text(move || t!("library.operators.hint"))
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    field(input().value(move || save_search_name.get()))
                        .flex_1()
                        .on_change(move |s: &str| save_search_name.set(s.to_owned()))
                        .placeholder(t!("library.saved.placeholder"))
                        .label(t!("library.saved.placeholder")),
                )
                .child(action_button(
                    "library.saved.save",
                    Tone::Plain,
                    save_search_action,
                ))
                .child(
                    field(input().value(move || new_col_name.get()))
                        .flex_1()
                        .on_change(move |s: &str| new_col_name.set(s.to_owned()))
                        .placeholder(t!("library.collection.placeholder"))
                        .label(t!("library.collection.placeholder")),
                )
                .child(action_button(
                    "library.collection.create",
                    Tone::Plain,
                    create_collection_action,
                )),
        )
        .child(dynamic(move || {
            let cols = collections.get();
            let cur = selected_col.get();
            let mut bar = segmented().child(segment_of(
                move || t!("library.collection.all"),
                move || cur.is_none(),
                move || {
                    selected_col.set(None);
                    refresh();
                },
            ));
            for col in cols {
                let cid = col.id;
                let cname = col.name.clone();
                bar = bar.child(segment_owned(
                    cname,
                    move || cur == Some(cid),
                    move || {
                        selected_col.set(Some(cid));
                        refresh();
                    },
                ));
            }
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(bar)
                .child(when(
                    move || selected_col.get().is_some(),
                    move || {
                        action_button(
                            "library.open_collection_ws",
                            Tone::Plain,
                            open_collection_ws,
                        )
                    },
                ))
        }))
        .child(dynamic(move || {
            let list = saved_searches.get();
            if list.is_empty() {
                return view();
            }
            list.into_iter()
                .fold(view().flex_row().flex_wrap().gap_1(), |row, saved| {
                    let q_text = saved.query.clone();
                    let label = saved.name.clone();
                    let sid = saved.id;
                    row.child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || theme().border)
                            .child(
                                nsh_base::button()
                                    .accessibility_label(saved.name.clone())
                                    .h(px(26.0))
                                    .px_2()
                                    .rounded(px(4.0))
                                    .hover(|s| s.bg(move || theme().muted))
                                    .on_press(move || {
                                        query.set(q_text.clone());
                                        run_search();
                                    })
                                    .child(
                                        text(label)
                                            .text_size(px(12.0))
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(
                                nsh_base::button()
                                    .accessibility_label(t!("library.saved.delete"))
                                    .h(px(26.0))
                                    .px_1()
                                    .rounded(px(4.0))
                                    .hover(|s| s.bg(move || theme().muted))
                                    .on_press(move || {
                                        if let Some(store) = app_library::store() {
                                            let _ = store
                                                .write(move |tx| fts::delete_search(tx, sid))
                                                .wait();
                                            refresh();
                                        }
                                    })
                                    .child(
                                        text("×")
                                            .text_size(px(12.0))
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            ),
                    )
                })
        }))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    field(input().value(move || tags_draft.get()))
                        .flex_1()
                        .on_change(move |s: &str| tags_draft.set(s.to_owned()))
                        .placeholder(t!("library.tags.placeholder"))
                        .label(t!("library.tags.placeholder")),
                )
                .child(when(
                    move || active_doc_clone.is_some(),
                    move || {
                        action_button("library.add_current", Tone::Plain, add_active_doc.clone())
                    },
                ))
                .child(
                    field(input().value(move || doi_or_arxiv.get()))
                        .flex_1()
                        .on_change(move |s: &str| doi_or_arxiv.set(s.to_owned()))
                        .placeholder(t!("library.metadata.placeholder"))
                        .label(t!("library.metadata.placeholder")),
                )
                .child(segment_of(
                    move || t!("library.metadata.consent"),
                    move || consent_on.get(),
                    move || {
                        let next = !untracked(|| consent_on.get());
                        consent_on.set(next);
                        settings::mutate(|s| {
                            s.set_flag(ONLINE_METADATA_CONSENT, Some(next));
                        });
                    },
                ))
                .child(action_button(
                    "library.metadata.fetch",
                    Tone::Plain,
                    fetch_meta_action,
                )),
        )
        .child(dynamic(move || {
            let msg = status_msg.get();
            if msg.is_empty() {
                view()
            } else {
                view().child(
                    text(msg)
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(scroll().max_h(px(260.0)).child(dynamic(move || {
            let hit_list = hits.get();
            if !hit_list.is_empty() {
                return hit_list.into_iter().fold(view().gap_1(), |col, hit| {
                    let p = hit.path.clone();
                    let pg = hit.page;
                    let title = format!(
                        "{} (p. {}) — {}",
                        hit.path
                            .as_ref()
                            .and_then(|fp| fp.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("Document"),
                        hit.page + 1,
                        hit.snippet
                    );
                    let a11y = title.clone();
                    col.child(
                        nsh_base::button()
                            .accessibility_label(a11y)
                            .h(px(28.0))
                            .px_2()
                            .rounded(px(6.0))
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || {
                                if let Some(path) = p.clone() {
                                    close();
                                    open_at(path, pg);
                                }
                            })
                            .child(
                                text(title)
                                    .text_size(px(12.0))
                                    .text_color(move || theme().foreground),
                            ),
                    )
                });
            }
            let lib_items = items.get();
            if lib_items.is_empty() {
                return view().p_3().child(
                    text(move || t!("library.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            lib_items.into_iter().fold(view().gap_1(), |col, item| {
                let path = item.path.clone();
                let p2 = item.path.clone();
                let next_status = match item.read_status {
                    ReadStatus::Unread => ReadStatus::Reading,
                    ReadStatus::Reading => ReadStatus::Finished,
                    ReadStatus::Finished => ReadStatus::Unread,
                };
                let status_key = match item.read_status {
                    ReadStatus::Unread => "library.status.unread",
                    ReadStatus::Reading => "library.status.reading",
                    ReadStatus::Finished => "library.status.finished",
                };
                let item_title = item.title.clone();
                col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .child(
                            nsh_base::button()
                                .flex_1()
                                .h(px(28.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .accessibility_label(item_title.clone())
                                .on_press(move || {
                                    close();
                                    open_at(path.clone(), 0);
                                })
                                .child(
                                    text(item_title)
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(action_button(status_key, Tone::Plain, move || {
                            if let Some(store) = app_library::store() {
                                let lib = Library::new(store);
                                let prog = match next_status {
                                    ReadStatus::Unread => 0.0,
                                    ReadStatus::Reading => 0.5,
                                    ReadStatus::Finished => 1.0,
                                };
                                let _ = lib.set_read_status(&p2, next_status, prog);
                                refresh();
                            }
                        })),
                )
            })
        })))
        .child(view().flex_row().justify_end().gap_2().child(action_button(
            "ink_tools.done",
            Tone::Plain,
            close,
        )));
    list_dialog(t!("library.title"), close, card(body))
}

/// Duplicate Finder dialog detecting exact SHA-256 and MinHash near-duplicates (plan F.8).
pub(crate) fn duplicates_dialog(
    state: LibraryUiState,
    open_at: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.set_open(DLG_DUPLICATES, false);
    let threshold = signal(0.8_f64);
    let tick = signal(0_u64);

    let body = view()
        .w(px(600.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.find_duplicates"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(scroll().max_h(px(300.0)).child(dynamic(move || {
            let _ = tick.get();
            let pairs = app_library::store()
                .and_then(|s| Library::new(s).find_duplicates(threshold.get()).ok())
                .unwrap_or_default();
            if pairs.is_empty() {
                return view().p_3().child(
                    text(move || t!("library.duplicates.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            pairs.into_iter().fold(view().gap_2(), |col, pair| {
                let pa = pair.first.clone();
                let pb = pair.second.clone();
                let pa_lbl = pa.display().to_string();
                let pb_lbl = pb.display().to_string();
                let kind_key = match pair.kind {
                    DuplicateKind::Exact | DuplicateKind::RelatedDoi => "library.duplicates.exact",
                    DuplicateKind::NearText { .. } => "library.duplicates.near",
                };
                col.child(
                    view()
                        .p_2()
                        .gap_1()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .child(caption(kind_key))
                        .child(
                            nsh_base::button()
                                .accessibility_label(pa_lbl.clone())
                                .h(px(26.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    close();
                                    open_at(pa.clone(), 0);
                                })
                                .child(
                                    text(pa_lbl)
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(
                            nsh_base::button()
                                .accessibility_label(pb_lbl.clone())
                                .h(px(26.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    close();
                                    open_at(pb.clone(), 0);
                                })
                                .child(
                                    text(pb_lbl)
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                ),
                        ),
                )
            })
        })))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button(
                    "library.duplicates.scan",
                    Tone::Plain,
                    move || {
                        tick.update(|n| *n += 1);
                    },
                ))
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.find_duplicates"), close, card(body))
}

/// Citations (BibTeX / RIS / CSL-JSON) & Zotero local/web sync dialog (plan F.10, F.11).
pub(crate) fn citations_dialog(state: LibraryUiState) -> impl IntoElement {
    let close = move || state.set_open(DLG_CITATIONS, false);
    let raw_text = signal(String::new());
    let zotero_url = signal(LOCAL_API_BASE.to_owned());
    let zotero_user = signal(String::new());
    let zotero_key = signal(String::new());
    let format_idx = signal(0_u8); // 0=BibTeX, 1=RIS, 2=CSL-JSON
    let style_idx = signal(0_u8); // 0=APA, 1=MLA, 2=Chicago, 3=IEEE
    let preview = signal(String::new());
    let entries = signal(Vec::<CitationEntry>::new());

    let parse_input = move || {
        let input_str = untracked(|| raw_text.get());
        let parsed = match untracked(|| format_idx.get()) {
            1 => Ok(parse_ris(&input_str)),
            2 => parse_csl_json(&input_str),
            _ => parse_bibtex(&input_str),
        };
        match parsed {
            Ok(mut list) => {
                for entry in &mut list {
                    if entry.key.is_empty() {
                        entry.key = generate_citation_key(
                            entry,
                            ecclesia_features::citations::KeyFormat::AuthorYear,
                        );
                    }
                }
                let style = match untracked(|| style_idx.get()) {
                    1 | 2 => CitationStyle::Chicago,
                    3 => CitationStyle::Ieee,
                    _ => CitationStyle::Apa,
                };
                let formatted = list
                    .iter()
                    .map(|e| format!("[{}] {}", e.key, format_citation(e, style)))
                    .collect::<Vec<_>>()
                    .join("\n");
                entries.set(list);
                preview.set(formatted);
            }
            Err(err) => preview.set(err.to_string()),
        }
    };

    let export_action = move || {
        let list = untracked(|| entries.get());
        let (ext, payload) = match untracked(|| format_idx.get()) {
            1 => ("ris", export_ris(&list)),
            2 => ("json", export_csl_json(&list)),
            _ => ("bib", export_bibtex(&list)),
        };
        let dialog = SaveDialog::new()
            .title(t!("command.citations"))
            .filter(FileFilter::new(t!("command.citations"), [ext]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(path)) = answer.await {
                let _ = write_atomically(&path, payload.as_bytes());
            }
        });
    };

    let sync_zotero = move || {
        let uid = untracked(|| zotero_user.get());
        let key = untracked(|| zotero_key.get());
        let base = untracked(|| zotero_url.get());
        let source = if !uid.trim().is_empty() && !key.trim().is_empty() {
            ZoteroSource::web_user(uid.trim(), key.trim())
        } else {
            ZoteroSource::LocalHttp {
                base_url: base.trim().to_owned(),
            }
        };
        let client = ZoteroClient::new(source);
        match client.fetch_items(25) {
            Ok(items) => {
                let list: Vec<CitationEntry> = items.into_iter().map(|i| i.citation).collect();
                raw_text.set(export_bibtex(&list));
                entries.set(list);
                parse_input();
            }
            Err(err) => preview.set(err.to_string()),
        }
    };

    let body = view()
        .w(px(640.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.citations"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    segmented()
                        .child(segment_of(
                            move || String::from("BibTeX"),
                            move || format_idx.get() == 0,
                            move || format_idx.set(0),
                        ))
                        .child(segment_of(
                            move || String::from("RIS"),
                            move || format_idx.get() == 1,
                            move || format_idx.set(1),
                        ))
                        .child(segment_of(
                            move || String::from("CSL-JSON"),
                            move || format_idx.get() == 2,
                            move || format_idx.set(2),
                        )),
                )
                .child(
                    segmented()
                        .child(segment_of(
                            move || String::from("APA"),
                            move || style_idx.get() == 0,
                            move || style_idx.set(0),
                        ))
                        .child(segment_of(
                            move || String::from("MLA"),
                            move || style_idx.get() == 1,
                            move || style_idx.set(1),
                        ))
                        .child(segment_of(
                            move || String::from("Chicago"),
                            move || style_idx.get() == 2,
                            move || style_idx.set(2),
                        ))
                        .child(segment_of(
                            move || String::from("IEEE"),
                            move || style_idx.get() == 3,
                            move || style_idx.set(3),
                        )),
                ),
        )
        .child(
            field(input().value(move || raw_text.get()))
                .multiline(true)
                .h(px(120.0))
                .on_change(move |s: &str| raw_text.set(s.to_owned()))
                .placeholder(t!("library.citations.placeholder"))
                .label(t!("command.citations")),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || zotero_url.get()))
                        .flex_1()
                        .on_change(move |s: &str| zotero_url.set(s.to_owned()))
                        .placeholder(t!("library.zotero.local_url"))
                        .label(t!("library.zotero.local_url")),
                )
                .child(
                    field(input().value(move || zotero_user.get()))
                        .w(px(110.0))
                        .on_change(move |s: &str| zotero_user.set(s.to_owned()))
                        .placeholder(t!("library.zotero.user"))
                        .label(t!("library.zotero.user")),
                )
                .child(
                    field(input().value(move || zotero_key.get()))
                        .w(px(130.0))
                        .on_change(move |s: &str| zotero_key.set(s.to_owned()))
                        .placeholder(t!("library.zotero.api_key"))
                        .label(t!("library.zotero.api_key")),
                )
                .child(action_button(
                    "library.zotero.sync",
                    Tone::Plain,
                    sync_zotero,
                )),
        )
        .child(
            scroll().max_h(px(120.0)).child(
                text(move || preview.get())
                    .text_size(px(12.0))
                    .text_color(move || theme().foreground),
            ),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(action_button(
                    "library.citations.parse",
                    Tone::Plain,
                    parse_input,
                ))
                .child(crate::shell::primary_button(
                    "library.citations.export",
                    export_action,
                )),
        );
    list_dialog(t!("command.citations"), close, card(body))
}

/// Markdown Notes with `[[backlinks]]` & Obsidian Deep-Link Export dialog (plans F.12, F.13).
pub(crate) fn notes_backlinks_dialog(
    state: LibraryUiState,
    active_doc: Option<PathBuf>,
    active_page: usize,
) -> impl IntoElement {
    let close = move || state.set_open(DLG_NOTES, false);
    let note_id = signal(new_id());
    let title = signal(String::new());
    let body_draft = signal(String::new());
    let status_msg = signal(String::new());
    let notes_list = signal(Vec::<Note>::new());
    let backlinks_count = signal(0_usize);
    let parsed_links_count = signal(0_usize);

    let reload_notes = move || {
        let Some(store) = app_library::store() else {
            return;
        };
        let mgr = Notes::new(store);
        if let Ok(all) = mgr.list() {
            notes_list.set(all);
        }
        if let Ok(graph) = mgr.graph() {
            let id = untracked(|| note_id.get());
            let inc = graph.incoming.get(&id).map_or(0, Vec::len);
            backlinks_count.set(inc);
        }
    };
    reload_notes();

    let save_note = {
        let doc = active_doc.clone();
        move || {
            let t_str = untracked(|| title.get());
            let b_str = untracked(|| body_draft.get());
            if t_str.trim().is_empty() {
                return;
            }
            let Some(store) = app_library::store() else {
                return;
            };
            let mgr = Notes::new(store);
            let now = now_secs();
            let note = Note {
                id: untracked(|| note_id.get()),
                title: t_str.trim().to_owned(),
                body: b_str.clone(),
                file: doc.clone(),
                page: Some(active_page),
                annotation_id: None,
                tags: Vec::new(),
                created_at: now,
                modified_at: now,
            };
            if mgr.save(&note).is_ok() {
                parsed_links_count.set(parse_links_in_markdown(&b_str).len());
                status_msg.set(t!("library.notes.saved"));
                reload_notes();
            }
        }
    };

    let export_obsidian = {
        let active_doc_export = active_doc.clone();
        move || {
            let Some(store) = app_library::store() else {
                return;
            };
            let all = Notes::new(store).list().unwrap_or_default();
            if all.is_empty() {
                let t_str = untracked(|| title.get());
                let b_str = untracked(|| body_draft.get());
                let single = Note {
                    id: untracked(|| note_id.get()),
                    title: if t_str.is_empty() {
                        "Note".to_owned()
                    } else {
                        t_str
                    },
                    body: b_str,
                    file: active_doc_export.clone(),
                    page: Some(active_page),
                    annotation_id: None,
                    tags: Vec::new(),
                    created_at: now_secs(),
                    modified_at: now_secs(),
                };
                let md = export_note_markdown(&single);
                let dialog = SaveDialog::new()
                    .title(t!("library.notes.export_obsidian"))
                    .filter(FileFilter::new(t!("library.notes.markdown_filter"), ["md"]));
                let answer = pick_save_path(dialog);
                spawn(async move {
                    if let Ok(Some(path)) = answer.await {
                        let _ = write_atomically(&path, md.as_bytes());
                    }
                });
                return;
            }
            let dialog = OpenDialog::folders().title(t!("library.notes.export_obsidian"));
            let answer = pick_paths(dialog);
            spawn(async move {
                if let Ok(Some(paths)) = answer.await
                    && let Some(dir) = paths.into_iter().next()
                    && let Ok(report) = export_obsidian_vault(&dir, &all, &[])
                {
                    status_msg.set(t!(
                        "library.notes.exported_vault",
                        count = report.notes_exported
                    ));
                }
            });
        }
    };

    let deep_link_preview = active_doc.as_ref().map(|p| {
        DeepLink {
            file: p.clone(),
            page: Some(active_page + 1),
            annot: None,
        }
        .to_uri()
    });
    let has_deep_link = deep_link_preview.is_some();

    let body = view()
        .w(px(640.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.library_notes"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || title.get()))
                .on_change(move |s: &str| title.set(s.to_owned()))
                .placeholder(t!("library.notes.title_placeholder"))
                .label(t!("library.notes.title_placeholder"))
                .auto_focus(true),
        )
        .child(
            field(input().value(move || body_draft.get()))
                .multiline(true)
                .h(px(130.0))
                .on_change(move |s: &str| body_draft.set(s.to_owned()))
                .placeholder(t!("library.notes.body_placeholder"))
                .label(t!("library.notes.body_placeholder")),
        )
        .child(
            text(move || {
                t!(
                    "library.notes.stats",
                    links = parsed_links_count.get(),
                    backlinks = backlinks_count.get()
                )
            })
            .text_size(px(11.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(when(
            move || has_deep_link,
            move || {
                let uri = deep_link_preview.clone().unwrap_or_default();
                text(uri)
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground)
            },
        ))
        .child(scroll().max_h(px(110.0)).child(dynamic(move || {
            notes_list.get().into_iter().fold(view().gap_1(), |col, n| {
                let n_title = n.title.clone();
                col.child(
                    nsh_base::button()
                        .accessibility_label(n_title.clone())
                        .h(px(26.0))
                        .px_2()
                        .rounded(px(4.0))
                        .hover(|s| s.bg(move || theme().muted))
                        .on_press(move || {
                            note_id.set(n.id.clone());
                            title.set(n.title.clone());
                            body_draft.set(n.body.clone());
                        })
                        .child(
                            text(n_title)
                                .text_size(px(12.0))
                                .text_color(move || theme().foreground),
                        ),
                )
            })
        })))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(action_button(
                    "library.notes.export_obsidian",
                    Tone::Plain,
                    export_obsidian,
                ))
                .child(crate::shell::primary_button(
                    "library.notes.save",
                    save_note,
                )),
        );
    list_dialog(t!("command.library_notes"), close, card(body))
}

/// Workspace Templates & Path Auto-Routing Rules dialog (plan W.12).
pub(crate) fn workspace_rules_dialog(
    state: LibraryUiState,
    active_ws: WorkspaceId,
    open_ws: impl Fn(WorkspaceId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.set_open(DLG_RULES, false);
    let tmpl_name = signal(String::new());
    let rule_name = signal(String::new());
    let folder_prefix = signal(String::new());
    let tick = signal(0_u64);

    let save_tmpl_action = move || {
        let name = untracked(|| tmpl_name.get());
        if name.trim().is_empty() {
            return;
        }
        let Some(store) = app_library::store() else {
            return;
        };
        let template = WorkspaceTemplate {
            id: new_id(),
            name: name.trim().to_owned(),
            icon: None,
            color: None,
            layout: None,
            items: Vec::new(),
            created_at: now_secs(),
        };
        if store
            .write(move |tx| save_template(tx, &template))
            .wait()
            .is_ok()
        {
            tmpl_name.set(String::new());
            tick.update(|n| *n += 1);
        }
    };

    let save_rule_action = move || {
        let rname = untracked(|| rule_name.get());
        let prefix = untracked(|| folder_prefix.get());
        if rname.trim().is_empty() || prefix.trim().is_empty() {
            return;
        }
        let Some(store) = app_library::store() else {
            return;
        };
        let rule = RoutingRule {
            id: new_id(),
            name: rname.trim().to_owned(),
            folder_prefix: PathBuf::from(prefix.trim()),
            workspace_id: Some(active_ws),
            workspace_name: String::new(),
            priority: 10,
            enabled: true,
        };
        if store.write(move |tx| save_rule(tx, &rule)).wait().is_ok() {
            rule_name.set(String::new());
            folder_prefix.set(String::new());
            tick.update(|n| *n += 1);
        }
    };

    let body = view()
        .w(px(620.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.workspace_rules"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("workspace.templates.heading"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || tmpl_name.get()))
                        .flex_1()
                        .on_change(move |s: &str| tmpl_name.set(s.to_owned()))
                        .placeholder(t!("workspace.templates.placeholder"))
                        .label(t!("workspace.templates.placeholder")),
                )
                .child(action_button(
                    "workspace.templates.save",
                    Tone::Plain,
                    save_tmpl_action,
                )),
        )
        .child(caption("workspace.rules.heading"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || rule_name.get()))
                        .w(px(160.0))
                        .on_change(move |s: &str| rule_name.set(s.to_owned()))
                        .placeholder(t!("workspace.rules.name_placeholder"))
                        .label(t!("workspace.rules.name_placeholder")),
                )
                .child(
                    field(input().value(move || folder_prefix.get()))
                        .flex_1()
                        .on_change(move |s: &str| folder_prefix.set(s.to_owned()))
                        .placeholder(t!("workspace.rules.prefix_placeholder"))
                        .label(t!("workspace.rules.prefix_placeholder")),
                )
                .child(action_button(
                    "workspace.rules.add",
                    Tone::Plain,
                    save_rule_action,
                )),
        )
        .child(scroll().max_h(px(200.0)).child(dynamic(move || {
            let _ = tick.get();
            let Some(store) = app_library::store() else {
                return view();
            };
            let tmpls = store.read(list_templates).unwrap_or_default();
            let rules = store.read(list_rules).unwrap_or_default();
            let mut col = view().gap_2();
            for tmpl in tmpls {
                let tname = tmpl.name.clone();
                let tmpl_clone = tmpl.clone();
                col = col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(tname)
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            nsh_base::button()
                                .accessibility_label(t!("workspace.templates.apply"))
                                .h(px(26.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    if let Ok(ws_id) =
                                        instantiate_template(store, &tmpl_clone, None)
                                    {
                                        close();
                                        open_ws(ws_id);
                                    }
                                })
                                .child(
                                    text(move || t!("workspace.templates.apply"))
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                ),
                        ),
                );
            }
            for r in rules {
                let rid = r.id.clone();
                let line = format!("{} — {}", r.name, r.folder_prefix.display());
                col = col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(line)
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            nsh_base::button()
                                .accessibility_label(t!("workspace.rules.delete"))
                                .h(px(26.0))
                                .px_1()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    let rid2 = rid.clone();
                                    let _ = store.write(move |tx| delete_rule(tx, &rid2)).wait();
                                    tick.update(|n| *n += 1);
                                })
                                .child(
                                    text("×")
                                        .text_size(px(12.0))
                                        .text_color(move || theme().muted_foreground),
                                ),
                        ),
                );
            }
            col
        })))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.workspace_rules"), close, card(body))
}

/// CLI, Agent Tool Table, MCP Server & UI Control Channel Reference dialog (plans E.1–E.4).
pub(crate) fn cli_mcp_docs_dialog(state: LibraryUiState) -> impl IntoElement {
    let close = move || state.set_open(DLG_CLI_DOCS, false);
    let status = control_channel::status_snapshot();
    let endpoint_line = status
        .endpoint
        .or(status.error)
        .unwrap_or_else(|| t!("control.inactive"));
    let token_line = status
        .token_file
        .map_or_else(String::new, |p| p.display().to_string());
    let has_token = !token_line.is_empty();

    let body = view()
        .w(px(620.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.cli_mcp_docs"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(move || t!("control.cli_summary"))
                .text_size(px(12.0))
                .text_color(move || theme().foreground),
        )
        .child(
            text(move || t!("control.mcp_summary"))
                .text_size(px(12.0))
                .text_color(move || theme().foreground),
        )
        .child(caption("control.channel.heading"))
        .child(
            text(endpoint_line)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(when(
            move || has_token,
            move || {
                text(token_line.clone())
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground)
            },
        ))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.cli_mcp_docs"), close, card(body))
}
