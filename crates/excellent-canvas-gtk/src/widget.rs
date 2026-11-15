//! The canvas's GTK widget (plan K.10, items 1 and 2): it holds the shared canvas, draws it in `snapshot`, follows
//! its size and the screen's scale, and turns GTK's input into the canvas's: the wheel and touchpad, the mouse (links,
//! selection, middle-click scrolling, back and forward buttons) and the pen through `GtkGestureStylus`.
//!
//! Positions reach the canvas in device pixels: GTK's widget pixels times the surface's scale.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};

use excellent_canvas::page::{ClickLimits, Cursor, PenSample};
use nsh_platform_linux::ComponentContext;
use nsh_platform_linux::gtk4 as gtk;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::access;
use crate::host::{Canvas, GtkHost};

/// One wheel notch's scroll, in widget pixels (about three lines of text, as browsers do; the same as Windows).
const NOTCH_SCROLL: f64 = 100.0;
/// How much one notch with Ctrl zooms.
const NOTCH_ZOOM: f32 = 1.1;
/// The pen's pointer id: GTK follows one stylus at a time.
const PEN: u32 = 1;
/// Device pixels to a page point at 100 % for each device pixel per widget pixel (GTK's widget pixel is 1/96 inch).
const POINTS_PER_PIXEL: f64 = 96.0 / 72.0;

mod imp {
    use super::*;

    /// The widget's state: its canvas, made right after the widget.
    #[derive(Default)]
    pub(crate) struct CanvasWidget {
        pub(super) canvas: OnceCell<Rc<RefCell<Canvas>>>,
        /// Where the mouse is, in widget pixels, for Ctrl+wheel zoom.
        pub(super) mouse: Cell<(f64, f64)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CanvasWidget {
        const NAME: &'static str = "ExcellentPageCanvas";
        type Type = super::CanvasWidget;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::AccessibleText,);

        fn class_init(class: &mut Self::Class) {
            class.set_accessible_role(gtk::AccessibleRole::Document);
        }
    }

    impl ObjectImpl for CanvasWidget {}

    impl WidgetImpl for CanvasWidget {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().follow_size();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }

    /// Screen readers read the pages' text (U.3); see `crate::access`. A busy canvas reads as empty for the moment.
    impl AccessibleTextImpl for CanvasWidget {
        fn contents(&self, start: u32, end: u32) -> Option<glib::Bytes> {
            self.obj()
                .read_canvas(|canvas| access::contents(canvas, start, end))
        }

        fn contents_at(
            &self,
            offset: u32,
            granularity: gtk::AccessibleTextGranularity,
        ) -> Option<(u32, u32, glib::Bytes)> {
            self.obj()
                .read_canvas(|canvas| access::contents_at(canvas, offset, granularity))
        }

        fn caret_position(&self) -> u32 {
            self.obj().read_canvas(access::caret).unwrap_or(0)
        }

        fn selection(&self) -> Vec<gtk::AccessibleTextRange> {
            self.obj()
                .read_canvas(access::selection)
                .unwrap_or_default()
        }

        fn attributes(
            &self,
            _offset: u32,
        ) -> Vec<(gtk::AccessibleTextRange, glib::GString, glib::GString)> {
            Vec::new()
        }

        fn default_attributes(&self) -> Vec<(glib::GString, glib::GString)> {
            Vec::new()
        }
    }
}

