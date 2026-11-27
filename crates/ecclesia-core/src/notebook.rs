//! Notebook format (`.expdf`) and PDF export (plan N.4).
//!
//! A notebook is a ZIP archive containing:
//! - `document.pdf`: the underlying PDF document (kept incremental where possible),
//! - `notebook.json`: versioned notebook metadata (pages, procedural paper templates, sections,
//!   links, audio recordings, cover, margin restoration records, and rich text-block sidecars),
//! - `assets/`: binary assets such as audio recordings (`assets/audio/...`) and images/stickers
//!   (`assets/images/...`).
//!
//! See `docs/formats/expdf.md` for the full specification.

pub mod archive;
pub mod model;
pub mod pdf_ops;

pub use archive::{AttachAudio, ExportOptions, Notebook};
pub use model::{
    AudioFormat, AudioRecordingMeta, CoverMeta, CoverStyle, FORMAT_VERSION, LinkDestination,
    NotebookError, NotebookMetadata, Orientation, OriginalBoxes, PageKind, PageLinkMeta, PageMeta,
    PageSize, PaperTemplateId, Result, SectionMeta,
};
pub use pdf_ops::{
    OutlineEntrySpec, PdfBuilder, PdfLinkSpec, PdfLinkTarget, PdfPageInput, SlideSlot,
};
