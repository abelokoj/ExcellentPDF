//! UI control channel protocol and transport layer (plan E.4).
//!
//! Provides a capability- and token-gated local control channel (NDJSON or length-prefixed framing)
//! over loopback TCP, Unix domain sockets, Windows named pipes, or mockable in-memory transports
//! so scripts, agents, and UI tests can open documents, jump to pages, run registered app actions,
//! inspect state, and capture screenshots.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::hash::{BuildHasher as _, Hasher as _};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Maximum control frame size (4 MiB).
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Capabilities granted to a control token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ControlCapability {
    /// Open or switch documents.
    OpenDocument,
    /// Navigate pages / scroll position.
    Navigate,
    /// Execute registered UI command-table actions.
    ExecuteAction,
    /// Invoke tools from [`crate::tools::ToolRegistry`].
    RunTool,
    /// Read current open documents, page, and UI state.
    InspectState,
    /// Capture a rendered screenshot or page preview.
    Screenshot,
}

impl ControlCapability {
    /// Full set of capabilities for a local owner session.
    #[must_use]
    pub fn all() -> BTreeSet<Self> {
        BTreeSet::from([
            Self::OpenDocument,
            Self::Navigate,
            Self::ExecuteAction,
            Self::RunTool,
            Self::InspectState,
            Self::Screenshot,
        ])
    }

    /// Stable wire name of this capability.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenDocument => "open_document",
            Self::Navigate => "navigate",
            Self::ExecuteAction => "execute_action",
            Self::RunTool => "run_tool",
            Self::InspectState => "inspect_state",
            Self::Screenshot => "screenshot",
        }
    }
}

/// Wire framing format for control channel messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FrameCodec {
    /// Newline-delimited UTF-8 JSON objects.
    #[default]
    Ndjson,
    /// 4-byte big-endian length prefix followed by UTF-8 JSON payload.
    LengthPrefixed,
}

/// Writes a single frame to `writer` according to `codec`.
///
/// # Errors
///
/// Returns [`io::Error`] if `payload` exceeds [`MAX_FRAME_BYTES`] or writing fails.
pub fn write_frame<W: Write>(writer: &mut W, codec: FrameCodec, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control frame exceeds MAX_FRAME_BYTES",
        ));
    }
    match codec {
        FrameCodec::Ndjson => {
            writer.write_all(payload)?;
            writer.write_all(b"\n")?;
        }
        FrameCodec::LengthPrefixed => {
            let len = u32::try_from(payload.len())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
            writer.write_all(&len.to_be_bytes())?;
            writer.write_all(payload)?;
        }
    }
    writer.flush()
}

/// Reads one frame from `reader` according to `codec`; `Ok(None)` on clean EOF.
///
/// # Errors
///
/// Returns [`io::Error`] on malformed frames, oversized payloads, or I/O failure.
pub fn read_frame<R: BufRead>(reader: &mut R, codec: FrameCodec) -> io::Result<Option<Vec<u8>>> {
    match codec {
        FrameCodec::Ndjson => {
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    return Ok(None);
                }
                if line.len() > MAX_FRAME_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ndjson frame exceeds MAX_FRAME_BYTES",
                    ));
                }
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.as_bytes().to_vec()));
                }
            }
        }
        FrameCodec::LengthPrefixed => {
            let mut hdr = [0u8; 4];
            match reader.read_exact(&mut hdr) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(err) => return Err(err),
            }
            let len = u32::from_be_bytes(hdr) as usize;
            if len > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "length-prefixed frame exceeds MAX_FRAME_BYTES",
                ));
            }
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;
            Ok(Some(buf))
        }
    }
}

/// Random capability authorization token persisted in a user-readable file.
#[derive(Clone, PartialEq, Eq)]
pub struct ControlToken {
    secret: String,
    capabilities: BTreeSet<ControlCapability>,
}

impl fmt::Debug for ControlToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlToken")
            .field("secret", &"[REDACTED]")
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

impl ControlToken {
    /// Generates a fresh 128-bit random token granting `capabilities`.
    #[must_use]
    pub fn generate(capabilities: BTreeSet<ControlCapability>) -> Self {
        let mut h1 = std::collections::hash_map::RandomState::new().build_hasher();
        let mut h2 = std::collections::hash_map::RandomState::new().build_hasher();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        h1.write_u128(nanos);
        h1.write_u32(std::process::id());
        h2.write_u64(h1.finish());
        h2.write_usize((&capabilities as *const BTreeSet<ControlCapability>) as usize);
        let secret = format!("{:016x}{:016x}", h1.finish(), h2.finish());
        Self {
            secret,
            capabilities,
        }
    }

