//! Windows page canvas (plan §5.4, K.8): the `ecclesia.page-canvas` NSH component, drawing PDF pages and overlays
//! directly into NSH's active Direct2D device context via [`nsh_platform_windows::NativeComponent::draw`].
//!
//! Sharing NSH's own Direct2D device and swap chain eliminates child-HWND airspace limits and clipping mismatches:
//! NSH overlays (menus, dialogs, popovers, command palette, settings) composite naturally over live pages without
//! hiding the canvas. The canvas's document, tile, layout, selection, and ink logic lives in `ecclesia_canvas::page`,
//! shared across all desktop platforms; this crate provides its Windows [`Host`](ecclesia_canvas::page::Host) and
//! NSH component bridge.

#[cfg(target_os = "windows")]
mod component;
#[cfg(target_os = "windows")]
mod gfx;
#[cfg(target_os = "windows")]
mod host;

#[cfg(target_os = "windows")]
pub use ecclesia_canvas::page::NAME;

/// Registers the page canvas with NSH under [`NAME`].
#[cfg(target_os = "windows")]
pub fn register(components: &nsh_platform_windows::Components) {
    use std::rc::Rc;

    use nsh_platform_windows::{ComponentContext, NativeComponent};

    components.register(
        NAME,
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            Box::new(component::PageCanvas::new(context))
        }),
    );
}
