//! Typed text blocks inside the canvas (plan N.6) with U.8 IME composition support.
//!
//! Supports paragraph, heading, list, checkbox, code, and table blocks. Each block computes
//! wrapped layout lines and caret geometry for in-canvas editing, integrates directly with
//! [`ImeBridge`] for CJK input, converts to a standard PDF FreeText annotation (readable in
//! Acrobat and every other PDF viewer, with XHTML rich contents `/RC`), and round-trips its
//! full structured model through JSON sidecars.

use ecclesia_core::anchor::new_id;
use ecclesia_core::annot::{Annotation, Boxed, Font, Shape, Style, TextStyle};
use ecclesia_mupdf::PageRect;
use serde_json::{Map, Value, json};

use crate::notebook::ime::{ImeAction, ImeBridge, ImeEvent};

/// Bullet or numbering style for a list block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ListStyle {
    /// Unordered bullet list (`•`).
    #[default]
    Bulleted,
    /// Ordered numbered list (`1.`, `2.`, ...).
    Numbered,
}

impl ListStyle {
    /// Serialization identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bulleted => "bulleted",
            Self::Numbered => "numbered",
        }
    }

    /// Parses a list style identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "bulleted" => Some(Self::Bulleted),
            "numbered" => Some(Self::Numbered),
            _ => None,
        }
    }
}

/// Structural kind of a typed text block (plan N.6 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// Standard prose paragraph.
    Paragraph,
    /// Section heading (level 1..=6).
    Heading {
        /// Heading level clamped to `1..=6`.
        level: u8,
    },
    /// Multi-item bulleted or numbered list.
    List {
        /// List marker style.
        style: ListStyle,
        /// Individual list item strings.
        items: Vec<String>,
    },
    /// Interactive checklist item.
    Checkbox {
        /// Whether the checkbox is checked.
        checked: bool,
    },
    /// Monospace code block with optional language tag.
    Code {
        /// Optional syntax language label (e.g., `"rust"`).
        language: Option<String>,
    },
    /// Structured table with header cells and data rows.
    Table {
        /// Column header labels.
        headers: Vec<String>,
        /// Data rows (each row is a list of cell strings).
        rows: Vec<Vec<String>>,
    },
}

/// Caret and selection state inside a [`TextBlock`], in UTF-8 byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextCursor {
    /// Selection anchor byte offset.
    pub anchor: usize,
    /// Active caret byte offset.
    pub caret: usize,
}

impl TextCursor {
    /// Collapsed caret at `offset`.
    #[must_use]
    pub fn collapsed(offset: usize) -> Self {
        Self {
            anchor: offset,
            caret: offset,
        }
    }

    /// Ordered byte range `(start, end)` of the current selection.
    #[must_use]
    pub fn range(self) -> (usize, usize) {
        (self.anchor.min(self.caret), self.anchor.max(self.caret))
    }

    /// Whether a non-empty selection exists.
    #[must_use]
    pub fn has_selection(self) -> bool {
        self.anchor != self.caret
    }
}

/// A laid-out visual line produced by [`TextBlock::layout_lines`].
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    /// Visual text shown on this line.
    pub text: String,
    /// Baseline Y offset relative to the block's top-left corner, in points.
    pub y_offset: f32,
    /// Line height in points.
    pub line_height: f32,
}

/// A typed text block placed on a notebook page (plan N.6).
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    /// Stable block UUID (also used as the linked FreeText annotation `/NM`).
    pub id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Bounding rectangle in top-left page points.
    pub rect: PageRect,
    /// Block structural kind.
    pub kind: BlockKind,
    /// Primary editable text content (for Paragraph, Heading, Checkbox, Code).
    pub text: String,
    /// Visual font, size, alignment, and color.
    pub style: TextStyle,
    /// Current editing caret / selection.
    pub cursor: TextCursor,
}

