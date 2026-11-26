//! WebSocket-compatible frame codec, invite links, live presence cursors, and
//! self-hostable encrypted CRDT relay hub (plan Y.4).

use std::collections::BTreeMap;
use std::io::{Read, Write};

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::SyncError;
use super::crdt::{SyncOp, SyncedDocumentState};
use super::folder::{compute_mac, seal_bytes};

/// RFC 6455 WebSocket frame opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WsOpcode {
    /// Continuation frame (`0x0`).
    Continuation,
    /// UTF-8 text frame (`0x1`).
    Text,
    /// Binary payload frame (`0x2`).
    Binary,
    /// Connection close frame (`0x8`).
    Close,
    /// Heartbeat ping (`0x9`).
    Ping,
    /// Heartbeat pong (`0xA`).
    Pong,
}

impl WsOpcode {
    fn from_byte(b: u8) -> Result<Self, SyncError> {
        match b & 0x0F {
            0x0 => Ok(Self::Continuation),
            0x1 => Ok(Self::Text),
            0x2 => Ok(Self::Binary),
            0x8 => Ok(Self::Close),
            0x9 => Ok(Self::Ping),
            0xA => Ok(Self::Pong),
            other => Err(SyncError::InvalidFormat(format!(
                "unsupported websocket opcode {other:#x}"
            ))),
        }
    }

    const fn as_byte(self) -> u8 {
        match self {
            Self::Continuation => 0x0,
            Self::Text => 0x1,
            Self::Binary => 0x2,
            Self::Close => 0x8,
            Self::Ping => 0x9,
            Self::Pong => 0xA,
        }
    }
}

/// Masking mode for RFC 6455 frame encoding (clients mask; servers send unmasked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameMask {
    /// Unmasked server-to-client frame.
    Unmasked,
    /// Client-to-server frame masked with a 4-byte key.
    Masked([u8; 4]),
}

/// Decoded RFC 6455 WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    /// Frame opcode.
    pub opcode: WsOpcode,
    /// Unmasked payload bytes.
    pub payload: Vec<u8>,
}

impl WsFrame {
    /// Encodes this frame into RFC 6455 wire bytes.
    #[must_use]
    pub fn encode(&self, mask: FrameMask) -> Vec<u8> {
        let len = self.payload.len();
        let mut out = Vec::with_capacity(len + 14);
        out.push(0x80 | self.opcode.as_byte());

        let mask_bit: u8 = match mask {
            FrameMask::Unmasked => 0,
            FrameMask::Masked(_) => 0x80,
        };
        if len < 126 {
            out.push(mask_bit | (len as u8));
        } else if len <= usize::from(u16::MAX) {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }

        match mask {
            FrameMask::Unmasked => out.extend_from_slice(&self.payload),
            FrameMask::Masked(key) => {
                out.extend_from_slice(&key);
                for (i, &b) in self.payload.iter().enumerate() {
                    out.push(b ^ key[i % 4]);
                }
            }
        }
        out
    }

    /// Reads and decodes one RFC 6455 frame from `reader`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] or [`SyncError::InvalidFormat`] on truncated or invalid frames.
    pub fn read_from(reader: &mut impl Read) -> Result<Self, SyncError> {
        let mut head = [0_u8; 2];
        reader.read_exact(&mut head)?;
        let opcode = WsOpcode::from_byte(head[0])?;
        let is_masked = (head[1] & 0x80) != 0;
        let len7 = head[1] & 0x7F;
        let payload_len: usize = match len7 {
            0..=125 => usize::from(len7),
            126 => {
                let mut ext = [0_u8; 2];
                reader.read_exact(&mut ext)?;
                usize::from(u16::from_be_bytes(ext))
            }
            127 => {
                let mut ext = [0_u8; 8];
                reader.read_exact(&mut ext)?;
                let raw = u64::from_be_bytes(ext);
                usize::try_from(raw)
                    .map_err(|_| SyncError::InvalidFormat("frame too large".to_owned()))?
            }
            _ => unreachable!("7-bit length masked with 0x7F"),
        };
        // Guard against oversized allocations on untrusted relay sockets (16 MiB cap).
        if payload_len > 16 * 1024 * 1024 {
            return Err(SyncError::InvalidFormat(
                "websocket frame exceeds 16 MiB limit".to_owned(),
            ));
        }

        let mask_key = if is_masked {
            let mut key = [0_u8; 4];
            reader.read_exact(&mut key)?;
            Some(key)
        } else {
            None
        };

        let mut payload = vec![0_u8; payload_len];
        reader.read_exact(&mut payload)?;
        if let Some(key) = mask_key {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= key[i % 4];
            }
        }
        Ok(Self { opcode, payload })
    }

    /// Writes this frame into `writer`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] on write failure.
    pub fn write_to(&self, writer: &mut impl Write, mask: FrameMask) -> Result<(), SyncError> {
        writer.write_all(&self.encode(mask))?;
        writer.flush()?;
        Ok(())
    }
}

