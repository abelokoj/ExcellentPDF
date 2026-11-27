//! Metal (`CAMetalLayer`) and Core Graphics (`CGImage`) tile surface bridge for iPadOS (plan K.11).
//!
//! Coordinates visible tile calculation through [`ecclesia_canvas::tiles`], renders PDF pages via
//! [`ecclesia_mupdf`] when a document is attached, and converts RGB MuPDF pixmaps into tightly packed
//! `BGRA8Unorm` (`MTLTexture`) or `RGB8` (`CGImage`) upload descriptors.

use ecclesia_canvas::Rect;
use ecclesia_canvas::layout::{Layout, PageSize};
use ecclesia_canvas::tiles::{TileCache, TileKey, visible_tiles};
use ecclesia_canvas::view::View;
use ecclesia_canvas::viewport::{Viewport, ZoomMode};
use ecclesia_mupdf::{Background, Document as MuDocument, Matrix, PageRect, Pixmap};

use crate::{IosCanvasError, Result};

/// Tile budget in bytes (64 MiB), matching the shared canvas policy.
const TILE_BUDGET_BYTES: usize = 64 << 20;

/// GPU/2D backend surface target on iPadOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TileSurfaceKind {
    /// `CAMetalLayer` texture (`MTLPixelFormatBGRA8Unorm`).
    #[default]
    MetalTexture,
    /// Core Graphics bitmap image (`CGImage` 24-bit RGB).
    CoreGraphicsImage,
}

/// Pixel encoding produced for tile uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TilePixelFormat {
    /// 4-byte-per-pixel `B, G, R, A` suitable for `MTLPixelFormatBGRA8Unorm`.
    Bgra8Unorm,
    /// 3-byte-per-pixel `R, G, B` suitable for `CGDataProvider` / `CGImage`.
    Rgb8,
}

impl TilePixelFormat {
    /// Bytes per pixel for this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra8Unorm => 4,
            Self::Rgb8 => 3,
        }
    }
}

/// Order in which pending tiles should be uploaded to Metal or Core Graphics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TileUploadOrder {
    /// Visible tiles nearest viewport center first.
    #[default]
    CenterOut,
    /// Reading order (page, row, column).
    TopToBottom,
}

/// CPU-prepared tile pixel buffer ready for `MTLTexture.replace` or `CGImage::new`.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadedTileSurface {
    /// Tile key identifying page, zoom bucket, column, and row.
    pub key: TileKey,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub bytes_per_row: usize,
    /// Target surface pixel format.
    pub pixel_format: TilePixelFormat,
    /// Screen placement rectangle in device pixels.
    pub screen_rect: Rect,
    /// Tightly packed pixel payload matching `pixel_format`.
    pub pixels: Vec<u8>,
}

/// Bridge coordinating viewport layout, tile cache eviction, and Metal/CG tile buffer preparation.
#[derive(Debug)]
pub struct TileSurfaceBridge {
    surface_kind: TileSurfaceKind,
    view: View,
    layout: Layout,
    viewport: Viewport,
    bounds: Vec<PageRect>,
    tiles: TileCache<Option<(UploadedTileSurface, usize)>, UploadedTileSurface>,
    document: Option<MuDocument>,
}

