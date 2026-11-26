//! Bulk restyle for ink strokes (H.15, plan §27.7): change color, width, opacity, or pen type across a selection or
//! every stroke matching a filter (such as "all red ink on this page") in a single [`Operation`] so one undo reverts
//! the entire restyle cleanly.

use excellent_core::annot::{Annotation, Shape};
use excellent_core::edit::{Change, Operation};
use excellent_ink::pens::{Blend, PenKind};

use crate::annotate::colors::MIN_OPACITY;
use crate::annotate::edit;
use crate::ink::layers::LayerStack;

/// Width bounds in page points (matching H.6 / Enhanced limits).
const WIDTH_BOUNDS: (f32, f32) = (0.1, 72.0);

/// Default Euclidean RGB matching tolerance when filtering by color (for example "all red ink").
pub const DEFAULT_COLOR_TOLERANCE: f32 = 0.15;

/// Color matching criterion in linear 0..=1 RGB space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorMatch {
    /// Target RGB color (`0.0..=1.0`).
    pub rgb: [f32; 3],
    /// Maximum Euclidean distance in RGB space (`0.0` for exact within floating-point epsilon).
    pub tolerance: f32,
}

impl ColorMatch {
    /// Exact color match (within `0.01` per channel to tolerate PDF rounding).
    #[must_use]
    pub fn exact(rgb: [f32; 3]) -> Self {
        Self {
            rgb: clamp_rgb(rgb),
            tolerance: 0.01,
        }
    }

    /// Match near a `0xRRGGBB` hex color with [`DEFAULT_COLOR_TOLERANCE`].
    #[must_use]
    pub fn from_hex(hex: u32) -> Self {
        Self {
            rgb: hex_to_rgb(hex),
            tolerance: DEFAULT_COLOR_TOLERANCE,
        }
    }

    /// Match near `rgb` with an explicit Euclidean tolerance.
    #[must_use]
    pub fn with_tolerance(rgb: [f32; 3], tolerance: f32) -> Self {
        Self {
            rgb: clamp_rgb(rgb),
            tolerance: tolerance.max(0.0),
        }
    }

    /// Whether `candidate` falls within this color match.
    #[must_use]
    pub fn matches(&self, candidate: [f32; 3]) -> bool {
        let dr = candidate[0] - self.rgb[0];
        let dg = candidate[1] - self.rgb[1];
        let db = candidate[2] - self.rgb[2];
        (dr * dr + dg * dg + db * db).sqrt() <= self.tolerance
    }
}

/// Filter on highlighter vs regular ink strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InkCategory {
    /// Only regular (non-multiply) ink strokes.
    PenOnly,
    /// Only highlighter (multiply-blended) ink strokes.
    HighlighterOnly,
}

/// Filter selecting which ink annotations participate in a bulk restyle.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrokeFilter {
    /// Restrict to specific annotation IDs (for example a lasso selection).
    pub ids: Option<Vec<String>>,
    /// Restrict to a single zero-based page index.
    pub page: Option<usize>,
    /// Restrict to strokes matching this color within tolerance.
    pub color: Option<ColorMatch>,
    /// Restrict to strokes whose base width falls inside `(min, max)` inclusive.
    pub width_range: Option<(f32, f32)>,
    /// Restrict to regular pen ink or highlighter ink.
    pub category: Option<InkCategory>,
    /// Restrict to strokes on a named layer.
    pub layer: Option<String>,
    /// Restrict to strokes authored by a specific author.
    pub author: Option<String>,
}

impl StrokeFilter {
    /// A filter matching every ink annotation.
    #[must_use]
    pub fn all_ink() -> Self {
        Self::default()
    }

