//! Form data export and import in JSON, CSV, FDF, and XFDF formats (plan O.12).

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io;

use excellent_core::forms::{AcroForm, FieldKind, FieldValue, FormError};
use quick_xml::escape::{escape, resolve_predefined_entity};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde_json::{Map, Value};

/// Supported interchange formats for form field data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormExchangeFormat {
    /// Key-value JSON object (`{"fields": {...}}`).
    Json,
    /// RFC 4180 CSV with field names as column headers.
    Csv,
    /// PDF Forms Data Format (`%FDF-1.2`).
    Fdf,
    /// ISO 19444-1 XML Forms Data Format (`<xfdf>`).
    Xfdf,
}

/// Which fields are included when exporting form values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportScope {
    /// Every non-signature, non-pushbutton field regardless of flags.
    AllFields,
    /// Only fields without `FieldFlags::NO_EXPORT` (default PDF submission rule).
    #[default]
    ExportableOnly,
    /// Exportable fields whose current value is non-empty.
    NonEmptyOnly,
}

/// Errors produced while serializing or importing form data.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum FormExchangeError {
    /// Core form update or calculation failure.
    Form(FormError),
    /// JSON parsing or formatting error.
    Json(serde_json::Error),
    /// XML parsing error while reading XFDF.
    Xml(String),
    /// Malformed CSV or FDF input.
    Format(String),
    /// Underlying I/O error.
    Io(io::Error),
}

impl fmt::Display for FormExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Form(err) => write!(f, "{err}"),
            Self::Json(err) => write!(f, "invalid form JSON: {err}"),
            Self::Xml(msg) => write!(f, "invalid form XFDF: {msg}"),
            Self::Format(msg) => write!(f, "invalid form data: {msg}"),
            Self::Io(err) => write!(f, "form exchange I/O error: {err}"),
        }
    }
}

impl std::error::Error for FormExchangeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Form(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Xml(_) | Self::Format(_) => None,
        }
    }
}

impl From<FormError> for FormExchangeError {
    fn from(err: FormError) -> Self {
        Self::Form(err)
    }
}

