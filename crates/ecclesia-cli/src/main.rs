//! Binary entry point for `ecclesia-cli` (plan E.1).

use std::process::ExitCode;

fn main() -> ExitCode {
    ecclesia_cli::run_cli(std::env::args_os())
}
