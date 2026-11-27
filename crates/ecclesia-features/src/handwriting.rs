//! Handwriting recognition, searchable ink index (H.23), and lasso handwriting-to-text
//! conversion (H.24).
//!
//! Provides a cross-platform [`Recognizer`] trait (supporting Windows InkAnalysis, Apple Vision,
//! Google ML Kit Digital Ink, ONNX fallback packs, and a built-in deterministic offline stroke-
//! feature/lexicon matcher), an FTS5-backed [`HandwritingIndex`] mapping recognized words to
//! stroke IDs and page bounding boxes, and a [`convert`] pipeline turning lassoed handwriting
//! into `FreeText` annotations while preserving line breaks and position.

pub mod convert;
pub mod index;
pub mod recognize;

pub use convert::{ConversionOutcome, ConversionTarget, ConvertOptions, convert_lasso_to_text};
pub use index::{
    BackendKind, HandwritingHit, HandwritingIndex, HandwritingStroke, IndexError, PendingPageQueue,
    RecognitionError, RecognizedWord, Recognizer,
};
pub use recognize::OfflineStrokeRecognizer;
