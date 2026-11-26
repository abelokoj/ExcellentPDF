# Ph7: 30/32 | O.19: Permissions and encryption

**Tag:** O.19; **Priority:** P2; **Phase:** 7; **Needs:** K.15; **Code:** `excellent-core::crypto`
**Plan:** §27 entry `#### Ph7: 30/32 | O.19`.

## Problem

Protecting confidential PDFs requires standard password encryption (AES-256 `/R 6` and legacy RC4-128 `/R 3`), ISO PDF `/P` permission bit enforcement, and sanitization that strips hidden metadata, scripts, and annotations before sharing externally.

## User story

As a user securing sensitive PDFs, I want to inspect and enforce permission flags, encrypt/decrypt with AES-256 or RC4-128 user/owner passwords, and sanitize hidden metadata/JavaScript cleanly.

## UI sketch

- Security Properties panel displaying encryption algorithm, active permission grants, password protection modal (`Aes256` / `Rc4_128`), and one-click `Sanitize Document` wizard.

## Mode

File -> Security Properties & Sanitize.

## Speed and size impact

- Pure Rust AES-256, SHA-256, MD5, and RC4 implementation with zero extra crate dependencies.

## Design

- **Permission policy (`PdfPermissions`, `DocumentAction`, `enforce_permission`)**: Encodes/decodes ISO `/P` permission bitfields cleanly and guards viewer actions (`PrintLowRes`, `PrintHighRes`, `CopyContent`, `ModifyContents`, etc.).
- **PDF encryption & inspection (`EncryptionScheme::Rc4_128 | Aes256`, `encrypt_pdf`, `decrypt_pdf`, `inspect_encryption`)**: Implements `/V 2 /R 3` RC4-128 and `/V 5 /R 6` AES-256 key derivation and object encryption/decryption round-tripping cleanly with `excellent_mupdf`.
- **Sanitization (`sanitize_pdf`, `SanitizeOptions`)**: Strips `/Info`, XMP `/Metadata`, `/JavaScript`/`/JS`/`/OpenAction`/`/AA`, attachments, hidden OCGs, or annotations per policy.

## Acceptance tests

- [x] `permissions_encode_decode_and_enforce`: validates `/P` bitfield encoding, decoding, and action enforcement.
- [x] `aes256_and_rc4_encrypt_decrypt_round_trip`: encrypts and decrypts PDFs under both AES-256 (`/V 5 /R 6`) and RC4-128 (`/V 2 /R 3`) with user/owner authentication and MuPDF verification.
- [x] `sanitize_strips_metadata_and_scripts`: verifies removal of `/Info`, `/Metadata`, and `/OpenAction` scripts on sanitized outputs.
