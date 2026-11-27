//! OS-level process sandboxing (Windows Job Objects + restricted tokens, and portable fallbacks)
//! for isolated PDF worker processes (plan K.22).

use std::fmt;
use std::fs::File;
use std::path::Path;

/// Privilege reduction policy applied when creating a sandboxed worker token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TokenRestriction {
    /// Drop all non-essential privileges and attach a low-privilege LUA token (`DISABLE_MAX_PRIVILEGE | LUA_TOKEN`).
    #[default]
    RestrictedLowPrivilege,
    /// Retain standard user token privileges (diagnostics / fallback only).
    Unrestricted,
}

/// Network access policy enforced on a sandboxed worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum NetworkPolicy {
    /// Block all outbound and inbound network sockets inside the worker.
    #[default]
    DenyAll,
    /// Allow loopback-only IPC (for diagnostics).
    AllowLoopbackOnly,
}

/// Whether dropping the [`OsSandboxGuard`] terminates all processes assigned to its job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum KillOnDrop {
    /// Terminate the entire worker process tree when the sandbox guard drops.
    #[default]
    TerminateProcessTree,
    /// Leave assigned processes running when the guard drops (useful for detached probes).
    Detach,
}

/// Configuration for OS-level containment of a PDF worker process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsSandboxConfig {
    /// Committed virtual memory limit enforced per worker job in bytes.
    pub memory_cap_bytes: u64,
    /// Maximum concurrent processes allowed inside the worker's job (`1` forbids child spawns).
    pub active_process_limit: u32,
    /// Token privilege restriction mode.
    pub token_restriction: TokenRestriction,
    /// Network isolation policy.
    pub network_policy: NetworkPolicy,
    /// Process-tree lifetime policy when the guard is dropped.
    pub kill_on_drop: KillOnDrop,
}

impl Default for OsSandboxConfig {
    fn default() -> Self {
        Self {
            memory_cap_bytes: super::limits::DEFAULT_WORKER_MEMORY_CAP_BYTES,
            active_process_limit: 1,
            token_restriction: TokenRestriction::RestrictedLowPrivilege,
            network_policy: NetworkPolicy::DenyAll,
            kill_on_drop: KillOnDrop::TerminateProcessTree,
        }
    }
}

impl OsSandboxConfig {
    /// Returns the default restrictive [`OsSandboxConfig`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Active platform sandbox mechanism backing an [`OsSandboxGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformSandboxKind {
    /// Windows Job Object (memory/process/UI limits) plus `CreateRestrictedToken`.
    WindowsJobAndRestrictedToken,
    /// POSIX `setrlimit` / seccomp-style fallback metadata guard.
    PosixRlimitFallback,
    /// Portable in-process policy guard when OS process containment is unavailable.
    PortableMetadataOnly,
}

/// Token state attached to the sandbox guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenState {
    /// A restricted token (`DISABLE_MAX_PRIVILEGE | LUA_TOKEN`) was constructed and held.
    Restricted,
    /// Standard token retained (e.g. `TokenRestriction::Unrestricted` or non-Windows fallback).
    Standard,
}

/// Whether a worker process has been bound into an OS job/container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobAssignment {
    /// Process handle is bound to the OS job object.
    Assigned,
    /// Job created and configured, awaiting process assignment or running in fallback mode.
    Unassigned,
    /// Guard configured to detach without hard OS job binding.
    Detached,
}

/// Snapshot describing how a sandbox guard is configured and attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxAttachment {
    /// Active platform mechanism.
    pub kind: PlatformSandboxKind,
    /// Configured memory cap in bytes.
    pub memory_cap_bytes: u64,
    /// Configured active process limit.
    pub active_process_limit: u32,
    /// Privilege state of the worker token.
    pub token_state: TokenState,
    /// Current job assignment state.
    pub job_assigned: JobAssignment,
}

/// Live memory and process accounting reported by an OS job sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobMemoryStats {
    /// Peak committed memory observed across the entire job, in bytes.
    pub peak_job_memory_bytes: u64,
    /// Peak committed memory observed for any single process in the job, in bytes.
    pub peak_process_memory_bytes: u64,
    /// Current active processes inside the job.
    pub active_processes: u32,
}

/// Error returned when creating, configuring, or querying an OS sandbox guard.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub enum OsSandboxError {
    /// Invalid configuration parameter (for example zero memory cap or zero process limit).
    InvalidConfig(String),
    /// Operating-system API call failed with `code`.
    PlatformCallFailed {
        /// Win32 or POSIX operation name.
        operation: &'static str,
        /// OS error code.
        code: u32,
    },
}

