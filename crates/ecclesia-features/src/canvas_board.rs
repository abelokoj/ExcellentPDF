//! Spatial research canvas board model, spatial index, argument-map groups/connectors,
//! cross-document evidence chain DAGs, and exporters (plan X.1–X.4).

pub mod evidence;
pub mod export;
pub mod spatial;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use ecclesia_core::anchor::{self, Placement, Quote};
use ecclesia_mupdf::{PagePoint, Quad};
use serde_json::{Value, json};

use self::spatial::SpatialGridIndex;

/// Errors returned by [`CanvasBoard`] operations and exports.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum BoardError {
    /// Referenced card id was not found on the board.
    CardNotFound(String),
    /// Referenced group id was not found on the board.
    GroupNotFound(String),
    /// Referenced connector id was not found on the board.
    ConnectorNotFound(String),
    /// Directed evidence graph contains a cycle among the listed card ids.
    CycleDetected(Vec<String>),
    /// Invalid input parameter or dimensions.
    InvalidInput(String),
    /// Malformed `.exboard` JSON or ZIP payload.
    InvalidFormat(String),
    /// Export encoding or archive failure.
    Export(String),
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CardNotFound(id) => write!(f, "board card not found: {id}"),
            Self::GroupNotFound(id) => write!(f, "board group not found: {id}"),
            Self::ConnectorNotFound(id) => write!(f, "board connector not found: {id}"),
            Self::CycleDetected(nodes) => {
                write!(f, "evidence chain cycle detected: {}", nodes.join(", "))
            }
            Self::InvalidInput(msg) => write!(f, "invalid board input: {msg}"),
            Self::InvalidFormat(msg) => write!(f, "invalid exboard format: {msg}"),
            Self::Export(msg) => write!(f, "board export failed: {msg}"),
        }
    }
}

impl std::error::Error for BoardError {}

/// Axis-aligned rectangle in infinite canvas world coordinates (points).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoardRect {
    /// Left X coordinate in world space.
    pub x: f64,
    /// Top Y coordinate in world space.
    pub y: f64,
    /// Width in world units.
    pub width: f64,
    /// Height in world units.
    pub height: f64,
}

impl BoardRect {
    /// Creates a normalized rectangle with non-negative `width` and `height`.
    #[must_use]
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
        .normalized()
    }

    /// Returns an equivalent rectangle with non-negative `width` and `height`.
    #[must_use]
    pub fn normalized(self) -> Self {
        let (x, width) = if self.width < 0.0 {
            (self.x + self.width, -self.width)
        } else {
            (self.x, self.width)
        };
        let (y, height) = if self.height < 0.0 {
            (self.y + self.height, -self.height)
        } else {
            (self.y, self.height)
        };
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Center point `(cx, cy)` in world coordinates.
    #[must_use]
    pub fn center(self) -> (f64, f64) {
        let n = self.normalized();
        (n.x + n.width * 0.5, n.y + n.height * 0.5)
    }

    /// Whether `self` and `other` overlap or touch.
    #[must_use]
    pub fn intersects(self, other: &Self) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.x <= b.x + b.width
            && a.x + a.width >= b.x
            && a.y <= b.y + b.height
            && a.y + a.height >= b.y
    }

    /// Whether world point `(px, py)` lies inside or on the boundary of `self`.
    #[must_use]
    pub fn contains_point(self, px: f64, py: f64) -> bool {
        let n = self.normalized();
        px >= n.x && px <= n.x + n.width && py >= n.y && py <= n.y + n.height
    }

    /// Smallest rectangle enclosing both `self` and `other`.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let a = self.normalized();
        let b = other.normalized();
        let min_x = a.x.min(b.x);
        let min_y = a.y.min(b.y);
        let max_x = (a.x + a.width).max(b.x + b.width);
        let max_y = (a.y + a.height).max(b.y + b.height);
        Self {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    /// Returns `self` translated by `(dx, dy)`.
    #[must_use]
    pub fn translated(self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            width: self.width,
            height: self.height,
        }
    }
}

/// Pan and zoom camera over the infinite canvas (plan X.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// World X coordinate at screen `(0, 0)`.
    pub pan_x: f64,
    /// World Y coordinate at screen `(0, 0)`.
    pub pan_y: f64,
    /// Scale factor (`1.0` = 100%, clamped to `0.05..=8.0`).
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// Minimum allowed zoom factor.
    pub const MIN_ZOOM: f64 = 0.05;
    /// Maximum allowed zoom factor.
    pub const MAX_ZOOM: f64 = 8.0;

    /// Pans the viewport by screen delta `(dx_px, dy_px)`.
    pub fn pan_by(&mut self, dx_px: f64, dy_px: f64) {
        self.pan_x -= dx_px / self.zoom.max(Self::MIN_ZOOM);
        self.pan_y -= dy_px / self.zoom.max(Self::MIN_ZOOM);
    }

    /// Zooms by `factor` keeping screen point `(anchor_x_px, anchor_y_px)` anchored in world space.
    pub fn zoom_at(&mut self, factor: f64, anchor_x_px: f64, anchor_y_px: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let (wx, wy) = self.screen_to_world(anchor_x_px, anchor_y_px);
        let new_zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.zoom = new_zoom;
        self.pan_x = wx - anchor_x_px / new_zoom;
        self.pan_y = wy - anchor_y_px / new_zoom;
    }

    /// Converts screen pixel coordinates to world coordinates.
    #[must_use]
    pub fn screen_to_world(self, screen_x: f64, screen_y: f64) -> (f64, f64) {
        let z = self.zoom.max(Self::MIN_ZOOM);
        (self.pan_x + screen_x / z, self.pan_y + screen_y / z)
    }

    /// Converts world coordinates to screen pixel coordinates.
    #[must_use]
    pub fn world_to_screen(self, world_x: f64, world_y: f64) -> (f64, f64) {
        (
            (world_x - self.pan_x) * self.zoom,
            (world_y - self.pan_y) * self.zoom,
        )
    }

    /// Returns the world rectangle visible inside a viewport of `width_px` × `height_px`.
    #[must_use]
    pub fn visible_world_rect(self, width_px: f64, height_px: f64) -> BoardRect {
        let z = self.zoom.max(Self::MIN_ZOOM);
        BoardRect::new(self.pan_x, self.pan_y, width_px / z, height_px / z)
    }
}

