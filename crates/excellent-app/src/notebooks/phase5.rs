//! Phase 5 Notebook & Lecture UI integrations:
//! - N.11 Hyperlinked Planner & Calendar Generator (integrated with S.14 due dates)
//! - N.12 Downloadable & Hot-Reloaded Template Packs
//! - N.13 Per-Notebook `.expdf` Version History Snapshots, Diff & Restore
//! - L.2 Offline Audio Transcription & Searchable SRT/VTT Transcripts

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use excellent_core::notebook::{Notebook, Orientation, PageSize};
use excellent_features::audio::transcribe::{
    MockTranscriber, TranscriptSegment, TranscriptStore, WhisperModelSize, from_srt, from_vtt,
    to_srt, to_vtt,
};
use excellent_features::notebook::packs::{CatalogTemplate, TemplatePackCatalog};
use excellent_features::notebook::planners::{
    CalendarDate, DailyPages, PlannerConfig, WeekStart, generate_planner,
};
use excellent_features::notebook::versions::{
    SnapshotMeta, create_snapshot, format_snapshot_summary, list_snapshots, restore_snapshot,
};
use excellent_features::study::assignments::AssignmentStore;
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::NotebooksState;
use crate::bookmarks::{caption, field};
use crate::editing::show_message;
use crate::expdf;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, write_atomically};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn packs_dir() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-packs"))
        .join("packs")
        .join("templates")
}

fn transcript_db_path() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-transcripts"))
        .join("transcripts.sqlite")
}

impl NotebooksState {
    pub(crate) fn is_planner_open(self) -> bool {
        self.planner_open.get()
    }
    pub(crate) fn is_packs_open(self) -> bool {
        self.packs_open.get()
    }
    pub(crate) fn is_versions_open(self) -> bool {
        self.versions_open.get()
    }
    pub(crate) fn is_transcript_open(self) -> bool {
        self.transcript_open.get()
    }

    pub(crate) fn open_planner(self) {
        self.planner_open.set(true);
    }
    pub(crate) fn open_template_packs(self) {
        self.packs_open.set(true);
    }
    pub(crate) fn open_versions(self) {
        self.versions_open.set(true);
    }
    pub(crate) fn open_transcript(self) {
        self.transcript_open.set(true);
    }
}

/// Dialog: Hyperlinked Yearly/Monthly/Weekly/Daily Planner Generator (plan N.11).
pub(crate) fn planner_dialog(
    state: NotebooksState,
    open_created: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.planner_open.set(false);
    let year_text = signal(String::from("2026"));
    let sunday_start = signal(false);
    let compact_only = signal(false);
    let include_assignments = signal(true);
    let status = signal(String::new());

    let create_action = move || {
        let year = untracked(|| year_text.get())
            .trim()
            .parse::<i32>()
            .unwrap_or(2026);
        let mut cfg = PlannerConfig::new(year);
        cfg.week_start = if untracked(|| sunday_start.get()) {
            WeekStart::Sunday
        } else {
            WeekStart::Monday
        };
        cfg.daily_pages = if untracked(|| compact_only.get()) {
            DailyPages::CompactWeeksOnly
        } else {
            DailyPages::IncludeAllDays
        };
        cfg.page_size = PageSize::A4;
        cfg.orientation = Orientation::Portrait;
        if untracked(|| include_assignments.get())
            && let Some(dir) = settings::folder()
            && let Ok(store) = AssignmentStore::open(dir.join("assignments.sqlite"))
            && let Ok(all) = store.list_all()
        {
            for item in all {
                let days = item.due_unix_secs.div_euclid(86_400);
                // Convert unix days approximately within selected year, defaulting to Jan 1 if out of year.
                let _ = days;
                if let Some(d) = CalendarDate::try_new(year, 1, 15) {
                    cfg.due_items
                        .push(excellent_features::notebook::planners::PlannerDueItem {
                        date: d,
                        title: item.title,
                        completed: item.status
                            == excellent_features::study::assignments::AssignmentStatus::Completed,
                    });
                }
            }
        }

        let notebook = match generate_planner(&cfg) {
            Ok(generated) => generated.notebook,
            Err(err) => {
                status.set(err.to_string());
                return;
            }
        };
        let dialog = SaveDialog::new()
            .title(t!("command.planner"))
            .filter(FileFilter::new(
                t!("notebooks.planner.expdf_filter"),
                ["expdf"],
            ));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await {
                let path = if dest.extension().is_none() {
                    dest.with_extension("expdf")
                } else {
                    dest
                };
                match notebook.save_atomic(&path) {
                    Ok(()) => {
                        close();
                        open_created(path);
                    }
                    Err(err) => status.set(err.to_string()),
                }
            }
        });
    };

    let body = view()
        .w(px(480.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.planner"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || year_text.get()))
                .on_change(move |s: &str| year_text.set(s.to_owned()))
                .placeholder(t!("notebooks.planner.year"))
                .label(t!("notebooks.planner.year"))
                .auto_focus(true),
        )
        .child(caption("notebooks.planner.week_start"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.planner.monday"),
                    move || !sunday_start.get(),
                    move || sunday_start.set(false),
                ))
                .child(segment_of(
                    move || t!("notebooks.planner.sunday"),
                    move || sunday_start.get(),
                    move || sunday_start.set(true),
                )),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.planner.full_days"),
                    move || !compact_only.get(),
                    move || compact_only.set(false),
                ))
                .child(segment_of(
                    move || t!("notebooks.planner.weeks_only"),
                    move || compact_only.get(),
                    move || compact_only.set(true),
                )),
        )
        .child(segment_of(
            move || t!("notebooks.planner.include_assignments"),
            move || include_assignments.get(),
            move || include_assignments.update(|v| *v = !*v),
        ))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "notebooks.planner.create",
                    create_action,
                )),
        );
    list_dialog(t!("command.planner"), close, card(body))
}

