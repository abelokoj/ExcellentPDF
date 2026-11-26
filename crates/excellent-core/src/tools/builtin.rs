//! Built-in tool implementations registered into [`ToolRegistry::with_defaults`] (plan E.2).

use std::fmt::Write as _;
use std::fs;

use excellent_mupdf::{
    Background, Document as MuDocument, Matrix, OutlineItem, PageBox, PagePoint, PageRect, Quad,
    Writer,
};
use serde_json::{Value, json};

use crate::annot::{Annotation, Boxed, Markup, Shape, Style, page_annotations, page_replies};
use crate::edit::{Change, Operation};

use super::registry::{ToolCapability, ToolContext, ToolError, ToolRegistry, ToolSpec};

/// Registers all core document, search, annotation, batch, and render tools into `registry`.
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    register_doc_info(registry);
    register_doc_extract_text(registry);
    register_doc_search(registry);
    register_doc_outline(registry);
    register_annot_list(registry);
    register_annot_add(registry);
    register_annot_export(registry);
    register_doc_merge(registry);
    register_doc_split(registry);
    register_doc_compress(registry);
    register_doc_ocr_stub(registry);
    register_page_render(registry);
    register_doc_attachments(registry);
    register_doc_save(registry);
    register_doc_redact(registry);
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments(format!("missing string '{key}'")))
}

fn opt_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
}

fn open_mupdf(
    ctx: &ToolContext,
    raw_path: &str,
) -> Result<(std::path::PathBuf, MuDocument), ToolError> {
    let path = ctx.resolve_path(raw_path)?;
    let doc = MuDocument::open(&path)?;
    Ok((path, doc))
}

fn register_doc_info(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.info".to_owned(),
            description: "Inspect document metadata, page count, page sizes, labels, outline, and attachments.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["path", "page_count", "is_pdf", "is_reflowable", "reads_right_to_left", "pages"],
                "properties": {
                    "path": { "type": "string" },
                    "page_count": { "type": "integer" },
                    "is_pdf": { "type": "boolean" },
                    "is_reflowable": { "type": "boolean" },
                    "reads_right_to_left": { "type": "boolean" },
                    "outline_count": { "type": "integer" },
                    "attachment_count": { "type": "integer" },
                    "page_labels": { "type": ["array", "null"] },
                    "pages": { "type": "array" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (path, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let page_count = doc.page_count()?;
            let mut pages = Vec::with_capacity(page_count);
            for index in 0..page_count {
                let rect = doc.page_box(index, PageBox::Crop)?;
                pages.push(json!({
                    "page": index,
                    "width_pts": rect.width(),
                    "height_pts": rect.height(),
                }));
            }
            let outline = doc.outline()?;
            let attachments = doc.attachments()?;
            let labels = doc.page_labels()?;
            Ok(json!({
                "path": path.to_string_lossy(),
                "page_count": page_count,
                "is_pdf": doc.is_pdf(),
                "is_reflowable": doc.is_reflowable()?,
                "reads_right_to_left": doc.reads_right_to_left()?,
                "outline_count": outline.len(),
                "attachment_count": attachments.len(),
                "page_labels": labels,
                "pages": pages,
            }))
        },
    );
}

fn register_doc_extract_text(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.extract_text".to_owned(),
            description:
                "Extract plain text from all pages or selected zero-based pages of a document."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "pages": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0 }
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["page_count", "pages", "full_text"],
                "properties": {
                    "page_count": { "type": "integer" },
                    "pages": { "type": "array" },
                    "full_text": { "type": "string" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf", "pages": [0] }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let total = doc.page_count()?;
            let target_pages: Vec<usize> = match args.get("pages").and_then(Value::as_array) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
                    .collect(),
                None => (0..total).collect(),
            };
            let mut page_entries = Vec::with_capacity(target_pages.len());
            let mut full_text = String::new();
            for page_idx in target_pages {
                if page_idx >= total {
                    return Err(ToolError::InvalidArguments(format!(
                        "page index {page_idx} out of range ({total} pages)"
                    )));
                }
                let text = doc.load_page(page_idx)?.text(None)?.text()?;
                if !full_text.is_empty() && !text.is_empty() {
                    full_text.push_str("\n\n");
                }
                full_text.push_str(&text);
                page_entries.push(json!({ "page": page_idx, "text": text }));
            }
            Ok(json!({
                "page_count": total,
                "pages": page_entries,
                "full_text": full_text,
            }))
        },
    );
}