/// Link from a board item back to its exact PDF page, quads, and K.28 text quote anchor (plan X.1).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentAnchor {
    /// Path to the source PDF or document.
    pub document_path: PathBuf,
    /// Stable document identifier or content key.
    pub document_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Selection quads in page points.
    pub quads: Vec<Quad>,
    /// K.28 text-quote anchor used to re-locate the excerpt if the source is re-typeset.
    pub quote: Option<Quote>,
}

impl DocumentAnchor {
    /// Re-locates this excerpt's K.28 text-quote anchor inside updated document text `full_text`.
    #[must_use]
    pub fn relocate_in_text(&self, full_text: &str) -> Placement {
        match &self.quote {
            Some(quote) => anchor::locate(quote, full_text),
            None => Placement::Orphaned,
        }
    }
}

/// Argument-map semantic role of a board card (plan X.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ArgumentRole {
    /// Unclassified card or note.
    #[default]
    General,
    /// Thesis or claim node.
    Claim,
    /// Supporting argument or premise.
    Support,
    /// Counter-argument or objection.
    Objection,
    /// Primary empirical or textual evidence.
    Evidence,
    /// Open research question.
    Question,
}

/// Content payload of a [`BoardCard`] (plan X.1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CardKind {
    /// Text excerpt dragged from a PDF page with anchor quads and K.28 quote.
    TextExcerpt {
        /// Source PDF anchor.
        anchor: DocumentAnchor,
        /// Excerpt text.
        excerpt: String,
    },
    /// Raster or vector region snapshot dragged from a PDF page.
    RegionCrop {
        /// Source PDF anchor.
        anchor: DocumentAnchor,
        /// User caption.
        caption: String,
        /// Optional PNG bytes of the crop.
        png_bytes: Option<Vec<u8>>,
    },
    /// Freeform Markdown note card on the board.
    StickyNote {
        /// Markdown body.
        markdown: String,
    },
    /// Standalone image asset placed on the board.
    ImageAsset {
        /// MIME type (e.g. `"image/png"`).
        mime: String,
        /// Raw image bytes.
        bytes: Vec<u8>,
        /// Optional caption.
        caption: String,
    },
}

/// One card item on a [`CanvasBoard`].
#[derive(Debug, Clone, PartialEq)]
pub struct BoardCard {
    /// Unique card identifier.
    pub id: String,
    /// Card heading or short label.
    pub title: String,
    /// Position and size in world coordinates.
    pub bounds: BoardRect,
    /// Argument-map role.
    pub role: ArgumentRole,
    /// Card payload.
    pub kind: CardKind,
    /// Enclosing group id, if grouped.
    pub group_id: Option<String>,
    /// Optional hex accent color.
    pub color: Option<String>,
    /// Tags attached to this card.
    pub tags: Vec<String>,
}

impl BoardCard {
    /// Returns the source [`DocumentAnchor`], if this card links back to a document.
    #[must_use]
    pub fn anchor(&self) -> Option<&DocumentAnchor> {
        match &self.kind {
            CardKind::TextExcerpt { anchor, .. } | CardKind::RegionCrop { anchor, .. } => {
                Some(anchor)
            }
            CardKind::StickyNote { .. } | CardKind::ImageAsset { .. } => None,
        }
    }

    /// Returns a concise single-line summary of the card's content.
    #[must_use]
    pub fn summary_text(&self) -> String {
        match &self.kind {
            CardKind::TextExcerpt { excerpt, .. } => excerpt.clone(),
            CardKind::RegionCrop { caption, .. } => caption.clone(),
            CardKind::StickyNote { markdown } => markdown.clone(),
            CardKind::ImageAsset { caption, .. } => caption.clone(),
        }
    }

    /// Re-locates a document-backed card against `updated_text` when the source PDF is re-typeset.
    #[must_use]
    pub fn relocate_anchor(&self, updated_text: &str) -> Placement {
        self.anchor()
            .map_or(Placement::Orphaned, |a| a.relocate_in_text(updated_text))
    }
}

/// Visual placement of a group's title label (plan X.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum GroupLabelPosition {
    /// Above or inside top-left header.
    #[default]
    TopLeft,
    /// Centered along top edge.
    TopCenter,
}

/// Named visual group containing a set of board cards (plan X.2).
#[derive(Debug, Clone, PartialEq)]
pub struct BoardGroup {
    /// Unique group identifier.
    pub id: String,
    /// Group heading.
    pub title: String,
    /// Hex color string.
    pub color: String,
    /// Bounding box of the group container.
    pub bounds: BoardRect,
    /// Member card ids.
    pub card_ids: BTreeSet<String>,
    /// Header label alignment.
    pub label_position: GroupLabelPosition,
}

/// Semantic kind of a connector arrow between two board cards (plan X.2, X.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ConnectorKind {
    /// General association.
    #[default]
    Relates,
    /// `from_card` supports `to_card`.
    Supports,
    /// `from_card` objects to `to_card`.
    Objects,
    /// `from_card` derives from `to_card`.
    DerivesFrom,
    /// `from_card` cites source `to_card`.
    Cites,
}

/// Directed edge connecting two cards on the board (plan X.2).
#[derive(Debug, Clone, PartialEq)]
pub struct BoardConnector {
    /// Unique connector identifier.
    pub id: String,
    /// Source card id.
    pub from_card: String,
    /// Target card id.
    pub to_card: String,
    /// Optional edge label.
    pub label: String,
    /// Semantic edge kind.
    pub kind: ConnectorKind,
    /// Additional control-point curvature offset `(dx, dy)`.
    pub control_offset: (f64, f64),
}

