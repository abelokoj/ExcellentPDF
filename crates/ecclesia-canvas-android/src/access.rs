//! Android `AccessibilityNodeProvider` virtual node tree for TalkBack on tablets (plan U.3).
//!
//! Maps the PDF canvas to a virtual node hierarchy (`HOST_VIEW_ID = -1`, document root, per-page nodes,
//! link buttons, and annotation notes), supports `findAccessibilityNodeInfosByText`, and dispatches
//! `performAction` (`ACTION_ACCESSIBILITY_FOCUS`, `ACTION_CLICK`, `ACTION_SCROLL_FORWARD/BACKWARD`).

use ecclesia_canvas::Rect;
use ecclesia_canvas::layout::Layout;
use ecclesia_canvas::viewport::Viewport;

/// Android `AccessibilityNodeProvider.HOST_VIEW_ID` sentinel (`-1`).
pub const HOST_VIEW_ID: i32 = -1;

/// Semantic role of a virtual node in the Android `AccessibilityNodeProvider` tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AndroidNodeRole {
    /// Root virtual container representing the open PDF document.
    RootDocument,
    /// Individual page node exposing reading-order text.
    Page,
    /// Hyperlink node supporting `ACTION_CLICK`.
    Link,
    /// Annotation note node.
    Annotation,
}

/// Android `AccessibilityNodeInfo` action subset supported by the virtual tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AndroidAccessibilityAction {
    /// `ACTION_ACCESSIBILITY_FOCUS` (`64`).
    AccessibilityFocus,
    /// `ACTION_CLEAR_ACCESSIBILITY_FOCUS` (`128`).
    ClearAccessibilityFocus,
    /// `ACTION_CLICK` (`16`).
    Click,
    /// `ACTION_SCROLL_FORWARD` (`4096`).
    ScrollForward,
    /// `ACTION_SCROLL_BACKWARD` (`8192`).
    ScrollBackward,
}

/// Outcome of dispatching an [`AndroidAccessibilityAction`] on a virtual node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionOutcome {
    /// Action handled; accessibility focus moved to the specified virtual node ID.
    FocusChanged(Option<i32>),
    /// Link or page element activated (`page_index`, `target_label`).
    Activated {
        /// Zero-based page index.
        page_index: usize,
        /// Target label or URI.
        target: String,
    },
    /// Scroll requested (`+1` forward, `-1` backward).
    Scrolled(i32),
    /// Virtual node or action was not applicable.
    Ignored,
}

/// One virtual `AccessibilityNodeInfo` descriptor in the Android canvas tree.
#[derive(Debug, Clone, PartialEq)]
pub struct AndroidAccessibilityNode {
    /// Virtual view ID (`HOST_VIEW_ID` for root, positive integers for descendants).
    pub virtual_view_id: i32,
    /// Parent virtual view ID (`HOST_VIEW_ID` for top-level children).
    pub parent_id: i32,
    /// Child virtual view IDs in reading order.
    pub child_ids: Vec<i32>,
    /// Semantic role.
    pub role: AndroidNodeRole,
    /// Android widget class name (`android.view.View`, `android.widget.TextView`, `android.widget.Button`).
    pub class_name: &'static str,
    /// Spoken `contentDescription`.
    pub content_description: String,
    /// Accessible text payload.
    pub text: String,
    /// Bounds in screen coordinates (pixels).
    pub bounds_in_screen: Rect,
    /// Zero-based page index if attached to a page.
    pub page_index: Option<usize>,
    /// Whether `ACTION_CLICK` is supported.
    pub clickable: bool,
    /// Whether `ACTION_ACCESSIBILITY_FOCUS` is supported.
    pub focusable: bool,
}

/// Virtual accessibility tree backing Kotlin `CanvasAccessibilityProvider`.
#[derive(Debug, Clone, Default)]
pub struct AndroidAccessibilityTree {
    nodes: Vec<AndroidAccessibilityNode>,
    focused_id: Option<i32>,
}