impl TextBlock {
    /// Creates a new text block on `page` at `rect`.
    #[must_use]
    pub fn new(page: usize, rect: PageRect, kind: BlockKind, text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.len();
        let mut style = TextStyle::default();
        match &kind {
            BlockKind::Heading { level } => {
                style.size = match level {
                    1 => 22.0,
                    2 => 18.0,
                    3 => 15.0,
                    _ => 13.0,
                };
            }
            BlockKind::Code { .. } => {
                style.font = Font::Mono;
                style.size = 10.5;
            }
            _ => {}
        }
        Self {
            id: new_id(),
            page,
            rect,
            kind,
            text,
            style,
            cursor: TextCursor::collapsed(len),
        }
    }

    /// Inserts `inserted` at the current caret (replacing any active selection).
    pub fn insert_text(&mut self, inserted: &str) {
        let (start, end) = self.clamped_range();
        self.text.replace_range(start..end, inserted);
        let next = start + inserted.len();
        self.cursor = TextCursor::collapsed(next);
    }

    /// Deletes the active selection, or one Unicode scalar value before the caret if collapsed.
    pub fn backspace(&mut self) {
        let (start, end) = self.clamped_range();
        if start < end {
            self.text.replace_range(start..end, "");
            self.cursor = TextCursor::collapsed(start);
        } else if start > 0
            && let Some((prev_idx, _)) = self.text[..start].char_indices().next_back()
        {
            self.text.replace_range(prev_idx..start, "");
            self.cursor = TextCursor::collapsed(prev_idx);
        }
    }

    /// Toggles the checked state when this block is a [`BlockKind::Checkbox`].
    pub fn toggle_checkbox(&mut self) {
        if let BlockKind::Checkbox { ref mut checked } = self.kind {
            *checked = !*checked;
        }
    }

    /// Applies an [`ImeEvent`] routed through `bridge`, updating this block when text commits and
    /// keeping the candidate window anchored to the current caret rectangle (U.8 + N.6).
    pub fn apply_ime(&mut self, bridge: &mut ImeBridge, event: ImeEvent) -> ImeAction {
        if bridge.focused_target() != Some(self.id.as_str()) {
            bridge.focus(self.id.clone(), self.caret_page_rect(None));
        }
        let action = bridge.handle_event(event);
        if let ImeAction::CommitText { ref text, .. } = action {
            self.insert_text(text);
        }
        let preedit = bridge.composition().map(|c| c.preedit.as_str());
        bridge.update_caret_rect(self.caret_page_rect(preedit));
        action
    }

    /// Returns the display text including any uncommitted IME preedit spliced at the caret.
    #[must_use]
    pub fn display_text_with_preedit(&self, preedit: Option<&str>) -> String {
        let plain = self.plain_text();
        let Some(pre) = preedit else {
            return plain;
        };
        if pre.is_empty() {
            return plain;
        }
        let (start, end) = self.clamped_range();
        if matches!(
            self.kind,
            BlockKind::Paragraph | BlockKind::Heading { .. } | BlockKind::Code { .. }
        ) {
            let mut out = String::with_capacity(self.text.len() + pre.len());
            out.push_str(&self.text[..start]);
            out.push_str(pre);
            out.push_str(&self.text[end..]);
            out
        } else {
            format!("{plain}{pre}")
        }
    }

