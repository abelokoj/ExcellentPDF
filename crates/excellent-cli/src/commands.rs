//! Execution of every `excellent-cli` subcommand (plan E.1, E.2, E.3, E.4).

use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::Path;

use excellent_core::tools::builtin::parse_page_spec_1based;
use excellent_core::tools::control::{
    ControlCapability, ControlClient, ControlCommand, ControlEndpoint, ControlToken,
    DocumentControlHost, FrameCodec, LoopbackControlServer, LoopbackTransport, MockAppControlHost,
};
use excellent_core::tools::{ScriptFailurePolicy, ToolCall, ToolContext};
use excellent_mupdf::Document as MuDocument;
use serde_json::{Value, json};

use crate::args::{Command, ControlAction, HELP_TEXT, ParsedCli};
use crate::build_full_tool_registry;
use crate::error::{CliError, CliExit};
use crate::mcp;

/// Executes a parsed CLI command, writing formatted text or JSON to `stdout`.
///
/// # Errors
///
/// Returns [`CliError`] with an appropriate exit code on usage, I/O, processing, or verification failure.
pub fn execute<W: Write>(cli: &ParsedCli, stdout: &mut W) -> Result<(), CliError> {
    match &cli.command {
        Command::Help => {
            if cli.json {
                write_json(stdout, &json!({ "help": HELP_TEXT }))?;
            } else {
                writeln!(stdout, "{HELP_TEXT}")?;
            }
            Ok(())
        }
        Command::Version => {
            let ver = env!("CARGO_PKG_VERSION");
            if cli.json {
                write_json(stdout, &json!({ "name": "excellent-cli", "version": ver }))?;
            } else {
                writeln!(stdout, "excellent-cli {ver}")?;
            }
            Ok(())
        }
        Command::Info { input } => cmd_info(input, cli.json, stdout),
        Command::Text {
            input,
            pages,
            output,
        } => cmd_text(input, pages.as_deref(), output.as_deref(), cli.json, stdout),
        Command::Search {
            input,
            query,
            case_sensitive,
            whole_word,
            max_hits,
        } => cmd_search(
            input,
            query,
            *case_sensitive,
            *whole_word,
            *max_hits,
            cli.json,
            stdout,
        ),
        Command::Merge { inputs, output } => cmd_merge(inputs, output, cli.json, stdout),
        Command::Split {
            input,
            out_dir,
            ranges,
            prefix,
        } => cmd_split(input, out_dir, ranges, prefix.as_deref(), cli.json, stdout),
        Command::Compress {
            input,
            output,
            preset,
            linearize,
        } => cmd_compress(input, output, preset, *linearize, cli.json, stdout),
        Command::Ocr { .. } => cmd_ocr(&cli.command, cli.json, stdout),
        Command::Redact { .. } => cmd_redact(&cli.command, cli.json, stdout),
        Command::Export {
            input,
            format,
            output,
        } => cmd_export(input, format, output.as_deref(), cli.json, stdout),
        Command::Render { .. } => cmd_render(&cli.command, cli.json, stdout),
        Command::Run {
            target,
            json_arg,
            list,
            root,
        } => cmd_run(
            target.as_deref(),
            json_arg.as_deref(),
            *list,
            root.as_deref(),
            cli.json,
            stdout,
        ),
        Command::Mcp { root } => mcp::serve_stdio(root.clone()),
        Command::Control { action } => cmd_control(action, cli.json, stdout),
    }
}

fn write_json<W: Write>(stdout: &mut W, value: &Value) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(value)?;
    writeln!(stdout, "{text}")?;
    Ok(())
}

fn path_str(path: &Path) -> Result<String, CliError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| CliError::usage("path is not valid UTF-8"))
}

fn cmd_info<W: Write>(input: &Path, json_out: bool, stdout: &mut W) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let res = reg.execute(&ctx, "doc.info", &json!({ "path": path_str(input)? }))?;
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(stdout, "File: {}", res["path"].as_str().unwrap_or(""))?;
    writeln!(stdout, "Pages: {}", res["page_count"])?;
    writeln!(stdout, "PDF: {}", res["is_pdf"])?;
    writeln!(stdout, "Outline entries: {}", res["outline_count"])?;
    writeln!(stdout, "Attachments: {}", res["attachment_count"])?;
    Ok(())
}

