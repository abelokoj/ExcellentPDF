//! Out-of-process document engine sandbox and central parser security limits (plan K.22, K.23).
//!
//! Splits document parsing and rendering away from the UI process through:
//! - [`SecurityLimits`] and structural preflight checks ([`limits`]) for PDF, XFDF, `.expdf`/ZIP,
//!   and SyncTeX inputs, enforcing page, recursion, XRef, image, and zip-bomb bounds before
//!   invoking MuPDF or archive decoders.
//! - [`IpcRequest`], [`IpcResponse`], and [`IpcFrame`] ([`ipc`]) for bounded length-prefixed
//!   communication with an isolated worker process.
//! - [`OsSandboxConfig`] and [`OsSandboxGuard`] ([`os`]) for OS-level containment (Windows Job
//!   Objects with memory/process/UI caps plus restricted tokens on Windows, and portable policy
//!   guards elsewhere).
//! - [`SandboxedWorkerPool`] ([`worker`]) for per-tab worker isolation, wall-clock watchdog
//!   timeouts, memory-cap checks, and automatic crash restart with operation-log replay.

pub mod ipc;
pub mod limits;
pub mod os;
pub mod worker;

pub use ipc::{
    IPC_MAGIC, IpcCodecError, IpcFrame, IpcRequest, IpcResponse, read_request, read_response,
    write_request, write_response,
};
pub use limits::{
    LimitViolation, PdfPreflightReport, SecurityLimits, UriSafety, ZipPreflightReport,
    classify_external_uri, open_pdf_with_limits, sanitize_archive_entry_path,
    sanitize_attachment_name,
};
pub use os::{
    JobAssignment, JobMemoryStats, KillOnDrop, NetworkPolicy, OsSandboxConfig, OsSandboxError,
    OsSandboxGuard, PlatformSandboxKind, SandboxAttachment, TokenRestriction, TokenState,
};
pub use worker::{
    InProcessSpawner, InProcessWorker, RestartPolicy, SandboxConfig, SandboxError,
    SandboxedWorkerPool, SubprocessSpawner, TabId, WorkerHealth, WorkerSpawner, WorkerTransport,
    run_worker_stdio_loop,
};

#[cfg(test)]
mod tests;