    /// Wraps an explicit token string (primarily for loading from token file or tests).
    #[must_use]
    pub fn from_secret(
        secret: impl Into<String>,
        capabilities: BTreeSet<ControlCapability>,
    ) -> Self {
        Self {
            secret: secret.into(),
            capabilities,
        }
    }

    /// Raw secret token string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.secret
    }

    /// Capabilities held by this token.
    #[must_use]
    pub fn capabilities(&self) -> &BTreeSet<ControlCapability> {
        &self.capabilities
    }

    /// Constant-time check that `candidate` matches this token and holds `required`.
    pub fn authorize(
        &self,
        candidate: &str,
        required: Option<ControlCapability>,
    ) -> Result<(), ControlErrorCode> {
        if !constant_time_eq(self.secret.as_bytes(), candidate.as_bytes()) {
            return Err(ControlErrorCode::Unauthorized);
        }
        if let Some(cap) = required
            && !self.capabilities.contains(&cap)
        {
            return Err(ControlErrorCode::Forbidden);
        }
        Ok(())
    }

    /// Writes `{ "endpoint": ..., "token": ..., "pid": ... }` to `path` with user-only permissions where supported.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if the file cannot be created or written.
    pub fn write_token_file(&self, path: &Path, endpoint: &str) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let body = json!({
            "endpoint": endpoint,
            "token": self.secret,
            "pid": std::process::id(),
            "capabilities": self.capabilities.iter().map(|c| c.name()).collect::<Vec<_>>(),
        })
        .to_string();

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(body.as_bytes())?;
            file.flush()?;
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            fs::write(path, body.as_bytes())
        }
    }

    /// Reads `{ endpoint, token }` from a token file written by [`Self::write_token_file`].
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if reading or parsing fails.
    pub fn read_token_file(path: &Path) -> io::Result<(String, String)> {
        let raw = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&raw)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let endpoint = val
            .get("endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing endpoint"))?
            .to_owned();
        let token = val
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing token"))?
            .to_owned();
        Ok((endpoint, token))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Machine-readable error codes returned by [`ControlResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlErrorCode {
    /// Missing or invalid authorization token.
    Unauthorized,
    /// Token is valid but lacks the capability required by the command.
    Forbidden,
    /// Malformed JSON or unknown command.
    InvalidRequest,
    /// Requested registered action does not exist.
    UnknownAction,
    /// Command execution failed.
    ExecutionFailed,
}

impl ControlErrorCode {
    /// Stable wire string for this error code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::InvalidRequest => "invalid_request",
            Self::UnknownAction => "unknown_action",
            Self::ExecutionFailed => "execution_failed",
        }
    }
}

/// Error returned by a [`ControlHandler`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ControlError {
    /// Error classification.
    pub code: ControlErrorCode,
    /// Human-readable reason.
    pub message: String,
}

impl ControlError {
    /// Constructs a new control error.
    pub fn new(code: ControlErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ControlError {}

/// Commands supported by the UI control channel protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    /// Liveness check.
    Ping,
    /// Open a document in the controlled instance.
    OpenDocument {
        /// File path to open.
        path: String,
    },
    /// Jump to a zero-based page index in the active document.
    JumpToPage {
        /// Zero-based page index.
        page: usize,
    },
    /// Execute a registered UI command-table action by id.
    RunAction {
        /// Registered action name (e.g. `"view.zoom_in"`, `"annot.highlight"`).
        action: String,
        /// Optional JSON arguments for the action.
        args: Value,
    },
    /// Invoke a tool from [`crate::tools::ToolRegistry`].
    RunTool {
        /// Dotted tool name.
        name: String,
        /// Tool JSON arguments.
        arguments: Value,
    },
    /// Inspect current app state (open files, active page, registered actions).
    InspectState,
    /// Capture a screenshot or page render.
    Screenshot {
        /// Optional target file path for PNG output.
        output: Option<String>,
        /// Optional zero-based page override.
        page: Option<usize>,
        /// Optional DPI override.
        dpi: Option<u32>,
    },
}

impl ControlCommand {
    /// Capability required to execute this command, or `None` for `Ping`.
    #[must_use]
    pub const fn required_capability(&self) -> Option<ControlCapability> {
        match self {
            Self::Ping => None,
            Self::OpenDocument { .. } => Some(ControlCapability::OpenDocument),
            Self::JumpToPage { .. } => Some(ControlCapability::Navigate),
            Self::RunAction { .. } => Some(ControlCapability::ExecuteAction),
            Self::RunTool { .. } => Some(ControlCapability::RunTool),
            Self::InspectState => Some(ControlCapability::InspectState),
            Self::Screenshot { .. } => Some(ControlCapability::Screenshot),
        }
    }

    /// Serializes this command to its wire JSON representation.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Ping => json!({ "cmd": "ping" }),
            Self::OpenDocument { path } => json!({ "cmd": "open_document", "path": path }),
            Self::JumpToPage { page } => json!({ "cmd": "jump_to_page", "page": page }),
            Self::RunAction { action, args } => {
                json!({ "cmd": "run_action", "action": action, "args": args })
            }
            Self::RunTool { name, arguments } => {
                json!({ "cmd": "run_tool", "name": name, "arguments": arguments })
            }
            Self::InspectState => json!({ "cmd": "inspect_state" }),
            Self::Screenshot { output, page, dpi } => json!({
                "cmd": "screenshot",
                "output": output,
                "page": page,
                "dpi": dpi,
            }),
        }
    }

    /// Parses a command from a JSON object.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError`] with [`ControlErrorCode::InvalidRequest`] if parsing fails.
    pub fn from_json(value: &Value) -> Result<Self, ControlError> {
        let obj = value.as_object().ok_or_else(|| {
            ControlError::new(
                ControlErrorCode::InvalidRequest,
                "command must be a JSON object",
            )
        })?;
        let cmd = obj.get("cmd").and_then(Value::as_str).ok_or_else(|| {
            ControlError::new(ControlErrorCode::InvalidRequest, "missing 'cmd' string")
        })?;
        match cmd {
            "ping" => Ok(Self::Ping),
            "open_document" => {
                let path = obj
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        ControlError::new(
                            ControlErrorCode::InvalidRequest,
                            "open_document requires non-empty 'path'",
                        )
                    })?;
                Ok(Self::OpenDocument {
                    path: path.to_owned(),
                })
            }
            "jump_to_page" => {
                let page = obj
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| {
                        ControlError::new(
                            ControlErrorCode::InvalidRequest,
                            "jump_to_page requires integer 'page'",
                        )
                    })?;
                Ok(Self::JumpToPage { page })
            }
            "run_action" => {
                let action = obj
                    .get("action")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        ControlError::new(
                            ControlErrorCode::InvalidRequest,
                            "run_action requires non-empty 'action'",
                        )
                    })?;
                Ok(Self::RunAction {
                    action: action.to_owned(),
                    args: obj.get("args").cloned().unwrap_or(Value::Null),
                })
            }
            "run_tool" => {
                let name = obj.get("name").and_then(Value::as_str).ok_or_else(|| {
                    ControlError::new(ControlErrorCode::InvalidRequest, "run_tool requires 'name'")
                })?;
                Ok(Self::RunTool {
                    name: name.to_owned(),
                    arguments: obj.get("arguments").cloned().unwrap_or_else(|| json!({})),
                })
            }
            "inspect_state" => Ok(Self::InspectState),
            "screenshot" => Ok(Self::Screenshot {
                output: obj.get("output").and_then(Value::as_str).map(str::to_owned),
                page: obj
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok()),
                dpi: obj
                    .get("dpi")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
            }),
            other => Err(ControlError::new(
                ControlErrorCode::InvalidRequest,
                format!("unknown command '{other}'"),
            )),
        }
    }
}

