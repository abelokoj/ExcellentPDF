//! Agent tool table, MCP JSON-RPC 2.0 server, and UI control channel (plan E.2, E.3, E.4).
//!
//! [`ToolRegistry`] exposes typed JSON-Schema tools over `ecclesia-core` and `ecclesia-mupdf`
//! for CLI batch commands (`ecclesia-cli run`), the stdio MCP server ([`mcp`]), and the
//! capability-gated UI control channel ([`control`]).

pub mod builtin;
pub mod control;
pub mod mcp;
pub mod registry;
pub mod schema;

pub use mcp::{McpServer, PROTOCOL_VERSION};
pub use registry::{
    ScriptFailurePolicy, ToolCall, ToolCallResult, ToolCapability, ToolContext, ToolError,
    ToolHandler, ToolRegistry, ToolSpec,
};
pub use schema::validate_json;
