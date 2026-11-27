//! Per-tab sandboxed PDF worker host, watchdog timeout, memory cap, and crash-restart
//! isolation (plan K.22).

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ecclesia_mupdf::{Background, DeviceRect, Document, Matrix};

use super::ipc::{
    IpcCodecError, IpcRequest, IpcResponse, read_request, read_response, write_request,
    write_response,
};
use super::limits::{LimitViolation, SecurityLimits, open_pdf_with_limits};
use super::os::{OsSandboxConfig, OsSandboxError, OsSandboxGuard};

/// Opaque tab identifier isolating each open tab into its own worker slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Policy controlling automatic restart and operation-log replay when a tab worker crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RestartPolicy {
    /// Restart the crashed tab's worker, reopen its document, and replay pending operations.
    #[default]
    RestartWithReplay,
    /// Surface the worker crash directly without automatic restart.
    FailFast,
}

/// Combined configuration for [`SandboxedWorkerPool`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// Central parser and worker resource caps (K.23).
    pub limits: SecurityLimits,
    /// OS-level job/token sandbox settings (K.22).
    pub os: OsSandboxConfig,
    /// Maximum automatic restarts allowed per tab before quarantining that tab.
    pub max_restarts_per_tab: u32,
    /// Policy applied when a worker crashes or is killed by the watchdog.
    pub restart_policy: RestartPolicy,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let limits = SecurityLimits::default();
        let os = OsSandboxConfig {
            memory_cap_bytes: limits.worker_memory_cap_bytes,
            ..OsSandboxConfig::default()
        };
        Self {
            limits,
            os,
            max_restarts_per_tab: 3,
            restart_policy: RestartPolicy::RestartWithReplay,
        }
    }
}

impl SandboxConfig {
    /// Returns the default [`SandboxConfig`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Health status reported for an individual tab's worker slot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkerHealth {
    /// Worker is live and serving requests.
    Running {
        /// Restart generation (0 for initial spawn).
        generation: u32,
        /// OS process id if backed by a subprocess.
        pid: Option<u32>,
    },
    /// Worker crashed or timed out and was replaced by a fresh generation.
    Restarted {
        /// New active generation after restart.
        generation: u32,
        /// Cumulative crashes observed for this tab.
        crash_count: u32,
    },
    /// Tab exceeded [`SandboxConfig::max_restarts_per_tab`] and was isolated without touching
    /// other tabs.
    Quarantined {
        /// Total crashes before quarantine.
        crash_count: u32,
        /// Reason for the final failure.
        reason: String,
    },
}

/// Error returned by the sandboxed worker pool and worker transports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub enum SandboxError {
    /// Untrusted document or frame breached [`SecurityLimits`].
    LimitViolation(LimitViolation),
    /// OS-level sandbox setup or attachment failed.
    Os(OsSandboxError),
    /// IPC framing or serialization failed (including unexpected worker exit).
    Ipc(IpcCodecError),
    /// Worker failed to answer within the wall-clock watchdog timeout.
    TimedOut {
        /// Configured timeout limit.
        timeout: Duration,
    },
    /// Worker exceeded its configured memory cap.
    MemoryCapExceeded {
        /// Observed memory bytes.
        used_bytes: u64,
        /// Configured cap in bytes.
        cap_bytes: u64,
    },
    /// Worker crashed (`reason`) and could not be recovered.
    WorkerCrashed {
        /// Crash diagnostic.
        reason: String,
    },
    /// Tab was quarantined after repeated worker crashes.
    WorkerQuarantined {
        /// Offending tab.
        tab: TabId,
        /// Cumulative crash count.
        crash_count: u32,
        /// Underlying failure message.
        reason: String,
    },
    /// MuPDF returned an engine error.
    Mupdf(String),
    /// Process spawn or I/O failed.
    Io(io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitViolation(err) => write!(f, "security limit violated: {err}"),
            Self::Os(err) => write!(f, "os sandbox error: {err}"),
            Self::Ipc(err) => write!(f, "worker ipc error: {err}"),
            Self::TimedOut { timeout } => {
                write!(f, "worker exceeded wall-clock timeout ({timeout:?})")
            }
            Self::MemoryCapExceeded {
                used_bytes,
                cap_bytes,
            } => write!(
                f,
                "worker memory usage {used_bytes} bytes exceeds cap {cap_bytes}"
            ),
            Self::WorkerCrashed { reason } => write!(f, "worker crashed: {reason}"),
            Self::WorkerQuarantined {
                tab,
                crash_count,
                reason,
            } => write!(
                f,
                "tab {} worker quarantined after {crash_count} crashes: {reason}",
                tab.0
            ),
            Self::Mupdf(msg) => write!(f, "mupdf worker error: {msg}"),
            Self::Io(err) => write!(f, "worker i/o error: {err}"),
        }
    }
}

