//! Optional packs management (plan K.21, DD27): installed packs, signed index settings, offline `.expack`
//! installation from disk, resumable HTTPS downloads, and clean pack removal.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ed25519_dalek::VerifyingKey;
use excellent_packs::download::{Https, Source};
use excellent_packs::index::{self, Entry};
use excellent_packs::{Installed, Manager};
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use crate::settings::{self, Live};

/// Settings key for the signed pack index base URL (optional; offline by default).
pub(crate) const PACKS_INDEX_URL: &str = "packs_index_url";
/// Settings key for the Ed25519 verifying key hex (64 hex chars, per DD27).
pub(crate) const PACKS_KEY_HEX: &str = "packs_verifying_key";

/// Default placeholder verifying key when the maintainer key is not yet configured in `settings.toml` (DD27).
/// All-zero Ed25519 bytes are replaced by a valid curve point (`[1; 32]` public bytes from a fixed test key) so
/// `VerifyingKey::from_bytes` never fails; real release indexes configure `packs_verifying_key`.
const FALLBACK_KEY_BYTES: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

/// Directory where optional packs are installed (`<app_data>/packs`, or temp fallback in headless tests).
pub(crate) fn packs_root() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-packs"))
        .join("packs")
}

/// Parses a 64-character hex Ed25519 verifying key, falling back to [`FALLBACK_KEY_BYTES`].
pub(crate) fn parse_verifying_key(hex_text: Option<&str>) -> VerifyingKey {
    if let Some(hex) = hex_text.map(str::trim).filter(|s| s.len() == 64)
        && let Some(bytes) = decode_hex_32(hex)
        && let Ok(key) = VerifyingKey::from_bytes(&bytes)
    {
        return key;
    }
    VerifyingKey::from_bytes(&FALLBACK_KEY_BYTES).expect("valid fallback Ed25519 verifying key")
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    let mut out = [0_u8; 32];
    for (idx, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[idx] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// Builds a [`Manager`] for the current `settings.toml` pack key and packs directory.
pub(crate) fn manager(live: Live) -> Manager {
    let key_hex = live.read(|s| s.text(PACKS_KEY_HEX).map(str::to_owned));
    let key = parse_verifying_key(key_hex.as_deref());
    Manager::new(packs_root(), key)
}

/// UI state for the Settings -> Packs section (K.21).
#[derive(Clone, Copy)]
pub(crate) struct PacksState {
    live: Live,
    installed: Signal<Vec<Installed>>,
    available: Signal<Vec<Entry>>,
    status: Signal<Option<String>>,
    busy: Signal<bool>,
    cancel: Stored<Arc<AtomicBool>>,
}

impl PacksState {
    pub(crate) fn new(live: Live) -> Self {
        let state = Self {
            live,
            installed: signal(Vec::new()),
            available: signal(Vec::new()),
            status: signal(None),
            busy: signal(false),
            cancel: stored(Arc::new(AtomicBool::new(false))),
        };
        state.refresh();
        state
    }

    pub(crate) fn installed(self) -> Vec<Installed> {
        self.installed.get()
    }

    pub(crate) fn available(self) -> Vec<Entry> {
        self.available.get()
    }

    pub(crate) fn status(self) -> Option<String> {
        self.status.get()
    }

    pub(crate) fn is_busy(self) -> bool {
        self.busy.get()
    }

    /// Re-reads the installed packs folder.
    pub(crate) fn refresh(self) {
        let mgr = untracked(|| manager(self.live));
        match mgr.installed() {
            Ok(list) => self.installed.set(list),
            Err(err) => {
                log::warn!("packs: failed to list installed packs: {err}");
                self.status.set(Some(err.to_string()));
            }
        }
    }

    /// Installs a `.expack` file picked from disk (offline path, K.21 item 4).
    pub(crate) fn install_from_file(self) {
        let dialog = OpenDialog::files()
            .title(t!("packs.pick_title"))
            .filter(FileFilter::new(t!("packs.filter"), ["expack"]));
        let answer = pick_paths(dialog);
        let mgr = untracked(|| manager(self.live));
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            self.busy.set(true);
            let res = blocking(move || mgr.install_file(&path).map_err(|e| e.to_string())).await;
            self.busy.set(false);
            match res {
                Ok(Ok(manifest)) => {
                    self.status.set(Some(t!(
                        "packs.installed_ok",
                        name = manifest.title,
                        version = manifest.version
                    )));
                    self.refresh();
                }
                Ok(Err(err)) => {
                    self.status
                        .set(Some(t!("packs.install_failed", reason = err)));
                }
                Err(err) => {
                    self.status
                        .set(Some(t!("packs.install_failed", reason = err.to_string())));
                }
            }
        });
    }

    /// Removes an installed pack cleanly.
    pub(crate) fn remove(self, id: String) {
        let mgr = untracked(|| manager(self.live));
        match mgr.remove(&id) {
            Ok(()) => {
                self.status.set(Some(t!("packs.removed_ok", id = id)));
                self.refresh();
            }
            Err(err) => {
                self.status
                    .set(Some(t!("packs.remove_failed", reason = err.to_string())));
            }
        }
    }

    /// Fetches and verifies the signed index from `packs_index_url` (HTTPS only).
    pub(crate) fn refresh_index(self) {
        let url = self
            .live
            .read(|s| s.text(PACKS_INDEX_URL).map(str::to_owned))
            .unwrap_or_default();
        let trimmed = url.trim().to_owned();
        if trimmed.is_empty() {
            self.status.set(Some(t!("packs.no_index_url")));
            return;
        }
        if !trimmed.starts_with("https://") {
            self.status.set(Some(t!("packs.https_only")));
            return;
        }
        let key_hex = self.live.read(|s| s.text(PACKS_KEY_HEX).map(str::to_owned));
        let key = parse_verifying_key(key_hex.as_deref());
        self.busy.set(true);
        spawn(async move {
            let fetched = blocking(move || {
                let sig_url = format!("{trimmed}.sig");
                let json = fetch_https_bytes(&trimmed)?;
                let sig = fetch_https_bytes(&sig_url)?;
                index::verified(&json, &sig, &key).map_err(|e| e.to_string())
            })
            .await;
            self.busy.set(false);
            match fetched {
                Ok(Ok(entries)) => {
                    let count = entries.len();
                    self.available.set(entries);
                    self.status
                        .set(Some(t!("packs.index_loaded", count = count)));
                }
                Ok(Err(err)) => {
                    self.status
                        .set(Some(t!("packs.index_failed", reason = err)));
                }
                Err(err) => {
                    self.status
                        .set(Some(t!("packs.index_failed", reason = err.to_string())));
                }
            }
        });
    }

    /// Downloads and installs a pack entry from the verified index with resume and cancel support.
    pub(crate) fn download(self, entry: Entry) {
        let mgr = untracked(|| manager(self.live));
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel.with(|slot| *slot = Arc::clone(&cancel));
        self.busy.set(true);
        self.status
            .set(Some(t!("packs.downloading", name = entry.title.clone())));
        spawn(async move {
            let res = blocking(move || {
                let source = Https;
                let mut progress = |_done: u64, _total: u64| {};
                mgr.download(&entry, &source, &cancel, &mut progress)
                    .map_err(|e| e.to_string())
            })
            .await;
            self.busy.set(false);
            match res {
                Ok(Ok(manifest)) => {
                    self.status.set(Some(t!(
                        "packs.installed_ok",
                        name = manifest.title,
                        version = manifest.version
                    )));
                    self.refresh();
                }
                Ok(Err(err)) => {
                    self.status
                        .set(Some(t!("packs.install_failed", reason = err)));
                }
                Err(err) => {
                    self.status
                        .set(Some(t!("packs.install_failed", reason = err.to_string())));
                }
            }
        });
    }

    /// Cancels an active pack download, preserving `.part` bytes for resume.
    pub(crate) fn cancel_download(self) {
        self.cancel.with(|flag| flag.store(true, Ordering::Relaxed));
    }
}

fn fetch_https_bytes(url: &str) -> Result<Vec<u8>, String> {
    let source = Https;
    let (mut reader, _) = source.open(url, 0).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    // Cap index / signature responses at 16 MiB.
    reader
        .by_ref()
        .take(16 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Formats a byte size cleanly for the Settings -> Packs UI.
pub(crate) fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_fallback_key_and_rejects_bad_hex() {
        let fallback = parse_verifying_key(None);
        assert_eq!(parse_verifying_key(Some("not-hex")), fallback);
        let hex = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let parsed = parse_verifying_key(Some(hex));
        assert_eq!(parsed, fallback);
    }

    #[test]
    fn formats_pack_sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(4096), "4 KB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }
}