fn register_doc_search(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.search".to_owned(),
            description:
                "Search for a text query across a document and return hits with snippets and quads."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "query"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "query": { "type": "string", "minLength": 1 },
                    "case_sensitive": { "type": "boolean" },
                    "whole_word": { "type": "boolean" },
                    "max_hits": { "type": "integer", "minimum": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["query", "total_hits", "hits"],
                "properties": {
                    "query": { "type": "string" },
                    "total_hits": { "type": "integer" },
                    "hits": { "type": "array" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/zlib-manual.pdf", "query": "deflate" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let query_raw = req_str(args, "query")?;
            let case_sensitive = args
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let whole_word = args
                .get("whole_word")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let max_hits = opt_usize(args, "max_hits").unwrap_or(200);

            let needle = if case_sensitive {
                query_raw.to_owned()
            } else {
                query_raw.to_lowercase()
            };
            let count = doc.page_count()?;
            let mut hits = Vec::new();
            'outer: for page_idx in 0..count {
                let layout = doc.load_page(page_idx)?.text(None)?.layout();
                for (line_idx, line) in layout.lines.iter().enumerate() {
                    let line_str: String = layout.chars[line.chars.clone()]
                        .iter()
                        .map(|c| c.c)
                        .collect();
                    let hay = if case_sensitive {
                        line_str.clone()
                    } else {
                        line_str.to_lowercase()
                    };
                    let mut search_from = 0usize;
                    while let Some(rel) = hay[search_from..].find(&needle) {
                        let byte_start = search_from + rel;
                        let byte_end = byte_start + needle.len();
                        let whole_ok =
                            !whole_word || is_whole_word_match(&hay, byte_start, byte_end);
                        if whole_ok {
                            let char_start = hay[..byte_start].chars().count();
                            let char_len = needle.chars().count();
                            let slice_end =
                                (line.chars.start + char_start + char_len).min(line.chars.end);
                            let slice_start = (line.chars.start + char_start).min(slice_end);
                            let bbox = union_char_quads(&layout.chars[slice_start..slice_end])
                                .unwrap_or(line.bbox);
                            hits.push(json!({
                                "page": page_idx,
                                "line": line_idx,
                                "snippet": line_str.trim(),
                                "rect": [bbox.x0, bbox.y0, bbox.x1, bbox.y1],
                            }));
                            if hits.len() >= max_hits {
                                break 'outer;
                            }
                        }
                        search_from = byte_end.max(byte_start + 1);
                    }
                }
            }
            Ok(json!({
                "query": query_raw,
                "total_hits": hits.len(),
                "hits": hits,
            }))
        },
    );
}

fn is_whole_word_match(hay: &str, start: usize, end: usize) -> bool {
    let before_ok = hay[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    let after_ok = hay[end..]
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    before_ok && after_ok
}

fn union_char_quads(chars: &[excellent_mupdf::TextChar]) -> Option<PageRect> {
    let first = chars.first()?.quad.bounds();
    let mut out = first;
    for ch in &chars[1..] {
        let b = ch.quad.bounds();
        out.x0 = out.x0.min(b.x0);
        out.y0 = out.y0.min(b.y0);
        out.x1 = out.x1.max(b.x1);
        out.y1 = out.y1.max(b.y1);
    }
    Some(out)
}

fn outline_to_json(item: &OutlineItem) -> Value {
    json!({
        "title": item.title,
        "uri": item.uri,
        "page": item.page,
        "open": item.open,
        "style": {
            "bold": item.style.bold,
            "italic": item.style.italic,
            "color": item.style.color,
        },
        "children": item.children.iter().map(outline_to_json).collect::<Vec<_>>(),
    })
}

fn register_doc_outline(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.outline".to_owned(),
            description: "Read the document's table of contents (/Outlines) tree.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": { "type": "array" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let items: Vec<Value> = doc.outline()?.iter().map(outline_to_json).collect();
            Ok(json!({ "items": items }))
        },
    );
}

