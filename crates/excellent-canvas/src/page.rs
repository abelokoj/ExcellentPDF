//! The page canvas's logic, shared by every platform (plan D5, K.8–K.10): the document a canvas shows, its layout,
//! viewport and tiles, input, selection and copy, links, find, the reading aids and ink, and the `excellent.page-canvas`
//! component's JSON props, commands and events.
//!
//! A platform layer makes the view, implements [`Host`] for it (drawing, redraws, events, waking from workers) and
//! forwards input and the component's props and commands to its [`PageCanvas`].

mod canvas;
mod clicks;
mod component;
pub mod gestures;
pub mod host;
mod near;
mod placement;
mod saved;

pub use canvas::{Cursor, PageCanvas, PenSample, ZoomChange};
pub use clicks::ClickLimits;
pub use excellent_ink::sample::Tilt;
pub use host::{Host, Wash};

/// The component's name, as the app's `native(...)` element uses it.
pub const NAME: &str = "excellent.page-canvas";