impl TileSurfaceBridge {
    /// Creates an empty bridge for `surface_kind` at `pixels_per_point` (typically 2.0 on Retina iPads).
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::InvalidGeometry`] if `pixels_per_point` is non-finite or non-positive.
    pub fn try_new(surface_kind: TileSurfaceKind, pixels_per_point: f32) -> Result<Self> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(IosCanvasError::InvalidGeometry(format!(
                "pixels_per_point must be positive and finite, got {pixels_per_point}"
            )));
        }
        Ok(Self {
            surface_kind,
            view: View::default(),
            layout: Layout::default(),
            viewport: Viewport::new(pixels_per_point),
            bounds: Vec::new(),
            tiles: TileCache::new(TILE_BUDGET_BYTES),
            document: None,
        })
    }

    /// Returns the configured surface kind (`MetalTexture` or `CoreGraphicsImage`).
    #[must_use]
    pub const fn surface_kind(&self) -> TileSurfaceKind {
        self.surface_kind
    }

    /// Returns the active layout.
    #[must_use]
    pub const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Returns the active viewport.
    #[must_use]
    pub const fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// Returns the number of pages loaded in this bridge.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.bounds.len()
    }

    /// Opens raw PDF bytes via MuPDF and lays out its pages.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::RenderFailed`] if MuPDF cannot open the document or inspect page boxes.
    pub fn open_pdf_bytes(&mut self, pdf_bytes: &[u8]) -> Result<()> {
        let doc = MuDocument::from_bytes(pdf_bytes, "document.pdf")
            .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
        let count = doc
            .page_count()
            .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
        let mut bounds = Vec::with_capacity(count);
        for index in 0..count {
            let rect = doc
                .page_box(index, self.view.page_box)
                .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
            bounds.push(rect);
        }
        self.document = Some(doc);
        self.set_page_bounds(&bounds);
        Ok(())
    }

    /// Attaches synthetic or pre-measured page bounds in PDF points (useful for headless layout tests).
    pub fn set_page_bounds(&mut self, bounds: &[PageRect]) {
        self.bounds = bounds.to_vec();
        let sizes: Vec<PageSize> = bounds
            .iter()
            .map(|r| PageSize {
                width: r.width().max(1.0),
                height: r.height().max(1.0),
            })
            .collect();
        self.layout = Layout::new(&sizes, self.view.spec);
        self.tiles.clear();
        self.viewport.fit(ZoomMode::FitWidth, &self.layout);
    }

    /// Resizes the view in device pixels and updates `pixels_per_point`.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::InvalidGeometry`] if `pixels_per_point` is invalid.
    pub fn resize(&mut self, width_px: u32, height_px: u32, pixels_per_point: f32) -> Result<()> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(IosCanvasError::InvalidGeometry(format!(
                "invalid pixels_per_point {pixels_per_point}"
            )));
        }
        self.viewport
            .set_pixels_per_point(pixels_per_point, &self.layout);
        self.viewport
            .resize(width_px as f32, height_px as f32, &self.layout);
        let _ = self.view.resized(width_px as f32);
        Ok(())
    }

    /// Scrolls the viewport by `(dx, dy)` device pixels.
    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        self.viewport.scroll_by(dx, dy, &self.layout);
    }

    /// Applies a pinch-zoom factor centered at view pixel `(x, y)`.
    pub fn zoom_about(&mut self, factor: f32, x: f32, y: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.viewport.zoom_about(factor, x, y, &self.layout);
        }
    }

    /// Toggles double-tap smart zoom between fit-width and 2.0x magnification about `(x, y)`.
    pub fn double_tap_zoom(&mut self, x: f32, y: f32) {
        if matches!(self.viewport.zoom_mode(), ZoomMode::FitWidth) {
            let target = (self.viewport.zoom() * 2.0).max(1.5);
            let factor = target / self.viewport.zoom().max(0.01);
            self.viewport.zoom_about(factor, x, y, &self.layout);
        } else {
            self.viewport.fit(ZoomMode::FitWidth, &self.layout);
        }
    }

    /// Returns the tile keys currently visible in the viewport, ordered per `order`.
    #[must_use]
    pub fn visible_tile_keys(&self, order: TileUploadOrder) -> Vec<TileKey> {
        let mut keys = visible_tiles(&self.layout, &self.viewport);
        if matches!(order, TileUploadOrder::CenterOut) {
            let cx = self.viewport.width() * 0.5;
            let cy = self.viewport.height() * 0.5;
            keys.sort_by(|a, b| {
                let da = a
                    .on_screen(&self.layout, &self.viewport)
                    .map_or(f32::MAX, |r| {
                        let rx = (r.x0 + r.x1) * 0.5 - cx;
                        let ry = (r.y0 + r.y1) * 0.5 - cy;
                        rx * rx + ry * ry
                    });
                let db = b
                    .on_screen(&self.layout, &self.viewport)
                    .map_or(f32::MAX, |r| {
                        let rx = (r.x0 + r.x1) * 0.5 - cx;
                        let ry = (r.y0 + r.y1) * 0.5 - cy;
                        rx * rx + ry * ry
                    });
                da.total_cmp(&db)
            });
        }
        keys
    }

    /// Converts a rendered MuPDF [`Pixmap`] into an [`UploadedTileSurface`] for Metal or Core Graphics.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::InvalidGeometry`] if the pixmap has zero area or cannot map onto screen.
    pub fn convert_pixmap(&self, key: TileKey, pixmap: &Pixmap) -> Result<UploadedTileSurface> {
        pack_tile_surface(&self.layout, &self.viewport, self.surface_kind, key, pixmap)
    }

    /// Renders and caches visible tiles from the opened PDF document, returning the newly uploaded tiles.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::RenderFailed`] if rendering a visible page fails.
    pub fn render_visible_tiles(
        &mut self,
        order: TileUploadOrder,
    ) -> Result<Vec<UploadedTileSurface>> {
        let keys = self.visible_tile_keys(order);
        let Some(doc) = self.document.as_ref() else {
            return Ok(Vec::new());
        };
        self.tiles.begin_frame();
        let mut uploaded = Vec::new();

        for key in keys {
            if let Some(cached) = self.tiles.ready(&key) {
                uploaded.push(cached.clone());
                continue;
            }
            let Some(page_rect) = self.layout.page(key.page) else {
                continue;
            };
            let area = key.area(&page_rect);
            let width = area.x1 - area.x0;
            let height = area.y1 - area.y0;
            if width <= 0 || height <= 0 {
                continue;
            }
            let scale = ecclesia_canvas::tiles::bucket_scale(key.bucket);
            let page = doc
                .load_page(key.page)
                .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
            let list = page
                .display_list(None)
                .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
            let device_area = ecclesia_mupdf::DeviceRect {
                x0: area.x0,
                y0: area.y0,
                x1: area.x1,
                y1: area.y1,
            };
            let matrix = Matrix::scale(scale, scale);
            let pixmap = list
                .render_area(&matrix, device_area, Background::White, None)
                .map_err(|err| IosCanvasError::RenderFailed(err.to_string()))?;
            let surface = pack_tile_surface(
                &self.layout,
                &self.viewport,
                self.surface_kind,
                key,
                &pixmap,
            )?;
            let byte_size = surface.pixels.len();
            uploaded.push(surface.clone());
            self.tiles.request(key, Some((surface, byte_size)));
        }

        let _ = self.tiles.poll(|slot| slot.take().map(Some));
        self.tiles.evict();
        Ok(uploaded)
    }

    /// Returns the number of visible tiles currently held in the tile cache.
    #[must_use]
    pub fn cached_tile_count(&self) -> usize {
        self.visible_tile_keys(TileUploadOrder::TopToBottom)
            .into_iter()
            .filter(|k| self.tiles.contains(k))
            .count()
    }
}

