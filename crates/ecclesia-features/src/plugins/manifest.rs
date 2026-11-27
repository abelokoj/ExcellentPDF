//! Signed plugin manifests, capability/hook declarations, and `.explugin` ZIP packaging (plan E.5).

use std::collections::BTreeSet;
use std::io::{Cursor, Read as _, Write as _};

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::PluginError;

/// Least-privilege capabilities a plugin may request and a user may grant (plan E.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PluginCapability {
    /// Read document metadata, pages, and extracted text.
    ReadDocs,
    /// Create, modify, or delete document annotations.
    WriteAnnots,
    /// Make outbound network requests.
    Network,
    /// Render or control an extension UI panel.
    UiPanel,
}

impl PluginCapability {
    /// Canonical string name used in `manifest.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadDocs => "read_docs",
            Self::WriteAnnots => "write_annots",
            Self::Network => "network",
            Self::UiPanel => "ui_panel",
        }
    }

    /// Parses a capability string from `manifest.json`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] for unknown capability names.
    pub fn parse(raw: &str) -> Result<Self, PluginError> {
        match raw.trim() {
            "read_docs" | "ReadDocs" => Ok(Self::ReadDocs),
            "write_annots" | "WriteAnnots" => Ok(Self::WriteAnnots),
            "network" | "Network" => Ok(Self::Network),
            "ui_panel" | "UiPanel" => Ok(Self::UiPanel),
            other => Err(PluginError::InvalidManifest(format!(
                "unknown plugin capability {other}"
            ))),
        }
    }
}

/// Deterministic lifecycle hooks a plugin can subscribe to (plan E.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PluginHook {
    /// Fired after a document opens.
    OnDocumentOpen,
    /// Fired before or after rendering a page.
    OnPageRender,
    /// Fired when an annotation is created or edited.
    OnAnnotCreate,
    /// Fired during document or note export.
    OnExport,
    /// Fired when a registered plugin command is invoked.
    OnCommand,
}

impl PluginHook {
    /// Canonical hook identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnDocumentOpen => "on_document_open",
            Self::OnPageRender => "on_page_render",
            Self::OnAnnotCreate => "on_annot_create",
            Self::OnExport => "on_export",
            Self::OnCommand => "on_command",
        }
    }

    /// Parses a hook name from `manifest.json`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] for unknown hook names.
    pub fn parse(raw: &str) -> Result<Self, PluginError> {
        match raw.trim() {
            "on_document_open" | "OnDocumentOpen" => Ok(Self::OnDocumentOpen),
            "on_page_render" | "OnPageRender" => Ok(Self::OnPageRender),
            "on_annot_create" | "OnAnnotCreate" => Ok(Self::OnAnnotCreate),
            "on_export" | "OnExport" => Ok(Self::OnExport),
            "on_command" | "OnCommand" => Ok(Self::OnCommand),
            other => Err(PluginError::InvalidManifest(format!(
                "unknown plugin hook {other}"
            ))),
        }
    }
}

/// Plugin metadata and requested capabilities parsed from `manifest.json` (plan E.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Safe folder-friendly identifier (`1..=64` lowercase ASCII/digits/`-`/`.`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Entrypoint executable or script name for stdio JSON-RPC runner.
    pub entry_command: String,
    /// Capabilities requested by the plugin.
    pub requested_capabilities: BTreeSet<PluginCapability>,
    /// Hooks this plugin subscribes to.
    pub hooks: BTreeSet<PluginHook>,
    /// Deterministic hook ordering priority (lower runs first).
    pub priority: i32,
    /// Expected SHA-256 hex digest of `payload.bin`.
    pub payload_sha256: String,
}

impl PluginManifest {
    /// Validates whether `id` is a safe plugin folder/package name.
    #[must_use]
    pub fn is_valid_id(id: &str) -> bool {
        (1..=64).contains(&id.len())
            && !id.starts_with('.')
            && !id.contains("..")
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
    }

