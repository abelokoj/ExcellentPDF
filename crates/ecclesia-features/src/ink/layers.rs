//! Ink and drawing layers (H.14, plan §27.7) built over PDF optional content groups (A.22).
//!
//! Provides four fixed kind layers (`Ink`, `Highlights`, `Text`, `Images`) plus arbitrary user-created layers. Each
//! layer maps directly to a PDF optional content group (`ecclesia_mupdf::Layer`) via [`Change::SetLayer`]. Locking a
//! layer makes every annotation on it unselectable and unerasable (in addition to locking its OCG visibility in PDF).

use std::fmt;

use ecclesia_core::annot::{Annotation, Boxed, Shape};
use ecclesia_core::edit::Change;
use ecclesia_mupdf::Layer;

use crate::annotate::erase::Erased;
use crate::annotate::layers as ocg;
use crate::annotate::notes::Target;

/// The four built-in fixed layer kinds always present in the ink layer stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixedLayer {
    /// Freehand pen strokes and vector shapes.
    Ink,
    /// Text highlights and highlighter-pen ink.
    Highlights,
    /// Text boxes, typewriter text, sticky notes, callouts, and insertion carets.
    Text,
    /// Stamps and placed image annotations.
    Images,
}

impl FixedLayer {
    /// All four fixed layers in display order.
    pub const ALL: [Self; 4] = [Self::Ink, Self::Highlights, Self::Text, Self::Images];

    /// The canonical PDF optional content group name for this fixed layer.
    #[must_use]
    pub fn ocg_name(self) -> &'static str {
        match self {
            Self::Ink => "Ink",
            Self::Highlights => "Highlights",
            Self::Text => "Text",
            Self::Images => "Images",
        }
    }

    /// Parses a PDF OCG name into a fixed layer kind, if it matches one.
    #[must_use]
    pub fn from_ocg_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.ocg_name().eq_ignore_ascii_case(name.trim()))
    }

    /// Classifies an annotation into its default fixed layer kind when it carries no explicit user layer name.
    #[must_use]
    pub fn classify(annotation: &Annotation) -> Self {
        match annotation.shape {
            Shape::Ink(_) if annotation.multiply => Self::Highlights,
            Shape::Markup(..) => Self::Highlights,
            Shape::Boxed(Boxed::Note | Boxed::FreeText | Boxed::Typewriter | Boxed::Caret, _)
            | Shape::Callout(..) => Self::Text,
            Shape::Boxed(Boxed::Stamp, _) => Self::Images,
            Shape::Ink(_)
            | Shape::Line(..)
            | Shape::Path(..)
            | Shape::Boxed(Boxed::Square | Boxed::Circle, _) => Self::Ink,
        }
    }
}

/// Whether a layer is one of the four fixed kinds or a user-created layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LayerKind {
    /// One of the four built-in fixed kinds.
    Fixed(FixedLayer),
    /// A custom layer created by the user.
    User(String),
}

/// Visibility state of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// Layer content is rendered and interactive (when unlocked).
    #[default]
    Visible,
    /// Layer content is hidden from the page view.
    Hidden,
}

impl Visibility {
    /// Whether the layer is currently visible.
    #[must_use]
    pub fn is_visible(self) -> bool {
        self == Self::Visible
    }

    fn from_flag(visible: bool) -> Self {
        if visible { Self::Visible } else { Self::Hidden }
    }
}

/// Lock state of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LockState {
    /// Annotations on the layer may be selected, edited, and erased.
    #[default]
    Unlocked,
    /// Annotations on the layer cannot be selected, edited, or erased, and visibility cannot be toggled.
    Locked,
}

impl LockState {
    /// Whether the layer is locked.
    #[must_use]
    pub fn is_locked(self) -> bool {
        self == Self::Locked
    }

    fn from_flag(locked: bool) -> Self {
        if locked { Self::Locked } else { Self::Unlocked }
    }
}

/// One layer entry in the ink layer stack, backed by a PDF OCG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InkLayer {
    /// Fixed kind or custom user layer.
    pub kind: LayerKind,
    /// Whether the layer is shown or hidden.
    pub visibility: Visibility,
    /// Whether the layer is locked.
    pub lock: LockState,
}

impl InkLayer {
    /// The PDF optional content group name for this layer.
    #[must_use]
    pub fn name(&self) -> &str {
        match &self.kind {
            LayerKind::Fixed(fixed) => fixed.ocg_name(),
            LayerKind::User(name) => name,
        }
    }

