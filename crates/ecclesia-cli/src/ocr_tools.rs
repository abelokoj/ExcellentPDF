//! Bridges `ecclesia-features` (O.4 OCR, A.24 XFDF, A.28 W3C annotations) into `ecclesia-core`'s
//! [`ToolRegistry`].

use std::path::Path;

use ecclesia_core::annot::{page_annotations, page_replies};
use ecclesia_core::tools::builtin::encode_base64;
use ecclesia_core::tools::{ToolCapability, ToolContext, ToolError, ToolRegistry, ToolSpec};
use ecclesia_features::annotate::{w3c, xfdf};
use ecclesia_features::ocr::{
    AutoOcrEngine, DeskewMode, DeterministicTestEngine, ExistingTextPolicy, OcrConfig, OcrEngine,
    PreprocessConfig, TesseractCliEngine, WindowsOcrEngine, ocr_pdf_file,
};
use ecclesia_mupdf::Document as MuDocument;
use serde_json::{Value, json};

/// Registers feature-backed implementations of `doc.ocr` and enhanced `annot.export` (XFDF/W3C/FDF/JSON).
pub fn register_feature_tools(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.ocr".to_owned(),
            description: "Run OCR on scanned PDF pages and write a searchable PDF with an invisible Tr 3 text layer.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "output"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "output": { "type": "string", "minLength": 1 },
                    "languages": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1 }
                    },
                    "dpi": { "type": "integer", "minimum": 72, "maximum": 1200 },
                    "deskew": { "type": "string", "enum": ["auto", "off"] },
                    "engine": { "type": "string", "enum": ["auto", "tesseract", "windows", "test"] },
                    "skip_existing_text": { "type": "boolean" },
                    "test_text": { "type": "string" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "page_count", "pages_ocred", "total_words"],
                "properties": {
                    "output": { "type": "string" },
                    "page_count": { "type": "integer" },
                    "pages_ocred": { "type": "integer" },
                    "total_words": { "type": "integer" },
                    "pages": { "type": "array" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/heavy-vector.pdf",
                "output": "out/searchable.pdf",
                "languages": ["eng"],
                "dpi": 300,
                "engine": "test"
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        run_ocr_tool,
    );

    registry.register_fn(
        ToolSpec {
            name: "annot.export".to_owned(),
            description: "Export PDF annotations in json, fdf, xfdf, or w3c JSON-LD format.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "format": { "type": "string", "enum": ["json", "fdf", "xfdf", "w3c"] },
                    "output": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["format", "count"],
                "properties": {
                    "format": { "type": "string" },
                    "count": { "type": "integer" },
                    "output": { "type": ["string", "null"] },
                    "data": {}
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf", "format": "xfdf" }),
            capability: ToolCapability::ReadOnly,
        },
        run_export_annots_tool,
    );
}

fn run_ocr_tool(ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
    let input_raw = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".to_owned()))?;
    let output_raw = args
        .get("output")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments("missing 'output'".to_owned()))?;

    let input = ctx.resolve_path(input_raw)?;
    let output = ctx.resolve_path(output_raw)?;

    let languages: Vec<String> = args
        .get("languages")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["eng".to_owned()]);
    let dpi = args
        .get("dpi")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(300);
    let deskew = match args.get("deskew").and_then(Value::as_str) {
        Some("off") => DeskewMode::Disabled,
        _ => DeskewMode::Auto,
    };
    let existing_text = if args.get("skip_existing_text").and_then(Value::as_bool) == Some(true) {
        ExistingTextPolicy::SkipPagesWithText
    } else {
        ExistingTextPolicy::ForceAllPages
    };
    let config = OcrConfig {
        languages,
        dpi,
        preprocess: PreprocessConfig {
            deskew,
            ..Default::default()
        },
        existing_text,
        tessdata_dir: None,
    };

    let engine_name = args.get("engine").and_then(Value::as_str).unwrap_or("auto");
    let test_text = args
        .get("test_text")
        .and_then(Value::as_str)
        .unwrap_or("OCR recognized text");

    let report = match engine_name {
        "test" => {
            let engine = DeterministicTestEngine::with_text(test_text);
            run_ocr_with(&input, &output, &engine, &config)?
        }
        "tesseract" => {
            let engine = TesseractCliEngine::default();
            run_ocr_with(&input, &output, &engine, &config)?
        }
        "windows" => {
            let engine = WindowsOcrEngine::new();
            run_ocr_with(&input, &output, &engine, &config)?
        }
        "auto" => {
            let engine = AutoOcrEngine::default();
            run_ocr_with(&input, &output, &engine, &config)?
        }
        other => {
            return Err(ToolError::InvalidArguments(format!(
                "unknown ocr engine '{other}'"
            )));
        }
    };

    let pages: Vec<Value> = report
        .pages
        .iter()
        .map(|p| {
            json!({
                "page": p.page_index,
                "ocred": p.ocred,
                "words_placed": p.words_placed,
                "mean_confidence": p.mean_confidence,
                "skew_degrees": p.skew_degrees,
                "text": p.text,
            })
        })
        .collect();

    Ok(json!({
        "output": output.to_string_lossy(),
        "page_count": report.page_count,
        "pages_ocred": report.pages_ocred,
        "total_words": report.total_words,
        "pages": pages,
    }))
}

