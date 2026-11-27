//! The Windows [`Host`] for [`ecclesia_canvas::page::PageCanvas`]: draws directly into NSH's active
//! [`ID2D1DeviceContext`] without creating a child HWND or secondary D3D11/DXGI swap chain.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use ecclesia_canvas::Rect;
use ecclesia_canvas::page::host::{InkPixel, Quad, TriangleDirection, Wake, Wash};
use ecclesia_canvas::page::{Host, PageCanvas};
use ecclesia_mupdf::Pixmap;
use nsh_core::JsonValue;
use nsh_platform_windows::ComponentContext;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1, ID2D1StrokeStyle1,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::core::{Error, Interface};

use crate::gfx;

/// The page canvas as Windows runs it.
pub(crate) type Canvas = PageCanvas<WinHost>;

/// Direct2D host backed by NSH's active device context.
pub(crate) struct WinHost {
    /// Channel for emitting component events and requesting NSH redraws.
    events: ComponentContext,
    /// Shared flag set by background worker wakers when finished jobs are ready to drain.
    jobs_pending: Arc<AtomicBool>,
    /// Active Direct2D device context passed by NSH during `NativeComponent::draw`.
    dc: Option<ID2D1DeviceContext>,
    /// Raw COM pointer identity of the `ID2D1Device` backing `dc`, used to detect device resets.
    device_ptr: usize,
    /// Set when a new `ID2D1Device` is observed so `resize()` triggers `PageCanvas::lose_device`.
    device_changed: bool,
    /// Direct2D factory obtained from `dc`, used for highlight and dimming geometries.
    factory: Option<ID2D1Factory1>,
    /// Round-cap, round-join stroke style created on `factory`, used for ink polylines.
    round: Option<ID2D1StrokeStyle1>,
    /// Current viewport size in physical device pixels.
    size: (f32, f32),
}

impl WinHost {
    /// Creates a new Windows host reporting events and redraws through `events`.
    pub(crate) fn new(events: ComponentContext, jobs_pending: Arc<AtomicBool>) -> Self {
        Self {
            events,
            jobs_pending,
            dc: None,
            device_ptr: 0,
            device_changed: false,
            factory: None,
            round: None,
            size: (0.0, 0.0),
        }
    }

    /// Updates the cached physical viewport size in device pixels.
    pub(crate) fn set_size(&mut self, width: f32, height: f32) {
        self.size = (width, height);
    }

    /// Attaches NSH's active Direct2D device context for the current frame.
    ///
    /// Returns `true` when the underlying `ID2D1Device` changed since the previous frame; the caller
    /// should then invoke [`PageCanvas::resize`] so cached bitmaps from the old device are discarded
    /// before calling `attach_dc` again.
    pub(crate) fn attach_dc(&mut self, dc: &ID2D1DeviceContext) -> bool {
        // SAFETY: `GetDevice` is a standard COM getter on the live context NSH passed to `draw`.
        let device = match unsafe { dc.GetDevice() } {
            Ok(device) => device,
            Err(err) => {
                log::warn!("page canvas: cannot query ID2D1Device from NSH context: {err}");
                return false;
            }
        };
        let ptr = device.as_raw() as usize;
        if self.device_ptr != 0 && self.device_ptr != ptr {
            self.device_changed = true;
            return true;
        }
        if self.device_ptr != ptr
            || self.dc.is_none()
            || self.factory.is_none()
            || self.round.is_none()
        {
            match gfx::factory_and_round(dc) {
                Ok((factory, round)) => {
                    self.factory = Some(factory);
                    self.round = Some(round);
                    self.device_ptr = ptr;
                    self.dc = Some(dc.clone());
                }
                Err(err) => {
                    log::warn!("page canvas: cannot initialize Direct2D factory: {err}");
                    self.lose_device();
                }
            }
        } else {
            self.dc = Some(dc.clone());
        }
        false
    }

    /// Returns the attached Direct2D context or `D2DERR_RECREATE_TARGET` if none is attached.
    fn require_dc(&self) -> windows::core::Result<&ID2D1DeviceContext> {
        self.dc
            .as_ref()
            .ok_or_else(|| Error::from_hresult(D2DERR_RECREATE_TARGET))
    }

    /// Returns the attached Direct2D factory or `D2DERR_RECREATE_TARGET` if unavailable.
    fn require_factory(&self) -> windows::core::Result<&ID2D1Factory1> {
        self.factory
            .as_ref()
            .ok_or_else(|| Error::from_hresult(D2DERR_RECREATE_TARGET))
    }

    /// Returns the round stroke style or `D2DERR_RECREATE_TARGET` if unavailable.
    fn require_round(&self) -> windows::core::Result<&ID2D1StrokeStyle1> {
        self.round
            .as_ref()
            .ok_or_else(|| Error::from_hresult(D2DERR_RECREATE_TARGET))
    }
}

impl Host for WinHost {
    type Image = ID2D1Bitmap1;
    type Error = windows::core::Error;

    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn invalidate(&self) {
        self.events.redraw();
    }

    fn emit(&self, name: &str, data: JsonValue) {
        self.events.emit(name, data);
    }

    fn waker(&self) -> Wake {
        let events = self.events.clone();
        let jobs_pending = Arc::clone(&self.jobs_pending);
        Box::new(move || {
            jobs_pending.store(true, Ordering::Release);
            events.redraw();
        })
    }

    fn can_upload(&self) -> bool {
        self.dc.is_some()
    }

