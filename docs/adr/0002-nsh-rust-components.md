# ADR 0002: UI in NSH v0.5.0 with Rust components

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D2 (plan §3)

## Context

We need native UI on Windows, macOS, Linux, iPad and Android tablets from one codebase, with accessibility built in.

## Decision

Build the UI with NSH v0.5.0 using Rust components, not TSX. Pin `nsh-core` and `nsh-base` to `=0.5.0` and update NSH deliberately, in a pull request of its own.

## Why

Native views on all five platforms, unstyled accessible base components, and Rust apps need no QuickJS.

## Consequences

NSH's API is not frozen, so an update may break us; the exact pin contains that. NSH has no document view and no pen pressure, which ADR 0005 handles.
