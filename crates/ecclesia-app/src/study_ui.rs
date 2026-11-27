//! Study & Reader Productivity UI (S.1–S.8, S.10–S.12, U.5):
//! - S.1 Make Flashcard from document / selection / highlight with SourceAnchor jump-back
//! - S.2 Image Occlusion card set generator
//! - S.3 Cloze Deletion editor & generator
//! - S.4 + S.5 FSRS-5 Daily Review session & Home summary metrics
//! - S.6 Offline Dictionary lookup & custom wordlist import
//! - S.7 Vocabulary Library, 6 practice activities & Word of the Day
//! - S.8 Study Export (Markdown, Text, Typst, HTML, DOCX, JSON, CSV)
//! - S.10 Exam Mode (hide notes/categories/H.4 tape, countdown timer, reveal & restore)
//! - S.11 Anki `.apkg` Import & Export
//! - S.12 Daily Study Goals, Pomodoro FocusTimer & Statistics dashboard
//! - U.5 Read Aloud bar with Windows SAPI / cross-platform SpeechSynth

pub(crate) mod phase5;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_core::anchor::new_id;
use ecclesia_core::edit::Operation;
use ecclesia_features::dictionary::{DictionaryIndex, OfflineMeaning, meaning_text};
use ecclesia_features::read_aloud::{PlaybackState as ReadAloudPlayState, ReadAloudSession};
use ecclesia_features::study::anki::{export_apkg, import_apkg};
use ecclesia_features::study::cards::{
    CardMedia, Flashcard, ImageOcclusionSpec, NormalizedRect, OcclusionBox, OcclusionCardMeta,
    OcclusionMode, SourceAnchor, wrap_cloze,
};
use ecclesia_features::study::exam::{ExamConfig, ExamSession, VisibilitySnapshot};
use ecclesia_features::study::export::{StudyDocument, StudyFilter, StudyFormat, render_study};
use ecclesia_features::study::review::{ReviewSession, rating_from_key};
use ecclesia_features::study::srs::{CardState, FsrsScheduler, Rating};
use ecclesia_features::study::stats::{
    FocusPhase, FocusTimer, MotionPreference, compute_statistics,
};
use ecclesia_features::vocabulary::{
    BUILTIN_DECKS, NewVocabularyWord, VocabActivity, VocabularyLibrary,
};
use ecclesia_platform::speech::{SpeechError, SpeechEvent, SpeechSynth, VoiceInfo, system_synth};
use ecclesia_store::study::{self, DeckRecord, StudyGoalsRecord};
use ecclesia_text::PageText;
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use crate::annotate::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::commands::CommandId;
use crate::editing::{Editing, show_message};
use crate::library;
use crate::list_dialog::{card, list_dialog};
use crate::settings::write_atomically;
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// Newtype adapter so `Box<dyn SpeechSynth>` satisfies `ReadAloudSession<S: SpeechSynth>` cleanly (U.5).
struct DynSpeechSynth(Box<dyn SpeechSynth>);

impl SpeechSynth for DynSpeechSynth {
    fn voices(&self) -> Result<Vec<VoiceInfo>, SpeechError> {
        self.0.voices()
    }
    fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError> {
        self.0.set_voice(voice_id)
    }
    fn set_rate(&mut self, speed: f32) -> Result<(), SpeechError> {
        self.0.set_rate(speed)
    }
    fn speak(&mut self, text: &str) -> Result<(), SpeechError> {
        self.0.speak(text)
    }
    fn pause(&mut self) -> Result<(), SpeechError> {
        self.0.pause()
    }
    fn resume(&mut self) -> Result<(), SpeechError> {
        self.0.resume()
    }
    fn stop(&mut self) -> Result<(), SpeechError> {
        self.0.stop()
    }
    fn is_speaking(&self) -> bool {
        self.0.is_speaking()
    }
    fn poll_events(&mut self) -> Vec<SpeechEvent> {
        self.0.poll_events()
    }
}

/// Summary metrics shown on the Home page (S.5, S.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct HomeStudySummary {
    pub(crate) due_cards: usize,
    pub(crate) streak_days: u32,
}