    /// A filter matching only annotations whose IDs are in `ids`.
    #[must_use]
    pub fn selection(ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            ids: Some(ids.into_iter().collect()),
            ..Self::default()
        }
    }

    /// A filter matching strokes of `color` on `page`.
    #[must_use]
    pub fn color_on_page(page: usize, color: ColorMatch) -> Self {
        Self {
            page: Some(page),
            color: Some(color),
            ..Self::default()
        }
    }

    /// Returns whether `annotation` matches all criteria of this filter. Non-ink annotations never match.
    #[must_use]
    pub fn matches(&self, annotation: &Annotation) -> bool {
        if !matches!(annotation.shape, Shape::Ink(_)) {
            return false;
        }
        if let Some(ids) = &self.ids
            && !ids.iter().any(|id| id == &annotation.id)
        {
            return false;
        }
        if let Some(page) = self.page
            && annotation.page != page
        {
            return false;
        }
        if let Some(color_match) = &self.color {
            let Some(rgb) = annotation.style.color else {
                return false;
            };
            if !color_match.matches(rgb) {
                return false;
            }
        }
        if let Some((min_w, max_w)) = self.width_range
            && (annotation.style.width < min_w || annotation.style.width > max_w)
        {
            return false;
        }
        if let Some(cat) = self.category {
            match cat {
                InkCategory::PenOnly if annotation.multiply => return false,
                InkCategory::HighlighterOnly if !annotation.multiply => return false,
                _ => {}
            }
        }
        if let Some(layer) = &self.layer
            && annotation.layer.as_deref() != Some(layer.as_str())
        {
            return false;
        }
        if let Some(author) = &self.author
            && annotation.author != *author
        {
            return false;
        }
        true
    }
}

/// How stroke widths are updated during a bulk restyle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WidthChange {
    /// Set the stroke's base width to an exact value in points, scaling any per-point pressure widths proportionally.
    Set(f32),
    /// Multiply the stroke's width (and any per-point widths) by a positive scale factor.
    Scale(f32),
}

/// Style properties to apply in bulk to matching ink strokes. Any `None` field leaves that property unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RestyleSpec {
    /// New RGB color (`0.0..=1.0`).
    pub color: Option<[f32; 3]>,
    /// New opacity (`MIN_OPACITY..=1.0`).
    pub opacity: Option<f32>,
    /// New stroke width rule.
    pub width: Option<WidthChange>,
    /// Convert strokes to a target pen preset (updates blend mode, opacity unless overridden, and width profile).
    pub pen_kind: Option<PenKind>,
}

impl RestyleSpec {
    /// Whether this spec specifies at least one style change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.opacity.is_none()
            && self.width.is_none()
            && self.pen_kind.is_none()
    }
}

/// Returns all ink annotations in `annotations` matching `filter`.
#[must_use]
pub fn select_matching<'a>(
    annotations: &'a [Annotation],
    filter: &StrokeFilter,
) -> Vec<&'a Annotation> {
    annotations.iter().filter(|a| filter.matches(a)).collect()
}

/// Computes the individual [`Change::UpdateAnnotation`] edits for a bulk restyle, skipping locked annotations and
/// annotations on locked layers (when `layers` is provided).
#[must_use]
pub fn restyle_changes(
    annotations: &[Annotation],
    filter: &StrokeFilter,
    spec: &RestyleSpec,
    layers: Option<&LayerStack>,
) -> Vec<Change> {
    if spec.is_empty() {
        return Vec::new();
    }
    annotations
        .iter()
        .filter(|a| filter.matches(a))
        .filter(|a| !a.locked && layers.is_none_or(|stack| !stack.is_locked(a)))
        .filter_map(|before| {
            let after = apply_spec(before, spec);
            edit::change(before, after)
        })
        .collect()
}

/// Builds a single undoable [`Operation`] that restyles all matching ink strokes at once. Returns `None` when nothing
/// changes.
#[must_use]
pub fn restyle_operation(
    annotations: &[Annotation],
    filter: &StrokeFilter,
    spec: &RestyleSpec,
    layers: Option<&LayerStack>,
) -> Option<Operation> {
    let changes = restyle_changes(annotations, filter, spec, layers);
    if changes.is_empty() {
        return None;
    }
    let count = changes.len();
    let label = if count == 1 {
        "Restyle stroke".to_owned()
    } else {
        format!("Restyle {count} strokes")
    };
    Some(Operation::new(label, changes))
}

/// Convenience helper: builds one undoable [`Operation`] restyling a specific pre-selected slice of strokes.
#[must_use]
pub fn restyle_selection(
    selected: &[Annotation],
    spec: &RestyleSpec,
    layers: Option<&LayerStack>,
) -> Option<Operation> {
    restyle_operation(selected, &StrokeFilter::all_ink(), spec, layers)
}

