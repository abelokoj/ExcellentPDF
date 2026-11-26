//! MCP stdio server entry point for `excellent-cli mcp` (plan E.3).

use std::path::PathBuf;

use excellent_core::tools::McpServer;

use crate::build_full_tool_registry;
use crate::error::CliError;

/// Runs the blocking MCP JSON-RPC 2.0 server over stdin/stdout, confined to `root` if provided.
///
/// # Errors
///
/// Returns [`CliError`] if `root` is invalid or stdio fails.
pub fn serve_stdio(root: Option<PathBuf>) -> Result<(), CliError> {
    let registry = build_full_tool_registry();
    let mut server = McpServer::new(&registry, root.as_deref())?;
    server.serve_stdio()?;
    Ok(())
}
