//! Annotation layers by person or purpose (A.22): PDF optional content groups, which Acrobat lists in its Layers
//! panel. An annotation is put on a layer by name; a hidden layer hides what is on it, here and in other readers; a
//! locked one can't be shown or hidden until it is unlocked. Each reviewer gets a layer of their own: sorting puts
//! every annotation without a layer on its author's, and from then on a new annotation goes on its author's layer by
//! itself. Pure: the app reads the layers and applies the changes.

use excellent_core::annot::Annotation;
use excellent_core::edit::Change;
use excellent_mupdf::Layer;

/// The longest layer name kept: Acrobat shows about this much in its panel.
const LONGEST: usize = 80;

/// `typed` as a layer name: one line, trimmed, at most [`LONGEST`] characters; `None` when that is empty.
#[must_use]
pub fn name(typed: &str) -> Option<String> {
    let line = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = line.chars().take(LONGEST).collect();
    let name = name.trim_end().to_owned();
    (!name.is_empty()).then_some(name)
}

/// `annotations` moved onto the layer `layer` (`None`: off any), one change each; none for those already there,
/// and none for a locked annotation (A.12) or a reply, which goes with its parent.
#[must_use]
pub fn move_to(annotations: &[Annotation], layer: Option<&str>) -> Vec<Change> {
    annotations
        .iter()
        .filter(|annotation| {
            !annotation.locked && !annotation.is_reply() && annotation.layer.as_deref() != layer
        })
        .map(|annotation| {
            let mut moved = annotation.clone();
            moved.layer = layer.map(str::to_owned);
            Change::UpdateAnnotation(moved)
        })
        .collect()
}

/// Every annotation of `annotations` on no layer put on its author's, named for them (`unnamed` for one without an
/// author); locked ones and replies stay as they are.
#[must_use]
pub fn by_author(annotations: &[Annotation], unnamed: &str) -> Vec<Change> {
    annotations
        .iter()
        .filter(|annotation| annotation.layer.is_none())
        .filter_map(|annotation| {
            let layer = name(&annotation.author).or_else(|| name(unnamed))?;
            move_to(std::slice::from_ref(annotation), Some(&layer)).pop()
        })
        .collect()
}

/// `layer` shown or hidden; `None` when it already is, or it is locked.
#[must_use]
pub fn show(layer: &Layer, visible: bool) -> Option<Change> {
    (layer.visible != visible && !layer.locked).then(|| Change::SetLayer {
        name: layer.name.clone(),
        visible,
        locked: layer.locked,
    })
}

/// `layer` locked or unlocked; `None` when it already is.
#[must_use]
pub fn lock(layer: &Layer, locked: bool) -> Option<Change> {
    (layer.locked != locked).then(|| Change::SetLayer {
        name: layer.name.clone(),
        visible: layer.visible,
        locked,
    })
}

/// Whether `annotation` is drawn, of `layers`: on no layer, or on one that is shown. The first layer of a name
/// counts, as the file's own order lists them.
#[must_use]
pub fn shown(annotation: &Annotation, layers: &[Layer]) -> bool {
    annotation.layer.as_deref().is_none_or(|on| {
        layers
            .iter()
            .find(|layer| layer.name == on)
            .is_none_or(|layer| layer.visible)
    })
}

/// The layer a new annotation by `author` goes on, of `layers`: the one named for them, when the document has it.
#[must_use]
pub fn default_for(author: &str, layers: &[Layer]) -> Option<String> {
    let own = name(author)?;
    layers.iter().any(|layer| layer.name == own).then_some(own)
}

/// `changes` with each new annotation on no layer (not a reply) put on its author's default layer, of `layers`.
#[must_use]
pub fn place_new(changes: &[Change], layers: &[Layer]) -> Vec<Change> {
    changes
        .iter()
        .map(|change| match change {
            Change::CreateAnnotation(made) if made.layer.is_none() && !made.is_reply() => {
                let mut made = made.clone();
                made.layer = default_for(&made.author, layers);
                Change::CreateAnnotation(made)
            }
            other => other.clone(),
        })
        .collect()
}

