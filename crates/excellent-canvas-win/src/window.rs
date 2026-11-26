//! The canvas's own window (plan K.8, item 1): a child of the app window that NSH places over the canvas's view, and
//! its window procedure: size, paint, wheel, pen, links and text under the mouse, middle-click scrolling, DPI and
//! finished jobs.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Once;

use excellent_canvas::page::gestures::Verdict;
use excellent_canvas::page::{ClickLimits, Cursor, PenSample, Tilt};
use nsh_platform_windows::ComponentContext;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetWindowRgnBox, RGN_ERROR, ScreenToClient, SetWindowRgn, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    FEEDBACK_PEN_BARRELVISUALIZATION, FEEDBACK_PEN_DOUBLETAP, FEEDBACK_PEN_PRESSANDHOLD,
    FEEDBACK_PEN_RIGHTTAP, FEEDBACK_PEN_TAP, SetWindowFeedbackSetting, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GA_ROOT, GWLP_USERDATA, GetAncestor, GetClientRect,
    GetMessageTime, GetParent, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HTCLIENT,
    HWND_TOP, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL,
    IMAGE_ICON, IsWindowVisible, LR_SHARED, LoadCursorW, LoadImageW, PostMessageW,
    RegisterClassExW, SM_CXDOUBLECLK, SM_CXICON, SM_CXSMICON, SM_CYDOUBLECLK, SW_HIDE,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW, SetCursor, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_APP, WM_CAPTURECHANGED,
    WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND, WM_GETOBJECT, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT,
    WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE,
    WM_SETCURSOR, WM_SETICON, WM_SIZE, WM_XBUTTONUP, WNDCLASSEXW, WS_CLIPCHILDREN, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, Result, w};
use windows_numerics::Matrix3x2;

use crate::host::{Canvas, WM_CANVAS_PAINT, WM_JOB_DONE, WinHost, request_paint};
use crate::pointer::{self, Device, Sample};

const CLASS: windows::core::PCWSTR = w!("ExcellentPDF.PageCanvas");

/// Posted after NSH draws the canvas view, to reconcile child-window bounds and clip region.
const WM_SYNC_PLACEMENT: u32 = WM_APP + 3;

/// Per-window data stored in `GWLP_USERDATA` until `WM_NCDESTROY`.
struct WindowData {
    canvas: Rc<RefCell<Canvas>>,
    placement: Rc<PlacementSync>,
    paint_posted: Rc<Cell<bool>>,
}

/// Reconciles the canvas child HWND's bounds, visibility and clip region with the visible
/// `AppWindow` NSH actually painted it into.
///
/// In NSH 0.7.0, `native::Placement::apply` clips child HWNDs against the hidden host surface's
/// `pixel_size()` (default 1280x800 DIPs) instead of the visible `AppWindow`'s client size. On any
/// larger window, NSH installs a truncated `SetWindowRgn` that chops off the right and bottom of
/// the PDF; on panes positioned past the host's edge (such as split panes), NSH hides the child
/// HWND altogether. Capturing the exact rectangle from `NativeComponent::draw` and reconciling on
/// `WM_SYNC_PLACEMENT` restores full-window display while honoring intentional hides.
pub(crate) struct PlacementSync {
    app_visible: Cell<bool>,
    /// Latest parent-client pixel rectangle painted by NSH (`NativeComponent::draw`); never cleared
    /// by temporary dialog hides so closing a dialog can restore the canvas HWND at once.
    last_rect: Cell<Option<RECT>>,
    posted: Cell<bool>,
    paint_posted: Rc<Cell<bool>>,
}

impl PlacementSync {
    pub(crate) fn new(paint_posted: Rc<Cell<bool>>) -> Self {
        Self {
            app_visible: Cell::new(true),
            last_rect: Cell::new(None),
            posted: Cell::new(false),
            paint_posted,
        }
    }

