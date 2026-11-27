//! Full-screen reading and focus mode (plan R.19), the model: which part of the page focus mode keeps lit around the
//! reading line, and where the pointer brings back the chrome full screen hides.
//!
//! The reading line is the one the reading ruler (R.20) steps through with the arrow keys; focus mode follows it.

use ecclesia_mupdf::PageRect;
use ecclesia_text::TextLayout;

/// How close to the window's top edge, in pixels at 96 dpi, the pointer brings back the title bar in full screen.
pub const REVEAL_EDGE: f32 = 6.0;

/// What focus mode keeps lit: the setting `focus_scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// The paragraph (the text block) the reading line is in.
    #[default]
    Paragraph,
    /// The reading line alone.
    Line,
}

impl Scope {
    /// The scope a setting names (`"paragraph"`, `"line"`); `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "paragraph" => Some(Self::Paragraph),
            "line" => Some(Self::Line),
            _ => None,
        }
    }
}

/// The box focus mode keeps lit around line `line` of a page's `layout`, in page points: the line, or its block.
pub fn lit(layout: &TextLayout, line: usize, scope: Scope) -> Option<PageRect> {
    let text_line = layout.lines.get(line)?;
    match scope {
        Scope::Line => Some(text_line.bbox),
        Scope::Paragraph => layout.blocks.get(text_line.block).map(|block| block.bbox),
    }
}

/// Whether a pointer `y` device pixels from the window's top, at `scale` device pixels to a 96-dpi pixel, brings
/// back the title bar.
pub fn reveals(y: f32, scale: f32) -> bool {
    y <= REVEAL_EDGE * scale
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::{PagePoint, TextBlock, TextLine};

    use super::*;

    fn rect(y0: f32, y1: f32) -> PageRect {
        PageRect {
            x0: 0.0,
            y0,
            x1: 100.0,
            y1,
        }
    }

    #[test]
    fn focus_mode_lights_the_reading_lines_paragraph_or_the_line_alone() {
        let line = |y0, block| TextLine {
            bbox: rect(y0, y0 + 10.0),
            dir: PagePoint { x: 1.0, y: 0.0 },
            vertical: false,
            joined: false,
            block,
            chars: 0..0,
        };
        let layout = TextLayout {
            blocks: vec![
                TextBlock {
                    bbox: rect(0.0, 30.0),
                    lines: 0..2,
                },
                TextBlock {
                    bbox: rect(50.0, 60.0),
                    lines: 2..3,
                },
            ],
            lines: vec![line(0.0, 0), line(20.0, 0), line(50.0, 1)],
            chars: Vec::new(),
        };
        assert_eq!(lit(&layout, 1, Scope::Paragraph), Some(rect(0.0, 30.0)));
        assert_eq!(lit(&layout, 1, Scope::Line), Some(rect(20.0, 30.0)));
        assert_eq!(lit(&layout, 2, Scope::Paragraph), Some(rect(50.0, 60.0)));
        assert_eq!(lit(&layout, 9, Scope::Line), None);
    }

    #[test]
    fn the_title_bar_comes_back_at_the_top_edge() {
        assert!(reveals(0.0, 1.0));
        assert!(reveals(8.0, 1.5));
        assert!(!reveals(10.0, 1.5));
        assert_eq!(Scope::parse("line"), Some(Scope::Line));
        assert_eq!(Scope::parse("word"), None);
    }
}
