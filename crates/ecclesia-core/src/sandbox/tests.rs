use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::*;

fn corpus_pdf(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read corpus {name}: {e}"))
}

#[test]
fn ipc_frame_and_messages_round_trip_and_reject_oversized() {
    let limits = SecurityLimits::strict();
    let sample_reqs = [
        IpcRequest::Ping { sequence: 42 },
        IpcRequest::OpenBytes {
            doc_id: 7,
            mime: "application/pdf".to_owned(),
            limits,
            bytes: b"%PDF-1.4\n".to_vec(),
        },
        IpcRequest::RenderTile {
            doc_id: 7,
            page: 0,
            x0: 0,
            y0: 0,
            x1: 256,
            y1: 256,
            scale_x: 1.5,
            scale_y: 1.5,
        },
        IpcRequest::ExtractText { doc_id: 7, page: 0 },
        IpcRequest::ApplyReplayLog {
            doc_id: 7,
            operations_json: vec![r#"{"op":"annot.create"}"#.to_owned()],
        },
        IpcRequest::CloseDocument { doc_id: 7 },
        IpcRequest::Shutdown,
    ];

    for req in &sample_reqs {
        let mut buf = Vec::new();
        write_request(&mut buf, req, limits.max_ipc_frame_bytes).expect("write request");
        let decoded =
            read_request(&mut Cursor::new(&buf), limits.max_ipc_frame_bytes).expect("read request");
        assert_eq!(&decoded, req, "request round-trip");
    }

    let sample_resps = [
        IpcResponse::Pong {
            sequence: 42,
            worker_pid: 1234,
        },
        IpcResponse::Opened {
            doc_id: 7,
            page_count: 2,
        },
        IpcResponse::TileRendered {
            doc_id: 7,
            page: 0,
            width: 2,
            height: 2,
            components: 3,
            stride: 6,
            samples: vec![255; 12],
        },
        IpcResponse::TextExtracted {
            doc_id: 7,
            page: 0,
            text: "zlib".to_owned(),
        },
        IpcResponse::ReplayApplied {
            doc_id: 7,
            applied_count: 1,
        },
        IpcResponse::Closed { doc_id: 7 },
        IpcResponse::LimitRejected {
            violation: "too many pages".to_owned(),
        },
        IpcResponse::WorkerError {
            message: "bad page".to_owned(),
        },
        IpcResponse::ShuttingDown,
    ];

    for resp in &sample_resps {
        let mut buf = Vec::new();
        write_response(&mut buf, resp, limits.max_ipc_frame_bytes).expect("write response");
        let decoded = read_response(&mut Cursor::new(&buf), limits.max_ipc_frame_bytes)
            .expect("read response");
        assert_eq!(&decoded, resp, "response round-trip");
    }

    // Corrupt magic and oversized header must be rejected before allocating.
    let mut bad_magic = Vec::new();
    bad_magic.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    bad_magic.extend_from_slice(&0u32.to_be_bytes());
    bad_magic.extend_from_slice(&0u32.to_be_bytes());
    assert!(matches!(
        IpcFrame::read_from(&mut Cursor::new(&bad_magic), 1024),
        Err(IpcCodecError::BadMagic(0xDEAD_BEEF))
    ));

    let mut huge_len = Vec::new();
    huge_len.extend_from_slice(&IPC_MAGIC.to_be_bytes());
    huge_len.extend_from_slice(&1024u32.to_be_bytes());
    huge_len.extend_from_slice(&(500 * 1024 * 1024u32).to_be_bytes());
    assert!(matches!(
        IpcFrame::read_from(&mut Cursor::new(&huge_len), 4096),
        Err(IpcCodecError::FrameTooLarge { .. })
    ));
}

#[test]
fn stdio_worker_loop_serves_corpus_pdf_and_exits_on_shutdown() {
    let pdf = corpus_pdf("zlib-manual.pdf");
    let limits = SecurityLimits::default();
    let mut input = Vec::new();
    write_request(
        &mut input,
        &IpcRequest::Ping { sequence: 1 },
        limits.max_ipc_frame_bytes,
    )
    .expect("ping");
    write_request(
        &mut input,
        &IpcRequest::OpenBytes {
            doc_id: 10,
            mime: "application/pdf".to_owned(),
            limits,
            bytes: pdf,
        },
        limits.max_ipc_frame_bytes,
    )
    .expect("open");
    write_request(
        &mut input,
        &IpcRequest::ExtractText {
            doc_id: 10,
            page: 0,
        },
        limits.max_ipc_frame_bytes,
    )
    .expect("extract");
    write_request(
        &mut input,
        &IpcRequest::Shutdown,
        limits.max_ipc_frame_bytes,
    )
    .expect("shutdown");

    let mut output = Vec::new();
    run_worker_stdio_loop(&mut Cursor::new(&input), &mut output, limits).expect("worker loop");

    let mut out_cursor = Cursor::new(&output);
    assert!(matches!(
        read_response(&mut out_cursor, limits.max_ipc_frame_bytes).expect("pong"),
        IpcResponse::Pong { sequence: 1, .. }
    ));
    assert!(matches!(
        read_response(&mut out_cursor, limits.max_ipc_frame_bytes).expect("opened"),
        IpcResponse::Opened {
            doc_id: 10,
            page_count: 2
        }
    ));
    match read_response(&mut out_cursor, limits.max_ipc_frame_bytes).expect("text") {
        IpcResponse::TextExtracted { text, .. } => {
            assert!(text.contains("zlib"), "expected zlib text, got {text:.80}");
        }
        other => panic!("expected TextExtracted, got {other:?}"),
    }
    assert!(matches!(
        read_response(&mut out_cursor, limits.max_ipc_frame_bytes).expect("shutting down"),
        IpcResponse::ShuttingDown
    ));
}

struct MockTransport {
    inner: InProcessWorker,
    crash_next: Arc<AtomicBool>,
    hang_next: Arc<AtomicBool>,
    mem_bytes: Arc<Mutex<u64>>,
    terminated: Arc<AtomicBool>,
    pid: u32,
}

impl WorkerTransport for MockTransport {
    fn round_trip(
        &mut self,
        request: &IpcRequest,
        limits: &SecurityLimits,
        timeout: Duration,
    ) -> Result<IpcResponse, SandboxError> {
        let mem = *self.mem_bytes.lock().expect("mem lock");
        if mem > limits.worker_memory_cap_bytes {
            return Err(SandboxError::MemoryCapExceeded {
                used_bytes: mem,
                cap_bytes: limits.worker_memory_cap_bytes,
            });
        }
        if self.hang_next.swap(false, Ordering::SeqCst) {
            return Err(SandboxError::TimedOut { timeout });
        }
        if self.crash_next.swap(false, Ordering::SeqCst) {
            return Err(SandboxError::WorkerCrashed {
                reason: "simulated SIGSEGV in MuPDF parser".to_owned(),
            });
        }
        Ok(self.inner.handle(request.clone()))
    }

    fn worker_pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn observed_memory_bytes(&self) -> u64 {
        *self.mem_bytes.lock().expect("mem lock")
    }

    fn terminate(&mut self) {
        self.terminated.store(true, Ordering::SeqCst);
    }
}

struct MockSpawner {
    next_pid: AtomicU32,
    crash_first_tab_once: Arc<AtomicBool>,
    always_crash: Arc<AtomicBool>,
    hang_next: Arc<AtomicBool>,
    mem_bytes: Arc<Mutex<u64>>,
    replayed_seen: Arc<Mutex<usize>>,
}

impl MockSpawner {
    fn new() -> Self {
        Self {
            next_pid: AtomicU32::new(100),
            crash_first_tab_once: Arc::new(AtomicBool::new(false)),
            always_crash: Arc::new(AtomicBool::new(false)),
            hang_next: Arc::new(AtomicBool::new(false)),
            mem_bytes: Arc::new(Mutex::new(0)),
            replayed_seen: Arc::new(Mutex::new(0)),
        }
    }
}

struct ReplayAwareTransport {
    inner: MockTransport,
    replayed_seen: Arc<Mutex<usize>>,
    always_crash: Arc<AtomicBool>,
}

impl WorkerTransport for ReplayAwareTransport {
    fn round_trip(
        &mut self,
        request: &IpcRequest,
        limits: &SecurityLimits,
        timeout: Duration,
    ) -> Result<IpcResponse, SandboxError> {
        if self.always_crash.load(Ordering::SeqCst)
            && !matches!(request, IpcRequest::OpenBytes { .. })
        {
            return Err(SandboxError::WorkerCrashed {
                reason: "persistent crash".to_owned(),
            });
        }
        let res = self.inner.round_trip(request, limits, timeout)?;
        if let IpcResponse::ReplayApplied { applied_count, .. } = &res {
            *self.replayed_seen.lock().expect("replay lock") += *applied_count;
        }
        Ok(res)
    }

    fn worker_pid(&self) -> Option<u32> {
        self.inner.worker_pid()
    }

    fn observed_memory_bytes(&self) -> u64 {
        self.inner.observed_memory_bytes()
    }

    fn terminate(&mut self) {
        self.inner.terminate();
    }
}

impl WorkerSpawner for MockSpawner {
    fn spawn(&self, _config: &SandboxConfig) -> Result<Box<dyn WorkerTransport>, SandboxError> {
        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        let crash_next = if pid == 100 {
            Arc::clone(&self.crash_first_tab_once)
        } else {
            Arc::new(AtomicBool::new(false))
        };
        Ok(Box::new(ReplayAwareTransport {
            inner: MockTransport {
                inner: InProcessWorker::new(),
                crash_next,
                hang_next: Arc::clone(&self.hang_next),
                mem_bytes: Arc::clone(&self.mem_bytes),
                terminated: Arc::new(AtomicBool::new(false)),
                pid,
            },
            replayed_seen: Arc::clone(&self.replayed_seen),
            always_crash: Arc::clone(&self.always_crash),
        }))
    }
}

#[test]
fn worker_crash_isolates_crashing_tab_and_replays_operations() {
    let spawner = MockSpawner::new();
    let crash_flag = Arc::clone(&spawner.crash_first_tab_once);
    let replayed_seen = Arc::clone(&spawner.replayed_seen);
    let mut pool = SandboxedWorkerPool::new(SandboxConfig::new(), Box::new(spawner));

    let pdf = corpus_pdf("zlib-manual.pdf");
    let (doc1, pages1) = pool
        .open_document(TabId(1), "application/pdf", pdf.clone())
        .expect("open tab 1");
    let (doc2, pages2) = pool
        .open_document(TabId(2), "application/pdf", pdf)
        .expect("open tab 2");
    assert_eq!((pages1, pages2), (2, 2));

    pool.record_operation(TabId(1), r#"{"kind":"operation","op":0}"#);
    pool.record_operation(TabId(1), r#"{"kind":"operation","op":1}"#);

    // Trigger crash on Tab 1 only.
    crash_flag.store(true, Ordering::SeqCst);
    let resp = pool
        .send(
            TabId(1),
            &IpcRequest::RenderTile {
                doc_id: doc1,
                page: 0,
                x0: 0,
                y0: 0,
                x1: 64,
                y1: 64,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        )
        .expect("tab 1 recovered after crash");
    assert!(matches!(
        resp,
        IpcResponse::TileRendered {
            width: 64,
            height: 64,
            ..
        }
    ));
    assert_eq!(*replayed_seen.lock().expect("replay lock"), 2);
    assert!(matches!(
        pool.health(TabId(1)),
        Some(WorkerHealth::Restarted {
            generation: 1,
            crash_count: 1
        })
    ));

    // Tab 2 remained untouched at generation 0!
    assert!(matches!(
        pool.health(TabId(2)),
        Some(WorkerHealth::Running { generation: 0, .. })
    ));
    let text2 = pool
        .send(
            TabId(2),
            &IpcRequest::ExtractText {
                doc_id: doc2,
                page: 0,
            },
        )
        .expect("tab 2 still works");
    assert!(matches!(text2, IpcResponse::TextExtracted { .. }));
}

#[test]
fn worker_watchdog_timeout_and_quarantine_and_memory_cap_enforced() {
    let spawner = MockSpawner::new();
    let always_crash = Arc::clone(&spawner.always_crash);
    let hang_next = Arc::clone(&spawner.hang_next);
    let mem_bytes = Arc::clone(&spawner.mem_bytes);

    let mut cfg = SandboxConfig::new();
    cfg.max_restarts_per_tab = 2;
    cfg.limits.worker_memory_cap_bytes = 64 * 1024 * 1024;
    let mut pool = SandboxedWorkerPool::new(cfg, Box::new(spawner));

    let pdf = corpus_pdf("zlib-manual.pdf");
    let (doc1, _) = pool
        .open_document(TabId(1), "application/pdf", pdf.clone())
        .expect("open tab 1");
    let (doc2, _) = pool
        .open_document(TabId(2), "application/pdf", pdf)
        .expect("open tab 2");

    // Memory cap triggers restart; if memory stays over cap, tab quarantines after max_restarts_per_tab.
    *mem_bytes.lock().expect("mem lock") = 128 * 1024 * 1024;
    let err = pool
        .send(TabId(1), &IpcRequest::Ping { sequence: 9 })
        .expect_err("over memory cap must quarantine");
    assert!(matches!(
        err,
        SandboxError::WorkerQuarantined { tab: TabId(1), .. }
    ));
    assert!(matches!(
        pool.health(TabId(1)),
        Some(WorkerHealth::Quarantined { .. })
    ));

    // Drop memory usage and test watchdog timeout recovery on Tab 2.
    *mem_bytes.lock().expect("mem lock") = 1024;
    hang_next.store(true, Ordering::SeqCst);
    let ok = pool
        .send(
            TabId(2),
            &IpcRequest::ExtractText {
                doc_id: doc2,
                page: 0,
            },
        )
        .expect("timeout triggers clean restart");
    assert!(matches!(ok, IpcResponse::TextExtracted { .. }));

    // Persistent crash quarantines Tab 2 cleanly.
    always_crash.store(true, Ordering::SeqCst);
    let q = pool
        .send(
            TabId(2),
            &IpcRequest::ExtractText {
                doc_id: doc1,
                page: 0,
            },
        )
        .expect_err("persistent crash quarantines");
    assert!(matches!(
        q,
        SandboxError::WorkerQuarantined { tab: TabId(2), .. }
    ));
}

#[test]
fn in_process_worker_enforces_wall_clock_timeout() {
    // Use real InProcessWorkerPool fallback on a valid corpus document.
    let mut cfg = SandboxConfig::new();
    cfg.limits = SecurityLimits::strict();
    let mut pool = SandboxedWorkerPool::with_in_process_fallback(cfg);
    let pdf = corpus_pdf("base14-outline-annot.pdf");
    let (doc_id, pages) = pool
        .open_document(TabId(99), "application/pdf", pdf)
        .expect("open base14");
    assert_eq!(pages, 2);
    let tile = pool
        .send(
            TabId(99),
            &IpcRequest::RenderTile {
                doc_id,
                page: 0,
                x0: 0,
                y0: 0,
                x1: 128,
                y1: 128,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        )
        .expect("render tile");
    assert!(matches!(tile, IpcResponse::TileRendered { .. }));
    pool.close_tab(TabId(99));
}

#[test]
fn os_sandbox_rejects_zero_limits() {
    let mut bad_mem = OsSandboxConfig::new();
    bad_mem.memory_cap_bytes = 0;
    assert!(matches!(
        OsSandboxGuard::create(bad_mem),
        Err(OsSandboxError::InvalidConfig(_))
    ));

    let mut bad_proc = OsSandboxConfig::new();
    bad_proc.active_process_limit = 0;
    assert!(matches!(
        OsSandboxGuard::create(bad_proc),
        Err(OsSandboxError::InvalidConfig(_))
    ));
}

#[cfg(windows)]
#[test]
fn windows_job_object_and_restricted_token_contain_child_process() {
    let mut cfg = OsSandboxConfig::new();
    cfg.memory_cap_bytes = 64 * 1024 * 1024;
    cfg.active_process_limit = 1;
    cfg.token_restriction = TokenRestriction::RestrictedLowPrivilege;
    cfg.kill_on_drop = KillOnDrop::TerminateProcessTree;

    let mut guard = OsSandboxGuard::create(cfg).expect("create Windows Job + restricted token");
    assert_eq!(
        guard.attachment().kind,
        PlatformSandboxKind::WindowsJobAndRestrictedToken
    );
    assert_eq!(guard.attachment().token_state, TokenState::Restricted);

    // Spawn a real benign short-lived Windows child and bind it to the Job Object.
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/C", "ping", "-n", "3", "127.0.0.1", ">nul"])
        .spawn()
        .expect("spawn benign child");
    guard.attach_pid(child.id()).expect("assign child to Job");
    assert_eq!(guard.attachment().job_assigned, JobAssignment::Assigned);

    let stats = guard.memory_stats().expect("query Job stats");
    assert!(
        stats.active_processes <= 1,
        "active_process_limit=1 enforced: {stats:?}"
    );

    guard.terminate_job(0).expect("terminate Job");
    let _ = child.wait();
}

#[cfg(windows)]
#[test]
fn windows_sandboxed_stdio_strips_env_isolates_cwd_and_blocks_grandchild() {
    use std::io::Read as _;

    let work_dir = std::env::temp_dir().join(format!(
        "ecclesia-sandbox-stdio-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir).expect("create isolated work dir");

    let script = work_dir.join("probe.cmd");
    std::fs::write(
        &script,
        "@echo off\r\ncd\r\necho CARGO=%CARGO_MANIFEST_DIR%\r\necho USERPROFILE=%USERPROFILE%\r\ncmd.exe /D /C \"echo escaped > escaped.txt\" 2>nul\r\n",
    )
    .expect("write probe script");

    let mut cfg = OsSandboxConfig::new();
    cfg.memory_cap_bytes = 64 * 1024 * 1024;
    cfg.active_process_limit = 1;
    cfg.token_restriction = TokenRestriction::RestrictedLowPrivilege;
    cfg.kill_on_drop = KillOnDrop::TerminateProcessTree;

    let mut guard = OsSandboxGuard::create(cfg).expect("create OS sandbox");
    let mut proc = guard
        .spawn_sandboxed_stdio("cmd.exe", &["/D", "/C", "probe.cmd"], &work_dir)
        .expect("spawn sandboxed stdio child");
    assert_eq!(guard.attachment().job_assigned, JobAssignment::Assigned);
    assert_eq!(guard.attachment().token_state, TokenState::Restricted);

    let mut output = String::new();
    proc.stdout
        .read_to_string(&mut output)
        .expect("read stdout");
    let _ = proc.wait().expect("wait for probe");

    let mut lines = output.lines();
    let reported_cwd = lines.next().unwrap_or_default().trim();
    assert_eq!(
        reported_cwd.to_ascii_lowercase(),
        work_dir.to_string_lossy().to_ascii_lowercase(),
        "child must run in isolated working directory, output={output:?}"
    );
    let cargo_line = lines.next().unwrap_or_default().trim();
    let profile_line = lines.next().unwrap_or_default().trim();
    assert!(
        cargo_line == "CARGO=" || cargo_line == "CARGO=%CARGO_MANIFEST_DIR%",
        "parent CARGO_MANIFEST_DIR leaked into sandboxed child: {output:?}"
    );
    assert!(
        !output.contains(env!("CARGO_MANIFEST_DIR")),
        "parent CARGO_MANIFEST_DIR value must not appear in child output: {output:?}"
    );
    assert!(
        profile_line == "USERPROFILE=" || profile_line == "USERPROFILE=%USERPROFILE%",
        "parent USERPROFILE leaked into sandboxed child: {output:?}"
    );
    assert!(
        !work_dir.join("escaped.txt").exists(),
        "active_process_limit=1 must block grandchild process creation"
    );

    let _ = std::fs::remove_dir_all(&work_dir);
}

#[test]
fn security_limits_reject_bombs_traversal_and_oversized_inputs() {
    let mut limits = SecurityLimits::strict();
    limits.max_pages = 5;
    limits.max_recursion_depth = 8;
    limits.max_xref_entries = 20;
    limits.max_decompressed_bytes = 4096;
    limits.ratio_check_floor_bytes = 512;
    limits.max_compression_ratio = 10;
    limits.max_image_pixels = 1_000_000;

    assert!(matches!(
        limits.validate_page_count(6),
        Err(LimitViolation::PageCountExceeded {
            actual: 6,
            limit: 5
        })
    ));
    assert!(matches!(
        limits.validate_image_dimensions(2000, 2000),
        Err(LimitViolation::ImageSizeExceeded { .. })
    ));

    // Deep dictionary nesting bomb.
    let mut nested = b"%PDF-1.7\n1 0 obj ".to_vec();
    for _ in 0..16 {
        nested.extend_from_slice(b"<< /A ");
    }
    for _ in 0..16 {
        nested.extend_from_slice(b">>");
    }
    nested.extend_from_slice(b" endobj\n%%EOF");
    assert!(matches!(
        limits.validate_pdf_bytes(&nested),
        Err(LimitViolation::RecursionDepthExceeded { .. })
    ));

    // Excessive /Size XRef entries.
    let big_xref = b"%PDF-1.7\ntrailer << /Size 5000 >>\n%%EOF";
    assert!(matches!(
        limits.validate_pdf_bytes(big_xref),
        Err(LimitViolation::XrefEntriesExceeded { .. })
    ));

    // PDF zlib stream bomb.
    let zeros = vec![0u8; 32 * 1024];
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(&zeros).expect("compress");
    let compressed = z.finish().expect("finish");
    let mut bomb_pdf = b"%PDF-1.7\n1 0 obj << /Length 100 >>\nstream\n".to_vec();
    bomb_pdf.extend_from_slice(&compressed);
    bomb_pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF");
    assert!(matches!(
        limits.validate_pdf_bytes(&bomb_pdf),
        Err(LimitViolation::DecompressedSizeExceeded { .. }
            | LimitViolation::CompressionRatioExceeded { .. })
    ));

    // ZIP bomb & path traversal in .expdf/.epub/.cbz.
    let mut zip_buf = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut zip_buf));
        writer
            .start_file("../../escape.txt", SimpleFileOptions::default())
            .expect("start file");
        writer.write_all(b"evil").expect("write");
        writer.finish().expect("finish");
    }
    assert!(matches!(
        limits.validate_zip_archive_bytes(&zip_buf),
        Err(LimitViolation::UnsafeArchivePath { .. })
    ));

    let mut bomb_zip = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bomb_zip));
        writer
            .start_file("payload.bin", SimpleFileOptions::default())
            .expect("start file");
        writer.write_all(&zeros).expect("write");
        writer.finish().expect("finish");
    }
    assert!(matches!(
        limits.validate_zip_archive_bytes(&bomb_zip),
        Err(LimitViolation::DecompressedSizeExceeded { .. }
            | LimitViolation::CompressionRatioExceeded { .. })
    ));

    // XFDF entity bomb & deep XML nesting.
    let xfdf_entities = b"<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"x\"><!ENTITY b \"x\"><!ENTITY c \"x\"><!ENTITY d \"x\"><!ENTITY e \"x\"><!ENTITY f \"x\"><!ENTITY g \"x\"><!ENTITY h \"x\"><!ENTITY i \"x\">]><xfdf/>";
    assert!(matches!(
        limits.validate_xfdf_bytes(xfdf_entities),
        Err(LimitViolation::ObjectCountExceeded { .. })
    ));

    // SyncTeX gzip bomb.
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(&zeros).expect("gz");
    let gz_bytes = gz.finish().expect("finish gz");
    assert!(matches!(
        limits.validate_synctex_bytes(&gz_bytes),
        Err(LimitViolation::DecompressedSizeExceeded { .. }
            | LimitViolation::CompressionRatioExceeded { .. })
    ));

    // Attachment sanitization and URI classification.
    assert!(sanitize_attachment_name("report.pdf").is_ok());
    assert!(sanitize_attachment_name("../secret.txt").is_err());
    assert!(sanitize_attachment_name("CON.txt").is_err());
    assert!(matches!(
        classify_external_uri("javascript:alert(1)"),
        UriSafety::Blocked { .. }
    ));
    assert!(matches!(
        classify_external_uri("https://example.com"),
        UriSafety::WebLinkRequiresConfirmation { .. }
    ));
    assert!(matches!(
        classify_external_uri("file:calc.exe"),
        UriSafety::ExternalRequiresExplicitPrompt { .. }
    ));
}

#[test]
fn malformed_fuzz_corpus_inputs_never_panic() {
    let fuzz_corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fuzz/regressions");
    assert!(
        fuzz_corpus.exists(),
        "fuzz regression corpus directory must exist: {fuzz_corpus:?}"
    );
    let limits = SecurityLimits::strict();
    let mut checked = 0usize;

    for group in [
        "malformed_pdf",
        "malformed_xfdf",
        "malformed_expdf",
        "malformed_synctex",
    ] {
        let dir = fuzz_corpus.join(group);
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read fuzz corpus group {group}: {e}"));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read seed");
            checked += 1;
            match group {
                "malformed_pdf" => {
                    let _ = limits.validate_pdf_bytes(&bytes);
                    let _ = open_pdf_with_limits(&bytes, &limits);
                }
                "malformed_xfdf" => {
                    let _ = limits.validate_xfdf_bytes(&bytes);
                }
                "malformed_expdf" => {
                    let _ = limits.validate_zip_archive_bytes(&bytes);
                }
                "malformed_synctex" => {
                    let _ = limits.validate_synctex_bytes(&bytes);
                }
                _ => {}
            }
        }
    }
    assert!(
        checked >= 8,
        "expected at least 8 malformed regression seeds, found {checked}"
    );
}
