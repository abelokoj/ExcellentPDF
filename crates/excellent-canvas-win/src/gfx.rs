//! Presenting (plan K.8, item 2): Direct3D 11 and Direct2D on a DXGI flip-model swap chain with a waitable object,
//! drawn in device pixels (Direct2D at 96 dpi, so one unit is one pixel). The swap chain is shown through
//! DirectComposition so that, on Windows 11 24H2 and later, the compositor can draw the newest pen points itself
//! (a delegated ink trail) on its next refresh, before the app's own frame with them is ready.

use std::sync::Mutex;
use std::thread::JoinHandle;

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_ALTERNATE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LINE_JOIN_ROUND,
    D2D1_PRIMITIVE_BLEND_MIN, D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES1, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Brush, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Geometry, ID2D1SolidColorBrush, ID2D1StrokeStyle1,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice3, DCompositionInkTrailPoint, IDCompositionDelegatedInkTrail,
    IDCompositionDesktopDevice, IDCompositionInkTrailDevice, IDCompositionTarget,
    IDCompositionVisual, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain2,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::core::{Interface, Result};

/// The swap chain's flags; `ResizeBuffers` must be given the same.
const FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

/// How long a frame waits for the swap chain at most, in milliseconds.
const FRAME_WAIT_MS: u32 = 100;

/// The first Windows build whose delegated ink trail draws without flicker (Windows 11 24H2; Chromium's finding).
const INK_TRAIL_BUILD: u32 = 26100;

/// A colour, from 0xRRGGBB.
pub(crate) fn color(rgb: u32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// A rectangle for Direct2D.
pub(crate) fn rect(r: &excellent_canvas::Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x0,
        top: r.y0,
        right: r.x1,
        bottom: r.y1,
    }
}

/// The device, the drawing context and the swap chain of one canvas window.
/// How a highlight or the ruler's band lies over the pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wash {
    /// Darkest wins: on light paper the colour tints the paper and dark text stays as it is.
    Darkest,
    /// Laid over at [`WASH_OPACITY`]: on dark paper (R.14), where darkest-wins would leave the paper as it is.
    Translucent,
}

/// How much of a translucent wash shows: enough to see on dark paper, little enough to keep its light text readable.
const WASH_OPACITY: f32 = 0.3;

/// Sets `context` and `brush` up for a fill laid as `wash`; the caller sets the blend back to source-over.
///
/// # Safety
///
/// Called between `begin` and `end`, with a brush made by `context`.
unsafe fn set_wash(context: &ID2D1DeviceContext, brush: &ID2D1SolidColorBrush, wash: Wash) {
    // SAFETY: the caller draws between `begin` and `end` with this context's brush.
    unsafe {
        match wash {
            Wash::Darkest => context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_MIN),
            Wash::Translucent => brush.SetOpacity(WASH_OPACITY),
        }
    }
}

pub(crate) struct Gfx {
    /// Makes the geometry of a selection's highlight.
    factory: ID2D1Factory1,
    context: ID2D1DeviceContext,
    swap_chain: IDXGISwapChain2,
    waitable: HANDLE,
    /// The swap chain took a frame (its waitable object was signalled) that no frame has drawn into yet.
    acquired: bool,
    target: Option<ID2D1Bitmap1>,
    /// Round ends and joins, for ink.
    round: ID2D1StrokeStyle1,
    /// What shows the swap chain in the window; kept alive with it.
    _composition: Composition,
    /// The compositor's ink trail, where Windows has one that works.
    trail: Option<IDCompositionDelegatedInkTrail>,
}

/// The DirectComposition objects that put the swap chain (and the ink trail over it) in the window.
struct Composition {
    _device: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    _root: IDCompositionVisual2,
}

impl Gfx {
    /// Starts drawing into `hwnd`, `width` × `height` pixels.
    pub(crate) fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let device = take_device()?;
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: COM calls on objects made here; every pointer passed lives for the call.
        unsafe {
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let context = factory
                .CreateDevice(&dxgi)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // Tiles meet on whole pixels; antialiased edges would show a seam between them.
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;

            let dxgi_factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            // A composition swap chain stretches to its visual; the visual is always the window's size.
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: FLAGS.0 as u32,
                ..Default::default()
            };
            let swap_chain: IDXGISwapChain2 = dxgi_factory
                .CreateSwapChainForComposition(&device, &desc, None)?
                .cast()?;
            // One frame queued at most: input shows on the next refresh.
            swap_chain.SetMaximumFrameLatency(1)?;
            let waitable = swap_chain.GetFrameLatencyWaitableObject();

