//! Integration tests for `ecclesia-cli` subcommands, tool scripts, MCP server, and control channel.

use std::fs;
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ecclesia_cli::{CliExit, build_full_tool_registry, run_cli_with_io};
use ecclesia_core::tools::McpServer;
use ecclesia_core::tools::control::{
    ControlCapability, ControlClient, ControlCommand, ControlErrorCode, ControlToken,
    DocumentControlHost, FrameCodec, LoopbackControlServer, LoopbackTransport,
};
use ecclesia_core::tools::{ToolContext, validate_json};
use ecclesia_mupdf::Document as MuDocument;
use serde_json::{Value, json};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ecclesia-cli-test-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn run_ok(args: &[&str]) -> String {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_cli_with_io(args, &mut out, &mut err);
    assert_eq!(
        code,
        CliExit::Success,
        "command {:?} failed: {}",
        args,
        String::from_utf8_lossy(&err)
    );
    String::from_utf8(out).expect("utf8 stdout")
}

#[test]
fn cli_subcommands_run_end_to_end_on_corpus() {
    let dir = temp_dir("subcommands");
    let base_pdf = corpus("base14-outline-annot.pdf");
    let zlib_pdf = corpus("zlib-manual.pdf");
    let vector_pdf = corpus("heavy-vector.pdf");
    let base_str = base_pdf.to_string_lossy().into_owned();
    let zlib_str = zlib_pdf.to_string_lossy().into_owned();
    let vector_str = vector_pdf.to_string_lossy().into_owned();

    // 1. info --json
    let info_out = run_ok(&["ecclesia-cli", "--json", "info", &base_str]);
    let info_json: Value = serde_json::from_str(&info_out).expect("info json");
    assert_eq!(info_json["page_count"], 2);
    assert_eq!(info_json["is_pdf"], true);
    assert_eq!(info_json["outline_count"], 2);

    // 2. text --json
    let text_out = run_ok(&["ecclesia-cli", "text", &base_str, "--json"]);
    let text_json: Value = serde_json::from_str(&text_out).expect("text json");
    assert!(
        !text_json["full_text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
    );

    // 3. search --json
    let search_out = run_ok(&["ecclesia-cli", "--json", "search", &zlib_str, "deflate"]);
    let search_json: Value = serde_json::from_str(&search_out).expect("search json");
    assert!(search_json["total_hits"].as_u64().is_some_and(|n| n > 0));

    // 4. merge --json
    let merged_path = dir.join("merged.pdf");
    let merged_str = merged_path.to_string_lossy().into_owned();
    let merge_out = run_ok(&[
        "ecclesia-cli",
        "--json",
        "merge",
        &base_str,
        &vector_str,
        "-o",
        &merged_str,
    ]);
    let merge_json: Value = serde_json::from_str(&merge_out).expect("merge json");
    assert_eq!(merge_json["pages_written"], 3);
    assert_eq!(
        MuDocument::open(&merged_path)
            .expect("open merged")
            .page_count()
            .expect("pages"),
        3
    );

    // 5. split --json
    let split_dir = dir.join("split");
    let split_dir_str = split_dir.to_string_lossy().into_owned();
    let split_out = run_ok(&[
        "ecclesia-cli",
        "--json",
        "split",
        &base_str,
        "--out-dir",
        &split_dir_str,
    ]);
    let split_json: Value = serde_json::from_str(&split_out).expect("split json");
    assert_eq!(split_json["files"].as_array().map(Vec::len), Some(2));

    // 6. compress --json
    let compressed_path = dir.join("compressed.pdf");
    let compressed_str = compressed_path.to_string_lossy().into_owned();
    let compress_out = run_ok(&[
        "ecclesia-cli",
        "--json",
        "compress",
        &zlib_str,
        "-o",
        &compressed_str,
        "--preset",
        "ebook",
    ]);
    let compress_json: Value = serde_json::from_str(&compress_out).expect("compress json");
    assert!(compress_json["bytes_after"].as_u64().is_some_and(|b| b > 0));
    assert!(MuDocument::open(&compressed_path).is_ok());

    // 7. ocr --engine test --json (adds searchable Tr 3 layer to heavy-vector.pdf)
    let ocr_path = dir.join("ocr.pdf");
    let ocr_str = ocr_path.to_string_lossy().into_owned();
    let ocr_out = run_ok(&[
        "ecclesia-cli",
        "--json",
        "ocr",
        &vector_str,
        "-o",
        &ocr_str,
        "--engine",
        "test",
        "--test-text",
        "VERIFIED_CLI_OCR_TEXT",
    ]);
    let ocr_json: Value = serde_json::from_str(&ocr_out).expect("ocr json");
    assert_eq!(ocr_json["pages_ocred"], 1);
    let ocr_doc = MuDocument::open(&ocr_path).expect("reopen ocr");
    let extracted = ocr_doc
        .load_page(0)
        .expect("p0")
        .text(None)
        .expect("stext")
        .text()
        .expect("text");
    assert!(extracted.contains("VERIFIED_CLI_OCR_TEXT"));

    // 8. redact --pattern ... --json (and verify zero remaining occurrences)
    let redacted_path = dir.join("redacted.pdf");
    let redacted_str = redacted_path.to_string_lossy().into_owned();
    let redact_out = run_ok(&[
        "ecclesia-cli",
        "--json",
        "redact",
        &ocr_str,
        "-o",
        &redacted_str,
        "--pattern",
        "VERIFIED_CLI_OCR_TEXT",
    ]);
    let redact_json: Value = serde_json::from_str(&redact_out).expect("redact json");
    assert_eq!(redact_json["verified_clean"], true);
    assert!(
        redact_json["redactions_applied"]
            .as_u64()
            .is_some_and(|n| n >= 1)
    );

    // 9. export (xfdf, w3c, markdown, flattened)
    let xfdf_path = dir.join("annots.xfdf");
    let xfdf_str = xfdf_path.to_string_lossy().into_owned();
    run_ok(&[
        "ecclesia-cli",
        "--json",
        "export",
        &base_str,
        "--format",
        "xfdf",
        "-o",
        &xfdf_str,
    ]);
    let xfdf_xml = fs::read_to_string(&xfdf_path).expect("read xfdf");
    assert!(xfdf_xml.contains("<xfdf"));

    let md_path = dir.join("doc.md");
    let md_str = md_path.to_string_lossy().into_owned();
    run_ok(&[
        "ecclesia-cli",
        "export",
        &base_str,
        "--format",
        "markdown",
        "-o",
        &md_str,
    ]);
    let md_text = fs::read_to_string(&md_path).expect("read md");
    assert!(md_text.contains("## Page 1"));

    let flat_path = dir.join("flat.pdf");
    let flat_str = flat_path.to_string_lossy().into_owned();
    run_ok(&[
        "ecclesia-cli",
        "--json",
        "export",
        &base_str,
        "--format",
        "flattened",
        "-o",
        &flat_str,
    ]);
    assert!(MuDocument::open(&flat_path).is_ok());

    // 10. render --page 0 -o out.png
    let png_path = dir.join("page0.png");
    let png_str = png_path.to_string_lossy().into_owned();
    run_ok(&[
        "ecclesia-cli",
        "--json",
        "render",
        &base_str,
        "--page",
        "0",
        "--width",
        "160",
        "-o",
        &png_str,
    ]);
    let png_bytes = fs::read(&png_path).expect("read png");
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");

    // 11. run <tool> --json AND run <script.json>
    let inline_args = json!({ "path": base_str }).to_string();
    let run_single = run_ok(&["ecclesia-cli", "run", "doc.info", "--json", &inline_args]);
    let run_single_json: Value = serde_json::from_str(&run_single).expect("run single");
    assert_eq!(run_single_json["page_count"], 2);

    let script_path = dir.join("script.json");
    let script_body = json!([
        { "tool": "doc.info", "arguments": { "path": base_str } },
        { "tool": "doc.outline", "arguments": { "path": base_str } }
    ]);
    fs::write(&script_path, script_body.to_string()).expect("write script");
    let script_out = run_ok(&["ecclesia-cli", "run", &script_path.to_string_lossy()]);
    let script_json: Value = serde_json::from_str(&script_out).expect("script json");
    assert_eq!(script_json["ok"], true);
    assert_eq!(script_json["results"].as_array().map(Vec::len), Some(2));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn all_full_registry_tools_have_valid_schemas_and_examples() {
    let reg = build_full_tool_registry();
    for spec in reg.specs() {
        assert!(
            validate_json(&spec.example_input, &spec.input_schema).is_ok(),
            "schema validation failed for {}",
            spec.name
        );
    }
}

#[test]
fn binary_process_mcp_and_exit_codes_work() {
    let bin = env!("CARGO_BIN_EXE_ecclesia-cli");
    let corpus_dir = fs::canonicalize(corpus("base14-outline-annot.pdf"))
        .expect("corpus")
        .parent()
        .expect("parent")
        .to_path_buf();

    // Invalid CLI args -> exit code 2.
    let bad = Command::new(bin)
        .arg("nonexistent-cmd")
        .output()
        .expect("run binary");
    assert_eq!(bad.status.code(), Some(2));

    // Real MCP subprocess over stdin/stdout with --root confinement.
    let mut child = Command::new(bin)
        .args(["mcp", "--root", &corpus_dir.to_string_lossy()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp");

    let reqs = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }).to_string(),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "doc.info", "arguments": { "path": "base14-outline-annot.pdf" } } }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "doc.info", "arguments": { "path": "../../Cargo.toml" } } }).to_string(),
    ]
    .join("\n");

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(reqs.as_bytes()).expect("write stdin");
        stdin.write_all(b"\n").expect("newline");
    }
    let out = child.wait_with_output().expect("wait mcp");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<Value> = String::from_utf8(out.stdout)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["result"]["isError"], false);
    assert_eq!(lines[1]["result"]["structuredContent"]["page_count"], 2);
    assert_eq!(lines[2]["result"]["isError"], true);
}