fn run_ocr_with(
    input: &Path,
    output: &Path,
    engine: &dyn OcrEngine,
    config: &OcrConfig,
) -> Result<ecclesia_features::ocr::OcrDocumentReport, ToolError> {
    ocr_pdf_file(input, output, engine, config).map_err(|err| ToolError::Execution(err.to_string()))
}

fn run_export_annots_tool(ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
    let raw_path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".to_owned()))?;
    let path = ctx.resolve_path(raw_path)?;
    let doc = MuDocument::open(&path)?;
    let format = args.get("format").and_then(Value::as_str).unwrap_or("json");
    let count = doc.page_count()?;
    let mut all = Vec::new();
    let mut spaces = Vec::with_capacity(count);
    for page in 0..count {
        all.extend(page_annotations(&doc, page)?);
        all.extend(page_replies(&doc, page)?);
        if format == "xfdf" {
            let tx = doc.page_transform(page)?;
            spaces.push(xfdf::PageSpace::new(tx));
        }
    }

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("document.pdf");

    let (data_val, file_bytes) = match format {
        "json" => {
            let val = Value::Array(all.iter().map(|a| a.to_json()).collect());
            let bytes = serde_json::to_vec_pretty(&val)
                .map_err(|err| ToolError::Execution(err.to_string()))?;
            (val, bytes)
        }
        "fdf" => {
            let fdf = doc.to_fdf()?;
            (
                json!({ "fdf_base64": encode_base64(&fdf), "byte_len": fdf.len() }),
                fdf,
            )
        }
        "xfdf" => {
            let xml = xfdf::write(&all, &spaces, file_name);
            let bytes = xml.as_bytes().to_vec();
            (json!({ "xfdf": xml }), bytes)
        }
        "w3c" => {
            let val = w3c::write(&all, file_name, file_name);
            let bytes = serde_json::to_vec_pretty(&val)
                .map_err(|err| ToolError::Execution(err.to_string()))?;
            (val, bytes)
        }
        other => {
            return Err(ToolError::InvalidArguments(format!(
                "unsupported export format '{other}'"
            )));
        }
    };

    let output_path = if let Some(out) = args.get("output").and_then(Value::as_str) {
        let resolved = ctx.resolve_path(out)?;
        if let Some(parent) = resolved.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&resolved, &file_bytes)?;
        Some(resolved.to_string_lossy().into_owned())
    } else {
        None
    };

    Ok(json!({
        "format": format,
        "count": all.len(),
        "output": output_path,
        "data": data_val,
    }))
}
