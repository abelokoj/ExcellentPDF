# Ph0: 8/13 | K.22: Out-of-process engine and sandbox

**Tag:** K.22; **Priority:** P0; **Phase:** 0 (spike), 1 (build); **Needs:** K.4; **Code:** `crates/excellent-core/src/sandbox.rs`, `crates/excellent-core/src/sandbox/{ipc,os,worker}.rs`
**Plan:** §27 entry `Ph0: 8/13 | K.22`, Appendix D.1.1.

## Problem

Running MuPDF directly inside the UI process means a hostile PDF crash or infinite hang could take down the entire application and lose unsaved ink across all open tabs.

## User story

As a reader with many documents open, I want a malformed PDF that crashes or hangs the document engine to affect only its own tab and recover cleanly, so that my other tabs and unsaved work stay safe.

## UI sketch

Transparent to the reader during normal operation; if a tab's sandboxed worker hits repeated unrecoverable crashes past `max_restarts_per_tab`, that tab transitions to a quarantined state with a clear diagnostic while every other tab continues normally.

## Mode

All modes.

## Speed and size impact

- Startup: nothing runs until the first document opens; the sandbox module adds zero external dependencies to `excellent-core`.
- IPC frames carry raw tile pixel blobs without JSON/base64 expansion and enforce `max_ipc_frame_bytes` before buffer allocation.

## Design

- **Framed IPC (`sandbox::ipc`):** length-prefixed `IpcFrame` (`IPC_MAGIC`, JSON header, raw binary payload) for `Ping`, `OpenBytes`, `RenderTile`, `ExtractText`, `ApplyReplayLog`, `CloseDocument`, and `Shutdown`.
- **OS Containment (`sandbox::os`):** `OsSandboxGuard` wraps an anonymous Windows Job Object (`CreateJobObjectW`, `SetInformationJobObject` for job/process committed memory caps, `active_process_limit = 1`, `KILL_ON_JOB_CLOSE`, and basic UI restrictions) plus a restricted low-privilege token (`CreateRestrictedToken` with `DISABLE_MAX_PRIVILEGE | LUA_TOKEN`) on `#[cfg(windows)]`, with portable fallbacks on POSIX and other targets.
- **Per-tab Worker Host (`sandbox::worker`):** `SandboxedWorkerPool` isolates each `TabId` in its own `WorkerTransport` (`SubprocessTransport` or portable `InProcessTransport`), enforces wall-clock watchdog timeouts and memory caps, and restarts a crashed tab worker with automatic document reopen and K.14 operation-log replay.

## Acceptance tests

- [x] `ipc_frame_and_messages_round_trip_and_reject_oversized`: framed IPC serialization/deserialization and pre-allocation length bounds.
- [x] `stdio_worker_loop_serves_corpus_pdf_and_exits_on_shutdown`: stdio worker loop serves `Ping`, `OpenBytes`, `ExtractText`, and `Shutdown`.
- [x] `worker_crash_isolates_crashing_tab_and_replays_operations`: a crashing tab restarts and replays logged operations while other tabs remain untouched.
- [x] `worker_watchdog_timeout_and_quarantine_and_memory_cap_enforced`: wall-clock watchdog timeout recovery, memory-cap rejection, and per-tab quarantine.
- [x] `windows_job_object_and_restricted_token_contain_child_process`: Windows Job Object and restricted token attach, query memory accounting, and terminate a child process cleanly.
