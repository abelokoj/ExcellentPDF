//! Language learning: tap-to-lookup + two-tap vocabulary capture to FSRS decks, pronunciation
//! TTS trigger, and paragraph-aligned parallel translation pane (plan S.15).

use std::fmt;

use excellent_core::anchor::{Quote, new_id};
use excellent_platform::speech::{SpeechError, SpeechSynth};
use excellent_store::study::{CardRecord, DeckRecord, upsert_card, upsert_deck};
use rusqlite::Connection;
use serde_json::json;

use crate::dictionary::{DictionaryIndex, OfflineMeaning, word_key};
use crate::translate::{ConsentPolicy, TranslateEngine, TranslateRequest, Translator};

/// Errors produced by language-learning lookup, deck capture, or parallel translation.
#[derive(Debug)]
#[non_exhaustive]
pub enum LanguageError {
    /// SQLite persistence failure.
    Sqlite(rusqlite::Error),
    /// Store failure.
    Store(excellent_store::Error),
    /// Speech pronunciation playback error.
    Speech(SpeechError),
    /// Translation backend error.
    Translate(crate::translate::TranslateError),
}

impl fmt::Display for LanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "sqlite error: {err}"),
            Self::Store(err) => write!(f, "vocabulary store error: {err}"),
            Self::Speech(err) => write!(f, "pronunciation error: {err}"),
            Self::Translate(err) => write!(f, "translation error: {err}"),
        }
    }
}

impl std::error::Error for LanguageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Store(err) => Some(err),
            Self::Speech(err) => Some(err),
            Self::Translate(err) => Some(err),
        }
    }
}

impl From<rusqlite::Error> for LanguageError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<excellent_store::Error> for LanguageError {
    fn from(err: excellent_store::Error) -> Self {
        Self::Store(err)
    }
}

impl From<SpeechError> for LanguageError {
    fn from(err: SpeechError) -> Self {
        Self::Speech(err)
    }
}

impl From<crate::translate::TranslateError> for LanguageError {
    fn from(err: crate::translate::TranslateError) -> Self {
        Self::Translate(err)
    }
}

/// Convenience result alias for language-learning operations.
pub type Result<T> = std::result::Result<T, LanguageError>;

/// Popover model built when a reader taps a word in language-learning mode (plan S.15 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordTapCard {
    /// Surface word as tapped on the page.
    pub surface_word: String,
    /// Normalized dictionary lookup key.
    pub normalized_key: String,
    /// Offline dictionary meaning, if found.
    pub meaning: Option<OfflineMeaning>,
    /// Optional translated gloss from S.16.
    pub translation: Option<String>,
    /// Optional zero-based source page index.
    pub source_page: Option<usize>,
    /// Optional source document path.
    pub source_path: Option<String>,
}

/// Builds a [`WordTapCard`] from a tapped `word` using the offline dictionary (`index`) and an
/// optional [`Translator`].
#[must_use]
pub fn inspect_tapped_word(
    word: &str,
    source_page: Option<usize>,
    source_path: Option<&str>,
    index: Option<&mut DictionaryIndex>,
    translator: Option<(&dyn Translator, &str, &str)>,
) -> WordTapCard {
    let key = word_key(word).unwrap_or_else(|| word.trim().to_ascii_lowercase());
    let meaning = index.and_then(|idx| idx.lookup(&key).into_iter().next());
    let translation = translator.and_then(|(tr, src_lang, dst_lang)| {
        tr.translate(&TranslateRequest {
            text: word.to_owned(),
            source_lang: src_lang.to_owned(),
            target_lang: dst_lang.to_owned(),
            engine: TranslateEngine::OfflineBergamot,
            consent: ConsentPolicy::OfflineOnly,
        })
        .ok()
        .map(|r| r.translated_text)
    });
    WordTapCard {
        surface_word: word.trim().to_owned(),
        normalized_key: key,
        meaning,
        translation,
        source_page,
        source_path: source_path.map(str::to_owned),
    }
}

/// Speaks the tapped word's pronunciation through the active TTS engine (plan S.15 step 1 / U.5).
///
/// # Errors
///
/// Propagates [`LanguageError::Speech`] if the synthesizer fails.
pub fn pronounce_word<S: SpeechSynth>(synth: &mut S, card: &WordTapCard) -> Result<()> {
    synth.speak(&card.surface_word)?;
    Ok(())
}

