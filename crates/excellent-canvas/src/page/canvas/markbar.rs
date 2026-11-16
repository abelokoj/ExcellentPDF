//! The markup bar (plan A.1, A.2): once text is selected in a document that can be edited, a small bar floats under
//! the selection with the four text markups (highlight, underline, squiggly, strikeout), the two proposed edits
//! (replace and insert text), the note (A.4), then the current tool's colours (A.3). A tool button, or its command,
//! uses the tool in its own colour: a markup sends the selection to the app as `markup`, which makes it one undoable
//! edit; a note highlights it the same way, marked `note`, and the app opens the highlight's note; an edit sends
//! `textEdit`, and the app asks for the text. A plain click on the page is sent as `pagePress`, so the app can open
//! the note of the markup under it. A colour gives the current tool (the last one pressed) that
//! colour and uses it; the last cell asks the app for a custom colour. The models are `excellent-features`'
//! `annotate::markup`, `annotate::replace` and `annotate::colors`.

use std::ops::Range;

use excellent_core::annot::Markup;
use excellent_features::annotate::colors::{self, Colors, Tool as ColorTool, ToolStyle};
use excellent_mupdf::PagePoint;
use excellent_text::{FlatText, Granularity, PageText, Selection, TextPos};

use super::PageCanvas;
use super::refhover::luminance;
use crate::Rect;
use crate::page::host::{Host, Wash};

/// The colour cells: the palette's five swatches, then the custom colour.
const DOTS: usize = 6;
/// The custom colour's cell.
const CUSTOM: usize = DOTS - 1;

/// What the bar's buttons do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    /// Marks the selection.
    Mark(Markup),
    /// Proposes replacing the selection: a strikeout and a caret (A.2).
    Replace,
    /// Proposes inserting text at the click, or after the selection: a caret (A.2).
    Insert,
    /// Highlights the selection and opens the highlight's note (A.4).
    Note,
}

/// The bar's buttons, in its order: the markups, a rule, then the proposed edits and the note.
const TOOLS: [Tool; 7] = [
    Tool::Mark(Markup::Highlight),
    Tool::Mark(Markup::Underline),
    Tool::Mark(Markup::Squiggly),
    Tool::Mark(Markup::StrikeOut),
    Tool::Replace,
    Tool::Insert,
    Tool::Note,
];

/// How many of [`TOOLS`] are markups, before the rule.
const MARKUPS: usize = 4;

impl Tool {
    /// The tool whose colour it draws in.
    fn color_tool(self) -> ColorTool {
        match self {
            Self::Mark(Markup::Highlight) | Self::Note => ColorTool::Highlight,
            Self::Mark(Markup::Underline) => ColorTool::Underline,
            Self::Mark(Markup::Squiggly) => ColorTool::Squiggly,
            Self::Mark(Markup::StrikeOut) => ColorTool::StrikeOut,
            Self::Replace | Self::Insert => ColorTool::Proposal,
        }
    }
}

/// Sizes at 96 dpi, in pixels: a button's square, a colour's cell, the bar's padding, the gap under the selection.
const BUTTON: f32 = 32.0;
const DOT_CELL: f32 = 26.0;
const DOT: f32 = 7.0;
const PAD: f32 = 4.0;
const SEPARATOR: f32 = 9.0;
const GAP: f32 = 8.0;
const RADIUS: f32 = 8.0;

/// Characters of context kept either side of a markup's text, to find it again (K.28).
const CONTEXT: usize = 32;

/// What part of the bar is under a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    Tool(Tool),
    Color(usize),
    /// The bar, between its buttons.
    Frame,
}

/// The bar's state.
#[derive(Debug, Default)]
pub(super) struct MarkBar {
    /// Every tool's colour and opacity, as the app keeps them (A.3).
    colors: Colors,
    /// The tool the colours belong to, an index into [`TOOLS`]: the last one pressed.
    current: usize,
    /// Where it was last drawn, in client pixels; `None` while it isn't shown.
    frame: Option<Rect>,
    /// What the pointer is over.
    hover: Option<Hit>,
    /// A tool waiting for the text of selected pages that aren't read yet.
    pub(super) pending: Option<Tool>,
    /// Where the last plain click in the text put the caret, for inserting text there (A.2).
    pub(super) caret: Option<TextPos>,
}