/// Authenticated control request envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlRequest {
    /// Correlation id echoed in [`ControlResponse`].
    pub id: u64,
    /// Capability authorization token.
    pub token: String,
    /// Command to execute.
    pub command: ControlCommand,
}

impl ControlRequest {
    /// Serializes this request as a JSON value.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = match self.command.to_json() {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        obj.insert("id".to_owned(), json!(self.id));
        obj.insert("token".to_owned(), json!(self.token));
        Value::Object(obj)
    }

    /// Parses a request envelope from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError`] on malformed JSON or invalid command fields.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ControlError> {
        let value: Value = serde_json::from_slice(bytes).map_err(|err| {
            ControlError::new(
                ControlErrorCode::InvalidRequest,
                format!("invalid JSON: {err}"),
            )
        })?;
        let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
        let token = value
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let command = ControlCommand::from_json(&value)?;
        Ok(Self { id, token, command })
    }
}

/// Response envelope returned by the control server.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlResponse {
    /// Correlation id from [`ControlRequest`].
    pub id: u64,
    /// Whether authorization and execution succeeded.
    pub ok: bool,
    /// Result payload when `ok` is `true`.
    pub result: Value,
    /// Error details when `ok` is `false`.
    pub error: Option<ControlError>,
}

impl ControlResponse {
    /// Builds a success response.
    #[must_use]
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result,
            error: None,
        }
    }

    /// Builds a failure response.
    #[must_use]
    pub fn err(id: u64, error: ControlError) -> Self {
        Self {
            id,
            ok: false,
            result: Value::Null,
            error: Some(error),
        }
    }

    /// Serializes this response to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "ok": self.ok,
            "result": self.result,
            "error": self.error.as_ref().map(|e| json!({
                "code": e.code.as_str(),
                "message": e.message,
            })),
        })
    }

    /// Parses a response from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if the JSON is malformed.
    pub fn from_slice(bytes: &[u8]) -> io::Result<Self> {
        let val: Value = serde_json::from_slice(bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let id = val.get("id").and_then(Value::as_u64).unwrap_or(0);
        let ok = val.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let result = val.get("result").cloned().unwrap_or(Value::Null);
        let error = val.get("error").and_then(Value::as_object).map(|e| {
            let code = match e.get("code").and_then(Value::as_str) {
                Some("unauthorized") => ControlErrorCode::Unauthorized,
                Some("forbidden") => ControlErrorCode::Forbidden,
                Some("unknown_action") => ControlErrorCode::UnknownAction,
                Some("execution_failed") => ControlErrorCode::ExecutionFailed,
                _ => ControlErrorCode::InvalidRequest,
            };
            let message = e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("control error")
                .to_owned();
            ControlError::new(code, message)
        });
        Ok(Self {
            id,
            ok,
            result,
            error,
        })
    }
}

