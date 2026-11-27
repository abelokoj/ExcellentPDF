# Ph9: 12/13 | Y.4: Live co-editing through a relay

**Tag:** Y.4; **Priority:** P2; **Phase:** 9; **Needs:** Y.1; **Code:** `ecclesia-features::sync::relay`
**Plan:** §27 entry `#### Ph9: 12/13 | Y.4`.

## Problem

Folder sync has multi-second propagation delay and does not carry live peer cursors or real-time co-annotation.

## User story

As a collaborator, I want to share an invite link carrying a room encryption key so peers can co-edit annotations/notes and see live cursors through a blind self-hostable WebSocket relay.

## UI sketch

- Collaborate toolbar button ("Start Live Session" / "Join by Invite Link") displaying active peer cursors and connection latency.

## Mode

Annotate, Study, Research.

## Speed and size impact

- Startup: WebSocket relay client activates only during live sessions.
- Budgets touched: sub-millisecond frame encode/decode and CRDT merge (< 300 ms end-to-end).
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::sync::relay::{WsFrame, WsOpcode, FrameMask, InviteLink, PresenceCursor, EncryptedRelayPacket, LocalRelayHub, CoEditPeer}`.

## Acceptance tests

- [ ] `live_websocket_relay_converges_two_peers_and_tracks_presence`: Encrypts ops & cursor presence over RFC 6455 WebSocket frames through `LocalRelayHub` and verifies identical converged state on both peers.
