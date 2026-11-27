//! JSON-RPC 2.0 / stdio plugin runner host with capability-gated host callbacks (plan E.5).

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};

use ecclesia_core::sandbox::os::SandboxedStdioProcess;
#[cfg(windows)]
use ecclesia_core::sandbox::{KillOnDrop, OsSandboxConfig, TokenRestriction};
use ecclesia_core::sandbox::{OsSandboxGuard, SandboxAttachment};
use serde_json::{Value, json};

use super::PluginError;
use super::manifest::PluginCapability;
use super::sandbox::PermissionSandbox;

/// Host callback trait invoked when a sandboxed plugin calls back into EcclesiaPDF tools,
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

/// Out-of-process stdio JSON-RPC 2.0 plugin runner contained inside an OS sandbox (plan E.5, K.22).
///
/// On Windows, the plugin runs under an [`OsSandboxGuard`] with a restricted low-privilege token
/// (`DISABLE_MAX_PRIVILEGE | LUA_TOKEN`), a Job Object enforcing `active_process_limit = 1`,
/// memory/UI limits, and `KILL_ON_JOB_CLOSE`, an empty environment block, and an isolated working
/// directory. On non-Windows platforms, external plugins are refused until an OS sandbox exists.
#[derive(Debug)]
pub struct StdioJsonRpcRunner {
    sandbox: OsSandboxGuard,
    process: SandboxedStdioProcess,
    work_dir: PathBuf,
    owns_work_dir: bool,
    next_id: u64,
}

impl StdioJsonRpcRunner {
    /// Spawns `command` with `args` inside a fresh isolated working directory and OS sandbox,
    /// communicating via line-delimited JSON-RPC 2.0 on stdin/stdout.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Runtime`] on platforms without an OS sandbox or if sandbox setup
    /// fails, or [`PluginError::Io`] if creating the working directory fails.
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, PluginError> {
        #[cfg(not(windows))]
        {
            let _ = (command, args);
            Err(PluginError::Runtime(
                "external plugins are disabled on this platform until an OS sandbox is implemented"
                    .to_owned(),
            ))
        }
        #[cfg(windows)]
        {
            static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(1);
            let seq = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
            let work_dir =
                std::env::temp_dir().join(format!("ecclesia-plugin-{}-{seq}", std::process::id()));
            std::fs::create_dir_all(&work_dir)?;
            match Self::spawn_inner(command, args, work_dir.clone(), true) {
                Ok(runner) => Ok(runner),
                Err(err) => {
                    let _ = std::fs::remove_dir_all(&work_dir);
                    Err(err)
                }
            }
        }
    }

    /// Spawns `command` with `args` inside the caller-provided isolated `work_dir` and OS sandbox,
    /// communicating via line-delimited JSON-RPC 2.0 on stdin/stdout.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Runtime`] on platforms without an OS sandbox or if sandbox setup
    /// fails, or [`PluginError::Io`] if creating `work_dir` fails.
    pub fn spawn_in_dir(
        command: &str,
        args: &[&str],
        work_dir: &Path,
    ) -> Result<Self, PluginError> {
        #[cfg(not(windows))]
        {
            let _ = (command, args, work_dir);
            Err(PluginError::Runtime(
                "external plugins are disabled on this platform until an OS sandbox is implemented"
                    .to_owned(),
            ))
        }
        #[cfg(windows)]
        {
            std::fs::create_dir_all(work_dir)?;
            Self::spawn_inner(command, args, work_dir.to_path_buf(), false)
        }
    }

    #[cfg(windows)]
    fn spawn_inner(
        command: &str,
        args: &[&str],
        work_dir: PathBuf,
        owns_work_dir: bool,
    ) -> Result<Self, PluginError> {
        let mut cfg = OsSandboxConfig::new();
        cfg.active_process_limit = 1;
        cfg.token_restriction = TokenRestriction::RestrictedLowPrivilege;
        cfg.kill_on_drop = KillOnDrop::TerminateProcessTree;

        let mut sandbox =
            OsSandboxGuard::create(cfg).map_err(|err| PluginError::Runtime(err.to_string()))?;
        let process = sandbox
            .spawn_sandboxed_stdio(command, args, &work_dir)
            .map_err(|err| PluginError::Runtime(err.to_string()))?;
        Ok(Self {
            sandbox,
            process,
            work_dir,
            owns_work_dir,
            next_id: 1,
        })
    }

