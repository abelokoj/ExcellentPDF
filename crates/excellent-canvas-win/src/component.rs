//! The `excellent.page-canvas` NSH component on Windows (plan D5, K.8): the canvas window, its accessible name, and
//! the props and commands the shared canvas handles ([`excellent_canvas::page`]).

use std::cell::RefCell;
use std::rc::Rc;

use nsh_core::JsonValue;
use nsh_platform_windows::{ComponentContext, NativeComponent};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SetWindowTextW};
use windows::core::HSTRING;
use windows_numerics::Matrix3x2;

use crate::host::Canvas;
use crate::window::{self, PlacementSync};

/// A page canvas in an NSH view.
pub(crate) struct PageCanvas {
    hwnd: HWND,
    canvas: Rc<RefCell<Canvas>>,
    placement: Rc<PlacementSync>,
}

impl PageCanvas {
    /// A new canvas, or `None` (logged) when Windows can't make its window.
    pub(crate) fn new(context: ComponentContext) -> Option<Self> {
        match window::create(context) {
            Ok((hwnd, canvas, placement)) => Some(Self {
                hwnd,
                canvas,
                placement,
            }),
            Err(err) => {
                log::error!("cannot make the page canvas window: {err}");
                None
            }
        }
    }
}

impl NativeComponent for PageCanvas {
    fn draw(&self, dc: &ID2D1DeviceContext, size: (f32, f32)) {
        let mut transform = Matrix3x2::default();
        let (mut dpi_x, mut dpi_y) = (96.0_f32, 96.0_f32);
        // SAFETY: COM getters on the live Direct2D context NSH passes for this frame.
        unsafe {
            dc.GetTransform(&mut transform);
            dc.GetDpi(&mut dpi_x, &mut dpi_y);
        }
        if let Some(rect) = window::rect_from_draw(&transform, dpi_x, dpi_y, size) {
            self.placement.note_drawn(self.hwnd, rect);
        }
    }

    fn window(&self) -> Option<HWND> {
        Some(self.hwnd)
    }

    /// `label`: the window's accessible name. `visible`: whether the app wants the canvas window shown.
    /// The rest: [`excellent_canvas::page::PageCanvas::set_prop`].
    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name == "label" {
            let text = value.and_then(JsonValue::as_str).unwrap_or("");
            // SAFETY: a plain call on our live window; the string lives for it.
            if let Err(err) = unsafe { SetWindowTextW(self.hwnd, &HSTRING::from(text)) } {
                log::warn!("cannot name the page canvas window: {err}");
            }
            return;
        }
        if name == "visible" {
            let visible = value.and_then(JsonValue::as_bool).unwrap_or(true);
            self.placement.set_app_visible(self.hwnd, visible);
            return;
        }
        self.canvas.borrow_mut().set_prop(name, value);
    }

    /// See [`excellent_canvas::page::PageCanvas::command`].
    fn command(&mut self, name: &str, args: &JsonValue) {
        self.canvas.borrow_mut().command(name, args);
    }
}

impl Drop for PageCanvas {
    fn drop(&mut self) {
        // SAFETY: our window, destroyed once; WM_NCDESTROY releases the canvas reference it holds.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
