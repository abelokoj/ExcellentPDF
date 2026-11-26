//! GPU / `SurfaceView` / `HardwareBuffer` tile upload bridge for Android (plan K.12).
//!
//! Coordinates visible tiles via [`excellent_canvas::tiles`], renders PDF pages with [`excellent_mupdf`],
//! and packs tile bitmaps into `RGBA8888` or `RGB565` buffers ready for `ANativeWindow_lock`,
//! `HardwareBuffer`, or OpenGL ES `glTexSubImage2D`.

use excellent_canvas::Rect;
use excellent_canvas::layout::{Layout, PageSize};
use excellent_canvas::tiles::{TileCache, TileKey, visible_tiles};
use excellent_canvas::view::View;
use excellent_canvas::viewport::{Viewport, ZoomMode};
use excellent_mupdf::{Background, Document as MuDocument, Matrix, PageRect, Pixmap};

use crate::{AndroidCanvasError, Result};

/// Default tile memory cache budget (64 MiB).
const TILE_BUDGET_BYTES: usize = 64 << 20;

/// Android surface presentation backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AndroidSurfaceKind {
    /// `SurfaceView` backed by `ANativeWindow` / `HardwareBuffer` RGBA8888.
    #[default]
    HardwareBuffer,
    /// Direct `ANativeWindow_Buffer` RGBA8888 blitting.
    NativeWindowRgba,
    /// OpenGL ES 3.0 texture tile upload (`GL_RGBA8`).
    OpenGlEsTexture,
}

/// Target pixel packing for Android tile uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SurfacePixelFormat {
    /// 32-bit `R, G, B, A` (`AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM`).
    #[default]
    Rgba8888,
    /// 16-bit packed `RGB565` for memory-constrained tablets.
    Rgb565,
}

impl SurfacePixelFormat {
    /// Bytes per pixel for this packing.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8888 => 4,
            Self::Rgb565 => 2,
        }
    }
}

/// Order in which pending visible tiles are uploaded to the Android surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TileUploadOrder {
    /// Visible tiles nearest screen center first.
    #[default]
    CenterOut,
    /// Document reading order.
    TopToBottom,
}

/// Prepared tile buffer ready for `Surface` / `HardwareBuffer` / OpenGL ES upload.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadedAndroidTile {
    /// Tile key (`page`, `bucket`, `col`, `row`).
    pub key: TileKey,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub stride_bytes: usize,
    /// Target Android pixel format.
    pub pixel_format: SurfacePixelFormat,
    /// Screen placement rectangle in view pixels.
    pub screen_rect: Rect,
    /// Tightly packed pixel bytes.
    pub pixels: Vec<u8>,
}

/// Coordinates Android viewport layout, MuPDF rendering, and tile buffer packing.
#[derive(Debug)]
pub struct AndroidTileSurfaceBridge {
    surface_kind: AndroidSurfaceKind,
    pixel_format: SurfacePixelFormat,
    view: View,
    layout: Layout,
    viewport: Viewport,
    bounds: Vec<PageRect>,
    tiles: TileCache<Option<(UploadedAndroidTile, usize)>, UploadedAndroidTile>,
    document: Option<MuDocument>,
}

impl AndroidTileSurfaceBridge {
    /// Creates an Android tile bridge at `pixels_per_point` (`DisplayMetrics.density`).
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::InvalidGeometry`] if `pixels_per_point` is non-positive or non-finite.
    pub fn try_new(
        surface_kind: AndroidSurfaceKind,
        pixel_format: SurfacePixelFormat,
        pixels_per_point: f32,
    ) -> Result<Self> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(AndroidCanvasError::InvalidGeometry(format!(
                "density must be positive and finite, got {pixels_per_point}"
            )));
        }
        Ok(Self {
            surface_kind,
            pixel_format,
            view: View::default(),
            layout: Layout::default(),
            viewport: Viewport::new(pixels_per_point),
            bounds: Vec::new(),
            tiles: TileCache::new(TILE_BUDGET_BYTES),
            document: None,
        })
    }

    /// Returns the surface kind.
    #[must_use]
    pub const fn surface_kind(&self) -> AndroidSurfaceKind {
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

    /// Returns the number of pages loaded.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.bounds.len()
    }

    /// Opens raw PDF bytes with MuPDF and computes page layout bounds.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::RenderFailed`] if MuPDF cannot open the document.
    pub fn open_pdf_bytes(&mut self, pdf_bytes: &[u8]) -> Result<()> {
        let doc = MuDocument::from_bytes(pdf_bytes, "document.pdf")
            .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
        let count = doc
            .page_count()
            .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
        let mut bounds = Vec::with_capacity(count);
        for idx in 0..count {
            let rect = doc
                .page_box(idx, self.view.page_box)
                .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
            bounds.push(rect);
        }
        self.document = Some(doc);
        self.set_page_bounds(&bounds);
        Ok(())
    }

    /// Sets explicit page bounds in points (useful in unit tests without PDF files).
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

    /// Updates surface dimensions in pixels and display density.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::InvalidGeometry`] if `pixels_per_point` is invalid.
    pub fn resize(&mut self, width_px: u32, height_px: u32, pixels_per_point: f32) -> Result<()> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(AndroidCanvasError::InvalidGeometry(format!(
                "invalid density {pixels_per_point}"
            )));
        }
        self.viewport
            .set_pixels_per_point(pixels_per_point, &self.layout);
        self.viewport
            .resize(width_px as f32, height_px as f32, &self.layout);
        let _ = self.view.resized(width_px as f32);
        Ok(())
    }

    /// Scrolls by `(dx, dy)` pixels.
    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        self.viewport.scroll_by(dx, dy, &self.layout);
    }

    /// Zooms by `factor` around pixel `(x, y)`.
    pub fn zoom_about(&mut self, factor: f32, x: f32, y: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.viewport.zoom_about(factor, x, y, &self.layout);
        }
    }

    /// Lists visible tiles in upload priority order.
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

    /// Packs a MuPDF [`Pixmap`] into [`UploadedAndroidTile`] bytes for the configured [`SurfacePixelFormat`].
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError`] if the pixmap is empty or the page index is out of range.
    pub fn pack_pixmap(&self, key: TileKey, pixmap: &Pixmap) -> Result<UploadedAndroidTile> {
        pack_android_tile(&self.layout, &self.viewport, self.pixel_format, key, pixmap)
    }

    /// Renders visible PDF tiles and caches the uploaded Android surface buffers.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::RenderFailed`] if rendering fails.
    pub fn render_visible_tiles(
        &mut self,
        order: TileUploadOrder,
    ) -> Result<Vec<UploadedAndroidTile>> {
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
            if area.x1 <= area.x0 || area.y1 <= area.y0 {
                continue;
            }
            let scale = excellent_canvas::tiles::bucket_scale(key.bucket);
            let page = doc
                .load_page(key.page)
                .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
            let list = page
                .display_list(None)
                .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
            let device_area = excellent_mupdf::DeviceRect {
                x0: area.x0,
                y0: area.y0,
                x1: area.x1,
                y1: area.y1,
            };
            let matrix = Matrix::scale(scale, scale);
            let pixmap = list
                .render_area(&matrix, device_area, Background::White, None)
                .map_err(|err| AndroidCanvasError::RenderFailed(err.to_string()))?;
            let tile = pack_android_tile(
                &self.layout,
                &self.viewport,
                self.pixel_format,
                key,
                &pixmap,
            )?;
            let size = tile.pixels.len();
            uploaded.push(tile.clone());
            self.tiles.request(key, Some((tile, size)));
        }
        let _ = self.tiles.poll(|slot| slot.take().map(Some));
        self.tiles.evict();
        Ok(uploaded)
    }

    /// Number of visible tiles currently held in the tile cache.
    #[must_use]
    pub fn cached_tile_count(&self) -> usize {
        self.visible_tile_keys(TileUploadOrder::TopToBottom)
            .into_iter()
            .filter(|k| self.tiles.contains(k))
            .count()
    }
}

