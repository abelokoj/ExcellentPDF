//! Zero-dependency typed CLI argument parser for `excellent-cli` (plan E.1).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::CliError;

/// Parsed CLI invocation including global flags and subcommand payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCli {
    /// Emit machine-readable JSON on stdout (`--json`).
    pub json: bool,
    /// Subcommand to execute.
    pub command: Command,
}

/// All subcommands supported by `excellent-cli`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Show top-level usage help.
    Help,
    /// Show version information.
    Version,
    /// Inspect PDF/document metadata (`info <input>`).
    Info {
        /// Input document path.
        input: PathBuf,
    },
    /// Extract plain text (`text <input>` or `extract-text <input>`).
    Text {
        /// Input document path.
        input: PathBuf,
        /// Optional 1-based page selection (e.g. `"1-3,5"`).
        pages: Option<String>,
        /// Optional output text file path.
        output: Option<PathBuf>,
    },
    /// Search document text (`search <input> <query>`).
    Search {
        /// Input document path.
        input: PathBuf,
        /// Search query string.
        query: String,
        /// Match exact letter case.
        case_sensitive: bool,
        /// Match whole words only.
        whole_word: bool,
        /// Maximum number of hits to return.
        max_hits: Option<usize>,
    },
    /// Merge multiple documents into one PDF (`merge <inputs>... -o <output>`).
    Merge {
        /// Ordered list of input documents.
        inputs: Vec<PathBuf>,
        /// Output PDF path.
        output: PathBuf,
    },
    /// Split a document into individual or ranged PDFs (`split <input> --out-dir <dir>`).
    Split {
        /// Input document path.
        input: PathBuf,
        /// Destination directory.
        out_dir: PathBuf,
        /// Optional comma-separated 1-based page ranges (e.g. `"1-2,3-4"`).
        ranges: Vec<String>,
        /// Output filename prefix.
        prefix: Option<String>,
    },
    /// Compress and rewrite a PDF (`compress <input> -o <output>`).
    Compress {
        /// Input PDF path.
        input: PathBuf,
        /// Output PDF path.
        output: PathBuf,
        /// Quality preset (`"screen"`, `"ebook"`, `"print"`).
        preset: String,
        /// Enable web linearization.
        linearize: bool,
    },
    /// OCR scanned pages into a searchable PDF (`ocr <input> -o <output>`).
    Ocr {
        /// Input PDF path.
        input: PathBuf,
        /// Output searchable PDF path.
        output: PathBuf,
        /// Language codes (comma-separated or repeated `--lang`).
        languages: Vec<String>,
        /// Rasterization DPI.
        dpi: u32,
        /// Deskew mode (`"auto"` or `"off"`).
        deskew: String,
        /// Backend (`"auto"`, `"tesseract"`, `"windows"`, `"test"`).
        engine: String,
        /// Skip pages that already contain selectable text.
        skip_existing_text: bool,
        /// Optional deterministic text used when `--engine test`.
        test_text: Option<String>,
    },
    /// Redact text patterns or rectangles (`redact <input> -o <output>`).
    Redact {
        /// Input PDF path.
        input: PathBuf,
        /// Output redacted PDF path.
        output: PathBuf,
        /// Text patterns to redact.
        patterns: Vec<String>,
        /// Explicit page rectangles (`page:x0,y0,x1,y1`).
        rects: Vec<String>,
        /// Match exact case when searching patterns.
        case_sensitive: bool,
        /// Rasterization DPI for redacted pages.
        dpi: u32,
    },
    /// Export annotations, Markdown text, or flattened PDF (`export <input> --format <fmt>`).
    Export {
        /// Input document path.
        input: PathBuf,
        /// Export format (`"xfdf"`, `"fdf"`, `"w3c"`, `"json"`, `"markdown"`, `"flattened"`).
        format: String,
        /// Output file path (optional for text formats; required for binary/flattened).
        output: Option<PathBuf>,
    },
    /// Render a page to PNG (`render <input> --page <n> [-o <output>]`).
    Render {
        /// Input document path.
        input: PathBuf,
        /// Zero-based page index.
        page: usize,
        /// Output PNG path.
        output: Option<PathBuf>,
        /// Render DPI.
        dpi: Option<u32>,
        /// Target pixel width.
        width: Option<u32>,
        /// Use transparent background instead of white.
        transparent: bool,
    },
    /// Run a registered E.2 tool (`run <tool> --json <args>` or `run <script.json>` or `run --list`).
    Run {
        /// Tool name or path to a JSON call/script file.
        target: Option<String>,
        /// Inline JSON arguments (`--json <json>`).
        json_arg: Option<String>,
        /// List available tools and schemas (`--list`).
        list: bool,
        /// Optional `--root` directory confinement.
        root: Option<PathBuf>,
    },
    /// Start the E.3 MCP JSON-RPC 2.0 stdio server (`mcp [--root <dir>]`).
    Mcp {
        /// Optional `--root` folder confinement.
        root: Option<PathBuf>,
    },
    /// Interact with or host the E.4 UI control channel (`control ...`).
    Control {
        /// Control sub-action.
        action: ControlAction,
    },
}