/// Queries live SQLite due card count and streak days for the Home page (S.5, S.12).
pub(crate) fn home_study_summary() -> HomeStudySummary {
    let Some(store) = library::store() else {
        return HomeStudySummary::default();
    };
    let now = now_secs();
    let due_cards = store
        .read(|conn| study::due_count(conn, None, now))
        .unwrap_or(0);
    let streak_days = store
        .read(|conn| {
            let reviews = study::list_review_log(conn, None, None)?;
            let reading = study::list_reading_log(conn, None, None)?;
            let goals = study::load_goals(conn)?;
            Ok(compute_statistics(
                &reviews,
                &reading,
                goals,
                now,
                30,
                MotionPreference::Reduced,
            )
            .streak
            .current_days)
        })
        .unwrap_or(0);
    HomeStudySummary {
        due_cards,
        streak_days,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Study, Review, Dictionary, Vocabulary, Exam, and Read Aloud controller state.
#[derive(Clone, Copy)]
pub(crate) struct StudyState {
    editing: Editing,
    annotate: AnnotateState,
    flashcard_open: Signal<bool>,
    occlusion_open: Signal<bool>,
    cloze_open: Signal<bool>,
    review_open: Signal<bool>,
    dict_open: Signal<bool>,
    vocab_open: Signal<bool>,
    export_open: Signal<bool>,
    stats_open: Signal<bool>,
    exam_active: Signal<bool>,
    read_aloud_open: Signal<bool>,
    summaries_open: Signal<bool>,
    assignments_open: Signal<bool>,
    language_open: Signal<bool>,
    translate_open: Signal<bool>,
    exam_session: Stored<Option<ExamSession>>,
    dictionary: Stored<DictionaryIndex>,
    vocabulary: Stored<VocabularyLibrary>,
    vocab_loaded: Stored<bool>,
    focus_timer: Stored<FocusTimer>,
    read_aloud: Stored<Option<ReadAloudSession<DynSpeechSynth>>>,
    spoken_word: Signal<String>,
    read_aloud_status: Signal<String>,
    read_aloud_playing: Signal<bool>,
    read_aloud_speed: Signal<f32>,
}

impl StudyState {
    /// Creates Study & Reader productivity state.
    pub(crate) fn new(editing: Editing, annotate: AnnotateState) -> Self {
        Self {
            editing,
            annotate,
            flashcard_open: signal(false),
            occlusion_open: signal(false),
            cloze_open: signal(false),
            review_open: signal(false),
            dict_open: signal(false),
            vocab_open: signal(false),
            export_open: signal(false),
            stats_open: signal(false),
            exam_active: signal(false),
            read_aloud_open: signal(false),
            summaries_open: signal(false),
            assignments_open: signal(false),
            language_open: signal(false),
            translate_open: signal(false),
            exam_session: stored(None),
            dictionary: stored(DictionaryIndex::new()),
            vocabulary: stored(VocabularyLibrary::default()),
            vocab_loaded: stored(false),
            focus_timer: stored(FocusTimer::default()),
            read_aloud: stored(None),
            spoken_word: signal(String::new()),
            read_aloud_status: signal(String::new()),
            read_aloud_playing: signal(false),
            read_aloud_speed: signal(1.0),
        }
    }

    fn ensure_vocabulary_loaded(self) {
        if self.vocab_loaded.with(|loaded| {
            let was = *loaded;
            *loaded = true;
            was
        }) {
            return;
        }
        if let Some(loaded) = library::store()
            .and_then(|s| {
                s.read(|conn| Ok(VocabularyLibrary::load_from_store(conn)))
                    .ok()
            })
            .and_then(Result::ok)
        {
            self.vocabulary.with(|slot| *slot = loaded);
        }
    }

    /// Whether any Study modal dialog is open (hides native canvas HWND cleanly).
    pub(crate) fn any_dialog_open(self) -> bool {
        self.flashcard_open.get()
            || self.occlusion_open.get()
            || self.cloze_open.get()
            || self.review_open.get()
            || self.dict_open.get()
            || self.vocab_open.get()
            || self.export_open.get()
            || self.stats_open.get()
            || self.summaries_open.get()
            || self.assignments_open.get()
            || self.language_open.get()
            || self.translate_open.get()
    }

    pub(crate) fn is_flashcard_open(self) -> bool {
        self.flashcard_open.get()
    }
    pub(crate) fn is_occlusion_open(self) -> bool {
        self.occlusion_open.get()
    }
    pub(crate) fn is_cloze_open(self) -> bool {
        self.cloze_open.get()
    }
    pub(crate) fn is_review_open(self) -> bool {
        self.review_open.get()
    }
    pub(crate) fn is_dict_open(self) -> bool {
        self.dict_open.get()
    }
    pub(crate) fn is_vocab_open(self) -> bool {
        self.vocab_open.get()
    }
    pub(crate) fn is_export_open(self) -> bool {
        self.export_open.get()
    }
    pub(crate) fn is_stats_open(self) -> bool {
        self.stats_open.get()
    }

    pub(crate) fn open_flashcard(self) {
        self.flashcard_open.set(true);
    }
    pub(crate) fn open_occlusion(self) {
        self.occlusion_open.set(true);
    }
    pub(crate) fn open_cloze(self) {
        self.cloze_open.set(true);
    }
    pub(crate) fn open_review(self) {
        self.review_open.set(true);
    }
    pub(crate) fn open_dictionary(self) {
        self.ensure_vocabulary_loaded();
        self.dict_open.set(true);
    }
    pub(crate) fn open_vocabulary(self) {
        self.ensure_vocabulary_loaded();
        self.vocab_open.set(true);
    }
    pub(crate) fn open_export(self) {
        self.export_open.set(true);
    }
    pub(crate) fn open_stats(self) {
        self.stats_open.set(true);
    }

    /// Starts or exits Exam Mode (S.10): covers H.4 tape, hides categories/notes, and restores on exit.
    pub(crate) fn toggle_exam_mode(self) {
        let active = untracked(|| self.exam_active.get());
        let page = self.annotate.active_page();
        let annots = self.annotate.page_annotations_for_study(page);
        if !active {
            let (sess, tape_changes) = ExamSession::enter(
                &annots,
                VisibilitySnapshot::default(),
                ExamConfig::default(),
                now_secs(),
            );
            if !tape_changes.is_empty() {
                self.editing
                    .apply(&Operation::new(t!("study.exam.enter"), tape_changes));
            }
            self.exam_session.with(|slot| *slot = Some(sess));
            self.exam_active.set(true);
        } else {
            let restore = self.exam_session.with(|slot| {
                slot.take()
                    .map(|sess| sess.exit(&annots).1)
                    .unwrap_or_default()
            });
            if !restore.is_empty() {
                self.editing
                    .apply(&Operation::new(t!("study.exam.exit"), restore));
            }
            self.exam_active.set(false);
        }
    }

    /// Reveals the next hidden item or H.4 tape strip during Exam Mode (S.10).
    pub(crate) fn exam_reveal_next(self) {
        let page = self.annotate.active_page();
        let annots = self.annotate.page_annotations_for_study(page);
        let tape_change = self.exam_session.with(|slot| {
            slot.as_mut()
                .and_then(|s| s.reveal_next(&annots).and_then(|(_, chg)| chg))
        });
        if let Some(chg) = tape_change {
            self.editing
                .apply(&Operation::new(t!("study.exam.reveal"), vec![chg]));
        }
    }

    /// Imports an Anki `.apkg` archive into SQLite `ecclesia_store::study` (S.11).
    pub(crate) fn import_apkg_dialog(self) {
        let dlg = OpenDialog::files()
            .title(t!("command.import_anki"))
            .filter(FileFilter::new(t!("study.anki.filter"), ["apkg"]));
        let answer = pick_paths(dlg);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            match import_apkg(&bytes, "anki-import", now_secs()) {
                Ok(imported) => {
                    let count = imported.cards.len();
                    if let Some(store) = library::store() {
                        let deck_name = imported.deck_name.clone();
                        let _ = store
                            .write(move |tx| {
                                study::ensure_schema(tx)?;
                                study::upsert_deck(
                                    tx,
                                    &DeckRecord {
                                        id: "anki-import".to_owned(),
                                        name: deck_name,
                                        description: String::new(),
                                        source: String::new(),
                                        license: String::new(),
                                        builtin: false,
                                        installed_total: 0,
                                        missing_definitions_json: "[]".to_owned(),
                                        created: now_secs(),
                                    },
                                )?;
                                for c in &imported.cards {
                                    study::upsert_card(tx, &c.to_record())?;
                                    study::save_srs(
                                        tx,
                                        &c.id,
                                        &CardState::new_at(now_secs()).to_record(),
                                    )?;
                                }
                                Ok(())
                            })
                            .wait();
                    }
                    show_message(
                        t!("command.import_anki"),
                        t!("study.anki.imported", count = count),
                    );
                }
                Err(err) => show_message(t!("command.import_anki"), err.to_string()),
            }
        });
    }

    /// Exports all saved flashcards as an Anki `.apkg` package (S.11).
    pub(crate) fn export_apkg_dialog(self) {
        let cards = load_all_flashcards();
        let bytes = match export_apkg("EcclesiaPDF", &cards, now_secs()) {
            Ok(bytes) => bytes,
            Err(err) => {
                show_message(t!("command.export_anki"), err.to_string());
                return;
            }
        };
        let dlg = SaveDialog::new()
            .file_name("EcclesiaPDF Cards.apkg".to_owned())
            .filter(FileFilter::new(t!("study.anki.filter"), ["apkg"]));
        let answer = pick_save_path(dlg);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, bytes) {
                show_message(t!("command.export_anki"), err.to_string());
            }
        });
    }

    /// Opens or toggles the Read Aloud playback bar for the active document (U.5).
    pub(crate) fn toggle_read_aloud(self) {
        let next = !untracked(|| self.read_aloud_open.get());
        self.read_aloud_open.set(next);
        if next {
            self.start_read_aloud();
        } else {
            self.stop_read_aloud();
        }
    }

    /// Initializes and begins Read Aloud from the current page using `system_synth()` or an installed neural voice (U.5).
    pub(crate) fn start_read_aloud(self) {
        let Some(doc) = self.editing.document() else {
            return;
        };
        let page = self.annotate.active_page();
        let page_texts = {
            let Ok(shared) = doc.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            let count = mupdf.page_count().unwrap_or(0);
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let text = mupdf
                    .load_page(i)
                    .and_then(|p| p.text(None))
                    .map(|s| PageText::from(s.layout()))
                    .unwrap_or_default();
                out.push(text);
            }
            out
        };
        let selected_neural = crate::settings::shared()
            .text(phase5::NEURAL_VOICE_ID)
            .map(str::to_owned);
        let synth_box: Box<dyn SpeechSynth> = if let Some(voice_id) = selected_neural
            && let Ok(cat) = ecclesia_features::neural_voices::VoicePackCatalog::open(
                phase5::neural_voices_dir(),
            )
            && cat.get(&voice_id).is_some()
        {
            let mut piper = ecclesia_features::neural_voices::PiperVoiceSynthesizer::new(
                cat,
                ecclesia_features::neural_voices::DeterministicPiperBackend,
            );
            let _ = piper.set_voice(Some(&voice_id));
            Box::new(piper)
        } else {
            match system_synth() {
                Ok(synth) => synth,
                Err(err) => {
                    log::warn!("read aloud: system speech synthesizer unavailable: {err}");
                    self.read_aloud.with(|slot| *slot = None);
                    self.read_aloud_playing.set(false);
                    self.spoken_word.set(String::new());
                    self.read_aloud_status.set(t!("read_aloud.unavailable"));
                    return;
                }
            }
        };
        self.read_aloud_status.set(String::new());
        let mut session = ReadAloudSession::new(DynSpeechSynth(synth_box), page_texts);
        let speed = untracked(|| self.read_aloud_speed.get());
        let _ = session.set_speed(speed);
        let started = session.start_from(page, 0).unwrap_or(false);
        self.read_aloud.with(|slot| *slot = Some(session));
        self.read_aloud_playing.set(started);
        if started {
            spawn(async move {
                loop {
                    let _ =
                        blocking(|| std::thread::sleep(std::time::Duration::from_millis(60))).await;
                    if !untracked(|| self.read_aloud_open.get()) {
                        break;
                    }
                    let keep = self.read_aloud.with(|slot| {
                        let Some(sess) = slot.as_mut() else {
                            return false;
                        };
                        let _ = sess.poll();
                        if let Some(hl) = sess.active_highlight() {
                            self.spoken_word.set(hl.word_text.clone());
                        }
                        let playing = sess.state() == ReadAloudPlayState::Playing;
                        self.read_aloud_playing.set(playing);
                        playing || sess.state() == ReadAloudPlayState::Paused
                    });
                    if !keep {
                        break;
                    }
                }
            });
        }
    }

    pub(crate) fn stop_read_aloud(self) {
        self.read_aloud.with(|slot| {
            if let Some(sess) = slot.as_mut() {
                let _ = sess.stop();
            }
        });
        self.read_aloud_playing.set(false);
    }

    /// Whether a Study/Reader toggle command should show pressed on the toolbar.
    pub(crate) fn is_on(self, id: CommandId) -> bool {
        match id {
            CommandId::ExamMode => self.exam_active.get(),
            CommandId::ReadAloud => self.read_aloud_open.get(),
            _ => false,
        }
    }
}

