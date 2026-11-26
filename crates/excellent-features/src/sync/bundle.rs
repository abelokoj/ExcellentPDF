//! Signed `.exbundle` share bundle packager/unpacker and idempotent CRDT merge (plan Y.3).

use std::collections::BTreeMap;
use std::io::{Cursor, Read as _, Write as _};

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::SyncError;
use super::crdt::{
    CrdtOpId, HlcTimestamp, SyncOp, SyncOpKind, SyncedDocumentState, SyncedNoteMeta,
};

/// Bundle annotation entry stored inside an `.exbundle` archive.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleAnnotation {
    /// Document identifier.
    pub doc_id: String,
    /// Stable annotation `/NM` identifier.
    pub annot_id: String,
    /// Updated HLC stamp.
    pub stamp: HlcTimestamp,
    /// Annotation JSON payload.
    pub payload: Value,
}

/// Bundle Markdown note stored inside an `.exbundle` archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleNote {
    /// Stable note identifier.
    pub note_id: String,
    /// Note title.
    pub title: String,
    /// Markdown body text.
    pub body: String,
    /// Optional document token.
    pub file_token: Option<String>,
    /// Optional zero-based page index.
    pub page: Option<usize>,
    /// Tags.
    pub tags: Vec<String>,
    /// Updated HLC stamp.
    pub stamp: HlcTimestamp,
}

/// Complete `.exbundle` share package holding PDF, annotations, notes, boards, and audio clips (plan Y.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ShareBundle {
    /// Unique bundle identifier.
    pub bundle_id: String,
    /// Human-readable title.
    pub title: String,
    /// Document identifier.
    pub doc_id: String,
    /// Author/creator device id.
    pub author_device: String,
    /// Unix UTC seconds when created.
    pub created_at: i64,
    /// Raw PDF bytes.
    pub pdf_bytes: Vec<u8>,
    /// Annotations keyed stably by `(doc_id, annot_id)`.
    pub annotations: Vec<BundleAnnotation>,
    /// Notes with bodies.
    pub notes: Vec<BundleNote>,
    /// Research canvas boards (`board_name.json -> json_text`).
    pub boards: BTreeMap<String, String>,
    /// Audio recordings (`clip_name -> audio_bytes`).
    pub audio_clips: BTreeMap<String, Vec<u8>>,
}

/// Signature policy when unpacking an `.exbundle` archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BundleVerification<'a> {
    /// Verify Ed25519 signature strictly against the provided public key.
    VerifySignature(&'a VerifyingKey),
    /// Verify internal SHA-256 digests only (for unsigned or caller-trusted local bundles).
    DigestOnly,
}