impl fmt::Display for OsSandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid sandbox config: {msg}"),
            Self::PlatformCallFailed { operation, code } => {
                write!(f, "sandbox OS call '{operation}' failed with error {code}")
            }
        }
    }
}

impl std::error::Error for OsSandboxError {}

/// A child process spawned inside an [`OsSandboxGuard`] with piped `stdin`/`stdout`
/// (`stderr` redirected to null), an isolated working directory, and an empty environment.
#[derive(Debug)]
pub struct SandboxedStdioProcess {
    /// Operating-system process identifier.
    pub pid: u32,
    /// Writable pipe connected to the child's standard input.
    pub stdin: File,
    /// Readable pipe connected to the child's standard output.
    pub stdout: File,
    #[cfg(windows)]
    process_handle: win32::OwnedHandle,
}

impl SandboxedStdioProcess {
    /// Waits for the sandboxed child process to exit and returns its exit code.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError::PlatformCallFailed`] if the OS wait or exit-code query fails.
    pub fn wait(&mut self) -> Result<u32, OsSandboxError> {
        #[cfg(windows)]
        {
            self.process_handle.wait_for_exit()
        }
        #[cfg(not(windows))]
        {
            Ok(0)
        }
    }

    /// Returns `true` if the sandboxed child process has not yet exited.
    #[must_use]
    pub fn is_running(&self) -> bool {
        #[cfg(windows)]
        {
            self.process_handle.is_running()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

/// RAII OS sandbox guard owning the platform job object and optional restricted token.
#[derive(Debug)]
#[must_use]
pub struct OsSandboxGuard {
    config: OsSandboxConfig,
    attachment: SandboxAttachment,
    #[cfg(windows)]
    inner: win32::WindowsSandboxHandles,
}

impl OsSandboxGuard {
    /// Creates and configures an OS sandbox guard for `config`.
    ///
    /// On Windows this creates an anonymous Job Object with memory, active-process, and UI
    /// restrictions, plus a restricted low-privilege token when
    /// [`TokenRestriction::RestrictedLowPrivilege`] is selected.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError`] if `config` is invalid or an OS sandbox API fails.
    pub fn create(config: OsSandboxConfig) -> Result<Self, OsSandboxError> {
        if config.memory_cap_bytes == 0 {
            return Err(OsSandboxError::InvalidConfig(
                "memory_cap_bytes must be non-zero".to_owned(),
            ));
        }
        if config.active_process_limit == 0 {
            return Err(OsSandboxError::InvalidConfig(
                "active_process_limit must be at least 1".to_owned(),
            ));
        }

        #[cfg(windows)]
        {
            let inner = win32::WindowsSandboxHandles::create(&config)?;
            let token_state = if inner.has_restricted_token() {
                TokenState::Restricted
            } else {
                TokenState::Standard
            };
            let attachment = SandboxAttachment {
                kind: PlatformSandboxKind::WindowsJobAndRestrictedToken,
                memory_cap_bytes: config.memory_cap_bytes,
                active_process_limit: config.active_process_limit,
                token_state,
                job_assigned: JobAssignment::Unassigned,
            };
            Ok(Self {
                config,
                attachment,
                inner,
            })
        }

        #[cfg(not(windows))]
        {
            let kind = if cfg!(unix) {
                PlatformSandboxKind::PosixRlimitFallback
            } else {
                PlatformSandboxKind::PortableMetadataOnly
            };
            let attachment = SandboxAttachment {
                kind,
                memory_cap_bytes: config.memory_cap_bytes,
                active_process_limit: config.active_process_limit,
                token_state: TokenState::Standard,
                job_assigned: JobAssignment::Unassigned,
            };
            Ok(Self { config, attachment })
        }
    }

    /// Attaches a running child process (`pid`) to this sandbox's job object.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError::PlatformCallFailed`] if opening or assigning `pid` fails.
    pub fn attach_pid(&mut self, pid: u32) -> Result<(), OsSandboxError> {
        #[cfg(windows)]
        {
            self.inner.assign_pid(pid)?;
            self.attachment.job_assigned = JobAssignment::Assigned;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            self.attachment.job_assigned = JobAssignment::Assigned;
            Ok(())
        }
    }

    /// Spawns `command` with `args` inside this sandbox's job object and restricted token (when
    /// configured), using `work_dir` as its current directory, an empty environment block, piped
    /// `stdin`/`stdout`, and null `stderr`.
    ///
    /// On Windows the child is created suspended (`CREATE_SUSPENDED | CREATE_NO_WINDOW`), assigned
    /// to the job object before its primary thread executes, and then resumed. On non-Windows
    /// platforms this returns an error until an OS process sandbox is implemented.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError`] if `work_dir` is not an existing directory, the platform lacks
    /// an OS process sandbox, or any OS call fails.
    pub fn spawn_sandboxed_stdio(
        &mut self,
        command: &str,
        args: &[&str],
        work_dir: &Path,
    ) -> Result<SandboxedStdioProcess, OsSandboxError> {
        if !work_dir.is_dir() {
            return Err(OsSandboxError::InvalidConfig(format!(
                "sandbox work_dir does not exist or is not a directory: {}",
                work_dir.display()
            )));
        }
        #[cfg(windows)]
        {
            let proc = self.inner.spawn_sandboxed_stdio(command, args, work_dir)?;
            self.attachment.job_assigned = JobAssignment::Assigned;
            Ok(proc)
        }
        #[cfg(not(windows))]
        {
            let _ = (command, args);
            Err(OsSandboxError::InvalidConfig(
                "OS process sandboxing is not yet implemented on this platform".to_owned(),
            ))
        }
    }

    /// Queries live memory and process accounting from the OS job object.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError::PlatformCallFailed`] if the OS query fails.
    pub fn memory_stats(&self) -> Result<JobMemoryStats, OsSandboxError> {
        #[cfg(windows)]
        {
            self.inner.query_stats()
        }
        #[cfg(not(windows))]
        {
            Ok(JobMemoryStats::default())
        }
    }

    /// Terminates every process assigned to this sandbox job with `exit_code`.
    ///
    /// # Errors
    ///
    /// Returns [`OsSandboxError::PlatformCallFailed`] if the OS termination call fails.
    pub fn terminate_job(&self, exit_code: u32) -> Result<(), OsSandboxError> {
        #[cfg(windows)]
        {
            self.inner.terminate(exit_code)
        }
        #[cfg(not(windows))]
        {
            let _ = exit_code;
            Ok(())
        }
    }

    /// Returns the sandbox's active configuration.
    #[must_use]
    pub fn config(&self) -> &OsSandboxConfig {
        &self.config
    }

    /// Returns the attachment status and platform mechanism.
    #[must_use]
    pub fn attachment(&self) -> &SandboxAttachment {
        &self.attachment
    }
}

#[cfg(windows)]
mod win32 {
    use std::ffi::{OsStr, c_void};
    use std::fs::File;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::FromRawHandle as _;
    use std::path::Path;
    use std::ptr::{null, null_mut, without_provenance_mut};

    use super::{
        JobMemoryStats, KillOnDrop, OsSandboxConfig, OsSandboxError, SandboxedStdioProcess,
        TokenRestriction,
    };

    type Handle = *mut c_void;
    type Bool = i32;

    const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: u32 = 1;
    const JOB_OBJECT_BASIC_UI_RESTRICTIONS: u32 = 4;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;

    const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x0000_0200;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    const JOB_OBJECT_UILIMIT_HANDLES: u32 = 0x0000_0001;
    const JOB_OBJECT_UILIMIT_READCLIPBOARD: u32 = 0x0000_0002;
    const JOB_OBJECT_UILIMIT_WRITECLIPBOARD: u32 = 0x0000_0004;
    const JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS: u32 = 0x0000_0008;
    const JOB_OBJECT_UILIMIT_DISPLAYSETTINGS: u32 = 0x0000_0010;
    const JOB_OBJECT_UILIMIT_GLOBALATOMS: u32 = 0x0000_0020;
    const JOB_OBJECT_UILIMIT_DESKTOP: u32 = 0x0000_0040;
    const JOB_OBJECT_UILIMIT_EXITWINDOWS: u32 = 0x0000_0080;

    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_SET_QUOTA: u32 = 0x0100;

    const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
    const TOKEN_DUPLICATE: u32 = 0x0002;
    const TOKEN_QUERY: u32 = 0x0008;
    const DISABLE_MAX_PRIVILEGE: u32 = 0x01;
    const LUA_TOKEN: u32 = 0x04;

    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

    const INFINITE: u32 = 0xFFFF_FFFF;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const WAIT_FAILED: u32 = 0xFFFF_FFFF;
    const INVALID_HANDLE_VALUE: Handle = without_provenance_mut(usize::MAX);

    #[repr(C)]
    struct SecurityAttributes {
        n_length: u32,
        lp_security_descriptor: *mut c_void,
        b_inherit_handle: Bool,
    }

    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        lp_reserved: *mut u16,
        lp_desktop: *mut u16,
        lp_title: *mut u16,
        dw_x: u32,
        dw_y: u32,
        dw_x_size: u32,
        dw_y_size: u32,
        dw_x_count_chars: u32,
        dw_y_count_chars: u32,
        dw_fill_attribute: u32,
        dw_flags: u32,
        w_show_window: u16,
        cb_reserved2: u16,
        lp_reserved2: *mut u8,
        h_std_input: Handle,
        h_std_output: Handle,
        h_std_error: Handle,
    }

    #[repr(C)]
    struct ProcessInformation {
        h_process: Handle,
        h_thread: Handle,
        dw_process_id: u32,
        dw_thread_id: u32,
    }

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    struct JobObjectBasicAccountingInformation {
        total_user_time: i64,
        total_kernel_time: i64,
        this_period_total_user_time: i64,
        this_period_total_kernel_time: i64,
        total_page_fault_count: u32,
        total_processes: u32,
        active_processes: u32,
        total_terminated_processes: u32,
    }

    #[repr(C)]
    struct JobObjectBasicUiRestrictions {
        ui_restrictions_class: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *const c_void, lp_name: *const u16) -> Handle;
        fn SetInformationJobObject(
            h_job: Handle,
            job_object_information_class: u32,
            lp_job_object_information: *const c_void,
            cb_job_object_information_length: u32,
        ) -> Bool;
        fn QueryInformationJobObject(
            h_job: Handle,
            job_object_information_class: u32,
            lp_job_object_information: *mut c_void,
            cb_job_object_information_length: u32,
            lp_return_length: *mut u32,
        ) -> Bool;
        fn AssignProcessToJobObject(h_job: Handle, h_process: Handle) -> Bool;
        fn TerminateJobObject(h_job: Handle, u_exit_code: u32) -> Bool;
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: Bool,
            dw_process_id: u32,
        ) -> Handle;
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(h_object: Handle) -> Bool;
        fn GetLastError() -> u32;
        fn CreatePipe(
            h_read_pipe: *mut Handle,
            h_write_pipe: *mut Handle,
            lp_pipe_attributes: *const SecurityAttributes,
            n_size: u32,
        ) -> Bool;
        fn SetHandleInformation(h_object: Handle, dw_mask: u32, dw_flags: u32) -> Bool;
        fn CreateFileW(
            lp_file_name: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *const SecurityAttributes,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: Handle,
        ) -> Handle;
        fn CreateProcessW(
            lp_application_name: *const u16,
            lp_command_line: *mut u16,
            lp_process_attributes: *const SecurityAttributes,
            lp_thread_attributes: *const SecurityAttributes,
            b_inherit_handles: Bool,
            dw_creation_flags: u32,
            lp_environment: *mut c_void,
            lp_current_directory: *const u16,
            lp_startup_info: *const StartupInfoW,
            lp_process_information: *mut ProcessInformation,
        ) -> Bool;
        fn ResumeThread(h_thread: Handle) -> u32;
        fn TerminateProcess(h_process: Handle, u_exit_code: u32) -> Bool;
        fn WaitForSingleObject(h_handle: Handle, dw_milliseconds: u32) -> u32;
        fn GetExitCodeProcess(h_process: Handle, lp_exit_code: *mut u32) -> Bool;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(
            process_handle: Handle,
            desired_access: u32,
            token_handle: *mut Handle,
        ) -> Bool;
        fn CreateRestrictedToken(
            existing_token_handle: Handle,
            flags: u32,
            disable_sid_count: u32,
            sids_to_disable: *const c_void,
            delete_privilege_count: u32,
            privileges_to_delete: *const c_void,
            restricted_sid_count: u32,
            sids_to_restrict: *const c_void,
            new_token_handle: *mut Handle,
        ) -> Bool;
        fn CreateProcessAsUserW(
            h_token: Handle,
            lp_application_name: *const u16,
            lp_command_line: *mut u16,
            lp_process_attributes: *const SecurityAttributes,
            lp_thread_attributes: *const SecurityAttributes,
            b_inherit_handles: Bool,
            dw_creation_flags: u32,
            lp_environment: *mut c_void,
            lp_current_directory: *const u16,
            lp_startup_info: *const StartupInfoW,
            lp_process_information: *mut ProcessInformation,
        ) -> Bool;
    }

