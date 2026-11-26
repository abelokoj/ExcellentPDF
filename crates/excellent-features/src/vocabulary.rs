//! Vocabulary library and practice activities (plan S.7), ported from SumatraPDF Enhanced's `Vocabulary.cpp` onto
//! FSRS-5 (`crate::study::srs`).
//!
//! Supports the 11 named study decks, all six practice activities (flashcards, meaning choice, word choice, spelling,
//! word scramble, matching pairs), undoable word removal, deterministic word of the day, SQLite persistence via
//! `excellent_store::study`, and lossless import of SumatraPDF Enhanced's `SumatraPDF-vocabulary.json` (plain or
//! gzipped) keeping every word and its exact due dates.

use std::fmt;
use std::io::Read;

use excellent_core::anchor;
use excellent_store::study::{self, CardRecord, DeckRecord};
use flate2::read::GzDecoder;
use rusqlite::{Connection, Transaction};
use serde_json::{Value, json};

use crate::study::srs::{
    CardPhase, CardState, EnhancedScheduling, FsrsScheduler, Rating, ReviewOutcome,
    SECONDS_PER_DAY, migrate_enhanced_state,
};

/// Maximum saved vocabulary words (matching Enhanced's `kMaxVocabWords`).
pub const MAX_VOCAB_WORDS: usize = 20_000;

/// Maximum vocabulary decks (matching Enhanced's `kMaxVocabDecks`).
pub const MAX_VOCAB_DECKS: usize = 200;

/// Maximum JSON import payload size (64 MiB, matching Enhanced's `kMaxVocabBytes`).
pub const MAX_VOCAB_BYTES: usize = 64 * 1024 * 1024;

/// Maximum undoable removals kept in memory (matching Enhanced's `kMaxRemovedWords`).
const MAX_REMOVED_UNDO: usize = 20;

/// Metadata for one of Enhanced's 11 built-in vocabulary study decks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinDeckInfo {
    /// Stable deck id (e.g. `"ws1"`).
    pub id: &'static str,
    /// Display name (e.g. `"Word Smart 1"`).
    pub name: &'static str,
    /// Short description.
    pub description: &'static str,
}

/// The 11 named study decks ported from SumatraPDF Enhanced's `VocabularyDecks.h`.
pub const BUILTIN_DECKS: [BuiltinDeckInfo; 11] = [
    BuiltinDeckInfo {
        id: "ws1",
        name: "Word Smart 1",
        description: "Princeton Review Word Smart, first list.",
    },
    BuiltinDeckInfo {
        id: "ws2",
        name: "Word Smart 2",
        description: "Princeton Review Word Smart, second list.",
    },
    BuiltinDeckInfo {
        id: "b333",
        name: "Barron's 333",
        description: "Barron's 333 high-frequency GRE words.",
    },
    BuiltinDeckInfo {
        id: "b800",
        name: "Barron's 800",
        description: "Barron's 800 essential GRE words.",
    },
    BuiltinDeckInfo {
        id: "b1100",
        name: "Barron's 1100",
        description: "Barron's 1100 Words You Need to Know.",
    },
    BuiltinDeckInfo {
        id: "mg1000",
        name: "Magoosh 1000",
        description: "Magoosh GRE vocabulary, 1000 words.",
    },
    BuiltinDeckInfo {
        id: "k900",
        name: "Kaplan 900",
        description: "Kaplan GRE vocabulary, 900 words.",
    },
    BuiltinDeckInfo {
        id: "p700",
        name: "Powerscore 700",
        description: "PowerScore GRE vocabulary, 700 words.",
    },
    BuiltinDeckInfo {
        id: "sn1000",
        name: "SparkNotes 1000",
        description: "SparkNotes GRE vocabulary, 1000 words.",
    },
    BuiltinDeckInfo {
        id: "mp1000",
        name: "Manhattan Prep 1000",
        description: "Manhattan Prep GRE vocabulary, 1000 words.",
    },
    BuiltinDeckInfo {
        id: "gm1100",
        name: "GregMat 1100",
        description: "GregMat GRE vocabulary, 1100 words.",
    },
];

/// The six vocabulary practice activities ported from SumatraPDF Enhanced (plan S.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VocabActivity {
    /// Prompt = word, answer = definition.
    Flashcards,
    /// Prompt = word, pick the matching definition from 2–4 choices.
    MeaningChoice,
    /// Prompt = definition, pick the matching word from 2–4 choices.
    WordChoice,
    /// Prompt = definition, type the exact word.
    Spelling,
    /// Prompt = scrambled letters + definition, type the unscrambled word.
    WordScramble,
    /// Match a set of words with their definitions.
    MatchPairs,
}

/// Due-queue query window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueWindow {
    /// Only words due at or before `now`.
    DueNow,
    /// Include words due within the next 24 hours ("study ahead").
    StudyAhead,
}

/// Import merge policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Merge imported words and decks into the existing library, keeping the most recently reviewed schedule.
    Merge,
    /// Replace all existing words and non-builtin decks with the imported file.
    Replace,
}

/// Filter for [`VocabularyLibrary::search`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnedFilter {
    /// Include both learned and unlearned words.
    All,
    /// Include only words marked learned.
    LearnedOnly,
}

/// Errors produced by vocabulary operations and JSON import/export.
#[derive(Debug)]
#[non_exhaustive]
pub enum VocabError {
    /// Invalid word, duplicate id, size limit exceeded or unsupported JSON schema.
    Invalid(String),
    /// Gzip decompression or I/O error.
    Io(std::io::Error),
    /// SQLite store error.
    Store(excellent_store::Error),
}

impl fmt::Display for VocabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(msg) => write!(f, "vocabulary error: {msg}"),
            Self::Io(e) => write!(f, "vocabulary i/o error: {e}"),
            Self::Store(e) => write!(f, "vocabulary store error: {e}"),
        }
    }
}