    /// Records the exact parent-client pixel rectangle NSH painted the canvas at this frame, and
    /// schedules reconciliation once NSH finishes `items_placed`.
    pub(crate) fn note_drawn(&self, hwnd: HWND, rect: RECT) {
        self.last_rect.set(Some(rect));
        self.schedule(hwnd);
    }

    /// Updates whether the app's `visible` prop wants the canvas shown; hiding takes effect at once.
    pub(crate) fn set_app_visible(&self, hwnd: HWND, visible: bool) {
        let prev = self.app_visible.replace(visible);
        if !visible {
            // SAFETY: plain call on our live child window.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        } else {
            // SAFETY: plain query on our live child window.
            let currently_visible = unsafe { IsWindowVisible(hwnd).as_bool() };
            if (!prev || !currently_visible) && self.last_rect.get().is_some() {
                self.schedule(hwnd);
            }
        }
    }

    /// Coalesces a `WM_SYNC_PLACEMENT` post onto `hwnd`'s message queue.
    fn schedule(&self, hwnd: HWND) {
        if !self.posted.replace(true) {
            // SAFETY: plain post to our live window on this thread.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_SYNC_PLACEMENT, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Clears any truncated clip region NSH set and ensures `hwnd` is positioned and shown at
    /// `last_rect` inside its visible adopted parent window.
    pub(crate) fn reconcile(&self, hwnd: HWND) {
        self.posted.set(false);
        if !self.app_visible.get() {
            // SAFETY: plain call on our live child window.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            return;
        }
        let Some(target) = self.last_rect.get() else {
            return;
        };
        // SAFETY: plain Win32 queries and updates on our live child window and its parent.
        unsafe {
            let Ok(parent) = GetParent(hwnd) else {
                return;
            };
            if parent.0.is_null() || !IsWindowVisible(parent).as_bool() {
                return;
            }
            ensure_window_icon(parent);
            let mut rgn_box = RECT::default();
            let had_rgn = GetWindowRgnBox(hwnd, &mut rgn_box) != RGN_ERROR;
            if had_rgn {
                let _ = SetWindowRgn(hwnd, None, true);
            }
            let mut current = RECT::default();
            let _ = GetWindowRect(hwnd, &mut current);
            let mut tl = POINT {
                x: current.left,
                y: current.top,
            };
            let mut br = POINT {
                x: current.right,
                y: current.bottom,
            };
            let _ = ScreenToClient(parent, &mut tl);
            let _ = ScreenToClient(parent, &mut br);
            let current_in_parent = RECT {
                left: tl.x,
                top: tl.y,
                right: br.x,
                bottom: br.y,
            };
            let visible = IsWindowVisible(hwnd).as_bool();
            let moved_or_shown = !visible || current_in_parent != target;
            if moved_or_shown {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    target.left,
                    target.top,
                    (target.right - target.left).max(1),
                    (target.bottom - target.top).max(1),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER,
                );
            }
            if had_rgn || moved_or_shown {
                request_paint(hwnd, &self.paint_posted);
            }
        }
    }
}

/// Ensures the top-level ancestor of `hwnd` carries our embedded app icon (`RT_GROUP_ICON` 1)
/// for both `ICON_BIG` and `ICON_SMALL` at the window's DPI, once per root `HWND`.
fn ensure_window_icon(hwnd: HWND) {
    thread_local! {
        static APPLIED: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
    }
    // SAFETY: plain Win32 queries on our live parent/root window handle.
    let root = unsafe {
        let ancestor = GetAncestor(hwnd, GA_ROOT);
        if ancestor.0.is_null() { hwnd } else { ancestor }
    };
    let already_set = APPLIED.with(|set| !set.borrow_mut().insert(root.0 as isize));
    if already_set {
        return;
    }
    // SAFETY: loading shared icon resources from this module (`id = 1`) and sending `WM_SETICON`
    // synchronously to a window on this UI thread.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let dpi = GetDpiForWindow(root);
        for (metric, slot) in [(SM_CXICON, ICON_BIG), (SM_CXSMICON, ICON_SMALL)] {
            let side = GetSystemMetricsForDpi(metric, dpi).max(16);
            if let Ok(handle) = LoadImageW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance::<u16>(1)),
                IMAGE_ICON,
                side,
                side,
                LR_SHARED,
            ) {
                let _ = SendMessageW(
                    root,
                    WM_SETICON,
                    Some(WPARAM(slot as usize)),
                    Some(LPARAM(handle.0 as isize)),
                );
            }
        }
    }
}