/// Sub-actions under `excellent-cli control`.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    /// Start a control server (`control serve`).
    Serve {
        /// Endpoint (`tcp://127.0.0.1:PORT` or `unix:///path`).
        endpoint: String,
        /// Path to write the generated token & endpoint JSON file.
        token_file: Option<PathBuf>,
        /// Wire framing (`"ndjson"` or `"length"`).
        codec: String,
        /// Stop after N connections (useful in tests).
        max_requests: Option<usize>,
        /// Use in-memory mock host instead of real document host.
        mock: bool,
    },
    /// Send a raw JSON command or convenience command to a running control server.
    Send {
        /// Server endpoint (`tcp://...` or `unix://...`).
        endpoint: Option<String>,
        /// Authorization token string.
        token: Option<String>,
        /// Token file written by `control serve`.
        token_file: Option<PathBuf>,
        /// Wire framing (`"ndjson"` or `"length"`).
        codec: String,
        /// JSON command payload.
        command_json: String,
    },
}

/// Parses `args` (typically `std::env::args_os()`, including `argv[0]`) into [`ParsedCli`].
///
/// # Errors
///
/// Returns [`CliError`] with [`crate::error::CliExit::Usage`] on unknown flags or missing arguments.
pub fn parse_args<I, S>(args: I) -> Result<ParsedCli, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let raw: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|s| s.into().to_string_lossy().into_owned())
        .collect();

    if raw.is_empty() {
        return Ok(ParsedCli {
            json: false,
            command: Command::Help,
        });
    }

    let mut json = false;
    let mut tokens: Vec<String> = Vec::with_capacity(raw.len());
    // Extract global `--json` only when it is NOT immediately followed by a JSON object argument for `run --json <str>`.
    let mut i = 0usize;
    while i < raw.len() {
        let tok = &raw[i];
        if tok == "--json" {
            // Check if the subcommand is `run` and the next token looks like a JSON payload (doesn't start with `-`).
            let is_run_sub = tokens.first().is_some_and(|s| s == "run");
            let next_is_payload = raw
                .get(i + 1)
                .is_some_and(|n| n.trim_start().starts_with('{'));
            if is_run_sub && next_is_payload {
                tokens.push(tok.clone());
            } else {
                json = true;
            }
        } else {
            tokens.push(tok.clone());
        }
        i += 1;
    }

    let Some(first) = tokens.first().map(String::as_str) else {
        return Ok(ParsedCli {
            json,
            command: Command::Help,
        });
    };

    let rest = &tokens[1..];
    let command = match first {
        "-h" | "--help" | "help" => Command::Help,
        "-V" | "--version" | "version" => Command::Version,
        "info" => parse_info(rest)?,
        "text" | "extract-text" => parse_text(rest)?,
        "search" => parse_search(rest)?,
        "merge" => parse_merge(rest)?,
        "split" => parse_split(rest)?,
        "compress" => parse_compress(rest)?,
        "ocr" => parse_ocr(rest)?,
        "redact" => parse_redact(rest)?,
        "export" => parse_export(rest)?,
        "render" => parse_render(rest)?,
        "run" => parse_run(rest)?,
        "mcp" => parse_mcp(rest)?,
        "control" => parse_control(rest)?,
        other => {
            return Err(CliError::usage(format!(
                "unknown subcommand '{other}' (run with --help for usage)"
            )));
        }
    };

    Ok(ParsedCli { json, command })
}

