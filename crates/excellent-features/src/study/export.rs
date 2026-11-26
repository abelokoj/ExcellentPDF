//! Study export (plan S.8): exact parity port of SumatraPDF Enhanced's `StudyExport.cpp` covering Markdown, plain
//! text, Typst, HTML, Word (`.docx`), JSON and formula-safe CSV exports of highlights, notes and vocabulary.

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Supported study-export output formats (matching SumatraPDF Enhanced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StudyFormat {
    /// GitHub-flavoured Markdown with escaped characters and `file://` source links.
    Markdown,
    /// Skim-style plain text.
    Text,
    /// Compilable Typst markup.
    Typst,
    /// Self-contained styled HTML5 document.
    Html,
    /// Office Open XML Word document (`.docx`).
    Docx,
    /// Structured JSON (`schema: 1`).
    Json,
    /// Spreadsheet-safe RFC 4180 CSV (leading formula characters escaped).
    Csv,
}

impl StudyFormat {
    /// All seven formats in Enhanced's UI order.
    pub const ALL: [Self; 7] = [
        Self::Markdown,
        Self::Text,
        Self::Typst,
        Self::Html,
        Self::Docx,
        Self::Json,
        Self::Csv,
    ];

    /// Default file extension without leading dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Typst => "typ",
            Self::Html => "html",
            Self::Docx => "docx",
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }
}

/// Annotation kind names matching SumatraPDF Enhanced's `AnnotationTypeNames()` and `NoteTypeName()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StudyNoteKind {
    /// Sticky note (`Text` / `"Note"`).
    Text,
    /// Free text (`FreeText` / `"Free Text"`).
    FreeText,
    /// Highlight.
    Highlight,
    /// Underline.
    Underline,
    /// Squiggly underline.
    Squiggly,
    /// Strikeout (`StrikeOut` / `"Strike Out"`).
    StrikeOut,
    /// Caret insertion mark.
    Caret,
    /// Freehand ink drawing.
    Ink,
    /// Line shape.
    Line,
    /// Square shape.
    Square,
    /// Circle shape.
    Circle,
    /// Polygon shape.
    Polygon,
    /// Polyline shape (`PolyLine` / `"Polyline"`).
    PolyLine,
    /// Stamp.
    Stamp,
}

impl StudyNoteKind {
    /// Raw PDF/Enhanced annotation type name (used in JSON and CSV).
    #[must_use]
    pub const fn raw_name(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::FreeText => "FreeText",
            Self::Highlight => "Highlight",
            Self::Underline => "Underline",
            Self::Squiggly => "Squiggly",
            Self::StrikeOut => "StrikeOut",
            Self::Caret => "Caret",
            Self::Ink => "Ink",
            Self::Line => "Line",
            Self::Square => "Square",
            Self::Circle => "Circle",
            Self::Polygon => "Polygon",
            Self::PolyLine => "PolyLine",
            Self::Stamp => "Stamp",
        }
    }

    /// Display heading name matching Enhanced's `NoteTypeName()`.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Text => "Note",
            Self::FreeText => "Free Text",
            Self::StrikeOut => "Strike Out",
            Self::PolyLine => "Polyline",
            _ => self.raw_name(),
        }
    }

    /// Whether this kind is a non-text drawing annotation (matching Enhanced's `CollectStudyNotes`).
    #[must_use]
    pub const fn is_drawing(self) -> bool {
        !matches!(
            self,
            Self::Text
                | Self::FreeText
                | Self::Highlight
                | Self::Underline
                | Self::Squiggly
                | Self::StrikeOut
                | Self::Caret
        )
    }
}

/// One highlight or annotation note prepared for study export (matching Enhanced's `StudyNote`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyNote {
    /// 1-based page number.
    pub page: usize,
    /// Original order index within the document.
    pub ordinal: usize,
    /// Annotation type.
    pub kind: StudyNoteKind,
    /// RGBA colour (`Some([r, g, b, a])`, or `None` when unset).
    pub color: Option<[u8; 4]>,
    /// Creation timestamp (UTC seconds since 1970, `0` if unknown).
    pub created: i64,
    /// Modification timestamp (UTC seconds since 1970, `0` if unknown).
    pub modified: i64,
    /// Printed page label (e.g. `"ii"`), empty if none.
    pub page_label: String,
    /// Highlighted quote extracted from the page.
    pub quote: String,
    /// User comment/note text.
    pub comment: String,
    /// Author name.
    pub author: String,
    /// Whether this annotation is currently selected.
    pub selected: bool,
    /// Whether this annotation is a non-text drawing.
    pub drawing: bool,
}

