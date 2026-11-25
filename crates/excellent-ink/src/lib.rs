//! Ink: stroke model, smoothing, prediction, variable-width outlines and appearance streams
//!
//! See plan §5.4 and §8. H.1's pipeline: [`sample`]s as the hardware gives them, a [`width`] from pressure, tilt and
//! speed, [`predict`]ion for the preview, [`smooth`]ing to Béziers, the finished [`stroke`], and recorded
//! [`session`]s that tests replay. H.2: the [`pens`], and a [`raster`]izer the golden-image tests draw them with.
//! H.5: the [`erase`]rs. H.6: the [`lasso`]. H.9: a stroke's [`outline`] for its saved appearance. H.10: [`shapes`]. H.11: [`guides`].

pub mod erase;
pub mod guides;
pub mod lasso;
pub mod outline;
pub mod pens;
pub mod predict;
pub mod raster;
pub mod sample;
pub mod session;
pub mod shapes;
pub mod smooth;
pub mod stroke;
pub mod width;
