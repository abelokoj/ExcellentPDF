//! The Windows side of the page canvas (plan D5, K.8): the canvas window as the shared canvas's [`Host`], drawing
//! with Direct2D ([`Gfx`]), telling screen readers what changed (`uia`) and waking the window from workers.

use std::sync::OnceLock;

use excellent_canvas::Rect;
use excellent_canvas::page::host::{InkPixel, Quad, Wake, Wash};
use excellent_canvas::page::{Host, PageCanvas};
use excellent_mupdf::Pixmap;
use nsh_core::JsonValue;
use nsh_platform_windows::ComponentContext;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PostMessageW, WM_APP};

use crate::gfx::{self, Gfx};

/// Posted to the canvas window when a background job is done.
pub(crate) const WM_JOB_DONE: u32 = WM_APP + 1;

/// The page canvas as Windows runs it.
pub(crate) type Canvas = PageCanvas<WinHost>;

/// The canvas window and what draws into it.
pub(crate) struct WinHost {
    hwnd: HWND,
    /// Made on the first frame, dropped when the device is lost.
    gfx: Option<Gfx>,
    /// Where the canvas tells the app about the document and the page shown.
    events: ComponentContext,
}

impl WinHost {
    /// The host of the canvas drawing into `hwnd`, telling the app through `events`.
    pub(crate) fn new(hwnd: HWND, events: ComponentContext) -> Self {
        Self {
            hwnd,
            gfx: None,
            events,
        }
    }

    /// The device, which [`Host::begin`] made.
    fn gfx(&self) -> &Gfx {
        self.gfx.as_ref().expect("drawn between begin and end")
    }
}

impl Host for WinHost {
    type Image = ID2D1Bitmap1;
    type Error = windows::core::Error;

    fn size(&self) -> (f32, f32) {
        let mut rect = RECT::default();
        // SAFETY: a plain call on our live window.
        let _ = unsafe { GetClientRect(self.hwnd, &mut rect) };
        (
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        )
    }

    fn invalidate(&self) {
        // SAFETY: a plain call on our live window.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn emit(&self, name: &str, data: JsonValue) {
        self.events.emit(name, data);
    }

    fn waker(&self) -> Wake {
        // The window as an integer: `HWND` isn't `Send`.
        let hwnd = self.hwnd.0 as isize;
        Box::new(move || {
            // SAFETY: a plain post; if the window is gone the call fails and nothing happens.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut _)),
                    WM_JOB_DONE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        })
    }

    // shortcut: RGB is swizzled to BGRA on the UI thread (about 0.1 ms a tile); move it to the worker if it shows in
    // K.24.
    fn upload(&mut self, pixmap: &Pixmap) -> Option<(ID2D1Bitmap1, usize)> {
        let gfx = self.gfx.as_ref()?;
        let (width, height) = (pixmap.width(), pixmap.height());
        if width == 0 || height == 0 {
            return None;
        }
        let n = pixmap.components();
        let stride = pixmap.stride();
        let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
        for row in pixmap.samples().chunks_exact(stride).take(height as usize) {
            for px in row[..width as usize * n].chunks_exact(n) {
                bgra.extend_from_slice(&[px[2], px[1], px[0], 0xff]);
            }
        }
        match gfx.bitmap(width, height, &bgra) {
            Ok(bitmap) => Some((bitmap, bgra.len())),
            Err(err) => {
                log::warn!("cannot make a tile bitmap: {err}");
                None
            }
        }
    }

    fn can_upload(&self) -> bool {
        self.gfx.is_some()
    }

    fn begin(&mut self, width: f32, height: f32) -> windows::core::Result<()> {
        if self.gfx.is_none() {
            log::debug!("page canvas: new device, {width} x {height}");
            self.gfx = Some(Gfx::new(self.hwnd, width as u32, height as u32)?);
        }
        let gfx = self.gfx.as_mut().expect("made above");
        gfx.begin().map(|_| ())
    }

    fn end(&mut self) -> windows::core::Result<()> {
        match &mut self.gfx {
            Some(gfx) => gfx.end(),
            None => Ok(()),
        }
    }

    fn resize(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        match &mut self.gfx {
            Some(gfx) => gfx.resize(width, height),
            None => Ok(()),
        }
    }

    fn lose_device(&mut self) {
        self.gfx = None;
    }

    fn clear(&mut self, rgb: u32) {
        gfx::clear(self.gfx().context(), rgb);
    }

    fn fill(&mut self, rect: &Rect, rgb: u32) -> windows::core::Result<()> {
        gfx::fill(self.gfx().context(), &gfx::rect(rect), rgb)
    }

    fn image(&mut self, image: &ID2D1Bitmap1, dest: &Rect) {
        gfx::draw_bitmap(self.gfx().context(), image, &gfx::rect(dest));
    }

    fn highlight(&mut self, quads: &[Quad], rgb: u32, wash: Wash) -> windows::core::Result<()> {
        self.gfx().highlight(quads, rgb, wash)
    }

    fn band(
        &mut self,
        rect: &Rect,
        radius: f32,
        rgb: u32,
        wash: Wash,
    ) -> windows::core::Result<()> {
        self.gfx().band(&gfx::rect(rect), radius, rgb, wash)
    }

    fn dim_outside(
        &mut self,
        screen: &Rect,
        hole: &Rect,
        radius: f32,
        rgb: u32,
        alpha: f32,
    ) -> windows::core::Result<()> {
        self.gfx()
            .dim_outside(&gfx::rect(screen), &gfx::rect(hole), radius, rgb, alpha)
    }

    fn polyline(&mut self, points: &[InkPixel], rgb: u32) -> windows::core::Result<()> {
        let gfx = self.gfx();
        gfx::polyline(gfx.context(), gfx.round(), points, rgb)
    }

    fn content_changed(&self) {
        crate::uia::content_changed(self.hwnd);
    }

    fn selection_changed(&self) {
        crate::uia::selection_changed(self.hwnd);
    }

    fn trail_start(&mut self, rgb: u32) {
        if let Some(gfx) = &self.gfx {
            gfx.trail_start(rgb);
        }
    }

    fn trail_add(&mut self, points: &[InkPixel]) -> Option<u32> {
        self.gfx.as_ref()?.trail_add(points)
    }

    fn trail_remove(&mut self, token: u32) {
        if let Some(gfx) = &self.gfx {
            gfx.trail_remove(token);
        }
    }

    fn frame_ready(&mut self) -> bool {
        self.gfx.as_mut().is_none_or(Gfx::frame_ready)
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