fn load_all_flashcards() -> Vec<Flashcard> {
    let Some(store) = library::store() else {
        return Vec::new();
    };
    store
        .read(|conn| study::list_cards(conn, None))
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.kind != "vocab")
        .filter_map(|r| Flashcard::from_record(&r))
        .collect()
}

fn save_card(card_item: &Flashcard, srs: CardState) {
    let Some(store) = library::store() else {
        return;
    };
    let rec = card_item.to_record();
    let srs_rec = srs.to_record();
    let deck_id = card_item.deck_id.clone();
    let id = card_item.id.clone();
    let _ = store
        .write(move |tx| {
            study::ensure_schema(tx)?;
            study::upsert_deck(
                tx,
                &DeckRecord {
                    id: deck_id.clone(),
                    name: deck_id,
                    description: String::new(),
                    source: String::new(),
                    license: String::new(),
                    builtin: false,
                    installed_total: 0,
                    missing_definitions_json: "[]".to_owned(),
                    created: now_secs(),
                },
            )?;
            study::upsert_card(tx, &rec)?;
            study::save_srs(tx, &id, &srs_rec)?;
            Ok(())
        })
        .wait();
}

/// Dialog: Make Flashcard with K.28 source anchor (S.1).
pub(crate) fn flashcard_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.flashcard_open.set(false);
    let front = signal(String::new());
    let back = signal(String::new());
    let deck = signal(String::from("default"));

    let create = move || {
        let f = untracked(|| front.get());
        let b = untracked(|| back.get());
        let d = untracked(|| deck.get());
        if f.trim().is_empty() {
            return;
        }
        let now = now_secs();
        let source = state.editing.document().map(|doc| SourceAnchor {
            path: doc.path().to_path_buf(),
            page: state.annotate.active_page(),
            quote: None,
            rect: None,
            annot_id: None,
        });
        let card_item = Flashcard {
            id: new_id(),
            deck_id: if d.trim().is_empty() {
                String::from("default")
            } else {
                d
            },
            kind: ecclesia_features::study::cards::CardKind::Basic,
            front: f,
            back: b,
            source,
            tags: Vec::new(),
            media: Vec::new(),
            created: now,
            modified: now,
        };
        save_card(&card_item, CardState::new_at(now));
        front.set(String::new());
        back.set(String::new());
        close();
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.make_flashcard"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("study.flashcard.front"))
        .child(
            field(input().value(move || front.get()))
                .multiline(true)
                .h(px(72.0))
                .on_change(move |s: &str| front.set(s.to_owned()))
                .label(t!("study.flashcard.front"))
                .auto_focus(true),
        )
        .child(caption("study.flashcard.back"))
        .child(
            field(input().value(move || back.get()))
                .multiline(true)
                .h(px(72.0))
                .on_change(move |s: &str| back.set(s.to_owned()))
                .label(t!("study.flashcard.back")),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("study.save_card", create)),
        );
    list_dialog(t!("command.make_flashcard"), close, card(body))
}

