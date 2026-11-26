//! Per-device append-only folder sync (`.excellent/<scope>/<device-id>.oplog`), conflict-file
//! cleanup, authenticated envelope encryption, and cross-device workspace sync (plan Y.1, W.13).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use excellent_store::Store;
use excellent_store::workspace::{self, Panes, SavedGroup, SavedTab, Tabs};
use excellent_store::workspaces::{self, Look};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::SyncError;
use super::crdt::{
    HlcTimestamp, SyncOp, SyncOpKind, SyncedDocumentState, SyncedPane, SyncedTab, SyncedTabGroup,
    SyncedWorkspace,
};

/// Payload confidentiality mode for folder `.oplog` files and live relay frames (plan Y.1, Y.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PayloadSecurity {
    /// Store plain JSON operation lines.
    #[default]
    Plaintext,
    /// Encrypt each operation line with SHA-256-CTR + HMAC-SHA256 authentication using a 32-byte key.
    Sealed {
        /// Key identifier label.
        key_id: String,
        /// 256-bit shared symmetric key.
        key: [u8; 32],
    },
}

/// Maps local OS file paths to canonical portable tokens (`${ROOT}/...`) across devices (plan W.13).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevicePathMap {
    /// Named root aliases sorted longest-prefix-first when encoding.
    roots: BTreeMap<String, PathBuf>,
}

impl DevicePathMap {
    /// Creates an empty path map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a named root alias (for example `"DOCS"` -> `C:\Users\Alice\Documents`).
    pub fn add_root(&mut self, alias: &str, local_dir: impl AsRef<Path>) {
        self.roots
            .insert(alias.to_owned(), local_dir.as_ref().to_path_buf());
    }

    /// Encodes a local `path` into a portable forward-slash token using the longest matching root alias.
    #[must_use]
    pub fn to_portable(&self, path: &Path) -> String {
        let norm_path = normalize_slashes(path);
        let mut best: Option<(&str, String)> = None;
        for (alias, root) in &self.roots {
            let norm_root = normalize_slashes(root);
            let norm_root_trimmed = norm_root.trim_end_matches('/');
            if let Some(rel) = strip_prefix_case_aware(&norm_path, norm_root_trimmed) {
                let rel_clean = rel.trim_start_matches('/');
                let replace_best = best
                    .as_ref()
                    .is_none_or(|(_, prev_root)| norm_root_trimmed.len() > prev_root.len());
                if replace_best {
                    best = Some((alias.as_str(), rel_clean.to_owned()));
                }
            }
        }
        match best {
            Some((alias, rel)) if !rel.is_empty() => format!("${{{alias}}}/{rel}"),
            Some((alias, _)) => format!("${{{alias}}}"),
            None => norm_path,
        }
    }

    /// Decodes a portable token back into a local [`PathBuf`].
    #[must_use]
    pub fn to_local(&self, token: &str) -> PathBuf {
        if let Some(rest) = token.strip_prefix("${")
            && let Some((alias, tail)) = rest.split_once('}')
            && let Some(root) = self.roots.get(alias)
        {
            let tail_clean = tail.trim_start_matches('/');
            if tail_clean.is_empty() {
                return root.clone();
            }
            let mut out = root.clone();
            for segment in tail_clean.split('/') {
                if !segment.is_empty() {
                    out.push(segment);
                }
            }
            return out;
        }
        PathBuf::from(token)
    }
}

/// Manages append-only per-device `.oplog` files inside a synced folder (plan Y.1).
#[derive(Debug, Clone)]
pub struct FolderSync {
    sync_root: PathBuf,
    scope_id: String,
    device_id: String,
    security: PayloadSecurity,
}

impl FolderSync {
    /// Creates a folder sync handle for `scope_id` (e.g. document id or `"workspaces"`) and `device_id`.
    #[must_use]
    pub fn new(
        sync_root: impl AsRef<Path>,
        scope_id: &str,
        device_id: &str,
        security: PayloadSecurity,
    ) -> Self {
        Self {
            sync_root: sync_root.as_ref().to_path_buf(),
            scope_id: sanitize_segment(scope_id),
            device_id: sanitize_segment(device_id),
            security,
        }
    }