/// Application state backend invoked by an authenticated control channel server.
pub trait ControlHandler: Send {
    /// Handles a validated [`ControlCommand`].
    ///
    /// # Errors
    ///
    /// Returns [`ControlError`] if the command fails.
    fn handle(&mut self, command: ControlCommand) -> Result<Value, ControlError>;
}

/// Processes a single framed request byte slice against `tokens` and `handler`.
#[must_use]
pub fn dispatch_bytes(
    raw: &[u8],
    tokens: &[ControlToken],
    handler: &mut dyn ControlHandler,
) -> Vec<u8> {
    let req = match ControlRequest::from_slice(raw) {
        Ok(req) => req,
        Err(err) => {
            return ControlResponse::err(0, err)
                .to_json()
                .to_string()
                .into_bytes();
        }
    };
    let required_cap = req.command.required_capability();
    let mut auth_err = ControlErrorCode::Unauthorized;
    let mut authorized = false;
    for tok in tokens {
        match tok.authorize(&req.token, required_cap) {
            Ok(()) => {
                authorized = true;
                break;
            }
            Err(ControlErrorCode::Forbidden) => {
                auth_err = ControlErrorCode::Forbidden;
            }
            Err(ControlErrorCode::Unauthorized) => {}
            Err(other) => auth_err = other,
        }
    }
    if !authorized {
        let msg = match auth_err {
            ControlErrorCode::Forbidden => "token lacks required capability",
            _ => "invalid or missing control token",
        };
        return ControlResponse::err(req.id, ControlError::new(auth_err, msg))
            .to_json()
            .to_string()
            .into_bytes();
    }

    let resp = match handler.handle(req.command) {
        Ok(result) => ControlResponse::ok(req.id, result),
        Err(err) => ControlResponse::err(req.id, err),
    };
    resp.to_json().to_string().into_bytes()
}

/// Mockable request/response transport for unit tests and headless automation.
pub trait ControlTransport {
    /// Sends one JSON command frame and returns the raw response JSON frame.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if transport delivery fails.
    fn exchange(&mut self, request_bytes: &[u8]) -> io::Result<Vec<u8>>;
}