/// Dialog: Image Occlusion card set generator (S.2).
pub(crate) fn occlusion_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.occlusion_open.set(false);
    let header = signal(String::from("Diagram Quiz"));
    let labels = signal(String::from("Nucleus\nMitochondria\nMembrane"));
    let hide_all = signal(true);

    let create = move || {
        let boxes: Vec<OcclusionBox> = untracked(|| labels.get())
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .filter_map(|(i, l)| {
                let top = ((i as f32) * 0.18).min(0.8);
                let rect = NormalizedRect::try_new(0.15, top + 0.05, 0.35, 0.12)?;
                Some(OcclusionBox {
                    id: format!("box-{}", i + 1),
                    rect,
                    label: Some(l.trim().to_owned()),
                })
            })
            .collect();
        if boxes.is_empty() {
            return;
        }
        let spec = ImageOcclusionSpec {
            image_name: String::from("figure.png"),
            boxes,
            mode: if untracked(|| hide_all.get()) {
                OcclusionMode::HideAllGuessOne
            } else {
                OcclusionMode::HideOneGuessOne
            },
        };
        let media = CardMedia {
            name: String::from("figure.png"),
            mime: String::from("image/png"),
            bytes: Vec::new(),
        };
        let hdr = untracked(|| header.get());
        let now = now_secs();
        let meta = OcclusionCardMeta {
            header: &hdr,
            back_extra: "",
            source: state.editing.document().map(|d| SourceAnchor {
                path: d.path().to_path_buf(),
                page: state.annotate.active_page(),
                quote: None,
                rect: None,
                annot_id: None,
            }),
            deck_id: "default",
            tags: &[],
            now,
        };
        let cards = Flashcard::from_image_occlusion(&spec, &media, &meta);
        for c in &cards {
            save_card(c, CardState::new_at(now));
        }
        close();
    };

    let body = view()
        .w(px(440.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.image_occlusion"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || header.get()))
                .on_change(move |s: &str| header.set(s.to_owned()))
                .label(t!("study.occlusion.header")),
        )
        .child(caption("study.occlusion.labels"))
        .child(
            field(input().value(move || labels.get()))
                .multiline(true)
                .h(px(90.0))
                .on_change(move |s: &str| labels.set(s.to_owned()))
                .label(t!("study.occlusion.labels")),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("study.occlusion.hide_all"),
                    move || hide_all.get(),
                    move || hide_all.set(true),
                ))
                .child(segment_of(
                    move || t!("study.occlusion.hide_one"),
                    move || !hide_all.get(),
                    move || hide_all.set(false),
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("study.save_card", create)),
        );
    list_dialog(t!("command.image_occlusion"), close, card(body))
}