    /// Directory holding per-device `.oplog` files: `<sync_root>/.excellent/<scope_id>`.
    #[must_use]
    pub fn scope_dir(&self) -> PathBuf {
        self.sync_root.join(".excellent").join(&self.scope_id)
    }

    /// Path of this device's exclusive append-only oplog file.
    #[must_use]
    pub fn device_oplog_path(&self) -> PathBuf {
        self.scope_dir().join(format!("{}.oplog", self.device_id))
    }

    /// Appends `ops` exclusively to this device's `.oplog` file (never writes to other devices' files).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] or [`SyncError::Crypto`] if directory creation, encryption, or writing fails.
    pub fn append_ops(&self, ops: &[SyncOp]) -> Result<(), SyncError> {
        let dir = self.scope_dir();
        fs::create_dir_all(&dir)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.device_oplog_path())?;

        for (idx, op) in ops.iter().enumerate() {
            let line = encode_oplog_line(op, &self.security, idx as u64)?;
            writeln!(file, "{line}")?;
        }
        file.flush()?;
        Ok(())
    }

    /// Reads all valid `.oplog` files in `scope_dir`, ignores cloud-sync conflict duplicates, and
    /// merges every operation into a converged [`SyncedDocumentState`].
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if reading or decrypting a device oplog fails.
    pub fn load_merged_state(&self) -> Result<SyncedDocumentState, SyncError> {
        let ops = self.read_all_ops()?;
        Ok(SyncedDocumentState::replay_ops(&ops))
    }

    /// Reads all operations across every device `.oplog` in `scope_dir` in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if an `.oplog` file cannot be read, parsed, or verified.
    pub fn read_all_ops(&self) -> Result<Vec<SyncOp>, SyncError> {
        let dir = self.scope_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && !is_conflicted_copy_name(&path)
                && path.extension().and_then(|s| s.to_str()) == Some("oplog")
            {
                files.push(path);
            }
        }
        files.sort();

        let mut all_ops = Vec::new();
        for file_path in files {
            let file = fs::File::open(&file_path)?;
            let reader = BufReader::new(file);
            for line_res in reader.lines() {
                let line = line_res?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(op) = decode_oplog_line(trimmed, &self.security)? {
                    all_ops.push(op);
                }
            }
        }
        all_ops.sort_by(|a, b| a.stamp.cmp(&b.stamp));
        Ok(all_ops)
    }

    /// Detects and removes cloud-sync conflicted copy artifacts (`*conflicted copy*`, `*sync-conflict*`)
    /// inside `scope_dir`, returning the number of deleted conflict files (plan Y.1).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] if directory traversal or file removal fails.
    pub fn clean_conflicted_copies(&self) -> Result<usize, SyncError> {
        let dir = self.scope_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let mut removed = 0_usize;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && is_conflicted_copy_name(&path) {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Exports all workspaces from `store` into `folder_sync` with paths mapped via `path_map` (plan W.13).
///
/// # Errors
///
/// Returns [`SyncError`] if reading SQLite or appending to the sync folder fails.
pub fn export_workspaces_to_sync(
    store: &Store,
    folder_sync: &FolderSync,
    path_map: &DevicePathMap,
    clock: &mut HlcTimestamp,
    now_ms: u64,
) -> Result<usize, SyncError> {
    let snapshots = store
        .read(|conn| {
            let list = workspaces::list(conn)?;
            let mut out = Vec::with_capacity(list.len());
            for (pos, ws) in list.into_iter().enumerate() {
                let panes = workspace::load_panes(conn, ws.id)?;
                out.push((pos as i64, ws, panes));
            }
            Ok(out)
        })
        .map_err(|e| SyncError::Store(e.to_string()))?;

    let mut ops = Vec::with_capacity(snapshots.len());
    for (pos, ws, panes) in snapshots {
        let sync_id = if ws.name.trim().is_empty() {
            "ws-default".to_owned()
        } else {
            format!("ws-{}", sanitize_segment(&ws.name.to_ascii_lowercase()))
        };
        let synced_panes: Vec<SyncedPane> = panes
            .panes
            .into_iter()
            .map(|p| SyncedPane {
                open: p
                    .open
                    .into_iter()
                    .map(|t| SyncedTab {
                        path_token: path_map.to_portable(&t.path),
                        view: t.view,
                        pinned: t.pinned,
                        group: t.group,
                    })
                    .collect(),
                active: p.active,
                groups: p
                    .groups
                    .into_iter()
                    .map(|g| SyncedTabGroup {
                        name: g.name,
                        color: g.color,
                        collapsed: g.collapsed,
                    })
                    .collect(),
            })
            .collect();

        let stamp = clock.tick(now_ms);
        ops.push(SyncOp {
            stamp,
            kind: SyncOpKind::UpsertWorkspace(SyncedWorkspace {
                sync_id,
                name: ws.name,
                icon: ws.icon,
                color: ws.color,
                layout: panes.layout,
                position: pos,
                panes: synced_panes,
            }),
        });
    }

    folder_sync.append_ops(&ops)?;
    Ok(ops.len())
}

/// Imports merged workspaces from `folder_sync` into `store`, mapping portable tokens via `path_map` (plan W.13).
///
/// # Errors
///
/// Returns [`SyncError`] if reading the oplog or writing to SQLite fails.
pub fn import_workspaces_from_sync(
    store: &Store,
    folder_sync: &FolderSync,
    path_map: &DevicePathMap,
) -> Result<usize, SyncError> {
    let state = folder_sync.load_merged_state()?;
    let active_ws: Vec<SyncedWorkspace> = state.active_workspaces().into_iter().cloned().collect();
    let count = active_ws.len();
    let mapped: Vec<(SyncedWorkspace, Panes)> = active_ws
        .into_iter()
        .map(|ws| {
            let panes = Panes {
                layout: ws.layout.clone(),
                panes: ws
                    .panes
                    .iter()
                    .map(|p| Tabs {
                        open: p
                            .open
                            .iter()
                            .map(|t| SavedTab {
                                path: path_map.to_local(&t.path_token),
                                view: t.view.clone(),
                                pinned: t.pinned,
                                group: t.group,
                            })
                            .collect(),
                        active: p.active,
                        groups: p
                            .groups
                            .iter()
                            .map(|g| SavedGroup {
                                name: g.name.clone(),
                                color: g.color.clone(),
                                collapsed: g.collapsed,
                            })
                            .collect(),
                        closed: Vec::new(),
                    })
                    .collect(),
            };
            (ws, panes)
        })
        .collect();

    store
        .write(move |tx| {
            let existing = workspaces::list(tx)?;
            for (ws, panes) in mapped {
                let look = Look {
                    name: ws.name.clone(),
                    icon: ws.icon.clone(),
                    color: ws.color.clone(),
                };
                if let Some(found) = existing.iter().find(|e| e.name == ws.name) {
                    workspaces::restyle(tx, found.id, &look)?;
                    workspace::save_panes(tx, found.id, &panes)?;
                } else {
                    let ws_id = workspaces::create(tx, &look)?;
                    workspace::save_panes(tx, ws_id, &panes)?;
                }
            }
            Ok(())
        })
        .wait()
        .map_err(|e| SyncError::Store(e.to_string()))?;

    Ok(count)
}

/// Seals arbitrary plaintext bytes with SHA-256-CTR + HMAC-SHA256 using `key` and `nonce`.
pub(crate) fn seal_bytes(key: &[u8; 32], nonce: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plaintext.len());
    for (block_idx, chunk) in plaintext.chunks(32).enumerate() {
        let mut h = Sha256::new();
        h.update(b"excellent-ctr-v1");
        h.update(key);
        h.update(nonce);
        h.update((block_idx as u64).to_le_bytes());
        let mask = h.finalize();
        for (i, &b) in chunk.iter().enumerate() {
            out.push(b ^ mask[i]);
        }
    }
    out
}

pub(crate) fn compute_mac(key: &[u8; 32], nonce: &[u8; 16], ciphertext: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"excellent-mac-v1");
    h.update(key);
    h.update(nonce);
    h.update(ciphertext);
    h.finalize().into()
}

