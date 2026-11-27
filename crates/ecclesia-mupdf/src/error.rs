//! Errors reported by MuPDF through the shim.

use core::ffi::CStr;
use core::fmt;

use ecclesia_mupdf_sys::{
    FZ_ERROR_ABORT, FZ_ERROR_ARGUMENT, FZ_ERROR_FORMAT, FZ_ERROR_SYNTAX, FZ_ERROR_TRYLATER,
    FZ_ERROR_UNSUPPORTED, ex_error,
};

/// The result of a MuPDF call.
pub type Result<T> = core::result::Result<T, MuError>;

/// An error from MuPDF, with its message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct MuError {
    code: i32,
    message: String,
}

impl MuError {
    /// MuPDF's message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The job was cancelled through its cookie.
    pub fn is_aborted(&self) -> bool {
        self.code == FZ_ERROR_ABORT
    }

    /// The data isn't there yet (a file still loading); try again later.
    pub fn is_try_later(&self) -> bool {
        self.code == FZ_ERROR_TRYLATER
    }

    /// The file is damaged, or not in a format MuPDF reads.
    pub fn is_invalid_file(&self) -> bool {
        matches!(
            self.code,
            FZ_ERROR_FORMAT | FZ_ERROR_SYNTAX | FZ_ERROR_UNSUPPORTED
        )
    }

    /// The error a failing shim call filled in.
    pub(crate) fn from_shim(err: &ex_error) -> Self {
        // SAFETY: the shim always NUL-terminates `message` (snprintf), and a zeroed one is an empty string.
        let message = unsafe { CStr::from_ptr(err.message.as_ptr()) };
        Self {
            code: err.code,
            message: message.to_string_lossy().into_owned(),
        }
    }

    /// An error raised on the Rust side, with MuPDF's code for it.
    pub(crate) fn new(code: i32, message: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
        }
    }

    /// A bad argument caught on the Rust side, before calling MuPDF.
    pub(crate) fn argument(message: &str) -> Self {
        Self::new(FZ_ERROR_ARGUMENT, message)
    }
}

impl fmt::Display for MuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MuError {}