/// Dialog: Cloze Deletion card editor (S.3).
pub(crate) fn cloze_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.cloze_open.set(false);
    let cloze_text = signal(String::from(
        "The {{c1::mitochondria}} generates {{c2::ATP}}.",
    ));
    let wrap_last_word = move || {
        let cur = untracked(|| cloze_text.get());
        if !cur.trim().is_empty()
            && !cur.contains("{{c")
            && let Some(wrapped) = wrap_cloze(&cur, 0..cur.len(), 1, None)
        {
            cloze_text.set(wrapped);
        }
    };
    let create = move || {
        let txt = untracked(|| cloze_text.get());
        let now = now_secs();
        let source = state.editing.document().map(|d| SourceAnchor {
            path: d.path().to_path_buf(),
            page: state.annotate.active_page(),
            quote: None,
            rect: None,
            annot_id: None,
        });
        let cards = Flashcard::from_cloze_text(&txt, "", source, "default", &[], now);
        if cards.is_empty() {
            show_message(t!("command.make_cloze"), t!("study.cloze.none"));
            return;
        }
        for c in &cards {
            save_card(c, CardState::new_at(now));
        }
        close();
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.make_cloze"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || cloze_text.get()))
                .multiline(true)
                .h(px(90.0))
                .on_change(move |s: &str| cloze_text.set(s.to_owned()))
                .label(t!("command.make_cloze")),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button(
                    "study.cloze.wrap",
                    Tone::Plain,
                    wrap_last_word,
                ))
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("study.save_card", create)),
        );
    list_dialog(t!("command.make_cloze"), close, card(body))
}

/// Dialog: FSRS-5 Daily Review session with 1-4 keyboard grading and source jump-back (S.4, S.5).
pub(crate) fn daily_review_dialog(
    state: StudyState,
    jump_to: impl Fn(PathBuf, usize) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.review_open.set(false);
    let now = now_secs();
    let due: Vec<(Flashcard, CardState)> = library::store()
        .and_then(|s| {
            s.read(|conn| {
                let rows = study::due_cards(conn, None, now, Some(100))?;
                Ok(rows
                    .into_iter()
                    .filter_map(|(card_rec, srs_rec)| {
                        Flashcard::from_record(&card_rec)
                            .map(|c| (c, CardState::from_record(srs_rec)))
                    })
                    .collect::<Vec<_>>())
            })
            .ok()
        })
        .unwrap_or_default();
    let session = stored(ReviewSession::from_due(due, now, FsrsScheduler::new()));
    let tick = signal(0_u64);

    let grade_rating = move |rating: Rating| {
        let outcome = session.with(|s| s.grade(rating, now_secs(), 2_000));
        if let Some(out) = outcome
            && let Some(store) = library::store()
        {
            let srs_rec = out.state.to_record();
            let card_id = out.log.card_id.clone();
            let log_rec = out.log;
            let _ = store
                .write(move |tx| {
                    study::save_srs(tx, &card_id, &srs_rec)?;
                    study::append_review_log(tx, &log_rec)?;
                    Ok(())
                })
                .wait();
        }
        tick.update(|t| *t += 1);
    };

    let body = view()
        .on_key_down(move |ev: &KeyEvent| {
            if let Some(ch) = ev.key.chars().next()
                && let Some(r) = rating_from_key(ch)
            {
                grade_rating(r);
            }
        })
        .child(dynamic(move || {
            let _ = tick.get();
            let entry_opt = session.with(|s| {
                s.current().map(|e| {
                    (
                        e.card.clone(),
                        s.is_answer_revealed(),
                        s.interval_preview(now_secs()),
                        s.remaining(),
                    )
                })
            });
            let Some((card_item, revealed, preview, remaining)) = entry_opt else {
                return view()
                    .w(px(440.0))
                    .p_4()
                    .gap_3()
                    .child(
                        text(move || t!("study.review.empty"))
                            .text_size(px(14.0))
                            .text_color(move || theme().popover_foreground),
                    )
                    .child(
                        view()
                            .flex_row()
                            .justify_end()
                            .child(crate::shell::primary_button("ink_tools.done", close)),
                    );
            };
            let mut col = view()
                .w(px(500.0))
                .p_4()
                .gap_3()
                .child(
                    view().flex_row().items_center().gap_2().child(
                        text(move || t!("study.review.title", remaining = remaining))
                            .flex_1()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().popover_foreground),
                    ),
                )
                .child(
                    view()
                        .p_3()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .child(
                            text(card_item.rendered_front())
                                .text_size(px(14.0))
                                .text_color(move || theme().popover_foreground),
                        ),
                );
            if let Some(src) = &card_item.source {
                let p = src.path.clone();
                let pg = src.page;
                col = col.child(action_button("study.review.jump", Tone::Plain, move || {
                    close();
                    jump_to(p.clone(), pg);
                }));
            }
            if revealed {
                col = col
                    .child(
                        view()
                            .p_3()
                            .rounded(px(8.0))
                            .bg(move || theme().muted)
                            .child(
                                text(card_item.rendered_back())
                                    .text_size(px(14.0))
                                    .text_color(move || theme().popover_foreground),
                            ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(action_button("study.grade.again", Tone::Plain, move || {
                                grade_rating(Rating::Again);
                            }))
                            .child(action_button("study.grade.hard", Tone::Plain, move || {
                                grade_rating(Rating::Hard);
                            }))
                            .child(action_button("study.grade.good", Tone::Plain, move || {
                                grade_rating(Rating::Good);
                            }))
                            .child(action_button("study.grade.easy", Tone::Plain, move || {
                                grade_rating(Rating::Easy);
                            }))
                            .child(preview.map_or_else(view, |pv| {
                                view().child(
                                    text(format!(
                                        "{}d / {}d / {}d / {}d",
                                        pv.again_days, pv.hard_days, pv.good_days, pv.easy_days
                                    ))
                                    .text_size(px(11.0))
                                    .text_color(move || theme().muted_foreground),
                                )
                            })),
                    );
            } else {
                col = col.child(view().flex_row().justify_end().child(
                    crate::shell::primary_button("study.review.reveal", move || {
                        session.with(ReviewSession::reveal_answer);
                        tick.update(|t| *t += 1);
                    }),
                ));
            }
            col
        }));

    list_dialog(t!("command.daily_review"), close, card(body))
}