impl std::error::Error for VocabError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(_) => None,
            Self::Io(e) => Some(e),
            Self::Store(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for VocabError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<excellent_store::Error> for VocabError {
    fn from(e: excellent_store::Error) -> Self {
        Self::Store(e)
    }
}

/// One saved vocabulary word scheduled with FSRS-5 (plan S.7).
#[derive(Debug, Clone, PartialEq)]
pub struct VocabularyWord {
    /// Stable UUID.
    pub id: String,
    /// Headword as saved.
    pub word: String,
    /// Plain-text definition.
    pub definition: String,
    /// Originating dictionary pack id.
    pub dictionary_id: String,
    /// Context sentence captured while reading.
    pub context: String,
    /// Source document path.
    pub source_path: String,
    /// 1-based page number (`0` when unknown).
    pub page: usize,
    /// Primary deck id (first membership).
    pub deck_id: String,
    /// Every deck id this word belongs to.
    pub deck_ids: Vec<String>,
    /// FSRS-5 scheduling state (authoritative scheduler).
    pub srs: CardState,
    /// Timestamp when the word was added (UTC seconds since 1970).
    pub added_time: i64,
    /// Preserved legacy Enhanced Leitner box (`0..=4`) for export compatibility.
    pub legacy_box: u32,
    /// Preserved legacy Enhanced SM-2 ease (`1.3..=10.0`) for export compatibility.
    pub legacy_ease: f64,
}

impl VocabularyWord {
    /// Whether this word belongs to `deck_id` (an empty `deck_id` matches every word).
    #[must_use]
    pub fn in_deck(&self, deck_id: &str) -> bool {
        deck_id.is_empty() || self.deck_id == deck_id || self.deck_ids.iter().any(|d| d == deck_id)
    }

    fn attach_deck(&mut self, deck: &str) {
        if deck.is_empty() {
            return;
        }
        if self.deck_id.is_empty() {
            self.deck_id = deck.to_owned();
        }
        if !self.deck_ids.iter().any(|d| d == deck) {
            self.deck_ids.push(deck.to_owned());
        }
    }
}

/// A vocabulary study deck (built-in or user-created).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyDeck {
    /// Stable deck id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Source URL or note.
    pub source: String,
    /// Licence attribution.
    pub license: String,
    /// Total words accounted for on installation.
    pub installed_total: u32,
    /// Words from a source pack that lacked definitions at install time.
    pub missing_definitions: Vec<String>,
    /// Whether this is one of the 11 built-in decks.
    pub builtin: bool,
}

/// Parameters for adding a word to the vocabulary library.
#[derive(Debug, Clone, Default)]
pub struct NewVocabularyWord<'a> {
    /// Word text.
    pub word: &'a str,
    /// Definition text.
    pub definition: &'a str,
    /// Originating dictionary id.
    pub dictionary_id: &'a str,
    /// Reading context sentence.
    pub context: &'a str,
    /// Source document path.
    pub source_path: &'a str,
    /// 1-based page number (`0` if none).
    pub page: usize,
    /// Optional deck id.
    pub deck_id: &'a str,
}

/// One generated practice prompt (plan S.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyQuestion {
    /// Target word id.
    pub word_id: String,
    /// Prompt text shown to the learner.
    pub prompt: String,
    /// Expected answer.
    pub answer: String,
    /// Multiple-choice options (populated for `MeaningChoice`, `WordChoice`, `MatchPairs`).
    pub choices: Vec<String>,
    /// Index of the correct option in `choices` (`None` for free-response activities).
    pub correct_index: Option<usize>,
    /// Activity kind.
    pub activity: VocabActivity,
}

/// One pair in a [`VocabActivity::MatchPairs`] board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPair {
    /// Word id.
    pub word_id: String,
    /// Headword.
    pub word: String,
    /// Definition.
    pub definition: String,
}

/// Generated matching-pairs exercise board (plan S.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPairsBoard {
    /// Pairs in display order on the word column.
    pub pairs: Vec<MatchPair>,
    /// Shuffled indices into `pairs` for the definition column.
    pub shuffled_definition_order: Vec<usize>,
}

#[derive(Debug, Clone)]
struct RemovedEntry {
    word: VocabularyWord,
    index: usize,
}

/// In-memory vocabulary library backed by FSRS-5 and convertible to/from SQLite and Enhanced JSON (plan S.7).
#[derive(Debug, Clone)]
pub struct VocabularyLibrary {
    words: Vec<VocabularyWord>,
    decks: Vec<VocabularyDeck>,
    removed: Vec<RemovedEntry>,
    scheduler: FsrsScheduler,
    rng_state: u32,
    revision: u64,
    daily_day: Option<i64>,
    daily_revision: u64,
    daily_word_id: Option<String>,
}

impl Default for VocabularyLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl VocabularyLibrary {
    /// Creates a new vocabulary library pre-populated with Enhanced's 11 built-in study decks.
    #[must_use]
    pub fn new() -> Self {
        let mut lib = Self {
            words: Vec::new(),
            decks: Vec::new(),
            removed: Vec::new(),
            scheduler: FsrsScheduler::new(),
            rng_state: 0x006b_8421,
            revision: 0,
            daily_day: None,
            daily_revision: 0,
            daily_word_id: None,
        };
        lib.ensure_builtin_decks();
        lib
    }

    /// Sets the deterministic PRNG seed used by choice/scramble activities (handy for reproducible tests).
    pub fn set_rng_seed(&mut self, seed: u32) {
        self.rng_state = if seed == 0 { 0x006b_8421 } else { seed };
    }

    /// All saved vocabulary words.
    #[must_use]
    pub fn words(&self) -> &[VocabularyWord] {
        &self.words
    }

    /// All decks (built-in and user-created).
    #[must_use]
    pub fn decks(&self) -> &[VocabularyDeck] {
        &self.decks
    }