/// Document metadata and collected annotations for study export (matching Enhanced's `StudyDocument`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StudyDocument {
    /// Document title (or file base name).
    pub title: String,
    /// Source file path.
    pub source: String,
    /// Total page count.
    pub pages: usize,
    /// Collected notes.
    pub notes: Vec<StudyNote>,
}

/// Sort order for exported notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StudyOrder {
    /// Ascending page number, then document ordinal.
    #[default]
    PageOrder,
    /// Ascending creation time with unknown dates (`0`) last, then document ordinal.
    CreationOrder,
}

/// Filter applied before rendering study notes (matching Enhanced's `StudyFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudyFilter {
    /// Inclusive first 1-based page.
    pub from_page: usize,
    /// Inclusive last 1-based page.
    pub to_page: usize,
    /// Optional annotation kind filter.
    pub kind: Option<StudyNoteKind>,
    /// Optional exact RGBA colour filter.
    pub color: Option<[u8; 4]>,
    /// Restrict to currently selected annotations.
    pub selection_only: bool,
    /// Output sort order.
    pub order: StudyOrder,
}

impl Default for StudyFilter {
    fn default() -> Self {
        Self {
            from_page: 1,
            to_page: usize::MAX,
            kind: None,
            color: None,
            selection_only: false,
            order: StudyOrder::PageOrder,
        }
    }
}

/// Filters and sorts `doc.notes` according to `filter` (matching Enhanced's `SelectStudyNotes`).
#[must_use]
pub fn select_notes(doc: &StudyDocument, filter: StudyFilter) -> Vec<&StudyNote> {
    let mut out: Vec<&StudyNote> = doc
        .notes
        .iter()
        .filter(|note| {
            note.page >= filter.from_page
                && note.page <= filter.to_page
                && (!filter.selection_only || note.selected)
                && filter.kind.is_none_or(|k| note.kind == k)
                && filter.color.is_none_or(|c| note.color == Some(c))
        })
        .collect();

    out.sort_by(|a, b| a.page.cmp(&b.page).then_with(|| a.ordinal.cmp(&b.ordinal)));
    if filter.order == StudyOrder::CreationOrder {
        out.sort_by(|a, b| {
            let cmp = match (a.created, b.created) {
                (0, 0) => std::cmp::Ordering::Equal,
                (0, _) => std::cmp::Ordering::Greater,
                (_, 0) => std::cmp::Ordering::Less,
                (x, y) => x.cmp(&y),
            };
            cmp.then_with(|| a.ordinal.cmp(&b.ordinal))
        });
    }
    out
}

/// Formats UTC seconds since 1970 as ISO-8601 `YYYY-MM-DDTHH:MM:SSZ` (empty when `date == 0`).
#[must_use]
pub fn study_date(date: i64) -> String {
    if date <= 0 {
        return String::new();
    }
    let (year, month, day, hour, minute, second) = utc_ymdhms(date);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Formats UTC seconds since 1970 as `YYYY-MM-DD HH:MM:SS` (empty when `date == 0`).
#[must_use]
pub fn study_timestamp(date: i64) -> String {
    if date <= 0 {
        return String::new();
    }
    let (year, month, day, hour, minute, second) = utc_ymdhms(date);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Formats an RGBA colour as `#rrggbbaa`, or `"none"` when unset (matching Enhanced's `StudyColor`).
#[must_use]
pub fn study_color(color: Option<[u8; 4]>) -> String {
    match color {
        Some([r, g, b, a]) => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
        None => "none".to_owned(),
    }
}

/// Builds a percent-encoded `file://...#page=<n>` link (matching Enhanced's `StudyLink`).
#[must_use]
pub fn study_link(source: &str, page: usize) -> String {
    let mut out = String::with_capacity(source.len() + 24);
    if source.starts_with("\\\\") {
        out.push_str("file:");
    } else {
        out.push_str("file:///");
    }
    for &raw in source.as_bytes() {
        let c = if raw == b'\\' { b'/' } else { raw };
        if c.is_ascii_alphanumeric() || matches!(c, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(c));
        } else {
            let _ = write!(out, "%{c:02X}");
        }
    }
    let _ = write!(out, "#page={page}");
    out
}

/// Escapes a JSON string with surrounding double quotes (matching Enhanced's `JsonText`).
#[must_use]
pub fn json_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escapes XML/HTML text and drops illegal control characters (matching Enhanced's `XmlText`).
#[must_use]
pub fn xml_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) >= 32 || matches!(c, '\n' | '\r' | '\t') => out.push(c),
            _ => {}
        }
    }
    out
}