/// Dialog: Offline Dictionary lookup (S.6).
pub(crate) fn dictionary_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.dict_open.set(false);
    let query = signal(String::new());
    let results = signal(Vec::<OfflineMeaning>::new());

    let lookup_word = move || {
        let q = untracked(|| query.get());
        let packs_dir = crate::packs::packs_root();
        let found = state
            .dictionary
            .with(|idx| idx.lookup_with_kaikki(&q, &packs_dir))
            .unwrap_or_default();
        results.set(found);
    };

    let import_tsv = move || {
        let dlg = OpenDialog::files()
            .title(t!("study.dict.import"))
            .filter(FileFilter::new(t!("study.dict.filter"), ["tsv", "txt"]));
        let answer = pick_paths(dlg);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(raw) = std::fs::read(&path) else {
                return;
            };
            let res = state
                .dictionary
                .with(|idx| idx.load_tsv(&raw, "local", "Local TSV"));
            if let Err(err) = res {
                show_message(t!("command.dictionary"), err.to_string());
            } else {
                lookup_word();
            }
        });
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.dictionary"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button("study.dict.import", Tone::Plain, import_tsv)),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().flex_1().child(
                        field(input().value(move || query.get()))
                            .on_change(move |s: &str| query.set(s.to_owned()))
                            .on_submit(move |_: &str| lookup_word())
                            .placeholder(t!("study.dict.placeholder"))
                            .label(t!("command.dictionary"))
                            .auto_focus(true),
                    ),
                )
                .child(crate::shell::primary_button(
                    "study.dict.lookup",
                    lookup_word,
                )),
        )
        .child(scroll().max_h(px(280.0)).child(dynamic(move || {
            let list = results.get();
            if list.is_empty() {
                return view().child(
                    text(move || t!("study.dict.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            list.into_iter().fold(view().gap_2(), |col, meaning| {
                let hw = meaning.headword.clone();
                let def = meaning_text(std::slice::from_ref(&meaning));
                let def_save = def.clone();
                col.child(
                    view()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .gap_1()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    text(hw.clone())
                                        .flex_1()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().popover_foreground),
                                )
                                .child(action_button(
                                    "study.dict.save_vocab",
                                    Tone::Plain,
                                    move || {
                                        let _ = state.vocabulary.with(|v| {
                                            v.add(
                                                NewVocabularyWord {
                                                    word: &hw,
                                                    definition: &def_save,
                                                    dictionary_id: "offline",
                                                    ..NewVocabularyWord::default()
                                                },
                                                now_secs(),
                                            )
                                        });
                                        if let Some(store) = library::store() {
                                            let snapshot = state.vocabulary.with(|v| v.clone());
                                            let _ = store.write(move |tx| {
                                                snapshot.save_to_store(tx).map_err(|e| {
                                                    ecclesia_store::Error::Io(
                                                        std::io::Error::other(e.to_string()),
                                                    )
                                                })
                                            });
                                        }
                                    },
                                )),
                        )
                        .child(
                            text(def)
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
            })
        })))
        .child(view().flex_row().justify_end().child(action_button(
            "ink_tools.done",
            Tone::Plain,
            close,
        )));
    list_dialog(t!("command.dictionary"), close, card(body))
}