fn register_annot_list(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "annot.list".to_owned(),
            description:
                "List annotations (and comment replies) on a page or across the entire PDF."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "page": { "type": "integer", "minimum": 0 },
                    "include_replies": { "type": "boolean" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["count", "annotations"],
                "properties": {
                    "count": { "type": "integer" },
                    "annotations": { "type": "array" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let count = doc.page_count()?;
            let include_replies = args
                .get("include_replies")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let pages: Vec<usize> = match opt_usize(args, "page") {
                Some(p) if p < count => vec![p],
                Some(p) => {
                    return Err(ToolError::InvalidArguments(format!(
                        "page {p} out of range ({count} pages)"
                    )));
                }
                None => (0..count).collect(),
            };
            let mut annotations = Vec::new();
            for page in pages {
                for annot in page_annotations(&doc, page)? {
                    annotations.push(annot.to_json());
                }
                if include_replies {
                    for reply in page_replies(&doc, page)? {
                        annotations.push(reply.to_json());
                    }
                }
            }
            Ok(json!({
                "count": annotations.len(),
                "annotations": annotations,
            }))
        },
    );
}

fn register_annot_add(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "annot.add".to_owned(),
            description: "Add an annotation (highlight, underline, strikeout, note, freetext, square, circle) inside an undoable operation and save.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "page", "kind", "rect"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "output": { "type": "string", "minLength": 1 },
                    "page": { "type": "integer", "minimum": 0 },
                    "kind": {
                        "type": "string",
                        "enum": ["highlight", "underline", "squiggly", "strikeout", "note", "freetext", "square", "circle"]
                    },
                    "rect": {
                        "type": "array",
                        "minItems": 4,
                        "items": { "type": "number" }
                    },
                    "contents": { "type": "string" },
                    "author": { "type": "string" },
                    "color": {
                        "type": "array",
                        "minItems": 3,
                        "items": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["id", "page", "saved_to"],
                "properties": {
                    "id": { "type": "string" },
                    "page": { "type": "integer" },
                    "saved_to": { "type": "string" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/base14-outline-annot.pdf",
                "output": "out/annotated.pdf",
                "page": 0,
                "kind": "note",
                "rect": [72.0, 72.0, 96.0, 96.0],
                "contents": "Reviewed"
            }),
            capability: ToolCapability::MutateDocument,
        },
        |ctx, args| {
            let input = ctx.resolve_path(req_str(args, "path")?)?;
            let output = match args.get("output").and_then(Value::as_str) {
                Some(out) => ctx.resolve_path(out)?,
                None => input.clone(),
            };
            let page = opt_usize(args, "page").ok_or_else(|| {
                ToolError::InvalidArguments("missing non-negative integer 'page'".to_owned())
            })?;
            let kind = req_str(args, "kind")?;
            let rect_vals: Vec<f32> = args
                .get("rect")
                .and_then(Value::as_array)
                .ok_or_else(|| ToolError::InvalidArguments("missing 'rect' array".to_owned()))?
                .iter()
                .filter_map(|v| v.as_f64().map(|n| n as f32))
                .collect();
            let [x0, y0, x1, y1] = <[f32; 4]>::try_from(rect_vals).map_err(|_| {
                ToolError::InvalidArguments("'rect' must contain 4 numbers [x0,y0,x1,y1]".to_owned())
            })?;
            let rect = PageRect {
                x0: x0.min(x1),
                y0: y0.min(y1),
                x1: x0.max(x1),
                y1: y0.max(y1),
            };
            let quad = Quad {
                ul: PagePoint { x: rect.x0, y: rect.y0 },
                ur: PagePoint { x: rect.x1, y: rect.y0 },
                ll: PagePoint { x: rect.x0, y: rect.y1 },
                lr: PagePoint { x: rect.x1, y: rect.y1 },
            };
            let shape = match kind {
                "highlight" => Shape::Markup(Markup::Highlight, vec![quad]),
                "underline" => Shape::Markup(Markup::Underline, vec![quad]),
                "squiggly" => Shape::Markup(Markup::Squiggly, vec![quad]),
                "strikeout" => Shape::Markup(Markup::StrikeOut, vec![quad]),
                "note" => Shape::Boxed(Boxed::Note, rect),
                "freetext" => Shape::Boxed(Boxed::FreeText, rect),
                "square" => Shape::Boxed(Boxed::Square, rect),
                "circle" => Shape::Boxed(Boxed::Circle, rect),
                other => {
                    return Err(ToolError::InvalidArguments(format!(
                        "unsupported annotation kind '{other}'"
                    )));
                }
            };
            let color = args
                .get("color")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    let nums: Vec<f32> = arr.iter().filter_map(|v| v.as_f64().map(|n| n as f32)).collect();
                    <[f32; 3]>::try_from(nums).ok()
                });
            let mut style = Style::default();
            if let Some(rgb) = color {
                style.color = Some(rgb);
            }
            let author = args
                .get("author")
                .and_then(Value::as_str)
                .unwrap_or("ExcellentPDF Agent");
            let mut annot = Annotation::new(page, shape, style, author);
            if let Some(contents) = args.get("contents").and_then(Value::as_str) {
                contents.clone_into(&mut annot.contents);
            }
            let id = annot.id.clone();

            if output == input {
                let doc = ctx.registry().open(&input)?;
                doc.apply(&Operation::new(
                    "Add annotation",
                    vec![Change::CreateAnnotation(annot)],
                ))?;
                doc.save()?;
            } else {
                if let Some(parent) = output.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                let doc = crate::document::Document::open(&input)?;
                doc.apply(&Operation::new(
                    "Add annotation",
                    vec![Change::CreateAnnotation(annot)],
                ))?;
                let shared = doc.mupdf()?;
                let mut mupdf = shared.lock().expect("document lock poisoned");
                mupdf.save_pdf(&output)?;
            }

            Ok(json!({
                "id": id,
                "page": page,
                "saved_to": output.to_string_lossy(),
            }))
        },
    );
}

fn register_annot_export(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "annot.export".to_owned(),
            description: "Export PDF annotations as JSON or FDF (optionally writing to an output file).".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "format": { "type": "string", "enum": ["json", "fdf"] },
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
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf", "format": "json" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let format = args
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("json");
            let count = doc.page_count()?;
            let mut all = Vec::new();
            for page in 0..count {
                all.extend(page_annotations(&doc, page)?);
                all.extend(page_replies(&doc, page)?);
            }
            let (data_val, file_bytes) = match format {
                "json" => {
                    let arr: Vec<Value> = all.iter().map(Annotation::to_json).collect();
                    let val = Value::Array(arr);
                    let bytes = serde_json::to_vec_pretty(&val)
                        .map_err(|err| ToolError::Execution(err.to_string()))?;
                    (val, bytes)
                }
                "fdf" => {
                    let fdf = doc.to_fdf()?;
                    let b64 = encode_base64(&fdf);
                    (json!({ "fdf_base64": b64, "byte_len": fdf.len() }), fdf)
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
                    fs::create_dir_all(parent)?;
                }
                fs::write(&resolved, &file_bytes)?;
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
        },
    );
}

