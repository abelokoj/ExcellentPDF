# ADR 0001: Rust, edition 2024, stable toolchain

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D1 (plan §3)

## Context

The app is a long-lived document editor on five platforms. A reader that opens untrusted files turns memory bugs into security bugs.

## Decision

Write EcclesiaPDF in Rust, edition 2024, on the stable toolchain pinned in `rust-toolchain.toml`.

## Why

Memory safety, one language for core and UI, and NSH is Rust.

## Consequences

Toolchain updates are deliberate, one pull request each. Unsafe code stays in the FFI and platform layers, every block with a `// SAFETY:` comment.