/// Dialog: Vocabulary Library & 6 practice activities (S.7).
pub(crate) fn vocabulary_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.vocab_open.set(false);
    let activity = signal(VocabActivity::Flashcards);
    let deck_id = signal(String::from(BUILTIN_DECKS[0].id));
    let answer_input = signal(String::new());
    let rev = signal(0_u64);

    let add_word = signal(String::new());
    let add_def = signal(String::new());
    let submit_word = move || {
        let w = untracked(|| add_word.get());
        let d = untracked(|| add_def.get());
        let did = untracked(|| deck_id.get());
        if state
            .vocabulary
            .with(|v| {
                v.add(
                    NewVocabularyWord {
                        word: &w,
                        definition: &d,
                        deck_id: &did,
                        ..NewVocabularyWord::default()
                    },
                    now_secs(),
                )
            })
            .is_ok()
        {
            add_word.set(String::new());
            add_def.set(String::new());
            rev.update(|r| *r += 1);
        }
    };

    let body = view()
        .w(px(560.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.vocabulary"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(dynamic(move || {
            let _ = rev.get();
            let wotd = state
                .vocabulary
                .with(|v| v.word_of_day(now_secs()).cloned());
            match wotd {
                Some(w) => view()
                    .p_2()
                    .rounded(px(6.0))
                    .bg(move || theme().muted)
                    .child(
                        text(t!("study.vocab.wotd", word = w.word, def = w.definition))
                            .text_size(px(12.0))
                            .text_color(move || theme().popover_foreground),
                    ),
                None => view(),
            }
        }))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("study.vocab.act.flashcards"),
                    move || activity.get() == VocabActivity::Flashcards,
                    move || activity.set(VocabActivity::Flashcards),
                ))
                .child(segment_of(
                    move || t!("study.vocab.act.meaning"),
                    move || activity.get() == VocabActivity::MeaningChoice,
                    move || activity.set(VocabActivity::MeaningChoice),
                ))
                .child(segment_of(
                    move || t!("study.vocab.act.word"),
                    move || activity.get() == VocabActivity::WordChoice,
                    move || activity.set(VocabActivity::WordChoice),
                ))
                .child(segment_of(
                    move || t!("study.vocab.act.spelling"),
                    move || activity.get() == VocabActivity::Spelling,
                    move || activity.set(VocabActivity::Spelling),
                ))
                .child(segment_of(
                    move || t!("study.vocab.act.scramble"),
                    move || activity.get() == VocabActivity::WordScramble,
                    move || activity.set(VocabActivity::WordScramble),
                ))
                .child(segment_of(
                    move || t!("study.vocab.act.match"),
                    move || activity.get() == VocabActivity::MatchPairs,
                    move || activity.set(VocabActivity::MatchPairs),
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().w(px(140.0)).child(
                        field(input().value(move || add_word.get()))
                            .on_change(move |s: &str| add_word.set(s.to_owned()))
                            .placeholder(t!("study.vocab.word_ph"))
                            .label(t!("study.vocab.word_ph")),
                    ),
                )
                .child(
                    view().flex_1().child(
                        field(input().value(move || add_def.get()))
                            .on_change(move |s: &str| add_def.set(s.to_owned()))
                            .placeholder(t!("study.vocab.def_ph"))
                            .label(t!("study.vocab.def_ph")),
                    ),
                )
                .child(action_button("study.vocab.add", Tone::Plain, submit_word)),
        )
        .child(dynamic(move || {
            let _ = rev.get();
            let act = activity.get();
            let first_id = state
                .vocabulary
                .with(|v| v.words().first().map(|w| w.id.clone()));
            let Some(id) = first_id else {
                return view().child(
                    text(move || t!("study.vocab.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            };
            let q = state.vocabulary.with(|v| v.make_question(&id, act));
            match q {
                Ok(question) => view()
                    .p_3()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .gap_2()
                    .child(
                        text(question.prompt.clone())
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().popover_foreground),
                    )
                    .child(
                        field(input().value(move || answer_input.get()))
                            .on_change(move |s: &str| answer_input.set(s.to_owned()))
                            .placeholder(question.answer.clone())
                            .label(t!("study.vocab.answer")),
                    ),
                Err(err) => view().child(
                    text(err.to_string())
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.vocabulary"), close, card(body))
}

/// Dialog: Export Study Notes in 7 formats (S.8).
pub(crate) fn study_export_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.export_open.set(false);
    let fmt = signal(StudyFormat::Markdown);

    let export = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let format = untracked(|| fmt.get());
        let title = doc
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Study Notes")
            .to_owned();
        let pages = doc
            .mupdf()
            .ok()
            .and_then(|s| s.lock().ok().and_then(|m| m.page_count().ok()))
            .unwrap_or(0);
        let study_doc = StudyDocument {
            title: title.clone(),
            source: doc.path().to_string_lossy().into_owned(),
            pages,
            notes: Vec::new(),
        };
        let selected =
            ecclesia_features::study::export::select_notes(&study_doc, StudyFilter::default());
        let bytes = match render_study(&study_doc, &selected, format) {
            Ok(b) => b,
            Err(err) => {
                show_message(t!("command.study_export"), err.to_string());
                return;
            }
        };
        let ext = format.extension();
        let dlg = SaveDialog::new()
            .file_name(format!("{title}.{ext}"))
            .filter(FileFilter::new(ext.to_uppercase(), [ext]));
        let answer = pick_save_path(dlg);
        close();
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, bytes) {
                show_message(t!("command.study_export"), err.to_string());
            }
        });
    };

    let format_chips = StudyFormat::ALL.into_iter().fold(segmented(), |g, f| {
        g.child(segment_of(
            move || f.extension().to_uppercase(),
            move || fmt.get() == f,
            move || fmt.set(f),
        ))
    });

    let body = view()
        .w(px(480.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.study_export"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(format_chips)
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("study.export.save", export)),
        );
    list_dialog(t!("command.study_export"), close, card(body))
}

/// Dialog: Study Goals, Focus Timer & Live Statistics (S.12).
pub(crate) fn statistics_dialog(state: StudyState) -> impl IntoElement {
    let close = move || state.stats_open.set(false);
    let rev = signal(0_u64);
    let goal_cards = signal(String::from("20"));
    let goal_pages = signal(String::from("10"));
    let goal_mins = signal(String::from("30"));

    let save_goals = move || {
        let cards = untracked(|| goal_cards.get())
            .trim()
            .parse::<u32>()
            .unwrap_or(20);
        let pages = untracked(|| goal_pages.get())
            .trim()
            .parse::<u32>()
            .unwrap_or(10);
        let mins = untracked(|| goal_mins.get())
            .trim()
            .parse::<u32>()
            .unwrap_or(30);
        if let Some(store) = library::store() {
            let _ = store
                .write(move |tx| {
                    study::ensure_schema(tx)?;
                    study::save_goals(
                        tx,
                        StudyGoalsRecord {
                            daily_cards: cards,
                            daily_pages: pages,
                            daily_minutes: mins,
                        },
                    )?;
                    Ok(())
                })
                .wait();
        }
        rev.update(|r| *r += 1);
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.study_statistics"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(dynamic(move || {
            let _ = rev.get();
            let now = now_secs();
            let (reviews, reading, goals) = library::store()
                .and_then(|s| {
                    s.read(|conn| {
                        Ok((
                            study::list_review_log(conn, None, None)?,
                            study::list_reading_log(conn, None, None)?,
                            study::load_goals(conn)?,
                        ))
                    })
                    .ok()
                })
                .unwrap_or_else(|| (Vec::new(), Vec::new(), StudyGoalsRecord::default()));
            let stats =
                compute_statistics(&reviews, &reading, goals, now, 7, MotionPreference::Full);
            view()
                .p_3()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || theme().border)
                .gap_1()
                .child(
                    text(t!(
                        "study.stats.summary",
                        streak = stats.streak.current_days,
                        reviews = stats.total_reviews,
                        retention = (stats.retention_rate * 100.0).round() as u32,
                        pages = stats.total_pages_read
                    ))
                    .text_size(px(13.0))
                    .text_color(move || theme().popover_foreground),
                )
        }))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || {
                        let phase = state.focus_timer.with(|t| match t.phase() {
                            FocusPhase::Idle => t!("study.focus.idle"),
                            FocusPhase::Focusing => t!("study.focus.work"),
                            FocusPhase::Break => t!("study.focus.break"),
                        });
                        let secs = state.focus_timer.with(|t| t.remaining_secs(now_secs()));
                        format!("{phase} - {:02}:{:02}", secs / 60, secs % 60)
                    })
                    .flex_1()
                    .text_size(px(13.0))
                    .tabular_nums(),
                )
                .child(action_button("study.focus.start", Tone::Plain, move || {
                    state.focus_timer.with(|t| t.start(now_secs()));
                    rev.update(|r| *r += 1);
                }))
                .child(action_button("study.focus.reset", Tone::Plain, move || {
                    state.focus_timer.with(FocusTimer::reset);
                    rev.update(|r| *r += 1);
                })),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().flex_1().child(
                        field(input().value(move || goal_cards.get()))
                            .on_change(move |s: &str| goal_cards.set(s.to_owned()))
                            .label(t!("study.goals.cards")),
                    ),
                )
                .child(
                    view().flex_1().child(
                        field(input().value(move || goal_pages.get()))
                            .on_change(move |s: &str| goal_pages.set(s.to_owned()))
                            .label(t!("study.goals.pages")),
                    ),
                )
                .child(
                    view().flex_1().child(
                        field(input().value(move || goal_mins.get()))
                            .on_change(move |s: &str| goal_mins.set(s.to_owned()))
                            .label(t!("study.goals.minutes")),
                    ),
                )
                .child(action_button("study.goals.save", Tone::Plain, save_goals)),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.study_statistics"), close, card(body))
}

