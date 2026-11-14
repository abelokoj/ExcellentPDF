//! Structured text.

use core::ops::Range;
use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    ex_drop_stext_page, ex_stext_copy, ex_stext_counts, ex_stext_page_text, ex_text_block,
    ex_text_char, ex_text_line, fz_point, fz_quad, fz_stext_page,
};

use crate::buffer;
use crate::context::{check, drop_with_context, with_context};
use crate::error::Result;
use crate::geometry::{PagePoint, PageRect, Quad};

/// The extraction options the text layer reads pages with (plan K.13): `FZ_STEXT_DEHYPHENATE` (hyphenated words
/// joined across lines), `FZ_STEXT_SEGMENT` (the page segmented into blocks) and `FZ_STEXT_COLLECT_STRUCTURE` (the
/// PDF's structure kept, for reading order). Not `FZ_STEXT_ACCURATE_BBOXES`: its glyph-tight boxes make a selection
/// ragged; the font's ascent and descent give every character on a line the same height.
pub(crate) const READING_FLAGS: core::ffi::c_int = 16 | 4096 | 256;

/// The text of a page, with its layout (blocks, lines, characters). Not `Send`.
#[derive(Debug)]
pub struct StextPage {
    raw: NonNull<fz_stext_page>,
}

/// A page's text as plain data: its text blocks in MuPDF's order, their lines and their characters. `Send`, so the
/// workers can hand it to the UI thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextLayout {
    /// The text blocks (paragraphs, roughly).
    pub blocks: Vec<TextBlock>,
    /// Every block's lines, block after block.
    pub lines: Vec<TextLine>,
    /// Every line's characters, line after line.
    pub chars: Vec<TextChar>,
}

/// A block of text lines.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    /// Its box, in page points.
    pub bbox: PageRect,
    /// Its lines, as indexes into [`TextLayout::lines`].
    pub lines: Range<usize>,
}

/// A line of characters sharing a baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Its box, in page points.
    pub bbox: PageRect,
    /// The baseline's direction, normalized: `(1, 0)` for ordinary horizontal text.
    pub dir: PagePoint,
    /// Whether it is written vertically.
    pub vertical: bool,
    /// Whether a hyphenated word continues on the next line: its trailing hyphen and the line break aren't part of
    /// the text.
    pub joined: bool,
    /// Its block, as an index into [`TextLayout::blocks`].
    pub block: usize,
    /// Its characters, as indexes into [`TextLayout::chars`].
    pub chars: Range<usize>,
}

/// One character and where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextChar {
    /// The character; U+FFFD when the file maps it to nothing valid.
    pub c: char,
    /// Its area on the page, which may be rotated.
    pub quad: Quad,
    /// Its font size, in points.
    pub size: f32,
}

fn point(p: fz_point) -> PagePoint {
    PagePoint { x: p.x, y: p.y }
}

fn quad(q: fz_quad) -> Quad {
    Quad {
        ul: point(q.ul),
        ur: point(q.ur),
        ll: point(q.ll),
        lr: point(q.lr),
    }
}

/// A count or index from the shim, which is never negative.
fn index(n: core::ffi::c_int) -> usize {
    usize::try_from(n).unwrap_or(0)
}

fn span(first: core::ffi::c_int, len: core::ffi::c_int) -> Range<usize> {
    index(first)..index(first) + index(len)
}

impl StextPage {
    pub(crate) fn new(raw: NonNull<fz_stext_page>) -> Self {
        Self { raw }
    }

    /// The page's text in reading order, one line per line and a blank line between blocks.
    ///
    /// # Errors
    ///
    /// MuPDF runs out of memory.
    pub fn text(&self) -> Result<String> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the text page is live and on its own thread.
            check(|err| unsafe { ex_stext_page_text(ctx, self.raw.as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { buffer::into_string(ctx, buf) })
        })
    }

    /// The page's text as plain data, for the text layer (K.13).
    pub fn layout(&self) -> TextLayout {
        let (mut blocks, mut lines, mut chars) = (0, 0, 0);
        // SAFETY: the text page is live and on this thread; the walk only reads it.
        unsafe { ex_stext_counts(self.raw.as_ptr(), &mut blocks, &mut lines, &mut chars) };
        let mut raw_blocks = vec![ex_text_block::default(); index(blocks)];
        let mut raw_lines = vec![ex_text_line::default(); index(lines)];
        let mut raw_chars = vec![ex_text_char::default(); index(chars)];
        // SAFETY: the arrays have the sizes the count just gave for the same page, which nothing changed since.
        unsafe {
            ex_stext_copy(
                self.raw.as_ptr(),
                raw_blocks.as_mut_ptr(),
                raw_lines.as_mut_ptr(),
                raw_chars.as_mut_ptr(),
            );
        }
        TextLayout {
            blocks: raw_blocks
                .iter()
                .map(|b| TextBlock {
                    bbox: b.bbox.into(),
                    lines: span(b.first_line, b.line_count),
                })
                .collect(),
            lines: raw_lines
                .iter()
                .map(|l| TextLine {
                    bbox: l.bbox.into(),
                    dir: point(l.dir),
                    vertical: l.wmode != 0,
                    joined: l.joined != 0,
                    block: index(l.block),
                    chars: span(l.first_char, l.char_count),
                })
                .collect(),
            chars: raw_chars
                .iter()
                .map(|c| TextChar {
                    c: u32::try_from(c.c)
                        .ok()
                        .and_then(char::from_u32)
                        .unwrap_or(char::REPLACEMENT_CHARACTER),
                    quad: quad(c.quad),
                    size: c.size,
                })
                .collect(),
        }
    }
}

impl Drop for StextPage {
    fn drop(&mut self) {
        // SAFETY: we own the text page.
        drop_with_context(|ctx| unsafe { ex_drop_stext_page(ctx, self.raw.as_ptr()) });
    }
}