fn next_val<'a>(args: &'a [String], idx: &mut usize, flag: &str) -> Result<&'a str, CliError> {
    *idx += 1;
    args.get(*idx)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("flag '{flag}' requires a value")))
}

fn parse_u32(val: &str, flag: &str) -> Result<u32, CliError> {
    val.parse::<u32>()
        .map_err(|_| CliError::usage(format!("invalid integer for {flag}: '{val}'")))
}

fn parse_usize(val: &str, flag: &str) -> Result<usize, CliError> {
    val.parse::<usize>()
        .map_err(|_| CliError::usage(format!("invalid integer for {flag}: '{val}'")))
}

fn reject_flag(tok: &str) -> Result<(), CliError> {
    if tok.starts_with('-') {
        Err(CliError::usage(format!("unknown flag '{tok}'")))
    } else {
        Ok(())
    }
}

fn parse_info(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    for tok in args {
        reject_flag(tok)?;
        if input.is_none() {
            input = Some(PathBuf::from(tok));
        } else {
            return Err(CliError::usage(format!("unexpected argument '{tok}'")));
        }
    }
    let input = input.ok_or_else(|| CliError::usage("info requires <input>"))?;
    Ok(Command::Info { input })
}

fn parse_text(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut pages = None;
    let mut output = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pages" | "-p" => pages = Some(next_val(args, &mut i, "--pages")?.to_owned()),
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| CliError::usage("text requires <input>"))?;
    Ok(Command::Text {
        input,
        pages,
        output,
    })
}

fn parse_search(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut query = None;
    let mut case_sensitive = false;
    let mut whole_word = false;
    let mut max_hits = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--case-sensitive" | "-c" => case_sensitive = true,
            "--whole-word" | "-w" => whole_word = true,
            "--max-hits" => {
                let v = next_val(args, &mut i, "--max-hits")?;
                max_hits = Some(parse_usize(v, "--max-hits")?);
            }
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else if query.is_none() {
                    query = Some(tok.to_owned());
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    Ok(Command::Search {
        input: input.ok_or_else(|| CliError::usage("search requires <input> <query>"))?,
        query: query.ok_or_else(|| CliError::usage("search requires <query>"))?,
        case_sensitive,
        whole_word,
        max_hits,
    })
}

fn parse_merge(args: &[String]) -> Result<Command, CliError> {
    let mut inputs = Vec::new();
    let mut output = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            tok => {
                reject_flag(tok)?;
                inputs.push(PathBuf::from(tok));
            }
        }
        i += 1;
    }
    if inputs.is_empty() {
        return Err(CliError::usage("merge requires at least one <input>"));
    }
    let output = output.ok_or_else(|| CliError::usage("merge requires -o <output>"))?;
    Ok(Command::Merge { inputs, output })
}

fn parse_split(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut out_dir = None;
    let mut ranges = Vec::new();
    let mut prefix = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out-dir" | "-d" => {
                out_dir = Some(PathBuf::from(next_val(args, &mut i, "--out-dir")?));
            }
            "--pages" | "-p" | "--range" => {
                ranges.push(next_val(args, &mut i, "--pages")?.to_owned());
            }
            "--prefix" => prefix = Some(next_val(args, &mut i, "--prefix")?.to_owned()),
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    Ok(Command::Split {
        input: input.ok_or_else(|| CliError::usage("split requires <input>"))?,
        out_dir: out_dir.ok_or_else(|| CliError::usage("split requires --out-dir <dir>"))?,
        ranges,
        prefix,
    })
}

