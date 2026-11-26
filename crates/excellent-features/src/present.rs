//! Teaching and presentation mode (plan §27.11, G.1–G.8).
//!
//! Pure Rust state models for full-screen presentation & USB clicker keys ([`session`], G.1),
//! same-frame canvas pointer overlays — laser with fading trails, spotlight, and magnifier
//! ([`pointers`], G.2–G.4), temporary & fading slide ink ([`ink`], G.5), temporary whiteboard
//! pages ([`whiteboard`], G.6), curtain reveal ([`curtain`], G.7), and multi-monitor presenter
//! view sync with disconnect resilience ([`presenter`], G.8).

pub mod curtain;
pub mod ink;
pub mod pointers;
pub mod presenter;
pub mod session;
pub mod whiteboard;

pub use curtain::{CurtainDirection, CurtainReveal};
pub use ink::{InkPersistence, SlideInkLayer, SlideStroke};
pub use pointers::{
    ActivePointerTool, LaserMode, LaserPointer, LaserStroke, LaserTrailPoint, MagnifierLens,
    MagnifierView, Spotlight, SpotlightOverlay, SpotlightShape, normalize_laser_width,
};
pub use presenter::{
    DisplayInfo, DisplayRect, PresenterNotes, PresenterTimer, PresenterViewSync, WindowRole,
};
pub use session::{ClickerKey, PresentAction, PresentSession, ScreenBlank, ToolbarVisibility};
pub use whiteboard::{WhiteboardBackground, WhiteboardDeck, WhiteboardPage};