fn cmd_text<W: Write>(
    input: &Path,
    pages_spec: Option<&str>,
    output: Option<&Path>,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let mut args = json!({ "path": path_str(input)? });
    if let Some(spec) = pages_spec {
        let doc = MuDocument::open(input)?;
        let indices = parse_page_spec_1based(spec, doc.page_count()?)?;
        args["pages"] = json!(indices);
    }
    let res = reg.execute(&ctx, "doc.extract_text", &args)?;
    let full_text = res["full_text"].as_str().unwrap_or("");
    if let Some(out) = output {
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(out, full_text.as_bytes())?;
    }
    if json_out {
        return write_json(stdout, &res);
    }
    if output.is_none() {
        writeln!(stdout, "{full_text}")?;
    } else if let Some(out) = output {
        writeln!(stdout, "Wrote extracted text to {}", out.display())?;
    }
    Ok(())
}

fn cmd_search<W: Write>(
    input: &Path,
    query: &str,
    case_sensitive: bool,
    whole_word: bool,
    max_hits: Option<usize>,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let mut args = json!({
        "path": path_str(input)?,
        "query": query,
        "case_sensitive": case_sensitive,
        "whole_word": whole_word,
    });
    if let Some(max) = max_hits {
        args["max_hits"] = json!(max);
    }
    let res = reg.execute(&ctx, "doc.search", &args)?;
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(stdout, "Found {} hit(s) for {:?}", res["total_hits"], query)?;
    if let Some(hits) = res["hits"].as_array() {
        for hit in hits {
            let page = hit["page"].as_u64().unwrap_or(0) + 1;
            let snippet = hit["snippet"].as_str().unwrap_or("");
            writeln!(stdout, "  p.{page}: {snippet}")?;
        }
    }
    Ok(())
}

fn cmd_merge<W: Write>(
    inputs: &[std::path::PathBuf],
    output: &Path,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let input_strs: Vec<String> = inputs
        .iter()
        .map(|p| path_str(p))
        .collect::<Result<_, _>>()?;
    let res = reg.execute(
        &ctx,
        "doc.merge",
        &json!({
            "inputs": input_strs,
            "output": path_str(output)?,
        }),
    )?;
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(
        stdout,
        "Merged {} pages into {} ({} bytes)",
        res["pages_written"],
        output.display(),
        res["bytes"]
    )?;
    Ok(())
}

fn cmd_split<W: Write>(
    input: &Path,
    out_dir: &Path,
    ranges: &[String],
    prefix: Option<&str>,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let mut args = json!({
        "path": path_str(input)?,
        "out_dir": path_str(out_dir)?,
    });
    if !ranges.is_empty() {
        args["ranges"] = json!(ranges);
    }
    if let Some(p) = prefix {
        args["prefix"] = json!(p);
    }
    let res = reg.execute(&ctx, "doc.split", &args)?;
    if json_out {
        return write_json(stdout, &res);
    }
    let count = res["files"].as_array().map_or(0, Vec::len);
    writeln!(
        stdout,
        "Split into {count} file(s) in {}",
        out_dir.display()
    )?;
    Ok(())
}

fn cmd_compress<W: Write>(
    input: &Path,
    output: &Path,
    preset: &str,
    linearize: bool,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let res = reg.execute(
        &ctx,
        "doc.compress",
        &json!({
            "path": path_str(input)?,
            "output": path_str(output)?,
            "preset": preset,
            "linearize": linearize,
        }),
    )?;
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(
        stdout,
        "Compressed {} -> {} ({} -> {} bytes, saved {})",
        input.display(),
        output.display(),
        res["bytes_before"],
        res["bytes_after"],
        res["saved_bytes"]
    )?;
    Ok(())
}

