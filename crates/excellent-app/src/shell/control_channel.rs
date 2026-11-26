//! UI Control Channel loopback server & command bridge (plan E.4).
//!
//! Starts an authenticated loopback TCP control server (`LoopbackControlServer`) when enabled in
//! settings (`control_channel_enabled`, default on), writes the endpoint and capability token to
//! `control.token` inside the app settings folder, and queues incoming UI actions for the shell.

use std::collections::VecDeque;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use excellent_core::tools::control::{
    ControlCapability, ControlCommand, ControlError, ControlErrorCode, ControlHandler,
    ControlToken, FrameCodec, LoopbackControlServer,
};
use serde_json::{Value, json};

use crate::commands::COMMANDS;
use crate::settings::{self, write_atomically};

/// `settings.toml` key controlling whether the UI control channel listens on startup (E.4).
pub(crate) const CONTROL_CHANNEL_ENABLED: &str = "control_channel_enabled";

/// `settings.toml` key optionally fixing the loopback port (`0` = OS ephemeral port).
pub(crate) const CONTROL_CHANNEL_PORT: &str = "control_channel_port";

/// Action queued by an external control client for execution on the UI thread.
#[derive(Debug, Clone)]
pub(crate) enum PendingControlAction {
    OpenDocument(PathBuf),
    JumpToPage(usize),
    RunCommand(String),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ControlStatusSnapshot {
    pub(crate) running: bool,
    pub(crate) endpoint: Option<String>,
    pub(crate) token_file: Option<PathBuf>,
    pub(crate) error: Option<String>,
}

#[derive(Default)]
struct SharedControlState {
    queue: VecDeque<PendingControlAction>,
    active_document: Option<String>,
    current_page: usize,
    status: ControlStatusSnapshot,
}

static STATE: OnceLock<Mutex<SharedControlState>> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();

fn shared_state() -> &'static Mutex<SharedControlState> {
    STATE.get_or_init(|| Mutex::new(SharedControlState::default()))
}

struct AppControlHandler;

impl ControlHandler for AppControlHandler {
    fn handle(&mut self, command: ControlCommand) -> Result<Value, ControlError> {
        let mut guard = shared_state().lock().expect("control state lock poisoned");
        match command {
            ControlCommand::Ping => Ok(json!({ "pong": true })),
            ControlCommand::OpenDocument { path } => {
                guard
                    .queue
                    .push_back(PendingControlAction::OpenDocument(PathBuf::from(&path)));
                guard.active_document = Some(path.clone());
                Ok(json!({ "queued": true, "active_document": path }))
            }
            ControlCommand::JumpToPage { page } => {
                guard
                    .queue
                    .push_back(PendingControlAction::JumpToPage(page));
                guard.current_page = page;
                Ok(json!({ "queued": true, "current_page": page }))
            }
            ControlCommand::RunAction { action, .. } => {
                if !COMMANDS.iter().any(|c| c.name == action) {
                    return Err(ControlError::new(
                        ControlErrorCode::ExecutionFailed,
                        format!("unknown action '{action}'"),
                    ));
                }
                guard
                    .queue
                    .push_back(PendingControlAction::RunCommand(action.clone()));
                Ok(json!({ "queued": true, "action": action }))
            }
            ControlCommand::RunTool { name, .. } => Err(ControlError::new(
                ControlErrorCode::ExecutionFailed,
                format!("use excellent-cli run {name} for headless document tools"),
            )),
            ControlCommand::InspectState => {
                let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
                Ok(json!({
                    "active_document": guard.active_document,
                    "current_page": guard.current_page,
                    "registered_actions_count": names.len(),
                }))
            }
            ControlCommand::Screenshot { output, page, dpi } => Ok(json!({
                "captured": false,
                "reason": "use excellent-cli render for deterministic headless page rendering",
                "output": output,
                "page": page.unwrap_or(guard.current_page),
                "dpi": dpi.unwrap_or(150),
            })),
        }
    }
}

/// Starts the loopback UI control channel once per process if enabled in settings (E.4).
pub(crate) fn ensure_started() {
    STARTED.get_or_init(|| {
        let enabled = settings::shared()
            .flag(CONTROL_CHANNEL_ENABLED)
            .unwrap_or(true);
        if !enabled {
            let mut guard = shared_state().lock().expect("control state lock poisoned");
            guard.status.running = false;
            return;
        }
        let port = settings::shared()
            .number(CONTROL_CHANNEL_PORT)
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 65535.0)
            .map_or(0, |n| n as u16);
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let token = ControlToken::generate(ControlCapability::all());
        match LoopbackControlServer::bind(addr, vec![token.clone()], FrameCodec::LengthPrefixed) {
            Ok(server) => {
                let bound = server.addr();
                let endpoint = format!("tcp://{bound}");
                let token_file = settings::folder().map(|dir| dir.join("control.token"));
                if let Some(ref path) = token_file {
                    if let Some(parent) = path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let payload = json!({
                        "endpoint": endpoint,
                        "token": token.as_str(),
                        "codec": "length-prefixed"
                    });
                    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
                        let _ = write_atomically(path, &bytes);
                    }
                }
                {
                    let mut guard = shared_state().lock().expect("control state lock poisoned");
                    guard.status = ControlStatusSnapshot {
                        running: true,
                        endpoint: Some(endpoint),
                        token_file,
                        error: None,
                    };
                }
                let _ = std::thread::Builder::new()
                    .name("control-channel".to_owned())
                    .spawn(move || {
                        let mut handler = AppControlHandler;
                        if let Err(err) = server.serve(&mut handler, None) {
                            log::warn!("control channel stopped: {err}");
                        }
                    });
            }
            Err(err) => {
                let mut guard = shared_state().lock().expect("control state lock poisoned");
                guard.status = ControlStatusSnapshot {
                    running: false,
                    endpoint: None,
                    token_file: None,
                    error: Some(err.to_string()),
                };
            }
        }
    });
}

/// Records the active pane's current file and page so `InspectState` stays up to date.
pub(crate) fn update_state(active_document: Option<PathBuf>, current_page: usize) {
    if let Ok(mut guard) = shared_state().try_lock() {
        guard.active_document = active_document.map(|p| p.to_string_lossy().into_owned());
        guard.current_page = current_page;
    }
}

/// Drains queued control commands so the UI shell can dispatch them on its main thread.
pub(crate) fn drain_pending() -> Vec<PendingControlAction> {
    let Ok(mut guard) = shared_state().try_lock() else {
        return Vec::new();
    };
    guard.queue.drain(..).collect()
}

/// Current control channel status for settings & CLI/MCP docs dialogs.
pub(crate) fn status_snapshot() -> ControlStatusSnapshot {
    shared_state()
        .lock()
        .expect("control state lock poisoned")
        .status
        .clone()
}