/// Evaluated cubic Bezier curve geometry for a [`BoardConnector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    /// Start point on the source card's edge.
    pub start: (f64, f64),
    /// First Bezier control point.
    pub cp1: (f64, f64),
    /// Second Bezier control point.
    pub cp2: (f64, f64),
    /// End point on the target card's edge.
    pub end: (f64, f64),
}

impl CubicBezier {
    /// Evaluates the cubic Bezier curve at parameter `t` in `0.0..=1.0`.
    #[must_use]
    pub fn sample(self, t: f64) -> (f64, f64) {
        let u = 1.0 - t;
        let tt = t * t;
        let uu = u * u;
        let uuu = uu * u;
        let ttt = tt * t;
        let x = uuu * self.start.0
            + 3.0 * uu * t * self.cp1.0
            + 3.0 * u * tt * self.cp2.0
            + ttt * self.end.0;
        let y = uuu * self.start.1
            + 3.0 * uu * t * self.cp1.1
            + 3.0 * u * tt * self.cp2.1
            + ttt * self.end.1;
        (x, y)
    }

    /// Samples `segments + 1` points along the curve for raster rendering or hit-testing.
    #[must_use]
    pub fn polyline(self, segments: usize) -> Vec<(f64, f64)> {
        let segs = segments.max(1);
        (0..=segs)
            .map(|i| self.sample(i as f64 / segs as f64))
            .collect()
    }
}

/// Infinite spatial research canvas holding cards, groups, connectors, and a spatial index (plan X.1–X.4).
#[derive(Debug, Clone)]
pub struct CanvasBoard {
    id: String,
    title: String,
    viewport: Viewport,
    cards: BTreeMap<String, BoardCard>,
    groups: BTreeMap<String, BoardGroup>,
    connectors: BTreeMap<String, BoardConnector>,
    spatial: SpatialGridIndex,
}

