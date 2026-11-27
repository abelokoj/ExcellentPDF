//! What can go wrong with a pack.

use std::fmt;

/// Why a pack can't be read, downloaded, installed or removed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A signature doesn't verify: the index or the pack was changed, or signed by someone else.
    Signature,
    /// The downloaded file isn't the size the index says.
    Size {
        /// What the index says.
        expected: u64,
        /// What came.
        got: u64,
    },
    /// The downloaded file's SHA-256 isn't the index's.
    Hash,
    /// The index or a pack's manifest isn't what this app reads.
    Format(String),
    /// A pack's archive names a file outside the pack's folder.
    Unsafe(String),
    /// The download failed or was refused.
    Network(String),
    /// The download was cancelled; what came so far is kept to resume.
    Cancelled,
    /// Reading or writing a file failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signature => f.write_str("the signature doesn't verify"),
            Self::Size { expected, got } => {
                write!(
                    f,
                    "the pack is {got} bytes, not the {expected} the index says"
                )
            }
            Self::Hash => f.write_str("the pack's hash isn't the index's"),
            Self::Format(what) => write!(f, "not a pack this app reads: {what}"),
            Self::Unsafe(name) => write!(f, "the pack names a file outside its folder: {name}"),
            Self::Network(what) => write!(f, "the download failed: {what}"),
            Self::Cancelled => f.write_str("the download was cancelled"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
