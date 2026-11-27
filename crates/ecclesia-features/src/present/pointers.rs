//! Same-frame presentation canvas overlays: laser with trails (G.2), spotlight (G.3), and
//! magnifier lens (G.4).
//!
//! All three operate purely as canvas overlays (never triggering PDF page tile re-renders),
//! tracking pointer updates in the exact same frame. Laser width normalization and trail expiry
//! match SumatraPDF Enhanced's `Canvas.cpp` rules.

use ecclesia_mupdf::{PagePoint, PageRect};

/// Maximum points stored across active laser trails (matching SumatraPDF Enhanced `kLaserTrailMaxPoints`).
pub const LASER_TRAIL_MAX_POINTS: usize = 65_536;
/// Default laser dot diameter in points (matching SumatraPDF Enhanced).
pub const DEFAULT_LASER_WIDTH: f32 = 8.0;
/// Default laser trail lifetime in milliseconds (2 seconds, matching SumatraPDF Enhanced).
pub const DEFAULT_LASER_LIFETIME_MS: u64 = 2_000;

/// Active pointer overlay tool during a presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActivePointerTool {
    /// Normal pointer (no overlay).
    #[default]
    None,
    /// Laser dot with fading trail (G.2).
    Laser,
    /// Spotlight dimming everything outside a region (G.3).
    Spotlight,
    /// Magnifier lens zooming into the region under the pointer (G.4).
    Magnifier,
}

/// Visual rendering mode of the laser dot (ported from SumatraPDF Enhanced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LaserMode {
    /// Solid white-hot center inside a glowing colored core.
    #[default]
    Solid,
    /// Hollow colored ring around the target point.
    Hollow,
}

/// Normalizes a laser diameter in points to `[0.1, 32.0]`, falling back to `8.0` for non-finite
/// values (ported directly from SumatraPDF Enhanced `NormalizeLaserWidth`).
#[must_use]
pub fn normalize_laser_width(width: f32) -> f32 {
    if !width.is_finite() {
        return DEFAULT_LASER_WIDTH;
    }
    width.clamp(0.1, 32.0)
}

/// A single timestamped point along a fading laser trail (G.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaserTrailPoint {
    /// Position in page points.
    pub point: PagePoint,
    /// Capture timestamp in milliseconds.
    pub time_ms: u64,
    /// Computed opacity `[0.0..=1.0]` at the latest tick.
    pub alpha: f32,
}

/// One continuous laser trail stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct LaserStroke {
    /// Points in drawing order.
    pub points: Vec<LaserTrailPoint>,
    /// Timestamp when the presenter released the trail (`None` while actively dragging).
    pub finished_at_ms: Option<u64>,
}

impl LaserStroke {
    /// Whether this laser trail has expired (matching Enhanced `LaserStrokeExpired`: active strokes
    /// never expire; finished strokes expire once `now - finished >= lifetime`).
    #[must_use]
    pub fn is_expired(&self, now_ms: u64, lifetime_ms: u64) -> bool {
        match self.finished_at_ms {
            None => false,
            Some(finished) => now_ms.wrapping_sub(finished) >= lifetime_ms,
        }
    }
}

/// Laser pointer with fading trails overlay model (plan G.2).
#[derive(Debug, Clone, PartialEq)]
pub struct LaserPointer {
    /// Current live dot position (follows pointer in the same frame).
    pub position: Option<PagePoint>,
    /// Laser RGB color in `0.0..=1.0` (default bright red).
    pub color: [f32; 3],
    /// Laser diameter in points (normalized to `[0.1, 32.0]`).
    pub width_pt: f32,
    /// Solid or hollow dot mode.
    pub mode: LaserMode,
    /// Trail fade lifetime in milliseconds.
    pub lifetime_ms: u64,
    /// Active and fading trail strokes.
    pub strokes: Vec<LaserStroke>,
    drawing: bool,
}

impl Default for LaserPointer {
    fn default() -> Self {
        Self {
            position: None,
            color: [1.0, 0.12, 0.12],
            width_pt: DEFAULT_LASER_WIDTH,
            mode: LaserMode::Solid,
            lifetime_ms: DEFAULT_LASER_LIFETIME_MS,
            strokes: Vec::new(),
            drawing: false,
        }
    }
}