fn pack_android_tile(
    layout: &Layout,
    viewport: &Viewport,
    pixel_format: SurfacePixelFormat,
    key: TileKey,
    pixmap: &Pixmap,
) -> Result<UploadedAndroidTile> {
    let width = pixmap.width();
    let height = pixmap.height();
    if width == 0 || height == 0 {
        return Err(AndroidCanvasError::InvalidGeometry(
            "cannot upload zero-area tile pixmap".to_owned(),
        ));
    }
    let screen_rect = key
        .on_screen(layout, viewport)
        .ok_or(AndroidCanvasError::OutOfRange {
            index: key.page as i64,
            count: layout.len(),
        })?;

    let w = width as usize;
    let h = height as usize;
    let stride = pixmap.stride();
    let comps = pixmap.components();
    let samples = pixmap.samples();
    let stride_bytes = w * pixel_format.bytes_per_pixel();
    let mut pixels = Vec::with_capacity(stride_bytes * h);

    match pixel_format {
        SurfacePixelFormat::Rgba8888 => {
            for row in samples.chunks_exact(stride).take(h) {
                for px in row[..w * comps].chunks_exact(comps) {
                    let r = px[0];
                    let g = *px.get(1).unwrap_or(&r);
                    let b = *px.get(2).unwrap_or(&r);
                    let a = if comps >= 4 { px[3] } else { 255 };
                    pixels.extend_from_slice(&[r, g, b, a]);
                }
            }
        }
        SurfacePixelFormat::Rgb565 => {
            for row in samples.chunks_exact(stride).take(h) {
                for px in row[..w * comps].chunks_exact(comps) {
                    let r = u16::from(px[0] >> 3);
                    let g = u16::from(*px.get(1).unwrap_or(&px[0]) >> 2);
                    let b = u16::from(*px.get(2).unwrap_or(&px[0]) >> 3);
                    let packed = (r << 11) | (g << 5) | b;
                    pixels.extend_from_slice(&packed.to_le_bytes());
                }
            }
        }
    }

    Ok(UploadedAndroidTile {
        key,
        width,
        height,
        stride_bytes,
        pixel_format,
        screen_rect,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

    #[test]
    fn packs_rgba8888_and_renders_pdf_tiles_for_surface_view() {
        let mut samples = Vec::with_capacity(64 * 64 * 3);
        for _ in 0..(64 * 64) {
            samples.extend_from_slice(&[0x12, 0x34, 0x56]);
        }
        let pdf = images_to_pdf(
            &[ImageInput::Rgb8 {
                width: 64,
                height: 64,
                dpi: 72.0,
                samples,
            }],
            PdfPaperSize::Letter,
            0.0,
        )
        .expect("build pdf");

        let mut bridge = AndroidTileSurfaceBridge::try_new(
            AndroidSurfaceKind::HardwareBuffer,
            SurfacePixelFormat::Rgba8888,
            2.0,
        )
        .expect("bridge");
        bridge.resize(1200, 1600, 2.0).expect("resize");
        bridge.open_pdf_bytes(&pdf).expect("open pdf");
        let tiles = bridge
            .render_visible_tiles(TileUploadOrder::CenterOut)
            .expect("render tiles");
        assert!(!tiles.is_empty());
        assert_eq!(tiles[0].pixel_format, SurfacePixelFormat::Rgba8888);
        assert!(bridge.cached_tile_count() > 0);
    }
}