    /// Converts to the `ecclesia_mupdf::Layer` representation.
    #[must_use]
    pub fn to_mupdf(&self) -> Layer {
        Layer {
            name: self.name().to_owned(),
            visible: self.visibility.is_visible(),
            locked: self.lock.is_locked(),
        }
    }

    /// Builds an [`InkLayer`] from a PDF OCG [`Layer`].
    #[must_use]
    pub fn from_mupdf(layer: &Layer) -> Self {
        let kind = match FixedLayer::from_ocg_name(&layer.name) {
            Some(fixed) => LayerKind::Fixed(fixed),
            None => LayerKind::User(layer.name.clone()),
        };
        Self {
            kind,
            visibility: Visibility::from_flag(layer.visible),
            lock: LockState::from_flag(layer.locked),
        }
    }

    /// Whether this is one of the four fixed layers.
    #[must_use]
    pub fn is_fixed(&self) -> bool {
        matches!(self.kind, LayerKind::Fixed(_))
    }
}

/// Errors produced when mutating or validating the layer stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerError {
    /// The supplied layer name is empty after trimming.
    EmptyName,
    /// A layer with this name already exists.
    DuplicateName(String),
    /// No layer with this name exists in the stack.
    NotFound(String),
    /// The target layer is locked and cannot be drawn on, modified, or toggled.
    Locked(String),
    /// The target layer is hidden and cannot be made active for drawing.
    Hidden(String),
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("layer name cannot be empty"),
            Self::DuplicateName(name) => write!(f, "layer {name:?} already exists"),
            Self::NotFound(name) => write!(f, "layer {name:?} does not exist"),
            Self::Locked(name) => write!(f, "layer {name:?} is locked"),
            Self::Hidden(name) => write!(f, "layer {name:?} is hidden"),
        }
    }
}

impl std::error::Error for LayerError {}

