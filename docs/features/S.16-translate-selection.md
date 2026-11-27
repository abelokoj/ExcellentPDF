# Ph5: 27/45 | S.16: Translate selection

**Tag:** S.16; **Priority:** P1 (parity); **Phase:** 5; **Needs:** K.13; **Code:** `ecclesia-features::translate`
**Plan:** §27 entry `#### Ph5: 27/45 | S.16: Translate selection` and SumatraPDF `src/SelectionTranslate.cpp`, `src/TranslationLangs.cpp`.

## Problem

Readers encountering passages in another language need quick selection translation that respects privacy (consent before network access) and works offline via language packs.

## User story

As a reader, I want to translate selected text offline using installed Bergamot language packs, or explicitly consent to online Google/DeepL translation across 73 languages.

## UI sketch

- Context menu / Command Palette (`study.translate_selection`) opens translation popover with source (`Auto` or language) and target dropdowns (`SUPPORTED_LANGUAGES`), engine picker, and consent prompt for online providers.

## Mode

Read, Study.

## Speed and size impact

- Startup: zero cost; offline Bergamot packs load on demand from `packs/translate/`.
- Privacy: `ConsentPolicy::OfflineOnly` enforces zero network requests unless `ConsentPolicy::OnlineConsented` is explicitly given.

## Design

- **Model (`ecclesia-features::translate`):**
  - 73-language catalog (`SUPPORTED_LANGUAGES`) ported from SumatraPDF Enhanced's `TranslationLangs.cpp`.
  - `BergamotPackCatalog` matches installed `<src>-<dst>` offline packs; `SelectionTranslator` enforces `ConsentPolicy` before any `ureq` network call.

## Acceptance tests

- [x] `translate::tests::offline_translation_works_with_network_disabled`: verifies offline translation succeeds with `ConsentPolicy::OfflineOnly`.
- [x] `translate::tests::online_engine_refuses_without_explicit_consent_and_catalog_has_73_languages`: verifies consent gating and full 73-language catalog.