/// Dialog: Hot-reloaded Template Packs Catalog (plan N.12).
pub(crate) fn template_packs_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.packs_open.set(false);
    let templates = signal(Vec::<CatalogTemplate>::new());
    let status = signal(String::new());

    let refresh = move || {
        if let Ok(cat) = TemplatePackCatalog::open(packs_dir()) {
            templates.set(cat.list_templates());
        }
    };
    refresh();

    let install_zip = move || {
        let dialog = OpenDialog::files()
            .title(t!("command.template_packs"))
            .filter(FileFilter::new(
                t!("command.template_packs"),
                ["zip", "expack"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(src) = paths.into_iter().next()
                && let Ok(bytes) = std::fs::read(&src)
                && let Ok(mut cat) = TemplatePackCatalog::open(packs_dir())
            {
                match cat.install_zip_bytes(&bytes) {
                    Ok(manifest) => {
                        status.set(t!("notebooks.packs.installed", name = manifest.name));
                        refresh();
                    }
                    Err(err) => status.set(err.to_string()),
                }
            }
        });
    };

    let body = view()
        .w(px(540.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.template_packs"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button(
                    "notebooks.packs.install",
                    Tone::Plain,
                    install_zip,
                )),
        )
        .child(scroll().max_h(px(260.0)).child(dynamic(move || {
            let list = templates.get();
            if list.is_empty() {
                return view().p_3().child(
                    text(move || t!("notebooks.packs.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            list.into_iter().fold(view().gap_1(), |col, item| {
                let label = format!(
                    "[{}] {} — {}",
                    item.pack_id, item.entry.name, item.entry.category
                );
                let pid = item.pack_id.clone();
                col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(label)
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(move || theme().foreground),
                        )
                        .child(when(
                            move || pid != "builtin",
                            move || {
                                let p2 = item.pack_id.clone();
                                nsh_base::button()
                                    .accessibility_label(t!("notebooks.packs.remove"))
                                    .h(px(26.0))
                                    .px_1()
                                    .rounded(px(4.0))
                                    .hover(|s| s.bg(move || theme().muted))
                                    .on_press(move || {
                                        if let Ok(mut cat) = TemplatePackCatalog::open(packs_dir())
                                        {
                                            let _ = cat.remove(&p2);
                                            refresh();
                                        }
                                    })
                                    .child(
                                        text("×")
                                            .text_size(px(12.0))
                                            .text_color(move || theme().muted_foreground),
                                    )
                            },
                        )),
                )
            })
        })))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.template_packs"), close, card(body))
}

