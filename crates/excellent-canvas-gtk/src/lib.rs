//! Linux page canvas (plan §5.4, K.10): the `excellent.page-canvas` NSH component, a GTK 4 widget that draws the
//! shared canvas ([`excellent_canvas::page`]) as GSK render nodes, tiles as `GdkTexture`s, and reads the pen through
//! `GtkGestureStylus` (pressure; the eraser end). This crate is the canvas's GTK [`host`](excellent_canvas::page::Host)
//! and input; layouts, tiles, selection, find, reading aids and ink are the shared core's, as on Windows.
//!
//! Still to come here: touch gestures (U.3), screen-reader text (U.2's GTK side) and the pen's tilt.

#[cfg(target_os = "linux")]
mod component;
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
mod widget;

#[cfg(target_os = "linux")]
pub use excellent_canvas::page::NAME;

/// Registers the page canvas with NSH under [`NAME`].
#[cfg(target_os = "linux")]
pub fn register(components: &nsh_platform_linux::Components) {
    use std::rc::Rc;

    use nsh_platform_linux::{ComponentContext, NativeComponent};

    components.register(
        NAME,
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            Box::new(component::PageCanvas::new(context))
        }),
    );
}