    /// Renders plain readable text for other PDF readers (FreeText `/Contents`).
    #[must_use]
    pub fn plain_text(&self) -> String {
        match &self.kind {
            BlockKind::Paragraph | BlockKind::Heading { .. } | BlockKind::Code { .. } => {
                self.text.clone()
            }
            BlockKind::Checkbox { checked } => {
                let mark = if *checked { "[x]" } else { "[ ]" };
                format!("{mark} {}", self.text)
            }
            BlockKind::List { style, items } => items
                .iter()
                .enumerate()
                .map(|(i, item)| match style {
                    ListStyle::Bulleted => format!("• {item}"),
                    ListStyle::Numbered => format!("{}. {item}", i + 1),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            BlockKind::Table { headers, rows } => {
                let mut lines = Vec::new();
                if !headers.is_empty() {
                    lines.push(headers.join(" | "));
                }
                for row in rows {
                    lines.push(row.join(" | "));
                }
                lines.join("\n")
            }
        }
    }

    /// Renders XHTML rich contents suitable for PDF FreeText `/RC` (`pdf_set_annot_rich_contents`).
    #[must_use]
    pub fn rich_contents_xhtml(&self) -> String {
        let body = match &self.kind {
            BlockKind::Paragraph => format!("<p>{}</p>", escape_xml(&self.text)),
            BlockKind::Heading { level } => {
                let lvl = (*level).clamp(1, 6);
                format!(
                    "<p style=\"font-weight:bold;font-size:{}pt\">{}</p>",
                    self.style.size,
                    escape_xml(&self.text)
                )
                .replace("<p ", &format!("<p data-heading=\"{lvl}\" "))
            }
            BlockKind::Checkbox { checked } => {
                let sym = if *checked { "&#x2611;" } else { "&#x2610;" };
                format!("<p>{sym} {}</p>", escape_xml(&self.text))
            }
            BlockKind::List { style, items } => {
                let mut s = String::new();
                for (i, item) in items.iter().enumerate() {
                    let prefix = match style {
                        ListStyle::Bulleted => "• ".to_owned(),
                        ListStyle::Numbered => format!("{}. ", i + 1),
                    };
                    s.push_str("<p>");
                    s.push_str(&prefix);
                    s.push_str(&escape_xml(item));
                    s.push_str("</p>");
                }
                s
            }
            BlockKind::Code { .. } => format!(
                "<p style=\"font-family:Courier,monospace\">{}</p>",
                escape_xml(&self.text)
            ),
            BlockKind::Table { headers, rows } => {
                let mut s = String::new();
                if !headers.is_empty() {
                    s.push_str("<p style=\"font-weight:bold\">");
                    s.push_str(&escape_xml(&headers.join(" | ")));
                    s.push_str("</p>");
                }
                for row in rows {
                    s.push_str("<p>");
                    s.push_str(&escape_xml(&row.join(" | ")));
                    s.push_str("</p>");
                }
                s
            }
        };
        format!("<?xml version=\"1.0\"?><body xmlns=\"http://www.w3.org/1999/xhtml\">{body}</body>")
    }

    /// Converts this text block into a standard PDF FreeText [`Annotation`] so every PDF reader
    /// displays its readable text at the exact location (plan N.6 step 3).
    #[must_use]
    pub fn to_freetext_annotation(&self, author: &str) -> Annotation {
        let mut annot = Annotation::new(
            self.page,
            Shape::Boxed(Boxed::FreeText, self.rect),
            Style {
                color: None,
                opacity: 1.0,
                width: 0.0,
            },
            author,
        );
        annot.id = self.id.clone();
        annot.contents = self.plain_text();
        annot.text = self.style;
        annot
    }

    /// Computes wrapped visual lines for in-canvas rendering.
    #[must_use]
    pub fn layout_lines(&self, preedit: Option<&str>) -> Vec<LayoutLine> {
        let content = self.display_text_with_preedit(preedit);
        let width = (self.rect.x1 - self.rect.x0).max(self.style.size * 2.0);
        let line_height = self.style.size * 1.3;
        let char_w = match self.style.font {
            Font::Mono => self.style.size * 0.6,
            Font::Serif => self.style.size * 0.55,
            Font::Sans => self.style.size * 0.58,
        };
        let max_cols = ((width / char_w.max(1.0)).floor() as usize).max(1);

        let mut lines = Vec::new();
        let mut y = 0.0_f32;
        for raw_line in content.split('\n') {
            if raw_line.is_empty() {
                lines.push(LayoutLine {
                    text: String::new(),
                    y_offset: y,
                    line_height,
                });
                y += line_height;
                continue;
            }
            let mut cur = String::new();
            let mut cols = 0usize;
            for ch in raw_line.chars() {
                let ch_cols = if is_cjk_wide(ch) { 2 } else { 1 };
                if cols + ch_cols > max_cols && !cur.is_empty() {
                    lines.push(LayoutLine {
                        text: std::mem::take(&mut cur),
                        y_offset: y,
                        line_height,
                    });
                    y += line_height;
                    cols = 0;
                }
                cur.push(ch);
                cols += ch_cols;
            }
            lines.push(LayoutLine {
                text: cur,
                y_offset: y,
                line_height,
            });
            y += line_height;
        }
        lines
    }

    /// Computes the caret rectangle in page points for IME candidate window placement (U.8).
    #[must_use]
    pub fn caret_page_rect(&self, preedit: Option<&str>) -> PageRect {
        let (start, _) = self.clamped_range();
        let prefix = format!("{}{}", &self.text[..start], preedit.unwrap_or(""));
        let line_height = self.style.size * 1.3;
        let line_idx = prefix.matches('\n').count();
        let last_line = prefix.rsplit('\n').next().unwrap_or("");
        let advance: f32 = last_line
            .chars()
            .map(|c| {
                if is_cjk_wide(c) {
                    self.style.size
                } else {
                    self.style.size * 0.58
                }
            })
            .sum();
        let x = (self.rect.x0 + advance).min(self.rect.x1);
        let y = self.rect.y0 + line_idx as f32 * line_height;
        PageRect {
            x0: x,
            y0: y,
            x1: x + 1.5,
            y1: y + line_height,
        }
    }

    /// Serializes full block structure for the notebook sidecar (plan N.6 step 3).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let kind_val = match &self.kind {
            BlockKind::Paragraph => json!({"type": "paragraph"}),
            BlockKind::Heading { level } => json!({"type": "heading", "level": level}),
            BlockKind::List { style, items } => json!({
                "type": "list",
                "style": style.as_str(),
                "items": items,
            }),
            BlockKind::Checkbox { checked } => json!({"type": "checkbox", "checked": checked}),
            BlockKind::Code { language } => json!({"type": "code", "language": language}),
            BlockKind::Table { headers, rows } => json!({
                "type": "table",
                "headers": headers,
                "rows": rows,
            }),
        };
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("page".into(), json!(self.page));
        map.insert(
            "rect".into(),
            json!([self.rect.x0, self.rect.y0, self.rect.x1, self.rect.y1]),
        );
        map.insert("kind".into(), kind_val);
        map.insert("text".into(), json!(self.text));
        map.insert("font_size".into(), json!(self.style.size));
        map.insert("color".into(), json!(self.style.color));
        Value::Object(map)
    }