fn register_doc_merge(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.merge".to_owned(),
            description: "Merge multiple PDF or MuPDF-readable documents into one output PDF.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["inputs", "output"],
                "additionalProperties": false,
                "properties": {
                    "inputs": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 }
                    },
                    "output": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "pages_written", "bytes"],
                "properties": {
                    "output": { "type": "string" },
                    "pages_written": { "type": "integer" },
                    "bytes": { "type": "integer" }
                }
            }),
            example_input: json!({
                "inputs": ["tests/corpus/base14-outline-annot.pdf", "tests/corpus/heavy-vector.pdf"],
                "output": "out/merged.pdf"
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        |ctx, args| {
            let inputs = args
                .get("inputs")
                .and_then(Value::as_array)
                .ok_or_else(|| ToolError::InvalidArguments("missing 'inputs' array".to_owned()))?;
            let output = ctx.resolve_path(req_str(args, "output")?)?;
            if let Some(parent) = output.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let mut writer = Writer::new(&output, Some("pdf"), Some("compress"))?;
            let mut pages_written = 0usize;
            for item in inputs {
                let raw = item.as_str().ok_or_else(|| {
                    ToolError::InvalidArguments("each input must be a string path".to_owned())
                })?;
                let (_, doc) = open_mupdf(ctx, raw)?;
                let count = doc.page_count()?;
                for idx in 0..count {
                    let page = doc.load_page(idx)?;
                    writer.write_page(&page)?;
                    pages_written += 1;
                }
            }
            writer.close()?;
            let bytes = fs::metadata(&output)?.len();
            Ok(json!({
                "output": output.to_string_lossy(),
                "pages_written": pages_written,
                "bytes": bytes,
            }))
        },
    );
}

fn register_doc_split(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.split".to_owned(),
            description:
                "Split a PDF into individual per-page PDFs or specified 1-based page range PDFs."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "out_dir"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "out_dir": { "type": "string", "minLength": 1 },
                    "prefix": { "type": "string", "minLength": 1 },
                    "ranges": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1 }
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["files"],
                "properties": {
                    "files": { "type": "array" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/base14-outline-annot.pdf",
                "out_dir": "out/split"
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        |ctx, args| {
            let (path, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let out_dir = ctx.resolve_path(req_str(args, "out_dir")?)?;
            fs::create_dir_all(&out_dir)?;
            let total = doc.page_count()?;
            let default_prefix = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page")
                .to_owned();
            let prefix = args
                .get("prefix")
                .and_then(Value::as_str)
                .unwrap_or(&default_prefix);

            let groups: Vec<Vec<usize>> = match args.get("ranges").and_then(Value::as_array) {
                Some(arr) if !arr.is_empty() => arr
                    .iter()
                    .map(|v| {
                        let spec = v.as_str().ok_or_else(|| {
                            ToolError::InvalidArguments("range entry must be string".to_owned())
                        })?;
                        parse_page_spec_1based(spec, total)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => (0..total).map(|i| vec![i]).collect(),
            };

            let mut files = Vec::with_capacity(groups.len());
            for (idx, page_indices) in groups.into_iter().enumerate() {
                let out_path = out_dir.join(format!("{prefix}-{:04}.pdf", idx + 1));
                let mut writer = Writer::new(&out_path, Some("pdf"), Some("compress"))?;
                for &p in &page_indices {
                    let page = doc.load_page(p)?;
                    writer.write_page(&page)?;
                }
                writer.close()?;
                files.push(json!({
                    "path": out_path.to_string_lossy(),
                    "pages": page_indices,
                }));
            }
            Ok(json!({ "files": files }))
        },
    );
}

/// Parses a 1-based page specification like `"1-3,5"` into zero-based page indices.
///
/// # Errors
///
/// Returns [`ToolError::InvalidArguments`] if syntax or page bounds are invalid.
pub fn parse_page_spec_1based(spec: &str, total_pages: usize) -> Result<Vec<usize>, ToolError> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let start: usize = a
                .trim()
                .parse()
                .map_err(|_| ToolError::InvalidArguments(format!("invalid page number '{a}'")))?;
            let end: usize = b
                .trim()
                .parse()
                .map_err(|_| ToolError::InvalidArguments(format!("invalid page number '{b}'")))?;
            if start == 0 || end < start || end > total_pages {
                return Err(ToolError::InvalidArguments(format!(
                    "page range '{part}' out of bounds 1..={total_pages}"
                )));
            }
            for p in start..=end {
                out.push(p - 1);
            }
        } else {
            let p: usize = part.parse().map_err(|_| {
                ToolError::InvalidArguments(format!("invalid page number '{part}'"))
            })?;
            if p == 0 || p > total_pages {
                return Err(ToolError::InvalidArguments(format!(
                    "page {p} out of bounds 1..={total_pages}"
                )));
            }
            out.push(p - 1);
        }
    }
    if out.is_empty() {
        return Err(ToolError::InvalidArguments(
            "empty page selection".to_owned(),
        ));
    }
    Ok(out)
}