/// Converts a Direct2D draw transform, DPI and local DIP size into parent-window physical pixels,
/// matching NSH's own pixel rounding.
pub(crate) fn rect_from_draw(
    transform: &Matrix3x2,
    dpi_x: f32,
    dpi_y: f32,
    size: (f32, f32),
) -> Option<RECT> {
    let (w, h) = size;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0)
        || !(dpi_x.is_finite() && dpi_y.is_finite() && dpi_x > 0.0 && dpi_y > 0.0)
    {
        return None;
    }
    let corners = [(0.0_f32, 0.0_f32), (w, 0.0), (0.0, h), (w, h)];
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (lx, ly) in corners {
        let tx = lx * transform.M11 + ly * transform.M21 + transform.M31;
        let ty = lx * transform.M12 + ly * transform.M22 + transform.M32;
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    let (sx, sy) = (dpi_x / 96.0, dpi_y / 96.0);
    let rect = RECT {
        left: (min_x * sx).round() as i32,
        top: (min_y * sy).round() as i32,
        right: (max_x * sx).round() as i32,
        bottom: (max_y * sy).round() as i32,
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

/// One wheel notch, as Windows counts it.
const NOTCH: f32 = 120.0;
/// How far one notch scrolls, in pixels at 96 dpi (about three lines of text, as browsers do).
const NOTCH_SCROLL: f32 = 100.0;
/// How much one notch with Ctrl zooms.
const NOTCH_ZOOM: f32 = 1.1;
/// `MK_SHIFT` and `MK_CONTROL` in a mouse message's key state.
const MK_SHIFT: usize = 0x0004;
const MK_CONTROL: usize = 0x0008;

/// Makes the canvas window, hidden and without a parent: NSH adopts it as a child of the app window. The canvas
/// tells the app what happens through `events`.
pub(crate) fn create(
    events: ComponentContext,
) -> Result<(HWND, Rc<RefCell<Canvas>>, Rc<PlacementSync>)> {
    // SAFETY: plain Win32 calls; the class's procedure is `proc` below, and the window's user data holds one owned
    // `WindowData`, released in WM_NCDESTROY.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 {
                log::error!("cannot register the page canvas's window class");
            }
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!(""),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        // No ripples or press-and-hold circles under the pen: they cost frames and hide the ink.
        let off = BOOL(0);
        for kind in [
            FEEDBACK_PEN_TAP,
            FEEDBACK_PEN_DOUBLETAP,
            FEEDBACK_PEN_PRESSANDHOLD,
            FEEDBACK_PEN_RIGHTTAP,
            FEEDBACK_PEN_BARRELVISUALIZATION,
        ] {
            let _ = SetWindowFeedbackSetting(
                hwnd,
                kind,
                0,
                size_of::<BOOL>() as u32,
                Some(std::ptr::from_ref(&off).cast()),
            );
        }
        let ppp = pixels_per_point(GetDpiForWindow(hwnd));
        let paint_posted = Rc::new(Cell::new(false));
        let canvas = Rc::new(RefCell::new(Canvas::new(
            WinHost::new(hwnd, events, Rc::clone(&paint_posted)),
            ppp,
        )));
        let placement = Rc::new(PlacementSync::new(Rc::clone(&paint_posted)));
        crate::uia::register(hwnd, &canvas);
        let data = Box::new(WindowData {
            canvas: Rc::clone(&canvas),
            placement: Rc::clone(&placement),
            paint_posted,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        Ok((hwnd, canvas, placement))
    }
}

/// The canvas window's procedure.
extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the user data is null or the boxed `WindowData` that `create` stored, alive until WM_NCDESTROY takes it
    // back below.
    let data = unsafe {
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData;
        if raw.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            crate::uia::forget(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(raw));
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        &*raw
    };
    // Screen readers (U.2) and placement reconciliation run before borrowing `canvas`: UIA providers borrow it per
    // call, and `reconcile` may call `SetWindowPos` which synchronously dispatches `WM_SIZE`.
    match msg {
        WM_GETOBJECT => {
            if let Some(answer) = crate::uia::get_object(hwnd, wparam, lparam) {
                return answer;
            }
        }
        crate::uia::WM_UIA_EVENT => {
            crate::uia::raise(hwnd, wparam);
            return LRESULT(0);
        }
        WM_SYNC_PLACEMENT => {
            data.placement.reconcile(hwnd);
            return LRESULT(0);
        }
        _ => {}
    }
    let canvas = &data.canvas;
    // Busy: a message sent while the canvas runs (it sends none it handles); let Windows have it,
    // except re-post completion/paint wakeups so a re-entrant message never strands the queue.
    let Ok(mut canvas) = canvas.try_borrow_mut() else {
        if msg == WM_JOB_DONE {
            // SAFETY: plain post to our live child window to re-queue missed completion handling.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_JOB_DONE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        if msg == WM_CANVAS_PAINT {
            data.paint_posted.set(false);
            request_paint(hwnd, &data.paint_posted);
            return LRESULT(0);
        }
        log::debug!("page canvas busy: message {msg:#x} goes to the default handling");
        // SAFETY: plain default handling.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        // A new size, or the window moved to a screen of another density.
        WM_SIZE | WM_DPICHANGED_AFTERPARENT => {
            let mut rect = RECT::default();
            // SAFETY: plain calls on a live window.
            let dpi = unsafe {
                let _ = GetClientRect(hwnd, &mut rect);
                GetDpiForWindow(hwnd)
            };
            canvas.resize(
                (rect.right - rect.left).max(0) as u32,
                (rect.bottom - rect.top).max(0) as u32,
                pixels_per_point(dpi),
            );
            LRESULT(0)
        }
        WM_PAINT => {
            data.paint_posted.set(false);
            // Validate before painting so any `InvalidateRect` triggered during `paint()` (smooth motion, device-loss
            // recovery, or newly uploaded tiles) schedules a fresh `WM_PAINT`.
            // SAFETY: plain call on a live window.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            canvas.paint();
            LRESULT(0)
        }
        WM_CANVAS_PAINT => {
            if data.paint_posted.replace(false) {
                // SAFETY: plain Win32 query on our live window handle.
                let visible = unsafe { IsWindowVisible(hwnd).as_bool() };
                if visible {
                    // SAFETY: validate before painting so any invalidation during `paint()` schedules a fresh frame.
                    unsafe {
                        let _ = ValidateRect(Some(hwnd), None);
                    }
                    canvas.paint();
                }
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let notches = f32::from((wparam.0 >> 16) as u16 as i16) / NOTCH;
            let keys = wparam.0 & 0xffff;
            // SAFETY: plain call on a live window.
            let dpi = unsafe { GetDpiForWindow(hwnd) } as f32;
            if msg == WM_MOUSEWHEEL && keys & MK_CONTROL != 0 {
                let mut at = POINT {
                    x: i32::from(lparam.0 as u16 as i16),
                    y: i32::from((lparam.0 >> 16) as u16 as i16),
                };
                // SAFETY: plain call on a live window; `at` lives for it.
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut at);
                }
                canvas.zoom_about(NOTCH_ZOOM.powf(notches), at.x as f32, at.y as f32);
            } else {
                let distance = notches * NOTCH_SCROLL * dpi / 96.0;
                // The wheel turns positive away from the user (scroll up); a tilt wheel positive to the right.
                if msg == WM_MOUSEHWHEEL {
                    canvas.scroll_wheel(distance, 0.0);
                } else if keys & MK_SHIFT != 0 {
                    canvas.scroll_wheel(-distance, 0.0);
                } else {
                    canvas.scroll_wheel(0.0, -distance);
                }
            }
            LRESULT(0)
        }
        WM_JOB_DONE => {
            canvas.jobs_done();
            LRESULT(0)
        }
        // Links and text under the mouse (R.7, K.13). Touch reaches here too, as the default handling's mouse
        // messages.
        WM_MOUSEMOVE => {
            let mut leave = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: `leave` describes this live window and lives for the call. Asking again while a request is
            // pending only renews it.
            unsafe {
                let _ = TrackMouseEvent(&mut leave);
            }
            canvas.mouse_moved(Some(client_point(lparam)));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            canvas.mouse_moved(None);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            canvas.pressed();
            if canvas.stop_pan() {
                // SAFETY: plain call; middle-click scrolling held the mouse.
                let _ = unsafe { ReleaseCapture() };
                return LRESULT(0);
            }
            let (x, y) = client_point(lparam);
            // V.9: Ctrl+click on a link opens it in another pane.
            if wparam.0 & MK_CONTROL != 0 && canvas.open_elsewhere(x, y) {
                return LRESULT(0);
            }
            // SAFETY: plain queries of the current message and the user's settings.
            let (time, limits) = unsafe {
                (
                    GetMessageTime() as u32,
                    ClickLimits {
                        time: GetDoubleClickTime(),
                        width: GetSystemMetrics(SM_CXDOUBLECLK),
                        height: GetSystemMetrics(SM_CYDOUBLECLK),
                    },
                )
            };
            if canvas.mouse_down(x, y, time, limits) {
                // A selection follows the mouse past the window's edge until the button comes up.
                // SAFETY: plain call on our live window.
                unsafe { SetCapture(hwnd) };
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            if canvas.mouse_up(x, y) {
                // Its WM_CAPTURECHANGED comes while the canvas is busy here and goes to the default handling.
                // SAFETY: plain call; we hold the capture.
                let _ = unsafe { ReleaseCapture() };
            }
            LRESULT(0)
        }
        // Middle-click scrolling (R.21): on at one press, off at the next.
        WM_MBUTTONDOWN => {
            canvas.pressed();
            let (x, y) = client_point(lparam);
            if canvas.middle_down(x, y) {
                // The pointer may leave the window while it scrolls: it still aims.
                // SAFETY: plain call on our live window.
                unsafe { SetCapture(hwnd) };
            } else {
                // SAFETY: plain call; we held the capture.
                let _ = unsafe { ReleaseCapture() };
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            canvas.capture_lost();
            LRESULT(0)
        }
        WM_SETCURSOR
            if (lparam.0 & 0xffff) as u32 == HTCLIENT && canvas.cursor() != Cursor::Arrow =>
        {
            let shape = match canvas.cursor() {
                Cursor::Hand => IDC_HAND,
                Cursor::Pan => IDC_SIZEALL,
                Cursor::Cross => IDC_CROSS,
                Cursor::Text | Cursor::Arrow => IDC_IBEAM,
            };
            // SAFETY: a system cursor, shared and never freed.
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, shape) {
                    SetCursor(Some(cursor));
                }
            }
            // The message wants TRUE when the cursor is set.
            LRESULT(1)
        }
        // The mouse's back and forward buttons (R.8): XBUTTON1 and XBUTTON2 in the high word.
        WM_XBUTTONUP => {
            match (wparam.0 >> 16) & 0xffff {
                1 => canvas.history_back(),
                2 => canvas.history_forward(),
                _ => {}
            }
            // The message wants TRUE when handled.
            LRESULT(1)
        }
        // The pen draws (plan §7.3). Touch passes the palm rejection first (H.8): a resting hand is swallowed, a
        // finger draws when "finger draws" is on; the rest goes to the default handling, which makes mouse messages.
        WM_POINTERDOWN
        | WM_POINTERUPDATE
        | WM_POINTERUP
        | WM_POINTERCAPTURECHANGED
        | WM_POINTERLEAVE => {
            let id = pointer::id(wparam.0);
            if msg == WM_POINTERDOWN {
                canvas.pressed();
            }
            let sample = pointer::read(id);
            let device = sample.map(|s| s.device);
            // SAFETY: a plain query of the current message.
            let now = unsafe { GetMessageTime() } as u32;
            let time = f64::from(sample.map_or(now, |s| s.time));
            if device == Some(Device::Pen) {
                if msg == WM_POINTERLEAVE {
                    canvas.pen_gone(time);
                } else {
                    canvas.pen_near(time);
                }
            }
            let taken = match (msg, device) {
                (WM_POINTERDOWN, Some(Device::Pen)) => match sample.filter(|s| s.contact) {
                    Some(sample) => {
                        // K.18: the app may switch to Write mode.
                        canvas.pen_contact();
                        canvas.pen_down(&pen_sample(&sample, client_origin(hwnd)))
                    }
                    None => false,
                },
                (WM_POINTERDOWN, Some(Device::Touch)) => {
                    let width = pointer::touch_width(id).map(|px| millimetres(px, hwnd));
                    match canvas.touch_down(id, time, width) {
                        Verdict::Ignore => true,
                        Verdict::Pass => false,
                        Verdict::Draw => sample.is_some_and(|sample| {
                            canvas.pen_down(&pen_sample(&sample, client_origin(hwnd)))
                        }),
                    }
                }
                (WM_POINTERUPDATE, _) if canvas.is_drawing(id) => {
                    let origin = client_origin(hwnd);
                    let history = pointer::pen_history(id);
                    // A finger has no pen history: its message's own sample.
                    let samples: Vec<PenSample> = if history.is_empty() {
                        sample.iter().map(|s| pen_sample(s, origin)).collect()
                    } else {
                        history.iter().map(|s| pen_sample(s, origin)).collect()
                    };
                    canvas.pen_moved(&samples);
                    true
                }
                (WM_POINTERUP | WM_POINTERCAPTURECHANGED, _) if canvas.is_drawing(id) => {
                    canvas.pen_up();
                    canvas.touch_up(id);
                    true
                }
                (WM_POINTERUP | WM_POINTERCAPTURECHANGED, Some(Device::Touch)) => {
                    let ignored = canvas.touch_verdict(id) == Verdict::Ignore;
                    canvas.touch_up(id);
                    ignored
                }
                (WM_POINTERUPDATE, Some(Device::Touch)) => {
                    canvas.touch_verdict(id) == Verdict::Ignore
                }
                _ => false,
            };
            if taken {
                return LRESULT(0);
            }
            drop(canvas);
            // SAFETY: plain default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => {
            // The default handling sends messages of its own (WM_WINDOWPOSCHANGED sends WM_SIZE): let go first.
            drop(canvas);
            // SAFETY: plain default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Device pixels to a page point at 100 % on a screen of `dpi`.
fn pixels_per_point(dpi: u32) -> f32 {
    dpi as f32 / 72.0
}

/// The window's top-left corner on the screen, in pixels: pen samples come in screen coordinates.
fn client_origin(hwnd: HWND) -> POINT {
    let mut origin = POINT::default();
    // SAFETY: a plain call on our live window; `origin` lives for it.
    let _ = unsafe { ClientToScreen(hwnd, &mut origin) };
    origin
}

/// `pixels` on this window's screen in millimetres (H.8's contact widths).
fn millimetres(pixels: i32, hwnd: HWND) -> f32 {
    // SAFETY: a plain query on our live window.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(1);
    pixels as f32 * 25.4 / dpi as f32
}

/// A pen sample as the shared canvas takes it: in the window's pixels.
fn pen_sample(sample: &Sample, origin: POINT) -> PenSample {
    PenSample {
        id: sample.id,
        x: (sample.x - origin.x) as f32,
        y: (sample.y - origin.y) as f32,
        pressure: sample.pressure,
        tilt: match (sample.tilt_x, sample.tilt_y) {
            (None, None) => None,
            (x, y) => Some(Tilt {
                x: x.unwrap_or(0) as f32,
                y: y.unwrap_or(0) as f32,
            }),
        },
        time: Some(f64::from(sample.time)),
        contact: sample.contact,
        eraser: sample.eraser,
        input: sample.performance_count,
    }
}

/// A mouse message's point, in client pixels (`GET_X_LPARAM`, `GET_Y_LPARAM`: signed, as on a second screen).
fn client_point(lparam: LPARAM) -> (i32, i32) {
    (
        i32::from(lparam.0 as u16 as i16),
        i32::from((lparam.0 >> 16) as u16 as i16),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::CreateRectRgn;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, SW_SHOW, SetParent, WS_CHILD, WS_OVERLAPPEDWINDOW,
    };

    fn make_test_window(
        style: windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE,
        parent: Option<HWND>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> HWND {
        // SAFETY: creating a plain hidden test window using the built-in STATIC class.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                style,
                x,
                y,
                w,
                h,
                parent,
                None,
                None,
                None,
            )
            .expect("test window")
        }
    }

    #[test]
    fn draw_transform_and_dpi_map_to_parent_physical_pixels() {
        // 150% DPI (144): logical (260, 116) size (2000 x 1324) -> physical (390, 174, 3390, 2160).
        let transform = Matrix3x2 {
            M11: 1.0,
            M12: 0.0,
            M21: 0.0,
            M22: 1.0,
            M31: 260.0,
            M32: 116.0,
        };
        let rect = rect_from_draw(&transform, 144.0, 144.0, (2000.0, 1324.0)).expect("rect");
        assert_eq!(
            rect,
            RECT {
                left: 390,
                top: 174,
                right: 3390,
                bottom: 2160,
            }
        );
        assert_eq!(rect_from_draw(&transform, 144.0, 144.0, (0.0, 500.0)), None);
        assert_eq!(rect_from_draw(&transform, 0.0, 144.0, (500.0, 500.0)), None);
    }

    #[test]
    fn placement_reconcile_clears_nsh_clipped_region_and_restores_split_pane() {
        let parent = make_test_window(WS_OVERLAPPEDWINDOW, None, 0, 0, 2600, 1600);
        let child = make_test_window(WS_CHILD, Some(parent), 0, 0, 1, 1);
        let paint_posted = Rc::new(Cell::new(false));
        let sync = PlacementSync::new(paint_posted);

        // Case 1: NSH placed the child at full size (2000x1200) inside a visible large AppWindow,
        // then installed a truncated SetWindowRgn (0,0,1000,700) from the smaller hidden host surface.
        let full = RECT {
            left: 260,
            top: 116,
            right: 2260,
            bottom: 1316,
        };
        sync.note_drawn(child, full);
        // SAFETY: plain Win32 setup simulating NSH's `Placement::apply`.
        unsafe {
            let _ = ShowWindow(parent, SW_SHOW);
            let _ = SetParent(child, Some(parent));
            let _ = SetWindowPos(
                child,
                Some(HWND_TOP),
                full.left,
                full.top,
                full.right - full.left,
                full.bottom - full.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER,
            );
            let bad_rgn = CreateRectRgn(0, 0, 1000, 700);
            assert_ne!(SetWindowRgn(child, Some(bad_rgn), true), 0);
            let mut rgn_box = RECT::default();
            assert_ne!(GetWindowRgnBox(child, &mut rgn_box), RGN_ERROR);
        }

        sync.reconcile(child);
        // SAFETY: plain Win32 verification on our test windows.
        unsafe {
            let mut rgn_box = RECT::default();
            assert_eq!(
                GetWindowRgnBox(child, &mut rgn_box),
                RGN_ERROR,
                "truncated NSH clip region must be cleared"
            );
            let mut client = RECT::default();
            let _ = GetClientRect(child, &mut client);
            assert_eq!((client.right, client.bottom), (2000, 1200));
        }

        // Case 2: Split pane positioned past the hidden host's right edge (x=1920..2500): NSH hid it.
        let split_right = RECT {
            left: 1920,
            top: 116,
            right: 2500,
            bottom: 1316,
        };
        sync.note_drawn(child, split_right);
        // SAFETY: simulating NSH calling `ShowWindow(SW_HIDE)` when `clip()` returns `None`.
        unsafe {
            let _ = ShowWindow(child, SW_HIDE);
            assert!(!IsWindowVisible(child).as_bool());
        }
        sync.reconcile(child);
        // SAFETY: plain Win32 queries on our test windows.
        unsafe {
            assert!(
                IsWindowVisible(child).as_bool(),
                "split pane past host bounds must be restored"
            );
            let mut client = RECT::default();
            let _ = GetClientRect(child, &mut client);
            assert_eq!((client.right, client.bottom), (580, 1200));
        }

        // Case 3: App hides canvas for a dialog (`visible = false`), then closes dialog
        // (`visible = true`) WITHOUT a new `note_drawn` frame -> canvas must become visible again.
        sync.set_app_visible(child, false);
        // SAFETY: plain Win32 queries and cleanup on our test windows.
        unsafe {
            assert!(!IsWindowVisible(child).as_bool());
            sync.reconcile(child);
            assert!(!IsWindowVisible(child).as_bool());

            sync.set_app_visible(child, true);
            sync.reconcile(child);
            assert!(
                IsWindowVisible(child).as_bool(),
                "closing dialog must restore child visibility even without a new draw"
            );
            let mut client = RECT::default();
            let _ = GetClientRect(child, &mut client);
            assert_eq!((client.right, client.bottom), (580, 1200));

            let _ = DestroyWindow(child);
            let _ = DestroyWindow(parent);
        }
    }

    #[test]
    fn request_paint_coalesces_and_survives_parent_validate_rect() {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PM_REMOVE, PeekMessageW};

        let parent = make_test_window(WS_OVERLAPPEDWINDOW, None, 0, 0, 800, 600);
        let child = make_test_window(WS_CHILD, Some(parent), 0, 0, 600, 400);
        let paint_posted = Cell::new(false);

        request_paint(child, &paint_posted);
        request_paint(child, &paint_posted);
        assert!(paint_posted.get());

        // Simulate NSH's `ValidateRect(parent, None)` on a parent lacking `WS_CLIPCHILDREN`:
        // `WM_CANVAS_PAINT` must still remain queued for the child HWND.
        // SAFETY: plain Win32 message queue inspection on test windows created on this thread.
        unsafe {
            let _ = ValidateRect(Some(parent), None);
            let mut msg = MSG::default();
            assert!(
                PeekMessageW(
                    &mut msg,
                    Some(child),
                    WM_CANVAS_PAINT,
                    WM_CANVAS_PAINT,
                    PM_NOREMOVE
                )
                .as_bool(),
                "WM_CANVAS_PAINT must remain queued even after parent ValidateRect"
            );
            assert!(
                PeekMessageW(
                    &mut msg,
                    Some(child),
                    WM_CANVAS_PAINT,
                    WM_CANVAS_PAINT,
                    PM_REMOVE
                )
                .as_bool()
            );
            assert!(
                !PeekMessageW(
                    &mut msg,
                    Some(child),
                    WM_CANVAS_PAINT,
                    WM_CANVAS_PAINT,
                    PM_NOREMOVE
                )
                .as_bool(),
                "coalesced request_paint must enqueue at most one WM_CANVAS_PAINT"
            );
            let _ = DestroyWindow(child);
            let _ = DestroyWindow(parent);
        }
    }
}