impl CanvasBoard {
    /// Creates an empty board with `id` and `title`.
    #[must_use]
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_owned(),
            title: title.to_owned(),
            viewport: Viewport::default(),
            cards: BTreeMap::new(),
            groups: BTreeMap::new(),
            connectors: BTreeMap::new(),
            spatial: SpatialGridIndex::default(),
        }
    }

    /// Board identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Board title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Sets the board title.
    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_owned();
    }

    /// Current pan/zoom viewport.
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Mutable reference to the pan/zoom viewport.
    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    /// Number of cards on the board.
    #[must_use]
    pub fn card_count(&self) -> usize {
        self.cards.len()
    }

    /// Looks up a card by id.
    #[must_use]
    pub fn card(&self, id: &str) -> Option<&BoardCard> {
        self.cards.get(id)
    }

    /// Iterates over all cards in deterministic id order.
    pub fn cards(&self) -> impl Iterator<Item = &BoardCard> {
        self.cards.values()
    }

    /// Iterates over all card ids in deterministic order.
    pub fn card_ids(&self) -> impl Iterator<Item = &str> {
        self.cards.keys().map(String::as_str)
    }

    /// Looks up a group by id.
    #[must_use]
    pub fn group(&self, id: &str) -> Option<&BoardGroup> {
        self.groups.get(id)
    }

    /// Iterates over all groups in deterministic id order.
    pub fn groups(&self) -> impl Iterator<Item = &BoardGroup> {
        self.groups.values()
    }

    /// Iterates over all connectors in deterministic id order.
    pub fn connectors(&self) -> impl Iterator<Item = &BoardConnector> {
        self.connectors.values()
    }

    /// Inserts or replaces a [`BoardCard`] and updates the spatial index.
    pub fn upsert_card(&mut self, mut card: BoardCard) {
        card.bounds = card.bounds.normalized();
        self.spatial.upsert(&card.id, card.bounds);
        if let Some(gid) = &card.group_id
            && let Some(group) = self.groups.get_mut(gid)
        {
            group.card_ids.insert(card.id.clone());
        }
        self.cards.insert(card.id.clone(), card);
    }

    /// Moves a single card by `(dx, dy)` and keeps its spatial index entry updated.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::CardNotFound`] if `card_id` is absent.
    pub fn move_card(&mut self, card_id: &str, dx: f64, dy: f64) -> Result<(), BoardError> {
        let card = self
            .cards
            .get_mut(card_id)
            .ok_or_else(|| BoardError::CardNotFound(card_id.to_owned()))?;
        card.bounds = card.bounds.translated(dx, dy);
        let updated = card.bounds;
        self.spatial.upsert(card_id, updated);
        Ok(())
    }

    /// Removes a card and any connectors attached to it.
    pub fn remove_card(&mut self, card_id: &str) -> Option<BoardCard> {
        let removed = self.cards.remove(card_id)?;
        self.spatial.remove(card_id);
        if let Some(gid) = &removed.group_id
            && let Some(group) = self.groups.get_mut(gid)
        {
            group.card_ids.remove(card_id);
        }
        self.connectors
            .retain(|_, c| c.from_card != card_id && c.to_card != card_id);
        Some(removed)
    }

    /// Creates or updates a [`BoardGroup`] spanning the supplied `card_ids`.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::CardNotFound`] if any id in `card_ids` does not exist.
    pub fn upsert_group(
        &mut self,
        id: &str,
        title: &str,
        color: &str,
        card_ids: &[&str],
    ) -> Result<(), BoardError> {
        let mut members = BTreeSet::new();
        let mut union_bounds: Option<BoardRect> = None;
        for &cid in card_ids {
            let card = self
                .cards
                .get(cid)
                .ok_or_else(|| BoardError::CardNotFound(cid.to_owned()))?;
            members.insert(cid.to_owned());
            union_bounds = Some(match union_bounds {
                None => card.bounds,
                Some(acc) => acc.union(card.bounds),
            });
        }
        for cid in &members {
            if let Some(card) = self.cards.get_mut(cid) {
                card.group_id = Some(id.to_owned());
            }
        }
        let padded = union_bounds.map_or_else(
            || BoardRect::new(0.0, 0.0, 240.0, 180.0),
            |b| BoardRect::new(b.x - 16.0, b.y - 36.0, b.width + 32.0, b.height + 52.0),
        );
        self.groups.insert(
            id.to_owned(),
            BoardGroup {
                id: id.to_owned(),
                title: title.to_owned(),
                color: color.to_owned(),
                bounds: padded,
                card_ids: members,
                label_position: GroupLabelPosition::TopLeft,
            },
        );
        Ok(())
    }

    /// Moves a group and every card inside it by `(dx, dy)` while keeping connectors attached (plan X.2).
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::GroupNotFound`] if `group_id` is absent.
    pub fn move_group(&mut self, group_id: &str, dx: f64, dy: f64) -> Result<(), BoardError> {
        let member_ids: Vec<String> = {
            let group = self
                .groups
                .get_mut(group_id)
                .ok_or_else(|| BoardError::GroupNotFound(group_id.to_owned()))?;
            group.bounds = group.bounds.translated(dx, dy);
            group.card_ids.iter().cloned().collect()
        };
        for cid in member_ids {
            if let Some(card) = self.cards.get_mut(&cid) {
                card.bounds = card.bounds.translated(dx, dy);
                self.spatial.upsert(&cid, card.bounds);
            }
        }
        Ok(())
    }

    /// Connects `from_card` to `to_card` with a semantic [`ConnectorKind`] and label.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::CardNotFound`] if either endpoint does not exist.
    pub fn connect(&mut self, connector: BoardConnector) -> Result<(), BoardError> {
        if !self.cards.contains_key(&connector.from_card) {
            return Err(BoardError::CardNotFound(connector.from_card));
        }
        if !self.cards.contains_key(&connector.to_card) {
            return Err(BoardError::CardNotFound(connector.to_card));
        }
        self.connectors.insert(connector.id.clone(), connector);
        Ok(())
    }

    /// Computes the live cubic Bezier geometry for `connector_id` anchored to its endpoint cards' edges.
    #[must_use]
    pub fn connector_bezier(&self, connector_id: &str) -> Option<CubicBezier> {
        let conn = self.connectors.get(connector_id)?;
        let from = self.cards.get(&conn.from_card)?.bounds.normalized();
        let to = self.cards.get(&conn.to_card)?.bounds.normalized();
        let fc = from.center();
        let tc = to.center();
        let dx = tc.0 - fc.0;
        let dy = tc.1 - fc.1;

        let (start, end, horizontal) = if dx.abs() >= dy.abs() {
            let s_x = if dx >= 0.0 {
                from.x + from.width
            } else {
                from.x
            };
            let e_x = if dx >= 0.0 { to.x } else { to.x + to.width };
            ((s_x, fc.1), (e_x, tc.1), true)
        } else {
            let s_y = if dy >= 0.0 {
                from.y + from.height
            } else {
                from.y
            };
            let e_y = if dy >= 0.0 { to.y } else { to.y + to.height };
            ((fc.0, s_y), (tc.0, e_y), false)
        };

        let tension = if horizontal {
            (end.0 - start.0).abs() * 0.45
        } else {
            (end.1 - start.1).abs() * 0.45
        }
        .max(24.0);

        let (cp1, cp2) = if horizontal {
            let sign = if end.0 >= start.0 { 1.0 } else { -1.0 };
            (
                (
                    start.0 + sign * tension + conn.control_offset.0,
                    start.1 + conn.control_offset.1,
                ),
                (
                    end.0 - sign * tension + conn.control_offset.0,
                    end.1 + conn.control_offset.1,
                ),
            )
        } else {
            let sign = if end.1 >= start.1 { 1.0 } else { -1.0 };
            (
                (
                    start.0 + conn.control_offset.0,
                    start.1 + sign * tension + conn.control_offset.1,
                ),
                (
                    end.0 + conn.control_offset.0,
                    end.1 - sign * tension + conn.control_offset.1,
                ),
            )
        };

        Some(CubicBezier {
            start,
            cp1,
            cp2,
            end,
        })
    }

    /// Queries all cards visible in `rect` via the spatial index.
    #[must_use]
    pub fn query_rect(&self, rect: &BoardRect) -> Vec<&BoardCard> {
        self.spatial
            .query_rect(rect)
            .into_iter()
            .filter_map(|id| self.cards.get(&id))
            .collect()
    }

    /// Queries all cards visible in the current viewport for a screen size of `width_px` × `height_px`.
    #[must_use]
    pub fn query_viewport(&self, width_px: f64, height_px: f64) -> Vec<&BoardCard> {
        let rect = self.viewport.visible_world_rect(width_px, height_px);
        self.query_rect(&rect)
    }

    /// Hit-tests cards at world coordinates `(x, y)`.
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Vec<&BoardCard> {
        self.spatial
            .hit_test(x, y)
            .into_iter()
            .filter_map(|id| self.cards.get(&id))
            .collect()
    }

    /// World bounding box enclosing all cards on the board.
    #[must_use]
    pub fn world_bounds(&self) -> Option<BoardRect> {
        self.spatial.world_bounds()
    }

    /// Serializes the board to a standalone JSON string (`.exboard` JSON format).
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::Export`] if JSON formatting fails.
    pub fn to_json(&self) -> Result<String, BoardError> {
        let mut ignored = BTreeMap::new();
        self.serialize_json(&mut ignored, false)
    }

    /// Deserializes a board from JSON produced by [`Self::to_json`].
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::InvalidFormat`] if the JSON is malformed.
    pub fn from_json(input: &str) -> Result<Self, BoardError> {
        Self::from_json_with_assets(input, &BTreeMap::new())
    }

    pub(crate) fn to_json_with_asset_map(
        &self,
        assets: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<String, BoardError> {
        self.serialize_json(assets, true)
    }

    fn serialize_json(
        &self,
        assets: &mut BTreeMap<String, Vec<u8>>,
        externalize_assets: bool,
    ) -> Result<String, BoardError> {
        let cards_json: Vec<Value> = self
            .cards
            .values()
            .map(|c| card_to_json(c, assets, externalize_assets))
            .collect();
        let groups_json: Vec<Value> = self.groups.values().map(group_to_json).collect();
        let conns_json: Vec<Value> = self.connectors.values().map(connector_to_json).collect();

        let root = json!({
            "schema": 1,
            "id": self.id,
            "title": self.title,
            "viewport": {
                "pan_x": self.viewport.pan_x,
                "pan_y": self.viewport.pan_y,
                "zoom": self.viewport.zoom,
            },
            "cards": cards_json,
            "groups": groups_json,
            "connectors": conns_json,
        });
        serde_json::to_string_pretty(&root)
            .map_err(|e| BoardError::Export(format!("serialize board json: {e}")))
    }

    pub(crate) fn from_json_with_assets(
        input: &str,
        assets: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, BoardError> {
        let val: Value = serde_json::from_str(input)
            .map_err(|e| BoardError::InvalidFormat(format!("invalid board json: {e}")))?;
        let id = val
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| BoardError::InvalidFormat("missing board id".to_owned()))?;
        let title = val
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled Board");

        let mut board = Self::new(id, title);
        if let Some(vp) = val.get("viewport") {
            board.viewport = Viewport {
                pan_x: vp.get("pan_x").and_then(Value::as_f64).unwrap_or(0.0),
                pan_y: vp.get("pan_y").and_then(Value::as_f64).unwrap_or(0.0),
                zoom: vp
                    .get("zoom")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
                    .clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
            };
        }

        if let Some(arr) = val.get("cards").and_then(Value::as_array) {
            for item in arr {
                let card = card_from_json(item, assets)?;
                board.upsert_card(card);
            }
        }
        if let Some(arr) = val.get("groups").and_then(Value::as_array) {
            for item in arr {
                let group = group_from_json(item)?;
                board.groups.insert(group.id.clone(), group);
            }
        }
        if let Some(arr) = val.get("connectors").and_then(Value::as_array) {
            for item in arr {
                let conn = connector_from_json(item)?;
                if board.cards.contains_key(&conn.from_card)
                    && board.cards.contains_key(&conn.to_card)
                {
                    board.connectors.insert(conn.id.clone(), conn);
                }
            }
        }
        Ok(board)
    }
}