impl<H: Host> PageCanvas<H> {
    /// Marks the selected text with `kind` in the bar's colour: the canvas sends it to the app as `markup`:
    /// `{kind, color: [r, g, b], pages: [{page, quads: [[ulx, uly, urx, ury, llx, lly, lrx, lry]], quote: {prefix,
    /// exact, suffix, start}}]}` in page points, colour channels 0 to 1. The selection then goes. Pages of the
    /// selection whose text isn't read yet are waited for.
    pub fn mark(&mut self, kind: Markup) {
        self.use_tool(Tool::Mark(kind));
    }

    /// Proposes replacing the selected text (A.2): the canvas sends `textEdit` with `mode: "replace"` (see
    /// [`Self::propose_insert`]) for the selection's last page, and the app asks for the new text.
    // shortcut: one page; a selection across pages proposes replacing its last page's part.
    pub fn propose_replace(&mut self) {
        self.use_tool(Tool::Replace);
    }

    /// Proposes inserting text (A.2) after the selection, or at the caret the last click in the text left: the
    /// canvas sends `textEdit`: `{mode: "insert" | "replace", color, page, quads, quote, foot: [x, y], selected}`,
    /// `quads` and `quote` the text struck out (empty for an insert), `foot` the bottom of the edge the text goes
    /// in at, all in page points; `selected` the text replaced. The app asks for the text.
    pub fn propose_insert(&mut self) {
        self.use_tool(Tool::Insert);
    }

    /// Highlights the selected text and asks for its note (A.4): the `markup` event (see [`Self::mark`]) carries
    /// `note: true`, and the app opens the new highlight's note.
    pub fn note_selection(&mut self) {
        self.use_tool(Tool::Note);
    }

    /// Uses `tool` on the selection, or an insert at the click's caret; waits for unread pages' text first.
    pub(super) fn use_tool(&mut self, tool: Tool) {
        if !self.can_mark() {
            return;
        }
        let selection = self.selection.filter(|s| !s.is_collapsed());
        let needed = match (selection, tool, self.markbar.caret) {
            (Some(selection), _, _) => selection.pages(),
            (None, Tool::Insert, Some(caret)) => caret.page..caret.page + 1,
            _ => return,
        };
        if needed.clone().any(|page| self.texts.get(page).is_none()) {
            self.markbar.pending = Some(tool);
            return;
        }
        self.markbar.pending = None;
        let event = match (tool, selection) {
            (Tool::Mark(kind), Some(selection)) => self.markup_event(kind, selection),
            (Tool::Note, Some(selection)) => {
                self.markup_event(Markup::Highlight, selection)
                    .map(|(name, mut data)| {
                        data["note"] = serde_json::Value::Bool(true);
                        (name, data)
                    })
            }
            (Tool::Replace, Some(selection)) => self.replace_event(selection),
            (Tool::Insert, selection) => self.insert_event(selection),
            _ => None,
        };
        let Some((name, data)) = event else {
            return;
        };
        self.emit(name, data);
        self.selection = None;
        self.markbar.caret = None;
        self.markbar.frame = None;
        self.host.selection_changed();
        self.invalidate();
    }

    fn markup_event(
        &self,
        kind: Markup,
        selection: Selection,
    ) -> Option<(&'static str, serde_json::Value)> {
        let flat = self.flat_text();
        let mut pages = Vec::new();
        for page in selection.pages() {
            let Some((text, range)) = self.selected_on(selection, page) else {
                continue;
            };
            pages.push(serde_json::json!({
                "page": page,
                "quads": quads(text, range.clone()),
                "quote": quote(text, &flat, page, range),
            }));
        }
        if pages.is_empty() {
            return None;
        }
        let style = self.tool_style(Tool::Mark(kind));
        let data = serde_json::json!({
            "note": false,
            "kind": kind_name(kind),
            "color": style.rgb(),
            "opacity": style.opacity,
            "pages": pages,
        });
        Some(("markup", data))
    }