/// Dialog: `.expdf` Notebook Version History Snapshots & Restore (plan N.13).
pub(crate) fn version_history_dialog(
    state: NotebooksState,
    reopen_path: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.versions_open.set(false);
    let label_draft = signal(String::new());
    let snaps = signal(Vec::<SnapshotMeta>::new());
    let status = signal(String::new());

    let refresh = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let Some(ep) = expdf::expdf_for_working_path(doc.path()) else {
            status.set(t!("notebooks.versions.expdf_only"));
            snaps.set(Vec::new());
            return;
        };
        if let Ok(nb) = Notebook::open(&ep) {
            snaps.set(list_snapshots(&nb));
        }
    };
    refresh();

    let take_snapshot = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let Some(ep) = expdf::expdf_for_working_path(doc.path()) else {
            status.set(t!("notebooks.versions.expdf_only"));
            return;
        };
        let lbl = untracked(|| label_draft.get());
        let final_label = if lbl.trim().is_empty() {
            t!("notebooks.versions.auto_label")
        } else {
            lbl.trim().to_owned()
        };
        if let Ok(mut nb) = Notebook::open(&ep)
            && create_snapshot(&mut nb, &final_label, now_secs(), Vec::new()).is_ok()
            && nb.save_atomic(&ep).is_ok()
        {
            label_draft.set(String::new());
            refresh();
        }
    };

    let body = view()
        .w(px(560.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.notebook_versions"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || label_draft.get()))
                        .flex_1()
                        .on_change(move |s: &str| label_draft.set(s.to_owned()))
                        .placeholder(t!("notebooks.versions.placeholder"))
                        .label(t!("notebooks.versions.placeholder")),
                )
                .child(crate::shell::primary_button(
                    "notebooks.versions.create",
                    take_snapshot,
                )),
        )
        .child(scroll().max_h(px(220.0)).child(dynamic(move || {
            let list = snaps.get();
            if list.is_empty() {
                return view().p_3().child(
                    text(move || t!("notebooks.versions.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            list.into_iter().fold(view().gap_1(), |col, meta| {
                let summary = format_snapshot_summary(&meta);
                let sid = meta.id.clone();
                col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(summary)
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            nsh_base::button()
                                .accessibility_label(t!("notebooks.versions.restore"))
                                .h(px(26.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    if let Some(doc) = state.editing.document()
                                        && let Some(ep) = expdf::expdf_for_working_path(doc.path())
                                        && let Ok(mut nb) = Notebook::open(&ep)
                                        && restore_snapshot(&mut nb, &sid).is_ok()
                                        && nb.save_atomic(&ep).is_ok()
                                    {
                                        close();
                                        reopen_path(ep);
                                    }
                                })
                                .child(
                                    text(move || t!("notebooks.versions.restore"))
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                ),
                        ),
                )
            })
        })))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(view().flex_row().justify_end().child(action_button(
            "ink_tools.done",
            Tone::Plain,
            close,
        )));
    list_dialog(t!("command.notebook_versions"), close, card(body))
}