fn rect_to_json(r: BoardRect) -> Value {
    json!({ "x": r.x, "y": r.y, "width": r.width, "height": r.height })
}

fn rect_from_json(v: &Value) -> BoardRect {
    BoardRect::new(
        v.get("x").and_then(Value::as_f64).unwrap_or(0.0),
        v.get("y").and_then(Value::as_f64).unwrap_or(0.0),
        v.get("width").and_then(Value::as_f64).unwrap_or(200.0),
        v.get("height").and_then(Value::as_f64).unwrap_or(120.0),
    )
}

fn role_to_str(role: ArgumentRole) -> &'static str {
    match role {
        ArgumentRole::General => "general",
        ArgumentRole::Claim => "claim",
        ArgumentRole::Support => "support",
        ArgumentRole::Objection => "objection",
        ArgumentRole::Evidence => "evidence",
        ArgumentRole::Question => "question",
    }
}

fn role_from_str(s: &str) -> ArgumentRole {
    match s {
        "claim" => ArgumentRole::Claim,
        "support" => ArgumentRole::Support,
        "objection" => ArgumentRole::Objection,
        "evidence" => ArgumentRole::Evidence,
        "question" => ArgumentRole::Question,
        _ => ArgumentRole::General,
    }
}

fn connector_kind_to_str(kind: ConnectorKind) -> &'static str {
    match kind {
        ConnectorKind::Relates => "relates",
        ConnectorKind::Supports => "supports",
        ConnectorKind::Objects => "objects",
        ConnectorKind::DerivesFrom => "derives_from",
        ConnectorKind::Cites => "cites",
    }
}

fn connector_kind_from_str(s: &str) -> ConnectorKind {
    match s {
        "supports" => ConnectorKind::Supports,
        "objects" => ConnectorKind::Objects,
        "derives_from" => ConnectorKind::DerivesFrom,
        "cites" => ConnectorKind::Cites,
        _ => ConnectorKind::Relates,
    }
}

fn anchor_to_json(a: &DocumentAnchor) -> Value {
    let quads: Vec<Value> = a
        .quads
        .iter()
        .map(|q| {
            json!({
                "ul": [q.ul.x, q.ul.y],
                "ur": [q.ur.x, q.ur.y],
                "ll": [q.ll.x, q.ll.y],
                "lr": [q.lr.x, q.lr.y],
            })
        })
        .collect();
    let quote = a.quote.as_ref().map(|q| {
        json!({
            "prefix": q.prefix,
            "exact": q.exact,
            "suffix": q.suffix,
            "start": q.start,
        })
    });
    json!({
        "document_path": a.document_path.to_string_lossy(),
        "document_id": a.document_id,
        "page": a.page,
        "quads": quads,
        "quote": quote,
    })
}