/// Adds a tapped word into a study vocabulary deck in SQLite (completing the "two-tap page-to-deck"
/// workflow of plan S.15).
///
/// # Errors
///
/// Returns [`LanguageError::Sqlite`] or [`LanguageError::Store`] if SQLite fails.
pub fn capture_word_to_deck(
    conn: &mut Connection,
    deck_id: &str,
    deck_name: &str,
    card: &WordTapCard,
    quote: Option<&Quote>,
    now_secs: i64,
) -> Result<CardRecord> {
    let tx = conn.transaction()?;
    upsert_deck(
        &tx,
        &DeckRecord {
            id: deck_id.to_owned(),
            name: deck_name.to_owned(),
            description: String::new(),
            source: String::new(),
            license: String::new(),
            builtin: false,
            installed_total: 0,
            missing_definitions_json: "[]".to_owned(),
            created: now_secs,
        },
    )?;

    let mut back_parts = Vec::new();
    if let Some(m) = &card.meaning {
        if !m.part_of_speech.is_empty() || !m.phonetic.is_empty() {
            back_parts.push(
                format!("{} {}", m.part_of_speech, m.phonetic)
                    .trim()
                    .to_owned(),
            );
        }
        if !m.definition.is_empty() {
            back_parts.push(m.definition.clone());
        }
        if !m.example.is_empty() {
            back_parts.push(format!("Example: {}", m.example));
        }
    }
    if let Some(tr) = &card.translation
        && !tr.is_empty()
    {
        back_parts.push(format!("Translation: {tr}"));
    }

    let anchor_json = quote.map(|q| {
        json!({
            "prefix": q.prefix,
            "exact": q.exact,
            "suffix": q.suffix,
            "start": q.start,
        })
        .to_string()
    });

    let record = CardRecord {
        id: new_id(),
        deck_id: deck_id.to_owned(),
        kind: "vocab".to_owned(),
        front: card.surface_word.clone(),
        back: back_parts.join("\n"),
        cloze_ordinal: None,
        occlusion_json: None,
        source_path: card.source_path.clone(),
        source_page: card.source_page,
        anchor_json,
        media_json: "[]".to_owned(),
        tags_json: "[]".to_owned(),
        extra_json: json!({
            "normalized_key": card.normalized_key,
            "dictionary_id": card.meaning.as_ref().map_or("", |m| m.dictionary_id.as_str()),
        })
        .to_string(),
        created: now_secs,
        modified: now_secs,
    };

    upsert_card(&tx, &record)?;
    tx.commit()?;
    Ok(record)
}

/// One paragraph pair in a [`ParallelTranslationPane`] (plan S.15 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedParagraph {
    /// Zero-based paragraph index shared by both panes.
    pub index: usize,
    /// Original source paragraph text.
    pub source_text: String,
    /// Paragraph-aligned translation text.
    pub translated_text: String,
}

/// Parallel translation side-pane state keeping source and target paragraphs 1-to-1 aligned
/// across split panes (plan S.15 step 2 / V.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTranslationPane {
    /// Zero-based page index currently displayed.
    pub page: usize,
    /// Source BCP-47 language code.
    pub source_lang: String,
    /// Target BCP-47 language code.
    pub target_lang: String,
    /// Aligned paragraphs in reading order.
    pub paragraphs: Vec<AlignedParagraph>,
    /// Currently focused/active paragraph index synchronized between panes.
    pub active_paragraph: usize,
}

