//! JSON-RPC 2.0 / stdio plugin runner host with capability-gated host callbacks (plan E.5).

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use super::PluginError;
use super::manifest::PluginCapability;
use super::sandbox::PermissionSandbox;

/// Host callback trait invoked when a sandboxed plugin calls back into ExcellentPDF tools,
/// annotations, network, or UI panels.
pub trait PluginHostBridge {
    /// Required capability for `method` (used by [`PermissionSandbox`] before invoking).
    fn required_capability(&self, method: &str) -> Result<PluginCapability, PluginError>;

    /// Executes a capability-authorized host callback `method` with `params`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError`] if the callback fails or is unknown.
    fn call_host(&mut self, method: &str, params: &Value) -> Result<Value, PluginError>;
}

/// Transport-independent runner executing JSON-RPC 2.0 requests against a plugin instance (plan E.5).
pub trait PluginRunner {
    /// Invokes a JSON-RPC `method` on the plugin, enforcing `sandbox` on any host callbacks.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError`] if the plugin fails, returns an invalid response, or violates `sandbox`.
    fn invoke(
        &mut self,
        plugin_id: &str,
        method: &str,
        params: &Value,
        sandbox: &PermissionSandbox,
        host: &mut dyn PluginHostBridge,
    ) -> Result<Value, PluginError>;
}

/// Pure-Rust in-process JSON-RPC 2.0 plugin runner for deterministic testing and built-in adapters.
#[derive(Debug, Clone)]
pub struct InMemoryJsonRpcPlugin<F> {
    handler: F,
}

impl<F> InMemoryJsonRpcPlugin<F> {
    /// Wraps a pure-Rust JSON-RPC handler function.
    #[must_use]
    pub const fn new(handler: F) -> Self {
        Self { handler }
    }
}

/// Host callback request emitted by an [`InMemoryJsonRpcPlugin`].
#[derive(Debug, Clone, PartialEq)]
pub struct HostCallbackRequest {
    /// Host method name (e.g. `"doc.info"`, `"annot.add"`, `"net.fetch"`, `"ui.panel"`).
    pub method: String,
    /// Structured parameters.
    pub params: Value,
}

/// Response returned by an [`InMemoryJsonRpcPlugin`] step.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PluginStep {
    /// Plugin completed and produced a JSON result.
    Done(Value),
    /// Plugin requests a host callback that must pass [`PermissionSandbox`] first.
    CallHost(HostCallbackRequest),
}

impl<F> PluginRunner for InMemoryJsonRpcPlugin<F>
where
    F: FnMut(&str, &Value, Option<&Value>) -> Result<PluginStep, PluginError>,
{
    fn invoke(
        &mut self,
        plugin_id: &str,
        method: &str,
        params: &Value,
        sandbox: &PermissionSandbox,
        host: &mut dyn PluginHostBridge,
    ) -> Result<Value, PluginError> {
        let mut last_host_result: Option<Value> = None;
        for _ in 0..16 {
            match (self.handler)(method, params, last_host_result.as_ref())? {
                PluginStep::Done(val) => return Ok(val),
                PluginStep::CallHost(req) => {
                    let cap = host.required_capability(&req.method)?;
                    sandbox.authorize(plugin_id, cap)?;
                    let res = host.call_host(&req.method, &req.params)?;
                    last_host_result = Some(res);
                }
            }
        }
        Err(PluginError::Runtime(
            "plugin exceeded maximum host-callback roundtrips".to_owned(),
        ))
    }
}

/// Out-of-process stdio JSON-RPC 2.0 plugin runner (zero extra runtime dependencies) (plan E.5).
#[derive(Debug)]
pub struct StdioJsonRpcRunner {
    child: Child,
    next_id: u64,
}

impl StdioJsonRpcRunner {
    /// Spawns `command` with `args` communicating via line-delimited JSON-RPC 2.0 on stdin/stdout.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Io`] if spawning the plugin process fails.
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, PluginError> {
        let child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self { child, next_id: 1 })
    }
}

impl Drop for StdioJsonRpcRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl PluginRunner for StdioJsonRpcRunner {
    fn invoke(
        &mut self,
        plugin_id: &str,
        method: &str,
        params: &Value,
        sandbox: &PermissionSandbox,
        host: &mut dyn PluginHostBridge,
    ) -> Result<Value, PluginError> {
        let req_id = self.next_id;
        self.next_id += 1;

        let req_line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| PluginError::Runtime(e.to_string()))?;

        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::Runtime("plugin stdin closed".to_owned()))?;
        writeln!(stdin, "{req_line}")?;
        stdin.flush()?;

        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| PluginError::Runtime("plugin stdout closed".to_owned()))?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                return Err(PluginError::Runtime(
                    "plugin exited unexpectedly".to_owned(),
                ));
            }
            let msg: Value = serde_json::from_str(line.trim())
                .map_err(|e| PluginError::Runtime(format!("invalid json-rpc line: {e}")))?;

            // Handle host-callback requests initiated by the plugin process.
            if let Some(cb_method) = msg.get("method").and_then(Value::as_str) {
                let cb_id = msg.get("id").cloned().unwrap_or(Value::Null);
                let cb_params = msg.get("params").cloned().unwrap_or(Value::Null);
                let reply = match host
                    .required_capability(cb_method)
                    .and_then(|cap| sandbox.authorize(plugin_id, cap))
                    .and_then(|()| host.call_host(cb_method, &cb_params))
                {
                    Ok(res) => json!({ "jsonrpc": "2.0", "id": cb_id, "result": res }),
                    Err(err) => json!({
                        "jsonrpc": "2.0",
                        "id": cb_id,
                        "error": { "code": -32001, "message": err.to_string() }
                    }),
                };
                let reply_line = serde_json::to_string(&reply)
                    .map_err(|e| PluginError::Runtime(e.to_string()))?;
                let stdin = self
                    .child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| PluginError::Runtime("plugin stdin closed".to_owned()))?;
                writeln!(stdin, "{reply_line}")?;
                stdin.flush()?;
                continue;
            }

            if msg.get("id").and_then(Value::as_u64) == Some(req_id) {
                if let Some(err) = msg.get("error") {
                    return Err(PluginError::Runtime(err.to_string()));
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}