fn anchor_from_json(v: &Value) -> Result<DocumentAnchor, BoardError> {
    let doc_path = v
        .get("document_path")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("anchor missing document_path".to_owned()))?;
    let doc_id = v
        .get("document_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let page = v
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|p| usize::try_from(p).ok())
        .unwrap_or(0);
    let mut quads = Vec::new();
    if let Some(arr) = v.get("quads").and_then(Value::as_array) {
        for q in arr {
            let pt = |k: &str| -> PagePoint {
                let coords = q.get(k).and_then(Value::as_array);
                let x = coords
                    .and_then(|c| c.first())
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                let y = coords
                    .and_then(|c| c.get(1))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                PagePoint { x, y }
            };
            quads.push(Quad {
                ul: pt("ul"),
                ur: pt("ur"),
                ll: pt("ll"),
                lr: pt("lr"),
            });
        }
    }
    let quote = v.get("quote").and_then(|q| {
        if q.is_null() {
            return None;
        }
        Some(Quote {
            prefix: q.get("prefix")?.as_str()?.to_owned(),
            exact: q.get("exact")?.as_str()?.to_owned(),
            suffix: q.get("suffix")?.as_str()?.to_owned(),
            start: usize::try_from(q.get("start")?.as_u64()?).ok()?,
        })
    });
    Ok(DocumentAnchor {
        document_path: PathBuf::from(doc_path),
        document_id: doc_id,
        page,
        quads,
        quote,
    })
}

fn card_to_json(
    card: &BoardCard,
    assets: &mut BTreeMap<String, Vec<u8>>,
    externalize_assets: bool,
) -> Value {
    use base64::Engine as _;
    let payload = match &card.kind {
        CardKind::TextExcerpt { anchor, excerpt } => json!({
            "type": "text_excerpt",
            "anchor": anchor_to_json(anchor),
            "excerpt": excerpt,
        }),
        CardKind::RegionCrop {
            anchor,
            caption,
            png_bytes,
        } => {
            let asset_ref = if externalize_assets {
                if let Some(bytes) = png_bytes {
                    let name = format!("{}.png", card.id);
                    assets.insert(name.clone(), bytes.clone());
                    json!({ "asset": name })
                } else {
                    Value::Null
                }
            } else {
                png_bytes
                    .as_ref()
                    .map(|b| json!({ "base64": base64::engine::general_purpose::STANDARD.encode(b) }))
                    .unwrap_or(Value::Null)
            };
            json!({
                "type": "region_crop",
                "anchor": anchor_to_json(anchor),
                "caption": caption,
                "image": asset_ref,
            })
        }
        CardKind::StickyNote { markdown } => json!({
            "type": "sticky_note",
            "markdown": markdown,
        }),
        CardKind::ImageAsset {
            mime,
            bytes,
            caption,
        } => {
            let image_val = if externalize_assets {
                let name = format!("{}.bin", card.id);
                assets.insert(name.clone(), bytes.clone());
                json!({ "asset": name })
            } else {
                json!({ "base64": base64::engine::general_purpose::STANDARD.encode(bytes) })
            };
            json!({
                "type": "image_asset",
                "mime": mime,
                "caption": caption,
                "image": image_val,
            })
        }
    };
    json!({
        "id": card.id,
        "title": card.title,
        "bounds": rect_to_json(card.bounds),
        "role": role_to_str(card.role),
        "kind": payload,
        "group_id": card.group_id,
        "color": card.color,
        "tags": card.tags,
    })
}

fn decode_image_field(
    v: &Value,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Vec<u8>>, BoardError> {
    use base64::Engine as _;
    if v.is_null() {
        return Ok(None);
    }
    if let Some(asset_name) = v.get("asset").and_then(Value::as_str) {
        return Ok(assets.get(asset_name).cloned());
    }
    if let Some(b64) = v.get("base64").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| BoardError::InvalidFormat(format!("invalid base64 image: {e}")))?;
        return Ok(Some(bytes));
    }
    Ok(None)
}

fn card_from_json(v: &Value, assets: &BTreeMap<String, Vec<u8>>) -> Result<BoardCard, BoardError> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("card missing id".to_owned()))?
        .to_owned();
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let bounds = v.get("bounds").map(rect_from_json).unwrap_or_default();
    let role = role_from_str(v.get("role").and_then(Value::as_str).unwrap_or("general"));
    let kobj = v
        .get("kind")
        .ok_or_else(|| BoardError::InvalidFormat(format!("card {id} missing kind")))?;
    let kind_type = kobj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat(format!("card {id} missing kind.type")))?;

    let kind = match kind_type {
        "text_excerpt" => CardKind::TextExcerpt {
            anchor: anchor_from_json(kobj.get("anchor").unwrap_or(&Value::Null))?,
            excerpt: kobj
                .get("excerpt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
        "region_crop" => CardKind::RegionCrop {
            anchor: anchor_from_json(kobj.get("anchor").unwrap_or(&Value::Null))?,
            caption: kobj
                .get("caption")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            png_bytes: decode_image_field(kobj.get("image").unwrap_or(&Value::Null), assets)?,
        },
        "sticky_note" => CardKind::StickyNote {
            markdown: kobj
                .get("markdown")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
        "image_asset" => CardKind::ImageAsset {
            mime: kobj
                .get("mime")
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_owned(),
            bytes: decode_image_field(kobj.get("image").unwrap_or(&Value::Null), assets)?
                .unwrap_or_default(),
            caption: kobj
                .get("caption")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
        other => {
            return Err(BoardError::InvalidFormat(format!(
                "unknown card kind {other}"
            )));
        }
    };

    let tags = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    Ok(BoardCard {
        id,
        title,
        bounds,
        role,
        kind,
        group_id: v.get("group_id").and_then(Value::as_str).map(str::to_owned),
        color: v.get("color").and_then(Value::as_str).map(str::to_owned),
        tags,
    })
}

fn group_to_json(g: &BoardGroup) -> Value {
    let ids: Vec<&str> = g.card_ids.iter().map(String::as_str).collect();
    json!({
        "id": g.id,
        "title": g.title,
        "color": g.color,
        "bounds": rect_to_json(g.bounds),
        "card_ids": ids,
    })
}

fn group_from_json(v: &Value) -> Result<BoardGroup, BoardError> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("group missing id".to_owned()))?
        .to_owned();
    let card_ids = v
        .get("card_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(BoardGroup {
        id,
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        color: v
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("#64748b")
            .to_owned(),
        bounds: v.get("bounds").map(rect_from_json).unwrap_or_default(),
        card_ids,
        label_position: GroupLabelPosition::TopLeft,
    })
}