fn cmd_ocr<W: Write>(cmd: &Command, json_out: bool, stdout: &mut W) -> Result<(), CliError> {
    let Command::Ocr {
        input,
        output,
        languages,
        dpi,
        deskew,
        engine,
        skip_existing_text,
        test_text,
    } = cmd
    else {
        return Err(CliError::usage("expected ocr command"));
    };
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let mut args = json!({
        "path": path_str(input)?,
        "output": path_str(output)?,
        "languages": languages,
        "dpi": dpi,
        "deskew": deskew,
        "engine": engine,
        "skip_existing_text": skip_existing_text,
    });
    if let Some(text) = test_text {
        args["test_text"] = json!(text);
    }
    let res = reg.execute(&ctx, "doc.ocr", &args)?;
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(
        stdout,
        "OCR'd {}/{} page(s), placed {} word(s) -> {}",
        res["pages_ocred"],
        res["page_count"],
        res["total_words"],
        output.display()
    )?;
    Ok(())
}

fn cmd_redact<W: Write>(cmd: &Command, json_out: bool, stdout: &mut W) -> Result<(), CliError> {
    let Command::Redact {
        input,
        output,
        patterns,
        rects,
        case_sensitive,
        dpi,
    } = cmd
    else {
        return Err(CliError::usage("expected redact command"));
    };
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let parsed_rects: Vec<Value> = rects
        .iter()
        .map(|spec| parse_rect_spec(spec))
        .collect::<Result<_, _>>()?;
    let mut args = json!({
        "path": path_str(input)?,
        "output": path_str(output)?,
        "case_sensitive": case_sensitive,
        "dpi": dpi,
    });
    if !patterns.is_empty() {
        args["patterns"] = json!(patterns);
    }
    if !parsed_rects.is_empty() {
        args["rects"] = json!(parsed_rects);
    }
    let res = reg.execute(&ctx, "doc.redact", &args)?;
    if res["verified_clean"].as_bool() != Some(true) {
        return Err(CliError::new(
            CliExit::VerificationOrAuth,
            "redaction verification failed: target text still detectable",
        ));
    }
    if json_out {
        return write_json(stdout, &res);
    }
    writeln!(
        stdout,
        "Applied {} redaction(s) -> {} (verified clean)",
        res["redactions_applied"],
        output.display()
    )?;
    Ok(())
}

fn parse_rect_spec(spec: &str) -> Result<Value, CliError> {
    let (page_str, coords_str) = spec.split_once(':').ok_or_else(|| {
        CliError::usage(format!(
            "invalid --rect '{spec}' (expected page:x0,y0,x1,y1)"
        ))
    })?;
    let page: usize = page_str
        .trim()
        .parse()
        .map_err(|_| CliError::usage(format!("invalid page in --rect '{spec}'")))?;
    let nums: Vec<f64> = coords_str
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<f64>()
                .map_err(|_| CliError::usage(format!("invalid coordinate in --rect '{spec}'")))
        })
        .collect::<Result<_, _>>()?;
    if nums.len() != 4 {
        return Err(CliError::usage(format!(
            "--rect '{spec}' must have 4 coordinates"
        )));
    }
    Ok(json!({ "page": page, "rect": nums }))
}

fn cmd_export<W: Write>(
    input: &Path,
    format: &str,
    output: Option<&Path>,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    match format {
        "xfdf" | "fdf" | "w3c" | "json" => {
            let mut args = json!({
                "path": path_str(input)?,
                "format": format,
            });
            if let Some(out) = output {
                args["output"] = json!(path_str(out)?);
            }
            let res = reg.execute(&ctx, "annot.export", &args)?;
            if json_out {
                return write_json(stdout, &res);
            }
            if let Some(out) = output {
                writeln!(
                    stdout,
                    "Exported {} annotation(s) ({format}) -> {}",
                    res["count"],
                    out.display()
                )?;
            } else if format == "xfdf" {
                writeln!(stdout, "{}", res["data"]["xfdf"].as_str().unwrap_or(""))?;
            } else {
                write_json(stdout, &res["data"])?;
            }
            Ok(())
        }
        "markdown" => {
            let doc = MuDocument::open(input)?;
            let count = doc.page_count()?;
            let mut md = String::new();
            let title = input
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Document");
            let _ = writeln!(md, "# {title}\n");
            for page_idx in 0..count {
                let text = doc.load_page(page_idx)?.text(None)?.text()?;
                let _ = writeln!(md, "## Page {}\n\n{}\n", page_idx + 1, text.trim());
            }
            if let Some(out) = output {
                if let Some(parent) = out.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(out, md.as_bytes())?;
            }
            if json_out {
                return write_json(
                    stdout,
                    &json!({
                        "format": "markdown",
                        "page_count": count,
                        "output": output.map(|p| p.to_string_lossy().into_owned()),
                        "markdown": md,
                    }),
                );
            }
            if let Some(out) = output {
                writeln!(
                    stdout,
                    "Exported Markdown ({count} pages) -> {}",
                    out.display()
                )?;
            } else {
                write!(stdout, "{md}")?;
            }
            Ok(())
        }
        "flattened" => {
            let out = output.ok_or_else(|| {
                CliError::usage("export --format flattened requires -o <output.pdf>")
            })?;
            let res = reg.execute(
                &ctx,
                "doc.save",
                &json!({
                    "path": path_str(input)?,
                    "output": path_str(out)?,
                    "flatten": true,
                }),
            )?;
            if json_out {
                return write_json(stdout, &res);
            }
            writeln!(stdout, "Saved flattened PDF -> {}", out.display())?;
            Ok(())
        }
        other => Err(CliError::usage(format!(
            "unsupported export format '{other}' (expected xfdf, fdf, w3c, json, markdown, or flattened)"
        ))),
    }
}

