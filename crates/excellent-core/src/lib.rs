//! Document model, layout, coordinate transforms, selection, render scheduling, tile cache and save
//!
//! See plan §5.4. Built up task by task (plan §22).

pub mod darkmode;
pub mod document;
pub mod outline;
pub mod registry;
pub mod render;
pub mod shared;
pub mod thumbnail;
pub mod transform;
pub mod view;
