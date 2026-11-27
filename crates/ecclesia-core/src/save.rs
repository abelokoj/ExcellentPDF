//! Writing a PDF over its own file without risking it (K.15): the new file is written beside the old one, read back,
//! flushed to disk, and only then moved over it. A failure at any point leaves the old file as it was.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use ecclesia_mupdf::Save;

use crate::document::{Error, Result};

/// Writes `mupdf` with its edits over the file at `path`, which it was opened from.
///
/// # Errors
///
/// MuPDF can't write the file, the new file doesn't read back with the same pages ([`Error::Unverified`]), or it
/// can't be flushed or moved over the old one. The old file is untouched.
pub(crate) fn replace(mupdf: &mut ecclesia_mupdf::Document, path: &Path) -> Result<Save> {
    let temp = temp_path(path)?;
    let written = write_verified(mupdf, &temp).and_then(|save| {
        fs::rename(&temp, path)?;
        Ok(save)
    });
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// Where the new file is written: beside `path`, so moving it over is one rename on one disk.
fn temp_path(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path names no file"))?;
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(".ecclesiapdf-saving");
    Ok(path.with_file_name(temp))
}

fn write_verified(mupdf: &mut ecclesia_mupdf::Document, temp: &Path) -> Result<Save> {
    let save = mupdf.save_pdf(temp)?;
    let pages = ecclesia_mupdf::Document::open_pdf(temp)?.page_count()?;
    if pages != mupdf.page_count()? {
        return Err(Error::Unverified);
    }
    // On disk before it takes the old file's place.
    OpenOptions::new().write(true).open(temp)?.sync_all()?;
    Ok(save)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_file_is_hidden_beside_the_original() {
        let temp = temp_path(Path::new("/books/a b.pdf")).expect("a file");
        assert_eq!(temp, Path::new("/books/.a b.pdf.ecclesiapdf-saving"));
        assert!(temp_path(Path::new("/")).is_err());
    }
}