/// How many of `annotations` are on the layer `name`.
#[must_use]
pub fn count(annotations: &[Annotation], name: &str) -> usize {
    annotations
        .iter()
        .filter(|annotation| annotation.layer.as_deref() == Some(name))
        .count()
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Annotation, Boxed, Shape, Style};
    use excellent_core::edit::Change;
    use excellent_mupdf::{Layer, PageRect};

    use super::*;

    fn by(author: &str) -> Annotation {
        Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Square,
                PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 10.0,
                    y1: 10.0,
                },
            ),
            Style::default(),
            author,
        )
    }

    fn layer(name: &str, visible: bool, locked: bool) -> Layer {
        Layer {
            name: name.into(),
            visible,
            locked,
        }
    }

    fn moved(change: &Change) -> &Annotation {
        match change {
            Change::UpdateAnnotation(annotation) | Change::CreateAnnotation(annotation) => {
                annotation
            }
            other => panic!("expected an annotation, got {other:?}"),
        }
    }

    #[test]
    fn a_name_is_one_trimmed_line() {
        assert_eq!(name("  Legal\n review \t").as_deref(), Some("Legal review"));
        assert_eq!(name(" \n "), None);
        assert_eq!(name(&"x".repeat(200)).map(|n| n.len()), Some(LONGEST));
    }

    #[test]
    fn moving_skips_what_is_there_locked_or_a_reply() {
        let mut there = by("Ada");
        there.layer = Some("Ada".into());
        let mut locked = by("Ada");
        locked.locked = true;
        let mut reply = by("Ada");
        reply.reply_to = Some(there.id.clone());
        let free = by("Ada");
        let changes = move_to(&[there, locked, reply, free.clone()], Some("Ada"));
        assert_eq!(changes.len(), 1);
        assert_eq!(moved(&changes[0]).id, free.id);
        assert_eq!(moved(&changes[0]).layer.as_deref(), Some("Ada"));
        let off = move_to(std::slice::from_ref(moved(&changes[0])), None);
        assert_eq!(moved(&off[0]).layer, None);
    }

    #[test]
    fn sorting_by_author_gives_each_reviewer_a_layer() {
        let mut placed = by("Grace");
        placed.layer = Some("Legal".into());
        let changes = by_author(&[by(" Ada "), by(""), placed], "Unknown");
        let layers: Vec<_> = changes
            .iter()
            .map(|change| moved(change).layer.clone())
            .collect();
        assert_eq!(
            layers,
            [Some("Ada".to_owned()), Some("Unknown".to_owned())],
            "a placed one stays"
        );
    }

    #[test]
    fn a_locked_layer_keeps_its_visibility() {
        let open = layer("Ada", true, false);
        assert_eq!(show(&open, true), None);
        assert_eq!(
            show(&open, false),
            Some(Change::SetLayer {
                name: "Ada".into(),
                visible: false,
                locked: false
            })
        );
        let shut = layer("Ada", true, true);
        assert_eq!(show(&shut, false), None, "locked");
        assert_eq!(
            lock(&shut, false),
            Some(Change::SetLayer {
                name: "Ada".into(),
                visible: true,
                locked: false
            })
        );
        assert_eq!(lock(&shut, true), None);
    }

    #[test]
    fn what_a_hidden_layer_holds_is_not_shown() {
        let layers = [layer("Ada", false, false), layer("Grace", true, false)];
        let mut on_ada = by("Ada");
        on_ada.layer = Some("Ada".into());
        let mut on_grace = by("Grace");
        on_grace.layer = Some("Grace".into());
        let mut on_gone = by("Grace");
        on_gone.layer = Some("Elsewhere".into());
        assert!(!shown(&on_ada, &layers));
        assert!(shown(&on_grace, &layers));
        assert!(shown(&by("Ada"), &layers), "on no layer");
        assert!(shown(&on_gone, &layers), "a layer the list lacks");
        assert_eq!(count(&[on_ada, on_grace, on_gone], "Grace"), 1);
    }

    #[test]
    fn a_new_annotation_goes_on_its_author_s_layer() {
        let layers = [layer("Ada", true, false)];
        let mut reply = by("Ada");
        reply.reply_to = Some("parent".into());
        let mut placed = by("Ada");
        placed.layer = Some("Legal".into());
        let changes = place_new(
            &[
                Change::CreateAnnotation(by("Ada")),
                Change::CreateAnnotation(by("Grace")),
                Change::CreateAnnotation(reply),
                Change::CreateAnnotation(placed),
                Change::DeleteAnnotation {
                    page: 0,
                    id: "x".into(),
                },
            ],
            &layers,
        );
        let layer_of = |at: usize| moved(&changes[at]).layer.clone();
        assert_eq!(layer_of(0).as_deref(), Some("Ada"));
        assert_eq!(layer_of(1), None, "Grace has no layer");
        assert_eq!(layer_of(2), None, "a reply goes with its parent");
        assert_eq!(layer_of(3).as_deref(), Some("Legal"));
        assert!(matches!(changes[4], Change::DeleteAnnotation { .. }));
    }
}