fn cmd_render<W: Write>(cmd: &Command, json_out: bool, stdout: &mut W) -> Result<(), CliError> {
    let Command::Render {
        input,
        page,
        output,
        dpi,
        width,
        transparent,
    } = cmd
    else {
        return Err(CliError::usage("expected render command"));
    };
    let reg = build_full_tool_registry();
    let ctx = ToolContext::default();
    let mut args = json!({
        "path": path_str(input)?,
        "page": page,
        "transparent": transparent,
    });
    if let Some(d) = dpi {
        args["dpi"] = json!(d);
    }
    if let Some(w) = width {
        args["width"] = json!(w);
    }
    if let Some(out) = output {
        args["output"] = json!(path_str(out)?);
    }
    let res = reg.execute(&ctx, "page.render", &args)?;
    if json_out {
        return write_json(stdout, &res);
    }
    if let Some(out) = output {
        writeln!(
            stdout,
            "Rendered page {} ({}x{}, {} bytes) -> {}",
            page,
            res["width"],
            res["height"],
            res["bytes"],
            out.display()
        )?;
    } else {
        writeln!(
            stdout,
            "Rendered page {} ({}x{}, {} bytes base64 PNG)",
            page, res["width"], res["height"], res["bytes"]
        )?;
    }
    Ok(())
}

fn cmd_run<W: Write>(
    target: Option<&str>,
    json_arg: Option<&str>,
    list: bool,
    root: Option<&Path>,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let reg = build_full_tool_registry();
    if list {
        let specs: Vec<Value> = reg
            .specs()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "input_schema": s.input_schema,
                    "output_schema": s.output_schema,
                    "example_input": s.example_input,
                })
            })
            .collect();
        if json_out {
            return write_json(stdout, &json!({ "tools": specs }));
        }
        for spec in reg.specs() {
            writeln!(stdout, "{:<18} {}", spec.name, spec.description)?;
        }
        return Ok(());
    }

    let ctx = match root {
        Some(r) => ToolContext::with_root(r)?,
        None => ToolContext::new(),
    };

    let target = target.ok_or_else(|| CliError::usage("run requires a tool name or JSON file"))?;
    let target_path = Path::new(target);
    if target_path.exists() && json_arg.is_none() {
        let raw = fs::read_to_string(target_path)?;
        let val: Value = serde_json::from_str(&raw)?;
        if let Some(arr) = val.as_array() {
            let calls: Vec<ToolCall> = arr
                .iter()
                .map(ToolCall::from_value)
                .collect::<Result<_, _>>()?;
            let results = reg.execute_script(&ctx, &calls, ScriptFailurePolicy::StopOnError);
            let all_ok = results.iter().all(|r| r.ok);
            let payload = json!({
                "ok": all_ok,
                "results": results.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
            });
            write_json(stdout, &payload)?;
            if !all_ok {
                return Err(CliError::processing("one or more script steps failed"));
            }
            return Ok(());
        }
        let call = ToolCall::from_value(&val)?;
        let out = reg.execute(&ctx, &call.tool, &call.arguments)?;
        return write_json(stdout, &out);
    }

    let args: Value = match json_arg {
        Some(raw) => serde_json::from_str(raw)?,
        None => json!({}),
    };
    let out = reg.execute(&ctx, target, &args)?;
    write_json(stdout, &out)
}

