//! macOS page canvas (plan §5.4, K.9): the `excellent.page-canvas` NSH component, an `NSView` that draws the shared
//! canvas ([`excellent_canvas::page`]) with Core Graphics, tiles as `CGImage`s, and reads trackpad scrolling and
//! pinches, the mouse, and the pen's tablet events (pressure; the eraser end). This crate is the canvas's AppKit
//! [`host`](excellent_canvas::page::Host) and input; layouts, tiles, selection, find, reading aids and ink are the
//! shared core's, as on Windows and Linux.
//!
//! VoiceOver reads the pages' text through the view's accessibility text-area methods (U.3, `access`). Still to come
//! here: the pen's tilt.

#[cfg(target_os = "macos")]
mod access;
#[cfg(target_os = "macos")]
mod component;
#[cfg(target_os = "macos")]
mod host;
#[cfg(target_os = "macos")]
mod view;

#[cfg(target_os = "macos")]
pub use excellent_canvas::page::NAME;

/// Registers the page canvas with NSH under [`NAME`].
#[cfg(target_os = "macos")]
pub fn register(components: &nsh_platform_macos::Components) {
    use std::rc::Rc;

    use nsh_platform_macos::{ComponentContext, NativeComponent};
    use objc2::MainThreadMarker;

    components.register(
        NAME,
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            let mtm = MainThreadMarker::new().expect("NSH makes components on the main thread");
            Box::new(component::PageCanvas::new(mtm, context))
        }),
    );
}