fn apply_spec(before: &Annotation, spec: &RestyleSpec) -> Annotation {
    let mut after = before.clone();

    if let Some(kind) = spec.pen_kind {
        let pen = kind.pen(after.style.width.max(WIDTH_BOUNDS.0));
        after.multiply = pen.blend == Blend::Multiply;
        after.style.opacity = pen.opacity.clamp(MIN_OPACITY, 1.0);
        // Constant-width pens flatten per-point variable widths; pressure pens keep them.
        match kind {
            PenKind::Ballpoint | PenKind::Marker | PenKind::Highlighter => {
                after.widths.clear();
                if kind == PenKind::Highlighter && spec.width.is_none() && !before.multiply {
                    after.style.width = pen.curve.max.clamp(WIDTH_BOUNDS.0, WIDTH_BOUNDS.1);
                }
            }
            PenKind::Fountain | PenKind::Pencil | PenKind::Brush | PenKind::Calligraphy => {
                if before.multiply && spec.width.is_none() {
                    // Coming from a wide highlighter back to a writing pen: scale down by 0.25.
                    after.style.width =
                        (after.style.width * 0.25).clamp(WIDTH_BOUNDS.0, WIDTH_BOUNDS.1);
                }
            }
        }
    }

    if let Some(color) = spec.color {
        after.style.color = Some(clamp_rgb(color));
    }

    if let Some(opacity) = spec.opacity
        && opacity.is_finite()
    {
        after.style.opacity = opacity.clamp(MIN_OPACITY, 1.0);
    }

    if let Some(width_change) = spec.width {
        let old_width = after.style.width.max(f32::EPSILON);
        let new_width = match width_change {
            WidthChange::Set(w) if w.is_finite() => w.clamp(WIDTH_BOUNDS.0, WIDTH_BOUNDS.1),
            WidthChange::Scale(f) if f.is_finite() && f > 0.0 => {
                (old_width * f).clamp(WIDTH_BOUNDS.0, WIDTH_BOUNDS.1)
            }
            _ => old_width,
        };
        let ratio = new_width / old_width;
        after.style.width = new_width;
        if !after.widths.is_empty() {
            for stroke_widths in &mut after.widths {
                for w in stroke_widths {
                    *w = (*w * ratio).clamp(WIDTH_BOUNDS.0, WIDTH_BOUNDS.1);
                }
            }
        }
    }

    after
}

fn clamp_rgb(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|c| {
        if c.is_finite() {
            c.clamp(0.0, 1.0)
        } else {
            0.0
        }
    })
}