    /// Looks up a word by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&VocabularyWord> {
        self.words.iter().find(|w| w.id == id)
    }

    /// Adds a word (or merges into an existing case-insensitive match with the same `dictionary_id`) and returns its id.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if the word is blank/over-length or the 20,000-word cap is reached.
    pub fn add(&mut self, input: NewVocabularyWord<'_>, now: i64) -> Result<String, VocabError> {
        let word = clean_word(input.word)
            .ok_or_else(|| VocabError::Invalid("word is empty or invalid".to_owned()))?;
        if input.definition.len() > 16_000
            || input.context.len() > 8_192
            || input.source_path.len() > 2_048
            || input.dictionary_id.len() > 256
            || input.deck_id.len() > 128
        {
            return Err(VocabError::Invalid(
                "word metadata exceeds allowed limits".to_owned(),
            ));
        }

        if let Some(existing) = self
            .words
            .iter_mut()
            .find(|w| same_word(&w.word, &word) && w.dictionary_id == input.dictionary_id)
        {
            if !input.definition.is_empty() {
                existing.definition = input.definition.to_owned();
            }
            if !input.context.is_empty() {
                existing.context = input.context.to_owned();
            }
            if !input.source_path.is_empty() {
                existing.source_path = input.source_path.to_owned();
                existing.page = input.page;
            }
            existing.attach_deck(input.deck_id);
            let id = existing.id.clone();
            self.bump_revision();
            return Ok(id);
        }

        if self.words.len() >= MAX_VOCAB_WORDS {
            return Err(VocabError::Invalid(
                "vocabulary limit is 20,000 saved words".to_owned(),
            ));
        }

        let id = anchor::new_id();
        let mut entry = VocabularyWord {
            id: id.clone(),
            word,
            definition: input.definition.to_owned(),
            dictionary_id: input.dictionary_id.to_owned(),
            context: input.context.to_owned(),
            source_path: input.source_path.to_owned(),
            page: input.page,
            deck_id: String::new(),
            deck_ids: Vec::new(),
            srs: CardState::new_at(now),
            added_time: now,
            legacy_box: 0,
            legacy_ease: 2.5,
        };
        entry.attach_deck(input.deck_id);
        self.words.push(entry);
        self.bump_revision();
        Ok(id)
    }

    /// Removes a word by id and pushes it onto the undo stack (depth 20). Returns `true` if removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.words.iter().position(|w| w.id == id) else {
            return false;
        };
        let word = self.words.remove(index);
        if self.removed.len() == MAX_REMOVED_UNDO {
            self.removed.remove(0);
        }
        self.removed.push(RemovedEntry { word, index });
        self.bump_revision();
        true
    }

    /// Whether an undoable removal is available.
    #[must_use]
    pub fn can_undo_remove(&self) -> bool {
        !self.removed.is_empty()
    }

    /// Restores the most recently removed word at its original index.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if nothing can be undone, the word/id was re-added, or the library is full.
    pub fn undo_remove(&mut self) -> Result<String, VocabError> {
        let Some(last) = self.removed.last() else {
            return Err(VocabError::Invalid("nothing to undo".to_owned()));
        };
        if self.words.iter().any(|w| {
            w.id == last.word.id
                || (same_word(&w.word, &last.word.word)
                    && w.dictionary_id == last.word.dictionary_id)
        }) {
            return Err(VocabError::Invalid(
                "cannot undo removal because the word was added again".to_owned(),
            ));
        }
        if self.words.len() >= MAX_VOCAB_WORDS {
            return Err(VocabError::Invalid(
                "cannot undo removal at the 20,000 saved words limit".to_owned(),
            ));
        }
        let entry = self.removed.pop().expect("checked non-empty");
        let id = entry.word.id.clone();
        let idx = entry.index.min(self.words.len());
        self.words.insert(idx, entry.word);
        self.bump_revision();
        Ok(id)
    }

    /// Marks a word learned (30-day stability/interval, box 4) or resets it to due immediately.
    pub fn set_learned(&mut self, id: &str, learned: bool, now: i64) -> bool {
        let Some(word) = self.words.iter_mut().find(|w| w.id == id) else {
            return false;
        };
        word.srs.learned = learned;
        if learned {
            word.srs.phase = CardPhase::Review;
            word.srs.stability = word.srs.stability.max(30.0);
            if word.srs.difficulty <= 0.0 {
                word.srs.difficulty = 5.0;
            }
            word.srs.scheduled_days = 30;
            word.srs.due = now.saturating_add(30 * SECONDS_PER_DAY);
            word.legacy_box = 4;
        } else {
            word.srs.phase = CardPhase::New;
            word.srs.scheduled_days = 0;
            word.srs.due = now;
            word.legacy_box = 0;
        }
        self.bump_revision();
        true
    }

    /// Reviews a vocabulary word using the unified FSRS-5 scheduler (plan S.4, S.7) and returns its [`ReviewOutcome`].
    pub fn review(
        &mut self,
        id: &str,
        rating: Rating,
        now: i64,
        duration_ms: u64,
    ) -> Option<ReviewOutcome> {
        let word = self.words.iter_mut().find(|w| w.id == id)?;
        let outcome = self
            .scheduler
            .review(&word.id, word.srs, rating, now, duration_ms);
        word.srs = outcome.state;
        word.legacy_box = match word.srs.scheduled_days {
            0..=2 => 0,
            3..=6 => 1,
            7..=13 => 2,
            14..=29 => 3,
            _ => 4,
        };
        self.bump_revision();
        Some(outcome)
    }

    /// Searches words by substring (case-insensitive across word, definition, context), deck and learned filter.
    #[must_use]
    pub fn search(
        &self,
        query: &str,
        deck_id: &str,
        learned: LearnedFilter,
    ) -> Vec<&VocabularyWord> {
        let q = query.trim().to_lowercase();
        self.words
            .iter()
            .filter(|w| {
                if !w.in_deck(deck_id) {
                    return false;
                }
                if learned == LearnedFilter::LearnedOnly && !w.srs.learned {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                w.word.to_lowercase().contains(&q)
                    || w.definition.to_lowercase().contains(&q)
                    || w.context.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Returns words due within `window` for `deck_id` (empty = all decks), sorted by `(due, word)`.
    #[must_use]
    pub fn due_words(&self, deck_id: &str, now: i64, window: DueWindow) -> Vec<&VocabularyWord> {
        let cutoff = match window {
            DueWindow::DueNow => now,
            DueWindow::StudyAhead => now.saturating_add(SECONDS_PER_DAY),
        };
        let mut out: Vec<&VocabularyWord> = self
            .words
            .iter()
            .filter(|w| w.in_deck(deck_id) && w.srs.due <= cutoff && !w.definition.is_empty())
            .collect();
        out.sort_by(|a, b| {
            a.srs
                .due
                .cmp(&b.srs.due)
                .then_with(|| a.word.to_lowercase().cmp(&b.word.to_lowercase()))
        });
        out
    }

    /// Counts words currently due for `deck_id`.
    #[must_use]
    pub fn due_count(&self, deck_id: &str, now: i64) -> usize {
        self.words
            .iter()
            .filter(|w| w.in_deck(deck_id) && w.srs.due <= now && !w.definition.is_empty())
            .count()
    }

    /// Creates a new custom vocabulary deck.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if the name/description is empty/over-length or the 200-deck cap is reached.
    pub fn create_deck(&mut self, name: &str, description: &str) -> Result<String, VocabError> {
        if name.trim().is_empty() || name.len() > 256 || description.len() > 8_192 {
            return Err(VocabError::Invalid(
                "invalid deck name or description".to_owned(),
            ));
        }
        if self.decks.len() >= MAX_VOCAB_DECKS {
            return Err(VocabError::Invalid("deck limit reached".to_owned()));
        }
        let id = anchor::new_id();
        self.decks.push(VocabularyDeck {
            id: id.clone(),
            name: name.to_owned(),
            description: description.to_owned(),
            source: String::new(),
            license: String::new(),
            installed_total: 0,
            missing_definitions: Vec::new(),
            builtin: false,
        });
        self.bump_revision();
        Ok(id)
    }

    /// Removes a user-created deck (built-in decks cannot be removed) and strips it from all saved and undoable words.
    pub fn remove_deck(&mut self, id: &str) -> bool {
        let Some(pos) = self.decks.iter().position(|d| d.id == id && !d.builtin) else {
            return false;
        };
        self.decks.remove(pos);
        for word in &mut self.words {
            detach_deck(word, id);
        }
        for entry in &mut self.removed {
            detach_deck(&mut entry.word, id);
        }
        self.bump_revision();
        true
    }

    /// Deterministic Word of the Day (matching Enhanced's `VocabularyWordOfDay`), cached per UTC day and revision.
    #[must_use]
    pub fn word_of_day(&mut self, now: i64) -> Option<&VocabularyWord> {
        let day = now.div_euclid(SECONDS_PER_DAY);
        if self.daily_day != Some(day) || self.daily_revision != self.revision {
            self.daily_word_id = self.compute_word_of_day(day);
            self.daily_day = Some(day);
            self.daily_revision = self.revision;
        }
        let id = self.daily_word_id.as_deref()?;
        self.find(id)
    }

    fn compute_word_of_day(&self, day: i64) -> Option<String> {
        if self.words.is_empty() {
            return None;
        }
        let mut ordered: Vec<&VocabularyWord> = self.words.iter().collect();
        ordered.sort_by(|a, b| {
            daily_word_hash(&a.word)
                .cmp(&daily_word_hash(&b.word))
                .then_with(|| a.word.to_lowercase().cmp(&b.word.to_lowercase()))
        });
        let count = ordered.len();
        let start = usize::try_from(day.rem_euclid(count as i64)).unwrap_or(0);
        for offset in 0..count {
            let candidate = ordered[(start + offset) % count];
            if !candidate.srs.learned {
                return Some(candidate.id.clone());
            }
        }
        Some(ordered[start].id.clone())
    }

    /// Builds a practice question for `word_id` in any of the 6 activities (plan S.7).
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if the word is missing/undefined, non-ASCII for `WordScramble`, or fewer than
    /// two distinct words exist for choice activities.
    pub fn make_question(
        &mut self,
        word_id: &str,
        activity: VocabActivity,
    ) -> Result<VocabularyQuestion, VocabError> {
        let Some(target) = self.find(word_id).cloned() else {
            return Err(VocabError::Invalid("word not found".to_owned()));
        };
        if target.definition.is_empty() {
            return Err(VocabError::Invalid("word has no definition".to_owned()));
        }

        let is_meaning_choice = activity == VocabActivity::MeaningChoice;
        let prompt = if matches!(
            activity,
            VocabActivity::Flashcards | VocabActivity::MeaningChoice
        ) {
            target.word.clone()
        } else {
            target.definition.clone()
        };
        let answer = if matches!(
            activity,
            VocabActivity::Flashcards | VocabActivity::MeaningChoice
        ) {
            target.definition.clone()
        } else {
            target.word.clone()
        };

        if activity == VocabActivity::WordScramble {
            if !target.word.is_ascii() {
                return Err(VocabError::Invalid(
                    "word scramble requires ASCII characters".to_owned(),
                ));
            }
            let mut letters: Vec<u8> = target.word.as_bytes().to_vec();
            for i in (1..letters.len()).rev() {
                let j = (self.next_rand() as usize) % (i + 1);
                letters.swap(i, j);
            }
            if letters == target.word.as_bytes() && letters.len() > 1 {
                letters.rotate_left(1);
            }
            let scrambled = String::from_utf8(letters).unwrap_or_else(|_| target.word.clone());
            return Ok(VocabularyQuestion {
                word_id: target.id,
                prompt: format!("{scrambled}\n\n{}", target.definition),
                answer: target.word,
                choices: Vec::new(),
                correct_index: None,
                activity,
            });
        }

        if matches!(
            activity,
            VocabActivity::MeaningChoice | VocabActivity::WordChoice | VocabActivity::MatchPairs
        ) {
            let mut choices = vec![answer.clone()];
            let total = self.words.len();
            if total > 0 {
                let start = (self.next_rand() as usize) % total;
                for offset in 0..total {
                    if choices.len() >= 4 {
                        break;
                    }
                    let alt = &self.words[(start + offset) % total];
                    let option = if is_meaning_choice {
                        &alt.definition
                    } else {
                        &alt.word
                    };
                    if !option.is_empty() && !choices.iter().any(|c| c == option) {
                        choices.push(option.clone());
                    }
                }
            }
            if choices.len() < 2 {
                return Err(VocabError::Invalid(
                    "save at least two different words to start a choice activity".to_owned(),
                ));
            }
            let dest = (self.next_rand() as usize) % choices.len();
            choices.swap(0, dest);
            return Ok(VocabularyQuestion {
                word_id: target.id,
                prompt,
                answer,
                choices,
                correct_index: Some(dest),
                activity,
            });
        }

        Ok(VocabularyQuestion {
            word_id: target.id,
            prompt,
            answer,
            choices: Vec::new(),
            correct_index: None,
            activity,
        })
    }

    /// Builds a [`MatchPairsBoard`] with up to `max_pairs` (minimum 2) words from `deck_id`.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if fewer than two defined words are available.
    pub fn make_match_pairs(
        &mut self,
        deck_id: &str,
        max_pairs: usize,
    ) -> Result<MatchPairsBoard, VocabError> {
        let cap = max_pairs.clamp(2, 8);
        let pool: Vec<MatchPair> = self
            .words
            .iter()
            .filter(|w| w.in_deck(deck_id) && !w.definition.is_empty())
            .take(cap)
            .map(|w| MatchPair {
                word_id: w.id.clone(),
                word: w.word.clone(),
                definition: w.definition.clone(),
            })
            .collect();
        if pool.len() < 2 {
            return Err(VocabError::Invalid(
                "need at least two defined words for matching pairs".to_owned(),
            ));
        }
        let mut order: Vec<usize> = (0..pool.len()).collect();
        for i in (1..order.len()).rev() {
            let j = (self.next_rand() as usize) % (i + 1);
            order.swap(i, j);
        }
        if order.iter().enumerate().all(|(i, &v)| i == v) {
            order.rotate_left(1);
        }
        Ok(MatchPairsBoard {
            pairs: pool,
            shuffled_definition_order: order,
        })
    }

    /// Checks a learner's answer against `question` (trimmed, Unicode case-insensitive, matching Enhanced).
    #[must_use]
    pub fn check_answer(question: &VocabularyQuestion, input: &str) -> bool {
        if input.len() > 16_000 || question.answer.len() > 16_000 {
            return false;
        }
        let a = input.trim();
        let b = question.answer.trim();
        !a.is_empty() && !b.is_empty() && same_word(a, b)
    }

    /// Serializes the library to SumatraPDF Enhanced's `sumatrapdf-vocabulary` v1 JSON format.
    #[must_use]
    pub fn export_json(&self) -> String {
        let decks: Vec<Value> = self
            .decks
            .iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "name": d.name,
                    "description": d.description,
                    "source": d.source,
                    "license": d.license,
                    "installedTotal": d.installed_total,
                    "missingDefinitions": d.missing_definitions,
                })
            })
            .collect();
        let words: Vec<Value> = self
            .words
            .iter()
            .map(|w| {
                json!({
                    "id": w.id,
                    "word": w.word,
                    "definition": w.definition,
                    "dictionaryId": w.dictionary_id,
                    "context": w.context,
                    "sourcePath": w.source_path,
                    "deckId": w.deck_id,
                    "deckIds": w.deck_ids,
                    "page": w.page,
                    "box": w.legacy_box,
                    "repetitions": w.srs.reps,
                    "reviews": w.srs.reps,
                    "lapses": w.srs.lapses,
                    "intervalDays": w.srs.scheduled_days,
                    "ease": w.legacy_ease,
                    "dueTime": w.srs.due,
                    "lastReviewed": w.srs.last_review.unwrap_or(0),
                    "addedTime": w.added_time,
                    "learned": w.srs.learned,
                })
            })
            .collect();
        format!(
            "{}\n",
            json!({
                "format": "sumatrapdf-vocabulary",
                "version": 1,
                "decks": decks,
                "words": words,
            })
        )
    }

    /// Imports an Enhanced `SumatraPDF-vocabulary.json` or `wmkeyboard-vocab` JSON (plain or gzip), migrating every
    /// word's Leitner/SM-2 state onto FSRS-5 while preserving every word and its exact due date (plan S.7 **Done
    /// when**).
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Invalid`] if the file is corrupt, unsupported, exceeds limits, or contains duplicate ids.
    /// On error, the existing library is left untouched.
    pub fn import_enhanced_json(
        &mut self,
        raw: &[u8],
        mode: ImportMode,
        now: i64,
    ) -> Result<usize, VocabError> {
        if raw.is_empty() || raw.len() > MAX_VOCAB_BYTES {
            return Err(VocabError::Invalid(
                "vocabulary file is empty or exceeds 64 MB".to_owned(),
            ));
        }
        let unpacked = if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
            let mut dec = GzDecoder::new(raw);
            let mut buf = Vec::new();
            dec.by_ref()
                .take(MAX_VOCAB_BYTES as u64 + 1)
                .read_to_end(&mut buf)?;
            if buf.len() > MAX_VOCAB_BYTES {
                return Err(VocabError::Invalid(
                    "compressed vocabulary exceeds 64 MB".to_owned(),
                ));
            }
            buf
        } else {
            raw.to_vec()
        };
        let text = std::str::from_utf8(&unpacked)
            .map_err(|_| VocabError::Invalid("vocabulary file is not UTF-8".to_owned()))?;
        let val: Value = serde_json::from_str(text)
            .map_err(|e| VocabError::Invalid(format!("invalid JSON: {e}")))?;

        let version = val.get("version").and_then(Value::as_i64).unwrap_or(0);
        let format = val.get("format").and_then(Value::as_str).unwrap_or("");
        if version != 1 || (format != "sumatrapdf-vocabulary" && format != "wmkeyboard-vocab") {
            return Err(VocabError::Invalid(
                "unsupported vocabulary format or version".to_owned(),
            ));
        }

        let (parsed_decks, parsed_words) = if format == "wmkeyboard-vocab" {
            parse_wm_vocab(&val, now)?
        } else {
            parse_enhanced_vocab(&val)?
        };

        let mut next_words = if mode == ImportMode::Merge {
            self.words.clone()
        } else {
            Vec::new()
        };
        let mut next_decks = if mode == ImportMode::Merge {
            self.decks.clone()
        } else {
            Vec::new()
        };

        if next_words.len() + parsed_words.len() > MAX_VOCAB_WORDS
            || next_decks.len() + parsed_decks.len() > MAX_VOCAB_DECKS
        {
            return Err(VocabError::Invalid(
                "import exceeds vocabulary size limits".to_owned(),
            ));
        }

        let imported_count = parsed_words.len();
        for mut word in parsed_words {
            if let Some(existing) = next_words
                .iter_mut()
                .find(|w| same_word(&w.word, &word.word) && w.dictionary_id == word.dictionary_id)
            {
                for d in &word.deck_ids {
                    existing.attach_deck(d);
                }
                if word.srs.last_review.unwrap_or(0) > existing.srs.last_review.unwrap_or(0) {
                    existing.srs = word.srs;
                    existing.legacy_box = word.legacy_box;
                    existing.legacy_ease = word.legacy_ease;
                }
                if existing.definition.is_empty() && !word.definition.is_empty() {
                    existing.definition = word.definition;
                }
            } else {
                if next_words.iter().any(|w| w.id == word.id) {
                    word.id = anchor::new_id();
                }
                next_words.push(word);
            }
        }

        for deck in parsed_decks {
            if let Some(existing) = next_decks.iter_mut().find(|d| d.id == deck.id) {
                if existing.installed_total == 0 {
                    existing.installed_total = deck.installed_total;
                    existing.missing_definitions = deck.missing_definitions;
                }
            } else {
                next_decks.push(deck);
            }
        }

        self.words = next_words;
        self.decks = next_decks;
        self.ensure_builtin_decks();
        self.removed.clear();
        self.bump_revision();
        Ok(imported_count)
    }

    /// Persists all decks and vocabulary words (with FSRS state) into SQLite via `excellent_store::study`.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Store`] if SQLite fails.
    pub fn save_to_store(&self, tx: &Transaction<'_>) -> Result<(), VocabError> {
        study::ensure_schema(tx)?;
        for deck in &self.decks {
            let rec = DeckRecord {
                id: deck.id.clone(),
                name: deck.name.clone(),
                description: deck.description.clone(),
                source: deck.source.clone(),
                license: deck.license.clone(),
                builtin: deck.builtin,
                installed_total: deck.installed_total,
                missing_definitions_json: serde_json::to_string(&deck.missing_definitions)
                    .unwrap_or_else(|_| "[]".to_owned()),
                created: 0,
            };
            study::upsert_deck(tx, &rec)?;
        }
        for word in &self.words {
            let extra_json = json!({
                "dictionary_id": word.dictionary_id,
                "context": word.context,
                "deck_ids": word.deck_ids,
                "legacy_box": word.legacy_box,
                "legacy_ease": word.legacy_ease,
            })
            .to_string();
            let card = CardRecord {
                id: word.id.clone(),
                deck_id: word.deck_id.clone(),
                kind: "vocab".to_owned(),
                front: word.word.clone(),
                back: word.definition.clone(),
                cloze_ordinal: None,
                occlusion_json: None,
                source_path: (!word.source_path.is_empty()).then(|| word.source_path.clone()),
                source_page: word.page.checked_sub(1),
                anchor_json: None,
                media_json: "[]".to_owned(),
                tags_json: "[]".to_owned(),
                extra_json,
                created: word.added_time,
                modified: word.srs.last_review.unwrap_or(word.added_time),
            };
            study::upsert_card(tx, &card)?;
            study::save_srs(tx, &word.id, &word.srs.to_record())?;
        }
        Ok(())
    }

    /// Loads vocabulary decks and words from SQLite (`excellent_store::study`).
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Store`] if SQLite fails.
    pub fn load_from_store(conn: &Connection) -> Result<Self, VocabError> {
        study::ensure_schema(conn)?;
        let mut lib = Self::new();
        for rec in study::list_decks(conn)? {
            let missing: Vec<String> =
                serde_json::from_str(&rec.missing_definitions_json).unwrap_or_default();
            if let Some(existing) = lib.decks.iter_mut().find(|d| d.id == rec.id) {
                existing.installed_total = rec.installed_total;
                existing.missing_definitions = missing;
            } else {
                lib.decks.push(VocabularyDeck {
                    id: rec.id,
                    name: rec.name,
                    description: rec.description,
                    source: rec.source,
                    license: rec.license,
                    installed_total: rec.installed_total,
                    missing_definitions: missing,
                    builtin: rec.builtin,
                });
            }
        }
        for card in study::list_cards(conn, None)? {
            if card.kind != "vocab" {
                continue;
            }
            let srs = study::load_srs(conn, &card.id)?
                .map(CardState::from_record)
                .unwrap_or_else(|| CardState::new_at(card.created));
            let extra: Value = serde_json::from_str(&card.extra_json).unwrap_or(Value::Null);
            let dictionary_id = extra
                .get("dictionary_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let context = extra
                .get("context")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let deck_ids: Vec<String> = extra
                .get("deck_ids")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| {
                    if card.deck_id.is_empty() {
                        Vec::new()
                    } else {
                        vec![card.deck_id.clone()]
                    }
                });
            let legacy_box = extra
                .get("legacy_box")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let legacy_ease = extra
                .get("legacy_ease")
                .and_then(Value::as_f64)
                .unwrap_or(2.5);
            lib.words.push(VocabularyWord {
                id: card.id,
                word: card.front,
                definition: card.back,
                dictionary_id,
                context,
                source_path: card.source_path.unwrap_or_default(),
                page: card.source_page.map_or(0, |p| p + 1),
                deck_id: card.deck_id,
                deck_ids,
                srs,
                added_time: card.created,
                legacy_box,
                legacy_ease,
            });
        }
        lib.bump_revision();
        Ok(lib)
    }

    /// Appends a vocabulary review's [`excellent_store::study::ReviewLogRecord`] to SQLite.
    ///
    /// # Errors
    ///
    /// Returns [`VocabError::Store`] if SQLite fails.
    pub fn record_review_in_store(
        tx: &Transaction<'_>,
        outcome: &ReviewOutcome,
    ) -> Result<i64, VocabError> {
        study::save_srs(tx, &outcome.log.card_id, &outcome.state.to_record())?;
        Ok(study::append_review_log(tx, &outcome.log)?)
    }

    fn ensure_builtin_decks(&mut self) {
        for info in BUILTIN_DECKS {
            if let Some(existing) = self.decks.iter_mut().find(|d| d.id == info.id) {
                existing.builtin = true;
            } else {
                self.decks.push(VocabularyDeck {
                    id: info.id.to_owned(),
                    name: info.name.to_owned(),
                    description: info.description.to_owned(),
                    source: format!(
                        "https://github.com/wasi-master/wmkeyboard-data/blob/master/vocab/en/{}.wmvocab.json.gz",
                        info.id
                    ),
                    license: "Word lists: MIT; Meanings: Wiktionary CC BY-SA 3.0; WordNet license."
                        .to_owned(),
                    installed_total: 0,
                    missing_definitions: Vec::new(),
                    builtin: true,
                });
            }
        }
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn next_rand(&mut self) -> u32 {
        let mut x = self.rng_state;
        if x == 0 {
            x = 0x006b_8421;
        }
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }
}

fn parse_enhanced_vocab(
    val: &Value,
) -> Result<(Vec<VocabularyDeck>, Vec<VocabularyWord>), VocabError> {
    let mut decks = Vec::new();
    if let Some(arr) = val.get("decks").and_then(Value::as_array) {
        if arr.len() > MAX_VOCAB_DECKS {
            return Err(VocabError::Invalid("too many decks".to_owned()));
        }
        for item in arr {
            let id = read_str(item, "id", 128)?;
            let name = read_str(item, "name", 256)?;
            if id.is_empty() || name.is_empty() || decks.iter().any(|d: &VocabularyDeck| d.id == id)
            {
                return Err(VocabError::Invalid("invalid or duplicate deck".to_owned()));
            }
            let missing_definitions = item
                .get("missingDefinitions")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .filter_map(clean_word)
                        .collect()
                })
                .unwrap_or_default();
            decks.push(VocabularyDeck {
                id,
                name,
                description: read_str(item, "description", 8_192)?,
                source: read_str(item, "source", 2_048)?,
                license: read_str(item, "license", 32_000)?,
                installed_total: read_u32(item, "installedTotal", MAX_VOCAB_WORDS as u32),
                missing_definitions,
                builtin: false,
            });
        }
    }

    let mut words = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    if let Some(arr) = val.get("words").and_then(Value::as_array) {
        if arr.len() > MAX_VOCAB_WORDS {
            return Err(VocabError::Invalid("too many words".to_owned()));
        }
        for item in arr {
            let id = read_str(item, "id", 128)?;
            let raw_word = read_str(item, "word", 256)?;
            let word = clean_word(&raw_word).ok_or_else(|| {
                VocabError::Invalid("vocabulary contains invalid word".to_owned())
            })?;
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                return Err(VocabError::Invalid(
                    "vocabulary contains missing or duplicate word IDs".to_owned(),
                ));
            }
            let legacy_box = read_u32(item, "box", 4);
            let legacy_ease = item
                .get("ease")
                .and_then(Value::as_f64)
                .filter(|e| e.is_finite())
                .map_or(2.5, |e| e.clamp(1.3, 10.0));
            let legacy = EnhancedScheduling {
                box_index: legacy_box,
                ease: legacy_ease,
                repetitions: read_u32(item, "repetitions", 1_000_000),
                reviews: read_u32(item, "reviews", 1_000_000),
                lapses: read_u32(item, "lapses", 1_000_000),
                interval_days: read_u32(item, "intervalDays", 3_650),
                due_time: read_i64(item, "dueTime", 4_102_444_800),
                last_reviewed: read_i64(item, "lastReviewed", 4_102_444_800),
                learned: item
                    .get("learned")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            let srs = migrate_enhanced_state(legacy);
            let mut entry = VocabularyWord {
                id,
                word,
                definition: read_str(item, "definition", 16_000)?,
                dictionary_id: read_str(item, "dictionaryId", 256)?,
                context: read_str(item, "context", 8_192)?,
                source_path: read_str(item, "sourcePath", 2_048)?,
                page: usize::try_from(read_u32(item, "page", 10_000_000)).unwrap_or(0),
                deck_id: read_str(item, "deckId", 128)?,
                deck_ids: Vec::new(),
                srs,
                added_time: read_i64(item, "addedTime", 4_102_444_800),
                legacy_box,
                legacy_ease,
            };
            let primary = entry.deck_id.clone();
            entry.attach_deck(&primary);
            if let Some(dids) = item.get("deckIds").and_then(Value::as_array) {
                for d in dids.iter().filter_map(Value::as_str) {
                    if d.len() > 128 {
                        return Err(VocabError::Invalid("deckId too long".to_owned()));
                    }
                    entry.attach_deck(d);
                }
            }
            words.push(entry);
        }
    }
    Ok((decks, words))
}

fn parse_wm_vocab(
    val: &Value,
    now: i64,
) -> Result<(Vec<VocabularyDeck>, Vec<VocabularyWord>), VocabError> {
    let pack = val.get("pack");
    let pack_id = pack
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(anchor::new_id, str::to_owned);
    let pack_name = pack
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported vocabulary")
        .to_owned();
    let license = pack
        .and_then(|p| p.get("attribution"))
        .map(Value::to_string)
        .unwrap_or_default();
    let deck_id = format!("import-{pack_id}");
    let deck = VocabularyDeck {
        id: deck_id.clone(),
        name: pack_name,
        description: String::new(),
        source: "Imported WM Keyboard vocabulary pack".to_owned(),
        license,
        installed_total: 0,
        missing_definitions: Vec::new(),
        builtin: false,
    };

    let mut words = Vec::new();
    if let Some(arr) = val.get("words").and_then(Value::as_array) {
        if arr.len() > MAX_VOCAB_WORDS {
            return Err(VocabError::Invalid(
                "WM vocabulary exceeds limit".to_owned(),
            ));
        }
        for item in arr {
            let raw_word = read_str(item, "word", 256)?;
            let word = clean_word(&raw_word)
                .ok_or_else(|| VocabError::Invalid("invalid WM word".to_owned()))?;
            let mut defs = Vec::new();
            if let Some(senses) = item.get("senses").and_then(Value::as_array) {
                for s in senses {
                    if let Some(d) = s.get("definition").and_then(Value::as_str)
                        && !d.is_empty()
                    {
                        defs.push(d);
                    }
                }
            }
            let definition = defs.join("\n");
            let mut entry = VocabularyWord {
                id: anchor::new_id(),
                word,
                definition,
                dictionary_id: "wmkeyboard-import".to_owned(),
                context: String::new(),
                source_path: String::new(),
                page: 0,
                deck_id: String::new(),
                deck_ids: Vec::new(),
                srs: CardState::new_at(now),
                added_time: now,
                legacy_box: 0,
                legacy_ease: 2.5,
            };
            entry.attach_deck(&deck_id);
            words.push(entry);
        }
    }
    Ok((vec![deck], words))
}

fn read_str(obj: &Value, key: &str, max_len: usize) -> Result<String, VocabError> {
    let Some(val) = obj.get(key) else {
        return Ok(String::new());
    };
    let Some(s) = val.as_str() else {
        return Err(VocabError::Invalid(format!("field {key} must be a string")));
    };
    if s.len() > max_len || s.contains('\0') {
        return Err(VocabError::Invalid(format!("field {key} exceeds limit")));
    }
    Ok(s.to_owned())
}

fn read_u32(obj: &Value, key: &str, max: u32) -> u32 {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map_or(0, |v| (v as u64).min(u64::from(max)) as u32)
}

fn read_i64(obj: &Value, key: &str, max: i64) -> i64 {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map_or(0, |v| (v as i64).min(max))
}

fn clean_word(input: &str) -> Option<String> {
    if input.len() > 256 {
        return None;
    }
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains(['\0', '\n', '\r']) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn same_word(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    a.to_lowercase() == b.to_lowercase()
}

fn detach_deck(word: &mut VocabularyWord, deck_id: &str) {
    word.deck_ids.retain(|d| d != deck_id);
    if word.deck_id == deck_id {
        word.deck_id = word.deck_ids.first().cloned().unwrap_or_default();
    }
}

/// FNV-1a hash matching Enhanced's `DailyWordHash`.
fn daily_word_hash(word: &str) -> u64 {
    let mut hash: u64 = 1_469_598_103_934_665_603;
    for &b in word.as_bytes() {
        let ch = b.to_ascii_lowercase();
        hash = (hash ^ u64::from(ch)).wrapping_mul(1_099_511_628_211);
    }
    hash
}

#[cfg(test)]
mod tests {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write as _;

    use super::*;

    /// Done when (S.7): importing SumatraPDF Enhanced's `SumatraPDF-vocabulary.json` keeps every word and its exact
    /// due dates while scheduling future reviews on FSRS-5.
    #[test]
    fn importing_enhanced_vocabulary_keeps_every_word_and_exact_due_dates() {
        let enhanced_json = r#"{
            "format": "sumatrapdf-vocabulary",
            "version": 1,
            "decks": [
                {"id": "custom-1", "name": "Seminar Reading", "description": "Paper terms", "source": "", "license": "", "installedTotal": 0, "missingDefinitions": []}
            ],
            "words": [
                {
                    "id": "w-1",
                    "word": "abhor",
                    "definition": "To regard with disgust and hatred.",
                    "dictionaryId": "wmkeyboard-vocab-en",
                    "context": "She came to abhor shortcuts.",
                    "sourcePath": "C:\\reading\\essay.pdf",
                    "deckId": "custom-1",
                    "deckIds": ["custom-1", "ws1"],
                    "page": 12,
                    "box": 3,
                    "repetitions": 4,
                    "reviews": 7,
                    "lapses": 1,
                    "intervalDays": 14,
                    "ease": 2.35,
                    "dueTime": 1701200000,
                    "lastReviewed": 1700000000,
                    "addedTime": 1699000000,
                    "learned": false
                },
                {
                    "id": "w-2",
                    "word": "ephemeral",
                    "definition": "Lasting for a very short time.",
                    "dictionaryId": "wordnet-en",
                    "context": "Ephemeral blossoms.",
                    "sourcePath": "C:\\reading\\poems.pdf",
                    "deckId": "ws1",
                    "deckIds": ["ws1"],
                    "page": 4,
                    "box": 4,
                    "repetitions": 6,
                    "reviews": 6,
                    "lapses": 0,
                    "intervalDays": 60,
                    "ease": 2.7,
                    "dueTime": 1705184000,
                    "lastReviewed": 1700000000,
                    "addedTime": 1698000000,
                    "learned": true
                }
            ]
        }"#;

        let mut lib = VocabularyLibrary::new();
        assert_eq!(lib.decks().len(), 11);

        let count = lib
            .import_enhanced_json(enhanced_json.as_bytes(), ImportMode::Replace, 1_700_000_000)
            .expect("import enhanced JSON");
        assert_eq!(count, 2);
        assert_eq!(lib.words().len(), 2);

        let w1 = lib.find("w-1").expect("abhor kept");
        assert_eq!(w1.word, "abhor");
        assert_eq!(w1.srs.due, 1_701_200_000, "exact due date preserved");
        assert_eq!(w1.srs.last_review, Some(1_700_000_000));
        assert_eq!(w1.srs.reps, 7);
        assert_eq!(w1.srs.lapses, 1);
        assert_eq!(w1.page, 12);
        assert_eq!(w1.deck_ids, ["custom-1", "ws1"]);

        let w2 = lib.find("w-2").expect("ephemeral kept");
        assert_eq!(w2.srs.due, 1_705_184_000, "exact due date preserved");
        assert!(w2.srs.learned);

        // Next review runs on FSRS-5.
        let outcome = lib
            .review("w-1", Rating::Good, 1_701_200_000, 1_800)
            .expect("fsrs review");
        assert!(outcome.state.scheduled_days > 14);
        assert_eq!(outcome.state.reps, 8);

        // Gzipped WM pack import also succeeds.
        let wm = r#"{"format":"wmkeyboard-vocab","version":1,"pack":{"id":"custom","name":"Custom","attribution":[{"license":"MIT"}]},"words":[{"word":"abate","senses":[{"definition":"Become less intense"}]}]}"#;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(wm.as_bytes()).expect("gz");
        let compressed = gz.finish().expect("finish gz");
        let wm_added = lib
            .import_enhanced_json(&compressed, ImportMode::Merge, 1_701_200_100)
            .expect("import gzipped wm");
        assert_eq!(wm_added, 1);
        assert_eq!(lib.words().len(), 3);

        // Corrupt import fails without altering library state.
        assert!(
            lib.import_enhanced_json(b"{broken", ImportMode::Replace, 1_701_200_200)
                .is_err()
        );
        assert_eq!(lib.words().len(), 3);
    }

    #[test]
    fn all_six_practice_activities_undo_remove_and_sqlite_round_trip() {
        let mut lib = VocabularyLibrary::new();
        lib.set_rng_seed(42);
        let deck_id = lib.create_deck("Physics", "Core terms").expect("deck");
        let id1 = lib
            .add(
                NewVocabularyWord {
                    word: "  entropy  ",
                    definition: "Measure of disorder.",
                    dictionary_id: "test",
                    context: "Entropy increases.",
                    source_path: "C:/phys.pdf",
                    page: 5,
                    deck_id: &deck_id,
                },
                1_700_000_000,
            )
            .expect("add 1");
        let id2 = lib
            .add(
                NewVocabularyWord {
                    word: "enthalpy",
                    definition: "Total heat content.",
                    dictionary_id: "test",
                    deck_id: &deck_id,
                    ..NewVocabularyWord::default()
                },
                1_700_000_000,
            )
            .expect("add 2");
        let _id3 = lib
            .add(
                NewVocabularyWord {
                    word: "adiabatic",
                    definition: "Without heat transfer.",
                    dictionary_id: "test",
                    deck_id: &deck_id,
                    ..NewVocabularyWord::default()
                },
                1_700_000_000,
            )
            .expect("add 3");

        // All 6 practice activities.
        for activity in [
            VocabActivity::Flashcards,
            VocabActivity::MeaningChoice,
            VocabActivity::WordChoice,
            VocabActivity::Spelling,
            VocabActivity::WordScramble,
            VocabActivity::MatchPairs,
        ] {
            let q = lib.make_question(&id1, activity).expect("make question");
            assert!(VocabularyLibrary::check_answer(&q, &q.answer));
            if let Some(idx) = q.correct_index {
                assert!(VocabularyLibrary::check_answer(&q, &q.choices[idx]));
            }
        }
        let board = lib.make_match_pairs(&deck_id, 4).expect("match pairs");
        assert_eq!(board.pairs.len(), 3);
        assert_eq!(board.shuffled_definition_order.len(), 3);

        // Undo remove restores exact position.
        assert!(lib.remove(&id2));
        assert!(lib.find(&id2).is_none());
        assert!(lib.can_undo_remove());
        assert_eq!(lib.undo_remove().expect("undo"), id2);
        assert_eq!(lib.words()[1].id, id2);

        // Word of the day is stable within the same UTC day.
        let wotd = lib.word_of_day(1_700_000_100).expect("wotd").id.clone();
        assert_eq!(
            lib.word_of_day(1_700_000_500).expect("wotd cached").id,
            wotd
        );

        // SQLite persistence round-trip.
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute_batch(
            "CREATE TABLE file (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE) STRICT;",
        )
        .expect("file table");
        let tx = conn.transaction().expect("tx");
        lib.save_to_store(&tx).expect("save to sqlite");
        tx.commit().expect("commit");

        let loaded = VocabularyLibrary::load_from_store(&conn).expect("load from sqlite");
        assert_eq!(loaded.words().len(), 3);
        assert_eq!(loaded.find(&id1).expect("w1").word, "entropy");
        assert_eq!(loaded.find(&id1).expect("w1").page, 5);
    }
}
