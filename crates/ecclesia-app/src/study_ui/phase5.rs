//! Phase 5 Study, Language Learning, Translation & Neural Offline Voices UI:
//! - S.13 Offline Summaries from Highlights & Notes
//! - S.14 Assignments with Due Dates, Checklists & Reminders
//! - S.15 Language Learning Tap-to-Lookup, Pronunciation & Parallel Translation
//! - S.16 Translate Selection Popover with 73 Languages, Consent Gate & Bergamot Packs
//! - U.6 Neural Offline Piper Voice Catalog Picker & Settings Integration

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_core::anchor::new_id;
use ecclesia_features::neural_voices::{
    DeterministicPiperBackend, PiperVoiceSynthesizer, VoicePackCatalog,
};
use ecclesia_features::study::assignments::{
    Assignment, AssignmentStatus, AssignmentStore, CheckState, ChecklistItem,
};
use ecclesia_features::study::export::StudyNoteKind;
use ecclesia_features::study::language::{
    ParallelTranslationPane, inspect_tapped_word, pronounce_word,
};
use ecclesia_features::study::summaries::{
    HeadingAnchor, HighlightItem, SummaryDetail, build_offline_summary,
};
use ecclesia_features::translate::{
    BergamotPackCatalog, ConsentPolicy, SUPPORTED_LANGUAGES, SelectionTranslator, TranslateEngine,
    TranslateRequest, Translator,
};
use ecclesia_store::study::{CardRecord, DeckRecord, upsert_card, upsert_deck};
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use super::StudyState;
use crate::bookmarks::{caption, field};
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, write_atomically};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// `settings.toml` key selecting the active offline neural Piper voice ID (U.6).
pub(crate) const NEURAL_VOICE_ID: &str = "read_aloud_neural_voice";

/// `settings.toml` key gating online translation engines behind explicit consent (S.16).
pub(crate) const TRANSLATE_ONLINE_CONSENT: &str = "translate_online_consent";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn assignments_db() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-assignments"))
        .join("assignments.sqlite")
}

pub(crate) fn neural_voices_dir() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-voices"))
        .join("packs")
        .join("voices")
}

fn bergamot_packs_dir() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-translate"))
        .join("packs")
        .join("bergamot")
}

impl StudyState {
    pub(crate) fn is_summaries_open(self) -> bool {
        self.summaries_open.get()
    }
    pub(crate) fn is_assignments_open(self) -> bool {
        self.assignments_open.get()
    }
    pub(crate) fn is_language_open(self) -> bool {
        self.language_open.get()
    }
    pub(crate) fn is_translate_open(self) -> bool {
        self.translate_open.get()
    }

    pub(crate) fn open_summaries(self) {
        self.summaries_open.set(true);
    }
    pub(crate) fn open_assignments(self) {
        self.assignments_open.set(true);
    }
    pub(crate) fn open_language(self) {
        self.language_open.set(true);
    }
    pub(crate) fn open_translate(self) {
        self.translate_open.set(true);
    }