    /// Deserializes full block structure from the notebook sidecar.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let id = obj.get("id")?.as_str()?.to_owned();
        let page = usize::try_from(obj.get("page")?.as_u64()?).ok()?;
        let r = obj.get("rect")?.as_array()?;
        if r.len() != 4 {
            return None;
        }
        let rect = PageRect {
            x0: r[0].as_f64()? as f32,
            y0: r[1].as_f64()? as f32,
            x1: r[2].as_f64()? as f32,
            y1: r[3].as_f64()? as f32,
        };
        let k = obj.get("kind")?.as_object()?;
        let kind = match k.get("type")?.as_str()? {
            "paragraph" => BlockKind::Paragraph,
            "heading" => BlockKind::Heading {
                level: u8::try_from(k.get("level")?.as_u64()?).ok()?,
            },
            "list" => BlockKind::List {
                style: ListStyle::parse(k.get("style")?.as_str()?)?,
                items: k
                    .get("items")?
                    .as_array()?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            },
            "checkbox" => BlockKind::Checkbox {
                checked: k.get("checked")?.as_bool()?,
            },
            "code" => BlockKind::Code {
                language: k.get("language").and_then(Value::as_str).map(str::to_owned),
            },
            "table" => BlockKind::Table {
                headers: k
                    .get("headers")?
                    .as_array()?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
                rows: k
                    .get("rows")?
                    .as_array()?
                    .iter()
                    .map(|row| {
                        row.as_array()?
                            .iter()
                            .map(|cell| cell.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()
                    })
                    .collect::<Option<Vec<_>>>()?,
            },
            _ => return None,
        };
        let text = obj.get("text")?.as_str()?.to_owned();
        let len = text.len();
        let mut style = TextStyle::default();
        if let Some(sz) = obj.get("font_size").and_then(Value::as_f64) {
            style.size = sz as f32;
        }
        if let Some(arr) = obj.get("color").and_then(Value::as_array)
            && arr.len() == 3
        {
            style.color = [
                arr[0].as_f64()? as f32,
                arr[1].as_f64()? as f32,
                arr[2].as_f64()? as f32,
            ];
        }
        Some(Self {
            id,
            page,
            rect,
            kind,
            text,
            style,
            cursor: TextCursor::collapsed(len),
        })
    }

