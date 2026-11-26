//! Temporary whiteboard pages during presentations (plan G.6).
//!
//! Inserts a volatile blank or grid whiteboard overlay during a presentation without mutating
//! the original slide PDF. After presenting, the whiteboard pages can be discarded or exported
//! into a standalone `.expdf` notebook.

use excellent_core::notebook::{Notebook, Orientation, PageSize, PaperTemplateId, Result};

use crate::notebook::infinite::{CanvasStroke, InfiniteCanvas, InfiniteExportMode};

/// Background pattern for a temporary presentation whiteboard (G.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WhiteboardBackground {
    /// Clean blank white board.
    #[default]
    Blank,
    /// Square grid background.
    Grid,
    /// Dot grid background.
    Dot,
    /// Lined background.
    Lined,
}

impl WhiteboardBackground {
    /// Matching procedural paper template identifier (N.1).
    #[must_use]
    pub fn template_id(self) -> PaperTemplateId {
        match self {
            Self::Blank => PaperTemplateId::Blank,
            Self::Grid => PaperTemplateId::Grid,
            Self::Dot => PaperTemplateId::Dot,
            Self::Lined => PaperTemplateId::Lined,
        }
    }
}

/// One volatile whiteboard page inserted temporarily during a talk.
#[derive(Debug, Clone, PartialEq)]
pub struct WhiteboardPage {
    /// Slide index the presenter was on when opening this whiteboard.
    pub inserted_after_slide: usize,
    /// Background pattern (blank, grid, dot, lined).
    pub background: WhiteboardBackground,
    /// Unbounded infinite canvas holding all strokes drawn on this whiteboard (N.5).
    pub canvas: InfiniteCanvas,
}

/// Manages temporary whiteboard pages during a presentation without modifying the slide file (G.6).
#[derive(Debug, Clone, Default)]
pub struct WhiteboardDeck {
    pages: Vec<WhiteboardPage>,
    active_index: Option<usize>,
}

impl WhiteboardDeck {
    /// Creates an empty temporary whiteboard deck.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a fresh temporary whiteboard page after `current_slide` and activates it.
    pub fn insert_and_activate(
        &mut self,
        current_slide: usize,
        background: WhiteboardBackground,
    ) -> usize {
        let idx = self.pages.len();
        self.pages.push(WhiteboardPage {
            inserted_after_slide: current_slide,
            background,
            canvas: InfiniteCanvas::new(),
        });
        self.active_index = Some(idx);
        idx
    }

    /// Returns to the slide view while keeping temporary whiteboard pages in memory for the talk.
    pub fn return_to_slides(&mut self) {
        self.active_index = None;
    }

    /// Whether a whiteboard page is currently displayed in place of the slide.
    #[must_use]
    pub fn is_showing_whiteboard(&self) -> bool {
        self.active_index.is_some()
    }

    /// Mutable access to the currently displayed whiteboard page, if active.
    pub fn active_page_mut(&mut self) -> Option<&mut WhiteboardPage> {
        let idx = self.active_index?;
        self.pages.get_mut(idx)
    }

    /// Adds a stroke to the currently active whiteboard page.
    pub fn push_stroke(&mut self, stroke: CanvasStroke) -> bool {
        if let Some(page) = self.active_page_mut() {
            page.canvas.push_stroke(stroke);
            true
        } else {
            false
        }
    }

    /// All temporary whiteboard pages created during this presentation.
    #[must_use]
    pub fn pages(&self) -> &[WhiteboardPage] {
        &self.pages
    }

    /// Exports all non-empty temporary whiteboard pages into a brand new `.expdf` [`Notebook`]
    /// without touching the original presentation PDF (plan G.6).
    ///
    /// # Errors
    ///
    /// Returns an error if notebook or PDF generation fails.
    pub fn export_to_notebook(&self, title: impl Into<String>) -> Result<Option<Notebook>> {
        let Some(first) = self.pages.first() else {
            return Ok(None);
        };
        let pdf = first
            .canvas
            .export_pdf(InfiniteExportMode::SinglePage { padding_pt: 24.0 })?;
        let mut nb = Notebook::from_pdf_bytes(title, pdf, first.background.template_id())?;
        for extra in &self.pages[1..] {
            let (w, h) = PageSize::Letter.dimensions(Orientation::Landscape);
            let next_pdf = excellent_core::notebook::pdf_ops::insert_page(
                nb.pdf_bytes(),
                nb.metadata.pages.len(),
                w,
                h,
                "",
            )?;
            nb.replace_pdf_bytes(next_pdf)?;
            if let Some(meta) = nb.metadata.pages.last_mut() {
                meta.template = extra.background.template_id();
            }
        }
        Ok(Some(nb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::infinite::CanvasPoint;
    use excellent_core::notebook::pdf_ops::build_blank_pdf;

    #[test]
    fn temporary_whiteboard_leaves_original_file_unchanged_and_can_save_to_notebook() {
        let original_slides = build_blank_pdf(&[(720.0, 405.0), (720.0, 405.0)]).expect("slides");
        let original_copy = original_slides.clone();

        let mut deck = WhiteboardDeck::new();
        assert!(!deck.is_showing_whiteboard());

        deck.insert_and_activate(0, WhiteboardBackground::Grid);
        assert!(deck.is_showing_whiteboard());
        assert!(deck.push_stroke(CanvasStroke {
            id: "wb-1".into(),
            color: [0.1, 0.2, 0.7],
            opacity: 1.0,
            points: vec![
                CanvasPoint {
                    x: 10.0,
                    y: 20.0,
                    width: 3.0,
                },
                CanvasPoint {
                    x: 200.0,
                    y: 160.0,
                    width: 3.0,
                },
            ],
        }));

        deck.return_to_slides();
        assert!(!deck.is_showing_whiteboard());
        assert_eq!(
            original_slides, original_copy,
            "original slide PDF untouched"
        );

        let saved_nb = deck
            .export_to_notebook("Lecture Whiteboard")
            .expect("export")
            .expect("some notebook");
        assert_eq!(saved_nb.metadata.pages.len(), 1);
        assert_eq!(saved_nb.metadata.pages[0].template, PaperTemplateId::Grid);
    }
}