impl<T: ControlTransport + ?Sized> ControlTransport for &mut T {
    fn exchange(&mut self, request_bytes: &[u8]) -> io::Result<Vec<u8>> {
        (**self).exchange(request_bytes)
    }
}

/// In-memory mock transport directly invoking [`dispatch_bytes`] over a shared handler.
pub struct MockControlTransport<H: ControlHandler> {
    tokens: Vec<ControlToken>,
    handler: Arc<Mutex<H>>,
}

impl<H: ControlHandler> fmt::Debug for MockControlTransport<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockControlTransport")
            .field("tokens", &self.tokens.len())
            .finish()
    }
}

impl<H: ControlHandler> MockControlTransport<H> {
    /// Creates an in-memory transport wired to `tokens` and `handler`.
    #[must_use]
    pub fn new(tokens: Vec<ControlToken>, handler: H) -> Self {
        Self {
            tokens,
            handler: Arc::new(Mutex::new(handler)),
        }
    }
}

impl<H: ControlHandler> ControlTransport for MockControlTransport<H> {
    fn exchange(&mut self, request_bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut guard = self.handler.lock().expect("control handler lock poisoned");
        Ok(dispatch_bytes(request_bytes, &self.tokens, &mut *guard))
    }
}

/// Stateful in-memory UI control host for tests and headless verification.
#[derive(Debug)]
pub struct MockAppControlHost {
    open_documents: Vec<String>,
    active_document: Option<String>,
    current_page: usize,
    zoom: f32,
    registered_actions: HashMap<String, String>,
    executed_actions: Vec<(String, Value)>,
}

impl Default for MockAppControlHost {
    fn default() -> Self {
        let mut actions = HashMap::new();
        actions.insert("view.zoom_in".to_owned(), "Zoom in".to_owned());
        actions.insert("view.zoom_out".to_owned(), "Zoom out".to_owned());
        actions.insert("view.next_page".to_owned(), "Next page".to_owned());
        actions.insert("view.prev_page".to_owned(), "Previous page".to_owned());
        actions.insert("file.save".to_owned(), "Save document".to_owned());
        Self {
            open_documents: Vec::new(),
            active_document: None,
            current_page: 0,
            zoom: 1.0,
            registered_actions: actions,
            executed_actions: Vec::new(),
        }
    }
}

impl MockAppControlHost {
    /// Registers an additional app action id.
    pub fn register_action(&mut self, id: impl Into<String>, label: impl Into<String>) {
        self.registered_actions.insert(id.into(), label.into());
    }
}

impl ControlHandler for MockAppControlHost {
    fn handle(&mut self, command: ControlCommand) -> Result<Value, ControlError> {
        match command {
            ControlCommand::Ping => Ok(json!({ "ok": true, "protocol": "excellent-control/1" })),
            ControlCommand::OpenDocument { path } => {
                if !self.open_documents.contains(&path) {
                    self.open_documents.push(path.clone());
                }
                self.active_document = Some(path.clone());
                self.current_page = 0;
                Ok(json!({
                    "active_document": path,
                    "open_documents": self.open_documents,
                    "current_page": self.current_page,
                }))
            }
            ControlCommand::JumpToPage { page } => {
                if self.active_document.is_none() {
                    return Err(ControlError::new(
                        ControlErrorCode::ExecutionFailed,
                        "no active document",
                    ));
                }
                self.current_page = page;
                Ok(json!({
                    "active_document": self.active_document,
                    "current_page": self.current_page,
                }))
            }
            ControlCommand::RunAction { action, args } => {
                if !self.registered_actions.contains_key(&action) {
                    return Err(ControlError::new(
                        ControlErrorCode::UnknownAction,
                        format!("unregistered action '{action}'"),
                    ));
                }
                match action.as_str() {
                    "view.zoom_in" => self.zoom = (self.zoom * 1.25).min(8.0),
                    "view.zoom_out" => self.zoom = (self.zoom / 1.25).max(0.25),
                    "view.next_page" => self.current_page += 1,
                    "view.prev_page" => {
                        self.current_page = self.current_page.saturating_sub(1);
                    }
                    _ => {}
                }
                self.executed_actions.push((action.clone(), args.clone()));
                Ok(json!({
                    "action": action,
                    "zoom": self.zoom,
                    "current_page": self.current_page,
                }))
            }
            ControlCommand::RunTool { name, .. } => Err(ControlError::new(
                ControlErrorCode::ExecutionFailed,
                format!("mock host does not execute tool '{name}'"),
            )),
            ControlCommand::InspectState => {
                let mut action_names: Vec<&str> =
                    self.registered_actions.keys().map(String::as_str).collect();
                action_names.sort_unstable();
                Ok(json!({
                    "open_documents": self.open_documents,
                    "active_document": self.active_document,
                    "current_page": self.current_page,
                    "zoom": self.zoom,
                    "registered_actions": action_names,
                    "executed_actions_count": self.executed_actions.len(),
                }))
            }
            ControlCommand::Screenshot { output, page, dpi } => Ok(json!({
                "captured": true,
                "page": page.unwrap_or(self.current_page),
                "dpi": dpi.unwrap_or(150),
                "output": output,
            })),
        }
    }
}

