//! Binary entry point for `excellent-cli` (plan E.1).

use std::process::ExitCode;

fn main() -> ExitCode {
    excellent_cli::run_cli(std::env::args_os())
}
