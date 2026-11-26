//! Exit codes and structured CLI errors for `excellent-cli` (plan E.1).

use std::fmt;
use std::process::ExitCode;

use excellent_core::tools::ToolError;
use excellent_features::ocr::OcrError;
use excellent_mupdf::MuError;

/// Process exit codes returned by `excellent-cli`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CliExit {
    /// Command completed successfully.
    Success = 0,
    /// Invalid CLI flags, arguments, JSON payload, or schema mismatch.
    Usage = 2,
    /// Filesystem error or path denied by `--root` confinement.
    Io = 3,
    /// PDF / MuPDF / OCR processing failure.
    Processing = 4,
    /// Verification failure (e.g. unverified redaction) or unauthorized control token.
    VerificationOrAuth = 5,
}

impl CliExit {
    /// Converts to [`std::process::ExitCode`].
    #[must_use]
    pub fn code(self) -> ExitCode {
        ExitCode::from(self as u8)
    }
}

/// Error returned by CLI argument parsing or command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CliError {
    /// Exit code associated with this failure.
    pub exit: CliExit,
    /// Human-readable diagnostic message.
    pub message: String,
}

impl CliError {
    /// Builds a CLI error with an explicit exit code.
    pub fn new(exit: CliExit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
        }
    }

    /// Usage / invalid-argument error (exit code 2).
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(CliExit::Usage, message)
    }

    /// Filesystem / path error (exit code 3).
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(CliExit::Io, message)
    }

    /// Document / OCR processing error (exit code 4).
    pub fn processing(message: impl Into<String>) -> Self {
        Self::new(CliExit::Processing, message)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        Self::new(CliExit::Io, err.to_string())
    }
}

impl From<MuError> for CliError {
    fn from(err: MuError) -> Self {
        Self::new(CliExit::Processing, err.to_string())
    }
}

impl From<OcrError> for CliError {
    fn from(err: OcrError) -> Self {
        let exit = match err {
            OcrError::Io(_) => CliExit::Io,
            _ => CliExit::Processing,
        };
        Self::new(exit, err.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        Self::new(CliExit::Usage, format!("invalid JSON: {err}"))
    }
}

impl From<ToolError> for CliError {
    fn from(err: ToolError) -> Self {
        let exit = match &err {
            ToolError::UnknownTool(_) | ToolError::InvalidArguments(_) => CliExit::Usage,
            ToolError::PathDenied { .. } | ToolError::Io(_) => CliExit::Io,
            ToolError::Document(_) | ToolError::Mupdf(_) | ToolError::Execution(_) => {
                CliExit::Processing
            }
            _ => CliExit::Processing,
        };
        Self::new(exit, err.to_string())
    }
}