fn pack_tile_surface(
    layout: &Layout,
    viewport: &Viewport,
    surface_kind: TileSurfaceKind,
    key: TileKey,
    pixmap: &Pixmap,
) -> Result<UploadedTileSurface> {
    let width = pixmap.width();
    let height = pixmap.height();
    if width == 0 || height == 0 {
        return Err(IosCanvasError::InvalidGeometry(
            "cannot upload zero-area tile pixmap".to_owned(),
        ));
    }
    let screen_rect = key
        .on_screen(layout, viewport)
        .ok_or(IosCanvasError::OutOfRange {
            index: key.page,
            count: layout.len(),
        })?;

    let target_format = match surface_kind {
        TileSurfaceKind::MetalTexture => TilePixelFormat::Bgra8Unorm,
        TileSurfaceKind::CoreGraphicsImage => TilePixelFormat::Rgb8,
    };

    let w = width as usize;
    let h = height as usize;
    let stride = pixmap.stride();
    let comps = pixmap.components();
    let samples = pixmap.samples();
    let bytes_per_row = w * target_format.bytes_per_pixel();
    let mut pixels = Vec::with_capacity(bytes_per_row * h);

    match target_format {
        TilePixelFormat::Bgra8Unorm => {
            for row in samples.chunks_exact(stride).take(h) {
                for px in row[..w * comps].chunks_exact(comps) {
                    let r = px[0];
                    let g = *px.get(1).unwrap_or(&r);
                    let b = *px.get(2).unwrap_or(&r);
                    let a = if comps >= 4 { px[3] } else { 255 };
                    pixels.extend_from_slice(&[b, g, r, a]);
                }
            }
        }
        TilePixelFormat::Rgb8 => {
            for row in samples.chunks_exact(stride).take(h) {
                for px in row[..w * comps].chunks_exact(comps) {
                    let r = px[0];
                    let g = *px.get(1).unwrap_or(&r);
                    let b = *px.get(2).unwrap_or(&r);
                    pixels.extend_from_slice(&[r, g, b]);
                }
            }
        }
    }

    Ok(UploadedTileSurface {
        key,
        width,
        height,
        bytes_per_row,
        pixel_format: target_format,
        screen_rect,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

    #[test]
    fn metal_and_cg_surface_conversion_produces_expected_pixel_layouts() {
        let mut bridge =
            TileSurfaceBridge::try_new(TileSurfaceKind::MetalTexture, 2.0).expect("valid bridge");
        bridge.resize(1024, 768, 2.0).expect("valid resize");
        bridge.set_page_bounds(&[PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        }]);

        let keys = bridge.visible_tile_keys(TileUploadOrder::CenterOut);
        assert!(!keys.is_empty(), "expected visible tiles");

        let mut pixmap = Pixmap::new_rgb(16, 16).expect("16x16 pixmap");
        for chunk in pixmap.samples_mut().as_chunks_mut::<3>().0 {
            chunk[0] = 0x11;
            chunk[1] = 0x22;
            chunk[2] = 0x33;
        }
        let tile = bridge
            .convert_pixmap(keys[0], &pixmap)
            .expect("metal tile upload");
        assert_eq!(tile.pixel_format, TilePixelFormat::Bgra8Unorm);
        assert_eq!(tile.bytes_per_row, 16 * 4);
        assert_eq!(&tile.pixels[..4], &[0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn renders_real_pdf_tiles_for_metal_layer() {
        let rgb = vec![40u8; 64 * 64 * 3];
        let pdf = images_to_pdf(
            &[ImageInput::Rgb8 {
                width: 64,
                height: 64,
                dpi: 72.0,
                samples: rgb,
            }],
            PdfPaperSize::Letter,
            0.0,
        )
        .expect("build pdf");

        let mut bridge =
            TileSurfaceBridge::try_new(TileSurfaceKind::MetalTexture, 2.0).expect("bridge");
        bridge.resize(800, 1000, 2.0).expect("resize");
        bridge.open_pdf_bytes(&pdf).expect("open pdf");
        let uploaded = bridge
            .render_visible_tiles(TileUploadOrder::TopToBottom)
            .expect("render tiles");
        assert!(!uploaded.is_empty(), "expected rendered Metal tiles");
        assert!(bridge.cached_tile_count() > 0);
    }
}