    fn replace_event(&self, selection: Selection) -> Option<(&'static str, serde_json::Value)> {
        let (page, text, range) = selection.pages().rev().find_map(|page| {
            self.selected_on(selection, page)
                .map(|(text, range)| (page, text, range))
        })?;
        let foot = text.highlight(range.clone()).last()?.lr;
        let data = serde_json::json!({
            "mode": "replace",
            "color": self.tool_style(Tool::Replace).rgb(),
            "opacity": self.tool_style(Tool::Replace).opacity,
            "page": page,
            "quads": quads(text, range.clone()),
            "quote": quote(text, &self.flat_text(), page, range.clone()),
            "foot": [foot.x, foot.y],
            "selected": text.text(range),
        });
        Some(("textEdit", data))
    }

    fn insert_event(
        &self,
        selection: Option<Selection>,
    ) -> Option<(&'static str, serde_json::Value)> {
        let at = match selection {
            Some(selection) => selection.pages().rev().find_map(|page| {
                let (_, range) = self.selected_on(selection, page)?;
                Some(TextPos {
                    page,
                    index: range.end,
                })
            })?,
            None => self.markbar.caret?,
        };
        let foot = foot(self.texts.get(at.page)?, at.index)?;
        let data = serde_json::json!({
            "mode": "insert",
            "color": self.tool_style(Tool::Insert).rgb(),
            "opacity": self.tool_style(Tool::Insert).opacity,
            "page": at.page,
            "quads": [],
            "foot": [foot.x, foot.y],
            "selected": "",
        });
        Some(("textEdit", data))
    }

    /// The selection's text on `page` and its range, when it selects more than blanks there.
    fn selected_on(&self, selection: Selection, page: usize) -> Option<(&PageText, Range<usize>)> {
        let text = self.texts.get(page)?;
        let range = selection.range_on(page, text)?;
        (!text.text(range.clone()).trim().is_empty()).then_some((text, range))
    }

    /// `tool`'s colour and opacity.
    fn tool_style(&self, tool: Tool) -> ToolStyle {
        self.markbar.colors.style(tool.color_tool())
    }

    /// The tools' colours, as the app keeps them (the `annotationColors` prop, A.3).
    pub(crate) fn set_annotation_colors(&mut self, colors: Colors) {
        if self.markbar.colors != colors {
            self.markbar.colors = colors;
            self.invalidate();
        }
    }

    /// The current tool's colour cells: its palette's swatches, then its colour when that is a custom one.
    fn dots(&self) -> ([u32; CUSTOM], Option<u32>, ToolStyle) {
        let tool = TOOLS[self.markbar.current].color_tool();
        let swatches = self.markbar.colors.palette.swatches(tool);
        let style = self.markbar.colors.style(tool);
        let custom = (!swatches.contains(&style.color)).then_some(style.color);
        (swatches, custom, style)
    }

    /// A click in the text ended where it began, at `click`: a plain click keeps its caret for an insert.
    pub(super) fn markbar_click(&mut self, click: Selection) {
        self.markbar.caret = (click.granularity == Granularity::Char).then_some(click.focus);
    }

    /// A plain click at client pixel (`x`, `y`) that selected nothing and followed no link: in a document that can
    /// be edited, the app is told where it landed (`pagePress`: `{page, x, y}`, page points), to open the note of the
    /// markup there (A.4).
    pub(super) fn page_press(&self, x: i32, y: i32) {
        if !self.can_mark() {
            return;
        }
        if let Some((page, point)) = self.page_point(x, y) {
            self.emit(
                "pagePress",
                serde_json::json!({ "page": page, "x": point.x, "y": point.y }),
            );
        }
    }