    /// Lists currently installed offline neural voice IDs & names for Settings UI (U.6).
    pub(crate) fn installed_neural_voices() -> Vec<(String, String)> {
        VoicePackCatalog::open(neural_voices_dir())
            .map(|cat| {
                cat.voices()
                    .into_iter()
                    .map(|v| (v.id, format!("{} ({})", v.name, v.language)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Dialog: Auto-Summaries from Highlights & Notes grouped by heading (plan S.13).
pub(crate) fn highlight_summaries_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.summaries_open.set(false);
    let compact = signal(false);
    let markdown_out = signal(String::new());

    let rebuild = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let title = doc
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Document")
            .to_owned();
        let detail = if untracked(|| compact.get()) {
            SummaryDetail::Compact
        } else {
            SummaryDetail::Full
        };
        let mut items = Vec::new();
        let mut headings = Vec::new();
        if let Ok(shared) = doc.mupdf() {
            let mupdf = shared.lock().expect("document lock poisoned");
            if let Ok(outline) = mupdf.outline() {
                for entry in outline {
                    if let Some(p) = entry.page {
                        headings.push(HeadingAnchor {
                            title: entry.title,
                            level: 1,
                            page: p,
                            y_pt: 0.0,
                        });
                    }
                }
            }
            let pages = mupdf.page_count().unwrap_or(0);
            for pg in 0..pages {
                if let Ok(annots) = ecclesia_core::annot::page_annotations(&mupdf, pg) {
                    for a in annots {
                        let quote = a
                            .anchor
                            .as_ref()
                            .map(|q| q.exact.clone())
                            .unwrap_or_default();
                        if !quote.is_empty() || !a.contents.trim().is_empty() {
                            items.push(HighlightItem {
                                id: a.id,
                                page: a.page,
                                y_pt: 0.0,
                                x_pt: 0.0,
                                kind: StudyNoteKind::Highlight,
                                quoted_text: quote,
                                note: a.contents,
                                tags: Vec::new(),
                            });
                        }
                    }
                }
            }
        }
        let doc_summary = build_offline_summary(&title, &headings, &items, detail);
        markdown_out.set(doc_summary.markdown);
    };
    rebuild();

    let export_md = move || {
        let body_md = untracked(|| markdown_out.get());
        let dlg = SaveDialog::new()
            .title(t!("command.highlight_summary"))
            .filter(FileFilter::new(t!("command.highlight_summary"), ["md"]));
        let answer = pick_save_path(dlg);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await {
                let _ = write_atomically(&dest, body_md.as_bytes());
            }
        });
    };

    let body = view()
        .w(px(600.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.highlight_summary"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    segmented()
                        .child(segment_of(
                            move || t!("study.summary.full"),
                            move || !compact.get(),
                            move || {
                                compact.set(false);
                                rebuild();
                            },
                        ))
                        .child(segment_of(
                            move || t!("study.summary.compact"),
                            move || compact.get(),
                            move || {
                                compact.set(true);
                                rebuild();
                            },
                        )),
                ),
        )
        .child(
            scroll().max_h(px(300.0)).child(
                text(move || {
                    let md = markdown_out.get();
                    if md.trim().is_empty() {
                        t!("study.summary.empty")
                    } else {
                        md
                    }
                })
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
                .child(crate::shell::primary_button(
                    "study.summary.export",
                    export_md,
                )),
        );
    list_dialog(t!("command.highlight_summary"), close, card(body))
}

/// Dialog: Assignments with Due Dates, Checklists & Reminders (plan S.14).
pub(crate) fn assignments_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.assignments_open.set(false);
    let title = signal(String::new());
    let course = signal(String::new());
    let days_until_due = signal(String::from("7"));
    let items = signal(Vec::<Assignment>::new());
    let due_alerts = signal(Vec::<String>::new());

    let refresh = move || {
        let store = AssignmentStore::open(assignments_db())
            .unwrap_or_else(|_| AssignmentStore::in_memory().expect("in-memory sqlite"));
        if let Ok(all) = store.list_all() {
            items.set(all);
        }
        if let Ok(mut store_mut) = AssignmentStore::open(assignments_db())
            && let Ok(reminders) = store_mut.poll_due_reminders(now_secs())
        {
            due_alerts.set(reminders.into_iter().map(|r| r.title).collect());
        }
    };
    refresh();

    let add_assignment = move || {
        let ttl = untracked(|| title.get());
        if ttl.trim().is_empty() {
            return;
        }
        let days = untracked(|| days_until_due.get())
            .trim()
            .parse::<i64>()
            .unwrap_or(7);
        let due = now_secs() + days * 86_400;
        let doc_path = state.editing.document().map(|d| d.path().to_path_buf());
        let mut store = AssignmentStore::open(assignments_db())
            .unwrap_or_else(|_| AssignmentStore::in_memory().expect("in-memory sqlite"));
        let course_tag = untracked(|| course.get()).trim().to_owned();
        let full_title = if course_tag.is_empty() {
            ttl.trim().to_owned()
        } else {
            format!("[{course_tag}] {}", ttl.trim())
        };
        let assignment = Assignment {
            id: new_id(),
            title: full_title,
            doc_path,
            page_range: None,
            due_unix_secs: due,
            reminder_lead_secs: 3600,
            reminder_fired_at: None,
            status: AssignmentStatus::Pending,
            checklist: vec![ChecklistItem {
                id: new_id(),
                text: ttl.trim().to_owned(),
                state: CheckState::Open,
            }],
        };
        if store.upsert(&assignment).is_ok() {
            title.set(String::new());
            refresh();
        }
    };

    let body = view()
        .w(px(580.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.assignments"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || title.get()))
                        .flex_1()
                        .on_change(move |s: &str| title.set(s.to_owned()))
                        .placeholder(t!("study.assignments.title_placeholder"))
                        .label(t!("study.assignments.title_placeholder"))
                        .auto_focus(true),
                )
                .child(
                    field(input().value(move || course.get()))
                        .w(px(130.0))
                        .on_change(move |s: &str| course.set(s.to_owned()))
                        .placeholder(t!("study.assignments.course_placeholder"))
                        .label(t!("study.assignments.course_placeholder")),
                )
                .child(
                    field(input().value(move || days_until_due.get()))
                        .w(px(70.0))
                        .on_change(move |s: &str| days_until_due.set(s.to_owned()))
                        .placeholder(t!("study.assignments.days"))
                        .label(t!("study.assignments.days")),
                )
                .child(crate::shell::primary_button(
                    "study.assignments.add",
                    add_assignment,
                )),
        )
        .child(dynamic(move || {
            let alerts = due_alerts.get();
            if alerts.is_empty() {
                view()
            } else {
                view().child(
                    text(t!(
                        "study.assignments.due_alert",
                        titles = alerts.join(", ")
                    ))
                    .text_size(px(12.0))
                    .text_color(move || theme().primary),
                )
            }
        }))
        .child(scroll().max_h(px(240.0)).child(dynamic(move || {
            let list = items.get();
            if list.is_empty() {
                return view().p_3().child(
                    text(move || t!("study.assignments.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            list.into_iter().fold(view().gap_1(), |col, a| {
                let aid = a.id.clone();
                let first_chk = a.checklist.first().cloned();
                let done = a.status == AssignmentStatus::Completed;
                let label = a.title.clone();
                col.child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            nsh_base::button()
                                .accessibility_label(label.clone())
                                .h(px(26.0))
                                .px_2()
                                .rounded(px(4.0))
                                .hover(|s| s.bg(move || theme().muted))
                                .on_press(move || {
                                    if let Some(chk) = first_chk.as_ref()
                                        && let Ok(mut store) =
                                            AssignmentStore::open(assignments_db())
                                    {
                                        let next = if chk.state == CheckState::Done {
                                            CheckState::Open
                                        } else {
                                            CheckState::Done
                                        };
                                        let _ = store.set_checklist_item_state(&aid, &chk.id, next);
                                        refresh();
                                    }
                                })
                                .child(
                                    when(
                                        move || done,
                                        move || {
                                            crate::shell::app_menu::check_mark(13.0, move || {
                                                theme().foreground
                                            })
                                        },
                                    )
                                    .otherwise(move || {
                                        text("○")
                                            .text_size(px(13.0))
                                            .text_color(move || theme().foreground)
                                    }),
                                ),
                        )
                        .child(
                            text(label)
                                .flex_1()
                                .text_size(px(13.0))
                                .text_color(move || theme().foreground),
                        ),
                )
            })
        })))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.assignments"), close, card(body))
}

/// Dialog: Language Learning Tap-to-Lookup, Pronunciation, Deck Capture & Parallel Pane (plan S.15).
pub(crate) fn language_learning_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.language_open.set(false);
    let word = signal(String::new());
    let deck_name = signal(String::from("Language Deck"));
    let lookup_result = signal(String::new());
    let parallel_sample = signal(String::new());
    let status = signal(String::new());

    let inspect_action = move || {
        let w = untracked(|| word.get());
        if w.trim().is_empty() {
            return;
        }
        let doc_path = state
            .editing
            .document()
            .map(|d| d.path().to_string_lossy().into_owned());
        let active_page = state.annotate.active_page();
        let card_info = state.dictionary.with(|idx| {
            inspect_tapped_word(&w, Some(active_page), doc_path.as_deref(), Some(idx), None)
        });
        let meaning_str = card_info
            .meaning
            .as_ref()
            .map(|m| m.definition.clone())
            .unwrap_or_else(|| t!("study.language.no_def"));
        lookup_result.set(format!("{} — {}", card_info.surface_word, meaning_str));

        let tr = SelectionTranslator::new(
            BergamotPackCatalog::load_from_dir(&bergamot_packs_dir())
                .unwrap_or_else(|_| BergamotPackCatalog::new()),
        );
        if let Ok(pane) = ParallelTranslationPane::build(
            active_page,
            std::slice::from_ref(&card_info.surface_word),
            "en",
            "es",
            TranslateEngine::OfflineBergamot,
            ConsentPolicy::OfflineOnly,
            &tr,
        ) {
            parallel_sample.set(
                pane.paragraphs
                    .iter()
                    .map(|p| format!("{}  |  {}", p.source_text, p.translated_text))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        } else {
            parallel_sample.set(format!("{}  |  {}", card_info.surface_word, meaning_str));
        }
    };

    let speak_action = move || {
        let w = untracked(|| word.get());
        if w.trim().is_empty() {
            return;
        }
        let card_info = state
            .dictionary
            .with(|idx| inspect_tapped_word(&w, None, None, Some(idx), None));
        if let Ok(cat) = VoicePackCatalog::open(neural_voices_dir())
            && !cat.voices().is_empty()
        {
            let mut piper = PiperVoiceSynthesizer::new(cat, DeterministicPiperBackend);
            let _ = pronounce_word(&mut piper, &card_info);
            status.set(t!("study.language.pronounced_neural"));
            return;
        }
        match ecclesia_platform::speech::system_synth() {
            Ok(synth_box) => {
                let mut synth = super::DynSpeechSynth(synth_box);
                match pronounce_word(&mut synth, &card_info) {
                    Ok(()) => status.set(t!("study.language.pronounced")),
                    Err(err) => {
                        log::warn!("language learning: pronunciation failed: {err}");
                        status.set(t!("read_aloud.unavailable"));
                    }
                }
            }
            Err(err) => {
                log::warn!("language learning: system speech synthesizer unavailable: {err}");
                status.set(t!("read_aloud.unavailable"));
            }
        }
    };

    let capture_action = move || {
        let w = untracked(|| word.get());
        if w.trim().is_empty() {
            return;
        }
        let dname = untracked(|| deck_name.get());
        let doc_path = state
            .editing
            .document()
            .map(|d| d.path().to_string_lossy().into_owned());
        let page = Some(state.annotate.active_page());
        let card_info = state
            .dictionary
            .with(|idx| inspect_tapped_word(&w, page, doc_path.as_deref(), Some(idx), None));
        if let Some(store) = crate::library::store() {
            let mut deck_id = dname.trim().to_ascii_lowercase().replace(' ', "-");
            if deck_id.is_empty() {
                deck_id = "language-deck".to_owned();
            }
            let dname_owned = dname.trim().to_owned();
            let now = now_secs();
            let front = card_info.surface_word.clone();
            let back = card_info
                .meaning
                .as_ref()
                .map(|m| m.definition.clone())
                .unwrap_or_default();
            let src_path = card_info.source_path.clone();
            let src_page = card_info.source_page;
            if store
                .write(move |tx| {
                    upsert_deck(
                        tx,
                        &DeckRecord {
                            id: deck_id.clone(),
                            name: dname_owned,
                            description: String::new(),
                            source: String::new(),
                            license: String::new(),
                            builtin: false,
                            installed_total: 0,
                            missing_definitions_json: "[]".to_owned(),
                            created: now,
                        },
                    )?;
                    upsert_card(
                        tx,
                        &CardRecord {
                            id: new_id(),
                            deck_id,
                            kind: "vocab".to_owned(),
                            front,
                            back,
                            cloze_ordinal: None,
                            occlusion_json: None,
                            source_path: src_path,
                            source_page: src_page,
                            anchor_json: None,
                            media_json: "[]".to_owned(),
                            tags_json: "[]".to_owned(),
                            extra_json: "{}".to_owned(),
                            created: now,
                            modified: now,
                        },
                    )?;
                    Ok(())
                })
                .wait()
                .is_ok()
            {
                status.set(t!("study.language.captured"));
            }
        }
    };

    let body = view()
        .w(px(540.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.language_learning"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || word.get()))
                        .flex_1()
                        .on_change(move |s: &str| word.set(s.to_owned()))
                        .placeholder(t!("study.language.word_placeholder"))
                        .label(t!("study.language.word_placeholder"))
                        .auto_focus(true),
                )
                .child(crate::shell::primary_button(
                    "study.language.lookup",
                    inspect_action,
                ))
                .child(action_button(
                    "study.language.pronounce",
                    Tone::Plain,
                    speak_action,
                )),
        )
        .child(
            field(input().value(move || deck_name.get()))
                .on_change(move |s: &str| deck_name.set(s.to_owned()))
                .placeholder(t!("study.language.deck_placeholder"))
                .label(t!("study.language.deck_placeholder")),
        )
        .child(
            text(move || lookup_result.get())
                .text_size(px(13.0))
                .text_color(move || theme().foreground),
        )
        .child(
            text(move || parallel_sample.get())
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || status.get())
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "study.language.capture",
                    capture_action,
                )),
        );
    list_dialog(t!("command.language_learning"), close, card(body))
}