fn register_doc_compress(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.compress".to_owned(),
            description: "Compress and deduplicate PDF streams using MuPDF's PDF writer."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "output"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "output": { "type": "string", "minLength": 1 },
                    "preset": { "type": "string", "enum": ["screen", "ebook", "print"] },
                    "linearize": { "type": "boolean" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "bytes_before", "bytes_after"],
                "properties": {
                    "output": { "type": "string" },
                    "bytes_before": { "type": "integer" },
                    "bytes_after": { "type": "integer" },
                    "saved_bytes": { "type": "integer" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/zlib-manual.pdf",
                "output": "out/compressed.pdf",
                "preset": "ebook"
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        |ctx, args| {
            let (input, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let output = ctx.resolve_path(req_str(args, "output")?)?;
            if let Some(parent) = output.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let bytes_before = fs::metadata(&input)?.len();
            let preset = args
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("ebook");
            let linearize = args
                .get("linearize")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut opts = match preset {
                "screen" => String::from(
                    "compress,compress-images,compress-fonts,garbage=deduplicate,objstms,clean",
                ),
                "print" => String::from("compress,garbage=compact,clean"),
                _ => String::from(
                    "compress,compress-images,compress-fonts,garbage=deduplicate,clean",
                ),
            };
            if linearize {
                opts.push_str(",linearize");
            }
            let mut writer = Writer::new(&output, Some("pdf"), Some(&opts))?;
            let count = doc.page_count()?;
            for idx in 0..count {
                let page = doc.load_page(idx)?;
                writer.write_page(&page)?;
            }
            writer.close()?;
            let bytes_after = fs::metadata(&output)?.len();
            let saved = i64::try_from(bytes_before).unwrap_or(i64::MAX)
                - i64::try_from(bytes_after).unwrap_or(i64::MAX);
            Ok(json!({
                "output": output.to_string_lossy(),
                "bytes_before": bytes_before,
                "bytes_after": bytes_after,
                "saved_bytes": saved,
                "preset": preset,
            }))
        },
    );
}

fn register_doc_ocr_stub(registry: &mut ToolRegistry) {
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
                    "engine": { "type": "string", "enum": ["auto", "tesseract", "windows", "test"] },
                    "skip_existing_text": { "type": "boolean" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "page_count", "pages_ocred", "total_words"],
                "properties": {
                    "output": { "type": "string" },
                    "page_count": { "type": "integer" },
                    "pages_ocred": { "type": "integer" },
                    "total_words": { "type": "integer" }
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
        |_ctx, _args| {
            Err(ToolError::Execution(
                "doc.ocr requires excellent-features::ocr handler registration (available via excellent-cli)".to_owned(),
            ))
        },
    );
}

fn register_page_render(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "page.render".to_owned(),
            description:
                "Render a single zero-based page to PNG, saving to disk or returning base64 PNG."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "page"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "page": { "type": "integer", "minimum": 0 },
                    "dpi": { "type": "integer", "minimum": 36, "maximum": 1200 },
                    "width": { "type": "integer", "minimum": 16, "maximum": 16384 },
                    "transparent": { "type": "boolean" },
                    "output": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["page", "width", "height", "bytes"],
                "properties": {
                    "page": { "type": "integer" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "bytes": { "type": "integer" },
                    "output": { "type": ["string", "null"] },
                    "png_base64": { "type": ["string", "null"] }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/base14-outline-annot.pdf",
                "page": 0,
                "dpi": 150
            }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let page_idx = opt_usize(args, "page")
                .ok_or_else(|| ToolError::InvalidArguments("missing integer 'page'".to_owned()))?;
            let page = doc.load_page(page_idx)?;
            let bounds = page.bounds()?;
            let scale = if let Some(w) = opt_usize(args, "width") {
                (w as f32) / bounds.width().max(1.0)
            } else {
                let dpi = opt_usize(args, "dpi").unwrap_or(150);
                (dpi as f32) / 72.0
            };
            let bg = if args.get("transparent").and_then(Value::as_bool) == Some(true) {
                Background::Transparent
            } else {
                Background::White
            };
            let pixmap = page.render(&Matrix::scale(scale, scale), bg)?;
            let png = pixmap.to_png()?;
            if let Some(out) = args.get("output").and_then(Value::as_str) {
                let resolved = ctx.resolve_path(out)?;
                if let Some(parent) = resolved.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&resolved, &png)?;
                Ok(json!({
                    "page": page_idx,
                    "width": pixmap.width(),
                    "height": pixmap.height(),
                    "bytes": png.len(),
                    "output": resolved.to_string_lossy(),
                    "png_base64": Value::Null,
                }))
            } else {
                Ok(json!({
                    "page": page_idx,
                    "width": pixmap.width(),
                    "height": pixmap.height(),
                    "bytes": png.len(),
                    "output": Value::Null,
                    "png_base64": encode_base64(&png),
                }))
            }
        },
    );
}