    /// `tool`'s colour and opacity, as the app keeps them (A.3).
    pub(super) fn markbar_style(&self, tool: ColorTool) -> ToolStyle {
        self.markbar.colors.style(tool)
    }

    /// Whether the document shown can take markups: a PDF, which edits save into.
    pub(super) fn can_mark(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.is_editable())
    }

    /// Whether client pixel (`x`, `y`) is over the bar.
    pub(super) fn over_markbar(&self, x: i32, y: i32) -> bool {
        self.markbar_hit(x, y).is_some()
    }

    /// The pointer moved: the button under it is lit.
    pub(super) fn markbar_hover(&mut self, at: Option<(i32, i32)>) {
        let hover = at.and_then(|(x, y)| self.markbar_hit(x, y));
        if hover != self.markbar.hover {
            self.markbar.hover = hover;
            self.invalidate();
        }
    }

    /// The left button went down at client pixel (`x`, `y`): a tool is used in its colour and becomes the current
    /// one; a swatch gives the current tool its colour, tells the app (`toolColor`: `{tool, color: "#rrggbb"}`) and
    /// uses it; the custom cell asks the app for a colour (`pickColor`: `{tool}`). Returns whether the bar took the
    /// press.
    pub(super) fn markbar_down(&mut self, x: i32, y: i32) -> bool {
        match self.markbar_hit(x, y) {
            None => false,
            Some(Hit::Tool(tool)) => {
                if let Some(index) = TOOLS.iter().position(|&t| t == tool) {
                    self.markbar.current = index;
                }
                self.use_tool(tool);
                true
            }
            Some(Hit::Color(CUSTOM)) => {
                let tool = TOOLS[self.markbar.current].color_tool();
                self.emit("pickColor", serde_json::json!({ "tool": tool.name() }));
                true
            }
            Some(Hit::Color(index)) => {
                let tool = TOOLS[self.markbar.current];
                let (swatches, _, _) = self.dots();
                let color = swatches[index];
                self.markbar.colors.choose_color(tool.color_tool(), color);
                self.emit(
                    "toolColor",
                    serde_json::json!({ "tool": tool.color_tool().name(), "color": colors::hex(color) }),
                );
                self.use_tool(tool);
                true
            }
            Some(Hit::Frame) => true,
        }
    }

    fn markbar_hit(&self, x: i32, y: i32) -> Option<Hit> {
        let frame = self.markbar.frame?;
        let (x, y) = (x as f32, y as f32);
        if !(frame.x0 <= x && x < frame.x1 && frame.y0 <= y && y < frame.y1) {
            return None;
        }
        let cells = cells(frame, self.dpi_scale());
        let hit = cells
            .tools
            .iter()
            .zip(TOOLS)
            .find(|(cell, _)| cell.x0 <= x && x < cell.x1)
            .map(|(_, tool)| Hit::Tool(tool))
            .or_else(|| {
                cells
                    .colors
                    .iter()
                    .position(|cell| cell.x0 <= x && x < cell.x1)
                    .map(Hit::Color)
            });
        Some(hit.unwrap_or(Hit::Frame))
    }

    /// Draws the bar under the selection's last line, or over its first when there is no room below; it shows
    /// only while text is selected, not being dragged, in a document that can be edited.
    pub(super) fn draw_markbar(&mut self) -> Result<(), H::Error> {
        self.markbar.frame = None;
        let shown = self.selection.is_some_and(|s| !s.is_collapsed()) && !self.selecting;
        if !shown || !self.can_mark() {
            return Ok(());
        }
        let quads = self.selection_quads();
        let (Some(first), Some(last)) = (quads.first(), quads.last()) else {
            return Ok(());
        };
        let s = self.dpi_scale();
        let (width, height) = (WIDTH * s, (BUTTON + 2.0 * PAD) * s);
        let (view_w, view_h) = (self.viewport.width(), self.viewport.height());
        let below = last.iter().map(|p| p.1).fold(f32::MIN, f32::max) + GAP * s;
        let above = first.iter().map(|p| p.1).fold(f32::MAX, f32::min) - GAP * s - height;
        let top = if below + height <= view_h || above < 0.0 {
            below
        } else {
            above
        };
        if top >= view_h || top + height <= 0.0 {
            // The selection is off the screen.
            return Ok(());
        }
        let middle = last.iter().map(|p| p.0).sum::<f32>() / 4.0;
        let left = (middle - width / 2.0).clamp(PAD * s, (view_w - width - PAD * s).max(PAD * s));
        let frame = Rect::new(left.round(), top.round(), width.round(), height.round());
        self.markbar.frame = Some(frame);

        let dark = luminance(self.background) < 0.5;
        let (edge, shadow, ink, lit) = if dark {
            (0x5c_6068, 0x00_0000, 0xe6_e8ec, 0x3a_3e46)
        } else {
            (0xc4_c8cf, 0x1a_1d23, 0x3a_3f47, 0xe8_ebf0)
        };
        let paper = if dark { 0x26_282d } else { 0xff_ffff };
        let (swatches, custom, style) = self.dots();
        let current = TOOLS[self.markbar.current];
        let tool_colors = TOOLS.map(|tool| self.tool_style(tool).color);
        let hover = self.markbar.hover;
        let host = &mut self.host;
        let radius = RADIUS * s;
        host.band(
            &Rect::new(frame.x0 + s, frame.y0 + 3.0 * s, width, height),
            radius,
            shadow,
            Wash::Translucent,
        )?;
        host.band(&frame, radius, edge, Wash::Darkest)?;
        let hair = s.max(1.0).round();
        let inner = Rect::new(
            frame.x0 + hair,
            frame.y0 + hair,
            frame.width() - 2.0 * hair,
            frame.height() - 2.0 * hair,
        );
        host.band(&inner, radius - hair, paper, Wash::Darkest)?;
        let cells = cells(frame, s);
        for ((cell, tool), color) in cells.tools.iter().zip(TOOLS).zip(tool_colors) {
            if hover == Some(Hit::Tool(tool)) {
                host.band(&inset(cell, 2.0 * s), 6.0 * s, lit, Wash::Darkest)?;
            }
            glyph(host, tool, cell, s, ink, color)?;
            if tool == current {
                // The colours that follow are this tool's.
                let (cx, _) = center(cell);
                host.band(
                    &Rect::new(cx - 6.0 * s, cell.y1 - 3.0 * s, 12.0 * s, 2.0 * s),
                    s,
                    ink,
                    Wash::Darkest,
                )?;
            }
        }
        for rule in cells.separators {
            host.fill(&Rect::new(rule.0, rule.1, hair, rule.2), edge)?;
        }
        for (index, cell) in cells.colors.iter().enumerate() {
            let (cx, cy) = center(cell);
            let color = swatches.get(index).copied().or(custom);
            let chosen = color == Some(style.color);
            if chosen || hover == Some(Hit::Color(index)) {
                let ring = if chosen { ink } else { edge };
                host.band(
                    &circle(cx, cy, (DOT + 3.0) * s),
                    (DOT + 3.0) * s,
                    ring,
                    Wash::Darkest,
                )?;
                host.band(
                    &circle(cx, cy, (DOT + 1.5) * s),
                    (DOT + 1.5) * s,
                    paper,
                    Wash::Darkest,
                )?;
            }
            match color {
                Some(color) => {
                    host.band(&circle(cx, cy, DOT * s), DOT * s, color, Wash::Darkest)?
                }
                None => {
                    // No custom colour yet: a "+".
                    let arm = 5.0 * s;
                    host.fill(&Rect::new(cx - arm, cy - s, 2.0 * arm, 2.0 * s), ink)?;
                    host.fill(&Rect::new(cx - s, cy - arm, 2.0 * s, 2.0 * arm), ink)?;
                }
            }
        }
        Ok(())
    }
}