impl From<serde_json::Error> for FormExchangeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<io::Error> for FormExchangeError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Summary of applying imported form data to an [`AcroForm`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportSummary {
    /// Number of fields updated with imported values.
    pub updated: usize,
    /// Number of matching fields skipped because they are read-only.
    pub skipped_read_only: usize,
    /// Imported field names that did not match any field in the target form.
    pub unknown_fields: Vec<String>,
    /// Dependent fields recomputed by the calculation DAG after import.
    pub calculated_fields: Vec<String>,
}

fn should_export(field: &excellent_core::forms::FormField, scope: ExportScope) -> bool {
    if matches!(
        field.kind,
        FieldKind::Signature { .. } | FieldKind::Pushbutton { .. }
    ) {
        return false;
    }
    match scope {
        ExportScope::AllFields => true,
        ExportScope::ExportableOnly => !field.flags.is_no_export(),
        ExportScope::NonEmptyOnly => !field.flags.is_no_export() && !field.value.is_empty(),
    }
}

/// Exports `form` values to pretty-printed JSON.
///
/// # Errors
///
/// Returns [`FormExchangeError::Json`] if serialization fails.
pub fn export_json(form: &AcroForm, scope: ExportScope) -> Result<String, FormExchangeError> {
    let mut map = Map::new();
    for field in form.fields() {
        if !should_export(field, scope) {
            continue;
        }
        let val = match &field.value {
            FieldValue::None => Value::Null,
            FieldValue::Text(s) => Value::String(s.clone()),
            FieldValue::Boolean(b) => Value::Bool(*b),
            FieldValue::Choice(items) => {
                if items.len() == 1 {
                    Value::String(items[0].clone())
                } else {
                    Value::Array(items.iter().cloned().map(Value::String).collect())
                }
            }
        };
        map.insert(field.export_name().to_owned(), val);
    }
    let root = Value::Object(map);
    Ok(serde_json::to_string_pretty(&root)?)
}

/// Imports form values from JSON (`{"field": value}` or `{"fields": {...}}`) into `form`.
///
/// # Errors
///
/// Returns [`FormExchangeError`] if `json` is invalid or calculation fails.
pub fn import_json(form: &mut AcroForm, json: &str) -> Result<ImportSummary, FormExchangeError> {
    let parsed: Value = serde_json::from_str(json)?;
    let obj = parsed
        .as_object()
        .and_then(|root| root.get("fields").and_then(Value::as_object).or(Some(root)))
        .ok_or_else(|| FormExchangeError::Format("JSON root must be an object".to_owned()))?;

    let mut map = BTreeMap::new();
    for (k, v) in obj {
        let fv = match v {
            Value::Null => FieldValue::None,
            Value::Bool(b) => FieldValue::Boolean(*b),
            Value::String(s) => FieldValue::Text(s.clone()),
            Value::Number(n) => FieldValue::Text(n.to_string()),
            Value::Array(arr) => FieldValue::Choice(
                arr.iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect(),
            ),
            Value::Object(_) => {
                return Err(FormExchangeError::Format(format!(
                    "nested object value unsupported for field {k:?}"
                )));
            }
        };
        map.insert(k.clone(), fv);
    }
    apply_field_map(form, &map)
}

/// Exports a single `form` as a two-row RFC 4180 CSV (header row + value row).
///
/// # Errors
///
/// Returns [`FormExchangeError`] if serialization fails.
pub fn export_csv(form: &AcroForm, scope: ExportScope) -> Result<String, FormExchangeError> {
    export_csv_rows(&[("form", form)], scope)
}

/// Exports multiple filled forms sharing field names as rows in a single RFC 4180 CSV table.
///
/// # Errors
///
/// Returns [`FormExchangeError`] if `forms` is empty.
pub fn export_csv_rows(
    forms: &[(&str, &AcroForm)],
    scope: ExportScope,
) -> Result<String, FormExchangeError> {
    if forms.is_empty() {
        return Err(FormExchangeError::Format(
            "at least one form is required for CSV export".to_owned(),
        ));
    }
    let mut headers = Vec::new();
    for (_, form) in forms {
        for field in form.fields() {
            if should_export(field, scope) {
                let name = field.export_name().to_owned();
                if !headers.contains(&name) {
                    headers.push(name);
                }
            }
        }
    }

    let mut out = String::new();
    for (idx, h) in headers.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&csv_escape(h));
    }
    out.push('\n');

    for (_, form) in forms {
        for (idx, h) in headers.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            let val = form
                .fields()
                .iter()
                .find(|f| f.export_name() == h || f.name == *h)
                .map(|f| f.value.as_display_string())
                .unwrap_or_default();
            out.push_str(&csv_escape(&val));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Imports the first data row of an RFC 4180 CSV document into `form`.
///
/// # Errors
///
/// Returns [`FormExchangeError`] if the CSV is malformed or lacks header/data rows.
pub fn import_csv(form: &mut AcroForm, csv: &str) -> Result<ImportSummary, FormExchangeError> {
    let rows = parse_csv(csv)?;
    if rows.len() < 2 {
        return Err(FormExchangeError::Format(
            "CSV must contain a header row and at least one data row".to_owned(),
        ));
    }
    let headers = &rows[0];
    let values = &rows[1];
    let mut map = BTreeMap::new();
    for (idx, header) in headers.iter().enumerate() {
        if let Some(raw) = values.get(idx) {
            let fv = if raw.is_empty() {
                FieldValue::None
            } else {
                FieldValue::Text(raw.clone())
            };
            map.insert(header.clone(), fv);
        }
    }
    apply_field_map(form, &map)
}

/// Exports `form` fields to ISO 19444-1 XFDF XML (`<xfdf><fields>...</fields></xfdf>`).
#[must_use]
pub fn export_xfdf(form: &AcroForm, pdf_href: Option<&str>, scope: ExportScope) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n",
    );
    if let Some(href) = pdf_href {
        let _ = writeln!(out, "<f href=\"{}\"/>", escape(href));
    }
    out.push_str("<fields>\n");
    for field in form.fields() {
        if !should_export(field, scope) {
            continue;
        }
        let segments: Vec<&str> = field.export_name().split('.').collect();
        for seg in &segments {
            let _ = write!(out, "<field name=\"{}\">", escape(*seg));
        }
        match &field.value {
            FieldValue::Choice(items) if items.len() > 1 => {
                for item in items {
                    let _ = write!(out, "<value>{}</value>", escape(item));
                }
            }
            other => {
                let _ = write!(out, "<value>{}</value>", escape(other.as_display_string()));
            }
        }
        for _ in &segments {
            out.push_str("</field>");
        }
        out.push('\n');
    }
    out.push_str("</fields>\n</xfdf>\n");
    out
}