    /// Returns the OS sandbox attachment metadata for this plugin process.
    #[must_use]
    pub fn sandbox_attachment(&self) -> &SandboxAttachment {
        self.sandbox.attachment()
    }

    /// Returns the isolated working directory assigned to this plugin process.
    #[must_use]
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Returns the operating-system PID of the sandboxed plugin process.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.process.pid
    }

    /// Returns `true` if the sandboxed plugin process is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.process.is_running()
    }
}

impl Drop for StdioJsonRpcRunner {
    fn drop(&mut self) {
        let _ = self.sandbox.terminate_job(0);
        let _ = self.process.wait();
        if self.owns_work_dir {
            let _ = std::fs::remove_dir_all(&self.work_dir);
        }
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

        writeln!(self.process.stdin, "{req_line}")?;
        self.process.stdin.flush()?;

        let mut reader = BufReader::new(&mut self.process.stdout);
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
                writeln!(self.process.stdin, "{reply_line}")?;
                self.process.stdin.flush()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    struct NoopBridge;

    #[cfg(windows)]
    impl PluginHostBridge for NoopBridge {
        fn required_capability(&self, method: &str) -> Result<PluginCapability, PluginError> {
            Err(PluginError::Runtime(format!("unexpected method: {method}")))
        }

        fn call_host(&mut self, method: &str, _params: &Value) -> Result<Value, PluginError> {
            Err(PluginError::Runtime(format!("unexpected method: {method}")))
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_stdio_runner_sandboxes_token_env_workdir_blocks_grandchild_and_kills_on_drop() {
        use ecclesia_core::sandbox::{JobAssignment, TokenState};

        let work_dir = std::env::temp_dir().join(format!(
            "ecclesia-plugin-runner-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&work_dir);
        std::fs::create_dir_all(&work_dir).expect("create plugin work dir");

        // Write a tiny batch plugin inside work_dir. Invoking it via relative name proves CWD is
        // isolated to work_dir; checking CARGO_MANIFEST_DIR proves the inherited env is empty;
        // spawning a grandchild cmd.exe proves active_process_limit=1 blocks child creation; and
        // blocking on a second `set /p` proves dropping the runner terminates the process cleanly.
        let script = work_dir.join("plugin.cmd");
        std::fs::write(
            &script,
            concat!(
                "@echo off\r\n",
                "set /p req=\r\n",
                "cmd.exe /D /C \"echo escaped > escaped.txt\" 2>nul\r\n",
                "if defined CARGO_MANIFEST_DIR (\r\n",
                "  echo {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"env_leaked\":true}}\r\n",
                ") else (\r\n",
                "  echo {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"env_leaked\":false}}\r\n",
                ")\r\n",
                "set /p hold=\r\n",
            ),
        )
        .expect("write plugin script");

        let mut runner =
            StdioJsonRpcRunner::spawn_in_dir("cmd.exe", &["/D", "/C", "plugin.cmd"], &work_dir)
                .expect("spawn sandboxed plugin");
        assert_eq!(
            runner.sandbox_attachment().job_assigned,
            JobAssignment::Assigned
        );
        assert_eq!(
            runner.sandbox_attachment().token_state,
            TokenState::Restricted
        );
        assert_eq!(runner.work_dir(), work_dir.as_path());
        assert!(runner.pid() > 0);

        let perms = PermissionSandbox::new();
        let mut bridge = NoopBridge;
        let res = runner
            .invoke("test-plugin", "ping", &json!({}), &perms, &mut bridge)
            .expect("json-rpc invoke");
        assert_eq!(res["env_leaked"], false, "parent env must not leak");
        assert!(
            !work_dir.join("escaped.txt").exists(),
            "Job Object active_process_limit=1 must block grandchild spawn"
        );
        assert!(
            runner.is_running(),
            "plugin should still be waiting on stdin before drop"
        );

        // Dropping the runner must terminate the blocked child via Job Object termination.
        drop(runner);
        let _ = std::fs::remove_dir_all(&work_dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_stdio_runner_refuses_external_plugins() {
        let err = StdioJsonRpcRunner::spawn("echo", &["hello"])
            .expect_err("external plugins must be refused without an OS sandbox");
        assert!(
            matches!(err, PluginError::Runtime(ref msg) if msg.contains("disabled on this platform")),
            "unexpected error: {err}"
        );
    }
}
