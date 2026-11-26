# Ph5: 45/45 | U.6: Neural offline voices

**Tag:** U.6; **Priority:** P1; **Phase:** 5; **Needs:** U.5, K.21; **Code:** `excellent-features::neural_voices`
**Plan:** §27 entry `#### Ph5: 45/45 | U.6: Neural offline voices`.

## Problem

Built-in OS voices on some platforms sound robotic or are unavailable offline; high-quality neural voices should be optional downloadable packs that stay off the core installer.

## User story

As a reader using Read Aloud, I want to install license-verified Piper-style offline neural voice packs and listen with accurate word highlighting without an internet connection.

## UI sketch

- Settings > Read Aloud > **Neural Voice Packs…** lists available/installed voices (`VoicePackCatalog`), license badges, quality tiers (`XLow`..`High`), Install, and Remove.
- Selected voice plugs directly into `read_aloud::ReadAloudSession`.

## Mode

All modes (Read Aloud `U.5`).

## Speed and size impact

- Startup: zero cost; packs under `packs/voices/` load only when Read Aloud selects a neural voice.
- Security & licensing: `VoicePackCatalog::install` enforces approved SPDX licenses (`MIT`, `CC0-1.0`, `Apache-2.0`, `BSD-*`, `CC-BY-4.0`) and SHA-256 verification.

## Design

- **Model (`excellent-features::neural_voices`):**
  - `VoicePackCatalog` + `VoicePackManifest` + `check_voice_license`.
  - `PiperVoiceSynthesizer<B: NeuralAcousticBackend>` implements `excellent_platform::speech::SpeechSynth`, emitting `SpeechEvent::WordBoundary` events synchronized to synthesized PCM timings.

## Acceptance tests

- [x] `neural_voices::tests::pack_voice_installs_verifies_license_and_speaks_with_word_highlighting_offline`: verifies license gating, SHA-256 verification, offline synthesis, and K.13 `SpokenPage::word_quads` word highlighting on a real MuPDF-parsed PDF.