fn parse_compress(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut output = None;
    let mut preset = String::from("ebook");
    let mut linearize = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            "--preset" => preset = next_val(args, &mut i, "--preset")?.to_owned(),
            "--linearize" => linearize = true,
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    if !matches!(preset.as_str(), "screen" | "ebook" | "print") {
        return Err(CliError::usage(format!(
            "invalid --preset '{preset}' (expected screen, ebook, or print)"
        )));
    }
    Ok(Command::Compress {
        input: input.ok_or_else(|| CliError::usage("compress requires <input>"))?,
        output: output.ok_or_else(|| CliError::usage("compress requires -o <output>"))?,
        preset,
        linearize,
    })
}

fn parse_ocr(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut output = None;
    let mut languages = Vec::new();
    let mut dpi = 300u32;
    let mut deskew = String::from("auto");
    let mut engine = String::from("auto");
    let mut skip_existing_text = false;
    let mut test_text = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            "--lang" | "-l" => {
                let raw = next_val(args, &mut i, "--lang")?;
                for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    languages.push(part.to_owned());
                }
            }
            "--dpi" => dpi = parse_u32(next_val(args, &mut i, "--dpi")?, "--dpi")?,
            "--deskew" => deskew = next_val(args, &mut i, "--deskew")?.to_owned(),
            "--engine" => engine = next_val(args, &mut i, "--engine")?.to_owned(),
            "--skip-text" => skip_existing_text = true,
            "--test-text" => test_text = Some(next_val(args, &mut i, "--test-text")?.to_owned()),
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    if languages.is_empty() {
        languages.push("eng".to_owned());
    }
    Ok(Command::Ocr {
        input: input.ok_or_else(|| CliError::usage("ocr requires <input>"))?,
        output: output.ok_or_else(|| CliError::usage("ocr requires -o <output>"))?,
        languages,
        dpi,
        deskew,
        engine,
        skip_existing_text,
        test_text,
    })
}

fn parse_redact(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut output = None;
    let mut patterns = Vec::new();
    let mut rects = Vec::new();
    let mut case_sensitive = false;
    let mut dpi = 150u32;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            "--pattern" | "-p" => patterns.push(next_val(args, &mut i, "--pattern")?.to_owned()),
            "--rect" => rects.push(next_val(args, &mut i, "--rect")?.to_owned()),
            "--case-sensitive" => case_sensitive = true,
            "--dpi" => dpi = parse_u32(next_val(args, &mut i, "--dpi")?, "--dpi")?,
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    if patterns.is_empty() && rects.is_empty() {
        return Err(CliError::usage(
            "redact requires at least one --pattern or --rect",
        ));
    }
    Ok(Command::Redact {
        input: input.ok_or_else(|| CliError::usage("redact requires <input>"))?,
        output: output.ok_or_else(|| CliError::usage("redact requires -o <output>"))?,
        patterns,
        rects,
        case_sensitive,
        dpi,
    })
}

fn parse_export(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut format = None;
    let mut output = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => format = Some(next_val(args, &mut i, "--format")?.to_owned()),
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    let format = format.unwrap_or_else(|| "json".to_owned());
    Ok(Command::Export {
        input: input.ok_or_else(|| CliError::usage("export requires <input>"))?,
        format,
        output,
    })
}