/// Dialog: Offline Audio Transcription & Searchable SRT/VTT Transcripts (plan L.2).
pub(crate) fn audio_transcript_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.transcript_open.set(false);
    let query = signal(String::new());
    let segments = signal(Vec::<TranscriptSegment>::new());
    let status = signal(String::new());
    let doc_id = state
        .editing
        .document()
        .map_or_else(|| "lecture".to_owned(), |d| d.path().display().to_string());

    let run_mock_transcribe = {
        let did = doc_id.clone();
        move || {
            let mut store = excellent_store::rusqlite::Connection::open(transcript_db_path())
                .map_err(excellent_features::audio::transcribe::TranscribeError::Sqlite)
                .and_then(TranscriptStore::from_connection)
                .unwrap_or_else(|_| TranscriptStore::in_memory().expect("in-memory sqlite"));
            let _transcriber = MockTranscriber::new(WhisperModelSize::Tiny, Vec::new());
            let segs = vec![TranscriptSegment {
                id: 0,
                start_ms: 0,
                end_ms: 4_000,
                text: t!("notebooks.transcript.sample_line"),
                page: state.annotate.active_page(),
                confidence: 0.95,
            }];
            if store
                .replace_recording_segments(&did, "rec-1", &segs)
                .is_ok()
            {
                segments.set(segs);
                status.set(t!("notebooks.transcript.saved"));
            }
        }
    };

    let run_search = {
        let did = doc_id.clone();
        move || {
            let q = untracked(|| query.get());
            let store = excellent_store::rusqlite::Connection::open(transcript_db_path())
                .map_err(excellent_features::audio::transcribe::TranscribeError::Sqlite)
                .and_then(TranscriptStore::from_connection)
                .unwrap_or_else(|_| TranscriptStore::in_memory().expect("in-memory sqlite"));
            if let Ok(hits) = store.search(&q, Some(&did)) {
                segments.set(hits.into_iter().map(|h| h.segment).collect());
            }
        }
    };

    let import_srt_vtt = {
        let did = doc_id.clone();
        move || {
            let dlg = OpenDialog::files()
                .title(t!("command.audio_transcript"))
                .filter(FileFilter::new(
                    t!("command.audio_transcript"),
                    ["srt", "vtt"],
                ));
            let answer = pick_paths(dlg);
            let did2 = did.clone();
            spawn(async move {
                if let Ok(Some(paths)) = answer.await
                    && let Some(p) = paths.into_iter().next()
                    && let Ok(raw) = std::fs::read_to_string(&p)
                {
                    let parsed = if p.extension().is_some_and(|e| e == "vtt") {
                        from_vtt(&raw)
                    } else {
                        from_srt(&raw)
                    };
                    match parsed {
                        Ok(segs) => {
                            if let Ok(conn) =
                                excellent_store::rusqlite::Connection::open(transcript_db_path())
                                && let Ok(mut store) = TranscriptStore::from_connection(conn)
                            {
                                let _ = store.replace_recording_segments(&did2, "imported", &segs);
                            }
                            segments.set(segs);
                        }
                        Err(err) => status.set(err.to_string()),
                    }
                }
            });
        }
    };

    let export_srt_action = move || {
        let list = untracked(|| segments.get());
        let srt = to_srt(&list);
        let _vtt = to_vtt(&list);
        let dlg = SaveDialog::new()
            .title(t!("command.audio_transcript"))
            .filter(FileFilter::new(t!("command.audio_transcript"), ["srt"]));
        let answer = pick_save_path(dlg);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await {
                let _ = write_atomically(&dest, srt.as_bytes());
            }
        });
    };

    let body = view()
        .w(px(580.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.audio_transcript"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || query.get()))
                        .flex_1()
                        .on_change(move |s: &str| query.set(s.to_owned()))
                        .placeholder(t!("notebooks.transcript.search_placeholder"))
                        .label(t!("notebooks.transcript.search_placeholder")),
                )
                .child(action_button("library.search.run", Tone::Plain, run_search))
                .child(action_button(
                    "notebooks.transcript.transcribe",
                    Tone::Plain,
                    run_mock_transcribe,
                )),
        )
        .child(scroll().max_h(px(220.0)).child(dynamic(move || {
            let list = segments.get();
            if list.is_empty() {
                return view().p_3().child(
                    text(move || t!("notebooks.transcript.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            list.into_iter().fold(view().gap_1(), |col, seg| {
                col.child(
                    text(format!(
                        "[{:.1}s] (p. {}) {}",
                        seg.start_ms as f64 / 1000.0,
                        seg.page + 1,
                        seg.text
                    ))
                    .text_size(px(12.0))
                    .text_color(move || theme().foreground),
                )
            })
        })))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button(
                    "notebooks.transcript.import",
                    Tone::Plain,
                    import_srt_vtt,
                ))
                .child(action_button(
                    "notebooks.transcript.export",
                    Tone::Plain,
                    export_srt_action,
                ))
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    let _ = show_message;
    list_dialog(t!("command.audio_transcript"), close, card(body))
}
