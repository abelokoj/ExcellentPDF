//! Which file a document is (plan §11.16, "How it stays efficient" 1): its path, size, modified time and a hash of
//! its first and last 64 KB. Two paths to one file, or a file opened twice, have one identity; a file changed on disk
//! has a new one.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::oplog::Fingerprint;

/// Bytes hashed at each end of the file.
const SAMPLE: u64 = 64 * 1024;

/// A file's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileId {
    /// The canonical path.
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    /// FNV-1a of the first and last [`SAMPLE`] bytes.
    sample: u64,
}

impl FileId {
    /// The identity of the file at `path`, reading at most 128 KB of it.
    ///
    /// # Errors
    ///
    /// The file can't be found or read.
    pub fn of(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let mut file = File::open(&path)?;
        let metadata = file.metadata()?;
        let size = metadata.len();
        let mut sample = Fnv::default();
        let mut buffer = Vec::new();
        (&mut file).take(SAMPLE).read_to_end(&mut buffer)?;
        sample.write(&buffer);
        if size > SAMPLE {
            buffer.clear();
            file.seek(SeekFrom::Start(size.saturating_sub(SAMPLE).max(SAMPLE)))?;
            file.take(SAMPLE).read_to_end(&mut buffer)?;
            sample.write(&buffer);
        }
        Ok(Self {
            path,
            size,
            modified: metadata.modified().ok(),
            sample: sample.0,
        })
    }

    /// The canonical path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A number for this identity that is the same in every run (FNV-1a of all of it), to name what is cached for
    /// the file on disk.
    pub fn key(&self) -> u64 {
        let mut hash = Fnv::default();
        hash.write(self.path.as_os_str().as_encoded_bytes());
        hash.write(&self.size.to_le_bytes());
        let modified = self
            .modified
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        hash.write(&modified.to_le_bytes());
        hash.write(&self.sample.to_le_bytes());
        hash.0
    }

    /// A number for the bytes alone (size and sample), wherever the file is and whenever it was written: finds a
    /// renamed or moved file again (B.6).
    pub fn content_key(&self) -> u64 {
        let mut hash = Fnv::default();
        hash.write(&self.size.to_le_bytes());
        hash.write(&self.sample.to_le_bytes());
        hash.0
    }

    /// A number for the path alone, the same in every run and through every version of the file: names what
    /// follows the file as it is saved, such as its operation log.
    pub(crate) fn path_key(&self) -> u64 {
        let mut hash = Fnv::default();
        hash.write(self.path.as_os_str().as_encoded_bytes());
        hash.0
    }

    /// This version of the file, as the operation log records it.
    pub(crate) fn fingerprint(&self) -> Fingerprint {
        let modified = self
            .modified
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            });
        Fingerprint {
            size: self.size,
            modified,
            sample: self.sample,
        }
    }

    /// Whether this file holds the same bytes as the version `fingerprint`, whenever it was written.
    pub(crate) fn same_bytes(&self, fingerprint: Fingerprint) -> bool {
        self.size == fingerprint.size && self.sample == fingerprint.sample
    }
}

/// 64-bit FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`, and enough to tell files apart
/// alongside the size and time.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ecclesia-core-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("write");
        path
    }

    #[test]
    fn one_file_has_one_identity_and_new_content_a_new_one() {
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let path = temp_file("identity.bin", &big);
        let first = FileId::of(&path).expect("id");
        assert_eq!(first, FileId::of(&path).expect("id"));

        let mut changed = big.clone();
        *changed.last_mut().expect("bytes") ^= 1;
        let other = temp_file("identity-other.bin", &changed);
        let second = FileId::of(&other).expect("id");
        assert_ne!(first.sample, second.sample, "the last 64 KB differ");
        assert_ne!(first.content_key(), second.content_key());

        // B.6: the same bytes under another name are the same content.
        let copy = temp_file("identity-copy.bin", &big);
        assert_eq!(
            first.content_key(),
            FileId::of(&copy).expect("id").content_key()
        );
        assert_ne!(first.key(), FileId::of(&copy).expect("id").key());

        std::fs::remove_file(path).expect("clean up");
        std::fs::remove_file(other).expect("clean up");
        std::fs::remove_file(copy).expect("clean up");
    }

    #[test]
    fn fnv_matches_its_reference_values() {
        let mut empty = Fnv::default();
        empty.write(b"");
        assert_eq!(empty.0, 0xcbf2_9ce4_8422_2325);
        let mut a = Fnv::default();
        a.write(b"a");
        assert_eq!(a.0, 0xaf63_dc4c_8601_ec8c);
    }
}