            let composition_device: IDCompositionDesktopDevice = DCompositionCreateDevice3(&dxgi)?;
            let target = composition_device.CreateTargetForHwnd(hwnd, true)?;
            let root = composition_device.CreateVisual()?;
            root.SetContent(&swap_chain)?;
            target.SetRoot(&root)?;
            let trail = ink_trail(&composition_device, &swap_chain, &root);
            composition_device.Commit()?;
            Ok(Self {
                factory,
                context,
                swap_chain,
                waitable,
                acquired: false,
                target: None,
                round,
                _composition: Composition {
                    _device: composition_device,
                    _target: target,
                    _root: root,
                },
                trail,
            })
        }
    }

    /// The window is now `width` × `height` pixels.
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.target = None;
        // SAFETY: the context lets go of the back buffer before the swap chain resizes it.
        unsafe {
            self.context.SetTarget(None);
            self.swap_chain.ResizeBuffers(
                0,
                width.max(1),
                height.max(1),
                DXGI_FORMAT_UNKNOWN,
                FLAGS,
            )
        }
    }

    /// Whether the swap chain takes a frame now, without waiting.
    pub(crate) fn frame_ready(&mut self) -> bool {
        if !self.acquired {
            // SAFETY: the handle is the swap chain's, open until `Drop`.
            self.acquired =
                unsafe { WaitForSingleObjectEx(self.waitable, 0, false) } == WAIT_OBJECT_0;
        }
        self.acquired
    }

    /// Waits until the swap chain takes a frame, then starts drawing one into the back buffer.
    pub(crate) fn begin(&mut self) -> Result<&ID2D1DeviceContext> {
        // SAFETY: the handle is the swap chain's, open until `Drop`; the COM calls are on live objects.
        unsafe {
            if !self.acquired {
                WaitForSingleObjectEx(self.waitable, FRAME_WAIT_MS, true);
            }
            self.acquired = false;
            if self.target.is_none() {
                let surface: IDXGISurface = self.swap_chain.GetBuffer(0)?;
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                };
                let target = self
                    .context
                    .CreateBitmapFromDxgiSurface(&surface, Some(std::ptr::from_ref(&props)))?;
                self.context.SetTarget(&target);
                self.target = Some(target);
            }
            self.context.BeginDraw();
        }
        Ok(&self.context)
    }

    /// Ends the frame and shows it on the next refresh. An error means the device is gone: make a new `Gfx`.
    pub(crate) fn end(&mut self) -> Result<()> {
        // SAFETY: a frame `begin` started.
        unsafe {
            self.context.EndDraw(None, None)?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    /// The stroke style with round ends and joins.
    pub(crate) fn round(&self) -> &ID2D1StrokeStyle1 {
        &self.round
    }

    /// Starts a new ink trail in `rgb`, dropping the old one.
    pub(crate) fn trail_start(&self, rgb: u32) {
        if let Some(trail) = &self.trail {
            let color = color(rgb);
            // SAFETY: a call on a live trail; `color` lives for it.
            if let Err(err) = unsafe { trail.StartNewTrail(&color) } {
                log::warn!("cannot start an ink trail: {err}");
            }
        }
    }

    /// Hands `points` (x, y and width, in pixels) to the compositor, which draws them on its next refresh. Returns
    /// the token that removes them, and every point before them, once a frame of ours shows them.
    pub(crate) fn trail_add(&self, points: &[(f32, f32, f32)]) -> Option<u32> {
        let trail = self.trail.as_ref()?;
        let points: Vec<DCompositionInkTrailPoint> = points
            .iter()
            .map(|&(x, y, width)| DCompositionInkTrailPoint {
                x,
                y,
                radius: width / 2.0,
            })
            .collect();
        // SAFETY: a call on a live trail; `points` lives for it.
        match unsafe { trail.AddTrailPoints(&points) } {
            Ok(token) => Some(token),
            Err(err) => {
                log::warn!("cannot add to the ink trail: {err}");
                None
            }
        }
    }

    /// Removes the trail's points up to the one `token` came with, from the next frame we present.
    pub(crate) fn trail_remove(&self, token: u32) {
        if let Some(trail) = &self.trail {
            // SAFETY: a call on a live trail.
            if let Err(err) = unsafe { trail.RemoveTrailPoints(token) } {
                log::warn!("cannot trim the ink trail: {err}");
            }
        }
    }

    /// Fills `quads` (corners in pixels, in order around each) with `rgb` the way a highlighter marks paper: each
    /// pixel keeps the darker of itself and `rgb`, channel by channel, so black text stays black and overlaps don't
    /// darken.
    pub(crate) fn highlight(&self, quads: &[[(f32, f32); 4]], rgb: u32, wash: Wash) -> Result<()> {
        if quads.is_empty() {
            return Ok(());
        }
        let at = |(x, y): (f32, f32)| windows_numerics::Vector2 { X: x, Y: y };
        // SAFETY: drawing between `begin` and `end`; the geometry, its sink and the brush are ours and live for the
        // calls, and the sink is closed before the geometry is drawn.
        unsafe {
            let geometry = self.factory.CreatePathGeometry()?;
            let sink = geometry.Open()?;
            for quad in quads {
                sink.BeginFigure(at(quad[0]), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(&[at(quad[1]), at(quad[2]), at(quad[3])]);
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            let brush = self.context.CreateSolidColorBrush(&color(rgb), None)?;
            self.context
                .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            set_wash(&self.context, &brush, wash);
            self.context
                .FillGeometry(&geometry, &brush, None::<&ID2D1Brush>);
            self.context
                .SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
            self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        }
        Ok(())
    }

    /// Fills `dest` with its corners rounded by `radius` pixels, laid as `wash` says, as [`Self::highlight`] does:
    /// the reading ruler's band (R.20).
    pub(crate) fn band(&self, dest: &D2D_RECT_F, radius: f32, rgb: u32, wash: Wash) -> Result<()> {
        let rounded = D2D1_ROUNDED_RECT {
            rect: *dest,
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: drawing between `begin` and `end`; the brush and `rounded` live for the calls.
        unsafe {
            let brush = self.context.CreateSolidColorBrush(&color(rgb), None)?;
            self.context
                .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            set_wash(&self.context, &brush, wash);
            self.context.FillRoundedRectangle(&rounded, &brush);
            self.context
                .SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
            self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        }
        Ok(())
    }

    /// Lays `rgb` at `alpha` (0 to 1) over all of `screen` but `hole`, whose corners are rounded by `radius`
    /// pixels: the dimmed page around focus mode's paragraph (R.19) and the inverted ruler's band (R.20).
    pub(crate) fn dim_outside(
        &self,
        screen: &D2D_RECT_F,
        hole: &D2D_RECT_F,
        radius: f32,
        rgb: u32,
        alpha: f32,
    ) -> Result<()> {
        let rounded = D2D1_ROUNDED_RECT {
            rect: *hole,
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: drawing between `begin` and `end`; the geometries, their group and the brush are ours and live for
        // the calls.
        unsafe {
            let outer: ID2D1Geometry = self.factory.CreateRectangleGeometry(screen)?.cast()?;
            let inner: ID2D1Geometry = self
                .factory
                .CreateRoundedRectangleGeometry(&rounded)?
                .cast()?;
            // Alternate filling leaves the inner shape, inside the outer one, empty.
            let group = self
                .factory
                .CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])?;
            let shade = D2D1_COLOR_F {
                a: alpha.clamp(0.0, 1.0),
                ..color(rgb)
            };
            let brush = self.context.CreateSolidColorBrush(&shade, None)?;
            self.context
                .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.context
                .FillGeometry(&group, &brush, None::<&ID2D1Brush>);
            self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        }
        Ok(())
    }

    /// A bitmap of `width` × `height` BGRA pixels (opaque), for drawing.
    pub(crate) fn bitmap(&self, width: u32, height: u32, bgra: &[u8]) -> Result<ID2D1Bitmap1> {
        assert_eq!(
            bgra.len(),
            (width * height * 4) as usize,
            "a full BGRA image"
        );
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        // SAFETY: `bgra` holds `height` rows of `width * 4` bytes and lives for the call, which copies it.
        unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U { width, height },
                Some(bgra.as_ptr().cast()),
                width * 4,
                &props,
            )
        }
    }
}

/// Fills the whole target with `rgb`.
pub(crate) fn clear(context: &ID2D1DeviceContext, rgb: u32) {
    let color = color(rgb);
    // SAFETY: drawing between `begin` and `end`; `color` lives for the call.
    unsafe { context.Clear(Some(std::ptr::from_ref(&color))) };
}

/// Fills `dest` with `rgb`.
pub(crate) fn fill(context: &ID2D1DeviceContext, dest: &D2D_RECT_F, rgb: u32) -> Result<()> {
    // SAFETY: drawing between `begin` and `end`; the brush and `dest` live for the calls.
    unsafe {
        let brush = context.CreateSolidColorBrush(&color(rgb), None)?;
        context.FillRectangle(dest, &brush);
    }
    Ok(())
}

/// Draws `bitmap` stretched into `dest`.
pub(crate) fn draw_bitmap(context: &ID2D1DeviceContext, bitmap: &ID2D1Bitmap1, dest: &D2D_RECT_F) {
    // SAFETY: drawing between `begin` and `end` with live objects.
    unsafe {
        context.DrawBitmap(
            bitmap,
            Some(std::ptr::from_ref(dest)),
            1.0,
            D2D1_INTERPOLATION_MODE_LINEAR,
            None,
            None,
        );
    }
}

/// Draws a line through `points` (x, y and width, in pixels) in `rgb`, antialiased: each segment as wide as the mean
/// of its ends, with round ends so the segments join smoothly; a single point is a dot.
pub(crate) fn polyline(
    context: &ID2D1DeviceContext,
    round: &ID2D1StrokeStyle1,
    points: &[(f32, f32, f32)],
    rgb: u32,
) -> Result<()> {
    let at = |(x, y, _): (f32, f32, f32)| windows_numerics::Vector2 { X: x, Y: y };
    // SAFETY: drawing between `begin` and `end`; the brush, style and ellipse live for the calls.
    unsafe {
        let brush = context.CreateSolidColorBrush(&color(rgb), None)?;
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        if let [only] = points {
            let ellipse = D2D1_ELLIPSE {
                point: at(*only),
                radiusX: only.2 / 2.0,
                radiusY: only.2 / 2.0,
            };
            context.FillEllipse(&ellipse, &brush);
        }
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            context.DrawLine(at(a), at(b), &brush, (a.2 + b.2) / 2.0, round);
        }
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

impl Drop for Gfx {
    fn drop(&mut self) {
        // SAFETY: the swap chain's waitable handle is ours to close, once.
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}

/// The compositor's ink trail over `swap_chain`, in a visual above `root`'s, or `None` where Windows has none or its
/// trail flickers (before Windows 11 24H2).
fn ink_trail(
    device: &IDCompositionDesktopDevice,
    swap_chain: &IDXGISwapChain2,
    root: &IDCompositionVisual2,
) -> Option<IDCompositionDelegatedInkTrail> {
    let build = windows_build();
    if build < INK_TRAIL_BUILD {
        log::info!("no compositor ink trail on Windows build {build}; the app draws all ink");
        return None;
    }
    let made = (|| -> Result<IDCompositionDelegatedInkTrail> {
        let ink_device: IDCompositionInkTrailDevice = device.cast()?;
        // SAFETY: COM calls on live objects.
        unsafe {
            let trail = ink_device.CreateDelegatedInkTrailForSwapChain(swap_chain)?;
            let visual = device.CreateVisual()?;
            visual.SetContent(&trail)?;
            root.AddVisual(&visual, true, None::<&IDCompositionVisual>)?;
            Ok(trail)
        }
    })();
    match made {
        Ok(trail) => {
            log::info!("ink: the compositor draws the newest pen points");
            Some(trail)
        }
        Err(err) => {
            log::warn!("no compositor ink trail ({err}); the app draws all ink");
            None
        }
    }
}

/// The Windows build number (what `GetVersionEx` won't tell an app without a compatibility manifest).
fn windows_build() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is sized as the call requires and lives for it.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() {
        info.dwBuildNumber
    } else {
        0
    }
}

/// A Direct3D 11 device that Direct2D can draw with: the GPU's, or WARP (software) when there is none.
/// A Direct3D device made on another thread.
struct Made(Result<ID3D11Device>);

// SAFETY: a Direct3D 11 device is free-threaded (it was made without `D3D11_CREATE_DEVICE_SINGLETHREADED`), so it
// may move to the UI thread.
unsafe impl Send for Made {}

/// The device [`prewarm`] is making.
static PREWARMED: Mutex<Option<JoinHandle<Made>>> = Mutex::new(None);

/// Starts making the Direct3D device on a background thread, so the first frame doesn't wait for the driver to load
/// (about 160 ms on the maintainer's machine). [`Gfx::new`] takes it.
pub(crate) fn prewarm() {
    let spawned = std::thread::Builder::new()
        .name("canvas-device".to_owned())
        .spawn(|| Made(device()));
    match spawned {
        Ok(handle) => *PREWARMED.lock().expect("lock poisoned") = Some(handle),
        Err(err) => log::warn!("cannot start a thread to make the Direct3D device: {err}"),
    }
}

/// The device [`prewarm`] made (waiting for it if it isn't ready), or a new one.
fn take_device() -> Result<ID3D11Device> {
    let handle = PREWARMED.lock().expect("lock poisoned").take();
    match handle.map(JoinHandle::join) {
        Some(Ok(Made(device))) => device,
        Some(Err(_)) => {
            log::warn!("the thread making the Direct3D device panicked; making it again");
            device()
        }
        None => device(),
    }
}

fn device() -> Result<ID3D11Device> {
    let make = |driver: D3D_DRIVER_TYPE| {
        let mut device = None;
        // SAFETY: plain creation call; `device` outlives it.
        unsafe {
            D3D11CreateDevice(
                None,
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .map(|()| device.expect("D3D11CreateDevice succeeded with a device"))
    };
    make(D3D_DRIVER_TYPE_HARDWARE).or_else(|err| {
        log::warn!("no hardware Direct3D device ({err}); drawing with WARP");
        make(D3D_DRIVER_TYPE_WARP)
    })
}