impl LaserPointer {
    /// Creates a new laser pointer with normalized width.
    #[must_use]
    pub fn new(color: [f32; 3], width_pt: f32, mode: LaserMode, lifetime_ms: u64) -> Self {
        Self {
            color,
            width_pt: normalize_laser_width(width_pt),
            mode,
            lifetime_ms: lifetime_ms.max(100),
            ..Self::default()
        }
    }

    /// Updates the laser dot position on pointer hover/move (same frame, no re-render).
    pub fn move_pointer(&mut self, point: PagePoint, now_ms: u64) {
        self.position = Some(point);
        if self.drawing {
            self.push_trail_point(point, now_ms);
        }
    }

    /// Begins a fading laser trail stroke at `point`.
    pub fn begin_trail(&mut self, point: PagePoint, now_ms: u64) {
        self.position = Some(point);
        self.drawing = true;
        self.strokes.push(LaserStroke {
            points: vec![LaserTrailPoint {
                point,
                time_ms: now_ms,
                alpha: 1.0,
            }],
            finished_at_ms: None,
        });
    }

    /// Ends the current laser trail stroke so it begins fading out.
    pub fn end_trail(&mut self, now_ms: u64) {
        self.drawing = false;
        if let Some(last) = self.strokes.last_mut()
            && last.finished_at_ms.is_none()
        {
            last.finished_at_ms = Some(now_ms);
        }
    }

    /// Updates trail point alphas and prunes expired strokes at `now_ms`.
    pub fn tick(&mut self, now_ms: u64) {
        let lifetime = self.lifetime_ms.max(1) as f32;
        self.strokes
            .retain(|s| !s.is_expired(now_ms, self.lifetime_ms));
        for stroke in &mut self.strokes {
            for pt in &mut stroke.points {
                let age = now_ms.saturating_sub(pt.time_ms) as f32;
                pt.alpha = (1.0 - age / lifetime).clamp(0.0, 1.0);
            }
        }
    }

    fn push_trail_point(&mut self, point: PagePoint, now_ms: u64) {
        let total: usize = self.strokes.iter().map(|s| s.points.len()).sum();
        if total >= LASER_TRAIL_MAX_POINTS {
            return;
        }
        if let Some(last) = self.strokes.last_mut() {
            last.points.push(LaserTrailPoint {
                point,
                time_ms: now_ms,
                alpha: 1.0,
            });
        }
    }
}

/// Geometric shape of the spotlight cutout (plan G.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpotlightShape {
    /// Circular cutout centered at the pointer.
    Circle {
        /// Cutout radius in page points.
        radius_pt: f32,
    },
    /// Rectangular cutout centered at the pointer.
    Rectangle {
        /// Cutout width in page points.
        width_pt: f32,
        /// Cutout height in page points.
        height_pt: f32,
    },
}

/// Overlay description rendered by the canvas for the spotlight (G.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotlightOverlay {
    /// Cutout center in page points.
    pub center: PagePoint,
    /// Shape and dimensions of the lit region.
    pub shape: SpotlightShape,
    /// Opacity `[0.0..=0.95]` of the darkened region outside the cutout.
    pub dim_opacity: f32,
}

/// Spotlight overlay controller darkening everything outside a circle or rectangle (plan G.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spotlight {
    /// Current spotlight center (tracks pointer in the same frame).
    pub center: PagePoint,
    /// Circle or rectangle shape.
    pub shape: SpotlightShape,
    /// Opacity of the dim veil outside the cutout.
    pub dim_opacity: f32,
}

impl Default for Spotlight {
    fn default() -> Self {
        Self {
            center: PagePoint { x: 0.0, y: 0.0 },
            shape: SpotlightShape::Circle { radius_pt: 72.0 },
            dim_opacity: 0.65,
        }
    }
}

impl Spotlight {
    /// Moves the spotlight center immediately to `point` in the same frame.
    pub fn move_to(&mut self, point: PagePoint) {
        self.center = point;
    }

    /// Produces the overlay geometry for the canvas compositor.
    #[must_use]
    pub fn overlay(&self) -> SpotlightOverlay {
        SpotlightOverlay {
            center: self.center,
            shape: self.shape,
            dim_opacity: self.dim_opacity.clamp(0.1, 0.95),
        }
    }
}

