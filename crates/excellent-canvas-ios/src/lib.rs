//! iPadOS page canvas: Apple Pencil input, Metal/CG tile surface bridge, C-ABI FFI, `UIAccessibility` rotor,
//! and tablet split-pane routing (plan §5.4, K.11, U.3, V.14).
//!
//! All core models, tile surface preparation, accessibility tree queries, split-pane routing, and C-ABI FFI
//! entrypoints compile and run portably on every host (including Windows CI) without requiring the iOS SDK;
//! Darwin/UIKit-specific constants and helpers are gated cleanly with `#[cfg(target_os = "ios")]`.

#![warn(unsafe_op_in_unsafe_fn)]

pub mod access;
pub mod ffi;
pub mod pencil;
pub mod split;
pub mod surface;

use std::fmt;

pub use access::{
    AccessibilityElementKind, AccessibilityRotorKind, IosAccessibilityNode, IosAccessibilityTree,
    RotorDirection, RotorNavigationOutcome,
};
pub use ffi::{
    FfiAccessibilityNode, FfiHoverEvent, FfiPencilSample, FfiSplitMetrics, FfiSplitRouteResult,
    FfiStatus, FfiTileDescriptor, IosCanvasHandle, SafeIosCanvas,
};
pub use pencil::{
    ActivePencilStroke, HoverPhase, PencilGesture, PencilHoverSample, PencilPreferredAction,
    PencilProcessor, PencilSample, PencilToolMode, SampleAcceptance, ScribblePolicy,
    ToolToggleOutcome, TouchKind, TouchSampleOrigin,
};
pub use split::{
    PaneSlot, PaneViewportState, SplitEligibility, SplitLinkTarget, SplitScreenMetrics,
    StylusRouteOutcome, TabletSplitController,
};
pub use surface::{
    TilePixelFormat, TileSurfaceBridge, TileSurfaceKind, TileUploadOrder, UploadedTileSurface,
};

/// Errors produced by iPadOS canvas operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum IosCanvasError {
    /// A numeric dimension, scale, or coordinate was invalid (non-finite or non-positive).
    InvalidGeometry(String),
    /// A requested page or element index was outside the document bounds.
    OutOfRange {
        /// Requested zero-based index.
        index: usize,
        /// Total available count.
        count: usize,
    },
    /// Opening or rendering a MuPDF page failed.
    RenderFailed(String),
    /// A C-ABI FFI call encountered a null pointer, invalid UTF-8, or caught panic.
    FfiFailure(String),
}

impl fmt::Display for IosCanvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry(msg) => write!(f, "invalid canvas geometry: {msg}"),
            Self::OutOfRange { index, count } => {
                write!(f, "canvas index {index} out of range for count {count}")
            }
            Self::RenderFailed(msg) => write!(f, "canvas tile render failed: {msg}"),
            Self::FfiFailure(msg) => write!(f, "ios canvas ffi failure: {msg}"),
        }
    }
}

impl std::error::Error for IosCanvasError {}

/// Convenience result alias for iPadOS canvas operations.
pub type Result<T> = std::result::Result<T, IosCanvasError>;
