//! The `excellent.page-canvas` NSH component on Linux (plan D5, K.10): the canvas widget, its accessible name, and
//! the props and commands the shared canvas handles ([`excellent_canvas::page`]).

use std::cell::RefCell;
use std::rc::Rc;

use nsh_core::JsonValue;
use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::{ComponentContext, NativeComponent};

use gtk::prelude::*;

use crate::host::Canvas;
use crate::widget::CanvasWidget;

/// A page canvas in an NSH view.
pub(crate) struct PageCanvas {
    widget: CanvasWidget,
    canvas: Rc<RefCell<Canvas>>,
}

impl PageCanvas {
    /// A new canvas telling the app what happens through `context`.
    pub(crate) fn new(context: ComponentContext) -> Self {
        let (widget, canvas) = CanvasWidget::new(context);
        Self { widget, canvas }
    }
}

impl NativeComponent for PageCanvas {
    fn widget(&self) -> gtk::Widget {
        self.widget.clone().upcast()
    }

    /// `label`: the widget's accessible name. The rest: [`excellent_canvas::page::PageCanvas::set_prop`].
    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name == "label" {
            let text = value.and_then(JsonValue::as_str).unwrap_or("");
            self.widget
                .update_property(&[gtk::accessible::Property::Label(text)]);
            return;
        }
        self.canvas.borrow_mut().set_prop(name, value);
    }

    /// See [`excellent_canvas::page::PageCanvas::command`].
    fn command(&mut self, name: &str, args: &JsonValue) {
        self.canvas.borrow_mut().command(name, args);
    }
}
