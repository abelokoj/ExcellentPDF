# Ph9: 11/13 | Y.3: Share bundles

**Tag:** Y.3; **Priority:** P2; **Phase:** 9; **Needs:** N.4; **Code:** `excellent-features::sync::bundle`
**Plan:** §27 entry `#### Ph9: 11/13 | Y.3`.

## Problem

Sharing a PDF with collaborators along with annotations, connected notes, research boards, and audio memos requires sending multiple disconnected files that create duplicates on re-import.

## User story

As a student or reviewer, I want to export and import a signed `.exbundle` archive containing the PDF, annotations, notes, boards, and audio clips that merges idempotently by stable IDs.

## UI sketch

- File -> Share Bundle (`.exbundle`) export/import wizard with optional Ed25519 signature verification badge.

## Mode

Read, Annotate, Study, Research.

## Speed and size impact

- Startup: bundle packager/unpacker runs only on explicit export/import.
- Budgets touched: streaming ZIP archive + SHA-256 verification.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::sync::bundle::{ShareBundle, BundleVerification, pack_exbundle, unpack_exbundle, merge_bundle_into_state}`.

## Acceptance tests

- [ ] `signed_exbundle_roundtrips_between_users_without_duplicates`: Packs a signed `.exbundle`, rejects wrong verifying keys, unpacks cleanly, and merges twice into CRDT state with zero duplicates.