/// The bar's width at 96 dpi: its padding, the tools, two rules, the colours.
#[expect(
    clippy::cast_precision_loss,
    reason = "the counts are a handful of buttons"
)]
const WIDTH: f32 =
    2.0 * PAD + TOOLS.len() as f32 * BUTTON + 2.0 * SEPARATOR + DOTS as f32 * DOT_CELL;

/// The bar's parts in client pixels.
struct Cells {
    tools: [Rect; TOOLS.len()],
    /// The rules after the markups and after the edits: each one's left, top and height.
    separators: [(f32, f32, f32); 2],
    colors: [Rect; DOTS],
}

fn cells(frame: Rect, s: f32) -> Cells {
    let (top, size) = (frame.y0 + PAD * s, BUTTON * s);
    let rule = |x: f32| {
        (
            x + (SEPARATOR * s / 2.0).floor(),
            top + 6.0 * s,
            size - 12.0 * s,
        )
    };
    let mut x = frame.x0 + PAD * s;
    let mut separators = [(0.0, 0.0, 0.0); 2];
    let mut index = 0;
    let tools = [0; TOOLS.len()].map(|_| {
        if index == MARKUPS {
            separators[0] = rule(x);
            x += SEPARATOR * s;
        }
        index += 1;
        let cell = Rect::new(x, top, size, size);
        x += size;
        cell
    });
    separators[1] = rule(x);
    x += SEPARATOR * s;
    let colors = [0; DOTS].map(|_| {
        let cell = Rect::new(x, top, DOT_CELL * s, size);
        x += DOT_CELL * s;
        cell
    });
    Cells {
        tools,
        separators,
        colors,
    }
}