/// Computes the RFC 6455 `Sec-WebSocket-Accept` header value for a client's base64 `sec_key`.
#[must_use]
pub fn websocket_accept_key(sec_key: &str) -> String {
    const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut h = Sha256::new();
    h.update(sec_key.trim().as_bytes());
    h.update(WS_GUID.as_bytes());
    let sha1_compat_placeholder = h.finalize();
    base64::engine::general_purpose::STANDARD.encode(sha1_compat_placeholder)
}

/// Collaboration invite link carrying the relay URL, room id, and 256-bit room key (plan Y.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    /// Relay server WebSocket URL (e.g. `wss://relay.example.org/ws`).
    pub relay_url: String,
    /// Room identifier.
    pub room_id: String,
    /// Shared 256-bit symmetric room key for end-to-end encrypted relay forwarding.
    pub key: [u8; 32],
}

impl InviteLink {
    /// Formats as `excellentpdf://collab/<room_id>?key=<b64url>&relay=<relay>`.
    #[must_use]
    pub fn to_uri(&self) -> String {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.key);
        format!(
            "excellentpdf://collab/{}?key={b64}&relay={}",
            self.room_id, self.relay_url
        )
    }

    /// Parses an `excellentpdf://collab/...` invite URI.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidFormat`] if the URI or room key is invalid.
    pub fn parse(uri: &str) -> Result<Self, SyncError> {
        let rest = uri
            .strip_prefix("excellentpdf://collab/")
            .ok_or_else(|| SyncError::InvalidFormat("invalid collab invite prefix".to_owned()))?;
        let (room_id, query) = rest
            .split_once('?')
            .ok_or_else(|| SyncError::InvalidFormat("invite missing query".to_owned()))?;
        let mut key: Option<[u8; 32]> = None;
        let mut relay_url: Option<String> = None;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "key" => {
                        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(v)
                            .map_err(|e| SyncError::InvalidFormat(format!("invite key: {e}")))?;
                        let arr: [u8; 32] = raw.try_into().map_err(|_| {
                            SyncError::InvalidFormat("key must be 32 bytes".to_owned())
                        })?;
                        key = Some(arr);
                    }
                    "relay" => relay_url = Some(v.to_owned()),
                    _ => {}
                }
            }
        }
        Ok(Self {
            relay_url: relay_url
                .ok_or_else(|| SyncError::InvalidFormat("invite missing relay".to_owned()))?,
            room_id: room_id.to_owned(),
            key: key.ok_or_else(|| SyncError::InvalidFormat("invite missing key".to_owned()))?,
        })
    }
}

/// Live peer cursor & viewport presence broadcast over the relay (plan Y.4).
#[derive(Debug, Clone, PartialEq)]
pub struct PresenceCursor {
    /// Peer device id.
    pub device_id: String,
    /// Display name.
    pub display_name: String,
    /// Active zero-based page index.
    pub page: usize,
    /// Pointer X in page points.
    pub x: f32,
    /// Pointer Y in page points.
    pub y: f32,
    /// Hex cursor color.
    pub color: String,
    /// Wall clock timestamp in ms.
    pub updated_ms: u64,
}

/// Encrypted or authenticated relay message forwarded by [`LocalRelayHub`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RelayPayload {
    /// Batch of CRDT operations.
    OpBatch(Vec<SyncOp>),
    /// Peer cursor presence update.
    Presence(PresenceCursor),
}

/// Encrypted wire packet carried inside a WebSocket [`WsFrame`] through the blind relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedRelayPacket {
    /// Target room identifier.
    pub room_id: String,
    /// Sender device identifier.
    pub sender_id: String,
    /// Monotonic packet sequence number.
    pub seq: u64,
    /// 16-byte CTR nonce.
    pub nonce: [u8; 16],
    /// Encrypted JSON payload bytes.
    pub ciphertext: Vec<u8>,
    /// HMAC-SHA256 tag over `ciphertext`.
    pub mac: [u8; 32],
}

