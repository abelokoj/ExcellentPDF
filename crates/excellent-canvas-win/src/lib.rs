//! Windows page canvas (plan §5.4, K.8): the `excellent.page-canvas` NSH component, a child window drawn with
//! Direct2D on a DXGI flip-model swap chain, showing tiles that `excellent-core`'s workers render.
//!
//! Built so far: the window, presenting, scroll and zoom, and tiles (T7); pen input through `WM_POINTER` with a
//! pressure-width stroke drawn in the same frame (the T8 spike); layouts, fit modes, view rotation and page boxes
//! (K.6, R.1–R.4: `excellent_core::view`'s commands, and `placement` keeping ink on its page); one shared document per
//! file through `excellent_core::registry` (K.5), back and forward (R.8), and text selection and copy (K.13). Still
//! to come: the GDI fallback (K.8), touch (U.3) and overlays.

#[cfg(target_os = "windows")]
mod canvas;
#[cfg(target_os = "windows")]
mod clicks;
#[cfg(target_os = "windows")]
mod component;
#[cfg(target_os = "windows")]
mod gfx;
#[cfg(target_os = "windows")]
mod near;
#[cfg(target_os = "windows")]
mod placement;
#[cfg(target_os = "windows")]
mod pointer;
#[cfg(target_os = "windows")]
mod saved;
#[cfg(target_os = "windows")]
mod window;

#[cfg(target_os = "windows")]
pub use component::NAME;

/// Registers the page canvas with NSH under [`NAME`].
#[cfg(target_os = "windows")]
pub fn register(components: &nsh_platform_windows::Components) {
    use std::rc::Rc;

    use nsh_platform_windows::{ComponentContext, NativeComponent};

    /// Draws nothing: stands in when Windows can't make the canvas window.
    struct Missing;
    impl NativeComponent for Missing {
        fn set_prop(&mut self, _name: &str, _value: Option<&nsh_core::JsonValue>) {}
    }

    components.register(
        NAME,
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            match component::PageCanvas::new(context) {
                Some(canvas) => Box::new(canvas),
                None => Box::new(Missing),
            }
        }),
    );
    // The app registers its components before NSH starts, so the device loads while NSH makes its window.
    gfx::prewarm();
}