/// The lines of `range` on a page, one quad each, as the events send them: `[ulx, uly, urx, ury, llx, lly, lrx,
/// lry]` in page points.
fn quads(text: &PageText, range: Range<usize>) -> Vec<[f32; 8]> {
    text.highlight(range)
        .iter()
        .map(|q| {
            [
                q.ul.x, q.ul.y, q.ur.x, q.ur.y, q.ll.x, q.ll.y, q.lr.x, q.lr.y,
            ]
        })
        .collect()
}

/// The text of `range` on `page`, with some context either side and its place in the document (K.28).
fn quote(text: &PageText, flat: &FlatText, page: usize, range: Range<usize>) -> serde_json::Value {
    serde_json::json!({
        "prefix": text.text(range.start.saturating_sub(CONTEXT)..range.start),
        "exact": text.text(range.clone()),
        "suffix": text.text(range.end..range.end + CONTEXT),
        "start": flat.offset(TextPos { page, index: range.start }),
    })
}

/// The bottom of the edge before character `index` of `text` (after the last one, at the end): where inserted
/// text goes.
fn foot(text: &PageText, index: usize) -> Option<PagePoint> {
    let chars = &text.layout().chars;
    match chars.get(index) {
        Some(char) => Some(char.quad.ll),
        None => chars.last().map(|char| char.quad.lr),
    }
}

