//! Android page canvas: JNI `MotionEvent` batching/history, `SurfaceView` GPU tile upload bridge,
//! `AccessibilityNodeProvider` tree, and tablet two-pane stylus routing (plan §5.4, K.12, U.3, V.14).
//!
//! All core logic and primitive JNI/C-ABI entrypoints compile and pass tests portably on Windows, Linux,
//! macOS, and Android hosts without requiring the Android NDK at host-test time.

#![warn(unsafe_op_in_unsafe_fn)]

pub mod access;
pub mod jni_bridge;
pub mod motion;
pub mod split;
pub mod surface;

use std::fmt;

pub use access::{
    ActionOutcome, AndroidAccessibilityAction, AndroidAccessibilityNode, AndroidAccessibilityTree,
    AndroidNodeRole, HOST_VIEW_ID,
};
pub use jni_bridge::{
    AndroidCanvasHandle, FfiAndroidMotionSample, FfiAndroidRouteResult, FfiAndroidStatus,
    SafeAndroidCanvas,
};
pub use motion::{
    ActiveAndroidStroke, AndroidMotionSample, DispatchLatencyPolicy, MotionAcceptance,
    MotionAction, MotionBatcher, MotionToolType, SampleOrigin,
};
pub use split::{
    PaneSlot, PaneViewportState, SplitEligibility, SplitLinkTarget, SplitScreenMetrics,
    StylusRouteOutcome, TabletSplitController,
};
pub use surface::{
    AndroidSurfaceKind, AndroidTileSurfaceBridge, SurfacePixelFormat, TileUploadOrder,
    UploadedAndroidTile,
};

/// Errors produced by Android canvas operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum AndroidCanvasError {
    /// A numeric dimension, DPI, or coordinate was invalid.
    InvalidGeometry(String),
    /// A requested page or virtual accessibility node index was out of range.
    OutOfRange {
        /// Requested index or node id.
        index: i64,
        /// Available count.
        count: usize,
    },
    /// Opening or rendering a MuPDF page failed.
    RenderFailed(String),
    /// A JNI or C-ABI boundary validation or panic failure occurred.
    BridgeFailure(String),
}

impl fmt::Display for AndroidCanvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry(msg) => write!(f, "invalid android canvas geometry: {msg}"),
            Self::OutOfRange { index, count } => {
                write!(
                    f,
                    "android canvas index {index} out of range for count {count}"
                )
            }
            Self::RenderFailed(msg) => write!(f, "android tile render failed: {msg}"),
            Self::BridgeFailure(msg) => write!(f, "android bridge failure: {msg}"),
        }
    }
}

impl std::error::Error for AndroidCanvasError {}

/// Convenience result alias for Android canvas operations.
pub type Result<T> = std::result::Result<T, AndroidCanvasError>;
