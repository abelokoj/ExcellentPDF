# Ph7: 25/32 | O.14: Sign documents

**Tag:** O.14; **Priority:** P1; **Phase:** 7; **Needs:** O.11, K.19; **Code:** `ecclesia-core::signing` (`der.rs`, `cms.rs`, `pdf_sig.rs`), `ecclesia-features::signing::panel`
**Plan:** §27 entry `#### Ph7: 25/32 | O.14`.

## Problem

Users need to apply verifiable cryptographic digital signatures to PDFs with optional visible handwritten, typed, or image appearances, without confusing non-cryptographic visual stamps for real signatures.

## User story

As a signer, I want to sign a PDF with an X.509 certificate and private key—choosing an invisible signature or a typed/handwritten/image visual box—producing a standard `/ByteRange` CMS/PKCS#7 signature container.

## UI sketch

- **Sign document…** opens a dialog separating cryptographic certificate selection (required for digital signatures) from optional visual appearance (typed, drawn strokes, or image, plus reason/location).
- Visual-only marks without a key are clearly labeled as stamps/annotations, never digital signatures.

## Mode

Read and Annotate modes on PDFs.

## Speed and size impact

- Startup: zero startup cost.
- Installer size: pure Rust ASN.1 DER + CMS/PKCS#7 over `ed25519-dalek` and `sha2` already in `ecclesia-core`.

## Design

- **DER & X.509 (`ecclesia_core::signing::der`)**: ASN.1 DER encoder/decoder and X.509 v3 certificate parser/builder for Ed25519 keys.
- **CMS (`ecclesia_core::signing::cms`)**: RFC 5652 detached `ContentInfo` / `SignedData` (`ETSI.CAdES.detached`) with `contentType`, `messageDigest` (SHA-256), `signingTime`, and PAdES `signing-certificate-v2` (`ESSCertIDv2`) signed attributes.
- **PDF `/Sig` container (`ecclesia_core::signing::pdf_sig`)**: `sign_pdf_bytes` appends an incremental update containing the `/Sig` dictionary, fixed-width `/ByteRange` and `/Contents` hex container, `/FT /Sig` widget annotation, Form XObject visual appearance, and `/AcroForm /SigFlags 3`, then patches exact `/ByteRange` offsets and embeds the hex-encoded CMS DER payload.

## Acceptance tests

- [x] `signs_verifies_trust_and_detects_tampering`: signs a PDF with a visible handwritten appearance, opens it in MuPDF, and verifies the cryptographic ByteRange CMS signature.