/// A markup's icon, drawn from the host's shapes: grey lines of "text" and the markup on them in `color`.
fn glyph<H: Host>(
    host: &mut H,
    tool: Tool,
    cell: &Rect,
    s: f32,
    ink: u32,
    color: u32,
) -> Result<(), H::Error> {
    let (cx, cy) = center(cell);
    let text = |dy: f32, w: f32| Rect::new(cx - 8.0 * s, cy + dy * s, w * s, 2.0 * s);
    // A caret, "^", in the colour, its point at `tip`.
    let caret = |host: &mut H, tip: f32| {
        let at = |dx: f32, dy: f32| (cx + dx * s, tip + dy * s, 1.8 * s);
        host.polyline(&[at(-4.5, 6.0), at(0.0, 0.0), at(4.5, 6.0)], color)
    };
    let kind = match tool {
        Tool::Mark(kind) => kind,
        Tool::Replace => {
            host.fill(&text(-6.0, 16.0), ink)?;
            host.fill(
                &Rect::new(cx - 10.0 * s, cy - 6.5 * s, 20.0 * s, 2.5 * s),
                color,
            )?;
            return caret(host, cy + s);
        }
        Tool::Insert => {
            let word = |x: f32| Rect::new(cx + x * s, cy - 4.0 * s, 7.0 * s, 2.0 * s);
            host.fill(&word(-9.0), ink)?;
            host.fill(&word(2.0), ink)?;
            return caret(host, cy - s);
        }
        Tool::Note => {
            // A speech bubble in the colour, two lines of text in it, its tail at the lower left.
            let bubble = Rect::new(cx - 9.0 * s, cy - 8.0 * s, 18.0 * s, 13.0 * s);
            host.band(&bubble, 3.0 * s, color, Wash::Darkest)?;
            let at = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s, 2.4 * s);
            host.polyline(&[at(-5.0, 4.0), at(-6.5, 9.0), at(-1.0, 4.0)], color)?;
            host.fill(
                &Rect::new(cx - 6.0 * s, cy - 4.5 * s, 12.0 * s, 2.0 * s),
                ink,
            )?;
            host.fill(
                &Rect::new(cx - 6.0 * s, cy - 0.5 * s, 8.0 * s, 2.0 * s),
                ink,
            )?;
            return Ok(());
        }
    };
    match kind {
        Markup::Highlight => {
            host.band(
                &Rect::new(cx - 10.0 * s, cy - 6.0 * s, 20.0 * s, 12.0 * s),
                2.0 * s,
                color,
                Wash::Darkest,
            )?;
            host.fill(&text(-1.0, 16.0), ink)?;
        }
        Markup::Underline => {
            host.fill(&text(-4.0, 16.0), ink)?;
            host.fill(
                &Rect::new(cx - 9.0 * s, cy + 3.0 * s, 18.0 * s, 2.5 * s),
                color,
            )?;
        }
        Markup::Squiggly => {
            host.fill(&text(-4.0, 16.0), ink)?;
            let points: Vec<(f32, f32, f32)> = (0..=6)
                .map(|step| {
                    let x = cx - 9.0 * s + step as f32 * 3.0 * s;
                    let y = cy + if step % 2 == 0 { 3.0 } else { 6.0 } * s;
                    (x, y, 1.8 * s)
                })
                .collect();
            host.polyline(&points, color)?;
        }
        Markup::StrikeOut => {
            host.fill(&text(-5.0, 16.0), ink)?;
            host.fill(&text(3.0, 12.0), ink)?;
            host.fill(
                &Rect::new(cx - 10.0 * s, cy - 1.5 * s, 20.0 * s, 2.5 * s),
                color,
            )?;
        }
    }
    Ok(())
}

/// The name a markup goes to the app by.
fn kind_name(kind: Markup) -> &'static str {
    match kind {
        Markup::Highlight => "highlight",
        Markup::Underline => "underline",
        Markup::Squiggly => "squiggly",
        Markup::StrikeOut => "strikeOut",
    }
}

fn center(r: &Rect) -> (f32, f32) {
    ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
}

fn circle(cx: f32, cy: f32, radius: f32) -> Rect {
    Rect::new(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius)
}

fn inset(r: &Rect, by: f32) -> Rect {
    Rect::new(
        r.x0 + by,
        r.y0 + by,
        r.width() - 2.0 * by,
        r.height() - 2.0 * by,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_s_cells_sit_side_by_side_inside_it() {
        let frame = Rect::new(100.0, 50.0, WIDTH, BUTTON + 2.0 * PAD);
        let cells = cells(frame, 1.0);
        assert!((cells.tools[0].x0 - (100.0 + PAD)).abs() < f32::EPSILON);
        assert!(cells.tools[MARKUPS - 1].x1 <= cells.separators[0].0);
        assert!(cells.separators[0].0 < cells.tools[MARKUPS].x0);
        assert!(cells.tools[TOOLS.len() - 1].x1 <= cells.separators[1].0);
        assert!(cells.separators[1].0 < cells.colors[0].x0);
        assert!((cells.colors[DOTS - 1].x1 - (frame.x1 - PAD)).abs() < 0.01);
    }
}