/// Real document-backed control host combining [`super::registry::ToolRegistry`] and app state.
#[derive(Debug)]
pub struct DocumentControlHost {
    state: MockAppControlHost,
    tools: super::registry::ToolRegistry,
    ctx: super::registry::ToolContext,
}

impl DocumentControlHost {
    /// Creates a real control host backed by `tools` and `ctx`.
    #[must_use]
    pub fn new(tools: super::registry::ToolRegistry, ctx: super::registry::ToolContext) -> Self {
        Self {
            state: MockAppControlHost::default(),
            tools,
            ctx,
        }
    }
}

impl ControlHandler for DocumentControlHost {
    fn handle(&mut self, command: ControlCommand) -> Result<Value, ControlError> {
        match command {
            ControlCommand::OpenDocument { path } => {
                let resolved = self.ctx.resolve_path(&path).map_err(|e| {
                    ControlError::new(ControlErrorCode::ExecutionFailed, e.to_string())
                })?;
                let doc = self.ctx.registry().open(&resolved).map_err(|e| {
                    ControlError::new(ControlErrorCode::ExecutionFailed, e.to_string())
                })?;
                let pages = doc
                    .mupdf()
                    .and_then(|m| Ok(m.lock().expect("mupdf lock poisoned").page_count()?))
                    .map_err(|e| {
                        ControlError::new(ControlErrorCode::ExecutionFailed, e.to_string())
                    })?;
                let res = self.state.handle(ControlCommand::OpenDocument {
                    path: resolved.to_string_lossy().into_owned(),
                })?;
                Ok(json!({
                    "active_document": res["active_document"],
                    "open_documents": res["open_documents"],
                    "current_page": 0,
                    "page_count": pages,
                }))
            }
            ControlCommand::RunTool { name, arguments } => self
                .tools
                .execute(&self.ctx, &name, &arguments)
                .map_err(|e| ControlError::new(ControlErrorCode::ExecutionFailed, e.to_string())),
            ControlCommand::Screenshot { output, page, dpi } => {
                let Some(active) = self.state.active_document.clone() else {
                    return Err(ControlError::new(
                        ControlErrorCode::ExecutionFailed,
                        "no active document to screenshot",
                    ));
                };
                let mut args = json!({
                    "path": active,
                    "page": page.unwrap_or(self.state.current_page),
                    "dpi": dpi.unwrap_or(150),
                });
                if let Some(out) = output
                    && let Some(map) = args.as_object_mut()
                {
                    map.insert("output".to_owned(), json!(out));
                }
                self.tools
                    .execute(&self.ctx, "page.render", &args)
                    .map_err(|e| {
                        ControlError::new(ControlErrorCode::ExecutionFailed, e.to_string())
                    })
            }
            other => self.state.handle(other),
        }
    }
}

/// High-level client speaking [`ControlRequest`] / [`ControlResponse`] over any [`ControlTransport`].
#[derive(Debug)]
pub struct ControlClient<T: ControlTransport> {
    transport: T,
    token: String,
    next_id: u64,
}

impl<T: ControlTransport> ControlClient<T> {
    /// Creates a client using `transport` and capability `token`.
    #[must_use]
    pub fn new(transport: T, token: impl Into<String>) -> Self {
        Self {
            transport,
            token: token.into(),
            next_id: 1,
        }
    }

    /// Sends `command` and returns the parsed [`ControlResponse`].
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] on transport or response decoding failure.
    pub fn send(&mut self, command: ControlCommand) -> io::Result<ControlResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let req = ControlRequest {
            id,
            token: self.token.clone(),
            command,
        };
        let payload = req.to_json().to_string();
        let reply = self.transport.exchange(payload.as_bytes())?;
        ControlResponse::from_slice(&reply)
    }
}