/// Escapes a Typst string literal with surrounding double quotes (matching Enhanced's `TypstText`).
#[must_use]
pub fn typst_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escapes Markdown syntax characters and turns newlines into hard breaks (`"  \n"`), matching Enhanced's
/// `MarkdownText`.
#[must_use]
pub fn markdown_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '\r' {
            continue;
        }
        if ch == '\n' {
            out.push_str("  \n");
            continue;
        }
        if "\\`*_{}[]()<>#+-.!|~".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Escapes a CSV cell and neutralizes spreadsheet formula injection (`=`, `+`, `-`, `@`, `\t`, `\r`) by prefixing `'`,
/// matching Enhanced's `CsvText`.
#[must_use]
pub fn csv_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 3);
    out.push('"');
    if let Some(first) = value.bytes().next()
        && matches!(first, b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')
    {
        out.push('\'');
    }
    for ch in value.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Formats a note's page label (`"page 2"` or `"page ii (2)"`), matching Enhanced's `NotePageLabel`.
#[must_use]
pub fn note_page_label(note: &StudyNote) -> String {
    if !note.page_label.is_empty() && note.page_label != note.page.to_string() {
        format!("page {} ({})", note.page_label, note.page)
    } else {
        format!("page {}", note.page)
    }
}

/// Formats a note's Skim-style heading (`"Highlight, page ii (2)"`), matching Enhanced's `NoteHeading`.
#[must_use]
pub fn note_heading(note: &StudyNote) -> String {
    format!("{}, {}", note.kind.display_name(), note_page_label(note))
}

/// Formats a note's metadata line (`Date`, `Author`, `Color`), matching Enhanced's `NoteDetails`.
#[must_use]
pub fn note_details(note: &StudyNote) -> String {
    let mut out = String::new();
    if note.created > 0 {
        let _ = write!(out, "Date: {}", study_timestamp(note.created));
    } else if note.modified > 0 {
        let _ = write!(out, "Modified: {}", study_timestamp(note.modified));
    }
    if !note.author.is_empty() {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        let _ = write!(out, "Author: {}", note.author);
    }
    if let Some(color) = note.color {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        let _ = write!(out, "Color: {}", study_color(Some(color)));
    }
    out
}

/// Formats a note's plain-text body, matching Enhanced's `NoteBody`.
#[must_use]
pub fn note_body(note: &StudyNote) -> String {
    let mut out = String::new();
    if !note.quote.is_empty() {
        out.push('"');
        out.push_str(&note.quote);
        out.push_str("\"\n\n");
    }
    if !note.comment.is_empty() {
        if !note.quote.is_empty() {
            out.push_str("Note:\n");
        }
        out.push_str(&note.comment);
        out.push_str("\n\n");
    }
    if note.quote.is_empty() && note.comment.is_empty() {
        if note.drawing {
            out.push_str("Non-text annotation (drawing).\n\n");
        } else {
            out.push_str("No text could be extracted.\n\n");
        }
    }
    out
}

/// Renders `notes` from `doc` into `format`, returning exact bytes (UTF-8 text or `.docx` ZIP archive).
///
/// # Errors
///
/// Only fails if `.docx` ZIP compression fails.
pub fn render_study(
    doc: &StudyDocument,
    notes: &[&StudyNote],
    format: StudyFormat,
) -> Result<Vec<u8>, std::io::Error> {
    if format == StudyFormat::Docx {
        return write_study_docx(doc, notes);
    }
    let mut out = String::new();
    match format {
        StudyFormat::Json => {
            let _ = writeln!(
                out,
                "{{\"schema\":1,\"title\":{},\"source\":{},\"annotations\":[",
                json_text(&doc.title),
                json_text(&doc.source)
            );
            for (i, n) in notes.iter().enumerate() {
                let prefix = if i > 0 { "," } else { "" };
                let created = if n.created > 0 {
                    json_text(&study_date(n.created))
                } else {
                    "null".to_owned()
                };
                let modified = if n.modified > 0 {
                    json_text(&study_date(n.modified))
                } else {
                    "null".to_owned()
                };
                let _ = writeln!(
                    out,
                    "{prefix}{{\"page\":{},\"page_label\":{},\"type\":{},\"color\":{},\"created\":{created},\"modified\":{modified},\"author\":{},\"quote\":{},\"note\":{},\"non_text\":{},\"source_link\":{}}}",
                    n.page,
                    json_text(&n.page_label),
                    json_text(n.kind.raw_name()),
                    json_text(&study_color(n.color)),
                    json_text(&n.author),
                    json_text(&n.quote),
                    json_text(&n.comment),
                    if n.drawing { "true" } else { "false" },
                    json_text(&study_link(&doc.source, n.page)),
                );
            }
            out.push_str("]}\n");
            return Ok(out.into_bytes());
        }
        StudyFormat::Csv => {
            out.push_str("document,source,page,page_label,type,color,created,modified,author,quote,note,non_text,source_link\r\n");
            for n in notes {
                let _ = write!(
                    out,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{}\r\n",
                    csv_text(&doc.title),
                    csv_text(&doc.source),
                    n.page,
                    csv_text(&n.page_label),
                    csv_text(n.kind.raw_name()),
                    csv_text(&study_color(n.color)),
                    csv_text(&study_date(n.created)),
                    csv_text(&study_date(n.modified)),
                    csv_text(&n.author),
                    csv_text(&n.quote),
                    csv_text(&n.comment),
                    if n.drawing { "true" } else { "false" },
                    csv_text(&study_link(&doc.source, n.page)),
                );
            }
            return Ok(out.into_bytes());
        }
        StudyFormat::Html => {
            let _ = writeln!(
                out,
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{}</title><style>body{{font:1.05rem/1.65 system-ui,sans-serif;max-width:54rem;margin:3rem auto;padding:0 1.25rem;color:#17251d}}article{{border-top:1px solid #bccbbf;padding:1.5rem 0}}h2{{margin:0 0 0.5rem;font-size:1.35rem}}small{{display:block;color:#405548;margin-bottom:0.75rem}}blockquote{{border-left:3px solid #166534;margin:1rem 0;padding:0.25rem 1rem;background:#f4f7f5;color:#23382b}}p{{white-space:pre-wrap;overflow-wrap:anywhere}}a{{color:#166534}}</style></head><body><h1>{}</h1><p>Source: {}</p>",
                xml_text(&doc.title),
                xml_text(&doc.title),
                xml_text(&doc.source)
            );
        }
        StudyFormat::Typst => {
            let _ = write!(
                out,
                "#set page(paper: \"a4\", margin: 2cm)\n#set text(size: 11pt)\n#let notes(s) = {{ for (i, line) in s.split(\"\\n\").enumerate() {{ if i > 0 {{ linebreak() }}; text(line) }} }}\n#heading(level: 1, {})\n#notes({})\n\n",
                typst_text(&doc.title),
                typst_text(&format!("Source: {}", doc.source))
            );
        }
        StudyFormat::Markdown => {
            let _ = write!(
                out,
                "# {}\n\nSource: {}\n\n",
                markdown_text(&doc.title),
                markdown_text(&doc.source)
            );
        }
        StudyFormat::Text => {
            let _ = write!(out, "{}\nSource: {}\n\n", doc.title, doc.source);
        }
        StudyFormat::Docx => unreachable!(),
    }

    if notes.is_empty() {
        if format == StudyFormat::Typst {
            out.push_str("#par(\"No annotations match the export filters.\")\n");
        } else {
            out.push_str("No annotations match the export filters.\n");
        }
    }

    for n in notes {
        let heading = note_heading(n);
        let details = note_details(n);
        let body = note_body(n);
        let link = study_link(&doc.source, n.page);
        match format {
            StudyFormat::Html => {
                let _ = writeln!(out, "<article><h2>{}</h2>", xml_text(&heading));
                if !details.is_empty() {
                    let _ = writeln!(out, "<small>{}</small>", xml_text(&details));
                }
                if !n.quote.is_empty() {
                    let _ = writeln!(
                        out,
                        "<blockquote><p>{}</p></blockquote>",
                        xml_text(&n.quote)
                    );
                }
                if !n.comment.is_empty() {
                    if !n.quote.is_empty() {
                        let _ = writeln!(
                            out,
                            "<p><strong>Note:</strong><br/>{}</p>",
                            xml_text(&n.comment)
                        );
                    } else {
                        let _ = writeln!(out, "<p>{}</p>", xml_text(&n.comment));
                    }
                }
                if n.quote.is_empty() && n.comment.is_empty() {
                    if n.drawing {
                        out.push_str("<p><em>Non-text annotation (drawing)</em></p>\n");
                    } else {
                        out.push_str("<p><em>No text could be extracted</em></p>\n");
                    }
                }
                let _ = writeln!(
                    out,
                    "<a href=\"{}\">Open source page</a></article>",
                    xml_text(&link)
                );
            }
            StudyFormat::Typst => {
                let _ = writeln!(out, "#heading(level: 2, {})", typst_text(&heading));
                if !details.is_empty() {
                    let _ = write!(
                        out,
                        "#text(fill: luma(100), size: 9pt, {})\n\n",
                        typst_text(&details)
                    );
                }
                if !n.quote.is_empty() {
                    let _ = write!(
                        out,
                        "#rect(stroke: (left: 2.5pt + rgb(\"#166534\")), inset: (left: 10pt, y: 4pt), fill: rgb(\"#f4f7f5\"))[#notes({})]\n\n",
                        typst_text(&n.quote)
                    );
                }
                if !n.comment.is_empty() {
                    if !n.quote.is_empty() {
                        let _ = write!(
                            out,
                            "*Note:* #linebreak() #notes({})\n\n",
                            typst_text(&n.comment)
                        );
                    } else {
                        let _ = write!(out, "#notes({})\n\n", typst_text(&n.comment));
                    }
                }
                if n.quote.is_empty() && n.comment.is_empty() {
                    if n.drawing {
                        out.push_str("_Non-text annotation (drawing)_\n\n");
                    } else {
                        out.push_str("_No text could be extracted_\n\n");
                    }
                }
                let _ = write!(out, "#link({})[Open source page]\n\n", typst_text(&link));
            }
            StudyFormat::Markdown => {
                let _ = write!(out, "## {}\n\n", markdown_text(&heading));
                if !details.is_empty() {
                    let _ = write!(out, "*{}*\n\n", markdown_text(&details));
                }
                if !n.quote.is_empty() {
                    for line in split_crlf_lines(&n.quote) {
                        out.push_str("> ");
                        out.push_str(&markdown_text(line));
                        out.push_str("  \n");
                    }
                    out.push('\n');
                }
                if !n.comment.is_empty() {
                    if !n.quote.is_empty() {
                        out.push_str("**Note:**  \n");
                    }
                    out.push_str(&markdown_text(&n.comment));
                    out.push_str("\n\n");
                }
                if n.quote.is_empty() && n.comment.is_empty() {
                    if n.drawing {
                        out.push_str("*(Drawing annotation)*\n\n");
                    } else {
                        out.push_str("*(No text extracted)*\n\n");
                    }
                }
                let _ = write!(out, "[Open source page]({link})\n\n");
            }
            StudyFormat::Text => {
                let _ = writeln!(out, "{heading}");
                if !details.is_empty() {
                    let _ = writeln!(out, "{details}");
                }
                out.push('\n');
                out.push_str(&body);
                let _ = write!(out, "Source page: {link}\n\n");
            }
            StudyFormat::Docx | StudyFormat::Json | StudyFormat::Csv => unreachable!(),
        }
    }

    if format == StudyFormat::Html {
        out.push_str("</body></html>\n");
    }
    Ok(out.into_bytes())
}

/// One vocabulary item exported alongside study notes (plan S.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabExportItem {
    /// Headword.
    pub word: String,
    /// Plain-text definition.
    pub definition: String,
    /// Optional reading context sentence.
    pub context: String,
    /// Source document path (may be empty).
    pub source_path: String,
    /// 1-based page number (`0` if unknown).
    pub page: usize,
}

/// Exports vocabulary entries in Markdown, Plain Text, JSON or formula-safe CSV (plan S.8).
#[must_use]
pub fn render_vocabulary_study(items: &[VocabExportItem], format: StudyFormat) -> String {
    let mut out = String::new();
    match format {
        StudyFormat::Csv => {
            out.push_str("word,definition,context,source,page\r\n");
            for item in items {
                let _ = write!(
                    out,
                    "{},{},{},{},{}\r\n",
                    csv_text(&item.word),
                    csv_text(&item.definition),
                    csv_text(&item.context),
                    csv_text(&item.source_path),
                    item.page
                );
            }
        }
        StudyFormat::Json => {
            out.push_str("{\"schema\":1,\"vocabulary\":[\n");
            for (i, item) in items.iter().enumerate() {
                let prefix = if i > 0 { "," } else { "" };
                let _ = writeln!(
                    out,
                    "{prefix}{{\"word\":{},\"definition\":{},\"context\":{},\"source\":{},\"page\":{}}}",
                    json_text(&item.word),
                    json_text(&item.definition),
                    json_text(&item.context),
                    json_text(&item.source_path),
                    item.page
                );
            }
            out.push_str("]}\n");
        }
        StudyFormat::Markdown => {
            out.push_str("# Vocabulary\n\n");
            for item in items {
                let _ = write!(
                    out,
                    "## {}\n\n{}\n\n",
                    markdown_text(&item.word),
                    markdown_text(&item.definition)
                );
                if !item.context.is_empty() {
                    let _ = write!(out, "> {}\n\n", markdown_text(&item.context));
                }
                if !item.source_path.is_empty() && item.page > 0 {
                    let _ = write!(
                        out,
                        "[Open source page]({})\n\n",
                        study_link(&item.source_path, item.page)
                    );
                }
            }
        }
        _ => {
            out.push_str("Vocabulary\n\n");
            for item in items {
                let _ = writeln!(out, "{}\n{}", item.word, item.definition);
                if !item.context.is_empty() {
                    let _ = writeln!(out, "Context: {}", item.context);
                }
                if !item.source_path.is_empty() && item.page > 0 {
                    let _ = writeln!(
                        out,
                        "Source page: {}",
                        study_link(&item.source_path, item.page)
                    );
                }
                out.push('\n');
            }
        }
    }
    out
}

fn write_study_docx(doc: &StudyDocument, notes: &[&StudyNote]) -> Result<Vec<u8>, std::io::Error> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:body>",
    );
    let append_paragraph = |buf: &mut String, text: &str, bold: bool| {
        buf.push_str("<w:p><w:r>");
        if bold {
            buf.push_str("<w:rPr><w:b/></w:rPr>");
        }
        buf.push_str("<w:t xml:space=\"preserve\">");
        for (i, line) in split_crlf_lines(text).enumerate() {
            if i > 0 {
                buf.push_str("</w:t><w:br/><w:t xml:space=\"preserve\">");
            }
            buf.push_str(&xml_text(line));
        }
        buf.push_str("</w:t></w:r></w:p>");
    };

    append_paragraph(&mut xml, &doc.title, true);
    append_paragraph(&mut xml, &format!("Source: {}", doc.source), false);

    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (i, n) in notes.iter().enumerate() {
        append_paragraph(&mut xml, &note_heading(n), true);
        let details = note_details(n);
        if !details.is_empty() {
            append_paragraph(&mut xml, &details, false);
        }
        append_paragraph(&mut xml, &note_body(n), false);
        let _ = write!(
            xml,
            "<w:p><w:hyperlink r:id=\"p{i}\"><w:r><w:rPr><w:color w:val=\"166534\"/><w:u w:val=\"single\"/></w:rPr><w:t>Open source page</w:t></w:r></w:hyperlink></w:p>"
        );
        let _ = write!(
            rels,
            "<Relationship Id=\"p{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            xml_text(&study_link(&doc.source, n.page))
        );
    }
    if notes.is_empty() {
        append_paragraph(&mut xml, "No annotations match the export filters.", false);
    }
    xml.push_str("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr></w:body></w:document>");
    rels.push_str("</Relationships>");

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let write_entry = |zip: &mut ZipWriter<&mut Cursor<Vec<u8>>>, name: &str, data: &str| {
            zip.start_file(name, opts).map_err(std::io::Error::other)?;
            zip.write_all(data.as_bytes())
        };
        write_entry(
            &mut zip,
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
        )?;
        write_entry(
            &mut zip,
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"main\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
        )?;
        write_entry(&mut zip, "word/document.xml", &xml)?;
        write_entry(&mut zip, "word/_rels/document.xml.rels", &rels)?;
        zip.finish().map_err(std::io::Error::other)?;
    }
    Ok(cursor.into_inner())
}