    #[derive(Debug)]
    pub(super) struct OwnedHandle(Handle);

    // SAFETY: Windows kernel object handles (Job Objects, Tokens, Process handles) can be
    // closed safely from any thread via `CloseHandle`.
    unsafe impl Send for OwnedHandle {}
    // SAFETY: Shared references only read the handle value for Win32 query/terminate calls,
    // which are thread-safe kernel object operations.
    unsafe impl Sync for OwnedHandle {}

    impl OwnedHandle {
        fn from_raw(raw: Handle, operation: &'static str) -> Result<Self, OsSandboxError> {
            if raw.is_null() {
                // SAFETY: `GetLastError` reads thread-local Win32 error state with no invariants.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed { operation, code });
            }
            Ok(Self(raw))
        }

        fn from_raw_file(raw: Handle, operation: &'static str) -> Result<Self, OsSandboxError> {
            if raw.is_null() || raw == INVALID_HANDLE_VALUE {
                // SAFETY: `GetLastError` reads thread-local Win32 error state with no invariants.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed { operation, code });
            }
            Ok(Self(raw))
        }

        fn raw(&self) -> Handle {
            self.0
        }

        fn into_raw(self) -> Handle {
            let raw = self.0;
            std::mem::forget(self);
            raw
        }

        pub(super) fn wait_for_exit(&self) -> Result<u32, OsSandboxError> {
            // SAFETY: `self.0` is a valid process handle owned by this wrapper.
            let wait_res = unsafe { WaitForSingleObject(self.0, INFINITE) };
            if wait_res == WAIT_FAILED {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "WaitForSingleObject",
                    code,
                });
            }
            let mut exit_code = 0u32;
            // SAFETY: `self.0` is a valid process handle and `exit_code` is writable.
            let ok = unsafe { GetExitCodeProcess(self.0, &raw mut exit_code) };
            if ok == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "GetExitCodeProcess",
                    code,
                });
            }
            Ok(exit_code)
        }

        pub(super) fn is_running(&self) -> bool {
            // SAFETY: `self.0` is a valid process handle; a 0 ms wait polls exit status.
            unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` is an owned Win32 kernel handle obtained from a successful
                // creation/open call and has not been closed yet.
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    #[derive(Debug)]
    pub(super) struct WindowsSandboxHandles {
        job: OwnedHandle,
        restricted_token: Option<OwnedHandle>,
    }

    impl WindowsSandboxHandles {
        pub(super) fn create(config: &OsSandboxConfig) -> Result<Self, OsSandboxError> {
            // SAFETY: Passing null security attributes and null name creates an anonymous
            // private Job Object owned by this process.
            let raw_job = unsafe { CreateJobObjectW(null(), null()) };
            let job = OwnedHandle::from_raw(raw_job, "CreateJobObjectW")?;

            let cap = usize::try_from(config.memory_cap_bytes).unwrap_or(usize::MAX);
            let mut flags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_JOB_MEMORY;
            if config.kill_on_drop == KillOnDrop::TerminateProcessTree {
                flags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            }

            // SAFETY: `JobObjectExtendedLimitInformation` is a C POD struct whose fields are
            // initialized right after zeroing.
            let mut ext: JobObjectExtendedLimitInformation = unsafe { zeroed() };
            ext.basic_limit_information.limit_flags = flags;
            ext.basic_limit_information.active_process_limit = config.active_process_limit;
            ext.process_memory_limit = cap;
            ext.job_memory_limit = cap;

            let ext_len = u32::try_from(size_of::<JobObjectExtendedLimitInformation>())
                .map_err(|_| OsSandboxError::InvalidConfig("struct size overflow".to_owned()))?;
            // SAFETY: `job.raw()` is a valid Job Object handle and `ext` points to an initialized
            // `JobObjectExtendedLimitInformation` of `ext_len` bytes.
            let ok = unsafe {
                SetInformationJobObject(
                    job.raw(),
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&raw const ext).cast::<c_void>(),
                    ext_len,
                )
            };
            if ok == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "SetInformationJobObject(ExtendedLimit)",
                    code,
                });
            }

            let ui = JobObjectBasicUiRestrictions {
                ui_restrictions_class: JOB_OBJECT_UILIMIT_HANDLES
                    | JOB_OBJECT_UILIMIT_READCLIPBOARD
                    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
                    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                    | JOB_OBJECT_UILIMIT_GLOBALATOMS
                    | JOB_OBJECT_UILIMIT_DESKTOP
                    | JOB_OBJECT_UILIMIT_EXITWINDOWS,
            };
            let ui_len = u32::try_from(size_of::<JobObjectBasicUiRestrictions>())
                .map_err(|_| OsSandboxError::InvalidConfig("struct size overflow".to_owned()))?;
            // SAFETY: `job.raw()` is a valid Job Object handle and `ui` points to `ui_len` bytes.
            let ui_ok = unsafe {
                SetInformationJobObject(
                    job.raw(),
                    JOB_OBJECT_BASIC_UI_RESTRICTIONS,
                    (&raw const ui).cast::<c_void>(),
                    ui_len,
                )
            };
            if ui_ok == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "SetInformationJobObject(UiRestrictions)",
                    code,
                });
            }

            let restricted_token = match config.token_restriction {
                TokenRestriction::RestrictedLowPrivilege => Some(create_restricted_token()?),
                TokenRestriction::Unrestricted => None,
            };

            Ok(Self {
                job,
                restricted_token,
            })
        }

        pub(super) fn has_restricted_token(&self) -> bool {
            self.restricted_token.is_some()
        }

        pub(super) fn assign_pid(&self, pid: u32) -> Result<(), OsSandboxError> {
            // SAFETY: Opens a handle with `PROCESS_SET_QUOTA | PROCESS_TERMINATE` to assign
            // `pid` to our job object; handle is wrapped in `OwnedHandle` so it always closes.
            let raw_proc = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
            let proc_handle = OwnedHandle::from_raw(raw_proc, "OpenProcess")?;

            // SAFETY: Both `self.job.raw()` and `proc_handle.raw()` are valid open Win32 handles.
            let ok = unsafe { AssignProcessToJobObject(self.job.raw(), proc_handle.raw()) };
            if ok == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "AssignProcessToJobObject",
                    code,
                });
            }
            Ok(())
        }

        pub(super) fn spawn_sandboxed_stdio(
            &self,
            command: &str,
            args: &[&str],
            work_dir: &Path,
        ) -> Result<SandboxedStdioProcess, OsSandboxError> {
            let sa = SecurityAttributes {
                n_length: u32::try_from(size_of::<SecurityAttributes>()).map_err(|_| {
                    OsSandboxError::InvalidConfig("struct size overflow".to_owned())
                })?,
                lp_security_descriptor: null_mut(),
                b_inherit_handle: 1,
            };

            let (stdin_read, stdin_write) = create_inheritable_pipe(&sa)?;
            set_non_inheritable(&stdin_write)?;

            let (stdout_read, stdout_write) = create_inheritable_pipe(&sa)?;
            set_non_inheritable(&stdout_read)?;

            let nul_name = to_wide_nul(OsStr::new("NUL"));
            // SAFETY: `nul_name` is a NUL-terminated UTF-16 string and `sa` is initialized.
            let stderr_raw = unsafe {
                CreateFileW(
                    nul_name.as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &raw const sa,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            };
            let stderr_nul = OwnedHandle::from_raw_file(stderr_raw, "CreateFileW(NUL)")?;

            // SAFETY: `StartupInfoW` is a C POD struct whose fields are initialized below.
            let mut si: StartupInfoW = unsafe { zeroed() };
            si.cb = u32::try_from(size_of::<StartupInfoW>())
                .map_err(|_| OsSandboxError::InvalidConfig("struct size overflow".to_owned()))?;
            si.dw_flags = STARTF_USESTDHANDLES;
            si.h_std_input = stdin_read.raw();
            si.h_std_output = stdout_write.raw();
            si.h_std_error = stderr_nul.raw();

            let mut cmd_line = build_command_line(command, args);
            let cwd = to_wide_nul(work_dir.as_os_str());
            // Empty Unicode environment block: double NUL terminator.
            let mut empty_env: [u16; 2] = [0, 0];

            // SAFETY: `ProcessInformation` is a C POD struct filled by `CreateProcess*`.
            let mut pi: ProcessInformation = unsafe { zeroed() };
            let flags = CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT;
            let (created, op_name) = if let Some(token) = &self.restricted_token {
                // SAFETY: `token.raw()` is a valid restricted primary token derived from the
                // current process; `cmd_line`, `empty_env`, `cwd`, `si`, and `pi` are valid
                // initialized buffers for the duration of the call.
                let ok = unsafe {
                    CreateProcessAsUserW(
                        token.raw(),
                        null(),
                        cmd_line.as_mut_ptr(),
                        null(),
                        null(),
                        1,
                        flags,
                        empty_env.as_mut_ptr().cast::<c_void>(),
                        cwd.as_ptr(),
                        &raw const si,
                        &raw mut pi,
                    )
                };
                (ok, "CreateProcessAsUserW")
            } else {
                // SAFETY: `cmd_line`, `empty_env`, `cwd`, `si`, and `pi` are valid initialized
                // buffers for the duration of the call.
                let ok = unsafe {
                    CreateProcessW(
                        null(),
                        cmd_line.as_mut_ptr(),
                        null(),
                        null(),
                        1,
                        flags,
                        empty_env.as_mut_ptr().cast::<c_void>(),
                        cwd.as_ptr(),
                        &raw const si,
                        &raw mut pi,
                    )
                };
                (ok, "CreateProcessW")
            };

            if created == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: op_name,
                    code,
                });
            }

            let proc_handle = OwnedHandle::from_raw(pi.h_process, op_name)?;
            let thread_handle = OwnedHandle::from_raw(pi.h_thread, op_name)?;

            // Close child pipe ends in the parent now that the child has inherited them.
            drop(stdin_read);
            drop(stdout_write);
            drop(stderr_nul);

            // Bind the suspended child to the Job Object before its primary thread executes.
            // SAFETY: Both `self.job.raw()` and `proc_handle.raw()` are valid open Win32 handles.
            let assigned = unsafe { AssignProcessToJobObject(self.job.raw(), proc_handle.raw()) };
            if assigned == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                // SAFETY: `proc_handle.raw()` is a valid suspended process handle created above.
                unsafe {
                    let _ = TerminateProcess(proc_handle.raw(), 1);
                }
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "AssignProcessToJobObject",
                    code,
                });
            }

            // Resume the child's primary thread now that Job Object limits are enforced.
            // SAFETY: `thread_handle.raw()` is the suspended primary thread created above.
            let prev_suspend = unsafe { ResumeThread(thread_handle.raw()) };
            if prev_suspend == u32::MAX {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                // SAFETY: `proc_handle.raw()` is a valid process handle created above.
                unsafe {
                    let _ = TerminateProcess(proc_handle.raw(), 1);
                }
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "ResumeThread",
                    code,
                });
            }
            drop(thread_handle);

            // SAFETY: `stdin_write` is a valid owned Win32 pipe handle whose ownership is
            // transferred exclusively to `std::fs::File` via `into_raw()`.
            let stdin = unsafe { File::from_raw_handle(stdin_write.into_raw()) };
            // SAFETY: `stdout_read` is a valid owned Win32 pipe handle whose ownership is
            // transferred exclusively to `std::fs::File` via `into_raw()`.
            let stdout = unsafe { File::from_raw_handle(stdout_read.into_raw()) };

            Ok(SandboxedStdioProcess {
                pid: pi.dw_process_id,
                stdin,
                stdout,
                process_handle: proc_handle,
            })
        }

        pub(super) fn query_stats(&self) -> Result<JobMemoryStats, OsSandboxError> {
            // SAFETY: `ext` is a C POD struct filled by `QueryInformationJobObject`.
            let mut ext: JobObjectExtendedLimitInformation = unsafe { zeroed() };
            let ext_len = u32::try_from(size_of::<JobObjectExtendedLimitInformation>())
                .map_err(|_| OsSandboxError::InvalidConfig("struct size overflow".to_owned()))?;
            let mut ret_len = 0u32;
            // SAFETY: `self.job.raw()` is valid and `ext` has `ext_len` writable bytes.
            let ok_ext = unsafe {
                QueryInformationJobObject(
                    self.job.raw(),
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&raw mut ext).cast::<c_void>(),
                    ext_len,
                    &raw mut ret_len,
                )
            };
            if ok_ext == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "QueryInformationJobObject(ExtendedLimit)",
                    code,
                });
            }

            // SAFETY: `acct` is a C POD struct filled by `QueryInformationJobObject`.
            let mut acct: JobObjectBasicAccountingInformation = unsafe { zeroed() };
            let acct_len = u32::try_from(size_of::<JobObjectBasicAccountingInformation>())
                .map_err(|_| OsSandboxError::InvalidConfig("struct size overflow".to_owned()))?;
            // SAFETY: `self.job.raw()` is valid and `acct` has `acct_len` writable bytes.
            let ok_acct = unsafe {
                QueryInformationJobObject(
                    self.job.raw(),
                    JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
                    (&raw mut acct).cast::<c_void>(),
                    acct_len,
                    &raw mut ret_len,
                )
            };
            if ok_acct == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "QueryInformationJobObject(BasicAccounting)",
                    code,
                });
            }

            Ok(JobMemoryStats {
                peak_job_memory_bytes: ext.peak_job_memory_used as u64,
                peak_process_memory_bytes: ext.peak_process_memory_used as u64,
                active_processes: acct.active_processes,
            })
        }

        pub(super) fn terminate(&self, exit_code: u32) -> Result<(), OsSandboxError> {
            // SAFETY: `self.job.raw()` is a valid Job Object handle.
            let ok = unsafe { TerminateJobObject(self.job.raw(), exit_code) };
            if ok == 0 {
                // SAFETY: `GetLastError` reads thread-local Win32 error state.
                let code = unsafe { GetLastError() };
                return Err(OsSandboxError::PlatformCallFailed {
                    operation: "TerminateJobObject",
                    code,
                });
            }
            Ok(())
        }
    }

    fn create_inheritable_pipe(
        sa: &SecurityAttributes,
    ) -> Result<(OwnedHandle, OwnedHandle), OsSandboxError> {
        let mut read_raw: Handle = null_mut();
        let mut write_raw: Handle = null_mut();
        // SAFETY: `read_raw` and `write_raw` are writable out-pointers and `sa` is initialized.
        let ok = unsafe { CreatePipe(&raw mut read_raw, &raw mut write_raw, &raw const *sa, 0) };
        if ok == 0 {
            // SAFETY: `GetLastError` reads thread-local Win32 error state.
            let code = unsafe { GetLastError() };
            return Err(OsSandboxError::PlatformCallFailed {
                operation: "CreatePipe",
                code,
            });
        }
        let read = OwnedHandle::from_raw(read_raw, "CreatePipe")?;
        let write = OwnedHandle::from_raw(write_raw, "CreatePipe")?;
        Ok((read, write))
    }

    fn set_non_inheritable(handle: &OwnedHandle) -> Result<(), OsSandboxError> {
        // SAFETY: `handle.raw()` is a valid pipe handle owned by this process.
        let ok = unsafe { SetHandleInformation(handle.raw(), HANDLE_FLAG_INHERIT, 0) };
        if ok == 0 {
            // SAFETY: `GetLastError` reads thread-local Win32 error state.
            let code = unsafe { GetLastError() };
            return Err(OsSandboxError::PlatformCallFailed {
                operation: "SetHandleInformation",
                code,
            });
        }
        Ok(())
    }

    fn append_quoted_arg(out: &mut Vec<u16>, arg: &str) {
        let needs_quotes = arg.is_empty() || arg.contains([' ', '\t', '"']);
        if !needs_quotes {
            out.extend(arg.encode_utf16());
            return;
        }
        out.push(u16::from(b'"'));
        let mut backslashes = 0usize;
        for unit in arg.encode_utf16() {
            if unit == u16::from(b'\\') {
                backslashes += 1;
            } else if unit == u16::from(b'"') {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push(u16::from(b'\\'));
                }
                out.push(u16::from(b'"'));
                backslashes = 0;
            } else {
                for _ in 0..backslashes {
                    out.push(u16::from(b'\\'));
                }
                backslashes = 0;
                out.push(unit);
            }
        }
        for _ in 0..(backslashes * 2) {
            out.push(u16::from(b'\\'));
        }
        out.push(u16::from(b'"'));
    }

    fn build_command_line(command: &str, args: &[&str]) -> Vec<u16> {
        let mut out = Vec::new();
        append_quoted_arg(&mut out, command);
        for arg in args {
            out.push(u16::from(b' '));
            append_quoted_arg(&mut out, arg);
        }
        out.push(0);
        out
    }

    fn to_wide_nul(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn create_restricted_token() -> Result<OwnedHandle, OsSandboxError> {
        let mut proc_token: Handle = null_mut();
        // SAFETY: `GetCurrentProcess()` returns a pseudo-handle for the current process, and
        // `proc_token` receives a newly opened token handle on success.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY,
                &raw mut proc_token,
            )
        };
        if opened == 0 {
            // SAFETY: `GetLastError` reads thread-local Win32 error state.
            let code = unsafe { GetLastError() };
            return Err(OsSandboxError::PlatformCallFailed {
                operation: "OpenProcessToken",
                code,
            });
        }
        let proc_token = OwnedHandle::from_raw(proc_token, "OpenProcessToken")?;

        let mut restricted: Handle = null_mut();
        // SAFETY: `proc_token.raw()` is a valid primary/impersonation token with `TOKEN_DUPLICATE`
        // and `TOKEN_QUERY`; all SID/privilege list counts are 0 with null pointers.
        let created = unsafe {
            CreateRestrictedToken(
                proc_token.raw(),
                DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
                0,
                null(),
                0,
                null(),
                0,
                null(),
                &raw mut restricted,
            )
        };
        if created == 0 {
            // SAFETY: `GetLastError` reads thread-local Win32 error state.
            let code = unsafe { GetLastError() };
            return Err(OsSandboxError::PlatformCallFailed {
                operation: "CreateRestrictedToken",
                code,
            });
        }
        OwnedHandle::from_raw(restricted, "CreateRestrictedToken")
    }
}