/// Floating Read Aloud & Exam Mode bars above the document stage (U.5, S.10).
pub(crate) fn study_bars(state: StudyState) -> impl IntoElement {
    view()
        .gap_0()
        .child(dynamic(move || {
            if !state.read_aloud_open.get() {
                return view();
            }
            let playing = state.read_aloud_playing.get();
            let status_msg = state.read_aloud_status.get();
            let cur_word = if status_msg.is_empty() {
                state.spoken_word.get()
            } else {
                status_msg
            };
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py(px(6.0))
                .border_b_1()
                .border_b_color(move || theme().border)
                .bg(move || theme().muted)
                .child(
                    text(move || t!("command.read_aloud"))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(action_button(
                    if playing {
                        "study.read_aloud.pause"
                    } else {
                        "study.read_aloud.play"
                    },
                    Tone::Plain,
                    move || {
                        let has_session = state.read_aloud.with(|slot| {
                            if let Some(s) = slot.as_mut() {
                                if s.state() == ReadAloudPlayState::Playing {
                                    let _ = s.pause();
                                    state.read_aloud_playing.set(false);
                                } else if s.state() == ReadAloudPlayState::Paused {
                                    let _ = s.resume();
                                    state.read_aloud_playing.set(true);
                                }
                                true
                            } else {
                                false
                            }
                        });
                        if !has_session {
                            state.read_aloud_status.set(t!("read_aloud.unavailable"));
                        }
                    },
                ))
                .child(
                    segmented()
                        .child(segment_of(
                            move || String::from("1.0×"),
                            move || (state.read_aloud_speed.get() - 1.0).abs() < 0.05,
                            move || {
                                state.read_aloud_speed.set(1.0);
                                state.read_aloud.with(|s| {
                                    if let Some(sess) = s.as_mut() {
                                        let _ = sess.set_speed(1.0);
                                    }
                                });
                            },
                        ))
                        .child(segment_of(
                            move || String::from("1.5×"),
                            move || (state.read_aloud_speed.get() - 1.5).abs() < 0.05,
                            move || {
                                state.read_aloud_speed.set(1.5);
                                state.read_aloud.with(|s| {
                                    if let Some(sess) = s.as_mut() {
                                        let _ = sess.set_speed(1.5);
                                    }
                                });
                            },
                        ))
                        .child(segment_of(
                            move || String::from("2.0×"),
                            move || (state.read_aloud_speed.get() - 2.0).abs() < 0.05,
                            move || {
                                state.read_aloud_speed.set(2.0);
                                state.read_aloud.with(|s| {
                                    if let Some(sess) = s.as_mut() {
                                        let _ = sess.set_speed(2.0);
                                    }
                                });
                            },
                        )),
                )
                .child(
                    text(cur_word)
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(action_button("ink_tools.done", Tone::Plain, move || {
                    state.toggle_read_aloud();
                }))
        }))
        .child(dynamic(move || {
            if !state.exam_active.get() {
                return view();
            }
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py(px(6.0))
                .border_b_1()
                .border_b_color(move || theme().border)
                .bg(move || theme().muted)
                .child(
                    text(move || t!("command.exam_mode"))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(action_button("study.exam.reveal", Tone::Plain, move || {
                    state.exam_reveal_next();
                }))
                .child(view().flex_1())
                .child(action_button("study.exam.exit", Tone::Plain, move || {
                    state.toggle_exam_mode();
                }))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsrs_daily_review_and_apkg_round_trip() {
        let card = Flashcard {
            id: "c1".to_owned(),
            deck_id: "default".to_owned(),
            kind: ecclesia_features::study::cards::CardKind::Basic,
            front: "Capital of France?".to_owned(),
            back: "Paris".to_owned(),
            source: None,
            tags: vec![],
            media: vec![],
            created: 1_000,
            modified: 1_000,
        };
        let bytes = export_apkg("Geo", &[card], 1_000).expect("export apkg");
        let imported = import_apkg(&bytes, "geo", 1_000).expect("import apkg");
        assert_eq!(imported.cards.len(), 1);
    }

    #[test]
    fn cloze_wrap_and_dictionary_lookup_work() {
        let wrapped = wrap_cloze("mitochondria", 0..12, 1, None);
        assert_eq!(wrapped.as_deref(), Some("{{c1::mitochondria}}"));
        let mut dict = DictionaryIndex::new();
        assert!(dict.add_entry("run", "move rapidly", "test", "Test Dict", None));
        let found = dict.lookup("running");
        assert!(!found.is_empty());
    }
}