fn split_crlf_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        for (idx, b) in current.bytes().enumerate() {
            if b == b'\n' {
                rest = Some(&current[idx + 1..]);
                return Some(&current[..idx]);
            }
            if b == b'\r' {
                let next = if current.as_bytes().get(idx + 1) == Some(&b'\n') {
                    idx + 2
                } else {
                    idx + 1
                };
                rest = Some(&current[next..]);
                return Some(&current[..idx]);
            }
        }
        rest = None;
        Some(current)
    })
}

/// Converts Unix UTC seconds to `(year, month, day, hour, minute, second)` using Howard Hinnant's civil algorithm.
fn utc_ymdhms(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use zip::ZipArchive;

    use super::*;

    fn enhanced_fixture() -> StudyDocument {
        StudyDocument {
            title: "Study <notes> & café".to_owned(),
            source: r"C:\Courses\Résumé #1.pdf".to_owned(),
            pages: 3,
            notes: vec![
                StudyNote {
                    page: 2,
                    ordinal: 0,
                    kind: StudyNoteKind::Highlight,
                    color: Some([0x00, 0xff, 0x00, 0xff]),
                    created: 1_700_000_000,
                    modified: 0,
                    page_label: "ii".to_owned(),
                    quote: "α + β < 2\nLine two: [quote] # $ \\\"".to_owned(),
                    comment: "=SUM(1,2)\nNotes: 中文\t'&'".to_owned(),
                    author: "Student".to_owned(),
                    selected: true,
                    drawing: false,
                },
                StudyNote {
                    page: 1,
                    ordinal: 1,
                    kind: StudyNoteKind::Ink,
                    color: None,
                    created: 0,
                    modified: 0,
                    page_label: "i".to_owned(),
                    quote: String::new(),
                    comment: String::new(),
                    author: String::new(),
                    selected: false,
                    drawing: true,
                },
            ],
        }
    }

    /// Port of SumatraPDF Enhanced's `StudyExport_UnitTests()` proving output parity across all 7 formats.
    #[test]
    fn matches_enhanced_study_export_unit_tests() {
        let doc = enhanced_fixture();

        // Filter & sort tests.
        let notes = select_notes(&doc, StudyFilter::default());
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].page, 1);

        let by_created = select_notes(
            &doc,
            StudyFilter {
                order: StudyOrder::CreationOrder,
                ..StudyFilter::default()
            },
        );
        assert_eq!(by_created[0].page, 2);

        let selected = select_notes(
            &doc,
            StudyFilter {
                selection_only: true,
                ..StudyFilter::default()
            },
        );
        assert_eq!(selected.len(), 1);
        assert!(selected[0].selected);

        let filtered = select_notes(
            &doc,
            StudyFilter {
                kind: Some(StudyNoteKind::Highlight),
                color: Some([0x00, 0xff, 0x00, 0xff]),
                ..StudyFilter::default()
            },
        );
        assert_eq!(filtered.len(), 1);

        let empty_range = select_notes(
            &doc,
            StudyFilter {
                from_page: 3,
                ..StudyFilter::default()
            },
        );
        assert!(empty_range.is_empty());

        // Escaping & formula-injection safety.
        assert!(json_text("a\n\"b").contains("\\n\\\""));
        assert!(xml_text("<&>").contains("&lt;&amp;&gt;"));
        assert!(csv_text("=1").contains("'=1"));
        assert!(study_link(&doc.source, 2).contains("%23"));

        // All 7 formats render both populated and empty notes cleanly.
        for format in StudyFormat::ALL {
            let bytes = render_study(&doc, &notes, format).expect("render");
            assert!(!bytes.is_empty(), "{format:?}");
            let empty_bytes = render_study(&doc, &[], format).expect("render empty");
            assert!(!empty_bytes.is_empty(), "empty {format:?}");
        }

        // Exact Skim-style plain text assertions matching Enhanced.
        let txt = String::from_utf8(render_study(&doc, &notes, StudyFormat::Text).expect("txt"))
            .expect("utf8");
        assert!(txt.contains("Highlight, page ii (2)"));
        assert!(txt.contains("Date: 2023-11-14 22:13:20"));
        assert!(txt.contains("\"α + β < 2"));
        assert!(txt.contains("Note:\n=SUM(1,2)"));
        assert!(txt.contains("Ink, page i (1)"));
        assert!(txt.contains("Non-text annotation (drawing)."));
        assert!(!txt.contains("Color: none"));

        // Exact Markdown assertions matching Enhanced.
        let md = String::from_utf8(render_study(&doc, &notes, StudyFormat::Markdown).expect("md"))
            .expect("utf8");
        assert!(md.contains("## Highlight, page ii \\(2\\)"));
        assert!(md.contains("*Date: 2023\\-11\\-14 22:13:20"));
        assert!(md.contains("> α \\+ β \\< 2"));
        assert!(md.contains("**Note:**"));
        assert!(md.contains("## Ink, page i \\(1\\)"));
        assert!(md.contains("*(Drawing annotation)*"));

        // CSV escapes `=SUM(1,2)` as `'=SUM(1,2)`.
        let csv = String::from_utf8(render_study(&doc, &notes, StudyFormat::Csv).expect("csv"))
            .expect("utf8");
        assert!(csv.contains("\"'=SUM(1,2)"));

        // JSON parses cleanly with schema 1.
        let json_bytes = render_study(&doc, &notes, StudyFormat::Json).expect("json");
        let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).expect("valid json");
        assert_eq!(parsed["schema"], 1);
        assert_eq!(parsed["annotations"].as_array().expect("array").len(), 2);

        // DOCX is a valid ZIP containing word/document.xml and rels.
        let docx_bytes = render_study(&doc, &notes, StudyFormat::Docx).expect("docx");
        let mut archive = ZipArchive::new(Cursor::new(docx_bytes)).expect("valid docx zip");
        assert!(archive.by_name("word/document.xml").is_ok());
        assert!(archive.by_name("word/_rels/document.xml.rels").is_ok());
        assert!(archive.by_name("[Content_Types].xml").is_ok());
    }

    #[test]
    fn sticky_note_omits_redundant_note_prefix_and_vocab_exports_safely() {
        let doc = enhanced_fixture();
        let sticky = StudyNote {
            page: 3,
            ordinal: 0,
            kind: StudyNoteKind::Text,
            color: None,
            created: 0,
            modified: 0,
            page_label: String::new(),
            quote: String::new(),
            comment: "Sticky thought".to_owned(),
            author: String::new(),
            selected: false,
            drawing: false,
        };
        assert_eq!(note_heading(&sticky), "Note, page 3");
        let txt =
            String::from_utf8(render_study(&doc, &[&sticky], StudyFormat::Text).expect("render"))
                .expect("utf8");
        assert!(txt.contains("Sticky thought"));
        assert!(!txt.contains("Note:\nSticky thought"));

        let vocab_csv = render_vocabulary_study(
            &[VocabExportItem {
                word: "abhor".to_owned(),
                definition: "=cmd|' /C calc'!A0".to_owned(),
                context: "I abhor waiting.".to_owned(),
                source_path: "C:/book.pdf".to_owned(),
                page: 12,
            }],
            StudyFormat::Csv,
        );
        assert!(vocab_csv.contains("\"'=cmd|"));
    }
}
