//! Downloading a pack with resume: the bytes go into a `.part` file beside the packs; a download that stops (the
//! network drops, the app quits, the reader cancels) starts again where the part file ends, with an HTTP range
//! request. Only a complete file, of the size and hash the signed index gives, is handed on.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::index::Entry;

/// Where a download's bytes come from: the network, or a test's own.
pub trait Source {
    /// A reader of `url` from byte `from` on. A source that can't start mid-way returns the whole file and says so
    /// with `false`.
    ///
    /// # Errors
    ///
    /// The source can't be reached or refuses.
    fn open(&self, url: &str, from: u64) -> Result<(Box<dyn Read>, bool), Error>;
}

/// The network, over HTTPS.
#[derive(Debug, Default, Clone, Copy)]
pub struct Https;

impl Source for Https {
    fn open(&self, url: &str, from: u64) -> Result<(Box<dyn Read>, bool), Error> {
        if !url.starts_with("https://") {
            return Err(Error::Network(format!("{url} isn't HTTPS")));
        }
        let mut request = ureq::get(url);
        if from > 0 {
            request = request.header("Range", &format!("bytes={from}-"));
        }
        let response = request
            .call()
            .map_err(|err| Error::Network(err.to_string()))?;
        let resumed = from > 0 && response.status().as_u16() == 206;
        Ok((
            Box::new(response.into_body().into_reader()),
            resumed || from == 0,
        ))
    }
}

/// How far a download is: bytes so far and the whole.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64);

/// Downloads `entry` from `source` into `part` (resuming what is there), checking its size and SHA-256 against the
/// index, and returns the whole file. `cancel` stops it between reads, keeping what came.
///
/// # Errors
///
/// [`Error::Cancelled`] (the part file kept), a network or file error (kept), or [`Error::Size`] / [`Error::Hash`]
/// when what came isn't the pack (the part file removed, so the next try starts afresh).
pub fn fetch(
    entry: &Entry,
    source: &dyn Source,
    part: &Path,
    cancel: &AtomicBool,
    progress: Progress<'_>,
) -> Result<Vec<u8>, Error> {
    let mut have = std::fs::metadata(part).map_or(0, |m| m.len());
    if have > entry.size {
        std::fs::remove_file(part)?;
        have = 0;
    }
    if have < entry.size {
        let (mut reader, resumed) = source.open(&entry.url, have)?;
        if !resumed && have > 0 {
            // The server sent the whole file again: start the part over.
            std::fs::remove_file(part)?;
            have = 0;
        }
        let mut out = OpenOptions::new().create(true).append(true).open(part)?;
        let mut buf = vec![0; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                out.flush()?;
                return Err(Error::Cancelled);
            }
            let n = reader
                .read(&mut buf)
                .map_err(|err| Error::Network(err.to_string()))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            have += n as u64;
            progress(have, entry.size);
            if have > entry.size {
                break;
            }
        }
        out.sync_all()?;
    }
    let bytes = std::fs::read(part)?;
    let checked = check(entry, &bytes);
    if checked.is_err() {
        std::fs::remove_file(part)?;
    }
    checked.map(|()| bytes)
}

/// Whether `bytes` are the pack `entry` names: its size and SHA-256.
fn check(entry: &Entry, bytes: &[u8]) -> Result<(), Error> {
    let got = bytes.len() as u64;
    if got != entry.size {
        return Err(Error::Size {
            expected: entry.size,
            got,
        });
    }
    let hash = format!("{:x}", Sha256::digest(bytes));
    if hash != entry.sha256 {
        return Err(Error::Hash);
    }
    Ok(())
}
