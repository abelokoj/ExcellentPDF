# Ph7: 26/32 | O.15: Validate signatures and trust

**Tag:** O.15; **Priority:** P1; **Phase:** 7; **Needs:** O.14; **Code:** `ecclesia-core::signing` (`pdf_sig.rs`, `cms.rs`, `der.rs`), `ecclesia-features::signing::panel`
**Plan:** §27 entry `#### Ph7: 26/32 | O.15`.

## Problem

Opening a signed PDF must tell the reader whether the document was altered after signing, whether the signer's certificate chains to a trusted authority, and allow inspecting the exact signed revision.

## User story

As a document reviewer, I want a signature panel showing each signature's trust status (`Trusted`, `UnknownIssuer`, `Tampered`, `ByteRangeIncomplete`), signer details, and a "View signed version" action.

## UI sketch

- **Signatures** sidebar panel listing signatures in revision order with trust badge, signer `CN`, issuer, signing time, reason, location, and **View signed version**.

## Mode

All document reading modes on PDFs containing `/Sig` or `/DocTimeStamp` objects.

## Speed and size impact

- Startup: zero startup cost; verification runs when inspecting signatures.

## Design

- **`verify_pdf_signatures` (`ecclesia_core::signing::pdf_sig`)**: Locates all `/Type /Sig` and `/Type /DocTimeStamp` dictionaries, validates `/ByteRange` bracket geometry and `%%EOF` termination, recomputes SHA-256 digests, verifies CMS Ed25519 signatures and `signing-certificate-v2` bindings, evaluates X.509 chains against `TrustStore`, and detects post-signing revisions (`modified_after_signing`).
- **Panel & revision viewer (`ecclesia_features::signing::panel`)**: `SignaturePanelModel::inspect` builds `SignatureSummary` rows and `extract_signed_revision` slices the exact byte prefix covered by a signature for "View signed version".

## Acceptance tests

- [x] `signs_verifies_trust_and_detects_tampering`: proves `Trusted`, `UnknownIssuer`, `Tampered` (single-byte modification inside signed range), and `ByteRangeIncomplete` (corrupted EOF marker).
- [x] `workflow_signs_lta_inspections_extracts_revision_and_guards_edits`: verifies `SignaturePanelModel` and opening `extract_signed_revision` in MuPDF.
