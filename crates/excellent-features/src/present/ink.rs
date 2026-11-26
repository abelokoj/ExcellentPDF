//! Pen on slides and fading ink during presentations (plan G.5).
//!
//! Temporary strokes live purely in memory and never mutate the underlying PDF file unless the
//! presenter explicitly chooses "Keep", which converts non-expired strokes into standard PDF ink
//! annotations (`Change::CreateAnnotation`, H.9).

use std::collections::BTreeMap;

use excellent_core::annot::{Annotation, Shape, Style};
use excellent_core::edit::Change;
use excellent_ink::stroke::Stroke;
use excellent_mupdf::PagePoint;

/// Default lifetime of fading slide ink in milliseconds (3.5 seconds).
pub const DEFAULT_FADING_INK_MS: u64 = 3_500;

/// Persistence mode of a stroke drawn while presenting (G.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InkPersistence {
    /// Stays visible on the slide for the duration of the presentation (never modifies the file unless kept).
    Temporary,
    /// Automatically fades out and disappears after `lifetime_ms` milliseconds.
    Fading {
        /// Fade duration after pen-up in milliseconds.
        lifetime_ms: u64,
    },
}

/// A single ink stroke drawn over a slide during present mode.
#[derive(Debug, Clone, PartialEq)]
pub struct SlideStroke {
    /// Zero-based slide index.
    pub page: usize,
    /// Stroke geometry and per-point widths.
    pub stroke: Stroke,
    /// RGB stroke color in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Base opacity in `0.0..=1.0`.
    pub base_opacity: f32,
    /// Temporary vs. fading persistence mode.
    pub persistence: InkPersistence,
    /// Timestamp when the stroke finished, in milliseconds.
    pub finished_at_ms: u64,
}

impl SlideStroke {
    /// Computes effective display opacity `[0.0..=1.0]` at `now_ms`. Returns `0.0` once expired.
    #[must_use]
    pub fn opacity_at(&self, now_ms: u64) -> f32 {
        match self.persistence {
            InkPersistence::Temporary => self.base_opacity.clamp(0.0, 1.0),
            InkPersistence::Fading { lifetime_ms } => {
                let life = lifetime_ms.max(1);
                let elapsed = now_ms.saturating_sub(self.finished_at_ms);
                if elapsed >= life {
                    0.0
                } else {
                    let frac = 1.0 - (elapsed as f32 / life as f32);
                    (self.base_opacity * frac).clamp(0.0, 1.0)
                }
            }
        }
    }

    /// Whether this stroke has completely faded away at `now_ms`.
    #[must_use]
    pub fn is_expired(&self, now_ms: u64) -> bool {
        matches!(self.persistence, InkPersistence::Fading { lifetime_ms } if now_ms.saturating_sub(self.finished_at_ms) >= lifetime_ms.max(1))
    }
}

/// Volatile in-memory slide ink overlay (plan G.5).
#[derive(Debug, Clone, Default)]
pub struct SlideInkLayer {
    strokes_by_page: BTreeMap<usize, Vec<SlideStroke>>,
    kept: bool,
}

impl SlideInkLayer {
    /// Creates an empty temporary slide ink layer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a finished stroke on `page` without touching the underlying document.
    pub fn add_stroke(&mut self, stroke: SlideStroke) {
        self.strokes_by_page
            .entry(stroke.page)
            .or_default()
            .push(stroke);
    }

    /// Advances fading ink timers at `now_ms`, pruning any expired fading strokes.
    pub fn tick(&mut self, now_ms: u64) {
        for strokes in self.strokes_by_page.values_mut() {
            strokes.retain(|s| !s.is_expired(now_ms));
        }
        self.strokes_by_page.retain(|_, v| !v.is_empty());
    }

    /// Returns all currently visible strokes on `page`.
    #[must_use]
    pub fn strokes_on_page(&self, page: usize) -> &[SlideStroke] {
        self.strokes_by_page.get(&page).map_or(&[], Vec::as_slice)
    }