/// Dialog: Translate Selection Popover across 73 languages with Bergamot offline packs (plan S.16).
pub(crate) fn translate_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.translate_open.set(false);
    let source_text = signal(String::new());
    let target_idx = signal(3_usize); // default Arabic or quick language switch
    let online_consent = signal(
        settings::shared()
            .flag(TRANSLATE_ONLINE_CONSENT)
            .unwrap_or(false),
    );
    let translated = signal(String::new());

    let run_translate = move || {
        let text_in = untracked(|| source_text.get());
        if text_in.trim().is_empty() {
            return;
        }
        let target_lang = SUPPORTED_LANGUAGES
            .get(untracked(|| target_idx.get()))
            .map_or("es", |l| l.code)
            .to_owned();
        let allowed = untracked(|| online_consent.get());
        let catalog = BergamotPackCatalog::load_from_dir(&bergamot_packs_dir())
            .unwrap_or_else(|_| BergamotPackCatalog::new());
        let translator = SelectionTranslator::new(catalog);
        let req = TranslateRequest {
            text: text_in,
            source_lang: "auto".to_owned(),
            target_lang,
            engine: if allowed {
                TranslateEngine::Google
            } else {
                TranslateEngine::OfflineBergamot
            },
            consent: if allowed {
                ConsentPolicy::OnlineConsented
            } else {
                ConsentPolicy::OfflineOnly
            },
        };
        match translator.translate(&req) {
            Ok(resp) => translated.set(resp.translated_text),
            Err(err) => translated.set(err.to_string()),
        }
    };

    let body = view()
        .w(px(560.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.translate_selection"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("study.translate.target_lang"))
        .child(
            [1_usize, 3, 14, 24, 30, 62]
                .into_iter()
                .fold(segmented(), |bar, idx| {
                    let lang_name = SUPPORTED_LANGUAGES.get(idx).map_or("ES", |l| l.name);
                    bar.child(segment_of(
                        move || lang_name.to_owned(),
                        move || target_idx.get() == idx,
                        move || target_idx.set(idx),
                    ))
                }),
        )
        .child(segment_of(
            move || t!("study.translate.online_consent"),
            move || online_consent.get(),
            move || {
                let next = !untracked(|| online_consent.get());
                online_consent.set(next);
                settings::mutate(|s| s.set_flag(TRANSLATE_ONLINE_CONSENT, Some(next)));
            },
        ))
        .child(
            field(input().value(move || source_text.get()))
                .multiline(true)
                .h(px(90.0))
                .on_change(move |s: &str| source_text.set(s.to_owned()))
                .placeholder(t!("study.translate.placeholder"))
                .label(t!("study.translate.placeholder"))
                .auto_focus(true),
        )
        .child(
            scroll().max_h(px(120.0)).child(
                text(move || translated.get())
                    .text_size(px(13.0))
                    .text_color(move || theme().foreground),
            ),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "study.translate.run",
                    run_translate,
                )),
        );
    list_dialog(t!("command.translate_selection"), close, card(body))
}
