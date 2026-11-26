# Ph5: 43/45 | E.3: MCP server

**Tag:** E.3; **Priority:** P1; **Phase:** 5; **Needs:** E.2; **Code:** `excellent-core::tools::mcp`, `crates/excellent-cli/src/mcp.rs`
**Plan:** §27 entry `#### Ph5: 43/45 | E.3` and Appendix C.3.1.

## Problem

External AI agents and IDE assistants speak Model Context Protocol (MCP) over JSON-RPC 2.0 stdio, and require strict filesystem confinement so untrusted prompts cannot access files outside a chosen workspace folder.

## User story

As a user connecting an external agent, I want `excellent-cli mcp --root <folder>` to expose the tool table over stdio while refusing any path outside `--root`.

## UI sketch

- Opt-in stdio server started with `excellent-cli mcp --root <dir>`.
- Never writes non-JSON-RPC logs to stdout.

## Mode

Headless stdio server (`excellent-cli mcp`).

## Speed and size impact

- Reuses `ToolRegistry` directly; no HTTP server or extra dependencies.

## Design

- **`McpServer`**: Line-delimited JSON-RPC 2.0 handler supporting `initialize`, `notifications/initialized` (no reply emitted for notifications), `ping`, `tools/list`, and `tools/call`, returning standard JSON-RPC error codes (`-32700`, `-32600`, `-32601`, `-32602`).
- **Confinement**: `ToolContext::with_root` canonicalizes `--root` and checks every input and output path (including not-yet-created output paths via deepest existing ancestor canonicalization), returning `ToolError::PathDenied` (`isError: true` in `tools/call`) if a path escapes `--root`.

## Acceptance tests

- [x] `mcp_round_trip_enforces_root_confinement`: verifies `initialize`, notification silence, `ping`, `tools/list`, `tools/call` inside root, `tools/call` traversal refusal (`../Cargo.toml`), and `-32601` for unknown methods.
- [x] `binary_process_mcp_and_exit_codes_work`: spawns the real `excellent-cli mcp --root` subprocess and tests JSON-RPC round-trip and root confinement over OS pipes.
