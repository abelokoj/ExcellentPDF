//! In-memory offline dictionary index and format parsers (plan S.6), ported from SumatraPDF Enhanced's
//! `OfflineDictionary.cpp`: TSV, StarDict (`.ifo`/`.idx`\[`.gz`\]/`.dict`\[`.dz`\]/`.syn`), WM JSON
//! (`.wmvocab.json[.gz]`), WordNet (`data.*` + `*.exc`) and online JSON parsers (Kaikki, Wiktionary REST, Free
//! Dictionary API).

use std::fmt;
use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use serde_json::Value;

use super::kaikki::{KAIKKI_PACKS, lookup_kaikki_records, parse_kaikki_record};
use super::normalize::{
    MAX_WORD_BYTES, dictionary_plain_text, english_stem_candidates, full_pos, percent_encode,
    plain_definition, word_key,
};

/// Maximum single dictionary file size (256 MiB, matching Enhanced).
pub const MAX_DICTIONARY_BYTES: usize = 256 * 1024 * 1024;

/// Maximum entries loaded into a [`DictionaryIndex`].
const MAX_DICTIONARY_ENTRIES: usize = 2_000_000;

/// Maximum definition length in bytes (256 KiB).
const MAX_DEFINITION_BYTES: usize = 256 * 1024;

/// Maximum results returned by a single lookup (matching Enhanced's `kDictionaryMaxResults`).
pub const MAX_LOOKUP_RESULTS: usize = 64;

/// Reserved dictionary id for Princeton WordNet 3.0.
pub const WORDNET_ID: &str = "wordnet-en";

/// Reserved dictionary id for WMKeyboard vocabulary packs.
pub const WM_VOCAB_ID: &str = "wmkeyboard-vocab-en";

/// Errors produced when loading or querying offline dictionaries.
#[derive(Debug)]
#[non_exhaustive]
pub enum DictError {
    /// Underlying filesystem or gzip decompression failure.
    Io(std::io::Error),
    /// Malformed dictionary file or invalid UTF-8 / unsupported StarDict option.
    Malformed(String),
}

impl fmt::Display for DictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "dictionary i/o error: {e}"),
            Self::Malformed(msg) => write!(f, "malformed dictionary: {msg}"),
        }
    }
}

impl std::error::Error for DictError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Malformed(_) => None,
        }
    }
}

impl From<std::io::Error> for DictError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Source provider selector (matching Enhanced's `DictionarySource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DictionarySource {
    /// Local offline packs only (default, zero network traffic).
    Offline,
    /// Wiktionary via Kaikki JSONL schema.
    Wiktionary,
    /// Wiktionary REST API schema.
    WiktionaryRest,
    /// Free Dictionary API v2 schema.
    FreeDictionary,
}

/// Catalog entry describing an available or installed offline dictionary pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineDictPack {
    /// Pack identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Language label.
    pub language: String,
    /// Licence summary.
    pub license: String,
    /// Source attribution URL.
    pub source_url: String,
    /// Whether installed on disk.
    pub installed: bool,
    /// Whether bundled with the app.
    pub bundled: bool,
    /// Download size in bytes.
    pub download_bytes: u64,
    /// Expanded disk size in bytes.
    pub expanded_bytes: u64,
    /// Whether compressed.
    pub compressed: bool,
}

/// One resolved dictionary sense/definition (matching Enhanced's `OfflineMeaning`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfflineMeaning {
    /// Original headword casing.
    pub headword: String,
    /// Cleaned plain-text definition.
    pub definition: String,
    /// Display name of the dictionary source.
    pub dictionary: String,
    /// Stable identifier of the dictionary source.
    pub dictionary_id: String,
    /// Normalized full part of speech (`"noun"`, `"verb"`, …).
    pub part_of_speech: String,
    /// Example usage sentence(s), newline-separated.
    pub example: String,
    /// Related synonyms, newline-separated.
    pub synonyms: String,
    /// Related antonyms, newline-separated.
    pub antonyms: String,
    /// US/default IPA transcription.
    pub phonetic: String,
    /// UK IPA transcription.
    pub phonetic_uk: String,
    /// Optional US pronunciation audio URL (HTTPS only).
    pub audio_url: String,
    /// Optional UK pronunciation audio URL (HTTPS only).
    pub audio_url_uk: String,
    /// Source entry URL.
    pub source_url: String,
    /// Licence attribution string.
    pub license: String,
}

/// Maps an inflected word form to its base lemma (matching Enhanced's `DictInflection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictInflection {
    /// Normalized inflected word key.
    pub word: String,
    /// Normalized base lemma key.
    pub lemma: String,
    /// Insertion order used to preserve priority ties.
    pub order: usize,
}

/// Internal indexed dictionary entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DictEntry {
    key: String,
    meaning: OfflineMeaning,
    order: usize,
}

/// Fast in-memory offline dictionary index supporting binary search over normalized keys and inflections (plan S.6).
#[derive(Debug, Clone, Default)]
pub struct DictionaryIndex {
    entries: Vec<DictEntry>,
    inflections: Vec<DictInflection>,
    sorted: bool,
}

