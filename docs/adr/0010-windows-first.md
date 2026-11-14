# ADR 0010: Windows first

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D10 (plan §3)

## Context

Five platforms cannot all be built at once.

## Decision

Build for Windows first, then macOS and Linux, then iPad and Android tablets, and the web last.

## Why

Users and the existing SumatraPDF Enhanced build are on Windows, and tablets need desktop parity first.

## Consequences

Phases 0 and 1 target Windows (MSVC). The other platform crates stay empty until their phase.
