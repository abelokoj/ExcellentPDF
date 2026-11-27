//! Optional packs management (plan K.21, DD27): installed packs, signed index settings, offline `.expack`
//! installation from disk, resumable HTTPS downloads, and clean pack removal.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ecclesia_packs::download::{Https, Source};
use ecclesia_packs::index::{self, Entry};
use ecclesia_packs::{Installed, Manager};
use ed25519_dalek::VerifyingKey;
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use crate::settings::{self, Live};

/// Settings key for the signed pack index base URL (optional; offline by default).
pub(crate) const PACKS_INDEX_URL: &str = "packs_index_url";

/// Compiled-in Ed25519 verifying key for official packs (DD27).
///
/// Only the maintainer sets this key for signed releases. While `None`, no index or `.expack`
/// archive is trusted and packs are unavailable.
const PACKS_VERIFYING_KEY: Option<[u8; 32]> = None;

/// Directory where optional packs are installed (`<app_data>/packs`, or temp fallback in headless tests).
pub(crate) fn packs_root() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-packs"))
        .join("packs")
}

/// Returns the compiled-in maintainer Ed25519 verifying key, if one has been configured (DD27).
pub(crate) fn verifying_key() -> Option<VerifyingKey> {
    PACKS_VERIFYING_KEY.and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
}

/// Returns `true` when a compiled-in maintainer verifying key is present (DD27).
pub(crate) fn packs_available() -> bool {
    verifying_key().is_some()
}

/// Verifies a signed pack index against `key`, refusing all indexes when `key` is `None`.
pub(crate) fn verify_index_with_key(
    key: Option<VerifyingKey>,
    json: &[u8],
    sig: &[u8],
) -> Result<Vec<Entry>, String> {
    let Some(key) = key else {
        return Err("packs unavailable: no maintainer verifying key configured".to_owned());
    };
    index::verified(json, sig, &key).map_err(|e| e.to_string())
}

/// Builds a [`Manager`] for the compiled-in maintainer key and packs directory, or `None` when
/// no maintainer verifying key is compiled in (DD27).
pub(crate) fn manager() -> Option<Manager> {
    let key = verifying_key()?;
    Some(Manager::new(packs_root(), key))
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
        let Some(mgr) = manager() else {
            self.installed.set(Vec::new());
            return;
        };
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
        let Some(mgr) = manager() else {
            self.status.set(Some(t!("packs.unavailable")));
            return;
        };
        let dialog = OpenDialog::files()
            .title(t!("packs.pick_title"))
            .filter(FileFilter::new(t!("packs.filter"), ["expack"]));
        let answer = pick_paths(dialog);
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
        let Some(mgr) = manager() else {
            self.status.set(Some(t!("packs.unavailable")));
            return;
        };
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
        let Some(key) = verifying_key() else {
            self.status.set(Some(t!("packs.unavailable")));
            return;
        };
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
        self.busy.set(true);
        spawn(async move {
            let fetched = blocking(move || {
                let sig_url = format!("{trimmed}.sig");
                let json = fetch_https_bytes(&trimmed)?;
                let sig = fetch_https_bytes(&sig_url)?;
                verify_index_with_key(Some(key), &json, &sig)
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
        let Some(mgr) = manager() else {
            self.status.set(Some(t!("packs.unavailable")));
            return;
        };
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
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn refuses_packs_without_compiled_in_maintainer_key_and_rejects_rfc8032_test_key() {
        // Until the maintainer compiles in a real key (DD27), packs must be unavailable.
        assert!(verifying_key().is_none());
        assert!(!packs_available());
        assert!(manager().is_none());

        // RFC 8032 §7.1 test vector 1 secret key (public key d75a9801...511a).
        const RFC8032_TEST1_SECRET: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let signing_key = SigningKey::from_bytes(&RFC8032_TEST1_SECRET);
        let sha = "ab".repeat(32);
        let json = format!(
            r#"{{"packs": [{{"id": "dict-en", "version": "1", "title": "English dictionary", "size": 1200, "sha256": "{sha}", "url": "https://packs.example/dict-en-1.expack", "licence": "CC-BY-4.0"}}]}}"#
        );
        let sig = signing_key.sign(json.as_bytes()).to_bytes();

        // An index signed by the published RFC 8032 test key MUST be refused.
        assert!(verify_index_with_key(verifying_key(), json.as_bytes(), &sig).is_err());

        // Even if a different real key were compiled in, RFC 8032 test-1 signatures still fail.
        let other_maintainer_key = SigningKey::from_bytes(&[0x42; 32]).verifying_key();
        assert!(verify_index_with_key(Some(other_maintainer_key), json.as_bytes(), &sig).is_err());
    }

    #[test]
    fn formats_pack_sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(4096), "4 KB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }
}
