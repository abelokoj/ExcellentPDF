//! The canvas's `NSView` (plan K.9, items 1 and 2): it holds the shared canvas, draws it in `drawRect:`, follows its
//! size and the window's backing scale, and turns AppKit's events into the canvas's: scrolling (wheel and trackpad),
//! pinch zoom, the mouse (links, selection, middle-click scrolling, back and forward buttons) and the pen, whose
//! tablet events carry pressure and whose proximity says which end is down.
//!
//! Positions reach the canvas in device pixels: the view's points (flipped: top-left origin) times the backing scale.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use excellent_canvas::page::{ClickLimits, Cursor, PenSample};
use nsh_platform_macos::ComponentContext;
use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityPostNotification,
    NSAccessibilitySelectedTextChangedNotification, NSAccessibilityTextAreaRole,
    NSAccessibilityValueChangedNotification, NSCursor, NSEvent, NSEventModifierFlags,
    NSEventSubtype, NSGraphicsContext, NSPointingDeviceType, NSTrackingArea, NSTrackingAreaOptions,
    NSView,
};
use objc2_core_foundation::{CGFloat, CGSize};
use objc2_foundation::{NSInteger, NSObjectProtocol, NSPoint, NSRange, NSRect, NSString};

use crate::access;
use crate::host::{Canvas, MacHost};

/// A line of a wheel that scrolls by lines, in points (about a line of text; three make the 100 px notch Windows
/// scrolls).
const LINE_SCROLL: f64 = 33.0;
/// How much one line with Command zooms.
const LINE_ZOOM: f32 = 1.1;
/// The pen's pointer id: AppKit follows one stylus at a time.
const PEN: u32 = 1;
/// How far apart two clicks of a double click may be, in device pixels (AppKit has no such setting).
const CLICK_DISTANCE: i32 = 4;
/// AppKit's numbers for the middle, back and forward buttons.
const MIDDLE: isize = 2;
const BACK: isize = 3;
const FORWARD: isize = 4;

/// The view's state.
#[derive(Default)]
pub(crate) struct Ivars {
    /// Made right after the view.
    canvas: OnceCell<Rc<RefCell<Canvas>>>,
    /// The pen's eraser end is near the tablet.
    eraser: Cell<bool>,
    /// The pen went down on the canvas and draws.
    pen: Cell<bool>,
    /// The area that sends mouse moves.
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    /// The page canvas's view.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "ExcellentPageCanvas"]
    pub(crate) struct CanvasView;

    unsafe impl NSObjectProtocol for CanvasView {}

