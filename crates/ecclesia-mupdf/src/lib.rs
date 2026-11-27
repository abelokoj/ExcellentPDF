//! Safe Rust API over SumatraPDF's MuPDF 1.28.5 (plan §6.3, K.3).
//!
//! Every call goes through `ecclesia-mupdf-sys`'s C shim, which catches MuPDF's errors, so failures come back as
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
mod attachment;
mod buffer;
mod context;
mod cookie;
mod display_list;
mod document;
mod edit;
mod error;
mod exchange;
mod fonts;
mod geometry;
mod html;
mod image;
mod layer;
mod link;
mod outline;
mod page;
mod pixmap;
mod text;
mod writer;

pub use annot::{Align, Annot, AnnotData, AnnotKind, Intent, LineEnd};
pub use attachment::Attachment;
pub use cookie::Cookie;
pub use display_list::DisplayList;
pub use document::{Document, Reflow};
pub use edit::{
    CompressFlags, CompressStats, Edit, Forms, MergeOutlineMode, MergeSourceBytes, NewOutlineEntry,
    OutlineChange, PdfPageBoxEdit, PdfPageBoxKind, PdfPageLabelRange, RedactionFlags,
    RedactionSpec, Save, StampArt, UndoState, compress_pdf_bytes, insert_blank_page_bytes,
    merge_pdf_bytes, pdf_info_entries, read_pdf_page_labels, rearrange_pdf_bytes, redact_pdf_bytes,
    rotate_pages_pdf_bytes, set_pdf_page_boxes_bytes, set_pdf_page_labels_bytes,
    uncrop_pdf_pages_bytes,
};
pub use error::{MuError, Result};
pub use geometry::{DevicePoint, DeviceRect, Matrix, PageBox, PagePoint, PageRect, Quad};
pub use html::html_to_pdf;
pub use image::PageImage;
pub use layer::Layer;
pub use link::{Link, Target};
pub use outline::{OutlineItem, OutlineStyle};
pub use page::Page;
pub use pixmap::{Background, Pixmap};
pub use text::{StextPage, TextBlock, TextChar, TextLayout, TextLine};
pub use writer::Writer;
