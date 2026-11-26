# Ph7: 18/32 | O.7: True redaction

**Tag:** O.7; **Priority:** P1; **Phase:** 7; **Needs:** A.9, F.4; **Code:** `excellent-core::redact`
**Plan:** §27 entry `#### Ph7: 18/32 | O.7`.

## Problem

Drawing black boxes over sensitive text leaves underlying text operators, copy-paste strings, and metadata completely readable; true redaction must strip all content-stream text, vectors, and images inside redaction boxes and verify removal afterward.

## User story

As a legal or compliance user, I want two-phase redaction (mark targets by box or pattern, review marks, then permanently apply) with verification that redacted strings cannot be extracted afterward.

## UI sketch

- Redact toolbar: `Mark Region`, `Find Patterns` (`SSN`, `Email`, `Credit Card`, `Phone`, custom text), preview callout boxes, `Scrub Metadata / Attachments / Hidden Layers`, and irreversible `Apply Redactions` confirmation.

## Mode

Redact tool in Organize / Annotate mode.

## Speed and size impact

- Rewrites non-incrementally (`rewrite_clean`) so previous incremental revisions are discarded, ensuring zero leaked history.

## Design

- **Phase 1 (`RedactionMark`, `mark_from_text_layout`, `RedactionPattern`)**: Identifies text spans/boxes via regex-free pattern scanners (`Ssn`, `Email`, `CreditCard`, `Phone`, `Literal`) and records reviewable `RedactionMark` rects.
- **Phase 2 (`apply_redactions`)**: Filters content streams (`strip_redacted_content`), drops intersecting annotations/forms/hidden OCGs/Info/XMP metadata per `RedactionOptions`, paints permanent solid fills over redacted rects, and rewrites cleanly without `/Prev` history.
- **Verification (`verify_redaction`)**: Reopens the output PDF via MuPDF, searches extracted text for forbidden terms and redaction patterns, and returns `RedactionVerification` (`is_clean`).

## Acceptance tests

- [x] `two_phase_redaction_permanently_strips_secrets_and_verifies_clean`: marks SSN, email, credit card, and literal secrets, applies redactions with metadata scrubbing, confirms MuPDF cannot extract redacted strings, and asserts `verify_redaction` is clean.