    impl CanvasView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: CGSize) {
            // SAFETY: NSView's own resize.
            let () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.follow_size();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            self.follow_size();
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            self.track_mouse();
            // SAFETY: NSView's own update.
            let () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.on_scroll(event);
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &NSEvent) {
            let (x, y) = self.device_point(event);
            let factor = (1.0 + event.magnification()).max(0.1) as f32;
            self.with_canvas(|canvas| canvas.zoom_about(factor, x as f32, y as f32));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let at = self.device_point(event);
            self.with_canvas(|canvas| {
                canvas.mouse_moved(Some(at));
                show_cursor(canvas.cursor());
            });
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.with_canvas(|canvas| {
                canvas.mouse_moved(None);
                show_cursor(canvas.cursor());
            });
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.on_down(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.on_dragged(event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.on_up(event);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            let at = self.device_point(event);
            self.with_canvas(|canvas| {
                match event.buttonNumber() {
                    MIDDLE => {
                        canvas.pressed();
                        canvas.middle_down(at.0, at.1);
                    }
                    BACK => canvas.history_back(),
                    FORWARD => canvas.history_forward(),
                    _ => {}
                }
                show_cursor(canvas.cursor());
            });
        }

        #[unsafe(method(tabletProximity:))]
        fn tablet_proximity(&self, event: &NSEvent) {
            self.ivars()
                .eraser
                .set(event.isEnteringProximity() && event.pointingDeviceType() == NSPointingDeviceType::Eraser);
        }

        // Screen readers (U.3): the view is a read-only text area over the pages' text; see `crate::access`.

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            self.read_canvas(access::value).map(|text| NSString::from_str(&text))
        }

        #[unsafe(method(accessibilityNumberOfCharacters))]
        fn accessibility_number_of_characters(&self) -> NSInteger {
            to_integer(self.read_canvas(access::count).unwrap_or(0))
        }

        #[unsafe(method_id(accessibilitySelectedText))]
        fn accessibility_selected_text(&self) -> Option<Retained<NSString>> {
            self.read_canvas(|canvas| access::string(canvas, access::selected(canvas)))
                .map(|text| NSString::from_str(&text))
        }

        #[unsafe(method(accessibilitySelectedTextRange))]
        fn accessibility_selected_text_range(&self) -> NSRange {
            ns_range(self.read_canvas(access::selected).unwrap_or(0..0))
        }

        #[unsafe(method(setAccessibilitySelectedTextRange:))]
        fn set_accessibility_selected_text_range(&self, range: NSRange) {
            self.with_canvas(|canvas| access::select(canvas, range_of(range)));
        }

        #[unsafe(method(accessibilityVisibleCharacterRange))]
        fn accessibility_visible_character_range(&self) -> NSRange {
            ns_range(self.read_canvas(access::shown).unwrap_or(0..0))
        }

        #[unsafe(method(accessibilityInsertionPointLineNumber))]
        fn accessibility_insertion_point_line_number(&self) -> NSInteger {
            let line = self.read_canvas(|canvas| access::line_at(canvas, access::selected(canvas).start));
            to_integer(line.unwrap_or(0))
        }

        #[unsafe(method_id(accessibilityStringForRange:))]
        fn accessibility_string_for_range(&self, range: NSRange) -> Option<Retained<NSString>> {
            self.read_canvas(|canvas| access::string(canvas, range_of(range)))
                .map(|text| NSString::from_str(&text))
        }

        #[unsafe(method(accessibilityLineForIndex:))]
        fn accessibility_line_for_index(&self, index: NSInteger) -> NSInteger {
            let index = usize::try_from(index).unwrap_or(0);
            to_integer(self.read_canvas(|canvas| access::line_at(canvas, index)).unwrap_or(0))
        }

        #[unsafe(method(accessibilityRangeForLine:))]
        fn accessibility_range_for_line(&self, line: NSInteger) -> NSRange {
            let line = usize::try_from(line).unwrap_or(0);
            ns_range(self.read_canvas(|canvas| access::line_range(canvas, line)).unwrap_or(0..0))
        }

        #[unsafe(method(accessibilityFrameForRange:))]
        fn accessibility_frame_for_range(&self, range: NSRange) -> NSRect {
            self.screen_frame(range_of(range))
        }
    }
);

