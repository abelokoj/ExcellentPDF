//! What can go wrong with the library database.

use std::fmt;

/// An error from the library database.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// SQLite refused or failed.
    Sqlite(rusqlite::Error),
    /// The database's folder couldn't be made.
    Io(std::io::Error),
    /// A newer version of the app wrote the database: this one leaves it alone rather than guess at its schema.
    TooNew {
        /// The database's schema version.
        version: u32,
        /// The newest version this build knows.
        known: u32,
    },
    /// The writer thread has stopped, so the write was not made.
    WriterGone,
    /// SQLite couldn't use WAL for the file (a network drive, say), so a crash could not be survived; the journal
    /// mode it is in instead.
    NoWal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "sqlite: {err}"),
            Self::Io(err) => write!(f, "library folder: {err}"),
            Self::TooNew { version, known } => write!(
                f,
                "library database is version {version}, newer than this app's {known}"
            ),
            Self::WriterGone => f.write_str("library writer has stopped"),
            Self::NoWal(mode) => write!(f, "library database can't use WAL (journal mode {mode})"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::TooNew { .. } | Self::WriterGone | Self::NoWal(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