/// Imports form field values from ISO 19444-1 XFDF XML into `form`.
///
/// # Errors
///
/// Returns [`FormExchangeError::Xml`] if the XML is malformed.
pub fn import_xfdf(form: &mut AcroForm, xfdf: &str) -> Result<ImportSummary, FormExchangeError> {
    let mut reader = Reader::from_str(xfdf);
    reader.config_mut().trim_text(false);
    let mut field_stack: Vec<String> = Vec::new();
    let mut in_value = false;
    let mut cur_value = String::new();
    let mut collected: BTreeMap<String, Vec<String>> = BTreeMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = e.local_name().as_ref().to_owned();
                if tag == "field" {
                    let mut name = String::new();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == "name" {
                            name = attr
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|err| FormExchangeError::Xml(err.to_string()))?
                                .into_owned();
                        }
                    }
                    field_stack.push(name);
                } else if tag == "value" && !field_stack.is_empty() {
                    in_value = true;
                    cur_value.clear();
                }
            }
            Ok(Event::Empty(e)) => {
                let tag = e.local_name().as_ref().to_owned();
                if tag == "value" && !field_stack.is_empty() {
                    let key = field_stack.join(".");
                    collected.entry(key).or_default().push(String::new());
                }
            }
            Ok(Event::Text(t)) if in_value => {
                cur_value.push_str(&t.xml_content(XmlVersion::Implicit1_0));
            }
            Ok(Event::GeneralRef(r)) if in_value => {
                let name = r.xml_content(XmlVersion::Implicit1_0);
                let resolved = match r
                    .resolve_char_ref()
                    .map_err(|err| FormExchangeError::Xml(err.to_string()))?
                {
                    Some(ch) => ch.to_string(),
                    None => resolve_predefined_entity(&name)
                        .ok_or_else(|| FormExchangeError::Xml(format!("unknown entity &{name};")))?
                        .to_owned(),
                };
                cur_value.push_str(&resolved);
            }
            Ok(Event::End(e)) => {
                let tag = e.local_name().as_ref().to_owned();
                if tag == "value" && in_value {
                    in_value = false;
                    let key = field_stack.join(".");
                    collected.entry(key).or_default().push(cur_value.clone());
                } else if tag == "field" {
                    field_stack.pop();
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(FormExchangeError::Xml(err.to_string())),
            _ => {}
        }
    }

    let map: BTreeMap<String, FieldValue> = collected
        .into_iter()
        .map(|(k, vals)| {
            let fv = if vals.is_empty() {
                FieldValue::None
            } else if vals.len() == 1 {
                if vals[0].is_empty() {
                    FieldValue::None
                } else {
                    FieldValue::Text(vals[0].clone())
                }
            } else {
                FieldValue::Choice(vals)
            };
            (k, fv)
        })
        .collect();
    apply_field_map(form, &map)
}

/// Exports `form` values as a `%FDF-1.2` PDF Forms Data Format document.
#[must_use]
pub fn export_fdf(form: &AcroForm, pdf_href: Option<&str>, scope: ExportScope) -> Vec<u8> {
    let mut fields_str = String::new();
    for field in form.fields() {
        if !should_export(field, scope) {
            continue;
        }
        let name_lit = fdf_escape_literal(field.export_name());
        let val_str = match &field.value {
            FieldValue::Choice(items) if items.len() > 1 => {
                let arr: Vec<String> = items.iter().map(|s| fdf_escape_literal(s)).collect();
                format!("[{}]", arr.join(" "))
            }
            other => fdf_escape_literal(&other.as_display_string()),
        };
        let _ = write!(fields_str, "<< /T {name_lit} /V {val_str} >> ");
    }

    let f_entry = pdf_href
        .map(|h| format!("/F {} ", fdf_escape_literal(h)))
        .unwrap_or_default();
    let body = format!(
        "%FDF-1.2\n1 0 obj\n<< /FDF << {f_entry}/Fields [ {fields_str}] >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n"
    );
    body.into_bytes()
}