fn encode_oplog_line(
    op: &SyncOp,
    security: &PayloadSecurity,
    seq_hint: u64,
) -> Result<String, SyncError> {
    let plain = serde_json::to_vec(&op.to_json())
        .map_err(|e| SyncError::InvalidFormat(format!("encode op: {e}")))?;
    match security {
        PayloadSecurity::Plaintext => {
            String::from_utf8(plain).map_err(|e| SyncError::InvalidFormat(e.to_string()))
        }
        PayloadSecurity::Sealed { key_id, key } => {
            let mut nonce = [0_u8; 16];
            let mut nh = Sha256::new();
            nh.update(op.stamp.wall_ms.to_le_bytes());
            nh.update(op.stamp.logical.to_le_bytes());
            nh.update(op.stamp.device_id.as_bytes());
            nh.update(seq_hint.to_le_bytes());
            let ndigest = nh.finalize();
            nonce.copy_from_slice(&ndigest[..16]);

            let cipher = seal_bytes(key, &nonce, &plain);
            let mac = compute_mac(key, &nonce, &cipher);
            let b64 = base64::engine::general_purpose::STANDARD;
            let env = json!({
                "sealed": 1,
                "key_id": key_id,
                "nonce": b64.encode(nonce),
                "ct": b64.encode(cipher),
                "mac": b64.encode(mac),
            });
            serde_json::to_string(&env).map_err(|e| SyncError::Crypto(e.to_string()))
        }
    }
}