impl DictionaryIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of indexed meanings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no meanings are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of indexed inflection mappings.
    #[must_use]
    pub fn inflection_count(&self) -> usize {
        self.inflections.len()
    }

    /// Adds a single headword/definition pair; returns `false` if invalid UTF-8, empty or over limits.
    pub fn add_entry(
        &mut self,
        word: &str,
        definition: &str,
        dictionary_id: &str,
        dictionary_title: &str,
        detail: Option<&OfflineMeaning>,
    ) -> bool {
        let Some(key) = word_key(word) else {
            return false;
        };
        if definition.is_empty()
            || definition.len() > MAX_DEFINITION_BYTES
            || self.entries.len() >= MAX_DICTIONARY_ENTRIES
        {
            return false;
        }
        let plain = dictionary_plain_text(definition);
        if plain.is_empty() {
            return false;
        }
        let mut meaning = detail.cloned().unwrap_or_default();
        meaning.headword = word.to_owned();
        meaning.definition = plain;
        meaning.dictionary_id = dictionary_id.to_owned();
        meaning.dictionary = dictionary_title.to_owned();
        let order = self.entries.len();
        self.entries.push(DictEntry {
            key,
            meaning,
            order,
        });
        self.sorted = false;
        true
    }

    /// Adds one inflection mapping (`inflected -> lemma`).
    pub fn add_inflection(&mut self, inflected: &str, lemma: &str) {
        if let (Some(w), Some(l)) = (word_key(inflected), word_key(lemma))
            && w != l
            && self.inflections.len() < 100_000
        {
            let order = self.inflections.len();
            self.inflections.push(DictInflection {
                word: w,
                lemma: l,
                order,
            });
            self.sorted = false;
        }
    }

    /// Sorts entries and inflections once for O(log N) binary search lookups.
    pub fn finalize(&mut self) {
        if self.sorted {
            return;
        }
        self.entries
            .sort_unstable_by(|a, b| a.key.cmp(&b.key).then_with(|| a.order.cmp(&b.order)));
        self.inflections
            .sort_unstable_by(|a, b| a.word.cmp(&b.word).then_with(|| a.order.cmp(&b.order)));
        self.sorted = true;
    }

    /// Parses a UTF-8 TSV dictionary (optional UTF-8 BOM, `#` comment lines, `word<TAB>definition`).
    ///
    /// # Errors
    ///
    /// Returns [`DictError::Malformed`] on invalid UTF-8, missing tab, or zero valid entries.
    pub fn load_tsv(&mut self, raw: &[u8], id: &str, title: &str) -> Result<usize, DictError> {
        let mut text = std::str::from_utf8(raw)
            .map_err(|_| DictError::Malformed("TSV dictionary is not valid UTF-8".to_owned()))?;
        if let Some(stripped) = text.strip_prefix('\u{feff}') {
            text = stripped;
        }
        let start = self.entries.len();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((word, def)) = line.split_once('\t') else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "TSV line missing tab separator".to_owned(),
                ));
            };
            if !self.add_entry(word, def, id, title, None) {
                self.entries.truncate(start);
                return Err(DictError::Malformed("invalid TSV entry".to_owned()));
            }
        }
        let added = self.entries.len() - start;
        if added == 0 {
            return Err(DictError::Malformed(
                "TSV dictionary contained no entries".to_owned(),
            ));
        }
        Ok(added)
    }

    /// Parses a StarDict dictionary from its `.ifo`, `.idx` (or `.idx.gz`), `.dict` (or `.dict.dz`) and optional
    /// `.syn` bytes, honouring `sametypesequence` and rejecting 64-bit offsets (`idxoffsetbits=64`), matching
    /// Enhanced's `DictionaryIndex::StarDict`.
    ///
    /// # Errors
    ///
    /// Returns [`DictError::Malformed`] or [`DictError::Io`] on invalid StarDict inputs.
    pub fn load_stardict(
        &mut self,
        ifo_raw: &[u8],
        idx_raw: &[u8],
        dict_raw: &[u8],
        syn_raw: Option<&[u8]>,
        id: &str,
        title: &str,
    ) -> Result<usize, DictError> {
        let ifo = std::str::from_utf8(ifo_raw)
            .map_err(|_| DictError::Malformed("StarDict .ifo is not UTF-8".to_owned()))?;
        if !ifo.starts_with("StarDict's dict ifo file\n")
            && !ifo.starts_with("StarDict's dict ifo file\r\n")
        {
            return Err(DictError::Malformed(
                "missing StarDict .ifo header".to_owned(),
            ));
        }
        if ifo.contains("idxoffsetbits=64") {
            return Err(DictError::Malformed(
                "unsupported StarDict idxoffsetbits=64".to_owned(),
            ));
        }

        let types: Vec<u8> = ifo
            .lines()
            .find_map(|line| line.strip_prefix("sametypesequence="))
            .map(|s| s.trim().as_bytes().to_vec())
            .unwrap_or_default();

        let idx = maybe_gunzip(idx_raw)?;
        let dict = maybe_gunzip(dict_raw)?;

        let start = self.entries.len();
        let mut cursor = idx.as_slice();
        while !cursor.is_empty() {
            let Some(zero) = cursor.iter().position(|&b| b == 0) else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "unterminated StarDict word".to_owned(),
                ));
            };
            if zero > MAX_WORD_BYTES || cursor.len() - zero < 9 {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "truncated StarDict index entry".to_owned(),
                ));
            }
            let word = std::str::from_utf8(&cursor[..zero]).map_err(|_| {
                self.entries.truncate(start);
                DictError::Malformed("StarDict headword is not UTF-8".to_owned())
            })?;
            let nums = &cursor[zero + 1..zero + 9];
            let offset = u32::from_be_bytes([nums[0], nums[1], nums[2], nums[3]]) as usize;
            let size = u32::from_be_bytes([nums[4], nums[5], nums[6], nums[7]]) as usize;
            cursor = &cursor[zero + 9..];

            if size == 0
                || size > MAX_DEFINITION_BYTES
                || offset.checked_add(size).is_none_or(|end| end > dict.len())
            {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "StarDict offset/size out of bounds".to_owned(),
                ));
            }
            let mut record = &dict[offset..offset + size];
            let mut def = String::new();
            let mut field = 0_usize;
            while !record.is_empty() {
                let kind = if !types.is_empty() {
                    if field < types.len() {
                        types[field]
                    } else {
                        self.entries.truncate(start);
                        return Err(DictError::Malformed("extra StarDict fields".to_owned()));
                    }
                } else {
                    let t = record[0];
                    record = &record[1..];
                    t
                };
                if kind == 0 {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "invalid StarDict field type".to_owned(),
                    ));
                }
                let is_last = !types.is_empty() && field + 1 == types.len();
                if kind.is_ascii_uppercase() {
                    if is_last {
                        break;
                    }
                    if record.len() < 4 {
                        self.entries.truncate(start);
                        return Err(DictError::Malformed(
                            "truncated StarDict binary field".to_owned(),
                        ));
                    }
                    let n =
                        u32::from_be_bytes([record[0], record[1], record[2], record[3]]) as usize;
                    if n.checked_add(4).is_none_or(|end| end > record.len()) {
                        self.entries.truncate(start);
                        return Err(DictError::Malformed(
                            "StarDict binary field out of bounds".to_owned(),
                        ));
                    }
                    record = &record[n + 4..];
                } else {
                    let (slice, rest) = if is_last {
                        (record, &[][..])
                    } else if let Some(z) = record.iter().position(|&b| b == 0) {
                        (&record[..z], &record[z + 1..])
                    } else {
                        self.entries.truncate(start);
                        return Err(DictError::Malformed(
                            "unterminated StarDict text field".to_owned(),
                        ));
                    };
                    if matches!(kind, b'm' | b'l' | b'h' | b'n' | b'g' | b'x') {
                        let piece = std::str::from_utf8(slice).map_err(|_| {
                            self.entries.truncate(start);
                            DictError::Malformed("StarDict text field is not UTF-8".to_owned())
                        })?;
                        if !def.is_empty() {
                            def.push('\n');
                        }
                        def.push_str(piece);
                    }
                    record = rest;
                }
                field += 1;
            }
            if !self.add_entry(word, &def, id, title, None) {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "invalid StarDict definition".to_owned(),
                ));
            }
        }

        let original_count = self.entries.len() - start;
        if original_count == 0 {
            return Err(DictError::Malformed(
                "StarDict contained no entries".to_owned(),
            ));
        }

        if let Some(syn_bytes) = syn_raw {
            let mut rem = syn_bytes;
            while !rem.is_empty() {
                let Some(zero) = rem.iter().position(|&b| b == 0) else {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "unterminated StarDict .syn alias".to_owned(),
                    ));
                };
                if zero > MAX_WORD_BYTES || rem.len() - zero < 5 {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "truncated StarDict .syn entry".to_owned(),
                    ));
                }
                let alias = std::str::from_utf8(&rem[..zero]).map_err(|_| {
                    self.entries.truncate(start);
                    DictError::Malformed("StarDict .syn alias is not UTF-8".to_owned())
                })?;
                let ref_bytes = &rem[zero + 1..zero + 5];
                let reference =
                    u32::from_be_bytes([ref_bytes[0], ref_bytes[1], ref_bytes[2], ref_bytes[3]])
                        as usize;
                if reference >= original_count {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "StarDict .syn index out of range".to_owned(),
                    ));
                }
                let original = self.entries[start + reference].meaning.clone();
                if !self.add_entry(alias, &original.definition, id, title, Some(&original)) {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "invalid StarDict .syn alias".to_owned(),
                    ));
                }
                rem = &rem[zero + 5..];
            }
        }

        Ok(self.entries.len() - start)
    }

    /// Parses a WMKeyboard vocabulary JSON pack (plain or gzipped `.wmvocab.json[.gz]`), matching Enhanced's
    /// `LoadWmJson`.
    ///
    /// # Errors
    ///
    /// Returns [`DictError::Malformed`] if the format/version does not match `wmkeyboard-vocab` v1 or contains
    /// invalid entries.
    pub fn load_wm_json(&mut self, raw: &[u8], fallback_title: &str) -> Result<usize, DictError> {
        let unpacked = maybe_gunzip(raw)?;
        let text = std::str::from_utf8(&unpacked)
            .map_err(|_| DictError::Malformed("WM JSON is not UTF-8".to_owned()))?;
        let val: Value = serde_json::from_str(text)
            .map_err(|e| DictError::Malformed(format!("invalid WM JSON: {e}")))?;

        let format = val.get("format").and_then(Value::as_str).unwrap_or("");
        let version = val.get("version").and_then(Value::as_i64).unwrap_or(0);
        if format != "wmkeyboard-vocab" || version != 1 {
            return Err(DictError::Malformed(
                "not a supported wmkeyboard-vocab v1 pack".to_owned(),
            ));
        }
        let title = val
            .get("pack")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_title);

        let Some(words) = val.get("words").and_then(Value::as_array) else {
            return Err(DictError::Malformed(
                "WM JSON missing words array".to_owned(),
            ));
        };
        if words.len() > 20_000 {
            return Err(DictError::Malformed(
                "WM JSON exceeds 20,000 word limit".to_owned(),
            ));
        }

        let start = self.entries.len();
        for item in words {
            let Some(headword) = item
                .get("word")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "WM JSON word missing headword".to_owned(),
                ));
            };
            if let Some(forms) = item.get("forms").and_then(Value::as_array) {
                for form in forms.iter().filter_map(Value::as_str) {
                    self.add_inflection(form, headword);
                }
            }
            let phonetic = item
                .get("ipa")
                .and_then(|i| i.get("us"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let phonetic_uk = item
                .get("ipa")
                .and_then(|i| i.get("uk"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let audio_us = item
                .get("audio")
                .and_then(|a| a.get("us"))
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://"))
                .unwrap_or("");
            let audio_uk = item
                .get("audio")
                .and_then(|a| a.get("uk"))
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://"))
                .unwrap_or("");
            let word_syns = collect_string_list(item.get("synonyms"));
            let word_ants = collect_string_list(item.get("antonyms"));

            let Some(senses) = item.get("senses").and_then(Value::as_array) else {
                continue;
            };
            for sense in senses {
                let Some(def) = sense
                    .get("definition")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let mut syns = word_syns.clone();
                for s in collect_string_list(sense.get("synonyms")) {
                    push_unique_ci(&mut syns, &s);
                }
                let mut ants = word_ants.clone();
                for a in collect_string_list(sense.get("antonyms")) {
                    push_unique_ci(&mut ants, &a);
                }
                let detail = OfflineMeaning {
                    part_of_speech: sense
                        .get("pos")
                        .and_then(Value::as_str)
                        .map(full_pos)
                        .unwrap_or_default(),
                    example: sense
                        .get("example")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    synonyms: syns.join("\n"),
                    antonyms: ants.join("\n"),
                    phonetic: phonetic.to_owned(),
                    phonetic_uk: phonetic_uk.to_owned(),
                    audio_url: audio_us.to_owned(),
                    audio_url_uk: audio_uk.to_owned(),
                    source_url: format!(
                        "https://en.wiktionary.org/wiki/{}",
                        percent_encode(headword)
                    ),
                    license: "WMKeyboard word lists: MIT; Wiktionary meanings: CC BY-SA 3.0; WordNet meanings: WordNet license."
                        .to_owned(),
                    ..OfflineMeaning::default()
                };
                if !self.add_entry(headword, def, WM_VOCAB_ID, title, Some(&detail)) {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed("invalid WM sense".to_owned()));
                }
            }
        }
        let added = self.entries.len() - start;
        if added == 0 {
            return Err(DictError::Malformed(
                "WM JSON contained no definitions".to_owned(),
            ));
        }
        Ok(added)
    }

    /// Parses a Princeton WordNet 3.0 `data.{noun,verb,adj,adv}` file (matching Enhanced's `DictionaryIndex::WordNet`).
    ///
    /// # Errors
    ///
    /// Returns [`DictError::Malformed`] on malformed synset lines.
    pub fn load_wordnet_data(&mut self, text: &str) -> Result<usize, DictError> {
        let start = self.entries.len();
        for line in text.lines() {
            if line.len() < 10 || !line.as_bytes()[0].is_ascii_digit() {
                continue;
            }
            let Some((meta, gloss_raw)) = line.split_once('|') else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "WordNet line missing '|' gloss".to_owned(),
                ));
            };
            let mut tokens = meta.split_whitespace();
            let _offset = tokens.next();
            let _lex_filenum = tokens.next();
            let Some(pos_tok) = tokens.next() else {
                self.entries.truncate(start);
                return Err(DictError::Malformed("WordNet line missing POS".to_owned()));
            };
            let Some(count_hex) = tokens.next() else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "WordNet line missing word count".to_owned(),
                ));
            };
            let Ok(word_count) = usize::from_str_radix(count_hex, 16) else {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "invalid WordNet hex word count".to_owned(),
                ));
            };
            if word_count == 0 || word_count > 255 {
                self.entries.truncate(start);
                return Err(DictError::Malformed(
                    "WordNet word count out of range".to_owned(),
                ));
            }
            let pos = full_pos(pos_tok);
            let gloss = gloss_raw.trim();
            let (definition, examples) = split_wordnet_gloss(gloss);

            let mut words = Vec::with_capacity(word_count);
            for _ in 0..word_count {
                let Some(mut w) = tokens.next() else {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed(
                        "truncated WordNet word list".to_owned(),
                    ));
                };
                let _lex_id = tokens.next();
                if let Some(s) = w
                    .strip_suffix("(ip)")
                    .or_else(|| w.strip_suffix("(a)"))
                    .or_else(|| w.strip_suffix("(p)"))
                {
                    w = s;
                }
                words.push(w.replace('_', " "));
            }
            for word in &words {
                let mut synonyms = Vec::new();
                for other in &words {
                    if !other.eq_ignore_ascii_case(word) {
                        push_unique_ci(&mut synonyms, other);
                    }
                }
                let detail = OfflineMeaning {
                    part_of_speech: pos.clone(),
                    example: examples.clone(),
                    synonyms: synonyms.join("\n"),
                    source_url: "https://wordnet.princeton.edu/".to_owned(),
                    license: "Princeton WordNet 3.0 license".to_owned(),
                    ..OfflineMeaning::default()
                };
                if !self.add_entry(
                    word,
                    definition,
                    WORDNET_ID,
                    "Princeton WordNet 3.0 (English)",
                    Some(&detail),
                ) {
                    self.entries.truncate(start);
                    return Err(DictError::Malformed("invalid WordNet entry".to_owned()));
                }
            }
        }
        let added = self.entries.len() - start;
        if added == 0 {
            return Err(DictError::Malformed(
                "WordNet file contained no synsets".to_owned(),
            ));
        }
        Ok(added)
    }

    /// Parses a WordNet exception file (`noun.exc`, `verb.exc`, `adj.exc`, `adv.exc`) into inflection rules.
    pub fn load_wordnet_exceptions(&mut self, text: &str) {
        for line in text.lines() {
            let mut tokens = line.split_whitespace();
            let Some(inflected) = tokens.next() else {
                continue;
            };
            for lemma in tokens.take(32) {
                self.add_inflection(inflected, lemma);
            }
        }
    }

    /// Looks up `word` in the finalized index using priority order:
    /// 1. WM vocabulary packs (exact, then via inflection table)
    /// 2. All loaded dictionaries (exact)
    /// 3. Inflection table + English suffix stemming fallback against WordNet / loaded entries.
    #[must_use]
    pub fn lookup(&mut self, word: &str) -> Vec<OfflineMeaning> {
        self.finalize();
        let Some(key) = word_key(word) else {
            return Vec::new();
        };
        let mut out = Vec::new();

        // Priority 1: WM vocabulary packs (exact, then inflection lemmas).
        self.match_key(&key, &mut out, MatchScope::WmOnly);
        if out.is_empty() {
            let lemmas = self.inflection_lemmas(&key);
            for lemma in &lemmas {
                self.match_key(lemma, &mut out, MatchScope::WmOnly);
            }
        }
        if !out.is_empty() {
            return out;
        }

        // Priority 2: All dictionaries exact match.
        self.match_key(&key, &mut out, MatchScope::All);
        if !out.is_empty() {
            return out;
        }

        // Priority 3: Inflections + English suffix stemming fallback.
        let mut candidates = self.inflection_lemmas(&key);
        for stem in english_stem_candidates(&key) {
            if !candidates.contains(&stem) {
                candidates.push(stem);
            }
        }
        for candidate in &candidates {
            self.match_key(candidate, &mut out, MatchScope::All);
        }
        out
    }

    /// Performs an offline lookup combining the in-memory index and any installed Kaikki packs under `kaikki_root`.
    ///
    /// # Errors
    ///
    /// Returns [`DictError::Malformed`] only when no local meanings were found and an installed Kaikki pack was
    /// unreadable/corrupt.
    pub fn lookup_with_kaikki(
        &mut self,
        word: &str,
        kaikki_root: &Path,
    ) -> Result<Vec<OfflineMeaning>, DictError> {
        let mut out = self.lookup(word);
        let Some(key) = word_key(word) else {
            return Ok(out);
        };
        let mut last_error = None;
        for pack in &KAIKKI_PACKS {
            if out.len() >= MAX_LOOKUP_RESULTS {
                break;
            }
            match lookup_kaikki_records(pack.id, kaikki_root, &key) {
                Ok(records) => {
                    for rec in records {
                        if out.len() >= MAX_LOOKUP_RESULTS {
                            break;
                        }
                        if !parse_kaikki_record(&rec, &key, pack, &mut out) {
                            last_error =
                                Some("installed Kaikki entry could not be parsed".to_owned());
                        }
                    }
                }
                Err(err) => {
                    last_error = Some(err.to_string());
                }
            }
        }
        if out.is_empty()
            && let Some(msg) = last_error
        {
            return Err(DictError::Malformed(msg));
        }
        Ok(out)
    }

    /// Returns the starting index of `word` in the sorted inflection table (binary search).
    #[must_use]
    pub fn find_inflection_index(&mut self, word: &str) -> usize {
        self.finalize();
        self.inflections
            .partition_point(|inf| inf.word.as_str() < word)
    }

    /// Returns every lemma mapped from `key` in insertion order.
    #[must_use]
    pub fn inflection_lemmas(&self, key: &str) -> Vec<String> {
        let start = self
            .inflections
            .partition_point(|inf| inf.word.as_str() < key);
        let mut lemmas = Vec::new();
        for inf in &self.inflections[start..] {
            if inf.word != key {
                break;
            }
            if !lemmas.contains(&inf.lemma) {
                lemmas.push(inf.lemma.clone());
            }
        }
        lemmas
    }

    fn match_key(&self, key: &str, out: &mut Vec<OfflineMeaning>, scope: MatchScope) {
        let start = self
            .entries
            .partition_point(|entry| entry.key.as_str() < key);
        for entry in &self.entries[start..] {
            if entry.key != key || out.len() >= MAX_LOOKUP_RESULTS {
                break;
            }
            if scope == MatchScope::WmOnly && entry.meaning.dictionary_id != WM_VOCAB_ID {
                continue;
            }
            let duplicate = out.iter().any(|existing| {
                existing.dictionary_id == entry.meaning.dictionary_id
                    && existing.definition == entry.meaning.definition
                    && existing.part_of_speech == entry.meaning.part_of_speech
            });
            if !duplicate {
                out.push(entry.meaning.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchScope {
    All,
    WmOnly,
}

/// Formats grouped dictionary meanings by part of speech (`"verb\n1. ...\nExample: ..."`), matching Enhanced's
/// `DictionaryMeaningText`.
#[must_use]
pub fn meaning_text(meanings: &[OfflineMeaning]) -> String {
    let mut out = String::new();
    let mut last_word = "";
    let mut last_pos = "";
    let mut number = 0_usize;
    for m in meanings {
        if !m.part_of_speech.is_empty() && (last_pos != m.part_of_speech || last_word != m.headword)
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&m.part_of_speech);
            out.push('\n');
            number = 0;
        }
        last_word = &m.headword;
        last_pos = &m.part_of_speech;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        number += 1;
        out.push_str(&format!(
            "{number}. {}",
            dictionary_plain_text(&m.definition)
        ));
        if !m.example.is_empty() {
            out.push_str(&format!("\nExample: {}", m.example));
        }
    }
    out
}

/// Parses JSON responses from Kaikki JSONL (`Wiktionary`), Wiktionary REST (`WiktionaryRest`) or Free Dictionary API
/// (`FreeDictionary`) into [`OfflineMeaning`] entries (matching Enhanced's `ParseOnlineDictionary`).
#[must_use]
pub fn parse_online_dictionary(
    body: &str,
    query: &str,
    source: DictionarySource,
    out: &mut Vec<OfflineMeaning>,
) -> bool {
    match source {
        DictionarySource::Wiktionary => {
            let mut parsed_any = false;
            for raw_line in body.lines() {
                if out.len() >= MAX_LOOKUP_RESULTS {
                    break;
                }
                let line = raw_line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(val) = serde_json::from_str::<Value>(line) else {
                    return false;
                };
                parse_kaikki_json_value(&val, out);
                parsed_any = true;
            }
            parsed_any
        }
        DictionarySource::WiktionaryRest => {
            let Ok(val) = serde_json::from_str::<Value>(body) else {
                return false;
            };
            let Some(blocks) = val.get("en").and_then(Value::as_array) else {
                return true;
            };
            for block in blocks {
                let pos = block
                    .get("partOfSpeech")
                    .and_then(Value::as_str)
                    .map(full_pos)
                    .unwrap_or_default();
                let Some(defs) = block.get("definitions").and_then(Value::as_array) else {
                    continue;
                };
                for def_obj in defs {
                    if out.len() >= MAX_LOOKUP_RESULTS {
                        return true;
                    }
                    let raw_def = def_obj
                        .get("definition")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let cleaned = dictionary_plain_text(raw_def);
                    if cleaned.is_empty() {
                        continue;
                    }
                    let example = def_obj
                        .get("examples")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.first())
                        .and_then(Value::as_str)
                        .map(plain_definition)
                        .unwrap_or_default();
                    out.push(OfflineMeaning {
                        headword: query.to_owned(),
                        definition: cleaned,
                        dictionary: "Wiktionary".to_owned(),
                        dictionary_id: "wiktionary-rest-en".to_owned(),
                        part_of_speech: pos.clone(),
                        example,
                        source_url: format!(
                            "https://en.wiktionary.org/wiki/{}",
                            percent_encode(query)
                        ),
                        license:
                            "Wiktionary content: CC BY-SA; source attribution and licensing apply."
                                .to_owned(),
                        ..OfflineMeaning::default()
                    });
                }
            }
            true
        }
        DictionarySource::FreeDictionary => {
            let Ok(val) = serde_json::from_str::<Value>(body) else {
                return false;
            };
            let Some(entries) = val.as_array() else {
                // Free Dictionary API returns `{"title":"No Definitions Found"}` on 404/empty.
                return val.is_object();
            };
            for item in entries {
                let headword = item.get("word").and_then(Value::as_str).unwrap_or("");
                if headword.is_empty() {
                    continue;
                }
                let mut phonetic = item
                    .get("phonetic")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let mut audio_url = String::new();
                if let Some(phonetics) = item.get("phonetics").and_then(Value::as_array) {
                    for p in phonetics {
                        let text = p.get("text").and_then(Value::as_str).unwrap_or("");
                        let audio = p
                            .get("audio")
                            .and_then(Value::as_str)
                            .filter(|u| u.starts_with("https://"))
                            .unwrap_or("");
                        if !text.is_empty() && phonetic.is_empty() {
                            phonetic = text.to_owned();
                        }
                        if !audio.is_empty() && audio_url.is_empty() {
                            audio_url = audio.to_owned();
                            if !text.is_empty() {
                                phonetic = text.to_owned();
                            }
                        }
                    }
                }
                let license = item
                    .get("license")
                    .and_then(Value::as_object)
                    .map(|lic| {
                        let name = lic.get("name").and_then(Value::as_str).unwrap_or("");
                        let url = lic.get("url").and_then(Value::as_str).unwrap_or("");
                        if !name.is_empty() && !url.is_empty() {
                            format!("{name}: {url}")
                        } else {
                            name.to_owned()
                        }
                    })
                    .unwrap_or_default();
                let source_url = item
                    .get("sourceUrls")
                    .and_then(Value::as_array)
                    .and_then(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .find(|u| u.starts_with("https://"))
                    })
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "https://en.wiktionary.org/wiki/{}",
                            percent_encode(headword)
                        )
                    });

                let Some(meanings) = item.get("meanings").and_then(Value::as_array) else {
                    continue;
                };
                for m in meanings {
                    let pos = m
                        .get("partOfSpeech")
                        .and_then(Value::as_str)
                        .map(full_pos)
                        .unwrap_or_default();
                    let block_syns = collect_string_list(m.get("synonyms"));
                    let block_ants = collect_string_list(m.get("antonyms"));
                    let Some(defs) = m.get("definitions").and_then(Value::as_array) else {
                        continue;
                    };
                    for d in defs {
                        if out.len() >= MAX_LOOKUP_RESULTS {
                            return true;
                        }
                        let raw_def = d.get("definition").and_then(Value::as_str).unwrap_or("");
                        let cleaned = dictionary_plain_text(raw_def);
                        if cleaned.is_empty() {
                            continue;
                        }
                        let mut syns = block_syns.clone();
                        for s in collect_string_list(d.get("synonyms")) {
                            push_unique_ci(&mut syns, &s);
                        }
                        let mut ants = block_ants.clone();
                        for a in collect_string_list(d.get("antonyms")) {
                            push_unique_ci(&mut ants, &a);
                        }
                        out.push(OfflineMeaning {
                            headword: headword.to_owned(),
                            definition: cleaned,
                            dictionary: "Free Dictionary API".to_owned(),
                            dictionary_id: "dictionary-api-en".to_owned(),
                            part_of_speech: pos.clone(),
                            example: d
                                .get("example")
                                .and_then(Value::as_str)
                                .map(plain_definition)
                                .unwrap_or_default(),
                            synonyms: syns.join("\n"),
                            antonyms: ants.join("\n"),
                            phonetic: phonetic.clone(),
                            audio_url: audio_url.clone(),
                            source_url: source_url.clone(),
                            license: license.clone(),
                            ..OfflineMeaning::default()
                        });
                    }
                }
            }
            true
        }
        DictionarySource::Offline => false,
    }
}