impl CanvasView {
    /// A view with its canvas, telling the app what happens through `events`.
    pub(crate) fn new(
        mtm: MainThreadMarker,
        events: ComponentContext,
    ) -> (Retained<Self>, Rc<RefCell<Canvas>>) {
        let this = Self::alloc(mtm).set_ivars(Ivars::default());
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        let host = MacHost::new(&view, events);
        let scale = view.backing_scale();
        // A point is 1/72 inch, as a PDF's: at 100 % a page point is a screen point, as in Preview.
        let canvas = Rc::new(RefCell::new(Canvas::new(host, scale as f32)));
        view.ivars()
            .canvas
            .set(Rc::clone(&canvas))
            .unwrap_or_else(|_| unreachable!("the canvas is set once, here"));
        view.setAccessibilityElement(true);
        // SAFETY: AppKit's role constant.
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityTextAreaRole }));
        view.track_mouse();
        (view, canvas)
    }

    /// Names the view for screen readers.
    pub(crate) fn set_label(&self, text: &str) {
        self.setAccessibilityLabel(Some(&NSString::from_str(text)));
    }

    /// Device pixels per point.
    fn backing_scale(&self) -> CGFloat {
        self.window()
            .map_or(1.0, |window| window.backingScaleFactor())
            .max(1.0)
    }

    /// Runs `f` on the canvas unless it is busy further up the stack (then `None`).
    fn with_canvas<T>(&self, f: impl FnOnce(&mut Canvas) -> T) -> Option<T> {
        let canvas = self.ivars().canvas.get()?;
        let mut canvas = canvas.try_borrow_mut().ok()?;
        Some(f(&mut canvas))
    }

    /// Runs `f` on the canvas unless it is being changed further up the stack (then `None`).
    fn read_canvas<T>(&self, f: impl FnOnce(&Canvas) -> T) -> Option<T> {
        let canvas = self.ivars().canvas.get()?;
        let canvas = canvas.try_borrow().ok()?;
        Some(f(&canvas))
    }

    /// Where characters `range` are on the screen, in AppKit's screen coordinates; empty when not laid out.
    fn screen_frame(&self, range: std::ops::Range<usize>) -> NSRect {
        let Some(area) = self
            .read_canvas(|canvas| access::area(canvas, range))
            .flatten()
        else {
            return NSRect::ZERO;
        };
        let scale = self.backing_scale();
        let in_view = NSRect::new(
            NSPoint::new(
                CGFloat::from(area.x0) / scale,
                CGFloat::from(area.y0) / scale,
            ),
            CGSize::new(
                CGFloat::from(area.width()) / scale,
                CGFloat::from(area.height()) / scale,
            ),
        );
        let in_window = self.convertRect_toView(in_view, None);
        self.window()
            .map_or(in_window, |window| window.convertRectToScreen(in_window))
    }

    /// Tells screen readers the text changed (pages read, a document opened).
    pub(crate) fn text_changed(&self) {
        // SAFETY: this view is an accessibility element; the name is AppKit's constant.
        unsafe { NSAccessibilityPostNotification(self, NSAccessibilityValueChangedNotification) };
    }

    /// Tells screen readers the selection moved.
    pub(crate) fn selection_moved(&self) {
        // SAFETY: as in `text_changed`.
        unsafe {
            NSAccessibilityPostNotification(self, NSAccessibilitySelectedTextChangedNotification)
        };
    }

    /// Tells the canvas the view's size and scale when either changed.
    fn follow_size(&self) {
        let scale = self.backing_scale();
        let bounds = self.bounds();
        let size = (
            (bounds.size.width * scale).round() as f32,
            (bounds.size.height * scale).round() as f32,
        );
        let followed = self.with_canvas(|canvas| {
            if canvas.host().set_size(size, scale) {
                canvas.resize(size.0 as u32, size.1 as u32, scale as f32);
            }
        });
        if followed.is_none() {
            // Busy: AppKit asks again on the next layout; redraw then.
            self.setNeedsDisplay(true);
        }
    }

    /// Draws the canvas into the current context.
    fn draw(&self) {
        // The scale can change without a resize (the window moved to another screen).
        if self.with_canvas(|canvas| canvas.host().scale()) != Some(self.backing_scale()) {
            self.follow_size();
        }
        let Some(context) = NSGraphicsContext::currentContext() else {
            return;
        };
        let context = context.CGContext().into();
        let drawn = self.with_canvas(|canvas| {
            canvas.host_mut().lend(Some(context));
            canvas.paint();
            canvas.host_mut().lend(None);
        });
        if drawn.is_none() {
            self.setNeedsDisplay(true);
        }
    }

    /// Takes the finished background jobs; `false` when the canvas was busy.
    pub(crate) fn jobs_done(&self) -> bool {
        self.with_canvas(Canvas::jobs_done).is_some()
    }

    /// Replaces the tracking area with one over the view as it is now.
    fn track_mouse(&self) {
        if let Some(old) = self.ivars().tracking.take() {
            self.removeTrackingArea(&old);
        }
        let options = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: the owner (this view) outlives the tracking area, which it holds itself.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                options,
                Some(self),
                None,
            )
        };
        self.addTrackingArea(&area);
        self.ivars().tracking.replace(Some(area));
    }

    /// The event's position in the view, in device pixels.
    fn device_point(&self, event: &NSEvent) -> (i32, i32) {
        let point: NSPoint = self.convertPoint_fromView(event.locationInWindow(), None);
        let scale = self.backing_scale();
        (
            (point.x * scale).round() as i32,
            (point.y * scale).round() as i32,
        )
    }

    /// The pen's sample for `event`.
    fn pen_sample(&self, event: &NSEvent) -> PenSample {
        let point: NSPoint = self.convertPoint_fromView(event.locationInWindow(), None);
        let scale = self.backing_scale();
        PenSample {
            id: PEN,
            x: (point.x * scale) as f32,
            y: (point.y * scale) as f32,
            pressure: Some(event.pressure().clamp(0.0, 1.0)),
            contact: true,
            eraser: self.ivars().eraser.get(),
            input: 0,
        }
    }

    /// Scrolling: a trackpad's in points, a wheel's in lines; Command zooms, Shift scrolls sideways.
    fn on_scroll(&self, event: &NSEvent) {
        let scale = self.backing_scale();
        let per = if event.hasPreciseScrollingDeltas() {
            scale
        } else {
            LINE_SCROLL * scale
        };
        // AppKit's deltas move the content; the canvas's move the view (positive: on through the document).
        let (mut dx, mut dy) = (
            -event.scrollingDeltaX() * per,
            -event.scrollingDeltaY() * per,
        );
        let keys = event.modifierFlags();
        if keys.contains(NSEventModifierFlags::Command) {
            let (x, y) = self.device_point(event);
            let lines = (dy / per) as f32;
            self.with_canvas(|canvas| {
                canvas.zoom_about(LINE_ZOOM.powf(-lines), x as f32, y as f32)
            });
            return;
        }
        if keys.contains(NSEventModifierFlags::Shift) && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        self.with_canvas(|canvas| canvas.scroll_wheel(dx as f32, dy as f32));
    }

    /// The first button: the pen draws (plan §7.3); the mouse follows links and selects text (K.13).
    fn on_down(&self, event: &NSEvent) {
        let tablet = event.subtype() == NSEventSubtype::TabletPoint;
        // V.9: Command-click on a link opens it in another pane.
        let elsewhere = event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command);
        let at = self.device_point(event);
        let sample = tablet.then(|| self.pen_sample(event));
        let pen = self
            .with_canvas(|canvas| {
                canvas.pressed();
                if let Some(sample) = &sample {
                    // K.18: the app may switch to Write mode.
                    canvas.pen_contact();
                    if canvas.pen_down(sample) {
                        return true;
                    }
                }
                if !canvas.stop_pan() && !(elsewhere && canvas.open_elsewhere(at.0, at.1)) {
                    let limits = ClickLimits {
                        time: (NSEvent::doubleClickInterval() * 1000.0) as u32,
                        width: CLICK_DISTANCE,
                        height: CLICK_DISTANCE,
                    };
                    canvas.mouse_down(at.0, at.1, (event.timestamp() * 1000.0) as u32, limits);
                }
                show_cursor(canvas.cursor());
                false
            })
            .unwrap_or(false);
        self.ivars().pen.set(pen);
    }

    // shortcut: one sample a drag event, without AppKit's coalesced tablet points; read them if strokes look coarse
    // on a fast pen (H.1).
    fn on_dragged(&self, event: &NSEvent) {
        if self.ivars().pen.get() {
            let sample = self.pen_sample(event);
            self.with_canvas(|canvas| {
                if canvas.is_drawing(PEN) {
                    canvas.pen_moved(&[sample]);
                }
            });
            return;
        }
        let at = self.device_point(event);
        self.with_canvas(|canvas| {
            canvas.mouse_moved(Some(at));
            show_cursor(canvas.cursor());
        });
    }

    fn on_up(&self, event: &NSEvent) {
        if self.ivars().pen.replace(false) {
            self.with_canvas(|canvas| {
                if canvas.is_drawing(PEN) {
                    canvas.pen_up();
                }
            });
            return;
        }
        let at = self.device_point(event);
        self.with_canvas(|canvas| {
            canvas.mouse_up(at.0, at.1);
            show_cursor(canvas.cursor());
        });
    }
}

/// A range of characters as AppKit's.
fn ns_range(range: std::ops::Range<usize>) -> NSRange {
    NSRange::new(range.start, range.len())
}

/// AppKit's range as characters.
fn range_of(range: NSRange) -> std::ops::Range<usize> {
    range.location..range.location.saturating_add(range.length)
}

fn to_integer(n: usize) -> NSInteger {
    NSInteger::try_from(n).unwrap_or(NSInteger::MAX)
}

/// Shows the pointer shape the canvas wants.
fn show_cursor(cursor: Cursor) {
    let cursor = match cursor {
        Cursor::Arrow => NSCursor::arrowCursor(),
        Cursor::Hand => NSCursor::pointingHandCursor(),
        Cursor::Pan => NSCursor::closedHandCursor(),
        Cursor::Cross => NSCursor::crosshairCursor(),
        Cursor::Text => NSCursor::IBeamCursor(),
    };
    cursor.set();
}