fn hex_to_rgb(hex: u32) -> [f32; 3] {
    [
        f32::from(((hex >> 16) & 0xff) as u8) / 255.0,
        f32::from(((hex >> 8) & 0xff) as u8) / 255.0,
        f32::from((hex & 0xff) as u8) / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::Style;
    use excellent_core::document::Document;
    use excellent_mupdf::PagePoint;

    use crate::ink::layers::LockState;

    use super::*;

    fn make_ink(id: &str, page: usize, color: [f32; 3], width: f32) -> Annotation {
        let mut a = Annotation::new(
            page,
            Shape::Ink(vec![vec![
                PagePoint { x: 20.0, y: 20.0 },
                PagePoint { x: 80.0, y: 20.0 },
                PagePoint { x: 120.0, y: 40.0 },
            ]]),
            Style {
                color: Some(color),
                opacity: 1.0,
                width,
            },
            "Ada",
        );
        a.id = id.to_owned();
        a.widths = vec![vec![width * 0.6, width, width * 1.4]];
        a
    }

    fn read_page_annots(doc: &Document, page: usize) -> Vec<Annotation> {
        let shared = doc.mupdf().expect("mupdf");
        let guard = shared.lock().expect("lock");
        let pg = guard.load_page(page).expect("load page");
        pg.annots()
            .expect("annots")
            .into_iter()
            .filter_map(|a| Annotation::from_data(page, &a.data().ok()?))
            .collect()
    }

    #[test]
    fn one_undo_reverts_the_whole_restyle() {
        let pdf_bytes = excellent_mupdf::html_to_pdf(
            "<p>Bulk restyle test page</p>",
            "p { margin: 20pt; }",
            612.0,
        )
        .expect("creates minimal PDF");
        let path =
            std::env::temp_dir().join(format!("excellent-h15-restyle-{}.pdf", std::process::id()));
        std::fs::write(&path, &pdf_bytes).expect("writes temp PDF");

        let doc = Document::open(&path).expect("opens PDF");
        let red = [0.9, 0.1, 0.1];
        let blue = [0.1, 0.2, 0.9];
        let green = [0.1, 0.7, 0.2];

        let initial = [
            make_ink("red-1", 0, red, 2.0),
            make_ink("red-2", 0, [0.88, 0.12, 0.1], 2.5),
            make_ink("red-3", 0, red, 3.0),
            make_ink("blue-1", 0, blue, 2.0),
        ];
        doc.apply(&Operation::new(
            "Add strokes",
            initial
                .iter()
                .cloned()
                .map(Change::CreateAnnotation)
                .collect(),
        ))
        .expect("adds initial strokes");

        let before = read_page_annots(&doc, 0);
        assert_eq!(before.len(), 4);

        // Bulk restyle: "all red ink on page 0" -> green, width 5.0, Marker.
        let filter = StrokeFilter::color_on_page(0, ColorMatch::with_tolerance(red, 0.15));
        let spec = RestyleSpec {
            color: Some(green),
            opacity: Some(0.8),
            width: Some(WidthChange::Set(5.0)),
            pen_kind: Some(PenKind::Marker),
        };
        let op = restyle_operation(&before, &filter, &spec, None).expect("builds restyle op");
        assert_eq!(op.changes.len(), 3, "only the 3 red strokes match");

        doc.apply(&op).expect("applies bulk restyle");
        let after = read_page_annots(&doc, 0);
        for id in ["red-1", "red-2", "red-3"] {
            let a = after.iter().find(|a| a.id == id).expect("found");
            let c = a.style.color.expect("color");
            assert!(
                (c[1] - green[1]).abs() < 0.02,
                "{id} should be green, got {c:?}"
            );
            assert!((a.style.width - 5.0).abs() < 0.05);
            assert!((a.style.opacity - 0.8).abs() < 0.02);
        }
        let blue_after = after.iter().find(|a| a.id == "blue-1").expect("blue kept");
        assert!((blue_after.style.color.expect("color")[2] - blue[2]).abs() < 0.02);

        // ONE undo reverts ALL 3 restyled strokes at once.
        assert!(doc.undo().expect("undo succeeds"));
        let reverted = read_page_annots(&doc, 0);
        for (id, orig_w) in [("red-1", 2.0), ("red-2", 2.5), ("red-3", 3.0)] {
            let a = reverted.iter().find(|a| a.id == id).expect("reverted");
            let c = a.style.color.expect("color");
            assert!(
                (c[0] - red[0]).abs() < 0.05,
                "{id} should be red again, got {c:?}"
            );
            assert!((a.style.width - orig_w).abs() < 0.05);
            assert!((a.style.opacity - 1.0).abs() < 0.02);
        }

        drop(doc);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pen_type_and_proportional_widths_and_locks_behave_correctly() {
        let mut stack = LayerStack::new();
        stack.add_user_layer("LockedLayer").expect("adds layer");
        stack
            .set_lock("LockedLayer", LockState::Locked)
            .expect("locks layer");

        let mut locked_stroke = make_ink("locked", 0, [0.0, 0.0, 0.0], 2.0);
        locked_stroke.layer = Some("LockedLayer".into());
        let free_stroke = make_ink("free", 0, [0.0, 0.0, 0.0], 2.0);

        // Scale widths by 2x keeping pressure proportions.
        let scaled_op = restyle_selection(
            &[locked_stroke.clone(), free_stroke.clone()],
            &RestyleSpec {
                width: Some(WidthChange::Scale(2.0)),
                ..RestyleSpec::default()
            },
            Some(&stack),
        )
        .expect("scales free stroke");
        assert_eq!(scaled_op.changes.len(), 1);
        let Change::UpdateAnnotation(updated) = &scaled_op.changes[0] else {
            panic!("expected update");
        };
        assert_eq!(updated.id, "free");
        assert!((updated.style.width - 4.0).abs() < 1e-4);
        assert!((updated.widths[0][0] - 2.4).abs() < 1e-4);
        assert!((updated.widths[0][2] - 5.6).abs() < 1e-4);

        // Convert to Highlighter sets multiply = true and clears per-point pressure widths.
        let hi_op = restyle_selection(
            &[free_stroke],
            &RestyleSpec {
                pen_kind: Some(PenKind::Highlighter),
                ..RestyleSpec::default()
            },
            None,
        )
        .expect("converts to highlighter");
        let Change::UpdateAnnotation(hi) = &hi_op.changes[0] else {
            panic!("expected update");
        };
        assert!(hi.multiply);
        assert!(hi.widths.is_empty());
        assert!(hi.style.width >= 8.0);
    }
}
