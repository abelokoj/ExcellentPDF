# Ph7: 3/32 | B.12: Other outline actions

**Tag:** B.12; **Priority:** P2; **Phase:** 7; **Needs:** B.1; **Code:** `excellent-features::outline_extra::actions`
**Plan:** §27 entry `#### Ph7: 3/32 | B.12`.

## Problem

PDF outline items can trigger actions beyond internal `/GoTo` destinations, including `/GoToR` (remote PDF page), `/Launch` (external file/application), `/Named` (`NextPage`, `PrevPage`, `FirstPage`, `LastPage`, `GoBack`), and `/URI` (web URLs), each needing distinct UI cues and security validation.

## User story

As a reader navigating complex document sets, I want the outline panel to parse, display, edit, and safely validate `/GoToR`, `/Launch`, `/Named`, and `/URI` bookmark actions.

## UI sketch

- Distinct action badges (`OutlineActionIcon`) beside external link, remote document, named command, and launch outline nodes.
- Confirmation prompt before activating `/Launch` targets, especially executable extensions (`is_executable_launch_target`).

## Mode

Outline sidebar across Read and Annotate modes.

## Speed and size impact

- Zero startup cost; parses outline `/A` and `/Dest` dictionaries on demand.

## Design

- **`OutlineAction`**: Models `InternalPage`, `RemoteGoto`, `Launch`, `Named` (`NamedAction`), and `Uri` (`UriAction`), with PDF action dictionary serialization (`to_pdf_action_dict`), link URI conversion (`to_link_uri`), and round-trip parsing (`parse`).
- **Path & executable safety (`resolve_remote_target`, `is_executable_launch_target`)**: Canonicalizes and guards `/GoToR` relative paths against directory traversal outside allowed roots and flags executable `/Launch` targets for explicit user confirmation.

## Acceptance tests

- [x] `outline_actions_round_trip_and_resolve_remote_files_safely`: verifies serialization/parsing round-trips across all action kinds, path-traversal rejection on remote targets, and executable launch detection.
