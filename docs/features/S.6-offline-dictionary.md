# Ph4: 32/44 | S.6: Offline dictionary

**Tag:** S.6; **Priority:** P0 (parity); **Phase:** 4; **Needs:** K.21, K.13; **Code:** `ecclesia-features::dictionary`
**Plan:** §27 entry `#### Ph4: 32/44 | S.6`, §11.7, and `sumatra_pdf_enhanced_plan.md` §5.12. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers encounter unfamiliar words while reading offline and need instant, zero-network dictionary lookups supporting inflected forms, imported dictionaries (TSV, StarDict, WM JSON, WordNet) and large Wiktionary/Kaikki packs.

## User story

As a reader, I want to double-click or press Shift+D on a word to see its definitions, phonetics, examples and synonyms offline in under 10 ms without sending any document text over the network.

## UI sketch

- Triggered via double-click/tap on a K.13 word boundary, `Shift+D`, context menu "Look up word", or command palette (`dictionary.lookup`).
- Displays headword, part of speech, IPA (US/UK), numbered definitions, examples, synonyms/antonyms and source/licence attribution, plus "Add to vocabulary" (S.7) and "Pronounce" (U.5).
- Empty/offline state explains how to install a dictionary pack (K.21) or import a `.tsv`, StarDict `.ifo` or `.wmvocab.json(.gz)` file.

## Mode

All modes (context menu, shortcut `Shift+D`, command palette); Study mode toolbar.

## Speed and size impact

- Startup: nothing loads at startup; `DictionaryIndex` and Kaikki stores open lazily on first lookup.
- Budgets touched (plan §13.1): lookup latency < 10 ms via O(log N) binary search over sorted vectors and 24-byte binary `index.bin`.
- Installer size: dictionary data ships as optional packs (K.21), never inside the core installer.

## Design

- **Normalization & stemming** (`dictionary::normalize`): `word_key`, FNV-1a 64-bit `word_hash`, `plain_definition`, `dictionary_plain_text`, `full_pos`, `english_stem_candidates`, `percent_encode`.
- **Kaikki packs** (`dictionary::kaikki`): 256-bucket FNV-1a 64-bit `KaikkiStoreBuilder`, `StoreHeader`/`StoreEntry` (u64 offsets supporting > 4 GiB), `manifest.json`, atomic `current` generation pointer with path-traversal validation (`is_valid_generation`), gzip streaming via `flate2`, binary search `lookup_kaikki_records` with full JSONL headword/forms collision verification, and `parse_kaikki_record`.
- **In-memory index & parsers** (`dictionary::index`): `DictionaryIndex` supporting TSV, StarDict (`.ifo`/`.idx`[`.gz`]/`.dict`[`.dz`]/`.syn`), WM JSON (`.wmvocab.json[.gz]`), WordNet (`data.*` + `*.exc`), online JSON schemas, inflection binary search (`DictInflection`), priority lookup and `meaning_text`.

## Acceptance tests

- [x] `dictionary::normalize::tests::word_key_and_plain_definition_match_enhanced`: WordKey normalization, HTML stripping, entity decoding and child-gloss cleaning match Enhanced.
- [x] `dictionary::kaikki::tests::matches_enhanced_kaikki_unit_tests`: ported Kaikki index tests pass (streaming, gzip, cancellation, collision verification, corrupt u64 offset rejection).
- [x] `dictionary::index::tests::matches_enhanced_offline_dictionary_unit_tests`: ported OfflineDictionary tests pass (TSV, StarDict, WM JSON, WordNet, inflections, online JSON schemas).
- [x] `dictionary::index::tests::offline_lookup_completes_under_10ms`: lookup over a 20,000-entry index completes well under 10 ms.