glib::wrapper! {
    /// The page canvas's widget.
    pub(crate) struct CanvasWidget(ObjectSubclass<imp::CanvasWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget;
}

impl CanvasWidget {
    /// A widget with its canvas, telling the app what happens through `events`.
    pub(crate) fn new(events: ComponentContext) -> (Self, Rc<RefCell<Canvas>>) {
        let widget: Self = glib::Object::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_overflow(gtk::Overflow::Hidden);
        let host = GtkHost::new(&widget, events);
        let scale = widget.surface_scale();
        let canvas = Rc::new(RefCell::new(Canvas::new(
            host,
            (scale * POINTS_PER_PIXEL) as f32,
        )));
        widget
            .imp()
            .canvas
            .set(Rc::clone(&canvas))
            .unwrap_or_else(|_| unreachable!("the canvas is set once, here"));
        widget.connect_scale_factor_notify(|widget| widget.follow_size());
        widget.add_input(Rc::downgrade(&canvas));
        (widget, canvas)
    }

    /// Device pixels per widget pixel: the surface's scale, fractional where the compositor scales so.
    fn surface_scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
            .max(1.0)
    }

    /// Runs `f` on the canvas unless it is busy further up the stack (then `None`).
    fn with_canvas<T>(&self, f: impl FnOnce(&mut Canvas) -> T) -> Option<T> {
        let canvas = self.imp().canvas.get()?;
        let mut canvas = canvas.try_borrow_mut().ok()?;
        Some(f(&mut canvas))
    }

    /// Runs `f` on the canvas unless it is being changed further up the stack (then `None`).
    pub(crate) fn read_canvas<T>(&self, f: impl FnOnce(&Canvas) -> T) -> Option<T> {
        let canvas = self.imp().canvas.get()?;
        let canvas = canvas.try_borrow().ok()?;
        Some(f(&canvas))
    }

    /// Tells screen readers the text changed (pages read, a document opened).
    // shortcut: announced as the whole text inserted; GTK has no "replaced" change. Send the pages' own ranges if
    // Orca repeats text it already read.
    pub(crate) fn text_changed(&self) {
        if let Some(len) = self.read_canvas(access::len) {
            self.update_contents(gtk::AccessibleTextContentChange::Insert, 0, len);
        }
    }

    /// Tells screen readers the selection, and so the caret, moved.
    pub(crate) fn selection_moved(&self) {
        self.update_selection_bound();
        self.update_caret_position();
    }

    /// Tells the canvas the widget's size and scale when either changed.
    fn follow_size(&self) {
        let scale = self.surface_scale();
        let size = (
            (f64::from(self.width()) * scale).round() as f32,
            (f64::from(self.height()) * scale).round() as f32,
        );
        let followed = self.with_canvas(|canvas| {
            if canvas.host().set_size(size, scale) {
                canvas.resize(
                    size.0 as u32,
                    size.1 as u32,
                    (scale * POINTS_PER_PIXEL) as f32,
                );
            }
        });
        if followed.is_none() {
            // Busy: come back once the canvas is free.
            let weak = self.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(widget) = weak.upgrade() {
                    widget.follow_size();
                }
            });
        }
    }

    /// Draws the canvas into `snapshot`.
    fn draw(&self, snapshot: &gtk::Snapshot) {
        // The scale can change without a new allocation (the window moved to another screen).
        if self.surface_scale() != self.with_canvas(|c| c.host().scale()).unwrap_or(1.0) {
            self.follow_size();
        }
        let drawn = self.with_canvas(|canvas| {
            canvas.host_mut().lend(Some(snapshot));
            canvas.paint();
            canvas.host_mut().lend(None);
        });
        if drawn.is_none() {
            self.queue_draw();
        }
    }

    /// Takes the finished background jobs; `false` when the canvas was busy.
    pub(crate) fn jobs_done(&self) -> bool {
        self.with_canvas(Canvas::jobs_done).is_some()
    }

    /// Shows the pointer shape the canvas wants.
    fn update_cursor(&self, cursor: Cursor) {
        self.set_cursor_from_name(Some(match cursor {
            Cursor::Arrow => "default",
            Cursor::Hand => "pointer",
            Cursor::Pan => "all-scroll",
            Cursor::Text => "text",
        }));
    }

    /// A widget point in device pixels, rounded as the mouse's are.
    fn device_point(&self, x: f64, y: f64) -> (i32, i32) {
        let scale = self.surface_scale();
        ((x * scale).round() as i32, (y * scale).round() as i32)
    }

    /// Wires GTK's input to the canvas. The closures hold the canvas weakly: the widget owns it.
    fn add_input(&self, canvas: Weak<RefCell<Canvas>>) {
        self.add_scroll(canvas.clone());
        self.add_motion(canvas.clone());
        self.add_stylus(canvas.clone());
        self.add_drag(canvas.clone());
        self.add_buttons(canvas);
    }

    fn add_scroll(&self, canvas: Weak<RefCell<Canvas>>) {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let widget = self.downgrade();
        scroll.connect_scroll(move |scroll, dx, dy| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), canvas.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            let Ok(mut canvas) = canvas.try_borrow_mut() else {
                return glib::Propagation::Stop;
            };
            let scale = widget.surface_scale();
            let keys = scroll.current_event_state();
            if keys.contains(gdk::ModifierType::CONTROL_MASK) {
                let (x, y) = widget.imp().mouse.get();
                canvas.zoom_about(
                    NOTCH_ZOOM.powf(-dy as f32),
                    (x * scale) as f32,
                    (y * scale) as f32,
                );
                return glib::Propagation::Stop;
            }
            // A wheel turns in notches; a touchpad scrolls in widget pixels.
            let per = match scroll.unit() {
                gdk::ScrollUnit::Wheel => NOTCH_SCROLL * scale,
                _ => scale,
            };
            let (dx, dy) = if keys.contains(gdk::ModifierType::SHIFT_MASK) && dx == 0.0 {
                (dy, 0.0)
            } else {
                (dx, dy)
            };
            canvas.scroll_wheel((dx * per) as f32, (dy * per) as f32);
            glib::Propagation::Stop
        });
        self.add_controller(scroll);
    }

    fn add_motion(&self, canvas: Weak<RefCell<Canvas>>) {
        let motion = gtk::EventControllerMotion::new();
        let (widget, moved) = (self.downgrade(), canvas.clone());
        motion.connect_motion(move |_, x, y| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), moved.upgrade()) else {
                return;
            };
            widget.imp().mouse.set((x, y));
            if let Ok(mut canvas) = canvas.try_borrow_mut() {
                canvas.mouse_moved(Some(widget.device_point(x, y)));
                widget.update_cursor(canvas.cursor());
            }
        });
        let widget = self.downgrade();
        motion.connect_leave(move |_| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), canvas.upgrade()) else {
                return;
            };
            if let Ok(mut canvas) = canvas.try_borrow_mut() {
                canvas.mouse_moved(None);
                widget.update_cursor(canvas.cursor());
            }
        });
        self.add_controller(motion);
    }

    /// The pen draws (plan §7.3): pressure from the stylus's axes, the eraser end from its tool.
    // shortcut: one sample a motion event, not the stylus's backlog; read `GestureStylus::backlog` if strokes look
    // coarse on a fast pen (H.1).
    fn add_stylus(&self, canvas: Weak<RefCell<Canvas>>) {
        let stylus = gtk::GestureStylus::new();
        let (widget, down) = (self.downgrade(), canvas.clone());
        stylus.connect_down(move |stylus, x, y| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), down.upgrade()) else {
                return;
            };
            let Ok(mut canvas) = canvas.try_borrow_mut() else {
                return;
            };
            canvas.pressed();
            let sample = widget.pen_sample(stylus, x, y, true);
            // K.18: the app may switch to Write mode.
            canvas.pen_contact();
            if canvas.pen_down(&sample) {
                // The pen's strokes are the canvas's: no selection under them.
                stylus.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        let (widget, motion) = (self.downgrade(), canvas.clone());
        stylus.connect_motion(move |stylus, x, y| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), motion.upgrade()) else {
                return;
            };
            let Ok(mut canvas) = canvas.try_borrow_mut() else {
                return;
            };
            if canvas.is_drawing(PEN) {
                canvas.pen_moved(&[widget.pen_sample(stylus, x, y, true)]);
            }
        });
        stylus.connect_up(move |_, _, _| {
            let Some(canvas) = canvas.upgrade() else {
                return;
            };
            if let Ok(mut canvas) = canvas.try_borrow_mut()
                && canvas.is_drawing(PEN)
            {
                canvas.pen_up();
            }
        });
        self.add_controller(stylus);
    }

    /// The stylus's sample at widget point `(x, y)`.
    fn pen_sample(&self, stylus: &gtk::GestureStylus, x: f64, y: f64, contact: bool) -> PenSample {
        let scale = self.surface_scale();
        let eraser = stylus
            .device_tool()
            .is_some_and(|tool| tool.tool_type() == gdk::DeviceToolType::Eraser);
        PenSample {
            id: PEN,
            x: (x * scale) as f32,
            y: (y * scale) as f32,
            pressure: stylus
                .axis(gdk::AxisUse::Pressure)
                .map(|pressure| pressure.clamp(0.0, 1.0) as f32),
            contact,
            eraser,
            input: 0,
        }
    }

    /// The first button: links, and selecting text by dragging (K.13). GTK keeps sending the drag's motion to the
    /// widget past its edges, as Windows' capture does.
    fn add_drag(&self, canvas: Weak<RefCell<Canvas>>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let (widget, begin) = (self.downgrade(), canvas.clone());
        drag.connect_drag_begin(move |drag, x, y| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), begin.upgrade()) else {
                return;
            };
            let Ok(mut canvas) = canvas.try_borrow_mut() else {
                return;
            };
            if canvas.is_drawing(PEN) {
                drag.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            canvas.pressed();
            if canvas.stop_pan() {
                return;
            }
            // V.9: Ctrl+click on a link opens it in another pane.
            if drag
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let (px, py) = widget.device_point(x, y);
                if canvas.open_elsewhere(px, py) {
                    return;
                }
            }
            let settings = gtk::Settings::default();
            let distance = settings
                .as_ref()
                .map_or(5, gtk::Settings::gtk_double_click_distance);
            let limits = ClickLimits {
                time: settings
                    .as_ref()
                    .map_or(400, gtk::Settings::gtk_double_click_time)
                    .max(0) as u32,
                width: distance,
                height: distance,
            };
            let (px, py) = widget.device_point(x, y);
            canvas.mouse_down(px, py, drag.current_event_time(), limits);
        });
        let (widget, end) = (self.downgrade(), canvas.clone());
        drag.connect_drag_end(move |drag, dx, dy| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), end.upgrade()) else {
                return;
            };
            let Some((x, y)) = drag.start_point() else {
                return;
            };
            if let Ok(mut canvas) = canvas.try_borrow_mut() {
                let (px, py) = widget.device_point(x + dx, y + dy);
                canvas.mouse_up(px, py);
            }
        });
        drag.connect_cancel(move |_, _| {
            if let Some(canvas) = canvas.upgrade()
                && let Ok(mut canvas) = canvas.try_borrow_mut()
            {
                canvas.capture_lost();
            }
        });
        self.add_controller(drag);
    }

    /// The middle button (middle-click scrolling, R.21) and the back and forward buttons (R.8).
    fn add_buttons(&self, canvas: Weak<RefCell<Canvas>>) {
        let click = gtk::GestureClick::new();
        // Every button; the first is the drag's.
        click.set_button(0);
        let widget = self.downgrade();
        click.connect_pressed(move |click, _, x, y| {
            let (Some(widget), Some(canvas)) = (widget.upgrade(), canvas.upgrade()) else {
                return;
            };
            let Ok(mut canvas) = canvas.try_borrow_mut() else {
                return;
            };
            match click.current_button() {
                gdk::BUTTON_MIDDLE => {
                    canvas.pressed();
                    let (px, py) = widget.device_point(x, y);
                    canvas.middle_down(px, py);
                }
                // X11 and Wayland number the back and forward buttons 8 and 9.
                8 => canvas.history_back(),
                9 => canvas.history_forward(),
                _ => {}
            }
            widget.update_cursor(canvas.cursor());
        });
        self.add_controller(click);
    }
}