fn register_doc_attachments(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.attachments".to_owned(),
            description: "List embedded files and file attachment annotations in a PDF.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["attachments"],
                "properties": {
                    "attachments": { "type": "array" }
                }
            }),
            example_input: json!({ "path": "tests/corpus/base14-outline-annot.pdf" }),
            capability: ToolCapability::ReadOnly,
        },
        |ctx, args| {
            let (_, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let list: Vec<Value> = doc
                .attachments()?
                .into_iter()
                .map(|a| {
                    json!({
                        "index": a.index,
                        "name": a.name,
                        "size": a.size,
                        "page": a.page,
                    })
                })
                .collect();
            Ok(json!({ "attachments": list }))
        },
    );
}

fn register_doc_save(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.save".to_owned(),
            description: "Save a copy of a PDF, optionally flattening annotations and form fields."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "output"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "output": { "type": "string", "minLength": 1 },
                    "flatten": { "type": "boolean" },
                    "flatten_forms": { "type": "boolean" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "flattened"],
                "properties": {
                    "output": { "type": "string" },
                    "flattened": { "type": "boolean" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/base14-outline-annot.pdf",
                "output": "out/flattened.pdf",
                "flatten": true
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        |ctx, args| {
            let (input, mut doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let output = ctx.resolve_path(req_str(args, "output")?)?;
            if output == input {
                return Err(ToolError::InvalidArguments(
                    "output path must differ from input path".to_owned(),
                ));
            }
            if let Some(parent) = output.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let flatten = args
                .get("flatten")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if flatten {
                let forms = if args.get("flatten_forms").and_then(Value::as_bool) == Some(true) {
                    excellent_mupdf::Forms::Flatten
                } else {
                    excellent_mupdf::Forms::Keep
                };
                doc.save_flattened(&output, forms)?;
            } else {
                doc.save_pdf(&output)?;
            }
            Ok(json!({
                "output": output.to_string_lossy(),
                "flattened": flatten,
            }))
        },
    );
}

fn register_doc_redact(registry: &mut ToolRegistry) {
    registry.register_fn(
        ToolSpec {
            name: "doc.redact".to_owned(),
            description: "Redact matching text patterns and/or page rectangles with solid black boxes, strip underlying text on affected pages, and verify zero matches remain.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "output"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "output": { "type": "string", "minLength": 1 },
                    "patterns": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1 }
                    },
                    "rects": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["page", "rect"],
                            "properties": {
                                "page": { "type": "integer", "minimum": 0 },
                                "rect": { "type": "array", "minItems": 4, "items": { "type": "number" } }
                            }
                        }
                    },
                    "case_sensitive": { "type": "boolean" },
                    "dpi": { "type": "integer", "minimum": 72, "maximum": 600 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["output", "redactions_applied", "verified_clean"],
                "properties": {
                    "output": { "type": "string" },
                    "redactions_applied": { "type": "integer" },
                    "pages_redacted": { "type": "array" },
                    "verified_clean": { "type": "boolean" }
                }
            }),
            example_input: json!({
                "path": "tests/corpus/base14-outline-annot.pdf",
                "output": "out/redacted.pdf",
                "patterns": ["Page"]
            }),
            capability: ToolCapability::FileSystemWrite,
        },
        |ctx, args| {
            let (input, doc) = open_mupdf(ctx, req_str(args, "path")?)?;
            let output = ctx.resolve_path(req_str(args, "output")?)?;
            if output == input {
                return Err(ToolError::InvalidArguments(
                    "output path must differ from input path".to_owned(),
                ));
            }
            let patterns: Vec<String> = args
                .get("patterns")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let mut explicit_rects: Vec<(usize, PageRect)> = Vec::new();
            if let Some(arr) = args.get("rects").and_then(Value::as_array) {
                for item in arr {
                    let p = item.get("page").and_then(Value::as_u64).ok_or_else(|| {
                        ToolError::InvalidArguments("rect entry missing 'page'".to_owned())
                    })? as usize;
                    let coords: Vec<f32> = item
                        .get("rect")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            ToolError::InvalidArguments("rect entry missing 'rect'".to_owned())
                        })?
                        .iter()
                        .filter_map(|v| v.as_f64().map(|n| n as f32))
                        .collect();
                    let [x0, y0, x1, y1] = <[f32; 4]>::try_from(coords).map_err(|_| {
                        ToolError::InvalidArguments("rect must have 4 numbers".to_owned())
                    })?;
                    explicit_rects.push((
                        p,
                        PageRect {
                            x0: x0.min(x1),
                            y0: y0.min(y1),
                            x1: x0.max(x1),
                            y1: y0.max(y1),
                        },
                    ));
                }
            }
            if patterns.is_empty() && explicit_rects.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "specify at least one pattern or rect to redact".to_owned(),
                ));
            }
            let case_sensitive = args
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let dpi = opt_usize(args, "dpi").unwrap_or(150).clamp(72, 600);
            let scale = (dpi as f32) / 72.0;

            let page_count = doc.page_count()?;
            let mut per_page_boxes: Vec<Vec<PageRect>> = vec![Vec::new(); page_count];
            for (p, r) in explicit_rects {
                if p >= page_count {
                    return Err(ToolError::InvalidArguments(format!(
                        "rect page {p} out of range ({page_count} pages)"
                    )));
                }
                per_page_boxes[p].push(r);
            }

            if !patterns.is_empty() {
                for (page_idx, page_boxes) in per_page_boxes.iter_mut().enumerate() {
                    let layout = doc.load_page(page_idx)?.text(None)?.layout();
                    for line in &layout.lines {
                        let line_str: String = layout.chars[line.chars.clone()]
                            .iter()
                            .map(|c| c.c)
                            .collect();
                        let hay = if case_sensitive {
                            line_str.clone()
                        } else {
                            line_str.to_lowercase()
                        };
                        for pat in &patterns {
                            let needle = if case_sensitive {
                                pat.clone()
                            } else {
                                pat.to_lowercase()
                            };
                            let mut from = 0usize;
                            while let Some(rel) = hay[from..].find(&needle) {
                                let bs = from + rel;
                                let be = bs + needle.len();
                                let cs = hay[..bs].chars().count();
                                let cl = needle.chars().count();
                                let s_start = (line.chars.start + cs).min(line.chars.end);
                                let s_end = (line.chars.start + cs + cl).min(line.chars.end);
                                if let Some(rect) =
                                    union_char_quads(&layout.chars[s_start..s_end])
                                {
                                    page_boxes.push(rect);
                                }
                                from = be.max(bs + 1);
                            }
                        }
                    }
                }
            }

            let mut pages_redacted = Vec::new();
            let mut redactions_applied = 0usize;
            let mut rendered_pages = Vec::with_capacity(page_count);
            for (page_idx, boxes) in per_page_boxes.iter().enumerate() {
                let page = doc.load_page(page_idx)?;
                let bounds = page.bounds()?;
                let mut pixmap = page.render(&Matrix::scale(scale, scale), Background::White)?;
                let pw = pixmap.width();
                let ph = pixmap.height();
                let stride = pixmap.stride();
                let comps = pixmap.components();
                let samples = pixmap.samples_mut();
                if !boxes.is_empty() {
                    pages_redacted.push(page_idx);
                    for rect in boxes {
                        redactions_applied += 1;
                        let px0 = ((rect.x0 - bounds.x0) * scale).floor().max(0.0) as u32;
                        let py0 = ((rect.y0 - bounds.y0) * scale).floor().max(0.0) as u32;
                        let px1 = (((rect.x1 - bounds.x0) * scale).ceil().max(0.0) as u32).min(pw);
                        let py1 = (((rect.y1 - bounds.y0) * scale).ceil().max(0.0) as u32).min(ph);
                        for y in py0..py1 {
                            let row = (y as usize) * stride;
                            for x in px0..px1 {
                                let off = row + (x as usize) * comps;
                                if off + 2 < samples.len() {
                                    samples[off] = 0;
                                    samples[off + 1] = 0;
                                    samples[off + 2] = 0;
                                }
                            }
                        }
                    }
                }
                rendered_pages.push(RasterPage {
                    width_pts: bounds.width().max(1.0),
                    height_pts: bounds.height().max(1.0),
                    width_px: pw,
                    height_px: ph,
                    rgb: extract_rgb_samples(&pixmap),
                });
            }

            if let Some(parent) = output.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let pdf_bytes = build_raster_pdf(&rendered_pages);
            fs::write(&output, &pdf_bytes)?;

            // Post-redaction verification: reopen and confirm every pattern has 0 hits.
            let reopened = MuDocument::open(&output)?;
            let mut remaining_hits = 0usize;
            for page_idx in 0..reopened.page_count()? {
                let text = reopened.load_page(page_idx)?.text(None)?.text()?;
                let hay = if case_sensitive {
                    text
                } else {
                    text.to_lowercase()
                };
                for pat in &patterns {
                    let needle = if case_sensitive {
                        pat.clone()
                    } else {
                        pat.to_lowercase()
                    };
                    if !needle.is_empty() && hay.contains(&needle) {
                        remaining_hits += 1;
                    }
                }
            }
            let verified_clean = remaining_hits == 0;
            Ok(json!({
                "output": output.to_string_lossy(),
                "redactions_applied": redactions_applied,
                "pages_redacted": pages_redacted,
                "verified_clean": verified_clean,
            }))
        },
    );
}