    fn clamped_range(&self) -> (usize, usize) {
        let (a, b) = self.cursor.range();
        (clamp_boundary(&self.text, a), clamp_boundary(&self.text, b))
    }
}

fn clamp_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn is_cjk_wide(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1100..=0x115f
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe30..=0xfe4f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
    )
}

fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::notebook::pdf_ops::build_blank_pdf;
    use ecclesia_mupdf::Document;

    const BOX: PageRect = PageRect {
        x0: 72.0,
        y0: 100.0,
        x1: 372.0,
        y1: 220.0,
    };

    #[test]
    fn all_six_block_kinds_round_trip_sidecar_and_save_readable_freetext_in_pdf() {
        let blocks = vec![
            TextBlock::new(0, BOX, BlockKind::Paragraph, "Key idea for lecture."),
            TextBlock::new(0, BOX, BlockKind::Heading { level: 2 }, "Thermodynamics"),
            TextBlock::new(
                0,
                BOX,
                BlockKind::List {
                    style: ListStyle::Numbered,
                    items: vec!["First law".into(), "Second law".into()],
                },
                "",
            ),
            TextBlock::new(
                0,
                BOX,
                BlockKind::Checkbox { checked: true },
                "Review notes",
            ),
            TextBlock::new(
                0,
                BOX,
                BlockKind::Code {
                    language: Some("rust".into()),
                },
                "fn main() {}",
            ),
            TextBlock::new(
                0,
                BOX,
                BlockKind::Table {
                    headers: vec!["Symbol".into(), "Unit".into()],
                    rows: vec![vec!["J".into(), "Joule".into()]],
                },
                "",
            ),
        ];

        // Sidecar JSON round-trip preserves full structure.
        for blk in &blocks {
            let json = blk.to_json();
            let parsed = TextBlock::from_json(&json).expect("parse block");
            assert_eq!(parsed.kind, blk.kind);
            assert_eq!(parsed.text, blk.text);
            assert!(
                blk.rich_contents_xhtml()
                    .contains("http://www.w3.org/1999/xhtml")
            );
        }

        // Verify FreeText annotation saves readable text into a real PDF opened by MuPDF.
        let pdf = build_blank_pdf(&[(612.0, 792.0)]).expect("blank");
        let mut doc = Document::from_bytes(&pdf, "application/pdf").expect("mupdf");
        for blk in &blocks {
            let annot = blk.to_freetext_annotation("Ada");
            doc.operation("Add text block", |edit| {
                edit.create_annot(0, &annot.to_data())
            })
            .expect("create freetext");
        }
        let page0 = doc.load_page(0).expect("p0");
        let annots = page0.annots().expect("annots");
        assert_eq!(annots.len(), 6);
        let texts: Vec<String> = annots
            .iter()
            .map(|a| a.contents().expect("contents"))
            .collect();
        assert!(texts.iter().any(|t| t.contains("Thermodynamics")));
        assert!(texts.iter().any(|t| t.contains("[x] Review notes")));
        assert!(texts.iter().any(|t| t.contains("Symbol | Unit")));
    }

    #[test]
    fn text_block_composes_cjk_via_ime_bridge_and_updates_caret_rect() {
        let mut blk = TextBlock::new(0, BOX, BlockKind::Paragraph, "Note: ");
        let mut ime = ImeBridge::new();

        blk.apply_ime(&mut ime, ImeEvent::Start);
        let caret_before = ime.candidate_window_rect().expect("caret");
        blk.apply_ime(
            &mut ime,
            ImeEvent::Preedit {
                text: "にほんご".into(),
                cursor_bytes: 12,
                clauses: vec![],
            },
        );
        let caret_during = ime.candidate_window_rect().expect("caret during");
        assert!(
            caret_during.x0 > caret_before.x0,
            "candidate window follows CJK preedit caret"
        );
        assert!(
            blk.display_text_with_preedit(ime.composition().map(|c| c.preedit.as_str()))
                .contains("Note: にほんご")
        );

        blk.apply_ime(&mut ime, ImeEvent::Commit("日本語".into()));
        assert_eq!(blk.text, "Note: 日本語");
        assert!(!ime.is_composing());
    }
}
