# Ph5: 33/45 | F.7: Metadata extraction

**Tag:** F.7; **Priority:** P1; **Phase:** 5; **Needs:** F.6; **Code:** `excellent-features::library::metadata`
**Plan:** §27 entry `#### Ph5: 33/45 | F.7`.

## Problem

Academic PDFs often have unhelpful filenames and missing `/Info` dictionaries even though DOI, arXiv ID, or ISBN appear on the first pages.

## User story

As a researcher, I want ExcellentPDF to extract title, authors, year, DOI, arXiv ID, and ISBN from PDF info, XMP, and opening pages—and optionally enrich them online with my consent—so that my library has clean bibliographic metadata.

## UI sketch

- Automatic metadata extraction on library import; inspector shows extracted DOI/arXiv/ISBN with an explicit "Look up metadata online…" consent button.

## Mode

All modes.

## Speed and size impact

- Startup: runs only on library import or metadata refresh.
- Budgets touched: inspects at most the first 3 pages when extracting text identifiers.
- Installer size: 0 bytes added (uses `quick-xml`, `serde_json`, `ureq` already in workspace).

## Design

- Model: `excellent-features::library::metadata::{extract_from_pdf, extract_doi, extract_arxiv_id, extract_isbn, fetch_crossref_metadata, fetch_arxiv_metadata, fetch_openlibrary_metadata, OnlineConsent}`.
- Network: strictly gated behind `OnlineConsent::Allowed`.

## Acceptance tests

- [x] `extracts_doi_arxiv_and_isbn_with_validation`: Extracts DOI, arXiv ID, and check-digit-validated ISBN-10/ISBN-13.
- [x] `online_lookups_refuse_without_consent_and_parse_responses`: Refuses online requests when `OnlineConsent::Denied` and parses Crossref responses cleanly.
