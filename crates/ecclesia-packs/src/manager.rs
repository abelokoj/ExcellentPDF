//! The packs on this machine: what is installed, installing from a download or a file on disk, and removing.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use ed25519_dalek::VerifyingKey;

use crate::download::{self, Progress, Source};
use crate::error::Error;
use crate::file::{self, MANIFEST, Manifest};
use crate::index::Entry;
use crate::install;

/// A pack installed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// What it says it is.
    pub manifest: Manifest,
    /// Its folder.
    pub folder: PathBuf,
}

/// The packs under one folder (the app data's `packs`), trusted by one key.
#[derive(Debug, Clone)]
pub struct Manager {
    root: PathBuf,
    key: VerifyingKey,
}

impl Manager {
    /// The packs under `root`, signed by `key`.
    #[must_use]
    pub fn new(root: PathBuf, key: VerifyingKey) -> Self {
        Self { root, key }
    }

    /// Every pack installed, by id.
    ///
    /// # Errors
    ///
    /// The packs folder can't be read (a missing one has none).
    pub fn installed(&self) -> Result<Vec<Installed>, Error> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(Vec::new());
        };
        let mut found: Vec<Installed> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| {
                let bytes = std::fs::read(e.path().join(MANIFEST)).ok()?;
                Some(Installed {
                    manifest: Manifest::parse(&bytes).ok()?,
                    folder: e.path(),
                })
            })
            .collect();
        found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        Ok(found)
    }

    /// Whether pack `id` is installed: features ask before using theirs, and offer the download when it isn't.
    #[must_use]
    pub fn is_installed(&self, id: &str) -> bool {
        file::is_id(id) && self.root.join(id).join(MANIFEST).is_file()
    }

    /// The folder of pack `id`, when it is installed.
    #[must_use]
    pub fn folder(&self, id: &str) -> Option<PathBuf> {
        self.is_installed(id).then(|| self.root.join(id))
    }

    /// Installs the pack file at `path` (offline: copied from a disk), after its signature verifies.
    ///
    /// # Errors
    ///
    /// It can't be read, its signature doesn't verify, or it can't be unpacked.
    pub fn install_file(&self, path: &Path) -> Result<Manifest, Error> {
        let bytes = std::fs::read(path)?;
        let archive = file::verified(&bytes, &self.key)?;
        install::unpack(archive, &self.root, None)
    }

    /// Downloads pack `entry` (from a verified index) from `source`, resuming an earlier try, checks its size, hash
    /// and signature, and installs it. `cancel` stops the download, keeping what came for next time.
    ///
    /// # Errors
    ///
    /// As [`download::fetch`], then as [`Manager::install_file`].
    pub fn download(
        &self,
        entry: &Entry,
        source: &dyn Source,
        cancel: &AtomicBool,
        progress: Progress<'_>,
    ) -> Result<Manifest, Error> {
        std::fs::create_dir_all(&self.root)?;
        let part = self
            .root
            .join(format!(".{}-{}.part", entry.id, entry.version));
        let bytes = download::fetch(entry, source, &part, cancel, progress)?;
        let installed = file::verified(&bytes, &self.key)
            .and_then(|archive| install::unpack(archive, &self.root, Some(&entry.id)));
        // A complete file is used once: good or bad, the next try downloads afresh.
        std::fs::remove_file(&part)?;
        installed
    }

    /// Removes pack `id` and every file of it. Nothing for one that isn't installed.
    ///
    /// # Errors
    ///
    /// The id isn't a pack id, or a file can't be removed.
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        if !file::is_id(id) {
            return Err(Error::Format(format!("{id:?} is not a pack id")));
        }
        let folder = self.root.join(id);
        if folder.exists() {
            std::fs::remove_dir_all(&folder)?;
        }
        Ok(())
    }
}