impl std::error::Error for SandboxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LimitViolation(err) => Some(err),
            Self::Os(err) => Some(err),
            Self::Ipc(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::TimedOut { .. }
            | Self::MemoryCapExceeded { .. }
            | Self::WorkerCrashed { .. }
            | Self::WorkerQuarantined { .. }
            | Self::Mupdf(_) => None,
        }
    }
}

impl From<LimitViolation> for SandboxError {
    fn from(err: LimitViolation) -> Self {
        Self::LimitViolation(err)
    }
}

impl From<OsSandboxError> for SandboxError {
    fn from(err: OsSandboxError) -> Self {
        Self::Os(err)
    }
}

impl From<IpcCodecError> for SandboxError {
    fn from(err: IpcCodecError) -> Self {
        Self::Ipc(err)
    }
}

impl From<io::Error> for SandboxError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Transport channel connected to one isolated worker instance.
pub trait WorkerTransport: Send {
    /// Sends `request` and waits up to `timeout` for a single [`IpcResponse`].
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::TimedOut`], [`SandboxError::WorkerCrashed`], or
    /// [`SandboxError::Ipc`] if the worker hangs, dies, or breaks framing.
    fn round_trip(
        &mut self,
        request: &IpcRequest,
        limits: &SecurityLimits,
        timeout: Duration,
    ) -> Result<IpcResponse, SandboxError>;

    /// Returns the child process id if this transport wraps an OS subprocess.
    fn worker_pid(&self) -> Option<u32>;

    /// Returns the worker's current or peak observed memory usage in bytes.
    fn observed_memory_bytes(&self) -> u64;

    /// Forcefully terminates the worker immediately (used by watchdog or shutdown).
    fn terminate(&mut self);
}

/// Factory spawning fresh isolated worker transports for tabs.
pub trait WorkerSpawner: Send + Sync {
    /// Spawns one worker transport configured with `config`.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] if the worker cannot be started or sandboxed.
    fn spawn(&self, config: &SandboxConfig) -> Result<Box<dyn WorkerTransport>, SandboxError>;
}

/// In-process MuPDF worker state that executes [`IpcRequest`]s identically to an out-of-process
/// worker, running requests on a dedicated worker thread guarded by a wall-clock timeout.
pub struct InProcessWorker {
    docs: HashMap<u64, Document>,
    replayed_ops: HashMap<u64, Vec<String>>,
    approx_bytes: u64,
}

impl InProcessWorker {
    /// Creates an empty in-process worker engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            replayed_ops: HashMap::new(),
            approx_bytes: 0,
        }
    }

    /// Processes a single [`IpcRequest`] and returns the corresponding [`IpcResponse`].
    #[must_use]
    pub fn handle(&mut self, request: IpcRequest) -> IpcResponse {
        match request {
            IpcRequest::Ping { sequence } => IpcResponse::Pong {
                sequence,
                worker_pid: std::process::id(),
            },
            IpcRequest::OpenBytes {
                doc_id,
                mime,
                limits,
                bytes,
            } => {
                if let Err(err) = limits.validate_input_size(bytes.len() as u64) {
                    return IpcResponse::LimitRejected {
                        violation: err.to_string(),
                    };
                }
                let opened = if mime == "application/pdf" {
                    open_pdf_with_limits(&bytes, &limits)
                } else {
                    Document::from_bytes(&bytes, &mime)
                        .map_err(|e| SandboxError::Mupdf(e.to_string()))
                        .and_then(|doc| {
                            let pages = doc
                                .page_count()
                                .map_err(|e| SandboxError::Mupdf(e.to_string()))?;
                            limits.validate_page_count(pages)?;
                            Ok(doc)
                        })
                };
                match opened {
                    Ok(doc) => {
                        let page_count = match doc.page_count() {
                            Ok(c) => c,
                            Err(err) => {
                                return IpcResponse::WorkerError {
                                    message: err.to_string(),
                                };
                            }
                        };
                        self.approx_bytes = self.approx_bytes.saturating_add(bytes.len() as u64);
                        self.docs.insert(doc_id, doc);
                        IpcResponse::Opened { doc_id, page_count }
                    }
                    Err(SandboxError::LimitViolation(err)) => IpcResponse::LimitRejected {
                        violation: err.to_string(),
                    },
                    Err(other) => IpcResponse::WorkerError {
                        message: other.to_string(),
                    },
                }
            }
            IpcRequest::RenderTile {
                doc_id,
                page,
                x0,
                y0,
                x1,
                y1,
                scale_x,
                scale_y,
            } => {
                let Some(doc) = self.docs.get(&doc_id) else {
                    return IpcResponse::WorkerError {
                        message: format!("unknown doc_id {doc_id}"),
                    };
                };
                let loaded = match doc.load_page(page) {
                    Ok(p) => p,
                    Err(err) => {
                        return IpcResponse::WorkerError {
                            message: err.to_string(),
                        };
                    }
                };
                let list = match loaded.display_list(None) {
                    Ok(l) => l,
                    Err(err) => {
                        return IpcResponse::WorkerError {
                            message: err.to_string(),
                        };
                    }
                };
                let area = DeviceRect { x0, y0, x1, y1 };
                let ctm = Matrix::scale(scale_x, scale_y);
                match list.render_area(&ctm, area, Background::White, None) {
                    Ok(pix) => IpcResponse::TileRendered {
                        doc_id,
                        page,
                        width: pix.width(),
                        height: pix.height(),
                        components: u8::try_from(pix.components()).unwrap_or(3),
                        stride: pix.stride(),
                        samples: pix.samples().to_vec(),
                    },
                    Err(err) => IpcResponse::WorkerError {
                        message: err.to_string(),
                    },
                }
            }
            IpcRequest::ExtractText { doc_id, page } => {
                let Some(doc) = self.docs.get(&doc_id) else {
                    return IpcResponse::WorkerError {
                        message: format!("unknown doc_id {doc_id}"),
                    };
                };
                let loaded = match doc.load_page(page) {
                    Ok(p) => p,
                    Err(err) => {
                        return IpcResponse::WorkerError {
                            message: err.to_string(),
                        };
                    }
                };
                match loaded.text(None).and_then(|st| st.text()) {
                    Ok(text) => IpcResponse::TextExtracted { doc_id, page, text },
                    Err(err) => IpcResponse::WorkerError {
                        message: err.to_string(),
                    },
                }
            }
            IpcRequest::ApplyReplayLog {
                doc_id,
                operations_json,
            } => {
                let applied_count = operations_json.len();
                self.replayed_ops
                    .entry(doc_id)
                    .or_default()
                    .extend(operations_json);
                IpcResponse::ReplayApplied {
                    doc_id,
                    applied_count,
                }
            }
            IpcRequest::CloseDocument { doc_id } => {
                self.docs.remove(&doc_id);
                self.replayed_ops.remove(&doc_id);
                IpcResponse::Closed { doc_id }
            }
            IpcRequest::Shutdown => {
                self.docs.clear();
                IpcResponse::ShuttingDown
            }
        }
    }

    /// Returns the number of replayed operations recorded for `doc_id`.
    #[must_use]
    pub fn replayed_operation_count(&self, doc_id: u64) -> usize {
        self.replayed_ops.get(&doc_id).map_or(0, Vec::len)
    }
}

impl Default for InProcessWorker {
    fn default() -> Self {
        Self::new()
    }
}

/// [`WorkerTransport`] running [`InProcessWorker`] on an isolated thread with wall-clock timeout.
pub struct InProcessTransport {
    req_tx: Option<mpsc::Sender<IpcRequest>>,
    resp_rx: mpsc::Receiver<IpcResponse>,
    approx_mem: std::sync::Arc<std::sync::atomic::AtomicU64>,
    alive: bool,
}

impl InProcessTransport {
    /// Spawns a new isolated in-process worker thread.
    #[must_use]
    pub fn start() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<IpcRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<IpcResponse>();
        let approx_mem = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let approx_mem_clone = std::sync::Arc::clone(&approx_mem);

        let _ = thread::Builder::new()
            .name("sandbox-inproc-worker".to_owned())
            .spawn(move || {
                let mut engine = InProcessWorker::new();
                while let Ok(req) = req_rx.recv() {
                    let is_shutdown = matches!(req, IpcRequest::Shutdown);
                    let resp = engine.handle(req);
                    approx_mem_clone
                        .store(engine.approx_bytes, std::sync::atomic::Ordering::Relaxed);
                    if resp_tx.send(resp).is_err() || is_shutdown {
                        break;
                    }
                }
            });

        Self {
            req_tx: Some(req_tx),
            resp_rx,
            approx_mem,
            alive: true,
        }
    }
}

impl WorkerTransport for InProcessTransport {
    fn round_trip(
        &mut self,
        request: &IpcRequest,
        limits: &SecurityLimits,
        timeout: Duration,
    ) -> Result<IpcResponse, SandboxError> {
        if !self.alive {
            return Err(SandboxError::WorkerCrashed {
                reason: "in-process worker already terminated".to_owned(),
            });
        }
        let Some(tx) = &self.req_tx else {
            return Err(SandboxError::WorkerCrashed {
                reason: "worker channel closed".to_owned(),
            });
        };
        if self.observed_memory_bytes() > limits.worker_memory_cap_bytes {
            return Err(SandboxError::MemoryCapExceeded {
                used_bytes: self.observed_memory_bytes(),
                cap_bytes: limits.worker_memory_cap_bytes,
            });
        }
        tx.send(request.clone())
            .map_err(|_| SandboxError::WorkerCrashed {
                reason: "worker thread exited before receiving request".to_owned(),
            })?;
        match self.resp_rx.recv_timeout(timeout) {
            Ok(resp) => Ok(resp),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.terminate();
                Err(SandboxError::TimedOut { timeout })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.alive = false;
                Err(SandboxError::WorkerCrashed {
                    reason: "worker thread disconnected".to_owned(),
                })
            }
        }
    }

    fn worker_pid(&self) -> Option<u32> {
        Some(std::process::id())
    }

    fn observed_memory_bytes(&self) -> u64 {
        self.approx_mem.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn terminate(&mut self) {
        self.alive = false;
        self.req_tx.take();
    }
}

/// [`WorkerSpawner`] producing [`InProcessTransport`] workers (portable fallback & tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct InProcessSpawner;

impl WorkerSpawner for InProcessSpawner {
    fn spawn(&self, _config: &SandboxConfig) -> Result<Box<dyn WorkerTransport>, SandboxError> {
        Ok(Box::new(InProcessTransport::start()))
    }
}

/// Out-of-process [`WorkerTransport`] talking over framed `stdin`/`stdout` to a child executable
/// attached to an [`OsSandboxGuard`].
pub struct SubprocessTransport {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    sandbox_guard: OsSandboxGuard,
}

impl SubprocessTransport {
    /// Spawns `executable` with `args`, attaches the child PID to an [`OsSandboxGuard`], and
    /// prepares framed IPC over `stdin`/`stdout`.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] if process creation or OS sandbox attachment fails.
    pub fn spawn(
        executable: &std::path::Path,
        args: &[String],
        config: &SandboxConfig,
    ) -> Result<Self, SandboxError> {
        let mut guard = OsSandboxGuard::create(config.os.clone())?;
        let mut child = Command::new(executable)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        guard.attach_pid(child.id())?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        Ok(Self {
            child,
            stdin,
            stdout,
            sandbox_guard: guard,
        })
    }
}

impl WorkerTransport for SubprocessTransport {
    fn round_trip(
        &mut self,
        request: &IpcRequest,
        limits: &SecurityLimits,
        timeout: Duration,
    ) -> Result<IpcResponse, SandboxError> {
        let used = self.observed_memory_bytes();
        if used > limits.worker_memory_cap_bytes {
            self.terminate();
            return Err(SandboxError::MemoryCapExceeded {
                used_bytes: used,
                cap_bytes: limits.worker_memory_cap_bytes,
            });
        }
        let Some(mut stdin) = self.stdin.take() else {
            return Err(SandboxError::WorkerCrashed {
                reason: "child stdin closed".to_owned(),
            });
        };
        let Some(mut stdout) = self.stdout.take() else {
            return Err(SandboxError::WorkerCrashed {
                reason: "child stdout closed".to_owned(),
            });
        };

        let req = request.clone();
        let max_frame = limits.max_ipc_frame_bytes;
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("sandbox-ipc-wait".to_owned())
            .spawn(move || {
                let res = write_request(&mut stdin, &req, max_frame)
                    .and_then(|()| read_response(&mut stdout, max_frame));
                let _ = tx.send((stdin, stdout, res));
            })?;

        match rx.recv_timeout(timeout) {
            Ok((stdin_back, stdout_back, res)) => {
                self.stdin = Some(stdin_back);
                self.stdout = Some(stdout_back);
                let _ = handle.join();
                res.map_err(|err| SandboxError::WorkerCrashed {
                    reason: err.to_string(),
                })
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.terminate();
                Err(SandboxError::TimedOut { timeout })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.terminate();
                Err(SandboxError::WorkerCrashed {
                    reason: "IPC waiter disconnected".to_owned(),
                })
            }
        }
    }

    fn worker_pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn observed_memory_bytes(&self) -> u64 {
        self.sandbox_guard.memory_stats().map_or(0, |s| {
            s.peak_job_memory_bytes.max(s.peak_process_memory_bytes)
        })
    }

    fn terminate(&mut self) {
        self.stdin.take();
        self.stdout.take();
        let _ = self.sandbox_guard.terminate_job(1);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SubprocessTransport {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// [`WorkerSpawner`] launching external sandboxed child worker processes.
#[derive(Debug, Clone)]
pub struct SubprocessSpawner {
    /// Path to the worker binary executable.
    pub executable: PathBuf,
    /// Arguments passed to the worker binary to enter stdio worker mode.
    pub args: Vec<String>,
}

impl WorkerSpawner for SubprocessSpawner {
    fn spawn(&self, config: &SandboxConfig) -> Result<Box<dyn WorkerTransport>, SandboxError> {
        let transport = SubprocessTransport::spawn(&self.executable, &self.args, config)?;
        Ok(Box::new(transport))
    }
}

/// Serves [`IpcRequest`]s over `reader` and `writer` until [`IpcRequest::Shutdown`] or EOF.
///
/// Intended as the entry point for an out-of-process `ecclesia-engine-host` worker executable.
///
/// # Errors
///
/// Returns [`SandboxError::Ipc`] if a frame violates framing bounds or JSON syntax.
pub fn run_worker_stdio_loop<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    limits: SecurityLimits,
) -> Result<(), SandboxError> {
    let mut worker = InProcessWorker::new();
    loop {
        let req = match read_request(reader, limits.max_ipc_frame_bytes) {
            Ok(req) => req,
            Err(IpcCodecError::Io(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(());
            }
            Err(err) => return Err(SandboxError::Ipc(err)),
        };
        let is_shutdown = matches!(req, IpcRequest::Shutdown);
        let resp = worker.handle(req);
        write_response(writer, &resp, limits.max_ipc_frame_bytes)?;
        if is_shutdown {
            return Ok(());
        }
    }
}

struct OpenedTabDoc {
    doc_id: u64,
    mime: String,
    bytes: Vec<u8>,
}

struct TabWorkerSlot {
    transport: Box<dyn WorkerTransport>,
    generation: u32,
    crash_count: u32,
    quarantined_reason: Option<String>,
    open_doc: Option<OpenedTabDoc>,
    replay_log: Vec<String>,
}

/// Coordinates per-tab sandboxed document workers with crash restart isolation and operation-log
/// replay (plan K.22).
#[must_use]
pub struct SandboxedWorkerPool {
    config: SandboxConfig,
    spawner: Box<dyn WorkerSpawner>,
    slots: HashMap<TabId, TabWorkerSlot>,
    next_doc_id: u64,
}

impl SandboxedWorkerPool {
    /// Creates a worker pool using `spawner` to launch per-tab workers under `config`.
    pub fn new(config: SandboxConfig, spawner: Box<dyn WorkerSpawner>) -> Self {
        Self {
            config,
            spawner,
            slots: HashMap::new(),
            next_doc_id: 1,
        }
    }

    /// Creates a worker pool backed by the portable [`InProcessSpawner`].
    pub fn with_in_process_fallback(config: SandboxConfig) -> Self {
        Self::new(config, Box::new(InProcessSpawner))
    }

    /// Opens `bytes` inside `tab`'s own isolated worker under [`SecurityLimits`], returning the
    /// assigned `doc_id` and page count.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] if preflight limits fail, the worker crashes without recovery, or
    /// the tab is quarantined.
    pub fn open_document(
        &mut self,
        tab: TabId,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<(u64, usize), SandboxError> {
        if mime == "application/pdf" {
            self.config.limits.validate_pdf_bytes(&bytes)?;
        } else {
            self.config.limits.validate_input_size(bytes.len() as u64)?;
        }

        self.ensure_slot(tab)?;
        let doc_id = self.next_doc_id;
        self.next_doc_id = self.next_doc_id.saturating_add(1);

        let req = IpcRequest::OpenBytes {
            doc_id,
            mime: mime.to_owned(),
            limits: self.config.limits,
            bytes: bytes.clone(),
        };
        let resp = self.dispatch_with_recovery(tab, &req)?;
        match resp {
            IpcResponse::Opened {
                doc_id: opened_id,
                page_count,
            } => {
                if let Some(slot) = self.slots.get_mut(&tab) {
                    slot.open_doc = Some(OpenedTabDoc {
                        doc_id: opened_id,
                        mime: mime.to_owned(),
                        bytes,
                    });
                    slot.replay_log.clear();
                }
                Ok((opened_id, page_count))
            }
            IpcResponse::LimitRejected { violation } => Err(SandboxError::LimitViolation(
                LimitViolation::MalformedInput {
                    format: "worker",
                    detail: violation,
                },
            )),
            IpcResponse::WorkerError { message } => Err(SandboxError::Mupdf(message)),
            other => Err(SandboxError::WorkerCrashed {
                reason: format!("unexpected response to OpenBytes: {other:?}"),
            }),
        }
    }

    /// Records a K.14 operation JSON line on `tab` so any future worker crash restart replays it
    /// automatically before serving subsequent requests.
    pub fn record_operation(&mut self, tab: TabId, operation_json: impl Into<String>) {
        if let Some(slot) = self.slots.get_mut(&tab) {
            slot.replay_log.push(operation_json.into());
        }
    }

    /// Sends a request to `tab`'s worker, transparently restarting the worker and replaying its
    /// open document + operation log if the worker crashes or times out.
    ///
    /// Other tabs in the pool remain untouched during a crash/restart.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] if `tab` is quarantined, hits [`RestartPolicy::FailFast`], or the
    /// restarted worker also fails.
    pub fn send(&mut self, tab: TabId, request: &IpcRequest) -> Result<IpcResponse, SandboxError> {
        self.ensure_slot(tab)?;
        self.dispatch_with_recovery(tab, request)
    }

    /// Returns the health state of `tab`'s worker, if a slot exists.
    #[must_use]
    pub fn health(&self, tab: TabId) -> Option<WorkerHealth> {
        let slot = self.slots.get(&tab)?;
        if let Some(reason) = &slot.quarantined_reason {
            return Some(WorkerHealth::Quarantined {
                crash_count: slot.crash_count,
                reason: reason.clone(),
            });
        }
        if slot.crash_count > 0 {
            Some(WorkerHealth::Restarted {
                generation: slot.generation,
                crash_count: slot.crash_count,
            })
        } else {
            Some(WorkerHealth::Running {
                generation: slot.generation,
                pid: slot.transport.worker_pid(),
            })
        }
    }

    /// Closes `tab`'s worker slot and terminates its underlying worker.
    pub fn close_tab(&mut self, tab: TabId) {
        if let Some(mut slot) = self.slots.remove(&tab) {
            slot.transport.terminate();
        }
    }

    fn ensure_slot(&mut self, tab: TabId) -> Result<(), SandboxError> {
        if let Some(slot) = self.slots.get(&tab) {
            if let Some(reason) = &slot.quarantined_reason {
                return Err(SandboxError::WorkerQuarantined {
                    tab,
                    crash_count: slot.crash_count,
                    reason: reason.clone(),
                });
            }
            return Ok(());
        }
        let transport = self.spawner.spawn(&self.config)?;
        self.slots.insert(
            tab,
            TabWorkerSlot {
                transport,
                generation: 0,
                crash_count: 0,
                quarantined_reason: None,
                open_doc: None,
                replay_log: Vec::new(),
            },
        );
        Ok(())
    }

    fn dispatch_with_recovery(
        &mut self,
        tab: TabId,
        request: &IpcRequest,
    ) -> Result<IpcResponse, SandboxError> {
        let limits = self.config.limits;
        let timeout = limits.worker_timeout;

        let first_err = {
            let slot = self
                .slots
                .get_mut(&tab)
                .ok_or_else(|| SandboxError::WorkerCrashed {
                    reason: format!("missing worker slot for tab {}", tab.0),
                })?;
            if let Some(reason) = &slot.quarantined_reason {
                return Err(SandboxError::WorkerQuarantined {
                    tab,
                    crash_count: slot.crash_count,
                    reason: reason.clone(),
                });
            }
            match slot.transport.round_trip(request, &limits, timeout) {
                Ok(resp) => return Ok(resp),
                Err(err) => err,
            }
        };

        if self.config.restart_policy == RestartPolicy::FailFast
            || !is_recoverable_worker_failure(&first_err)
        {
            return Err(first_err);
        }

        // Crash restart isolation: terminate ONLY `tab`'s broken worker, spawn a fresh one,
        // reload `tab`'s document and replay its operation log without touching other tabs.
        loop {
            let can_restart = {
                let slot = self
                    .slots
                    .get_mut(&tab)
                    .ok_or_else(|| SandboxError::WorkerCrashed {
                        reason: format!("missing worker slot for tab {}", tab.0),
                    })?;
                slot.transport.terminate();
                slot.crash_count = slot.crash_count.saturating_add(1);
                if slot.crash_count > self.config.max_restarts_per_tab {
                    let reason = first_err.to_string();
                    slot.quarantined_reason = Some(reason.clone());
                    return Err(SandboxError::WorkerQuarantined {
                        tab,
                        crash_count: slot.crash_count,
                        reason,
                    });
                }
                true
            };
            if !can_restart {
                return Err(first_err);
            }

            let mut fresh = self.spawner.spawn(&self.config)?;
            let reload_outcome = {
                let slot = self
                    .slots
                    .get(&tab)
                    .ok_or_else(|| SandboxError::WorkerCrashed {
                        reason: format!("missing worker slot for tab {}", tab.0),
                    })?;
                rehydrate_worker(&mut *fresh, slot, &limits, timeout)
            };

            match reload_outcome {
                Ok(()) => {
                    let retry = fresh.round_trip(request, &limits, timeout);
                    let slot =
                        self.slots
                            .get_mut(&tab)
                            .ok_or_else(|| SandboxError::WorkerCrashed {
                                reason: format!("missing worker slot for tab {}", tab.0),
                            })?;
                    slot.transport = fresh;
                    slot.generation = slot.generation.saturating_add(1);
                    if let Ok(resp) = retry {
                        return Ok(resp);
                    }
                    // Retry failed on the new worker too; loop to respect max_restarts_per_tab.
                }
                Err(_) => {
                    fresh.terminate();
                }
            }
        }
    }
}

fn is_recoverable_worker_failure(err: &SandboxError) -> bool {
    matches!(
        err,
        SandboxError::WorkerCrashed { .. }
            | SandboxError::TimedOut { .. }
            | SandboxError::Ipc(_)
            | SandboxError::MemoryCapExceeded { .. }
    )
}

fn rehydrate_worker(
    transport: &mut dyn WorkerTransport,
    slot: &TabWorkerSlot,
    limits: &SecurityLimits,
    timeout: Duration,
) -> Result<(), SandboxError> {
    if let Some(doc) = &slot.open_doc {
        let open_req = IpcRequest::OpenBytes {
            doc_id: doc.doc_id,
            mime: doc.mime.clone(),
            limits: *limits,
            bytes: doc.bytes.clone(),
        };
        match transport.round_trip(&open_req, limits, timeout)? {
            IpcResponse::Opened { .. } => {}
            IpcResponse::WorkerError { message } => return Err(SandboxError::Mupdf(message)),
            IpcResponse::LimitRejected { violation } => {
                return Err(SandboxError::LimitViolation(
                    LimitViolation::MalformedInput {
                        format: "worker",
                        detail: violation,
                    },
                ));
            }
            other => {
                return Err(SandboxError::WorkerCrashed {
                    reason: format!("unexpected rehydrate OpenBytes response: {other:?}"),
                });
            }
        }
        if !slot.replay_log.is_empty() {
            let replay_req = IpcRequest::ApplyReplayLog {
                doc_id: doc.doc_id,
                operations_json: slot.replay_log.clone(),
            };
            match transport.round_trip(&replay_req, limits, timeout)? {
                IpcResponse::ReplayApplied { .. } => {}
                other => {
                    return Err(SandboxError::WorkerCrashed {
                        reason: format!("unexpected rehydrate Replay response: {other:?}"),
                    });
                }
            }
        }
    }
    Ok(())
}