struct RasterPage {
    width_pts: f32,
    height_pts: f32,
    width_px: u32,
    height_px: u32,
    rgb: Vec<u8>,
}

fn extract_rgb_samples(pixmap: &excellent_mupdf::Pixmap) -> Vec<u8> {
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let stride = pixmap.stride();
    let comps = pixmap.components();
    let samples = pixmap.samples();
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &samples[y * stride..];
        for x in 0..w {
            let off = x * comps;
            rgb.push(row[off]);
            rgb.push(row[off + 1]);
            rgb.push(row[off + 2]);
        }
    }
    rgb
}

fn build_raster_pdf(pages: &[RasterPage]) -> Vec<u8> {
    let mut out = Vec::from(b"%PDF-1.4\n".as_slice());
    let mut offsets = vec![0usize]; // index 0 unused

    let page_count = pages.len();
    let pages_obj_id = 2u32;
    let first_page_id = 3u32;

    // 1 0 obj: Catalog
    offsets.push(out.len());
    out.extend_from_slice(
        format!("1 0 obj\n<< /Type /Catalog /Pages {pages_obj_id} 0 R >>\nendobj\n").as_bytes(),
    );

    // 2 0 obj: Pages
    let mut kids = String::new();
    for i in 0..page_count {
        let pid = first_page_id + (i as u32) * 3;
        let _ = write!(kids, "{pid} 0 R ");
    }
    offsets.push(out.len());
    out.extend_from_slice(
        format!("2 0 obj\n<< /Type /Pages /Count {page_count} /Kids [ {kids}] >>\nendobj\n")
            .as_bytes(),
    );

    for (i, p) in pages.iter().enumerate() {
        let page_id = first_page_id + (i as u32) * 3;
        let contents_id = page_id + 1;
        let img_id = page_id + 2;

        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent {pages_obj_id} 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /XObject << /Im0 {img_id} 0 R >> >> /Contents {contents_id} 0 R >>\nendobj\n",
                p.width_pts, p.height_pts
            )
            .as_bytes(),
        );

        let ops = format!(
            "q\n{:.2} 0 0 {:.2} 0 0 cm\n/Im0 Do\nQ\n",
            p.width_pts, p.height_pts
        );
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{contents_id} 0 obj\n<< /Length {} >>\nstream\n{ops}endstream\nendobj\n",
                ops.len()
            )
            .as_bytes(),
        );

        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{img_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                p.width_px,
                p.height_px,
                p.rgb.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&p.rgb);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xref_offset = out.len();
    let total = offsets.len();
    let mut xref = format!("xref\n0 {total}\n0000000000 65535 f \n");
    for &off in &offsets[1..] {
        let _ = writeln!(xref, "{off:010} 00000 n ");
    }
    let _ = writeln!(
        xref,
        "trailer\n<< /Size {total} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF"
    );
    out.extend_from_slice(xref.as_bytes());
    out
}

