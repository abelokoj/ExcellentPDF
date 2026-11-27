# ADR 0007: App data in SQLite with FTS5

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D7 (plan §3)

## Context

The library, notes and search index must work offline and survive crashes.

## Decision

Keep app data (library, search index, notes) in SQLite through `rusqlite`, with FTS5 for full-text search.

## Why

Proven, offline, a single file, and fast full-text search.

## Consequences

Schema changes go through migrations in `ecclesia-store`.
