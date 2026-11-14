# ADR 0008: Optional features as signed packs

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D8 (plan §3)

## Context

OCR models, fonts, dictionaries, handwriting and AI models are large, and not everyone needs them.

## Decision

Ship optional features as downloadable, signed, removable packs, never in the core installer.

## Why

The Small rule: the core installer stays at or under 15 MB.

## Consequences

Pack sources live in `packs/`. `excellent-packs` downloads, verifies (Ed25519), installs and removes them. Every feature that needs a pack works, or fails clearly, without it.
