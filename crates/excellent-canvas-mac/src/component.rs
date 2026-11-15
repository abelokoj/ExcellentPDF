//! The `excellent.page-canvas` NSH component on macOS (plan D5, K.9): the canvas view, its accessible name, and the
//! props and commands the shared canvas handles ([`excellent_canvas::page`]).

use std::cell::RefCell;
use std::rc::Rc;

use nsh_core::JsonValue;
use nsh_platform_macos::{ComponentContext, NativeComponent};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;

use crate::host::Canvas;
use crate::view::CanvasView;

/// A page canvas in an NSH view.
pub(crate) struct PageCanvas {
    view: Retained<CanvasView>,
    canvas: Rc<RefCell<Canvas>>,
}

impl PageCanvas {
    /// A new canvas telling the app what happens through `context`; made on the main thread, as NSH makes components.
    pub(crate) fn new(mtm: MainThreadMarker, context: ComponentContext) -> Self {
        let (view, canvas) = CanvasView::new(mtm, context);
        Self { view, canvas }
    }
}

impl NativeComponent for PageCanvas {
    fn view(&self) -> Retained<NSView> {
        Retained::into_super(self.view.clone())
    }

    /// `label`: the view's accessible name. The rest: [`excellent_canvas::page::PageCanvas::set_prop`].
    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name == "label" {
            self.view
                .set_label(value.and_then(JsonValue::as_str).unwrap_or(""));
            return;
        }
        self.canvas.borrow_mut().set_prop(name, value);
    }

    /// See [`excellent_canvas::page::PageCanvas::command`].
    fn command(&mut self, name: &str, args: &JsonValue) {
        self.canvas.borrow_mut().command(name, args);
    }
}