fn parse_codec(raw: &str) -> Result<FrameCodec, CliError> {
    match raw {
        "ndjson" => Ok(FrameCodec::Ndjson),
        "length" | "length-prefixed" => Ok(FrameCodec::LengthPrefixed),
        other => Err(CliError::usage(format!(
            "invalid --codec '{other}' (expected ndjson or length)"
        ))),
    }
}

fn cmd_control<W: Write>(
    action: &ControlAction,
    json_out: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    match action {
        ControlAction::Serve {
            endpoint,
            token_file,
            codec,
            max_requests,
            mock,
        } => {
            let codec = parse_codec(codec)?;
            let ControlEndpoint::Tcp(addr) = ControlEndpoint::parse(endpoint)? else {
                return Err(CliError::usage(
                    "control serve currently supports tcp://127.0.0.1:<port> loopback endpoints",
                ));
            };
            let token = ControlToken::generate(ControlCapability::all());
            let server = LoopbackControlServer::bind(addr, vec![token.clone()], codec)?;
            let bound = server.addr();
            let ep_str = format!("tcp://{bound}");
            if let Some(tf) = token_file {
                token.write_token_file(tf, &ep_str)?;
            }
            if json_out {
                write_json(
                    stdout,
                    &json!({
                        "listening": ep_str,
                        "token_file": token_file.as_ref().map(|p| p.to_string_lossy().into_owned()),
                        "token": if token_file.is_none() { Some(token.as_str()) } else { None },
                    }),
                )?;
            } else {
                writeln!(stdout, "Control server listening on {ep_str}")?;
            }
            stdout.flush()?;

            if *mock {
                let mut host = MockAppControlHost::default();
                server.serve(&mut host, *max_requests)?;
            } else {
                let reg = build_full_tool_registry();
                let ctx = ToolContext::default();
                let mut host = DocumentControlHost::new(reg, ctx);
                server.serve(&mut host, *max_requests)?;
            }
            Ok(())
        }
        ControlAction::Send {
            endpoint,
            token,
            token_file,
            codec,
            command_json,
        } => {
            let codec = parse_codec(codec)?;
            let (ep_str, tok_str) = match (endpoint, token, token_file) {
                (Some(e), Some(t), _) => (e.clone(), t.clone()),
                (_, _, Some(tf)) => ControlToken::read_token_file(tf)?,
                _ => {
                    return Err(CliError::usage(
                        "provide --token-file <path> or both --endpoint and --token",
                    ));
                }
            };
            let cmd_val: Value = serde_json::from_str(command_json)?;
            let cmd =
                ControlCommand::from_json(&cmd_val).map_err(|e| CliError::usage(e.to_string()))?;
            let ControlEndpoint::Tcp(addr) = ControlEndpoint::parse(&ep_str)? else {
                return Err(CliError::usage(
                    "control send currently supports tcp://127.0.0.1:<port> loopback endpoints",
                ));
            };
            let mut client = ControlClient::new(LoopbackTransport::new(addr, codec), tok_str);
            let resp = client.send(cmd)?;
            if !resp.ok {
                let exit = match resp.error.as_ref().map(|e| e.code) {
                    Some(
                        excellent_core::tools::control::ControlErrorCode::Unauthorized
                        | excellent_core::tools::control::ControlErrorCode::Forbidden,
                    ) => CliExit::VerificationOrAuth,
                    _ => CliExit::Processing,
                };
                return Err(CliError::new(
                    exit,
                    resp.error
                        .map_or_else(|| "control command failed".to_owned(), |e| e.to_string()),
                ));
            }
            if json_out {
                write_json(stdout, &resp.to_json())?;
            } else {
                write_json(stdout, &resp.result)?;
            }
            Ok(())
        }
    }
}