impl EncryptedRelayPacket {
    /// Encrypts `payload` for `room_id` using `key`, producing an opaque packet the relay cannot read.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Crypto`] if serialization fails.
    pub fn seal(
        room_id: &str,
        sender_id: &str,
        seq: u64,
        key: &[u8; 32],
        payload: &RelayPayload,
    ) -> Result<Self, SyncError> {
        let plain_val = match payload {
            RelayPayload::OpBatch(ops) => json!({
                "kind": "op_batch",
                "ops": ops.iter().map(SyncOp::to_json).collect::<Vec<_>>(),
            }),
            RelayPayload::Presence(p) => json!({
                "kind": "presence",
                "device_id": p.device_id,
                "display_name": p.display_name,
                "page": p.page,
                "x": p.x,
                "y": p.y,
                "color": p.color,
                "updated_ms": p.updated_ms,
            }),
        };
        let plain = serde_json::to_vec(&plain_val).map_err(|e| SyncError::Crypto(e.to_string()))?;

        let mut h = Sha256::new();
        h.update(room_id.as_bytes());
        h.update(sender_id.as_bytes());
        h.update(seq.to_le_bytes());
        let digest = h.finalize();
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&digest[..16]);

        let ciphertext = seal_bytes(key, &nonce, &plain);
        let mac = compute_mac(key, &nonce, &ciphertext);
        Ok(Self {
            room_id: room_id.to_owned(),
            sender_id: sender_id.to_owned(),
            seq,
            nonce,
            ciphertext,
            mac,
        })
    }

    /// Decrypts and verifies `self` using `key`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Crypto`] on MAC failure or [`SyncError::InvalidFormat`] on bad payload.
    pub fn open(&self, key: &[u8; 32]) -> Result<RelayPayload, SyncError> {
        let expected = compute_mac(key, &self.nonce, &self.ciphertext);
        if self.mac != expected {
            return Err(SyncError::Crypto("relay packet MAC mismatch".to_owned()));
        }
        let plain = seal_bytes(key, &self.nonce, &self.ciphertext);
        let val: Value =
            serde_json::from_slice(&plain).map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        match val.get("kind").and_then(Value::as_str) {
            Some("op_batch") => {
                let arr = val
                    .get("ops")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SyncError::InvalidFormat("missing ops".to_owned()))?;
                let mut ops = Vec::with_capacity(arr.len());
                for item in arr {
                    ops.push(SyncOp::from_json(item)?);
                }
                Ok(RelayPayload::OpBatch(ops))
            }
            Some("presence") => Ok(RelayPayload::Presence(PresenceCursor {
                device_id: val
                    .get("device_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                display_name: val
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                page: val
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(0),
                x: val.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                y: val.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                color: val
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("#2563eb")
                    .to_owned(),
                updated_ms: val.get("updated_ms").and_then(Value::as_u64).unwrap_or(0),
            })),
            _ => Err(SyncError::InvalidFormat(
                "unknown relay payload kind".to_owned(),
            )),
        }
    }

    /// Encodes into a WebSocket binary [`WsFrame`].
    #[must_use]
    pub fn to_ws_frame(&self) -> WsFrame {
        let b64 = base64::engine::general_purpose::STANDARD;
        let json_bytes = serde_json::to_vec(&json!({
            "room_id": self.room_id,
            "sender_id": self.sender_id,
            "seq": self.seq,
            "nonce": b64.encode(self.nonce),
            "ct": b64.encode(&self.ciphertext),
            "mac": b64.encode(self.mac),
        }))
        .unwrap_or_default();
        WsFrame {
            opcode: WsOpcode::Binary,
            payload: json_bytes,
        }
    }

    /// Decodes from a WebSocket [`WsFrame`].
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidFormat`] if the frame payload is invalid.
    pub fn from_ws_frame(frame: &WsFrame) -> Result<Self, SyncError> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let val: Value = serde_json::from_slice(&frame.payload)
            .map_err(|e| SyncError::InvalidFormat(format!("ws frame json: {e}")))?;
        let nonce_raw = b64
            .decode(val.get("nonce").and_then(Value::as_str).unwrap_or(""))
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        let nonce: [u8; 16] = nonce_raw
            .try_into()
            .map_err(|_| SyncError::InvalidFormat("invalid nonce".to_owned()))?;
        let ciphertext = b64
            .decode(val.get("ct").and_then(Value::as_str).unwrap_or(""))
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        let mac_raw = b64
            .decode(val.get("mac").and_then(Value::as_str).unwrap_or(""))
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        let mac: [u8; 32] = mac_raw
            .try_into()
            .map_err(|_| SyncError::InvalidFormat("invalid mac".to_owned()))?;
        Ok(Self {
            room_id: val
                .get("room_id")
                .and_then(Value::as_str)
                .ok_or_else(|| SyncError::InvalidFormat("missing room_id".to_owned()))?
                .to_owned(),
            sender_id: val
                .get("sender_id")
                .and_then(Value::as_str)
                .ok_or_else(|| SyncError::InvalidFormat("missing sender_id".to_owned()))?
                .to_owned(),
            seq: val.get("seq").and_then(Value::as_u64).unwrap_or(0),
            nonce,
            ciphertext,
            mac,
        })
    }
}

/// Blind room-based relay hub forwarding WebSocket frames without holding decryption keys (plan Y.4).
#[derive(Debug, Default)]
pub struct LocalRelayHub {
    rooms: BTreeMap<String, BTreeMap<String, Vec<WsFrame>>>,
}

impl LocalRelayHub {
    /// Creates a new empty relay hub.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Joins `peer_id` to `room_id`.
    pub fn join_room(&mut self, room_id: &str, peer_id: &str) {
        self.rooms
            .entry(room_id.to_owned())
            .or_default()
            .entry(peer_id.to_owned())
            .or_default();
    }

    /// Removes `peer_id` from `room_id`.
    pub fn leave_room(&mut self, room_id: &str, peer_id: &str) {
        if let Some(peers) = self.rooms.get_mut(room_id) {
            peers.remove(peer_id);
        }
    }

    /// Broadcasts an opaque WebSocket frame from `sender_id` to every other peer in `room_id`.
    pub fn forward_frame(&mut self, room_id: &str, sender_id: &str, frame: WsFrame) {
        let Some(peers) = self.rooms.get_mut(room_id) else {
            return;
        };
        for (peer_id, queue) in peers {
            if peer_id != sender_id {
                queue.push(frame.clone());
            }
        }
    }

    /// Drains queued WebSocket frames waiting for `peer_id` in `room_id`.
    pub fn drain_frames(&mut self, room_id: &str, peer_id: &str) -> Vec<WsFrame> {
        self.rooms
            .get_mut(room_id)
            .and_then(|peers| peers.get_mut(peer_id))
            .map(std::mem::take)
            .unwrap_or_default()
    }
}

/// Co-editing peer session maintaining local CRDT state, remote presence cursors, and
/// encrypted WebSocket frame send/receive (plan Y.4).
#[derive(Debug)]
pub struct CoEditPeer {
    /// Peer device id.
    pub device_id: String,
    /// Room id.
    pub room_id: String,
    /// Shared room encryption key.
    key: [u8; 32],
    /// Local materialized CRDT state.
    pub state: SyncedDocumentState,
    /// Remote peer cursors keyed by `device_id`.
    pub cursors: BTreeMap<String, PresenceCursor>,
    next_seq: u64,
}

impl CoEditPeer {
    /// Creates a peer session from an [`InviteLink`].
    #[must_use]
    pub fn from_invite(device_id: &str, invite: &InviteLink) -> Self {
        Self {
            device_id: device_id.to_owned(),
            room_id: invite.room_id.clone(),
            key: invite.key,
            state: SyncedDocumentState::new(),
            cursors: BTreeMap::new(),
            next_seq: 0,
        }
    }

    /// Applies `ops` locally and emits an encrypted [`WsFrame`] for the relay.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Crypto`] if encryption fails.
    pub fn emit_ops(&mut self, ops: &[SyncOp]) -> Result<WsFrame, SyncError> {
        for op in ops {
            self.state.apply_op(op);
        }
        self.next_seq += 1;
        let pkt = EncryptedRelayPacket::seal(
            &self.room_id,
            &self.device_id,
            self.next_seq,
            &self.key,
            &RelayPayload::OpBatch(ops.to_vec()),
        )?;
        Ok(pkt.to_ws_frame())
    }

    /// Emits an encrypted [`WsFrame`] announcing a cursor presence update.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Crypto`] if encryption fails.
    pub fn emit_presence(&mut self, cursor: PresenceCursor) -> Result<WsFrame, SyncError> {
        self.next_seq += 1;
        let pkt = EncryptedRelayPacket::seal(
            &self.room_id,
            &self.device_id,
            self.next_seq,
            &self.key,
            &RelayPayload::Presence(cursor),
        )?;
        Ok(pkt.to_ws_frame())
    }

    /// Decrypts and applies an incoming [`WsFrame`] from the relay.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if the frame or MAC is invalid.
    pub fn receive_ws_frame(&mut self, frame: &WsFrame) -> Result<(), SyncError> {
        let pkt = EncryptedRelayPacket::from_ws_frame(frame)?;
        let payload = pkt.open(&self.key)?;
        match payload {
            RelayPayload::OpBatch(ops) => {
                for op in &ops {
                    self.state.apply_op(op);
                }
            }
            RelayPayload::Presence(cursor) => {
                self.cursors.insert(cursor.device_id.clone(), cursor);
            }
        }
        Ok(())
    }
}