fn parse_kaikki_json_value(val: &Value, out: &mut Vec<OfflineMeaning>) {
    let headword = val.get("word").and_then(Value::as_str).unwrap_or("");
    let lang = val.get("lang_code").and_then(Value::as_str).unwrap_or("");
    if headword.is_empty() || (!lang.is_empty() && lang != "en") {
        return;
    }
    let pos = val
        .get("pos")
        .and_then(Value::as_str)
        .map(full_pos)
        .unwrap_or_default();

    let mut phonetic = String::new();
    let mut phonetic_uk = String::new();
    let mut audio_url = String::new();
    let mut audio_url_uk = String::new();

    if let Some(sounds) = val.get("sounds").and_then(Value::as_array) {
        for sound in sounds {
            let tags = collect_string_list(sound.get("tags")).join("\n");
            let british = tags.contains("UK")
                || tags.contains("British")
                || tags.contains("Received-Pronunciation");
            let american = tags.contains("US") || tags.contains("General-American");
            let ipa = sound
                .get("ipa")
                .or_else(|| sound.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let audio = sound
                .get("mp3_url")
                .or_else(|| sound.get("audio"))
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://"))
                .unwrap_or("");

            let target_ipa = if british {
                &mut phonetic_uk
            } else {
                &mut phonetic
            };
            let target_audio = if british {
                &mut audio_url_uk
            } else {
                &mut audio_url
            };
            if !ipa.is_empty() && (target_ipa.is_empty() || american) {
                *target_ipa = ipa.to_owned();
            }
            if !audio.is_empty() && (target_audio.is_empty() || american) {
                *target_audio = audio.to_owned();
                if !ipa.is_empty() {
                    *target_ipa = ipa.to_owned();
                }
            }
        }
    }

    let block_syns = collect_kaikki_related(val.get("synonyms"));
    let block_ants = collect_kaikki_related(val.get("antonyms"));

    let Some(senses) = val.get("senses").and_then(Value::as_array) else {
        return;
    };
    for sense in senses {
        if out.len() >= MAX_LOOKUP_RESULTS {
            return;
        }
        // Enhanced takes the most specific (last) gloss in `glosses[]`.
        let raw_def = sense
            .get("glosses")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().rev().find_map(Value::as_str))
            .unwrap_or("");
        let cleaned = dictionary_plain_text(raw_def);
        if cleaned.is_empty() {
            continue;
        }
        let example = sense
            .get("examples")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find_map(|e| e.get("text").and_then(Value::as_str))
            })
            .map(plain_definition)
            .unwrap_or_default();

        let mut syns = block_syns.clone();
        for s in collect_kaikki_related(sense.get("synonyms")) {
            push_unique_ci(&mut syns, &s);
        }
        let mut ants = block_ants.clone();
        for a in collect_kaikki_related(sense.get("antonyms")) {
            push_unique_ci(&mut ants, &a);
        }

        out.push(OfflineMeaning {
            headword: headword.to_owned(),
            definition: cleaned,
            dictionary: "Wiktionary via Kaikki".to_owned(),
            dictionary_id: "wiktionary-en".to_owned(),
            part_of_speech: pos.clone(),
            example,
            synonyms: syns.join("\n"),
            antonyms: ants.join("\n"),
            phonetic: phonetic.clone(),
            phonetic_uk: phonetic_uk.clone(),
            audio_url: audio_url.clone(),
            audio_url_uk: audio_url_uk.clone(),
            source_url: format!(
                "https://en.wiktionary.org/wiki/{}",
                percent_encode(headword)
            ),
            license: "Wiktionary content: CC BY-SA; source attribution and licensing apply."
                .to_owned(),
        });
    }
}

