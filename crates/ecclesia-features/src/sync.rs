//! CRDT folder sync, OS-portable settings, signed `.exbundle` sharing, live relay co-editing,
//! and cross-device workspace sync (plan Y.1–Y.4, W.13).

pub mod bundle;
pub mod crdt;
pub mod folder;
pub mod relay;
pub mod settings;

use std::fmt;

/// Errors returned by sync, bundle, settings, and relay operations.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum SyncError {
    /// Filesystem or socket I/O error.
    Io(std::io::Error),
    /// Malformed oplog, bundle, or relay message.
    InvalidFormat(String),
    /// Encryption, decryption, or MAC verification failure.
    Crypto(String),
    /// Ed25519 signature verification failure.
    Signature,
    /// SQLite store error during workspace sync.
    Store(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "sync i/o error: {err}"),
            Self::InvalidFormat(msg) => write!(f, "invalid sync format: {msg}"),
            Self::Crypto(msg) => write!(f, "sync crypto error: {msg}"),
            Self::Signature => f.write_str("sync signature verification failed"),
            Self::Store(msg) => write!(f, "sync store error: {msg}"),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidFormat(_) | Self::Crypto(_) | Self::Signature | Self::Store(_) => None,
        }
    }
}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ecclesia_store::Store;
    use ecclesia_store::workspace::{self, Panes, SavedTab, Tabs};
    use ecclesia_store::workspaces::{self, Look};
    use ed25519_dalek::SigningKey;
    use serde_json::json;

    use super::bundle::{
        BundleAnnotation, BundleNote, BundleVerification, ShareBundle, merge_bundle_into_state,
        pack_exbundle, unpack_exbundle,
    };
    use super::crdt::{
        CrdtOpId, HlcTimestamp, NoteTextCrdt, SyncOp, SyncOpKind, SyncedDocumentState,
        SyncedNoteMeta,
    };
    use super::folder::{DevicePathMap, FolderSync, PayloadSecurity};
    use super::relay::{CoEditPeer, FrameMask, InviteLink, LocalRelayHub, PresenceCursor, WsFrame};
    use super::settings::{
        TargetOs, materialize_for_os, merge_portable_settings, sanitize_local_settings,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ecclesia-sync-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn offline_devices_sync_and_converge_under_arbitrary_interleavings() {
        let dir = temp_dir("y1-interleave");
        let mut clock_a = HlcTimestamp::new("dev-a", 1000);
        let mut clock_b = HlcTimestamp::new("dev-b", 1000);

        let mut ops = Vec::new();
        // Concurrent annotation edits on same id -> HLC resolves deterministically.
        ops.push(SyncOp {
            stamp: clock_a.tick(1005),
            kind: SyncOpKind::UpsertAnnotation {
                doc_id: "doc-1".to_owned(),
                annot_id: "a-1".to_owned(),
                payload: json!({"color": "yellow", "note": "from A"}),
            },
        });
        ops.push(SyncOp {
            stamp: clock_b.tick(1010),
            kind: SyncOpKind::UpsertAnnotation {
                doc_id: "doc-1".to_owned(),
                annot_id: "a-1".to_owned(),
                payload: json!({"color": "blue", "note": "from B"}),
            },
        });
        // Concurrent RGA note text edits.
        let id1 = CrdtOpId {
            clock: 1,
            device_id: "dev-a".to_owned(),
        };
        let id2 = CrdtOpId {
            clock: 2,
            device_id: "dev-a".to_owned(),
        };
        let id3 = CrdtOpId {
            clock: 2,
            device_id: "dev-b".to_owned(),
        };
        ops.push(SyncOp {
            stamp: clock_a.tick(1015),
            kind: SyncOpKind::UpsertNoteMeta(SyncedNoteMeta {
                note_id: "n-1".to_owned(),
                title: "Shared Note".to_owned(),
                file_token: None,
                page: None,
                tags: vec!["sync".to_owned()],
            }),
        });
        ops.push(SyncOp {
            stamp: clock_a.tick(1016),
            kind: SyncOpKind::NoteTextInsert {
                note_id: "n-1".to_owned(),
                op_id: id1.clone(),
                parent: None,
                ch: 'A',
            },
        });
        ops.push(SyncOp {
            stamp: clock_a.tick(1017),
            kind: SyncOpKind::NoteTextInsert {
                note_id: "n-1".to_owned(),
                op_id: id2,
                parent: Some(id1.clone()),
                ch: 'B',
            },
        });
        ops.push(SyncOp {
            stamp: clock_b.tick(1018),
            kind: SyncOpKind::NoteTextInsert {
                note_id: "n-1".to_owned(),
                op_id: id3,
                parent: Some(id1),
                ch: 'C',
            },
        });

        let forward = SyncedDocumentState::replay_ops(&ops);
        let mut reversed_ops = ops.clone();
        reversed_ops.reverse();
        let backward = SyncedDocumentState::replay_ops(&reversed_ops);

        // Deterministic pseudo-random shuffle interleaving.
        let mut shuffled = ops;
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for i in (1..shuffled.len()).rev() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let j = (seed as usize) % (i + 1);
            shuffled.swap(i, j);
        }
        let random_order = SyncedDocumentState::replay_ops(&shuffled);

        assert_eq!(
            forward.active_annotations("doc-1"),
            backward.active_annotations("doc-1")
        );
        assert_eq!(
            forward.active_annotations("doc-1"),
            random_order.active_annotations("doc-1")
        );
        assert_eq!(
            forward.active_note("n-1").expect("note").1,
            backward.active_note("n-1").expect("note").1
        );
        assert_eq!(
            forward.active_note("n-1").expect("note").1,
            random_order.active_note("n-1").expect("note").1
        );

        // Also test encrypted per-device folder oplogs + conflict-file cleanup.
        let sec = PayloadSecurity::Sealed {
            key_id: "k1".to_owned(),
            key: [42_u8; 32],
        };
        let sync_a = FolderSync::new(&dir, "doc-1", "dev-a", sec.clone());
        let sync_b = FolderSync::new(&dir, "doc-1", "dev-b", sec);
        sync_a.append_ops(&shuffled[..3]).expect("append a");
        sync_b.append_ops(&shuffled[3..]).expect("append b");

        // Create a cloud provider conflicted copy and verify cleanup removes it.
        let conflict = sync_a
            .scope_dir()
            .join("dev-a (Alice's conflicted copy 2026-11-25).oplog");
        std::fs::write(&conflict, "corrupt").expect("write conflict");
        let merged = sync_a.load_merged_state().expect("merged");
        assert_eq!(
            merged.active_note("n-1").expect("note").1,
            forward.active_note("n-1").expect("note").1
        );
        assert_eq!(sync_a.clean_conflicted_copies().expect("clean"), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_sync_maps_paths_across_windows_and_macos() {
        let dir = temp_dir("w13");
        let sync_root = dir.join("sync");
        let store_win = Store::open(&dir.join("win.db")).expect("open win");
        let store_mac = Store::open(&dir.join("mac.db")).expect("open mac");

        let mut win_map = DevicePathMap::new();
        win_map.add_root("DOCS", r"C:\Users\Alice\Documents");
        let mut mac_map = DevicePathMap::new();
        mac_map.add_root("DOCS", "/Users/alice/Documents");

        // Create workspace on Windows device.
        store_win
            .write(|tx| {
                let ws_id = workspaces::create(
                    tx,
                    &Look {
                        name: "Thesis".to_owned(),
                        icon: Some("📚".to_owned()),
                        color: Some("purple".to_owned()),
                    },
                )?;
                workspace::save_panes(
                    tx,
                    ws_id,
                    &Panes {
                        layout: None,
                        panes: vec![Tabs {
                            open: vec![SavedTab {
                                path: PathBuf::from(
                                    r"C:\Users\Alice\Documents\Papers\attention.pdf",
                                ),
                                view: "{\"page\":3}".to_owned(),
                                pinned: true,
                                group: None,
                            }],
                            active: Some(0),
                            ..Tabs::default()
                        }],
                    },
                )
            })
            .wait()
            .expect("seed win ws");

        let fs_win = FolderSync::new(
            &sync_root,
            "workspaces",
            "win-laptop",
            PayloadSecurity::Plaintext,
        );
        let mut clock = HlcTimestamp::new("win-laptop", 2000);
        super::folder::export_workspaces_to_sync(&store_win, &fs_win, &win_map, &mut clock, 2001)
            .expect("export win ws");

        let fs_mac = FolderSync::new(
            &sync_root,
            "workspaces",
            "mac-studio",
            PayloadSecurity::Plaintext,
        );
        super::folder::import_workspaces_from_sync(&store_mac, &fs_mac, &mac_map)
            .expect("import mac ws");

        let mac_ws = store_mac.read(workspaces::list).expect("list mac ws");
        let thesis = mac_ws
            .iter()
            .find(|w| w.name == "Thesis")
            .expect("Thesis workspace synced to mac");
        let panes = store_mac
            .read(|c| workspace::load_panes(c, thesis.id))
            .expect("load mac panes");
        assert_eq!(
            panes.panes[0].open[0].path,
            PathBuf::from("/Users/alice/Documents/Papers/attention.pdf")
        );

        drop((store_win, store_mac));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn portable_settings_share_pen_from_windows_to_macos() {
        let win_raw = json!({
            "theme": "dawn",
            "window_hwnd": 99421,
            "win_mica": true,
            "keymap": { "find": "Ctrl+F" },
            "pens": [
                { "id": "pen-red", "name": "Red Marker", "color": "#dc2626", "width_pt": 2.5, "updated_at": 100 }
            ],
            "palettes": { "study": ["#2563eb", "#16a34a"] }
        });
        let mac_raw = json!({
            "theme": "dawn",
            "mac_vibrancy": true,
            "keymap": { "save": "Cmd+S" },
            "pens": []
        });

        let win_portable = sanitize_local_settings(&win_raw, TargetOs::Windows);
        let mut mac_portable = sanitize_local_settings(&mac_raw, TargetOs::MacOs);
        merge_portable_settings(&mut mac_portable, &win_portable);

        let materialized = materialize_for_os(&mac_portable, TargetOs::MacOs);
        assert!(materialized.get("window_hwnd").is_none());
        assert_eq!(materialized["keymap"]["find"], "Cmd+F");
        assert_eq!(materialized["mac_vibrancy"], true);
        assert_eq!(materialized["pens"][0]["name"], "Red Marker");
    }

    #[test]
    fn signed_exbundle_roundtrips_between_users_without_duplicates() {
        let key = SigningKey::from_bytes(&[9_u8; 32]);
        let bundle = ShareBundle {
            bundle_id: "bnd-1".to_owned(),
            title: "Paper Review".to_owned(),
            doc_id: "doc-abc".to_owned(),
            author_device: "alice".to_owned(),
            created_at: 1700000000,
            pdf_bytes: b"%PDF-1.7\n%%EOF\n".to_vec(),
            annotations: vec![BundleAnnotation {
                doc_id: "doc-abc".to_owned(),
                annot_id: "nm-1".to_owned(),
                stamp: HlcTimestamp::new("alice", 100),
                payload: json!({"kind": "highlight", "page": 0}),
            }],
            notes: vec![BundleNote {
                note_id: "note-1".to_owned(),
                title: "Summary".to_owned(),
                body: "Key claim.".to_owned(),
                file_token: Some("doc-abc".to_owned()),
                page: Some(0),
                tags: vec!["review".to_owned()],
                stamp: HlcTimestamp::new("alice", 101),
            }],
            boards: std::iter::once(("main.json".to_owned(), "{\"id\":\"b1\"}".to_owned()))
                .collect(),
            audio_clips: std::iter::once(("memo.wav".to_owned(), vec![1, 2, 3, 4])).collect(),
        };

        let archive = pack_exbundle(&bundle, Some(&key)).expect("pack");
        let unpacked = unpack_exbundle(
            &archive,
            BundleVerification::VerifySignature(&key.verifying_key()),
        )
        .expect("unpack verified");
        assert_eq!(unpacked.notes.len(), 1);

        let mut state = SyncedDocumentState::new();
        merge_bundle_into_state(&mut state, &unpacked);
        // Round-trip a second time: zero duplicates in annotations or note body.
        merge_bundle_into_state(&mut state, &unpacked);
        assert_eq!(state.active_annotations("doc-abc").len(), 1);
        assert_eq!(state.active_note("note-1").expect("note").1, "Key claim.");

        let wrong_key = SigningKey::from_bytes(&[1_u8; 32]).verifying_key();
        assert!(
            unpack_exbundle(&archive, BundleVerification::VerifySignature(&wrong_key)).is_err()
        );
    }

    #[test]
    fn live_websocket_relay_converges_two_peers_and_tracks_presence() {
        let invite = InviteLink {
            relay_url: "wss://relay.local/ws".to_owned(),
            room_id: "room-42".to_owned(),
            key: [77_u8; 32],
        };
        let parsed = InviteLink::parse(&invite.to_uri()).expect("parse invite");
        assert_eq!(parsed, invite);

        let mut hub = LocalRelayHub::new();
        hub.join_room("room-42", "alice");
        hub.join_room("room-42", "bob");

        let mut alice = CoEditPeer::from_invite("alice", &parsed);
        let mut bob = CoEditPeer::from_invite("bob", &parsed);

        let mut clock_a = HlcTimestamp::new("alice", 5000);
        let mut clock_b = HlcTimestamp::new("bob", 5000);

        let frame_a = alice
            .emit_ops(&[SyncOp {
                stamp: clock_a.tick(5001),
                kind: SyncOpKind::UpsertAnnotation {
                    doc_id: "doc-live".to_owned(),
                    annot_id: "hl-1".to_owned(),
                    payload: json!({"text": "Alice highlight"}),
                },
            }])
            .expect("emit a");
        // Verify RFC 6455 wire round-trip (masked -> unmasked decode).
        let wire = frame_a.encode(FrameMask::Masked([1, 2, 3, 4]));
        let decoded_ws = WsFrame::read_from(&mut std::io::Cursor::new(wire)).expect("ws decode");
        hub.forward_frame("room-42", "alice", decoded_ws);

        let cursor_frame = alice
            .emit_presence(PresenceCursor {
                device_id: "alice".to_owned(),
                display_name: "Alice".to_owned(),
                page: 2,
                x: 120.0,
                y: 240.0,
                color: "#2563eb".to_owned(),
                updated_ms: 5002,
            })
            .expect("emit cursor");
        hub.forward_frame("room-42", "alice", cursor_frame);

        let frame_b = bob
            .emit_ops(&[SyncOp {
                stamp: clock_b.tick(5003),
                kind: SyncOpKind::UpsertAnnotation {
                    doc_id: "doc-live".to_owned(),
                    annot_id: "hl-2".to_owned(),
                    payload: json!({"text": "Bob highlight"}),
                },
            }])
            .expect("emit b");
        hub.forward_frame("room-42", "bob", frame_b);

        for frame in hub.drain_frames("room-42", "bob") {
            bob.receive_ws_frame(&frame).expect("bob recv");
        }
        for frame in hub.drain_frames("room-42", "alice") {
            alice.receive_ws_frame(&frame).expect("alice recv");
        }

        assert_eq!(
            alice.state.active_annotations("doc-live"),
            bob.state.active_annotations("doc-live")
        );
        assert_eq!(alice.state.active_annotations("doc-live").len(), 2);
        assert_eq!(bob.cursors.get("alice").expect("cursor").page, 2);

        // Sanity check NoteTextCrdt merge directly too.
        let mut t1 = NoteTextCrdt::new();
        let mut c1 = 0_u64;
        t1.insert_str("a", &mut c1, None, "Hello");
        let mut t2 = t1.clone();
        t2.insert_str("b", &mut c1, None, "!");
        t1.merge(&t2);
        assert_eq!(t1.render().len(), 6);
    }
}
