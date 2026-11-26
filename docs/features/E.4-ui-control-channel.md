# Ph5: 44/45 | E.4: UI control channel

**Tag:** E.4; **Priority:** P1; **Phase:** 5; **Needs:** E.2; **Code:** `excellent-core::tools::control`, `crates/excellent-cli/src/commands.rs` (`control`)
**Plan:** §27 entry `#### Ph5: 44/45 | E.4` and Appendix C.3.1.

## Problem

Automated UI tests and agent scripts need an opt-in local channel to drive the application—opening documents, jumping to pages, running registered app actions, inspecting state, and capturing screenshots—without exposing unauthenticated access to other local users.

## User story

As a developer or script author, I want an authenticated loopback control channel with capability-scoped tokens so tests and agents can drive the app safely.

## UI sketch

- Opt-in loopback server (`excellent-cli control serve`) generating a random 128-bit `ControlToken` written to a user-only token file (`--token-file`).
- CLI client (`excellent-cli control send|ping|state|open|goto|action`) reading the token file and sending NDJSON or length-prefixed commands.

## Mode

Core protocol (`excellent-core::tools::control`) and CLI host/client (`excellent-cli control`).

## Speed and size impact

- Opt-in only; zero overhead when not enabled. Uses `std::net` loopback TCP, Unix domain sockets (`#[cfg(unix)]`), or mockable `ControlTransport`.

## Design

- **Auth & capabilities**: `ControlToken` checks secrets in constant time (`Unauthorized` on mismatch) and verifies `ControlCapability` (`OpenDocument`, `Navigate`, `ExecuteAction`, `RunTool`, `InspectState`, `Screenshot`), returning `Forbidden` when a token lacks the command's capability.
- **Framing & transports**: `FrameCodec::Ndjson` and `FrameCodec::LengthPrefixed` over `LoopbackControlServer`/`LoopbackTransport`, `UnixSocketTransport` (Unix), and `MockControlTransport` for deterministic unit tests.
- **Hosts**: `MockAppControlHost` for lightweight UI state testing and `DocumentControlHost` wired to `Registry` and `ToolRegistry`.

## Acceptance tests

- [x] `mock_transport_enforces_tokens_and_capabilities`: tests wrong token (`Unauthorized`), read-only capability (`Forbidden`), and full open/jump/action/state sequence.
- [x] `loopback_server_round_trips_both_codecs_and_token_file`: tests NDJSON and length-prefixed framing plus token file write/read round-trips over loopback TCP.
- [x] `document_control_host_and_mcp_work_together`: drives `DocumentControlHost` over loopback TCP against `base14-outline-annot.pdf` (open, jump, PNG screenshot, unauthorized rejection).