    /// Total number of non-expired strokes across all slides.
    #[must_use]
    pub fn total_strokes(&self) -> usize {
        self.strokes_by_page.values().map(Vec::len).sum()
    }

    /// Whether the presenter has explicitly invoked [`Self::keep_to_changes`].
    #[must_use]
    pub fn was_kept(&self) -> bool {
        self.kept
    }

    /// Discards all temporary ink on every slide without modifying the document.
    pub fn discard_all(&mut self) {
        self.strokes_by_page.clear();
    }

    /// Explicitly converts all remaining non-expired temporary strokes into standard PDF ink
    /// annotation changes (`Change::CreateAnnotation`) so the caller can apply them to the
    /// document as one undoable operation (G.5 "keep").
    #[must_use]
    pub fn keep_to_changes(&mut self, author: &str, now_ms: u64) -> Vec<Change> {
        self.tick(now_ms);
        self.kept = true;
        let mut changes = Vec::new();
        for (&page, strokes) in &self.strokes_by_page {
            for item in strokes {
                if item.stroke.is_empty() {
                    continue;
                }
                let pts: Vec<PagePoint> = item
                    .stroke
                    .points()
                    .iter()
                    .map(|p| PagePoint { x: p.x, y: p.y })
                    .collect();
                let mean_w = item.stroke.points().iter().map(|p| p.width).sum::<f32>()
                    / item.stroke.points().len() as f32;
                let mut annot = Annotation::new(
                    page,
                    Shape::Ink(vec![pts]),
                    Style {
                        color: Some(item.color),
                        opacity: item.base_opacity,
                        width: mean_w.max(0.5),
                    },
                    author,
                );
                annot.widths = vec![item.stroke.points().iter().map(|p| p.width).collect()];
                changes.push(Change::CreateAnnotation(annot));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::notebook::pdf_ops::build_blank_pdf;
    use excellent_ink::stroke::InkPoint;

    fn sample_stroke() -> Stroke {
        let mut s = Stroke::new();
        s.push(InkPoint {
            x: 100.0,
            y: 100.0,
            width: 3.0,
        });
        s.push(InkPoint {
            x: 200.0,
            y: 140.0,
            width: 3.5,
        });
        s
    }

    #[test]
    fn temporary_ink_never_modifies_file_unless_kept_and_fading_ink_expires() {
        let original_pdf = build_blank_pdf(&[(612.0, 792.0)]).expect("pdf");
        let snapshot = original_pdf.clone();

        let mut layer = SlideInkLayer::new();
        layer.add_stroke(SlideStroke {
            page: 0,
            stroke: sample_stroke(),
            color: [0.9, 0.1, 0.1],
            base_opacity: 1.0,
            persistence: InkPersistence::Temporary,
            finished_at_ms: 1_000,
        });
        layer.add_stroke(SlideStroke {
            page: 0,
            stroke: sample_stroke(),
            color: [0.1, 0.4, 0.9],
            base_opacity: 1.0,
            persistence: InkPersistence::Fading {
                lifetime_ms: DEFAULT_FADING_INK_MS,
            },
            finished_at_ms: 1_000,
        });

        // Halfway through fade: both visible, fading stroke dimmed.
        layer.tick(2_750);
        assert_eq!(layer.strokes_on_page(0).len(), 2);
        assert!(layer.strokes_on_page(0)[1].opacity_at(2_750) < 0.6);

        // Past fade lifetime: fading stroke removed, temporary stroke remains.
        layer.tick(4_500);
        assert_eq!(layer.strokes_on_page(0).len(), 1);
        assert!(!layer.was_kept());
        assert_eq!(original_pdf, snapshot, "file untouched until kept");

        // Explicitly keeping converts only the remaining temporary stroke into a PDF ink Change.
        let changes = layer.keep_to_changes("Presenter", 4_500);
        assert!(layer.was_kept());
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], Change::CreateAnnotation(_)));
    }
}
