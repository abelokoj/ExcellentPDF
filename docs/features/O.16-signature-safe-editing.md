# Ph7: 27/32 | O.16: Signature-safe editing

**Tag:** O.16; **Priority:** P1; **Phase:** 7; **Needs:** O.15; **Code:** `ecclesia-core::signing::mdp`, `ecclesia-features::signing::panel`
**Plan:** §27 entry `#### Ph7: 27/32 | O.16`.

## Problem

Editing a signed PDF carelessly—or rewriting the entire file instead of appending an incremental update—silently invalidates existing digital signatures.

## User story

As a user working on a signed PDF, I want edits permitted by `/DocMDP` (such as filling forms or co-signing under P2, or annotating under P3) to save incrementally without breaking signatures, and to receive a warning before any edit that would invalidate a signature.

## UI sketch

- Before applying structural, content, or disallowed annotation/form edits to a signed PDF, a confirmation prompt names the affected signatures and explains the `/DocMDP` or `/FieldMDP` restriction.

## Mode

All editing modes on PDFs with at least one digital signature.

## Speed and size impact

- Startup: zero startup cost.
- Save: strictly appends incremental updates (`enforce_incremental_save`), avoiding full file rewrites.

## Design

- **`ecclesia_core::signing::mdp`**:
  - Models `DocMdpPermission` (`NoChangesAllowed` P1, `FormFillAndSign` P2, `FormFillSignAndAnnotate` P3) and `FieldMdpRule` (`All`, `Include`, `Exclude`).
  - `inspect_edit_safety` evaluates `DocumentEditKind` (`FormFill`, `Sign`, `Annotation`, `PageStructure`, `ContentEdit`, `MetadataOrSecurity`) against active signatures.
  - `enforce_incremental_save` guarantees the updated PDF preserves the signed PDF bytes as an exact prefix.
- **`ecclesia_features::signing::evaluate_edit_guard`**: Maps safety reports to `EditGuardDecision::ProceedSilently` or `EditGuardDecision::WarnUser`.

## Acceptance tests

- [x] `doc_mdp_permits_or_blocks_edits_and_enforces_incremental_save`: verifies P2 allows incremental form filling while keeping the certification signature `Trusted`, blocks annotations/content edits, and rejects non-incremental rewrites.
