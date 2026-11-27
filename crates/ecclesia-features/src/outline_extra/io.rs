//! Multi-format outline round-trip import and export (plan B.11): indented text, JSON,
//! pdftk `BookmarkBegin` format, and Markdown.

use std::fmt::{self, Write as _};

use ecclesia_mupdf::OutlineStyle;
use serde_json::{Value, json};

use crate::outline::Entry;

/// Supported outline import/export formats (plan B.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutlineFormat {
    /// Tab-indented plain text (`Title\t<page>`).
    IndentedText,
    /// Lossless structured JSON preserving styles, open states, URIs, and hierarchy.
    Json,
    /// pdftk `dump_data` / `update_info` `BookmarkBegin` records.
    Pdftk,
    /// Nested Markdown bullet list (`- [Title](#page=N)`).
    Markdown,
}

/// Errors produced when importing or parsing an outline format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlineIoError {
    /// Input syntax did not match the requested format.
    Parse(String),
    /// JSON serialization or deserialization failed.
    Json(String),
}

impl fmt::Display for OutlineIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "outline parse error: {msg}"),
            Self::Json(msg) => write!(f, "outline JSON error: {msg}"),
        }
    }
}

impl std::error::Error for OutlineIoError {}

/// Convenience result alias for outline import/export.
pub type Result<T> = std::result::Result<T, OutlineIoError>;

// ---------------------------------------------------------------------------
// 1. Indented Text
// ---------------------------------------------------------------------------

/// Exports `entries` as indented text (`  ` per depth level, `<title>\t<1-based page or ->`).
#[must_use]
pub fn export_indented_text(entries: &[Entry]) -> String {
    fn write_level(out: &mut String, entries: &[Entry], depth: usize) {
        for entry in entries {
            for _ in 0..depth {
                out.push_str("  ");
            }
            let title = entry.title.replace(['\t', '\r', '\n'], " ");
            let _ = write!(out, "{title}");
            if let Some(page) = entry.page {
                let _ = write!(out, "\t{}", page + 1);
            } else if let Some(uri) = &entry.uri {
                let _ = write!(out, "\t{uri}");
            }
            out.push('\n');
            write_level(out, &entry.children, depth + 1);
        }
    }
    let mut out = String::new();
    write_level(&mut out, entries, 0);
    out
}

/// Imports an indented text outline produced by [`export_indented_text`].
///
/// # Errors
///
/// Returns [`OutlineIoError::Parse`] if a line has malformed indentation or page number.
pub fn import_indented_text(text: &str) -> Result<Vec<Entry>> {
    let mut flat: Vec<(usize, Entry)> = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let mut spaces = 0usize;
        for b in raw.bytes() {
            match b {
                b' ' => spaces += 1,
                b'\t' => spaces += 2,
                _ => break,
            }
        }
        let depth = spaces / 2;
        let content = raw.trim_start();
        let (title, page, uri) = if let Some((t, target)) = content.rsplit_once('\t') {
            let target = target.trim();
            if let Ok(p1) = target.parse::<usize>() {
                if p1 == 0 {
                    return Err(OutlineIoError::Parse(format!(
                        "line {}: page numbers are 1-based",
                        line_no + 1
                    )));
                }
                (t.to_owned(), Some(p1 - 1), Some(format!("#page={p1}")))
            } else {
                (t.to_owned(), None, Some(target.to_owned()))
            }
        } else {
            (content.to_owned(), None, None)
        };
        flat.push((
            depth,
            Entry {
                title,
                page,
                uri,
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
        ));
    }
    Ok(nest_by_depth(flat))
}

// ---------------------------------------------------------------------------
// 2. JSON
// ---------------------------------------------------------------------------

/// Exports `entries` as pretty-printed lossless JSON (plan B.11).
///
/// # Errors
///
/// Returns [`OutlineIoError::Json`] if serialization fails.
pub fn export_json(entries: &[Entry]) -> Result<String> {
    let arr: Vec<Value> = entries.iter().map(entry_to_json).collect();
    serde_json::to_string_pretty(&arr).map_err(|e| OutlineIoError::Json(e.to_string()))
}

/// Imports an outline from JSON produced by [`export_json`].
///
/// # Errors
///
/// Returns [`OutlineIoError::Json`] if the JSON is malformed.
pub fn import_json(json_str: &str) -> Result<Vec<Entry>> {
    let val: Value =
        serde_json::from_str(json_str).map_err(|e| OutlineIoError::Json(e.to_string()))?;
    let arr = val
        .as_array()
        .ok_or_else(|| OutlineIoError::Json("root JSON value must be an array".into()))?;
    arr.iter().map(entry_from_json).collect()
}

