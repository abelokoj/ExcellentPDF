//! Phase 5 outline features: heading/numbered/structure-tag generation (B.8), multi-format
//! round-trip import/export (B.11), and custom editable sidecar outlines for non-PDF documents
//! (B.13).

pub mod actions;
pub mod generate;
pub mod io;
pub mod sidecar;

pub use generate::{
    GenerateOptions, HeadingCandidate, HeadingSource, NumberingPattern, OutlinePreviewTree,
    PreviewNode, StructureTagNode, detect_numbering_level, for_combined_files, from_structure_tags,
    generate_from_layouts,
};
pub use io::{
    OutlineFormat, OutlineIoError, export_indented_text, export_json, export_markdown,
    export_pdftk, import_indented_text, import_json, import_markdown, import_pdftk,
};
pub use sidecar::{SidecarError, SidecarOutlineStore, effective_outline};
