//! Installing a verified pack: its archive is unpacked into a staging folder beside the packs, checked, and renamed
//! into place in one step, so a pack is either there whole or not at all. Every name in the archive must stay inside
//! the pack's folder.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::file::{MANIFEST, Manifest};

/// The largest a pack may unpack to, in bytes (8 GiB: the largest AI models): an archive claiming more is refused.
pub const MAX_UNPACKED: u64 = 8 << 30;

/// Unpacks `archive` (a verified pack's zip) as pack `expected_id` (when the index named one) into `root`, replacing
/// an older version of it. Returns its manifest.
///
/// # Errors
///
/// The archive isn't a zip, has no manifest or another id, names a file outside the pack's folder, unpacks to more
/// than [`MAX_UNPACKED`], or writing fails; then nothing under `root` changed but a staging folder it removes.
pub fn unpack(archive: &[u8], root: &Path, expected_id: Option<&str>) -> Result<Manifest, Error> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(archive)).map_err(|err| Error::Format(err.to_string()))?;
    let manifest = {
        let mut file = zip
            .by_name(MANIFEST)
            .map_err(|_| Error::Format("the pack has no manifest".into()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Manifest::parse(&bytes)?
    };
    if expected_id.is_some_and(|id| id != manifest.id) {
        return Err(Error::Format(format!(
            "the pack is {}, not {}",
            manifest.id,
            expected_id.unwrap_or_default()
        )));
    }
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.size()))
        .sum();
    if total > MAX_UNPACKED {
        return Err(Error::Format("the pack unpacks to too much".into()));
    }
    std::fs::create_dir_all(root)?;
    let staging = root.join(format!(".{}.staging-{}", manifest.id, std::process::id()));
    let result = extract(&mut zip, &staging).and_then(|()| {
        let target = root.join(&manifest.id);
        let old = root.join(format!(".{}.old-{}", manifest.id, std::process::id()));
        if target.exists() {
            std::fs::rename(&target, &old)?;
        }
        if let Err(err) = std::fs::rename(&staging, &target) {
            // Put the version that was there back.
            if old.exists() {
                std::fs::rename(&old, &target)?;
            }
            return Err(err.into());
        }
        if old.exists() {
            std::fs::remove_dir_all(&old)?;
        }
        Ok(())
    });
    if result.is_err() && staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result.map(|()| manifest)
}

/// Writes every file of `zip` under `into`, refusing a name that would leave it.
fn extract(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, into: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(into)?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|err| Error::Format(err.to_string()))?;
        let relative: PathBuf = file.enclosed_name().ok_or_else(|| {
            Error::Unsafe(file.name().map(|n| n.into_owned()).unwrap_or_default())
        })?;
        let path = into.join(relative);
        if file.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&path)?;
        std::io::copy(&mut file, &mut out)?;
    }
    Ok(())
}