fn connector_to_json(c: &BoardConnector) -> Value {
    json!({
        "id": c.id,
        "from_card": c.from_card,
        "to_card": c.to_card,
        "label": c.label,
        "kind": connector_kind_to_str(c.kind),
        "control_offset": [c.control_offset.0, c.control_offset.1],
    })
}

fn connector_from_json(v: &Value) -> Result<BoardConnector, BoardError> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("connector missing id".to_owned()))?
        .to_owned();
    let from_card = v
        .get("from_card")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("connector missing from_card".to_owned()))?
        .to_owned();
    let to_card = v
        .get("to_card")
        .and_then(Value::as_str)
        .ok_or_else(|| BoardError::InvalidFormat("connector missing to_card".to_owned()))?
        .to_owned();
    let off = v.get("control_offset").and_then(Value::as_array);
    let ox = off
        .and_then(|a| a.first())
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let oy = off
        .and_then(|a| a.get(1))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Ok(BoardConnector {
        id,
        from_card,
        to_card,
        label: v
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        kind: connector_kind_from_str(v.get("kind").and_then(Value::as_str).unwrap_or("relates")),
        control_offset: (ox, oy),
    })
}

#[cfg(test)]
mod tests {
    use super::evidence::{
        export_evidence_chains_markdown, query_evidence_chains, validate_evidence_dag,
    };
    use super::export::{
        export_board_markdown, export_board_outline, export_board_png_summary, export_board_svg,
        export_exboard_zip, import_exboard_zip,
    };
    use super::*;

    fn sample_anchor(doc: &str, page: usize, text: &str) -> DocumentAnchor {
        let quote = Quote::of(text, 0..text.len());
        DocumentAnchor {
            document_path: PathBuf::from(doc),
            document_id: format!("id-{doc}"),
            page,
            quads: vec![Quad {
                ul: PagePoint { x: 72.0, y: 100.0 },
                ur: PagePoint { x: 300.0, y: 100.0 },
                ll: PagePoint { x: 72.0, y: 114.0 },
                lr: PagePoint { x: 300.0, y: 114.0 },
            }],
            quote,
        }
    }

    #[test]
    fn spatial_index_handles_1500_cards_and_relocates_retypeset_excerpt() {
        let mut board = CanvasBoard::new("b1", "Research Matrix");
        for i in 0..1500 {
            let col = i % 50;
            let row = i / 50;
            board.upsert_card(BoardCard {
                id: format!("c-{i:04}"),
                title: format!("Excerpt {i}"),
                bounds: BoardRect::new(
                    f64::from(col) * 260.0,
                    f64::from(row) * 180.0,
                    220.0,
                    140.0,
                ),
                role: ArgumentRole::Evidence,
                kind: CardKind::TextExcerpt {
                    anchor: sample_anchor(
                        "papers/attention.pdf",
                        (i as usize) % 12,
                        "Scaled dot-product attention computes weights via softmax.",
                    ),
                    excerpt: "Scaled dot-product attention computes weights via softmax."
                        .to_owned(),
                },
                group_id: None,
                color: None,
                tags: vec!["transformer".to_owned()],
            });
        }
        assert_eq!(board.card_count(), 1500);

        let visible = board.query_rect(&BoardRect::new(0.0, 0.0, 500.0, 350.0));
        assert_eq!(visible.len(), 4, "2x2 cards visible in viewport");

        let retypeset_text = "Section 3.2 (Revised Edition)\n\nScaled dot-\nproduct attention computes weights via softmax.\nAdditional appendix.";
        let placed = visible[0].relocate_anchor(retypeset_text);
        assert!(
            matches!(placed, Placement::Exact(_) | Placement::Fuzzy { .. }),
            "re-typeset document still locates excerpt: {placed:?}"
        );
    }

