//! The `ecclesia.page-canvas` NSH component on Windows (plan D5, K.8): draws directly into NSH's
//! Direct2D context and routes NSH pointer, wheel, cursor, prop, and command callbacks into the
//! shared [`ecclesia_canvas::page::PageCanvas`].

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ecclesia_canvas::page::host::PopoverWheel;
use ecclesia_canvas::page::{ClickLimits, Cursor as CanvasCursor};
use nsh_core::{Cursor as NshCursor, JsonValue};
use nsh_platform_windows::{ComponentContext, NativeComponent, Pointer, PointerKind, Wheel};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_PRIMITIVE_BLEND_SOURCE_OVER, ID2D1DeviceContext,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK};
use windows_numerics::Matrix3x2;

use crate::gfx;
use crate::host::{Canvas, WinHost};
use crate::window_bridge;

/// Zoom factor applied per Ctrl+wheel notch.
const NOTCH_ZOOM: f32 = 1.1;

/// Initial pixels-per-point before the first frame reports the window's actual DPI.
const DEFAULT_PPP: f32 = 96.0 / 72.0;

/// A page canvas rendered directly inside an NSH view's Direct2D pass.
pub(crate) struct PageCanvas {
    context: ComponentContext,
    canvas: RefCell<Canvas>,
    jobs_pending: Arc<AtomicBool>,
    scale: Cell<(f32, f32)>,
    last_metrics: Cell<(u32, u32, u32)>,
}

impl PageCanvas {
    /// Creates a new page canvas component bound to NSH's `context`.
    pub(crate) fn new(context: ComponentContext) -> Self {
        let jobs_pending = Arc::new(AtomicBool::new(false));
        let host = WinHost::new(context.clone(), Arc::clone(&jobs_pending));
        let canvas = RefCell::new(Canvas::new(host, DEFAULT_PPP));
        Self {
            context,
            canvas,
            jobs_pending,
            scale: Cell::new((1.0, 1.0)),
            last_metrics: Cell::new((0, 0, 0)),
        }
    }
}

impl NativeComponent for PageCanvas {
    fn draw(&self, dc: &ID2D1DeviceContext, size: (f32, f32)) {
        window_bridge::ensure_window_hooks();
        if !(size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0) {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96.0_f32, 96.0_f32);
        // SAFETY: COM getter on the live Direct2D context NSH passes for this frame.
        unsafe {
            dc.GetDpi(&mut dpi_x, &mut dpi_y);
        }
        let scale_x = if dpi_x.is_finite() && dpi_x > 0.0 {
            (dpi_x / 96.0).max(0.5)
        } else {
            1.0
        };
        let scale_y = if dpi_y.is_finite() && dpi_y > 0.0 {
            (dpi_y / 96.0).max(0.5)
        } else {
            1.0
        };
        let ppp = (scale_x * 96.0) / 72.0;
        let phys_w = (size.0 * scale_x).round().max(1.0);
        let phys_h = (size.1 * scale_y).round().max(1.0);
        let phys_w_u32 = phys_w as u32;
        let phys_h_u32 = phys_h as u32;
        self.scale.set((scale_x, scale_y));

        let Ok(mut canvas) = self.canvas.try_borrow_mut() else {
            return;
        };
        let device_changed = canvas.host_mut().attach_dc(dc);
        let metrics = (phys_w_u32, phys_h_u32, ppp.to_bits());
        if device_changed || self.last_metrics.get() != metrics {
            self.last_metrics.set(metrics);
            canvas.resize(phys_w_u32, phys_h_u32, ppp);
            if device_changed {
                let _ = canvas.host_mut().attach_dc(dc);
                canvas.host_mut().set_size(phys_w, phys_h);
            }
        } else {
            canvas.host_mut().set_size(phys_w, phys_h);
        }
        if self.jobs_pending.swap(false, Ordering::AcqRel) {
            canvas.jobs_done();
        }

        let mut prev_transform = Matrix3x2::default();
        // SAFETY: saving and configuring Direct2D state on the live context NSH passed for this frame.
        let (prev_aa, prev_blend) = unsafe {
            dc.GetTransform(&mut prev_transform);
            let prev_aa = dc.GetAntialiasMode();
            let prev_blend = dc.GetPrimitiveBlend();
            let pixel_xform = gfx::pixel_transform(&prev_transform, scale_x, scale_y);
            dc.SetTransform(&pixel_xform);
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
            (prev_aa, prev_blend)
        };

        canvas.paint();

        // SAFETY: restoring the exact Direct2D context state NSH held prior to `draw`.
        unsafe {
            dc.SetTransform(&prev_transform);
            dc.SetAntialiasMode(prev_aa);
            dc.SetPrimitiveBlend(prev_blend);
        }
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name == "label" || name == "visible" {
            return;
        }
        self.canvas.borrow_mut().set_prop(name, value);
        self.context.redraw();
    }

