//! Offline dictionary (plan S.6): parity port of SumatraPDF Enhanced's `OfflineDictionary.cpp` and
//! `KaikkiDictionary.cpp`, including `WordKey` normalization, English inflection/stemming lookup, TSV / StarDict /
//! WM JSON / WordNet parsers, and Kaikki JSONL 256-bucket FNV-1a 64-bit indexed packs with hash-collision
//! verification.

pub mod index;
pub mod kaikki;
pub mod normalize;

pub use index::{
    DictError, DictInflection, DictionaryIndex, DictionarySource, OfflineDictPack, OfflineMeaning,
    meaning_text, parse_online_dictionary,
};
pub use kaikki::{
    KAIKKI_PACKS, KaikkiError, KaikkiPack, KaikkiStoreBuilder, StoreHeader, build_kaikki_store,
    is_valid_generation, kaikki_pack_installed, lookup_kaikki_records, parse_kaikki_record,
    remove_kaikki_pack,
};
pub use normalize::{
    dictionary_plain_text, english_stem_candidates, full_pos, percent_encode, plain_definition,
    word_hash, word_key,
};