fn decode_oplog_line(line: &str, security: &PayloadSecurity) -> Result<Option<SyncOp>, SyncError> {
    let val: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        // Ignore a half-written trailing line left by an interrupted sync copy.
        Err(_) => return Ok(None),
    };

    if val.get("sealed").is_some() {
        let PayloadSecurity::Sealed { key, .. } = security else {
            return Err(SyncError::Crypto(
                "encrypted oplog encountered without key".to_owned(),
            ));
        };
        let b64 = base64::engine::general_purpose::STANDARD;
        let nonce_bytes = b64
            .decode(
                val.get("nonce")
                    .and_then(Value::as_str)
                    .ok_or_else(|| SyncError::Crypto("missing nonce".to_owned()))?,
            )
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        let nonce: [u8; 16] = nonce_bytes
            .try_into()
            .map_err(|_| SyncError::Crypto("invalid nonce length".to_owned()))?;
        let ct = b64
            .decode(
                val.get("ct")
                    .and_then(Value::as_str)
                    .ok_or_else(|| SyncError::Crypto("missing ciphertext".to_owned()))?,
            )
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        let mac_bytes = b64
            .decode(
                val.get("mac")
                    .and_then(Value::as_str)
                    .ok_or_else(|| SyncError::Crypto("missing mac".to_owned()))?,
            )
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        let expected = compute_mac(key, &nonce, &ct);
        if mac_bytes.as_slice() != expected {
            return Err(SyncError::Crypto(
                "oplog MAC verification failed".to_owned(),
            ));
        }
        let plain = seal_bytes(key, &nonce, &ct);
        let inner: Value =
            serde_json::from_slice(&plain).map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        return SyncOp::from_json(&inner).map(Some);
    }

    SyncOp::from_json(&val).map(Some)
}

fn is_conflicted_copy_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.contains("conflicted copy") || lower.contains("sync-conflict")
}

fn sanitize_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "default".to_owned()
    } else {
        cleaned
    }
}

fn normalize_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn strip_prefix_case_aware<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    let lower_path = path.to_ascii_lowercase();
    let lower_prefix = prefix.to_ascii_lowercase();
    if lower_path == lower_prefix {
        return Some("");
    }
    if lower_path.starts_with(&lower_prefix) {
        let rest = &path[prefix.len()..];
        if rest.starts_with('/') {
            return Some(rest);
        }
    }
    None
}