impl ParallelTranslationPane {
    /// Builds a paragraph-aligned parallel translation pane for `source_paragraphs` on `page`.
    ///
    /// # Errors
    ///
    /// Returns [`LanguageError::Translate`] if any non-empty paragraph fails to translate.
    pub fn build(
        page: usize,
        source_paragraphs: &[String],
        source_lang: &str,
        target_lang: &str,
        engine: TranslateEngine,
        consent: ConsentPolicy,
        translator: &dyn Translator,
    ) -> Result<Self> {
        let mut aligned = Vec::with_capacity(source_paragraphs.len());
        for (index, raw) in source_paragraphs.iter().enumerate() {
            let trimmed = raw.trim();
            let translated_text = if trimmed.is_empty() {
                String::new()
            } else {
                translator
                    .translate(&TranslateRequest {
                        text: trimmed.to_owned(),
                        source_lang: source_lang.to_owned(),
                        target_lang: target_lang.to_owned(),
                        engine,
                        consent,
                    })?
                    .translated_text
            };
            aligned.push(AlignedParagraph {
                index,
                source_text: raw.clone(),
                translated_text,
            });
        }
        Ok(Self {
            page,
            source_lang: source_lang.to_owned(),
            target_lang: target_lang.to_owned(),
            paragraphs: aligned,
            active_paragraph: 0,
        })
    }

    /// Focuses `paragraph_index` (clamped to valid bounds) and returns the aligned pair.
    pub fn focus_paragraph(&mut self, paragraph_index: usize) -> Option<&AlignedParagraph> {
        if self.paragraphs.is_empty() {
            return None;
        }
        self.active_paragraph = paragraph_index.min(self.paragraphs.len() - 1);
        self.paragraphs.get(self.active_paragraph)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use excellent_platform::speech::MockSpeechSynth;
    use excellent_store::study::{ensure_schema, list_cards};

    use super::*;
    use crate::translate::{BergamotPack, BergamotPackCatalog, SelectionTranslator};

    #[test]
    fn word_goes_from_page_to_deck_and_pronounces() {
        let mut dict = DictionaryIndex::new();
        let detail = OfflineMeaning {
            part_of_speech: "adjective".into(),
            phonetic: "/ɪˈfɛm.ər.əl/".into(),
            example: "ephemeral pleasures".into(),
            ..OfflineMeaning::default()
        };
        assert!(dict.add_entry(
            "ephemeral",
            "lasting for a very short time",
            "wordnet-en",
            "WordNet",
            Some(&detail),
        ));

        let tap_card = inspect_tapped_word("Ephemeral", Some(4), None, Some(&mut dict), None);
        assert_eq!(tap_card.normalized_key, "ephemeral");
        assert!(tap_card.meaning.is_some());

        let mut synth = MockSpeechSynth::default();
        pronounce_word(&mut synth, &tap_card).expect("pronounce");
        assert!(synth.is_speaking());

        let mut conn = Connection::open_in_memory().expect("sqlite");
        conn.execute_batch("CREATE TABLE IF NOT EXISTS file (id INTEGER PRIMARY KEY);")
            .expect("file table");
        ensure_schema(&conn).expect("study schema");
        let saved = capture_word_to_deck(
            &mut conn,
            "my-reading",
            "My Reading",
            &tap_card,
            None,
            1_700_000_000,
        )
        .expect("capture");
        assert_eq!(saved.front, "Ephemeral");
        let cards = list_cards(&conn, Some("my-reading")).expect("list");
        assert_eq!(cards.len(), 1);
        assert!(cards[0].back.contains("lasting for a very short time"));
    }

    #[test]
    fn parallel_pane_aligns_source_and_translated_paragraphs() {
        let mut catalog = BergamotPackCatalog::new();
        let mut table = BTreeMap::new();
        table.insert("hello".into(), "hola".into());
        table.insert("world".into(), "mundo".into());
        catalog.register_pack(BergamotPack {
            source_lang: "en".into(),
            target_lang: "es".into(),
            path: PathBuf::from("packs/translate/en-es"),
            phrase_table: table,
        });
        let translator = SelectionTranslator::new(catalog);

        let mut pane = ParallelTranslationPane::build(
            0,
            &["hello".into(), "world".into()],
            "en",
            "es",
            TranslateEngine::OfflineBergamot,
            ConsentPolicy::OfflineOnly,
            &translator,
        )
        .expect("build parallel pane");
        assert_eq!(pane.paragraphs.len(), 2);
        assert_eq!(pane.paragraphs[0].translated_text, "hola");
        assert_eq!(pane.paragraphs[1].translated_text, "mundo");
        let focused = pane.focus_paragraph(1).expect("focus");
        assert_eq!(focused.index, 1);
        assert_eq!(focused.translated_text, "mundo");
    }
}