/// Loopback TCP transport connecting to `addr` using `codec`.
#[derive(Debug, Clone)]
pub struct LoopbackTransport {
    addr: SocketAddr,
    codec: FrameCodec,
}

impl LoopbackTransport {
    /// Creates a loopback transport targeting `addr`.
    #[must_use]
    pub fn new(addr: SocketAddr, codec: FrameCodec) -> Self {
        Self { addr, codec }
    }
}

impl ControlTransport for LoopbackTransport {
    fn exchange(&mut self, request_bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut stream = TcpStream::connect(self.addr)?;
        write_frame(&mut stream, self.codec, request_bytes)?;
        let mut reader = BufReader::new(stream);
        read_frame(&mut reader, self.codec)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "empty control response"))
    }
}

/// Unix domain socket transport (`#[cfg(unix)]`).
#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct UnixSocketTransport {
    path: PathBuf,
    codec: FrameCodec,
}

#[cfg(unix)]
impl UnixSocketTransport {
    /// Creates a transport targeting the Unix domain socket at `path`.
    #[must_use]
    pub fn new(path: PathBuf, codec: FrameCodec) -> Self {
        Self { path, codec }
    }
}

#[cfg(unix)]
impl ControlTransport for UnixSocketTransport {
    fn exchange(&mut self, request_bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        write_frame(&mut stream, self.codec, request_bytes)?;
        let mut reader = BufReader::new(stream);
        read_frame(&mut reader, self.codec)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "empty control response"))
    }
}

/// Loopback TCP server accepting control requests on `127.0.0.1:<port>`.
pub struct LoopbackControlServer {
    listener: TcpListener,
    addr: SocketAddr,
    tokens: Vec<ControlToken>,
    codec: FrameCodec,
}

impl fmt::Debug for LoopbackControlServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoopbackControlServer")
            .field("addr", &self.addr)
            .field("codec", &self.codec)
            .finish()
    }
}

impl LoopbackControlServer {
    /// Binds to `127.0.0.1:0` (or an explicit loopback address) with authorized `tokens`.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if binding fails.
    pub fn bind(
        addr: SocketAddr,
        tokens: Vec<ControlToken>,
        codec: FrameCodec,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            listener,
            addr: bound,
            tokens,
            codec,
        })
    }

    /// Actual bound loopback socket address (useful when bound to port `0`).
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serves up to `max_requests` connections (or indefinitely when `None`).
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if accepting a connection fails.
    pub fn serve(
        &self,
        handler: &mut dyn ControlHandler,
        max_requests: Option<usize>,
    ) -> io::Result<()> {
        for (handled, stream) in self.listener.incoming().enumerate() {
            let stream = stream?;
            serve_stream(stream, self.codec, &self.tokens, handler)?;
            if max_requests.is_some_and(|limit| handled + 1 >= limit) {
                break;
            }
        }
        Ok(())
    }
}

/// Serves framed requests over an arbitrary bidirectional stream until EOF.
///
/// # Errors
///
/// Returns [`io::Error`] if reading or writing frames fails.
pub fn serve_stream<S: Read + Write>(
    stream: S,
    codec: FrameCodec,
    tokens: &[ControlToken],
    handler: &mut dyn ControlHandler,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    while let Some(frame) = read_frame(&mut reader, codec)? {
        let reply = dispatch_bytes(&frame, tokens, handler);
        write_frame(reader.get_mut(), codec, &reply)?;
    }
    Ok(())
}

/// Endpoint descriptor parsed from `--endpoint` or a token file (`tcp://127.0.0.1:PORT`, `unix:///path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEndpoint {
    /// Loopback TCP `SocketAddr`.
    Tcp(SocketAddr),
    /// Unix domain socket path.
    Unix(PathBuf),
}

