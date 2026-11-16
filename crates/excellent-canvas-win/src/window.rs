//! The canvas's own window (plan K.8, item 1): a child of the app window that NSH places over the canvas's view, and
//! its window procedure: size, paint, wheel, pen, links and text under the mouse, middle-click scrolling, DPI and
//! finished jobs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Once;

use excellent_canvas::page::{ClickLimits, Cursor, PenSample, Tilt};
use nsh_platform_windows::ComponentContext;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    FEEDBACK_PEN_BARRELVISUALIZATION, FEEDBACK_PEN_DOUBLETAP, FEEDBACK_PEN_PRESSANDHOLD,
    FEEDBACK_PEN_RIGHTTAP, FEEDBACK_PEN_TAP, SetWindowFeedbackSetting, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetClientRect, GetMessageTime,
    GetSystemMetrics, GetWindowLongPtrW, HTCLIENT, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM,
    IDC_SIZEALL, LoadCursorW, RegisterClassExW, SM_CXDOUBLECLK, SM_CYDOUBLECLK, SetCursor,
    SetWindowLongPtrW, WINDOW_EX_STYLE, WM_CAPTURECHANGED, WM_DPICHANGED_AFTERPARENT,
    WM_ERASEBKGND, WM_GETOBJECT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN,
    WM_POINTERUP, WM_POINTERUPDATE, WM_SETCURSOR, WM_SIZE, WM_XBUTTONUP, WNDCLASSEXW,
    WS_CLIPCHILDREN, WS_POPUP,
};
use windows::core::BOOL;
use windows::core::{Result, w};

use crate::host::{Canvas, WM_JOB_DONE, WinHost};
use crate::pointer::{self, Device, Sample};

const CLASS: windows::core::PCWSTR = w!("ExcellentPDF.PageCanvas");

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
pub(crate) fn create(events: ComponentContext) -> Result<(HWND, Rc<RefCell<Canvas>>)> {
    // SAFETY: plain Win32 calls; the class's procedure is `proc` below, and the window's user data holds one counted
    // reference to its canvas, released in WM_NCDESTROY.
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
        let canvas = Rc::new(RefCell::new(Canvas::new(WinHost::new(hwnd, events), ppp)));
        crate::uia::register(hwnd, &canvas);
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            Rc::into_raw(Rc::clone(&canvas)) as isize,
        );
        Ok((hwnd, canvas))
    }
}

/// The canvas window's procedure.
extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the user data is null or the `Rc` that `create` stored, alive until WM_NCDESTROY takes it back below.
    let canvas = unsafe {
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Canvas>;
        if raw.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            crate::uia::forget(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Rc::from_raw(raw));
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        &*raw
    };
    // Screen readers (U.2): their providers borrow the canvas on each call, so these come before it is borrowed here.
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
        _ => {}
    }
    // Busy: a message sent while the canvas runs (it sends none it handles); let Windows have it.
    let Ok(mut canvas) = canvas.try_borrow_mut() else {
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
            canvas.paint();
            // SAFETY: plain call on a live window.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
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
        // The pen draws (plan §7.3). Touch and anything the canvas doesn't take go to the default handling, which
        // turns them into mouse messages.
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERCAPTURECHANGED => {
            let id = pointer::id(wparam.0);
            if msg == WM_POINTERDOWN {
                canvas.pressed();
            }
            let taken = match msg {
                WM_POINTERDOWN => {
                    match pointer::read(id).filter(|s| s.device == Device::Pen && s.contact) {
                        Some(sample) => {
                            // K.18: the app may switch to Write mode.
                            canvas.pen_contact();
                            canvas.pen_down(&pen_sample(&sample, client_origin(hwnd)))
                        }
                        None => false,
                    }
                }
                WM_POINTERUPDATE if canvas.is_drawing(id) => {
                    let origin = client_origin(hwnd);
                    let samples: Vec<PenSample> = pointer::pen_history(id)
                        .iter()
                        .map(|sample| pen_sample(sample, origin))
                        .collect();
                    canvas.pen_moved(&samples);
                    true
                }
                WM_POINTERUP | WM_POINTERCAPTURECHANGED if canvas.is_drawing(id) => {
                    canvas.pen_up();
                    true
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