    fn command(&mut self, name: &str, args: &JsonValue) {
        self.canvas.borrow_mut().command(name, args);
        self.context.redraw();
    }

    fn hit(&self, _x: f32, _y: f32, size: (f32, f32)) -> bool {
        size.0 > 0.0 && size.1 > 0.0
    }

    fn pointer(&mut self, event: &Pointer) -> bool {
        let (sx, sy) = self.scale.get();
        let px_x = (event.x * sx).round() as i32;
        let px_y = (event.y * sy).round() as i32;
        let mut canvas = self.canvas.borrow_mut();
        match event.kind {
            PointerKind::Down => {
                canvas.pressed();
                if canvas.stop_pan() {
                    return true;
                }
                if event.ctrl && canvas.open_elsewhere(px_x, px_y) {
                    return true;
                }
                let _ = canvas.mouse_down(px_x, px_y, tick_ms(), click_limits());
            }
            PointerKind::Move => {
                canvas.mouse_moved(Some((px_x, px_y)));
            }
            PointerKind::Up => {
                let _ = canvas.mouse_up(px_x, px_y);
            }
            PointerKind::Cancel => {
                canvas.capture_lost();
            }
            PointerKind::Leave => {
                canvas.mouse_moved(None);
            }
            _ => {}
        }
        true
    }

    fn wheel(&mut self, event: &Wheel) -> bool {
        let (sx, sy) = self.scale.get();
        let px_x = event.x * sx;
        let px_y = event.y * sy;
        let mut canvas = self.canvas.borrow_mut();
        let popover_action = if event.ctrl {
            PopoverWheel::Zoom {
                notches: -event.notches_y,
            }
        } else {
            PopoverWheel::Scroll {
                delta_y: event.dy * sy,
            }
        };
        if canvas.popover_wheel(px_x, px_y, popover_action) {
            return true;
        }
        if event.ctrl {
            canvas.zoom_about(NOTCH_ZOOM.powf(-event.notches_y), px_x, px_y);
        } else if event.shift && event.dx == 0.0 {
            canvas.scroll_wheel(event.dy * sx, 0.0);
        } else {
            canvas.scroll_wheel(event.dx * sx, event.dy * sy);
        }
        true
    }

    fn cursor(&self, x: f32, y: f32) -> Option<NshCursor> {
        let canvas = self.canvas.try_borrow().ok()?;
        let cursor = if x.is_finite() && y.is_finite() {
            let (sx, sy) = self.scale.get();
            canvas.cursor_at((x * sx).round() as i32, (y * sy).round() as i32)
        } else {
            canvas.cursor()
        };
        Some(match cursor {
            CanvasCursor::Arrow => NshCursor::Default,
            CanvasCursor::Hand => NshCursor::Pointer,
            CanvasCursor::Text => NshCursor::Text,
            CanvasCursor::Pan => NshCursor::Move,
            CanvasCursor::Cross => NshCursor::Crosshair,
            CanvasCursor::ResizeEw => NshCursor::ColResize,
            CanvasCursor::ResizeNs => NshCursor::RowResize,
            CanvasCursor::ResizeNwse => NshCursor::NwseResize,
            CanvasCursor::ResizeNesw => NshCursor::NeswResize,
        })
    }
}

/// System tick count in milliseconds, used for multi-click detection.
fn tick_ms() -> u32 {
    // SAFETY: plain Win32 query returning system uptime in milliseconds.
    unsafe { GetTickCount() }
}

/// System double-click time and spatial slop limits.
fn click_limits() -> ClickLimits {
    // SAFETY: plain Win32 queries of the user's double-click preferences.
    unsafe {
        ClickLimits {
            time: GetDoubleClickTime(),
            width: GetSystemMetrics(SM_CXDOUBLECLK),
            height: GetSystemMetrics(SM_CYDOUBLECLK),
        }
    }
}
