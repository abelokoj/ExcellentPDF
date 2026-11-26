//! `UIAccessibility` elements and custom rotor navigation for VoiceOver on iPadOS (plan U.3).
//!
//! Models document container, per-page elements, links, and annotation notes with screen frames
//! and a custom rotor iterator (`Pages`, `Headings`, `Links`, `Annotations`).

use excellent_canvas::Rect;
use excellent_canvas::layout::Layout;
use excellent_canvas::viewport::Viewport;

/// Accessibility element classification mapped to UIKit `UIAccessibilityTraits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessibilityElementKind {
    /// Root document container (`UIAccessibilityTraitSummaryElement`).
    DocumentContainer,
    /// Individual page element exposing reading-order text.
    PageElement,
    /// Hyperlink element (`UIAccessibilityTraitLink`).
    LinkElement,
    /// Annotation note or highlight (`UIAccessibilityTraitStaticText`).
    AnnotationElement,
}

/// Custom VoiceOver rotor category (`UIAccessibilityCustomRotor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessibilityRotorKind {
    /// Step through document pages.
    Pages,
    /// Step through outline/heading targets.
    Headings,
    /// Step through clickable document links.
    Links,
    /// Step through annotations and comments.
    Annotations,
}

/// Direction for VoiceOver rotor traversal (`UIAccessibilityCustomRotorDirection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RotorDirection {
    /// Next matching element (`UIAccessibilityCustomRotorDirectionNext`).
    Next,
    /// Previous matching element (`UIAccessibilityCustomRotorDirectionPrevious`).
    Previous,
}

/// Single accessible node in the iPadOS `UIAccessibility` element array.
#[derive(Debug, Clone, PartialEq)]
pub struct IosAccessibilityNode {
    /// Stable node identifier within the canvas accessibility tree.
    pub id: u64,
    /// Element classification.
    pub kind: AccessibilityElementKind,
    /// Zero-based page index if bound to a page.
    pub page_index: Option<usize>,
    /// VoiceOver `accessibilityLabel`.
    pub label: String,
    /// VoiceOver `accessibilityValue`.
    pub value: String,
    /// VoiceOver `accessibilityHint`.
    pub hint: String,
    /// Element bounds in view points.
    pub frame: Rect,
}

/// Result of a custom rotor lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct RotorNavigationOutcome {
    /// Node selected by the rotor step.
    pub node: IosAccessibilityNode,
    /// Zero-based index into the rotor's matching elements.
    pub match_index: usize,
}

/// `UIAccessibility` element tree and rotor model for an iPadOS document canvas.
#[derive(Debug, Clone, Default)]
pub struct IosAccessibilityTree {
    nodes: Vec<IosAccessibilityNode>,
    heading_ids: Vec<u64>,
}

impl IosAccessibilityTree {
    /// Rebuilds the `UIAccessibility` element tree from `layout`, `viewport`, and per-page metadata.
    #[must_use]
    pub fn rebuild(
        layout: &Layout,
        viewport: &Viewport,
        page_texts: &[String],
        links: &[(usize, String, Rect)],
        annotations: &[(usize, String, Rect)],
        headings: &[(usize, String)],
    ) -> Self {
        let mut nodes = Vec::new();
        let mut next_id = 1u64;

        let container_frame = Rect::new(0.0, 0.0, viewport.width(), viewport.height());
        nodes.push(IosAccessibilityNode {
            id: next_id,
            kind: AccessibilityElementKind::DocumentContainer,
            page_index: None,
            label: format!("Document with {} pages", layout.len()),
            value: String::new(),
            hint: "Use the rotor to move between pages, headings, links, and annotations"
                .to_owned(),
            frame: container_frame,
        });
        next_id += 1;

        for page_index in 0..layout.len() {
            let Some(page_rect) = layout.page(page_index) else {
                continue;
            };
            let screen = viewport.to_screen(&page_rect);
            let value = page_texts.get(page_index).cloned().unwrap_or_default();
            nodes.push(IosAccessibilityNode {
                id: next_id,
                kind: AccessibilityElementKind::PageElement,
                page_index: Some(page_index),
                label: format!("Page {}", page_index + 1),
                value,
                hint: "Swipe right to read page elements".to_owned(),
                frame: screen,
            });
            next_id += 1;
        }

        for &(page_index, ref target_label, frame) in links {
            nodes.push(IosAccessibilityNode {
                id: next_id,
                kind: AccessibilityElementKind::LinkElement,
                page_index: Some(page_index),
                label: target_label.clone(),
                value: format!("Page {}", page_index + 1),
                hint: "Double-tap to follow link".to_owned(),
                frame,
            });
            next_id += 1;
        }

        for &(page_index, ref note, frame) in annotations {
            nodes.push(IosAccessibilityNode {
                id: next_id,
                kind: AccessibilityElementKind::AnnotationElement,
                page_index: Some(page_index),
                label: format!("Annotation on page {}", page_index + 1),
                value: note.clone(),
                hint: "Double-tap to inspect annotation".to_owned(),
                frame,
            });
            next_id += 1;
        }

        let mut heading_ids = Vec::new();
        for &(page_index, ref _title) in headings {
            if let Some(page_node) = nodes.iter().find(|n| {
                n.kind == AccessibilityElementKind::PageElement && n.page_index == Some(page_index)
            }) {
                heading_ids.push(page_node.id);
            }
        }

        Self { nodes, heading_ids }
    }

