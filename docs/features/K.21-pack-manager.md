# Ph4: 1/44 | K.21: Pack manager

**Tag:** K.21; **Priority:** P1; **Phase:** 4 (dictionaries need it), finished in Phase 5; **Needs:** K.20;
**Code:** `crates/excellent-packs`
**Plan:** §2.1 rule 2, D8, and §27 entry `#### Ph4: 1/44 | K.21`.

## Problem

Dictionaries, fonts, voices, OCR and AI models are too large for a 15 MB installer, but have to be fetched and
trusted safely when a reader wants them.

## User story

As a reader, I want to add a dictionary when I need it, trust that it is the real one, and remove it to get the space
back, even on a machine that is offline if I bring the file on a stick.

## UI sketch

None yet: Phase 5 adds the packs page in Settings (what is installed, what is offered, download with progress and
cancel, remove, install from a file). Features ask the manager whether their pack is there.

## Mode

All.

## Speed and size impact

Nothing at startup. No new dependencies: `ed25519-dalek`, `sha2`, `ureq` and `zip` are the versions NSH already
builds, with the features it turns on.

## Design

- **Index:** signed JSON of what is offered (id, version, title, size, SHA-256, HTTPS URL, licence); nothing in it is
  read until its Ed25519 signature verifies, and plain HTTP addresses and malformed hashes are refused.
- **Pack file (`.expack`):** a 64-byte Ed25519 signature, then the zip it signs (`manifest.json` with id, version and
  title, then the files). A pack checks itself, so one installed from a disk is as trusted as one downloaded.
- **Download:** into a `.part` file beside the packs; a dropped connection, a cancel or a quit keeps what came, and the
  next try asks for the rest with an HTTP range request (a server that sends it all again starts the part over). A
  complete file must have the index's size and hash, then its own signature, before it is unpacked.
- **Install:** unpacked into a staging folder beside the packs (every name checked to stay inside, at most 8 GiB in
  all), then renamed into place, replacing an older version, so a pack is there whole or not at all.
- **Remove:** the pack's folder and everything in it; only a pack id is accepted.

## Acceptance tests

- [x] A tampered pack is rejected: `a_tampered_pack_is_rejected` (a changed byte in a file from disk fails the
  signature; a changed download fails the index's hash first), and the model's `a_tampered_pack_is_rejected` (a
  changed signature, archive or key).
- [x] Interrupted downloads resume: `an_interrupted_download_resumes_where_it_stopped` (dropped half way, the second
  request starts there, and the pack installs).
- [x] Removing a pack leaves no files: `removing_a_pack_leaves_no_files`.
- [x] Also: `an_archive_that_escapes_its_folder_is_refused`, `an_index_is_read_only_when_its_signature_verifies`,
  `plain_http_and_bad_hashes_are_refused`, `ids_are_safe_folder_names`.

## Open

- The key the app trusts is the maintainer's to make (DD27): no pack is published and the app trusts none yet.
- The Settings page, the index's address and the release tooling that signs packs come in Phase 5.