/// Ordered stack of fixed kind layers plus user layers, synchronized with a document's A.22 OCGs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerStack {
    layers: Vec<InkLayer>,
    active: String,
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerStack {
    /// Creates a stack with the four default visible, unlocked fixed layers and `"Ink"` active.
    #[must_use]
    pub fn new() -> Self {
        Self::from_ocgs(&[])
    }

    /// Builds the stack from the PDF's existing A.22 OCG list, ensuring all four fixed layers are always present.
    #[must_use]
    pub fn from_ocgs(ocgs: &[Layer]) -> Self {
        let mut layers: Vec<InkLayer> = FixedLayer::ALL
            .into_iter()
            .map(|fixed| {
                let matching = ocgs
                    .iter()
                    .find(|l| FixedLayer::from_ocg_name(&l.name) == Some(fixed));
                match matching {
                    Some(existing) => InkLayer {
                        kind: LayerKind::Fixed(fixed),
                        visibility: Visibility::from_flag(existing.visible),
                        lock: LockState::from_flag(existing.locked),
                    },
                    None => InkLayer {
                        kind: LayerKind::Fixed(fixed),
                        visibility: Visibility::Visible,
                        lock: LockState::Unlocked,
                    },
                }
            })
            .collect();

        for ocg_layer in ocgs {
            if FixedLayer::from_ocg_name(&ocg_layer.name).is_some() {
                continue;
            }
            if ocg_layer.name.trim().is_empty() {
                continue;
            }
            if !layers.iter().any(|l| l.name() == ocg_layer.name) {
                layers.push(InkLayer::from_mupdf(ocg_layer));
            }
        }

        Self {
            layers,
            active: FixedLayer::Ink.ocg_name().to_owned(),
        }
    }

    /// All layers in display order (the four fixed layers first, then user layers).
    #[must_use]
    pub fn layers(&self) -> &[InkLayer] {
        &self.layers
    }

    /// Exports the full stack as A.22 [`Layer`] records.
    #[must_use]
    pub fn to_mupdf_layers(&self) -> Vec<Layer> {
        self.layers.iter().map(InkLayer::to_mupdf).collect()
    }

    /// Looks up a layer by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&InkLayer> {
        self.layers.iter().find(|l| l.name() == name)
    }

    /// Name of the active layer for new drawings.
    #[must_use]
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Sets the active drawing layer.
    ///
    /// # Errors
    ///
    /// Fails if the layer does not exist, is locked, or is hidden.
    pub fn set_active(&mut self, name: &str) -> Result<(), LayerError> {
        let Some(layer) = self.find(name) else {
            return Err(LayerError::NotFound(name.to_owned()));
        };
        if layer.lock.is_locked() {
            return Err(LayerError::Locked(name.to_owned()));
        }
        if !layer.visibility.is_visible() {
            return Err(LayerError::Hidden(name.to_owned()));
        }
        self.active = layer.name().to_owned();
        Ok(())
    }

    /// Adds a new visible, unlocked user layer and returns the A.22 [`Change::SetLayer`] to persist it.
    ///
    /// # Errors
    ///
    /// Returns [`LayerError::EmptyName`] if `typed` is blank, or [`LayerError::DuplicateName`] if a fixed or user
    /// layer with that name already exists.
    pub fn add_user_layer(&mut self, typed: &str) -> Result<Change, LayerError> {
        let clean = ocg::name(typed).ok_or(LayerError::EmptyName)?;
        if FixedLayer::from_ocg_name(&clean).is_some()
            || self.layers.iter().any(|l| l.name() == clean)
        {
            return Err(LayerError::DuplicateName(clean));
        }
        self.layers.push(InkLayer {
            kind: LayerKind::User(clean.clone()),
            visibility: Visibility::Visible,
            lock: LockState::Unlocked,
        });
        Ok(Change::SetLayer {
            name: clean,
            visible: true,
            locked: false,
        })
    }

    /// Shows or hides layer `name`, returning the A.22 [`Change::SetLayer`] when the state changes.
    ///
    /// # Errors
    ///
    /// Returns [`LayerError::NotFound`] if the layer is absent, or [`LayerError::Locked`] if it is locked.
    pub fn set_visibility(
        &mut self,
        name: &str,
        visibility: Visibility,
    ) -> Result<Option<Change>, LayerError> {
        let Some(layer) = self.layers.iter_mut().find(|l| l.name() == name) else {
            return Err(LayerError::NotFound(name.to_owned()));
        };
        if layer.lock.is_locked() {
            return Err(LayerError::Locked(name.to_owned()));
        }
        let change = ocg::show(&layer.to_mupdf(), visibility.is_visible());
        if change.is_some() {
            layer.visibility = visibility;
        }
        Ok(change)
    }

    /// Locks or unlocks layer `name`, returning the A.22 [`Change::SetLayer`] when the state changes.
    ///
    /// # Errors
    ///
    /// Returns [`LayerError::NotFound`] if the layer is absent.
    pub fn set_lock(&mut self, name: &str, lock: LockState) -> Result<Option<Change>, LayerError> {
        let Some(layer) = self.layers.iter_mut().find(|l| l.name() == name) else {
            return Err(LayerError::NotFound(name.to_owned()));
        };
        let change = ocg::lock(&layer.to_mupdf(), lock.is_locked());
        if change.is_some() {
            layer.lock = lock;
        }
        Ok(change)
    }

    /// Returns the effective layer name for `annotation`: its explicit OCG layer if present, otherwise its fixed kind.
    #[must_use]
    pub fn effective_layer<'a>(&self, annotation: &'a Annotation) -> &'a str {
        annotation
            .layer
            .as_deref()
            .unwrap_or_else(|| FixedLayer::classify(annotation).ocg_name())
    }

    /// Whether `annotation`'s layer is visible.
    #[must_use]
    pub fn is_visible(&self, annotation: &Annotation) -> bool {
        let name = self.effective_layer(annotation);
        self.find(name).is_none_or(|l| l.visibility.is_visible())
    }

    /// Whether `annotation` is locked either directly (A.12) or because its layer is locked (H.14).
    #[must_use]
    pub fn is_locked(&self, annotation: &Annotation) -> bool {
        if annotation.locked {
            return true;
        }
        let name = self.effective_layer(annotation);
        self.find(name).is_some_and(|l| l.lock.is_locked())
    }

    /// Whether `annotation` may be selected (hit-test, marquee, or lasso): must be on a visible, unlocked layer.
    #[must_use]
    pub fn can_select(&self, annotation: &Annotation) -> bool {
        self.is_visible(annotation) && !self.is_locked(annotation)
    }

    /// Whether `annotation` may be erased by any eraser tool: must be on a visible, unlocked layer.
    #[must_use]
    pub fn can_erase(&self, annotation: &Annotation) -> bool {
        self.is_visible(annotation) && !self.is_locked(annotation)
    }

    /// Returns only those annotations that can currently be selected.
    #[must_use]
    pub fn filter_selectable<'a>(&self, annotations: &'a [Annotation]) -> Vec<&'a Annotation> {
        annotations.iter().filter(|a| self.can_select(a)).collect()
    }

    /// Returns only those annotations that can currently be erased.
    #[must_use]
    pub fn filter_erasable<'a>(&self, annotations: &'a [Annotation]) -> Vec<&'a Annotation> {
        annotations.iter().filter(|a| self.can_erase(a)).collect()
    }

    /// Filters a candidate selection (for example from lasso or marquee) so annotations on locked or hidden layers
    /// are excluded.
    #[must_use]
    pub fn filter_selection(&self, targets: &[Target], annotations: &[Annotation]) -> Vec<Target> {
        targets
            .iter()
            .filter(|target| {
                annotations
                    .iter()
                    .find(|a| a.page == target.page && a.id == target.id)
                    .is_some_and(|a| self.can_select(a))
            })
            .cloned()
            .collect()
    }

    /// Runs an eraser pass (`H.5`) while guaranteeing annotations on locked or hidden layers are never erased or split.
    #[must_use]
    pub fn erase_changes(&self, erased: &Erased, annotations: &[Annotation]) -> Vec<Change> {
        let erasable: Vec<Annotation> = annotations
            .iter()
            .filter(|a| self.can_erase(a))
            .cloned()
            .collect();
        erased.changes(&erasable)
    }

    /// Moves unlocked annotations from unlocked layers onto `target_layer`.
    ///
    /// # Errors
    ///
    /// Returns [`LayerError::NotFound`] if `target_layer` does not exist, or [`LayerError::Locked`] if `target_layer`
    /// is locked.
    pub fn assign_layer(
        &self,
        annotations: &[Annotation],
        target_layer: &str,
    ) -> Result<Vec<Change>, LayerError> {
        let Some(dest) = self.find(target_layer) else {
            return Err(LayerError::NotFound(target_layer.to_owned()));
        };
        if dest.lock.is_locked() {
            return Err(LayerError::Locked(target_layer.to_owned()));
        }
        let movable: Vec<Annotation> = annotations
            .iter()
            .filter(|a| !self.is_locked(a))
            .cloned()
            .collect();
        Ok(ocg::move_to(&movable, Some(dest.name())))
    }

    /// Tags newly created annotations in `changes` with the active layer when it is a user layer, or with their fixed
    /// kind layer when the active layer is a fixed layer.
    ///
    /// # Errors
    ///
    /// Fails if the active layer is locked or hidden.
    pub fn place_new(&self, changes: &[Change]) -> Result<Vec<Change>, LayerError> {
        let Some(active) = self.find(&self.active) else {
            return Err(LayerError::NotFound(self.active.clone()));
        };
        if active.lock.is_locked() {
            return Err(LayerError::Locked(self.active.clone()));
        }
        if !active.visibility.is_visible() {
            return Err(LayerError::Hidden(self.active.clone()));
        }
        let user_active = (!active.is_fixed()).then(|| active.name().to_owned());
        Ok(changes
            .iter()
            .map(|change| match change {
                Change::CreateAnnotation(made) if made.layer.is_none() && !made.is_reply() => {
                    let mut placed = made.clone();
                    placed.layer = Some(
                        user_active
                            .clone()
                            .unwrap_or_else(|| FixedLayer::classify(made).ocg_name().to_owned()),
                    );
                    Change::CreateAnnotation(placed)
                }
                other => other.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::{Markup, Style};
    use ecclesia_mupdf::{PagePoint, PageRect};
    use serde_json::json;

    use crate::annotate::edit::in_marquee;
    use crate::annotate::lasso::Looped;

    use super::*;

    fn ink_annot(id: &str, y: f32, layer: Option<&str>) -> Annotation {
        let mut a = Annotation::new(
            0,
            Shape::Ink(vec![vec![
                PagePoint { x: 10.0, y },
                PagePoint { x: 90.0, y },
            ]]),
            Style {
                color: Some([0.1, 0.2, 0.8]),
                opacity: 1.0,
                width: 2.0,
            },
            "Ada",
        );
        a.id = id.to_owned();
        a.layer = layer.map(str::to_owned);
        a
    }

    #[test]
    fn locked_layers_cannot_be_selected_or_erased() {
        let mut stack = LayerStack::new();
        let added = stack.add_user_layer("Diagrams").expect("adds user layer");
        assert_eq!(
            added,
            Change::SetLayer {
                name: "Diagrams".into(),
                visible: true,
                locked: false,
            }
        );

        let on_ink = ink_annot("stroke-ink", 20.0, Some("Ink"));
        let on_diagrams = ink_annot("stroke-diag", 20.0, Some("Diagrams"));
        let mut on_highlights = ink_annot("stroke-hi", 20.0, None);
        on_highlights.multiply = true;
        let all = [on_ink.clone(), on_diagrams.clone(), on_highlights.clone()];

        // Lock Diagrams and Highlights layers.
        let lock_diag = stack
            .set_lock("Diagrams", LockState::Locked)
            .expect("locks Diagrams");
        assert_eq!(
            lock_diag,
            Some(Change::SetLayer {
                name: "Diagrams".into(),
                visible: true,
                locked: true,
            })
        );
        stack
            .set_lock("Highlights", LockState::Locked)
            .expect("locks Highlights");

        // Lasso selection covering everything must exclude locked layers.
        let lasso = Looped::parse(&json!({
            "page": 0,
            "path": [[0, 0], [100, 0], [100, 50], [0, 50]],
            "add": false
        }))
        .expect("parses lasso");
        let raw_lasso = lasso.takes(&all);
        assert_eq!(raw_lasso.len(), 3);
        let allowed_lasso = stack.filter_selection(&raw_lasso, &all);
        assert_eq!(allowed_lasso.len(), 1);
        assert_eq!(allowed_lasso[0].id, "stroke-ink");

        // Marquee selection likewise excludes locked layers.
        let raw_marquee = in_marquee(
            &all,
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 50.0,
            },
        );
        let allowed_marquee = stack.filter_selection(&raw_marquee, &all);
        assert_eq!(allowed_marquee.len(), 1);
        assert_eq!(allowed_marquee[0].id, "stroke-ink");

        // Stroke, area, and highlighter erasers cannot erase anything on locked layers.
        for eraser in ["stroke", "area", "highlighter"] {
            let pass = Erased::parse(&json!({
                "page": 0,
                "eraser": eraser,
                "radius": 6.0,
                "points": [[50.0, 10.0], [50.0, 30.0]]
            }))
            .expect("parses eraser");
            let changes = stack.erase_changes(&pass, &all);
            for change in &changes {
                let touched_id = match change {
                    Change::DeleteAnnotation { id, .. } => id.as_str(),
                    Change::UpdateAnnotation(a) => a.id.as_str(),
                    other => panic!("unexpected change: {other:?}"),
                };
                assert_ne!(
                    touched_id, "stroke-diag",
                    "{eraser} eraser touched locked Diagrams layer"
                );
                assert_ne!(
                    touched_id, "stroke-hi",
                    "{eraser} eraser touched locked Highlights layer"
                );
            }
            if eraser == "highlighter" {
                assert!(changes.is_empty(), "highlighter on locked layer kept");
            } else {
                assert_eq!(changes.len(), 1, "only unlocked Ink stroke erased");
            }
        }

        // Unlocking Diagrams makes its strokes selectable and erasable again.
        stack
            .set_lock("Diagrams", LockState::Unlocked)
            .expect("unlocks Diagrams");
        assert!(stack.can_select(&on_diagrams));
        assert!(stack.can_erase(&on_diagrams));
    }

    #[test]
    fn fixed_layers_classify_kinds_and_round_trip_through_ocgs() {
        let mut stack = LayerStack::from_ocgs(&[
            Layer {
                name: "Text".into(),
                visible: false,
                locked: true,
            },
            Layer {
                name: "Drafts".into(),
                visible: true,
                locked: false,
            },
        ]);
        assert_eq!(stack.layers().len(), 5);
        assert_eq!(
            stack.find("Text").map(|l| (l.visibility, l.lock)),
            Some((Visibility::Hidden, LockState::Locked))
        );
        assert!(stack.set_visibility("Text", Visibility::Visible).is_err());
        assert_eq!(
            stack.set_active("Text"),
            Err(LayerError::Locked("Text".into()))
        );
        assert!(stack.add_user_layer("ink").is_err(), "collides with fixed");

        let highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        let note = Annotation::new(
            0,
            Shape::Boxed(Boxed::FreeText, PageRect::default()),
            Style::default(),
            "Ada",
        );
        let stamp = Annotation::new(
            0,
            Shape::Boxed(Boxed::Stamp, PageRect::default()),
            Style::default(),
            "Ada",
        );
        assert_eq!(FixedLayer::classify(&highlight), FixedLayer::Highlights);
        assert_eq!(FixedLayer::classify(&note), FixedLayer::Text);
        assert_eq!(FixedLayer::classify(&stamp), FixedLayer::Images);
        assert!(!stack.can_select(&note), "Text layer is hidden and locked");

        stack.set_active("Drafts").expect("activates Drafts");
        let placed = stack
            .place_new(&[Change::CreateAnnotation(ink_annot("new", 10.0, None))])
            .expect("places on active user layer");
        let Change::CreateAnnotation(ann) = &placed[0] else {
            panic!("expected create");
        };
        assert_eq!(ann.layer.as_deref(), Some("Drafts"));
    }
}
