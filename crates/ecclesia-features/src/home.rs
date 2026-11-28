//! The home page (plan F.17), the model: which file "Continue reading" offers, which recent files the search box
//! keeps, and what each file's cover is called in the disk cache. The page itself is the app's empty document panel.

use std::path::Path;

/// The pixel width covers are rendered at for compact/legacy uses.
pub const COVER_WIDTH: u32 = 192;

/// SumatraPDF Enhanced home thumbnail base width at 100% scale (`212 px`).
pub const THUMB_BASE_WIDTH: u32 = 212;
/// SumatraPDF Enhanced top-cropped half-page thumbnail height at 100% scale (`150 px`, `√2 : 1`).
pub const THUMB_CROPPED_HEIGHT: u32 = 150;
/// SumatraPDF Enhanced whole-page thumbnail height at 100% scale (`300 px`, `1 : √2`, exact `2×` cropped height).
pub const THUMB_FULL_HEIGHT: u32 = 300;
/// Maximum home thumbnail render width at 250% zoom (`530 px`).
pub const THUMB_MAX_WIDTH: u32 = THUMB_BASE_WIDTH * 250 / 100;
/// Maximum top-cropped half-page thumbnail render height at 250% zoom (`375 px`).
pub const THUMB_MAX_CROPPED_HEIGHT: u32 = THUMB_CROPPED_HEIGHT * 250 / 100;
/// Maximum whole-page thumbnail render height at 250% zoom (`750 px`).
pub const THUMB_MAX_FULL_HEIGHT: u32 = THUMB_FULL_HEIGHT * 250 / 100;

/// The file to continue reading: the one shown last, of those `exists` finds. `files` are `(path, last shown)`
/// pairs, the time in any unit that grows.
pub fn continue_reading<'a>(
    files: impl IntoIterator<Item = (&'a Path, i64)>,
    exists: impl Fn(&Path) -> bool,
) -> Option<&'a Path> {
    let mut files: Vec<_> = files.into_iter().collect();
    // Newest first, so only the files shown after the one offered are looked for on disk.
    files.sort_by_key(|&(_, seen)| std::cmp::Reverse(seen));
    files
        .into_iter()
        .map(|(path, _)| path)
        .find(|path| exists(path))
}

/// Whether the file at `path` is one the reader searched for: every word of `query` is in its name or its folder,
/// ignoring case. A blank query keeps every file.
pub fn matches(query: &str, path: &Path) -> bool {
    let haystack = path.to_string_lossy().to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// The cache file name of the cover of the file at `path`, of `len` bytes and last written at `modified` (seconds
/// since 1970). A file written again gets a new name, so a stale cover is never shown.
pub fn cover_name(path: &Path, len: u64, modified: u64) -> String {
    let mut hash = Fnv::default();
    hash.write(path.to_string_lossy().as_bytes());
    hash.write(&len.to_le_bytes());
    hash.write(&modified.to_le_bytes());
    format!("cover-{:016x}-w{COVER_WIDTH}.png", hash.0)
}

/// Cache file name for a SumatraPDF Enhanced-sized home cover (`530×375` top-cropped or `530×750` full-page).
pub fn cover_name_for_mode(path: &Path, len: u64, modified: u64, full_page: bool) -> String {
    let mut hash = Fnv::default();
    hash.write(path.to_string_lossy().as_bytes());
    hash.write(&len.to_le_bytes());
    hash.write(&modified.to_le_bytes());
    let mode = if full_page { "full" } else { "cropped" };
    format!("cover-{:016x}-w{THUMB_MAX_WIDTH}-{mode}.png", hash.0)
}

/// FNV-1a, 64 bits: the same name on every run and every build, as a cache file name must be.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_reading_offers_the_file_shown_last_that_is_still_there() {
        let files = [
            (Path::new("old.pdf"), 1),
            (Path::new("gone.pdf"), 9),
            (Path::new("new.pdf"), 5),
        ];
        let there = |path: &Path| path != Path::new("gone.pdf");
        assert_eq!(continue_reading(files, there), Some(Path::new("new.pdf")));
        assert_eq!(continue_reading([], there), None);
    }

    #[test]
    fn the_search_keeps_files_with_every_word_in_their_name_or_folder() {
        let path = Path::new("C:/Papers/Physics/Quantum Field Theory.pdf");
        assert!(matches("", path));
        assert!(matches("quantum", path));
        assert!(matches("PHYSICS field", path));
        assert!(!matches("quantum chemistry", path));
    }

    #[test]
    fn a_cover_is_named_by_its_file_and_changes_when_the_file_does() {
        let path = Path::new("C:/a.pdf");
        let name = cover_name(path, 100, 7);
        assert_eq!(name, cover_name(path, 100, 7));
        assert_ne!(name, cover_name(path, 101, 7));
        assert_ne!(name, cover_name(path, 100, 8));
        assert_ne!(name, cover_name(Path::new("C:/b.pdf"), 100, 7));
        assert!(name.starts_with("cover-") && name.ends_with(".png"));
    }
}