#[test]
fn document_control_host_and_mcp_work_together() {
    let reg = build_full_tool_registry();
    let corpus_dir = fs::canonicalize(corpus("base14-outline-annot.pdf"))
        .expect("corpus")
        .parent()
        .expect("parent")
        .to_path_buf();

    // Also verify McpServer directly with the full registry.
    let mut server = McpServer::new(&reg, Some(&corpus_dir)).expect("mcp");
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "annot.export",
            "arguments": { "path": "base14-outline-annot.pdf", "format": "xfdf" }
        }
    })
    .to_string();
    server
        .serve(Cursor::new(req.as_bytes()), &mut out)
        .expect("serve");
    let resp: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(resp["result"]["isError"], false);
    assert_eq!(resp["result"]["structuredContent"]["format"], "xfdf");

    // Verify DocumentControlHost over real loopback server.
    let token = ControlToken::generate(ControlCapability::all());
    let server = LoopbackControlServer::bind(
        "127.0.0.1:0".parse().expect("addr"),
        vec![token.clone()],
        FrameCodec::Ndjson,
    )
    .expect("bind");
    let addr = server.addr();
    let handle = std::thread::spawn(move || {
        let mut host = DocumentControlHost::new(reg, ToolContext::default());
        server.serve(&mut host, Some(4)).expect("serve 4");
    });

    let mut client = ControlClient::new(
        LoopbackTransport::new(addr, FrameCodec::Ndjson),
        token.as_str(),
    );
    let open = client
        .send(ControlCommand::OpenDocument {
            path: corpus("base14-outline-annot.pdf")
                .to_string_lossy()
                .into_owned(),
        })
        .expect("open");
    assert!(open.ok);
    assert_eq!(open.result["page_count"], 2);

    let jump = client
        .send(ControlCommand::JumpToPage { page: 1 })
        .expect("jump");
    assert!(jump.ok);
    assert_eq!(jump.result["current_page"], 1);

    let shot = client
        .send(ControlCommand::Screenshot {
            output: None,
            page: None,
            dpi: Some(72),
        })
        .expect("screenshot");
    assert!(shot.ok);
    assert!(
        shot.result["png_base64"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );

    let mut bad_client = ControlClient::new(
        LoopbackTransport::new(addr, FrameCodec::Ndjson),
        "bad-token",
    );
    let denied = bad_client
        .send(ControlCommand::InspectState)
        .expect("denied");
    assert!(!denied.ok);
    assert_eq!(
        denied.error.as_ref().map(|e| e.code),
        Some(ControlErrorCode::Unauthorized)
    );

    handle.join().expect("join");
}
