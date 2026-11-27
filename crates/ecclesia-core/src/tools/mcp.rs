//! MCP JSON-RPC 2.0 stdio server wrapping [`ToolRegistry`] with `--root` path confinement (plan E.3).

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use super::registry::{ToolContext, ToolError, ToolRegistry};

/// MCP protocol version advertised in `initialize` responses.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

const JSONRPC_PARSE_ERROR: i64 = -32700;
const JSONRPC_INVALID_REQUEST: i64 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;

/// Stateful MCP JSON-RPC 2.0 server over line-delimited stdio streams.
#[derive(Debug)]
pub struct McpServer<'a> {
    registry: &'a ToolRegistry,
    ctx: ToolContext,
    initialized: bool,
}

impl<'a> McpServer<'a> {
    /// Creates an MCP server exposing `registry`, optionally confined to `root_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] if `root_dir` is `Some` and does not exist or is not a directory.
    pub fn new(registry: &'a ToolRegistry, root_dir: Option<&Path>) -> Result<Self, ToolError> {
        let ctx = match root_dir {
            Some(root) => ToolContext::with_root(root)?,
            None => ToolContext::new(),
        };
        Ok(Self {
            registry,
            ctx,
            initialized: false,
        })
    }

    /// Runs the blocking stdio loop on `std::io::stdin().lock()` and `std::io::stdout().lock()`.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if reading stdin or writing stdout fails.
    pub fn serve_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        self.serve(stdin.lock(), stdout.lock())
    }

    /// Serves newline-delimited JSON-RPC 2.0 messages from `reader` and writes responses to `writer`.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if reading or writing fails.
    pub fn serve<R: BufRead, W: Write>(&mut self, reader: R, mut writer: W) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line) {
                writer.write_all(reply.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Processes a single JSON-RPC 2.0 request or notification line.
    ///
    /// Returns `None` for notifications (requests without an `"id"` field).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                return Some(error_response(
                    Value::Null,
                    JSONRPC_PARSE_ERROR,
                    &format!("parse error: {err}"),
                ));
            }
        };
        let Some(obj) = req.as_object() else {
            return Some(error_response(
                Value::Null,
                JSONRPC_INVALID_REQUEST,
                "request must be a JSON object",
            ));
        };
        let id = obj.get("id").cloned();
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id
                .map(|id| error_response(id, JSONRPC_INVALID_REQUEST, "jsonrpc must equal '2.0'"));
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return id
                .map(|id| error_response(id, JSONRPC_INVALID_REQUEST, "missing 'method' string"));
        };
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));

        // Notifications have no `id` and must never emit a response.
        let Some(id) = id else {
            if matches!(method, "notifications/initialized" | "initialized") {
                self.initialized = true;
            }
            return None;
        };

        let reply = match method {
            "initialize" => self.handle_initialize(id, &params),
            "ping" => ok_response(id, json!({})),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, &params),
            other => error_response(
                id,
                JSONRPC_METHOD_NOT_FOUND,
                &format!("method not found: {other}"),
            ),
        };
        Some(reply)
    }

    fn handle_initialize(&mut self, id: Value, params: &Value) -> String {
        let requested_version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        let negotiated =
            if requested_version.starts_with("2024-") || requested_version.starts_with("2025-") {
                requested_version
            } else {
                PROTOCOL_VERSION
            };
        let root_note = self.ctx.root_dir.as_ref().map_or_else(
            || "unconfined".to_owned(),
            |p| format!("confined to root {}", p.display()),
        );
        ok_response(
            id,
            json!({
                "protocolVersion": negotiated,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "ecclesiapdf-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": format!("EcclesiaPDF MCP server ({root_note})."),
            }),
        )
    }

    fn handle_tools_list(&self, id: Value) -> String {
        let tools: Vec<Value> = self
            .registry
            .specs()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.input_schema,
                })
            })
            .collect();
        ok_response(id, json!({ "tools": tools }))
    }

    fn handle_tools_call(&self, id: Value, params: &Value) -> String {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error_response(id, JSONRPC_INVALID_PARAMS, "missing params.name string");
        };
        if self.registry.spec(name).is_none() {
            return error_response(
                id,
                JSONRPC_INVALID_PARAMS,
                &format!("unknown tool '{name}'"),
            );
        }
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.registry.execute(&self.ctx, name, &arguments) {
            Ok(output) => {
                let text =
                    serde_json::to_string_pretty(&output).unwrap_or_else(|_| output.to_string());
                ok_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": output,
                        "isError": false,
                    }),
                )
            }
            Err(err) => ok_response(
                id,
                json!({
                    "content": [{ "type": "text", "text": err.to_string() }],
                    "isError": true,
                }),
            ),
        }
    }
}

fn ok_response(id: Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use super::*;

    fn corpus_root() -> PathBuf {
        std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus"))
            .expect("corpus dir")
    }

    #[test]
    fn mcp_round_trip_enforces_root_confinement() {
        let reg = ToolRegistry::with_defaults();
        let root = corpus_root();
        let mut server = McpServer::new(&reg, Some(&root)).expect("server");

        let input_lines = [
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05" }
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            })
            .to_string(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }).to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "doc.info",
                    "arguments": { "path": "base14-outline-annot.pdf" }
                }
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "doc.info",
                    "arguments": { "path": "../Cargo.toml" }
                }
            })
            .to_string(),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "nonexistent/method" }).to_string(),
        ]
        .join("\n");

        let mut out = Vec::new();
        server
            .serve(Cursor::new(input_lines.as_bytes()), &mut out)
            .expect("serve");
        let responses: Vec<Value> = String::from_utf8(out)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid json response"))
            .collect();

        // Notification produced no response, so 6 responses for 7 messages.
        assert_eq!(responses.len(), 6);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(
            responses[0]["result"]["serverInfo"]["name"],
            "ecclesiapdf-mcp"
        );
        assert_eq!(responses[1]["id"], 2);
        assert_eq!(responses[1]["result"], json!({}));
        assert!(
            responses[2]["result"]["tools"]
                .as_array()
                .is_some_and(|t| t.len() >= 14)
        );

        // Inside root succeeded.
        assert_eq!(responses[3]["id"], 4);
        assert_eq!(responses[3]["result"]["isError"], false);
        assert_eq!(responses[3]["result"]["structuredContent"]["page_count"], 2);

        // Outside root was refused.
        assert_eq!(responses[4]["id"], 5);
        assert_eq!(responses[4]["result"]["isError"], true);
        let msg = responses[4]["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("");
        assert!(msg.contains("access denied"), "msg was: {msg}");

        // Unknown method returned -32601.
        assert_eq!(responses[5]["id"], 6);
        assert_eq!(responses[5]["error"]["code"], -32601);
    }
}
