# Ph5: 34/45 | F.8: Duplicate detection

**Tag:** F.8; **Priority:** P1; **Phase:** 5; **Needs:** F.6; **Code:** `ecclesia-features::library::metadata`, `ecclesia-store::library`
**Plan:** §27 entry `#### Ph5: 34/45 | F.8`.

## Problem

Users accumulate identical copies of PDFs as well as related versions (such as an arXiv preprint and its published journal PDF) without noticing.

## User story

As a researcher, I want EcclesiaPDF to detect exact file duplicates and flag DOI/text-similar versions as related rather than identical, so that I can deduplicate without losing distinct editions.

## UI sketch

- Library "Find duplicates" tool listing groups separated into "Exact copies" and "Related versions (DOI / text similarity)".

## Mode

All modes.

## Speed and size impact

- Startup: computed only during library import or duplicate scan.
- Budgets touched: O(1) indexed SHA-256 hash lookup for exact copies; compact 64-integer MinHash signatures stored per document for fast pairwise similarity.
- Installer size: 0 bytes added (uses `sha2` already in workspace).

## Design

- Model: `ecclesia-features::library::metadata::{sha256_bytes, sha256_file, compute_minhash, minhash_similarity, detect_duplicates, DuplicateKind, DuplicatePair}`.
- Storage: `content_hash` and `minhash_json` columns in `library_item`.

## Acceptance tests

- [x] `preprint_and_published_version_are_flagged_related_not_identical`: Flags identical bytes as `DuplicateKind::Exact` and preprint vs published versions as `DuplicateKind::NearText` (or `RelatedDoi`), not identical.
