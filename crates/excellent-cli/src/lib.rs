//! Batch tools, JSON-Schema tool runner, MCP stdio server, and UI control client/host
//! on top of `excellent-core`, `excellent-mupdf`, `excellent-features`, and `excellent-store`
//! (plan E.1, E.2, E.3, E.4).

pub mod args;
pub mod commands;
pub mod error;
pub mod mcp;
pub mod ocr_tools;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use excellent_core::tools::ToolRegistry;

pub use error::{CliError, CliExit};

/// Builds a [`ToolRegistry`] containing all core tools plus `excellent-features` OCR (`doc.ocr`)
/// and XFDF/W3C annotation export (`annot.export`).
#[must_use]
pub fn build_full_tool_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::with_defaults();
    ocr_tools::register_feature_tools(&mut reg);
    reg
}

/// Runs the CLI with process arguments, printing to stdout/stderr and returning an [`ExitCode`].
#[must_use]
pub fn run_cli<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    run_cli_with_io(args, &mut stdout, &mut stderr).code()
}

/// Runs the CLI writing all output to `stdout` and errors to `stderr`.
#[must_use]
pub fn run_cli_with_io<I, S, W1, W2>(args: I, stdout: &mut W1, stderr: &mut W2) -> CliExit
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    W1: Write,
    W2: Write,
{
    let parsed = match args::parse_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            let _ = writeln!(stderr, "error: {err}");
            return err.exit;
        }
    };
    if let Err(err) = commands::execute(&parsed, stdout) {
        if parsed.json {
            let payload = serde_json::json!({
                "ok": false,
                "exit_code": err.exit as u8,
                "error": err.message,
            });
            let _ = writeln!(stdout, "{payload}");
        } else {
            let _ = writeln!(stderr, "error: {err}");
        }
        return err.exit;
    }
    CliExit::Success
}
