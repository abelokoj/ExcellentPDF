//! Document model, layout, coordinate transforms, selection, render scheduling, tile cache and save
//!
//! See plan §5.4. Built up task by task (plan §22).

pub mod anchor;
pub mod darkmode;
pub mod document;
pub mod edit;
pub mod formats;
mod oplog;
pub mod outline;
pub mod print;
pub mod registry;
pub mod render;
mod save;
pub mod shared;
pub mod thumbnail;
pub mod transform;