/// Imports form field values from FDF bytes (`%FDF-1.2`) into `form`.
///
/// # Errors
///
/// Returns [`FormExchangeError::Format`] if `fdf_bytes` does not look like valid FDF.
pub fn import_fdf(
    form: &mut AcroForm,
    fdf_bytes: &[u8],
) -> Result<ImportSummary, FormExchangeError> {
    if !fdf_bytes.windows(4).take(64).any(|w| w == b"%FDF") {
        return Err(FormExchangeError::Format("missing %FDF header".to_owned()));
    }
    let text = String::from_utf8_lossy(fdf_bytes);
    let mut map = BTreeMap::new();
    let mut pos = 0usize;
    let bytes = text.as_bytes();
    while let Some(rel) = text[pos..].find("/T") {
        let t_pos = pos + rel + 2;
        if let Some((name, after_t)) = read_fdf_string(bytes, t_pos)
            && let Some(v_rel) = text[after_t..].find("/V")
        {
            let v_pos = after_t + v_rel + 2;
            let trimmed = skip_ws(bytes, v_pos);
            if trimmed < bytes.len() && bytes[trimmed] == b'/' {
                // Name value like /Yes or /Off
                let mut end = trimmed + 1;
                while end < bytes.len()
                    && !bytes[end].is_ascii_whitespace()
                    && !matches!(bytes[end], b'>' | b'/' | b']')
                {
                    end += 1;
                }
                let val = String::from_utf8_lossy(&bytes[trimmed + 1..end]).into_owned();
                map.insert(name, FieldValue::Text(val));
                pos = end;
                continue;
            } else if let Some((val, after_v)) = read_fdf_string(bytes, v_pos) {
                let fv = if val.is_empty() {
                    FieldValue::None
                } else {
                    FieldValue::Text(val)
                };
                map.insert(name, fv);
                pos = after_v;
                continue;
            }
        }
        pos = t_pos;
    }
    apply_field_map(form, &map)
}

pub(crate) fn apply_field_map(
    form: &mut AcroForm,
    map: &BTreeMap<String, FieldValue>,
) -> Result<ImportSummary, FormExchangeError> {
    let mut summary = ImportSummary::default();
    for (key, value) in map {
        let target_name = form
            .fields()
            .iter()
            .find(|f| f.name == *key || f.export_name() == key)
            .map(|f| f.name.clone());
        let Some(name) = target_name else {
            summary.unknown_fields.push(key.clone());
            continue;
        };
        let field = form.field(&name).expect("field resolved");
        if field.flags.is_read_only() {
            summary.skipped_read_only += 1;
            continue;
        }
        if matches!(
            field.kind,
            FieldKind::Signature { .. } | FieldKind::Pushbutton { .. }
        ) {
            continue;
        }
        match form.set_value(&name, value.clone()) {
            Ok(_) => summary.updated += 1,
            Err(FormError::ReadOnlyField(_)) => summary.skipped_read_only += 1,
            Err(err) => return Err(err.into()),
        }
    }
    let calc = form.recalculate()?;
    summary.calculated_fields = calc.updated_fields;
    Ok(summary)
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn parse_csv(src: &str) -> Result<Vec<Vec<String>>, FormExchangeError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut chars = src.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cell.push(ch);
            }
        } else {
            match ch {
                '"' if cell.is_empty() => in_quotes = true,
                ',' => {
                    row.push(std::mem::take(&mut cell));
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                '\n' => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                other => cell.push(other),
            }
        }
    }
    if in_quotes {
        return Err(FormExchangeError::Format(
            "unterminated CSV quote".to_owned(),
        ));
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    Ok(rows)
}

fn fdf_escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('(');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push(')');
    out
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn read_fdf_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let mut i = skip_ws(bytes, start);
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    i += 1;
    let mut out = Vec::new();
    let mut depth = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        match b {
            b'(' => {
                depth += 1;
                out.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((String::from_utf8_lossy(&out).into_owned(), i));
                }
                out.push(b')');
            }
            b'\\' => {
                let esc = *bytes.get(i)?;
                i += 1;
                match esc {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}
