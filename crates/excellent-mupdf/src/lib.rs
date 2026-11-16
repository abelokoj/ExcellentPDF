//! Safe Rust API over SumatraPDF's MuPDF 1.28.5 (plan §6.3, K.3).
//!
//! Every call goes through `excellent-mupdf-sys`'s C shim, which catches MuPDF's errors, so failures come back as
//! [`MuError`] and no `longjmp` crosses a Rust frame. Owned MuPDF objects are Rust types whose `Drop` releases them.
//!
//! # Threads
//!
//! MuPDF needs a context per thread. This crate keeps one per thread, cloned on first use from a process-wide base
//! context that has real locks, so callers never see contexts. What may cross threads is in the types:
//!
//! | Type | Send | Sync | Why |
//! | --- | --- | --- | --- |
//! | [`Document`] | yes | no | usable from one thread at a time; share it behind a `Mutex` |
//! | [`Page`], [`Annot`], [`StextPage`], [`Writer`] | no | no | tied to their document and thread |
//! | [`DisplayList`] | yes | yes | immutable; render it from several threads at once |
//! | [`Pixmap`] | yes | yes | owned pixels |

mod annot;
mod buffer;
mod context;
mod cookie;
mod display_list;
mod document;
mod edit;
mod error;
mod fonts;
mod geometry;
mod image;
mod link;
mod outline;
mod page;
mod pixmap;
mod text;
mod writer;

pub use annot::{Align, Annot, AnnotData, AnnotKind, Intent};
pub use cookie::Cookie;
pub use display_list::DisplayList;
pub use document::{Document, Reflow};
pub use edit::{Edit, NewOutlineEntry, OutlineChange, Save, UndoState};
pub use error::{MuError, Result};
pub use geometry::{DevicePoint, DeviceRect, Matrix, PageBox, PagePoint, PageRect, Quad};
pub use image::PageImage;
pub use link::{Link, Target};
pub use outline::{OutlineItem, OutlineStyle};
pub use page::Page;
pub use pixmap::{Background, Pixmap};
pub use text::{StextPage, TextBlock, TextChar, TextLayout, TextLine};
pub use writer::Writer;