    #[test]
    fn moving_group_moves_items_and_keeps_bezier_connectors_attached() {
        let mut board = CanvasBoard::new("b2", "Argument Map");
        board.upsert_card(BoardCard {
            id: "claim".to_owned(),
            title: "Main Claim".to_owned(),
            bounds: BoardRect::new(400.0, 100.0, 200.0, 100.0),
            role: ArgumentRole::Claim,
            kind: CardKind::StickyNote {
                markdown: "CRDTs guarantee convergence.".to_owned(),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board.upsert_card(BoardCard {
            id: "sup".to_owned(),
            title: "RGA Proof".to_owned(),
            bounds: BoardRect::new(80.0, 100.0, 200.0, 100.0),
            role: ArgumentRole::Support,
            kind: CardKind::TextExcerpt {
                anchor: sample_anchor(
                    "docs/crdt.pdf",
                    4,
                    "RGA orders concurrent siblings by Lamport id.",
                ),
                excerpt: "RGA orders concurrent siblings by Lamport id.".to_owned(),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board
            .upsert_group("g1", "Core Proofs", "#16a34a", &["sup"])
            .expect("create group");
        board
            .connect(BoardConnector {
                id: "e1".to_owned(),
                from_card: "sup".to_owned(),
                to_card: "claim".to_owned(),
                label: "proves".to_owned(),
                kind: ConnectorKind::Supports,
                control_offset: (0.0, 0.0),
            })
            .expect("connect");

        let before = board.connector_bezier("e1").expect("bezier before");
        board.move_group("g1", 40.0, 60.0).expect("move group");

        let moved_sup = board.card("sup").expect("sup exists");
        assert_eq!(moved_sup.bounds.x, 120.0);
        assert_eq!(moved_sup.bounds.y, 160.0);

        let after = board.connector_bezier("e1").expect("bezier after");
        assert!(
            (after.start.0 - (before.start.0 + 40.0)).abs() < 1e-6
                && (after.start.1 - (before.start.1 + 60.0)).abs() < 1e-6,
            "arrow start moved cleanly with group card"
        );
        assert_eq!(after.end, before.end, "unmoved target end stayed attached");
    }

    #[test]
    fn evidence_chains_validate_dag_and_export_cross_doc_citations() {
        let mut board = CanvasBoard::new("b3", "Cross-Doc Evidence");
        board.upsert_card(BoardCard {
            id: "c1".to_owned(),
            title: "Sub-ms Latency".to_owned(),
            bounds: BoardRect::new(400.0, 40.0, 200.0, 90.0),
            role: ArgumentRole::Claim,
            kind: CardKind::StickyNote {
                markdown: "Local cache keeps page turns < 16ms".to_owned(),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board.upsert_card(BoardCard {
            id: "ev1".to_owned(),
            title: "Bench Table".to_owned(),
            bounds: BoardRect::new(100.0, 40.0, 200.0, 90.0),
            role: ArgumentRole::Evidence,
            kind: CardKind::TextExcerpt {
                anchor: sample_anchor("bench/report.pdf", 2, "P99 tile decode is 4.2 ms."),
                excerpt: "P99 tile decode is 4.2 ms.".to_owned(),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board
            .connect(BoardConnector {
                id: "edge1".to_owned(),
                from_card: "ev1".to_owned(),
                to_card: "c1".to_owned(),
                label: "supports".to_owned(),
                kind: ConnectorKind::Supports,
                control_offset: (0.0, 0.0),
            })
            .expect("connect");

        assert!(validate_evidence_dag(&board).is_ok());
        let chains = query_evidence_chains(&board, "c1").expect("chains");
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].sources.len(), 1);
        assert_eq!(chains[0].sources[0].page_one_based, 3);

        let md = export_evidence_chains_markdown(&board).expect("export md");
        assert!(md.contains("bench/report.pdf"));
        assert!(md.contains("ecclesiapdf://open?"));

        // Introduce a cycle and verify rejection.
        board
            .connect(BoardConnector {
                id: "cycle".to_owned(),
                from_card: "c1".to_owned(),
                to_card: "ev1".to_owned(),
                label: "circular".to_owned(),
                kind: ConnectorKind::DerivesFrom,
                control_offset: (0.0, 0.0),
            })
            .expect("add cycle edge");
        // Wait: c1->ev1 DerivesFrom adds edge c1->ev1, while ev1->c1 Supports also means c1->ev1;
        // add ev1->c1 DerivesFrom to form a true 2-cycle.
        board
            .connect(BoardConnector {
                id: "cycle2".to_owned(),
                from_card: "ev1".to_owned(),
                to_card: "c1".to_owned(),
                label: "circular back".to_owned(),
                kind: ConnectorKind::DerivesFrom,
                control_offset: (0.0, 0.0),
            })
            .expect("add reverse edge");
        assert!(matches!(
            validate_evidence_dag(&board),
            Err(BoardError::CycleDetected(_))
        ));
    }

    #[test]
    fn board_exports_svg_png_markdown_outline_and_exboard_zip_roundtrip() {
        let mut board = CanvasBoard::new("b4", "Export Board");
        board.upsert_card(BoardCard {
            id: "crop1".to_owned(),
            title: "Figure 1".to_owned(),
            bounds: BoardRect::new(20.0, 20.0, 220.0, 140.0),
            role: ArgumentRole::Evidence,
            kind: CardKind::RegionCrop {
                anchor: sample_anchor("docs/fig.pdf", 0, "Architecture diagram"),
                caption: "Pipeline".to_owned(),
                png_bytes: Some(vec![137, 80, 78, 71, 13, 10, 26, 10]),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board.upsert_card(BoardCard {
            id: "note1".to_owned(),
            title: "Synthesis".to_owned(),
            bounds: BoardRect::new(320.0, 40.0, 200.0, 110.0),
            role: ArgumentRole::Claim,
            kind: CardKind::StickyNote {
                markdown: "Key takeaway".to_owned(),
            },
            group_id: None,
            color: None,
            tags: Vec::new(),
        });
        board
            .connect(BoardConnector {
                id: "k1".to_owned(),
                from_card: "crop1".to_owned(),
                to_card: "note1".to_owned(),
                label: "evidence".to_owned(),
                kind: ConnectorKind::Supports,
                control_offset: (0.0, 0.0),
            })
            .expect("connect");

        let svg = export_board_svg(&board);
        assert!(svg.starts_with("<svg") && svg.contains("Figure 1") && svg.contains("<path"));

        let png = export_board_png_summary(&board, 320, 200).expect("png summary");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let md = export_board_markdown(&board);
        assert!(md.contains("ecclesiapdf://open?") && md.contains("Synthesis"));

        let outline = export_board_outline(&board);
        assert!(outline.contains("Export Board") && outline.contains("Figure 1"));

        let zip_bytes = export_exboard_zip(&board).expect("export zip");
        let restored = import_exboard_zip(&zip_bytes).expect("import zip");
        assert_eq!(restored.card_count(), 2);
        assert_eq!(restored.card("crop1").expect("crop1").title, "Figure 1");
        match &restored.card("crop1").expect("crop1").kind {
            CardKind::RegionCrop { png_bytes, .. } => {
                assert_eq!(
                    png_bytes.as_deref(),
                    Some([137, 80, 78, 71, 13, 10, 26, 10].as_slice())
                );
            }
            other => panic!("unexpected card kind: {other:?}"),
        }
    }
}