/// Computed lens geometry and source tile sampling rectangle for the magnifier (plan G.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagnifierView {
    /// Lens bounding box on screen/page coordinates.
    pub lens_rect: PageRect,
    /// Source region sampled from higher-zoom tiles, clamped to page bounds.
    pub source_rect: PageRect,
    /// Effective magnification multiplier (`> 1.0`).
    pub zoom_factor: f32,
}

/// Magnifier lens overlay sampling higher-zoom tiles around the pointer (plan G.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagnifierLens {
    /// Pointer center in page points.
    pub center: PagePoint,
    /// Lens radius in page points.
    pub radius_pt: f32,
    /// Magnification factor clamped to `[1.25, 8.0]`.
    pub zoom_factor: f32,
}

impl Default for MagnifierLens {
    fn default() -> Self {
        Self {
            center: PagePoint { x: 0.0, y: 0.0 },
            radius_pt: 90.0,
            zoom_factor: 2.5,
        }
    }
}

impl MagnifierLens {
    /// Moves the lens immediately to `point` in the same frame.
    pub fn move_to(&mut self, point: PagePoint) {
        self.center = point;
    }

    /// Computes the exact lens and higher-zoom source tile rectangle clamped inside `page_bounds`.
    #[must_use]
    pub fn view_for_page(&self, page_bounds: PageRect) -> MagnifierView {
        let zoom = self.zoom_factor.clamp(1.25, 8.0);
        let r = self.radius_pt.max(24.0);
        let src_r = r / zoom;

        let cx = self.center.x.clamp(page_bounds.x0, page_bounds.x1);
        let cy = self.center.y.clamp(page_bounds.y0, page_bounds.y1);

        let lens_rect = PageRect {
            x0: cx - r,
            y0: cy - r,
            x1: cx + r,
            y1: cy + r,
        };
        let source_rect = PageRect {
            x0: (cx - src_r).max(page_bounds.x0),
            y0: (cy - src_r).max(page_bounds.y0),
            x1: (cx + src_r).min(page_bounds.x1),
            y1: (cy + src_r).min(page_bounds.y1),
        };
        MagnifierView {
            lens_rect,
            source_rect,
            zoom_factor: zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laser_width_and_expiry_match_sumatrapdf_enhanced() {
        assert_eq!(normalize_laser_width(-1.0), 0.1);
        assert_eq!(normalize_laser_width(0.1), 0.1);
        assert_eq!(normalize_laser_width(8.0), 8.0);
        assert_eq!(normalize_laser_width(32.0), 32.0);
        assert_eq!(normalize_laser_width(100.0), 32.0);
        assert_eq!(normalize_laser_width(f32::NAN), 8.0);
        assert_eq!(normalize_laser_width(f32::INFINITY), 8.0);

        let mut laser = LaserPointer::default();
        laser.begin_trail(PagePoint { x: 10.0, y: 20.0 }, 0);
        laser.move_pointer(PagePoint { x: 50.0, y: 60.0 }, 100);
        assert_eq!(laser.position, Some(PagePoint { x: 50.0, y: 60.0 }));
        // Active stroke never expires even past lifetime.
        laser.tick(50_000);
        assert_eq!(laser.strokes.len(), 1);

        laser.end_trail(50_000);
        laser.tick(51_999);
        assert_eq!(laser.strokes.len(), 1);
        laser.tick(52_000);
        assert!(laser.strokes.is_empty(), "expired exactly at lifetime_ms");
    }

    #[test]
    fn spotlight_and_magnifier_track_pointer_in_same_frame() {
        let page = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        let p = PagePoint { x: 200.0, y: 300.0 };

        let mut spot = Spotlight::default();
        spot.move_to(p);
        assert_eq!(spot.overlay().center, p);

        let mut mag = MagnifierLens {
            center: PagePoint { x: 0.0, y: 0.0 },
            radius_pt: 100.0,
            zoom_factor: 2.0,
        };
        mag.move_to(p);
        let view = mag.view_for_page(page);
        assert_eq!(view.lens_rect.x1 - view.lens_rect.x0, 200.0);
        assert_eq!(view.source_rect.x1 - view.source_rect.x0, 100.0);
    }
}
