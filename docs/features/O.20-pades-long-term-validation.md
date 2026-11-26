# Ph7: 31/32 | O.20: PAdES long-term validation

**Tag:** O.20; **Priority:** P2; **Phase:** 7; **Needs:** O.15; **Code:** `excellent-core::signing::ltv`, `excellent-features::signing::panel`
**Plan:** §27 entry `#### Ph7: 31/32 | O.20`.

## Problem

Signatures must remain verifiable years after the signing certificate expires or is revoked; without embedded timestamps, certificate chains, and revocation responses, long-term verification fails.

## User story

As a compliance or legal user, I want to add RFC 3161 timestamps, embed a Document Security Store (`/DSS`) with certificates/OCSP/CRL/VRI data, and seal the document with a `/DocTimeStamp` so the PDF achieves PAdES B-T, B-LT, or B-LTA conformance.

## UI sketch

- Signature panel displays the achieved PAdES level (`B`, `T`, `LT`, `LTA`) and any missing LTV artifacts, with an **Add LTV / Timestamp** action (network TSA requests require explicit user consent).

## Mode

Signature panel and signing workflow on PDFs.

## Speed and size impact

- Startup: zero startup cost.
- Offline safety: remote RFC 3161 TSA queries are gated behind `TsaConsent::Allowed`.

## Design

- **RFC 3161 TSA (`excellent_core::signing::ltv`)**: `TimeStampRequest::encode_sha256`, `LocalTimestampAuthority` (offline Ed25519 TSA), consent-gated `fetch_tsa_response` (`ureq`), and `parse_and_verify_timestamp_response` / `parse_and_verify_timestamp_token`.
- **Document Security Store**: `DocumentSecurityStore` (`/Certs`, `/OCSPs`, `/CRLs`, `/VRI` keyed by uppercase hex SHA-1 of `/Contents`), `embed_dss` (incremental catalog `/DSS` writer), and `extract_dss`.
- **Document Timestamp & Conformance**: `append_document_timestamp` (`/Type /DocTimeStamp`, `/SubFilter /ETSI.RFC3161`) and `check_pades_conformance` reporting `PadesLevel::{None, B, T, LT, LTA}`.

## Acceptance tests

- [x] `pades_ltv_upgrades_from_bb_to_bt_blt_and_blta`: progresses a signed PDF step by step through `PadesLevel::B` -> `PadesLevel::T` -> `PadesLevel::LT` (verifying `/DSS` `/Certs`, `/OCSPs`, `/CRLs`, `/VRI` round-trip) -> `PadesLevel::LTA`, and verifies `TsaConsent::Denied` refuses network access.