/// Packages `bundle` into a `.exbundle` ZIP archive with SHA-256 file digests and an optional
/// Ed25519 signature over `manifest.json` (plan Y.3).
///
/// # Errors
///
/// Returns [`SyncError::InvalidFormat`] if ZIP writing or JSON encoding fails.
pub fn pack_exbundle(
    bundle: &ShareBundle,
    signing_key: Option<&SigningKey>,
) -> Result<Vec<u8>, SyncError> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert("document.pdf".to_owned(), bundle.pdf_bytes.clone());

    let annots_json = serde_json::to_vec_pretty(&Value::Array(
        bundle
            .annotations
            .iter()
            .map(|a| {
                json!({
                    "doc_id": a.doc_id,
                    "annot_id": a.annot_id,
                    "stamp": a.stamp.to_json(),
                    "payload": a.payload,
                })
            })
            .collect(),
    ))
    .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
    files.insert("annotations.json".to_owned(), annots_json);

    let notes_json = serde_json::to_vec_pretty(&Value::Array(
        bundle
            .notes
            .iter()
            .map(|n| {
                json!({
                    "note_id": n.note_id,
                    "title": n.title,
                    "body": n.body,
                    "file_token": n.file_token,
                    "page": n.page,
                    "tags": n.tags,
                    "stamp": n.stamp.to_json(),
                })
            })
            .collect(),
    ))
    .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
    files.insert("notes.json".to_owned(), notes_json);

    for (name, json_str) in &bundle.boards {
        let safe = safe_entry_name(name);
        files.insert(format!("boards/{safe}"), json_str.as_bytes().to_vec());
    }
    for (name, bytes) in &bundle.audio_clips {
        let safe = safe_entry_name(name);
        files.insert(format!("audio/{safe}"), bytes.clone());
    }

    let mut digests = BTreeMap::new();
    for (path, data) in &files {
        digests.insert(path.clone(), sha256_hex(data));
    }

    let manifest = json!({
        "schema": 1,
        "bundle_id": bundle.bundle_id,
        "title": bundle.title,
        "doc_id": bundle.doc_id,
        "author_device": bundle.author_device,
        "created_at": bundle.created_at,
        "digests": digests,
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;

    let mut cursor = Cursor::new(Vec::new());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = ZipWriter::new(&mut cursor);

    writer
        .start_file("manifest.json", options)
        .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
    writer.write_all(&manifest_bytes)?;

    if let Some(key) = signing_key {
        let sig = key.sign(&manifest_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
        writer
            .start_file("manifest.sig", options)
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        writer.write_all(sig_b64.as_bytes())?;
    }

    for (path, data) in files {
        writer
            .start_file(&path, options)
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        writer.write_all(&data)?;
    }

    writer
        .finish()
        .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
    Ok(cursor.into_inner())
}

/// Unpacks and validates an `.exbundle` archive, verifying digests and optional Ed25519 signature.
///
/// # Errors
///
/// Returns [`SyncError::Signature`] on signature mismatch, or [`SyncError::InvalidFormat`] on malformed archives.
pub fn unpack_exbundle(
    bytes: &[u8],
    verification: BundleVerification<'_>,
) -> Result<ShareBundle, SyncError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| SyncError::InvalidFormat(format!("invalid exbundle zip: {e}")))?;

    let manifest_bytes = read_zip_entry(&mut archive, "manifest.json")?;
    if let BundleVerification::VerifySignature(vk) = verification {
        let sig_bytes =
            read_zip_entry(&mut archive, "manifest.sig").map_err(|_| SyncError::Signature)?;
        let sig_str = std::str::from_utf8(&sig_bytes).map_err(|_| SyncError::Signature)?;
        let raw_sig = base64::engine::general_purpose::STANDARD
            .decode(sig_str.trim())
            .map_err(|_| SyncError::Signature)?;
        let sig = Signature::from_slice(&raw_sig).map_err(|_| SyncError::Signature)?;
        vk.verify_strict(&manifest_bytes, &sig)
            .map_err(|_| SyncError::Signature)?;
    }

    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| SyncError::InvalidFormat(format!("invalid manifest json: {e}")))?;
    let digests = manifest
        .get("digests")
        .and_then(Value::as_object)
        .ok_or_else(|| SyncError::InvalidFormat("manifest missing digests".to_owned()))?;

    for (path, expected_val) in digests {
        let expected_hex = expected_val
            .as_str()
            .ok_or_else(|| SyncError::InvalidFormat(format!("invalid digest for {path}")))?;
        let data = read_zip_entry(&mut archive, path)?;
        if sha256_hex(&data) != expected_hex {
            return Err(SyncError::InvalidFormat(format!(
                "digest mismatch for {path}"
            )));
        }
    }

    let pdf_bytes = read_zip_entry(&mut archive, "document.pdf")?;
    let annots_raw = read_zip_entry(&mut archive, "annotations.json")?;
    let notes_raw = read_zip_entry(&mut archive, "notes.json")?;

    let annotations = parse_annotations(&annots_raw)?;
    let notes = parse_notes(&notes_raw)?;

    let mut boards = BTreeMap::new();
    let mut audio_clips = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| SyncError::InvalidFormat(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Some(name) = rel_str.strip_prefix("boards/") {
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            boards.insert(name.to_owned(), text);
        } else if let Some(name) = rel_str.strip_prefix("audio/") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            audio_clips.insert(name.to_owned(), buf);
        }
    }

    Ok(ShareBundle {
        bundle_id: manifest
            .get("bundle_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        title: manifest
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        doc_id: manifest
            .get("doc_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        author_device: manifest
            .get("author_device")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        created_at: manifest
            .get("created_at")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        pdf_bytes,
        annotations,
        notes,
        boards,
        audio_clips,
    })
}

/// Merges a received [`ShareBundle`] into `state` by stable annotation and note IDs without
/// creating duplicates (plan Y.3).
pub fn merge_bundle_into_state(state: &mut SyncedDocumentState, bundle: &ShareBundle) {
    for annot in &bundle.annotations {
        state.apply_op(&SyncOp {
            stamp: annot.stamp.clone(),
            kind: SyncOpKind::UpsertAnnotation {
                doc_id: annot.doc_id.clone(),
                annot_id: annot.annot_id.clone(),
                payload: annot.payload.clone(),
            },
        });
    }
    for note in &bundle.notes {
        state.apply_op(&SyncOp {
            stamp: note.stamp.clone(),
            kind: SyncOpKind::UpsertNoteMeta(SyncedNoteMeta {
                note_id: note.note_id.clone(),
                title: note.title.clone(),
                file_token: note.file_token.clone(),
                page: note.page,
                tags: note.tags.clone(),
            }),
        });
        // Skip duplicate character insertion if the note already contains identical text.
        let already_matches = state
            .active_note(&note.note_id)
            .is_some_and(|(_, existing_body)| existing_body == note.body);
        if !already_matches {
            let mut parent: Option<CrdtOpId> = None;
            let base_clock = note.stamp.wall_ms.saturating_mul(1000);
            for (idx, ch) in note.body.chars().enumerate() {
                let op_id = CrdtOpId {
                    clock: base_clock.saturating_add(idx as u64 + 1),
                    device_id: note.stamp.device_id.clone(),
                };
                state.apply_op(&SyncOp {
                    stamp: note.stamp.clone(),
                    kind: SyncOpKind::NoteTextInsert {
                        note_id: note.note_id.clone(),
                        op_id: op_id.clone(),
                        parent,
                        ch,
                    },
                });
                parent = Some(op_id);
            }
        }
    }
}

fn read_zip_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, SyncError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| SyncError::InvalidFormat(format!("missing bundle entry {name}")))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn parse_annotations(raw: &[u8]) -> Result<Vec<BundleAnnotation>, SyncError> {
    let val: Value = serde_json::from_slice(raw)
        .map_err(|e| SyncError::InvalidFormat(format!("annotations.json: {e}")))?;
    let arr = val
        .as_array()
        .ok_or_else(|| SyncError::InvalidFormat("annotations.json not array".to_owned()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(BundleAnnotation {
            doc_id: item
                .get("doc_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            annot_id: item
                .get("annot_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            stamp: HlcTimestamp::from_json(
                item.get("stamp")
                    .ok_or_else(|| SyncError::InvalidFormat("annot missing stamp".to_owned()))?,
            )?,
            payload: item.get("payload").cloned().unwrap_or(Value::Null),
        });
    }
    Ok(out)
}

fn parse_notes(raw: &[u8]) -> Result<Vec<BundleNote>, SyncError> {
    let val: Value = serde_json::from_slice(raw)
        .map_err(|e| SyncError::InvalidFormat(format!("notes.json: {e}")))?;
    let arr = val
        .as_array()
        .ok_or_else(|| SyncError::InvalidFormat("notes.json not array".to_owned()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(BundleNote {
            note_id: item
                .get("note_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            title: item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            body: item
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            file_token: item
                .get("file_token")
                .and_then(Value::as_str)
                .map(str::to_owned),
            page: item
                .get("page")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok()),
            tags: item
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            stamp: HlcTimestamp::from_json(
                item.get("stamp")
                    .ok_or_else(|| SyncError::InvalidFormat("note missing stamp".to_owned()))?,
            )?,
        });
    }
    Ok(out)
}

fn safe_entry_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
