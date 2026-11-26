# Ph0: 1/13 | K.1: Repository and workspace skeleton

**Tag:** K.1; **Priority:** P0; **Phase:** 0; **Needs:** none; **Code:** `Cargo.toml`, `deny.toml`, `rust-toolchain.toml`, `AGENTS.md`, `docs/adr/`
**Plan:** §27 entry `Ph0: 1/13 | K.1`, §5.4, §17.

## Problem

A multi-crate desktop/tablet Rust workspace with C bindings and strict licensing needs a single authoritative workspace layout, dependency policy, lint baseline, and ADR record before feature work begins.

## User story

As a contributor, I want one clean workspace with pinned toolchains, centralized dependencies, and `cargo deny` checks, so that builds are reproducible and compliant from day one.

## UI sketch

None.

## Mode

All modes (workspace infrastructure).

## Speed and size impact

Release profile configures `lto = "fat"`, `codegen-units = 1`, `strip = true`, and `panic = "unwind"` for NSH error boundaries.

## Design

- Flat `crates/*` workspace in root `Cargo.toml` with `[workspace.dependencies]` and `[workspace.lints.rust]` (`missing_docs = "warn"`, `unreachable_pub = "deny"`).
- `deny.toml` enforcing advisory, license, ban, and source checks.
- Architectural Decision Records `docs/adr/0001`–`0010` covering decisions D1–D10.

## Acceptance tests

- [x] `cargo build --workspace` and `cargo deny check`: pass on clean workspace.