    // shortcut: RGB is swizzled to BGRA on the UI thread (about 0.1 ms a tile); move it to the worker if it shows in
    // K.24.
    fn upload(&mut self, pixmap: &Pixmap) -> Option<(ID2D1Bitmap1, usize)> {
        let dc = self.dc.as_ref()?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let n = pixmap.components();
        if width == 0 || height == 0 || n < 3 {
            return None;
        }
        let stride = pixmap.stride();
        let pixel_count = width as usize * height as usize;
        let mut bgra = vec![0xff_u8; pixel_count * 4];
        let mut dst_rows = bgra.chunks_exact_mut(width as usize * 4);
        for (src_row, dst_row) in pixmap
            .samples()
            .chunks_exact(stride)
            .take(height as usize)
            .zip(&mut dst_rows)
        {
            let (dst_chunks, _) = dst_row.as_chunks_mut::<4>();
            for (src, dst) in src_row[..width as usize * n]
                .chunks_exact(n)
                .zip(dst_chunks.iter_mut())
            {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
            }
        }
        match gfx::bitmap(dc, width, height, &bgra) {
            Ok(bitmap) => Some((bitmap, bgra.len())),
            Err(err) => {
                log::warn!("cannot make a tile bitmap: {err}");
                None
            }
        }
    }

    fn begin(&mut self, width: f32, height: f32) -> windows::core::Result<()> {
        let _ = self.require_dc()?;
        self.size = (width, height);
        Ok(())
    }

    fn end(&mut self) -> windows::core::Result<()> {
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        self.size = (width as f32, height as f32);
        if self.device_changed {
            self.device_changed = false;
            return Err(Error::from_hresult(D2DERR_RECREATE_TARGET));
        }
        Ok(())
    }

    fn lose_device(&mut self) {
        self.dc = None;
        self.factory = None;
        self.round = None;
        self.device_ptr = 0;
        self.device_changed = false;
    }

    fn clear(&mut self, rgb: u32) {
        // Never call `dc.Clear`: NSH owns the render target and clip stack. Fill the exact canvas rectangle instead.
        let Some(dc) = &self.dc else {
            return;
        };
        let full = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.size.0,
            bottom: self.size.1,
        };
        if let Err(err) = gfx::fill(dc, &full, rgb) {
            log::warn!("page canvas: cannot fill background: {err}");
        }
    }

    fn fill(&mut self, rect: &Rect, rgb: u32) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        gfx::fill(dc, &gfx::rect(rect), rgb)
    }

    fn image(&mut self, image: &ID2D1Bitmap1, dest: &Rect) {
        if let Some(dc) = &self.dc {
            gfx::draw_bitmap(dc, image, &gfx::rect(dest), 1.0);
        }
    }

    fn faint_image(&mut self, image: &ID2D1Bitmap1, dest: &Rect, opacity: f32) {
        if let Some(dc) = &self.dc {
            gfx::draw_bitmap(dc, image, &gfx::rect(dest), opacity.clamp(0.0, 1.0));
        }
    }

    fn highlight(&mut self, quads: &[Quad], rgb: u32, wash: Wash) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        let factory = self.require_factory()?;
        gfx::highlight(dc, factory, quads, rgb, wash)
    }

    fn band(
        &mut self,
        rect: &Rect,
        radius: f32,
        rgb: u32,
        wash: Wash,
    ) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        gfx::band(dc, &gfx::rect(rect), radius, rgb, wash)
    }

    fn rounded_triangle(
        &mut self,
        rect: &Rect,
        direction: TriangleDirection,
        corner_radius: f32,
        rgb: u32,
    ) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        let factory = self.require_factory()?;
        gfx::rounded_triangle(dc, factory, &gfx::rect(rect), direction, corner_radius, rgb)
    }

    fn dim_outside(
        &mut self,
        screen: &Rect,
        hole: &Rect,
        radius: f32,
        rgb: u32,
        alpha: f32,
    ) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        let factory = self.require_factory()?;
        gfx::dim_outside(
            dc,
            factory,
            &gfx::rect(screen),
            &gfx::rect(hole),
            radius,
            rgb,
            alpha,
        )
    }

    fn polyline(&mut self, points: &[InkPixel], rgb: u32) -> windows::core::Result<()> {
        let dc = self.require_dc()?;
        let round = self.require_round()?;
        gfx::polyline(dc, round, points, rgb)
    }

    fn shift_down(&self) -> bool {
        // SAFETY: plain Win32 query reading modifier key state on the UI thread.
        unsafe { GetKeyState(i32::from(VK_SHIFT.0)) < 0 }
    }

    fn ctrl_down(&self) -> bool {
        // SAFETY: plain Win32 query reading modifier key state on the UI thread.
        unsafe { GetKeyState(i32::from(VK_CONTROL.0)) < 0 }
    }

    fn since_input_ms(&self, input: u64) -> Option<f64> {
        Some((now() - input as i64) as f64 * 1000.0 / frequency() as f64)
    }
}

/// The performance counter now.
fn now() -> i64 {
    let mut count = 0;
    // SAFETY: plain query; `count` lives for the call. It cannot fail on Windows XP and later.
    let _ = unsafe { QueryPerformanceCounter(&mut count) };
    count
}

/// Performance counter ticks per second.
fn frequency() -> i64 {
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut frequency = 1;
        // SAFETY: plain query; `frequency` lives for the call.
        let _ = unsafe { QueryPerformanceFrequency(&mut frequency) };
        frequency.max(1)
    })
}