fn parse_render(args: &[String]) -> Result<Command, CliError> {
    let mut input = None;
    let mut page = 0usize;
    let mut output = None;
    let mut dpi = None;
    let mut width = None;
    let mut transparent = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--page" | "-p" => page = parse_usize(next_val(args, &mut i, "--page")?, "--page")?,
            "--output" | "-o" => {
                output = Some(PathBuf::from(next_val(args, &mut i, "--output")?));
            }
            "--dpi" => dpi = Some(parse_u32(next_val(args, &mut i, "--dpi")?, "--dpi")?),
            "--width" | "-w" => {
                width = Some(parse_u32(next_val(args, &mut i, "--width")?, "--width")?);
            }
            "--transparent" => transparent = true,
            tok => {
                reject_flag(tok)?;
                if input.is_none() {
                    input = Some(PathBuf::from(tok));
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    Ok(Command::Render {
        input: input.ok_or_else(|| CliError::usage("render requires <input>"))?,
        page,
        output,
        dpi,
        width,
        transparent,
    })
}

fn parse_run(args: &[String]) -> Result<Command, CliError> {
    let mut target = None;
    let mut json_arg = None;
    let mut list = false;
    let mut root = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" | "-j" => json_arg = Some(next_val(args, &mut i, "--json")?.to_owned()),
            "--list" | "-l" => list = true,
            "--root" => root = Some(PathBuf::from(next_val(args, &mut i, "--root")?)),
            tok => {
                reject_flag(tok)?;
                if target.is_none() {
                    target = Some(tok.to_owned());
                } else {
                    return Err(CliError::usage(format!("unexpected argument '{tok}'")));
                }
            }
        }
        i += 1;
    }
    if !list && target.is_none() {
        return Err(CliError::usage(
            "run requires <tool-or-script.json> or --list",
        ));
    }
    Ok(Command::Run {
        target,
        json_arg,
        list,
        root,
    })
}

fn parse_mcp(args: &[String]) -> Result<Command, CliError> {
    let mut root = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => root = Some(PathBuf::from(next_val(args, &mut i, "--root")?)),
            tok => return Err(CliError::usage(format!("unknown mcp argument '{tok}'"))),
        }
        i += 1;
    }
    Ok(Command::Mcp { root })
}

fn parse_control(args: &[String]) -> Result<Command, CliError> {
    let Some(sub) = args.first().map(String::as_str) else {
        return Err(CliError::usage(
            "control requires a subcommand: serve, send, ping, state, open, goto, action",
        ));
    };
    let rest = &args[1..];
    match sub {
        "serve" => {
            let mut endpoint = String::from("127.0.0.1:0");
            let mut token_file = None;
            let mut codec = String::from("ndjson");
            let mut max_requests = None;
            let mut mock = false;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--endpoint" => {
                        endpoint = next_val(rest, &mut i, "--endpoint")?.to_owned();
                    }
                    "--token-file" => {
                        token_file = Some(PathBuf::from(next_val(rest, &mut i, "--token-file")?));
                    }
                    "--codec" => codec = next_val(rest, &mut i, "--codec")?.to_owned(),
                    "--max-requests" => {
                        max_requests = Some(parse_usize(
                            next_val(rest, &mut i, "--max-requests")?,
                            "--max-requests",
                        )?);
                    }
                    "--mock" => mock = true,
                    tok => {
                        return Err(CliError::usage(format!(
                            "unknown control serve arg '{tok}'"
                        )));
                    }
                }
                i += 1;
            }
            Ok(Command::Control {
                action: ControlAction::Serve {
                    endpoint,
                    token_file,
                    codec,
                    max_requests,
                    mock,
                },
            })
        }
        "send" | "ping" | "state" | "open" | "goto" | "action" => {
            let mut endpoint = None;
            let mut token = None;
            let mut token_file = None;
            let mut codec = String::from("ndjson");
            let mut command_json = None;
            let mut positional = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--endpoint" => {
                        endpoint = Some(next_val(rest, &mut i, "--endpoint")?.to_owned());
                    }
                    "--token" => token = Some(next_val(rest, &mut i, "--token")?.to_owned()),
                    "--token-file" => {
                        token_file = Some(PathBuf::from(next_val(rest, &mut i, "--token-file")?));
                    }
                    "--codec" => codec = next_val(rest, &mut i, "--codec")?.to_owned(),
                    "--command" => {
                        command_json = Some(next_val(rest, &mut i, "--command")?.to_owned());
                    }
                    tok => {
                        reject_flag(tok)?;
                        positional.push(tok.to_owned());
                    }
                }
                i += 1;
            }
            let command_json = match sub {
                "send" => command_json
                    .ok_or_else(|| CliError::usage("control send requires --command '<json>'"))?,
                "ping" => serde_json::json!({ "cmd": "ping" }).to_string(),
                "state" => serde_json::json!({ "cmd": "inspect_state" }).to_string(),
                "open" => {
                    let path = positional
                        .first()
                        .ok_or_else(|| CliError::usage("control open requires <path>"))?;
                    serde_json::json!({ "cmd": "open_document", "path": path }).to_string()
                }
                "goto" => {
                    let page_str = positional
                        .first()
                        .ok_or_else(|| CliError::usage("control goto requires <page>"))?;
                    let page = parse_usize(page_str, "<page>")?;
                    serde_json::json!({ "cmd": "jump_to_page", "page": page }).to_string()
                }
                "action" => {
                    let act = positional
                        .first()
                        .ok_or_else(|| CliError::usage("control action requires <action>"))?;
                    serde_json::json!({ "cmd": "run_action", "action": act }).to_string()
                }
                _ => unreachable!(),
            };
            Ok(Command::Control {
                action: ControlAction::Send {
                    endpoint,
                    token,
                    token_file,
                    codec,
                    command_json,
                },
            })
        }
        other => Err(CliError::usage(format!(
            "unknown control subcommand '{other}'"
        ))),
    }
}

