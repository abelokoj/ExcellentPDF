//! Where pages sit (plan K.6, R.1, R.3): pure functions from page sizes and a [`Spec`] to page rectangles.
//!
//! Layout space is in page points (1/72 inch), y down, with its origin at the top-left corner of the first margin.
//! Sizes are as shown, after the view's rotation; turning page space into layout space is `ecclesia-core::transform`.
//!
//! - `spec`: what the reader chose (single, facing, book, horizontal, grid; continuous or paged; reading direction).
//! - `pages`: the result, [`Layout`], and its queries (pages in an area, the page at a point, bands).
//! - `arrange`: the placement itself, one function per family of layouts.

mod arrange;
mod pages;
#[cfg(test)]
mod properties;
mod spec;

pub use pages::{Axis, Band, GAP, Layout, PageSize};
pub use spec::{Direction, Flow, Mode, Spec};