fn maybe_gunzip(raw: &[u8]) -> Result<Vec<u8>, DictError> {
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut decoder = GzDecoder::new(raw);
        let mut out = Vec::new();
        let mut limited = decoder.by_ref().take(MAX_DICTIONARY_BYTES as u64 + 1);
        limited.read_to_end(&mut out)?;
        if out.len() > MAX_DICTIONARY_BYTES {
            return Err(DictError::Malformed(
                "compressed dictionary exceeds 256 MiB".to_owned(),
            ));
        }
        Ok(out)
    } else {
        Ok(raw.to_vec())
    }
}

fn split_wordnet_gloss(gloss: &str) -> (&str, String) {
    let Some(first_quote) = gloss.find('"') else {
        return (gloss, String::new());
    };
    let definition = gloss[..first_quote].trim().trim_end_matches(';').trim();
    let mut examples = String::new();
    let mut rem = &gloss[first_quote..];
    while let Some(start) = rem.find('"') {
        let after = &rem[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        if !examples.is_empty() {
            examples.push('\n');
        }
        examples.push_str(&after[..end]);
        rem = &after[end + 1..];
    }
    (definition, examples)
}

fn collect_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr.iter().filter_map(Value::as_str) {
        push_unique_ci(&mut out, item);
    }
    out
}