/// Help text printed by `excellent-cli --help`.
pub const HELP_TEXT: &str = "\
excellent-cli - Batch PDF tools, Agent Tool runner, MCP server, and UI control client

USAGE:
  excellent-cli [--json] <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
  info <input>                              Inspect document metadata and page dimensions
  text <input> [--pages 1-3] [-o out.txt]   Extract plain text (alias: extract-text)
  search <input> <query> [FLAGS]            Search text across pages
  merge <in1> <in2>... -o <out.pdf>         Merge documents into a single PDF
  split <input> --out-dir <dir> [--pages]   Split document into page or range PDFs
  compress <input> -o <out.pdf> [--preset]  Compress PDF (screen|ebook|print)
  ocr <input> -o <out.pdf> [FLAGS]          Run OCR and embed invisible Tr 3 text layer
  redact <input> -o <out.pdf> --pattern ... Redact text/regions and verify zero matches
  export <input> --format <fmt> [-o out]    Export xfdf|fdf|w3c|json|markdown|flattened
  render <input> --page <n> -o <out.png>    Render page to PNG
  run <tool|script.json> [--json '<args>']  Execute E.2 JSON-Schema tool(s) (--list)
  mcp [--root <dir>]                        Serve E.3 MCP JSON-RPC 2.0 over stdio
  control <serve|send|ping|state|open|...>  Host or drive E.4 UI control channel
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_flag_before_or_after_subcommand() {
        let a = parse_args(["excellent-cli", "--json", "info", "doc.pdf"]).expect("parse a");
        assert!(a.json);
        assert_eq!(
            a.command,
            Command::Info {
                input: PathBuf::from("doc.pdf")
            }
        );

        let b = parse_args(["excellent-cli", "info", "doc.pdf", "--json"]).expect("parse b");
        assert!(b.json);
        assert_eq!(a.command, b.command);
    }

    #[test]
    fn parses_run_tool_with_inline_json_without_swallowing_payload() {
        let parsed = parse_args([
            "excellent-cli",
            "run",
            "doc.info",
            "--json",
            r#"{"path":"a.pdf"}"#,
        ])
        .expect("parse run");
        assert!(!parsed.json);
        assert_eq!(
            parsed.command,
            Command::Run {
                target: Some("doc.info".into()),
                json_arg: Some(r#"{"path":"a.pdf"}"#.into()),
                list: false,
                root: None,
            }
        );
    }
}
