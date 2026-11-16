//! Ink: stroke model, smoothing, prediction, variable-width outlines and appearance streams
//!
//! See plan §5.4 and §8. H.1's pipeline: [`sample`]s as the hardware gives them, a [`width`] from pressure, tilt and
//! speed, [`predict`]ion for the preview, [`smooth`]ing to Béziers, the finished [`stroke`], and recorded
//! [`session`]s that tests replay.

pub mod predict;
pub mod sample;
pub mod session;
pub mod smooth;
pub mod stroke;
pub mod width;