    /// Serializes to canonical JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] if serialization fails.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, PluginError> {
        let caps: Vec<&str> = self
            .requested_capabilities
            .iter()
            .map(|c| c.as_str())
            .collect();
        let hooks: Vec<&str> = self.hooks.iter().map(|h| h.as_str()).collect();
        let val = json!({
            "schema": 1,
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "entry_command": self.entry_command,
            "requested_capabilities": caps,
            "hooks": hooks,
            "priority": self.priority,
            "payload_sha256": self.payload_sha256,
        });
        serde_json::to_vec_pretty(&val).map_err(|e| PluginError::InvalidManifest(e.to_string()))
    }

    /// Parses and validates `bytes` as a [`PluginManifest`].
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] if required fields are missing or invalid.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, PluginError> {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| PluginError::InvalidManifest(format!("invalid manifest json: {e}")))?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError::InvalidManifest("manifest missing id".to_owned()))?
            .to_owned();
        if !Self::is_valid_id(&id) {
            return Err(PluginError::InvalidManifest(format!(
                "unsafe plugin id {id:?}"
            )));
        }
        let version = v
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError::InvalidManifest("manifest missing version".to_owned()))?
            .to_owned();
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_owned();
        let entry_command = v
            .get("entry_command")
            .and_then(Value::as_str)
            .unwrap_or("plugin-host")
            .to_owned();
        let payload_sha256 = v
            .get("payload_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                PluginError::InvalidManifest("manifest missing payload_sha256".to_owned())
            })?
            .to_owned();

        let mut requested_capabilities = BTreeSet::new();
        if let Some(arr) = v.get("requested_capabilities").and_then(Value::as_array) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    requested_capabilities.insert(PluginCapability::parse(s)?);
                }
            }
        }
        let mut hooks = BTreeSet::new();
        if let Some(arr) = v.get("hooks").and_then(Value::as_array) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    hooks.insert(PluginHook::parse(s)?);
                }
            }
        }
        let priority = v
            .get("priority")
            .and_then(Value::as_i64)
            .and_then(|p| i32::try_from(p).ok())
            .unwrap_or(0);

        Ok(Self {
            id,
            name,
            version,
            entry_command,
            requested_capabilities,
            hooks,
            priority,
            payload_sha256,
        })
    }
}

/// Verified unpacked `.explugin` package containing manifest and payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPluginPackage {
    /// Validated manifest.
    pub manifest: PluginManifest,
    /// Verified payload bytes matching `manifest.payload_sha256`.
    pub payload_bytes: Vec<u8>,
}

/// Computes lowercase hex SHA-256 of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Packs and signs a `.explugin` ZIP archive using Ed25519 `signing_key` (plan E.5).
///
/// # Errors
///
/// Returns [`PluginError::InvalidManifest`] if the manifest id is unsafe or ZIP encoding fails.
pub fn pack_signed_plugin(
    manifest: &PluginManifest,
    payload_bytes: &[u8],
    signing_key: &SigningKey,
) -> Result<Vec<u8>, PluginError> {
    if !PluginManifest::is_valid_id(&manifest.id) {
        return Err(PluginError::InvalidManifest(format!(
            "unsafe plugin id {:?}",
            manifest.id
        )));
    }
    let mut normalized = manifest.clone();
    normalized.payload_sha256 = sha256_hex(payload_bytes);
    let manifest_bytes = normalized.to_json_bytes()?;
    let sig = signing_key.sign(&manifest_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let mut cursor = Cursor::new(Vec::new());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = ZipWriter::new(&mut cursor);

    writer
        .start_file("manifest.json", options)
        .map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
    writer.write_all(&manifest_bytes)?;

    writer
        .start_file("manifest.sig", options)
        .map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
    writer.write_all(sig_b64.as_bytes())?;

    writer
        .start_file("payload.bin", options)
        .map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
    writer.write_all(payload_bytes)?;

    writer
        .finish()
        .map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
    Ok(cursor.into_inner())
}

/// Verifies an Ed25519-signed `.explugin` archive against `verifying_key` and checks payload integrity.
///
/// # Errors
///
/// Returns [`PluginError::Signature`] if signature verification fails, or
/// [`PluginError::InvalidManifest`] if the archive or digest is invalid.
pub fn verify_signed_plugin(
    archive_bytes: &[u8],
    verifying_key: &VerifyingKey,
) -> Result<VerifiedPluginPackage, PluginError> {
    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|e| PluginError::InvalidManifest(format!("invalid explugin zip: {e}")))?;

    let manifest_bytes = read_entry(&mut archive, "manifest.json")?;
    let sig_bytes = read_entry(&mut archive, "manifest.sig").map_err(|_| PluginError::Signature)?;
    let sig_str = std::str::from_utf8(&sig_bytes).map_err(|_| PluginError::Signature)?;
    let sig_raw = base64::engine::general_purpose::STANDARD
        .decode(sig_str.trim())
        .map_err(|_| PluginError::Signature)?;
    let sig = Signature::from_slice(&sig_raw).map_err(|_| PluginError::Signature)?;
    verifying_key
        .verify_strict(&manifest_bytes, &sig)
        .map_err(|_| PluginError::Signature)?;

    let manifest = PluginManifest::from_json_bytes(&manifest_bytes)?;
    let payload_bytes = read_entry(&mut archive, "payload.bin")?;
    if sha256_hex(&payload_bytes) != manifest.payload_sha256 {
        return Err(PluginError::InvalidManifest(
            "payload sha256 digest mismatch".to_owned(),
        ));
    }

    Ok(VerifiedPluginPackage {
        manifest,
        payload_bytes,
    })
}

fn read_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, PluginError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| PluginError::InvalidManifest(format!("missing archive entry {name}")))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}