fn collect_kaikki_related(value: Option<&Value>) -> Vec<String> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        if let Some(word) = item.get("word").and_then(Value::as_str) {
            push_unique_ci(&mut out, word);
        }
    }
    out
}

fn push_unique_ci(list: &mut Vec<String>, item: &str) {
    let trimmed = item.trim();
    if trimmed.is_empty() || trimmed.len() > 2_048 {
        return;
    }
    if !list
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        list.push(trimmed.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Port of SumatraPDF Enhanced's `OfflineDictionary_UnitTests()` covering inflections, WM JSON, WordNet,
    /// TSV, StarDict, online JSON schemas and lookup priority.
    #[test]
    fn matches_enhanced_offline_dictionary_unit_tests() {
        // Inflection binary search ordering.
        let mut inf_index = DictionaryIndex::new();
        for i in (0..8192).rev() {
            inf_index.add_inflection(&format!("word{i:05}"), "first");
        }
        inf_index.add_inflection("word04000", "second");
        let found = inf_index.find_inflection_index("word04000");
        assert_eq!(found, 4000);
        assert_eq!(
            inf_index.inflection_lemmas("word04000"),
            vec!["first", "second"]
        );
        assert_eq!(inf_index.find_inflection_index("word00000"), 0);
        assert_eq!(
            inf_index.find_inflection_index("zzzz"),
            inf_index.inflection_count()
        );

        // WM JSON pack with IPA, audio, forms->inflections, synonyms and grouped meaning_text.
        let mut wm = DictionaryIndex::new();
        let wm_record = concat!(
            "{\"format\":\"wmkeyboard-vocab\",\"version\":1,\"pack\":{\"name\":\"Test dictionary\"},",
            "\"words\":[{\"word\":\"abhor\",\"ipa\":{\"us\":\"/æbˈhɔɹ/\",\"uk\":\"/əbˈhɔː/\"},",
            "\"audio\":{\"us\":\"https://example.org/abhor.mp3\"},\"forms\":[\"abhorred\"],",
            "\"synonyms\":[\"hate\"],\"senses\":[{\"pos\":\"verb\",\"definition\":\"To detest.\",",
            "\"example\":\"I abhor waiting.\",\"synonyms\":[\"loathe\"]},",
            "{\"pos\":\"verb\",\"definition\":\"To reject.\"}]}]}"
        );
        assert_eq!(
            wm.load_wm_json(wm_record.as_bytes(), "Test").expect("wm"),
            2
        );
        assert_eq!(wm.inflection_count(), 1);
        let meanings = wm.lookup("abhorred");
        assert_eq!(meanings.len(), 2);
        assert_eq!(meanings[0].part_of_speech, "verb");
        assert_eq!(meanings[0].synonyms, "hate\nloathe");
        assert_eq!(meanings[0].phonetic_uk, "/əbˈhɔː/");
        assert_eq!(meanings[0].audio_url, "https://example.org/abhor.mp3");
        assert_eq!(
            meaning_text(&meanings),
            "verb\n1. To detest.\nExample: I abhor waiting.\n2. To reject."
        );
        assert!(
            wm.load_wm_json(b"{\"format\":\"wrong\",\"words\":[]}", "Bad")
                .is_err()
        );

        // WordNet synset + exception parsing.
        let mut wn = DictionaryIndex::new();
        assert_eq!(
            wn.load_wordnet_data(
                "00001740 03 n 02 cat 0 feline 0 000 | a small domesticated mammal; \"a pet cat\"\n"
            )
            .expect("wordnet"),
            2
        );
        wn.load_wordnet_exceptions("kitties cat\n");
        let cat = wn.lookup("cats");
        assert_eq!(cat.len(), 1);
        assert_eq!(cat[0].definition, "a small domesticated mammal");
        assert_eq!(cat[0].part_of_speech, "noun");
        assert_eq!(cat[0].example, "a pet cat");
        assert_eq!(cat[0].synonyms, "feline");
        assert_eq!(wn.lookup("kitties").len(), 1);
        assert!(
            wn.load_wordnet_data("00001740 03 n FF entity 0 | invalid word count\n")
                .is_err()
        );

        // TSV parsing + invalid UTF-8 / missing tab rejection.
        let mut tsv = DictionaryIndex::new();
        assert_eq!(
            tsv.load_tsv(b"cat\ta small mammal\nCAT\ta feline\n", "test", "Test")
                .expect("tsv"),
            2
        );
        assert_eq!(tsv.lookup("Cat").len(), 2);
        assert!(tsv.load_tsv(b"missing tab\n", "bad", "Bad").is_err());
        assert!(
            tsv.load_tsv(&[b'w', b'\t', 0xff, b'\n'], "bad", "Bad")
                .is_err()
        );

        // StarDict `.ifo` / `.idx` / `.dict` / `.syn` parsing + validation.
        let ifo = b"StarDict's dict ifo file\nversion=2.4.2\nsametypesequence=m\n";
        let idx = [b'c', b'a', b't', 0, 0, 0, 0, 0, 0, 0, 0, 6];
        let syn = [b'k', b'i', b't', b't', b'y', 0, 0, 0, 0, 0];
        let mut star = DictionaryIndex::new();
        assert_eq!(
            star.load_stardict(ifo, &idx, b"feline", Some(&syn), "star", "Star")
                .expect("stardict"),
            2
        );
        assert_eq!(star.lookup("kitty")[0].definition, "feline");
        assert!(
            star.load_stardict(ifo, &idx[..idx.len() - 1], b"feline", None, "s", "S")
                .is_err()
        );
        assert!(
            star.load_stardict(ifo, &idx, b"tiny", None, "s", "S")
                .is_err()
        );
        assert!(
            star.load_stardict(b"not an ifo", &idx, b"feline", None, "s", "S")
                .is_err()
        );
        assert!(
            star.load_stardict(
                b"StarDict's dict ifo file\nidxoffsetbits=64\n",
                &idx,
                b"feline",
                None,
                "s",
                "S"
            )
            .is_err()
        );

        // Online JSON parsers (FreeDictionary, Kaikki, WiktionaryRest).
        let mut online = Vec::new();
        let api = concat!(
            "[{\"word\":\"clear\",\"phonetic\":\"/klɪə/\",\"phonetics\":[{\"text\":\"/klɪr/\",",
            "\"audio\":\"https://example.org/clear.mp3\"}],\"license\":{\"name\":\"CC BY-SA 3.0\",",
            "\"url\":\"https://creativecommons.org/licenses/by-sa/3.0/\"},",
            "\"meanings\":[{\"partOfSpeech\":\"adjective\",\"synonyms\":[\"plain\"],\"antonyms\":[\"obscure\"],",
            "\"definitions\":[{\"definition\":\"Easy to understand.\",\"example\":\"A clear explanation.\",",
            "\"synonyms\":[\"lucid\"]},{\"definition\":\"Transparent.\"}]},",
            "{\"partOfSpeech\":\"verb\",\"definitions\":[{\"definition\":\"To remove.\"}]}]}]"
        );
        assert!(parse_online_dictionary(
            api,
            "clear",
            DictionarySource::FreeDictionary,
            &mut online
        ));
        assert_eq!(online.len(), 3);
        assert_eq!(online[0].synonyms, "plain\nlucid");
        assert_eq!(online[0].phonetic, "/klɪr/");
        assert_eq!(online[0].audio_url, "https://example.org/clear.mp3");

        online.clear();
        let rest = "{\"en\":[{\"partOfSpeech\":\"Noun\",\"definitions\":[{\"definition\":\"A <b>small</b> mammal.\",\"examples\":[\"A cat sleeps.\"]},{\"definition\":\"A person.\"}]}]}";
        assert!(parse_online_dictionary(
            rest,
            "cat",
            DictionarySource::WiktionaryRest,
            &mut online
        ));
        assert_eq!(online.len(), 2);
        assert_eq!(online[0].definition, "A small mammal.");
    }

    /// Done when (S.6): lookups over a large dictionary index take well under 10 ms.
    #[test]
    fn offline_lookup_completes_under_10ms() {
        let mut dict = DictionaryIndex::new();
        let mut tsv = String::with_capacity(20_000 * 48);
        for i in 0..20_000 {
            tsv.push_str(&format!(
                "lexeme{i:05}\tDefinition of lexeme {i} in the study dictionary.\n"
            ));
        }
        dict.load_tsv(tsv.as_bytes(), "bench", "Bench")
            .expect("load");
        dict.finalize();

        let started = Instant::now();
        let results = dict.lookup("LEXEME12345");
        let elapsed = started.elapsed();
        assert_eq!(results.len(), 1);
        assert!(
            elapsed.as_millis() < 10,
            "lookup took {} ms (budget < 10 ms)",
            elapsed.as_millis()
        );
    }
}