fn entry_to_json(entry: &Entry) -> Value {
    let color = entry
        .style
        .color
        .map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
    json!({
        "title": entry.title,
        "page": entry.page,
        "uri": entry.uri,
        "open": entry.open,
        "style": {
            "bold": entry.style.bold,
            "italic": entry.style.italic,
            "color": color,
        },
        "children": entry.children.iter().map(entry_to_json).collect::<Vec<_>>(),
    })
}

fn entry_from_json(value: &Value) -> Result<Entry> {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| OutlineIoError::Json("outline entry missing string title".into()))?
        .to_owned();
    let page = value
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    let uri = value.get("uri").and_then(Value::as_str).map(str::to_owned);
    let open = value.get("open").and_then(Value::as_bool).unwrap_or(true);
    let style_val = value.get("style");
    let bold = style_val
        .and_then(|s| s.get("bold"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let italic = style_val
        .and_then(|s| s.get("italic"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let color = style_val
        .and_then(|s| s.get("color"))
        .and_then(Value::as_str)
        .and_then(parse_hex_rgb);
    let children = match value.get("children").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .map(entry_from_json)
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(Entry {
        title,
        page,
        uri,
        open,
        style: OutlineStyle {
            bold,
            italic,
            color,
        },
        children,
    })
}

fn parse_hex_rgb(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&digits[0..2], 16).ok()?;
    let g = u8::from_str_radix(&digits[2..4], 16).ok()?;
    let b = u8::from_str_radix(&digits[4..6], 16).ok()?;
    Some([r, g, b])
}

// ---------------------------------------------------------------------------
// 3. pdftk BookmarkBegin format
// ---------------------------------------------------------------------------

/// Exports `entries` in pdftk's `dump_data` / `update_info` `BookmarkBegin` format (plan B.11).
#[must_use]
pub fn export_pdftk(entries: &[Entry]) -> String {
    fn write_items(out: &mut String, entries: &[Entry], level: usize) {
        for entry in entries {
            let _ = writeln!(out, "BookmarkBegin");
            let _ = writeln!(out, "BookmarkTitle: {}", escape_pdftk(&entry.title));
            let _ = writeln!(out, "BookmarkLevel: {level}");
            let page_num = entry.page.map_or(0, |p| p + 1);
            let _ = writeln!(out, "BookmarkPageNumber: {page_num}");
            write_items(out, &entry.children, level + 1);
        }
    }
    let mut out = String::new();
    write_items(&mut out, entries, 1);
    out
}

/// Imports an outline in pdftk's `BookmarkBegin` format (plan B.11).
///
/// # Errors
///
/// Returns [`OutlineIoError::Parse`] if a bookmark record is missing a title or has an invalid level.
pub fn import_pdftk(text: &str) -> Result<Vec<Entry>> {
    let mut flat: Vec<(usize, Entry)> = Vec::new();
    let mut in_record = false;
    let mut title: Option<String> = None;
    let mut level: usize = 1;
    let mut page: Option<usize> = None;

    let flush = |in_record: &mut bool,
                 title: &mut Option<String>,
                 level: &mut usize,
                 page: &mut Option<usize>,
                 flat: &mut Vec<(usize, Entry)>|
     -> Result<()> {
        if !*in_record {
            return Ok(());
        }
        let t = title
            .take()
            .ok_or_else(|| OutlineIoError::Parse("BookmarkBegin missing BookmarkTitle".into()))?;
        let depth = level.saturating_sub(1);
        let p = page.take();
        let uri = p.map(|idx| format!("#page={}", idx + 1));
        flat.push((
            depth,
            Entry {
                title: t,
                page: p,
                uri,
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
        ));
        *level = 1;
        *in_record = false;
        Ok(())
    };

    for raw in text.lines() {
        let line = raw.trim();
        if line == "BookmarkBegin" {
            flush(&mut in_record, &mut title, &mut level, &mut page, &mut flat)?;
            in_record = true;
        } else if let Some(val) = line.strip_prefix("BookmarkTitle:") {
            in_record = true;
            title = Some(unescape_pdftk(val.trim()));
        } else if let Some(val) = line.strip_prefix("BookmarkLevel:") {
            level = val
                .trim()
                .parse::<usize>()
                .map_err(|_| OutlineIoError::Parse(format!("invalid BookmarkLevel: {val}")))?;
            if level == 0 {
                return Err(OutlineIoError::Parse("BookmarkLevel must be >= 1".into()));
            }
        } else if let Some(val) = line.strip_prefix("BookmarkPageNumber:") {
            let n = val
                .trim()
                .parse::<usize>()
                .map_err(|_| OutlineIoError::Parse(format!("invalid BookmarkPageNumber: {val}")))?;
            page = (n > 0).then_some(n - 1);
        }
    }
    flush(&mut in_record, &mut title, &mut level, &mut page, &mut flat)?;
    Ok(nest_by_depth(flat))
}

fn escape_pdftk(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\r', '\n'], " ")
}

fn unescape_pdftk(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

// ---------------------------------------------------------------------------
// 4. Markdown
// ---------------------------------------------------------------------------

/// Exports `entries` as a nested Markdown bullet list (`- [Title](#page=N)`).
#[must_use]
pub fn export_markdown(entries: &[Entry]) -> String {
    fn write_md(out: &mut String, entries: &[Entry], depth: usize) {
        for entry in entries {
            for _ in 0..depth {
                out.push_str("  ");
            }
            let escaped_title = entry.title.replace(['[', ']'], "");
            if let Some(uri) = &entry.uri {
                let _ = writeln!(out, "- [{escaped_title}]({uri})");
            } else if let Some(page) = entry.page {
                let _ = writeln!(out, "- [{escaped_title}](#page={})", page + 1);
            } else {
                let _ = writeln!(out, "- {escaped_title}");
            }
            write_md(out, &entry.children, depth + 1);
        }
    }
    let mut out = String::new();
    write_md(&mut out, entries, 0);
    out
}

/// Imports a nested Markdown bullet list produced by [`export_markdown`].
///
/// # Errors
///
/// Returns [`OutlineIoError::Parse`] on malformed list lines.
pub fn import_markdown(md: &str) -> Result<Vec<Entry>> {
    let mut flat: Vec<(usize, Entry)> = Vec::new();
    for raw in md.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let spaces = raw.bytes().take_while(|&b| b == b' ').count();
        let depth = spaces / 2;
        let trimmed = raw.trim_start();
        let body = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .ok_or_else(|| OutlineIoError::Parse(format!("expected markdown list item: {raw}")))?;

        let (title, page, uri) = if let Some(after_open) = body.strip_prefix('[')
            && let Some((label, rest)) = after_open.split_once("](")
            && let Some(link) = rest.strip_suffix(')')
        {
            let page = link
                .strip_prefix("#page=")
                .and_then(|s| s.split('&').next())
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|&n| n > 0)
                .map(|n| n - 1);
            (label.to_owned(), page, Some(link.to_owned()))
        } else {
            (body.to_owned(), None, None)
        };

        flat.push((
            depth,
            Entry {
                title,
                page,
                uri,
                open: true,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
        ));
    }
    Ok(nest_by_depth(flat))
}

fn nest_by_depth(flat: Vec<(usize, Entry)>) -> Vec<Entry> {
    let mut roots: Vec<Entry> = Vec::new();
    let mut stack: Vec<(usize, Entry)> = Vec::new();
    let pop_one = |stack: &mut Vec<(usize, Entry)>, roots: &mut Vec<Entry>| {
        if let Some((_, done)) = stack.pop() {
            if let Some((_, parent)) = stack.last_mut() {
                parent.children.push(done);
            } else {
                roots.push(done);
            }
        }
    };
    for (depth, entry) in flat {
        while stack.last().is_some_and(|(d, _)| *d >= depth) {
            pop_one(&mut stack, &mut roots);
        }
        stack.push((depth, entry));
    }
    while !stack.is_empty() {
        pop_one(&mut stack, &mut roots);
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<Entry> {
        vec![
            Entry {
                title: "Chapter 1".into(),
                page: Some(0),
                uri: Some("#page=1".into()),
                open: true,
                style: OutlineStyle {
                    bold: true,
                    italic: false,
                    color: Some([0x2f, 0x6f, 0xd6]),
                },
                children: vec![Entry {
                    title: "Section 1.1".into(),
                    page: Some(2),
                    uri: Some("#page=3".into()),
                    open: true,
                    style: OutlineStyle::default(),
                    children: Vec::new(),
                }],
            },
            Entry {
                title: "Chapter 2 & Notes".into(),
                page: Some(9),
                uri: Some("#page=10".into()),
                open: false,
                style: OutlineStyle::default(),
                children: Vec::new(),
            },
        ]
    }

    #[test]
    fn json_round_trips_losslessly() {
        let entries = sample_entries();
        let json_str = export_json(&entries).expect("export json");
        let back = import_json(&json_str).expect("import json");
        assert_eq!(back, entries);
    }

    #[test]
    fn indented_pdftk_and_markdown_round_trip_structure_and_pages() {
        let entries: Vec<Entry> = sample_entries()
            .into_iter()
            .map(|mut e| {
                e.style = OutlineStyle::default();
                e.open = true;
                e
            })
            .collect();

        let indented = export_indented_text(&entries);
        assert_eq!(import_indented_text(&indented).expect("indented"), entries);

        let pdftk = export_pdftk(&entries);
        assert_eq!(import_pdftk(&pdftk).expect("pdftk"), entries);

        let md = export_markdown(&entries);
        assert_eq!(import_markdown(&md).expect("markdown"), entries);
    }
}