impl AndroidAccessibilityTree {
    /// Rebuilds the virtual `AccessibilityNodeProvider` tree from `layout`, `viewport`, and page data.
    #[must_use]
    pub fn rebuild(
        layout: &Layout,
        viewport: &Viewport,
        page_texts: &[String],
        links: &[(usize, String, Rect)],
        annotations: &[(usize, String, Rect)],
    ) -> Self {
        let mut nodes = Vec::new();
        let mut root_children = Vec::new();
        let mut next_id = 1i32;

        // Allocate root first, populate children afterwards.
        nodes.push(AndroidAccessibilityNode {
            virtual_view_id: HOST_VIEW_ID,
            parent_id: HOST_VIEW_ID,
            child_ids: Vec::new(),
            role: AndroidNodeRole::RootDocument,
            class_name: "android.view.View",
            content_description: format!("PDF document, {} pages", layout.len()),
            text: String::new(),
            bounds_in_screen: Rect::new(0.0, 0.0, viewport.width(), viewport.height()),
            page_index: None,
            clickable: false,
            focusable: true,
        });

        for page_index in 0..layout.len() {
            let Some(page_rect) = layout.page(page_index) else {
                continue;
            };
            let page_id = next_id;
            next_id += 1;
            root_children.push(page_id);

            let mut page_children = Vec::new();
            let screen = viewport.to_screen(&page_rect);
            let page_text = page_texts.get(page_index).cloned().unwrap_or_default();

            let page_pos = nodes.len();
            nodes.push(AndroidAccessibilityNode {
                virtual_view_id: page_id,
                parent_id: HOST_VIEW_ID,
                child_ids: Vec::new(),
                role: AndroidNodeRole::Page,
                class_name: "android.widget.TextView",
                content_description: format!("Page {}", page_index + 1),
                text: page_text,
                bounds_in_screen: screen,
                page_index: Some(page_index),
                clickable: false,
                focusable: true,
            });

            for &(link_page, ref target, frame) in links {
                if link_page != page_index {
                    continue;
                }
                let link_id = next_id;
                next_id += 1;
                page_children.push(link_id);
                nodes.push(AndroidAccessibilityNode {
                    virtual_view_id: link_id,
                    parent_id: page_id,
                    child_ids: Vec::new(),
                    role: AndroidNodeRole::Link,
                    class_name: "android.widget.Button",
                    content_description: format!("Link: {target}"),
                    text: target.clone(),
                    bounds_in_screen: frame,
                    page_index: Some(page_index),
                    clickable: true,
                    focusable: true,
                });
            }

            for &(annot_page, ref note, frame) in annotations {
                if annot_page != page_index {
                    continue;
                }
                let annot_id = next_id;
                next_id += 1;
                page_children.push(annot_id);
                nodes.push(AndroidAccessibilityNode {
                    virtual_view_id: annot_id,
                    parent_id: page_id,
                    child_ids: Vec::new(),
                    role: AndroidNodeRole::Annotation,
                    class_name: "android.widget.TextView",
                    content_description: format!("Annotation on page {}", page_index + 1),
                    text: note.clone(),
                    bounds_in_screen: frame,
                    page_index: Some(page_index),
                    clickable: true,
                    focusable: true,
                });
            }

            nodes[page_pos].child_ids = page_children;
        }

        nodes[0].child_ids = root_children;
        Self {
            nodes,
            focused_id: None,
        }
    }

    /// Returns the node with `virtual_view_id` (`HOST_VIEW_ID = -1` for root).
    #[must_use]
    pub fn node(&self, virtual_view_id: i32) -> Option<&AndroidAccessibilityNode> {
        self.nodes
            .iter()
            .find(|n| n.virtual_view_id == virtual_view_id)
    }

    /// Returns all nodes in the tree.
    #[must_use]
    pub fn nodes(&self) -> &[AndroidAccessibilityNode] {
        &self.nodes
    }

    /// Returns the currently focused virtual view ID, if any.
    #[must_use]
    pub const fn focused_id(&self) -> Option<i32> {
        self.focused_id
    }

    /// Searches nodes matching `query` case-insensitively (`findAccessibilityNodeInfosByText`).
    #[must_use]
    pub fn find_by_text(&self, query: &str) -> Vec<&AndroidAccessibilityNode> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.nodes
            .iter()
            .filter(|node| {
                node.text.to_lowercase().contains(&needle)
                    || node.content_description.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Executes `action` on `virtual_view_id`.
    pub fn perform_action(
        &mut self,
        virtual_view_id: i32,
        action: AndroidAccessibilityAction,
    ) -> ActionOutcome {
        let Some(node) = self.node(virtual_view_id).cloned() else {
            return ActionOutcome::Ignored;
        };
        match action {
            AndroidAccessibilityAction::AccessibilityFocus => {
                if node.focusable {
                    self.focused_id = Some(virtual_view_id);
                    ActionOutcome::FocusChanged(self.focused_id)
                } else {
                    ActionOutcome::Ignored
                }
            }
            AndroidAccessibilityAction::ClearAccessibilityFocus => {
                if self.focused_id == Some(virtual_view_id) {
                    self.focused_id = None;
                }
                ActionOutcome::FocusChanged(self.focused_id)
            }
            AndroidAccessibilityAction::Click => {
                if node.clickable {
                    ActionOutcome::Activated {
                        page_index: node.page_index.unwrap_or(0),
                        target: if node.text.is_empty() {
                            node.content_description
                        } else {
                            node.text
                        },
                    }
                } else {
                    ActionOutcome::Ignored
                }
            }
            AndroidAccessibilityAction::ScrollForward => ActionOutcome::Scrolled(1),
            AndroidAccessibilityAction::ScrollBackward => ActionOutcome::Scrolled(-1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_canvas::layout::{PageSize, Spec};

    #[test]
    fn accessibility_node_provider_tree_resolves_hierarchy_search_and_click() {
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

        let mut tree = AndroidAccessibilityTree::rebuild(
            &layout,
            &viewport,
            &[
                "Introduction to Homography".to_owned(),
                "Appendix results".to_owned(),
            ],
            &[(
                0,
                "https://example.org".to_owned(),
                Rect::new(10.0, 20.0, 100.0, 24.0),
            )],
            &[(
                1,
                "Review footnote".to_owned(),
                Rect::new(30.0, 40.0, 120.0, 40.0),
            )],
        );

        let root = tree.node(HOST_VIEW_ID).expect("host root");
        assert_eq!(root.child_ids.len(), 2);

        let hits = tree.find_by_text("homography");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page_index, Some(0));

        let link_id = tree.node(root.child_ids[0]).expect("page 0").child_ids[0];
        assert_eq!(
            tree.perform_action(link_id, AndroidAccessibilityAction::AccessibilityFocus),
            ActionOutcome::FocusChanged(Some(link_id))
        );
        assert_eq!(
            tree.perform_action(link_id, AndroidAccessibilityAction::Click),
            ActionOutcome::Activated {
                page_index: 0,
                target: "https://example.org".to_owned(),
            }
        );
    }
}