/// Encodes arbitrary bytes as RFC 4648 standard base64.
#[must_use]
pub fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::tools::schema::validate_json;

    fn corpus(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name)
    }

    #[test]
    fn every_tool_example_matches_its_input_schema() {
        let reg = ToolRegistry::with_defaults();
        assert!(reg.len() >= 14);
        for spec in reg.specs() {
            assert!(
                validate_json(&spec.example_input, &spec.input_schema).is_ok(),
                "tool {} example_input failed schema: {:?}",
                spec.name,
                validate_json(&spec.example_input, &spec.input_schema)
            );
            assert!(spec.input_schema.get("type") == Some(&Value::String("object".into())));
            assert!(spec.output_schema.get("type") == Some(&Value::String("object".into())));
        }
    }

    #[test]
    fn core_tools_execute_on_corpus_documents() {
        let reg = ToolRegistry::with_defaults();
        let ctx = ToolContext::default();
        let pdf = corpus("base14-outline-annot.pdf");
        let pdf_str = pdf.to_string_lossy().into_owned();

        let info = reg
            .execute(&ctx, "doc.info", &json!({ "path": pdf_str }))
            .expect("doc.info");
        assert_eq!(info["page_count"], 2);
        assert_eq!(info["is_pdf"], true);

        let text = reg
            .execute(&ctx, "doc.extract_text", &json!({ "path": pdf_str }))
            .expect("doc.extract_text");
        assert!(!text["full_text"].as_str().unwrap_or("").is_empty());

        let outline = reg
            .execute(&ctx, "doc.outline", &json!({ "path": pdf_str }))
            .expect("doc.outline");
        assert_eq!(outline["items"].as_array().map(Vec::len), Some(2));

        let annots = reg
            .execute(&ctx, "annot.list", &json!({ "path": pdf_str }))
            .expect("annot.list");
        assert!(annots["count"].as_u64().is_some());

        let rendered = reg
            .execute(
                &ctx,
                "page.render",
                &json!({ "path": pdf_str, "page": 0, "width": 100 }),
            )
            .expect("page.render");
        assert_eq!(rendered["width"], 100);
        assert!(
            rendered["png_base64"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
    }
}