    /// Returns all accessibility elements in reading order.
    #[must_use]
    pub fn elements(&self) -> &[IosAccessibilityNode] {
        &self.nodes
    }

    /// Returns the element count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns whether the tree has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Steps through a VoiceOver custom rotor from `current_id` in `direction`.
    #[must_use]
    pub fn rotor_step(
        &self,
        rotor: AccessibilityRotorKind,
        current_id: Option<u64>,
        direction: RotorDirection,
    ) -> Option<RotorNavigationOutcome> {
        let candidates: Vec<&IosAccessibilityNode> = match rotor {
            AccessibilityRotorKind::Pages => self
                .nodes
                .iter()
                .filter(|n| n.kind == AccessibilityElementKind::PageElement)
                .collect(),
            AccessibilityRotorKind::Links => self
                .nodes
                .iter()
                .filter(|n| n.kind == AccessibilityElementKind::LinkElement)
                .collect(),
            AccessibilityRotorKind::Annotations => self
                .nodes
                .iter()
                .filter(|n| n.kind == AccessibilityElementKind::AnnotationElement)
                .collect(),
            AccessibilityRotorKind::Headings => self
                .heading_ids
                .iter()
                .filter_map(|id| self.nodes.iter().find(|n| n.id == *id))
                .collect(),
        };
        if candidates.is_empty() {
            return None;
        }

        let next_idx = match current_id.and_then(|id| candidates.iter().position(|n| n.id == id)) {
            None => match direction {
                RotorDirection::Next => 0,
                RotorDirection::Previous => candidates.len() - 1,
            },
            Some(pos) => match direction {
                RotorDirection::Next => (pos + 1) % candidates.len(),
                RotorDirection::Previous => (pos + candidates.len() - 1) % candidates.len(),
            },
        };

        Some(RotorNavigationOutcome {
            node: candidates[next_idx].clone(),
            match_index: next_idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_canvas::layout::{PageSize, Spec};

    #[test]
    fn rotor_steps_through_pages_links_and_annotations() {
        let layout = Layout::new(
            &[
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
            ],
            Spec::default(),
        );
        let mut viewport = Viewport::new(2.0);
        viewport.resize(1200.0, 1600.0, &layout);

        let tree = IosAccessibilityTree::rebuild(
            &layout,
            &viewport,
            &["First page".to_owned(), "Second page".to_owned()],
            &[(0, "Section 2".to_owned(), Rect::new(20.0, 30.0, 80.0, 20.0))],
            &[(
                1,
                "Check figure".to_owned(),
                Rect::new(40.0, 50.0, 100.0, 30.0),
            )],
            &[(1, "Chapter 2".to_owned())],
        );

        let p1 = tree
            .rotor_step(AccessibilityRotorKind::Pages, None, RotorDirection::Next)
            .expect("first page");
        assert_eq!(p1.node.page_index, Some(0));
        let p2 = tree
            .rotor_step(
                AccessibilityRotorKind::Pages,
                Some(p1.node.id),
                RotorDirection::Next,
            )
            .expect("second page");
        assert_eq!(p2.node.page_index, Some(1));

        let link = tree
            .rotor_step(AccessibilityRotorKind::Links, None, RotorDirection::Next)
            .expect("link");
        assert_eq!(link.node.kind, AccessibilityElementKind::LinkElement);

        let annot = tree
            .rotor_step(
                AccessibilityRotorKind::Annotations,
                None,
                RotorDirection::Previous,
            )
            .expect("annotation");
        assert_eq!(annot.node.value, "Check figure");
    }
}