impl ControlEndpoint {
    /// Parses `raw` as `tcp://host:port`, `host:port`, or `unix:///path`.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if the endpoint string is invalid or non-loopback.
    pub fn parse(raw: &str) -> io::Result<Self> {
        if let Some(path) = raw.strip_prefix("unix://") {
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        let host_port = raw.strip_prefix("tcp://").unwrap_or(raw);
        let addr: SocketAddr = host_port
            .parse()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "control endpoint must be loopback",
            ));
        }
        Ok(Self::Tcp(addr))
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn mock_transport_enforces_tokens_and_capabilities() {
        let full_token = ControlToken::generate(ControlCapability::all());
        let readonly_token =
            ControlToken::generate(BTreeSet::from([ControlCapability::InspectState]));

        let mut transport = MockControlTransport::new(
            vec![full_token.clone(), readonly_token.clone()],
            MockAppControlHost::default(),
        );

        // Bad token is rejected as Unauthorized.
        let mut bad_client = ControlClient::new(
            MockControlTransport {
                tokens: vec![full_token.clone()],
                handler: Arc::clone(&transport.handler),
            },
            "wrong-token",
        );
        let err_resp = bad_client
            .send(ControlCommand::InspectState)
            .expect("exchange");
        assert!(!err_resp.ok);
        assert_eq!(
            err_resp.error.as_ref().map(|e| e.code),
            Some(ControlErrorCode::Unauthorized)
        );

        // Read-only token can inspect state, but cannot open documents.
        let mut ro_client = ControlClient::new(
            MockControlTransport {
                tokens: vec![full_token.clone(), readonly_token.clone()],
                handler: Arc::clone(&transport.handler),
            },
            readonly_token.as_str(),
        );
        assert!(
            ro_client
                .send(ControlCommand::InspectState)
                .expect("inspect")
                .ok
        );
        let forbidden = ro_client
            .send(ControlCommand::OpenDocument {
                path: "doc.pdf".into(),
            })
            .expect("open");
        assert!(!forbidden.ok);
        assert_eq!(
            forbidden.error.as_ref().map(|e| e.code),
            Some(ControlErrorCode::Forbidden)
        );

        // Full token can open, jump, run action, and inspect state.
        let mut client = ControlClient::new(&mut transport, full_token.as_str());
        assert!(client.send(ControlCommand::Ping).expect("ping").ok);
        assert!(
            client
                .send(ControlCommand::OpenDocument {
                    path: "notes.pdf".into()
                })
                .expect("open")
                .ok
        );
        assert!(
            client
                .send(ControlCommand::JumpToPage { page: 4 })
                .expect("jump")
                .ok
        );
        assert!(
            client
                .send(ControlCommand::RunAction {
                    action: "view.zoom_in".into(),
                    args: Value::Null
                })
                .expect("action")
                .ok
        );
        let state = client.send(ControlCommand::InspectState).expect("state");
        assert!(state.ok);
        assert_eq!(state.result["active_document"], "notes.pdf");
        assert_eq!(state.result["current_page"], 4);
        assert_eq!(state.result["zoom"], 1.25);
    }

    #[test]
    fn loopback_server_round_trips_both_codecs_and_token_file() {
        for codec in [FrameCodec::Ndjson, FrameCodec::LengthPrefixed] {
            let token = ControlToken::generate(ControlCapability::all());
            let addr: SocketAddr = "127.0.0.1:0".parse().expect("loopback");
            let server =
                LoopbackControlServer::bind(addr, vec![token.clone()], codec).expect("bind");
            let bound = server.addr();

            let token_file = std::env::temp_dir().join(format!(
                "excellent-ctrl-{}-{}.json",
                std::process::id(),
                bound.port()
            ));
            token
                .write_token_file(&token_file, &format!("tcp://{bound}"))
                .expect("write token");
            let (ep, read_tok) = ControlToken::read_token_file(&token_file).expect("read token");
            let _ = fs::remove_file(&token_file);
            assert_eq!(read_tok, token.as_str());
            let ControlEndpoint::Tcp(parsed_addr) = ControlEndpoint::parse(&ep).expect("parse")
            else {
                panic!("expected tcp endpoint");
            };

            let handle = thread::spawn(move || {
                let mut host = MockAppControlHost::default();
                server.serve(&mut host, Some(2)).expect("serve 2");
            });

            let mut client =
                ControlClient::new(LoopbackTransport::new(parsed_addr, codec), read_tok);
            let open_res = client
                .send(ControlCommand::OpenDocument {
                    path: "book.pdf".into(),
                })
                .expect("open");
            assert!(open_res.ok);
            let state_res = client.send(ControlCommand::InspectState).expect("state");
            assert_eq!(state_res.result["active_document"], "book.pdf");

            handle.join().expect("server thread finished");
        }
    }
}
