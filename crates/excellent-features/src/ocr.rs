//! OCR to searchable PDF (plan O.4): engine abstraction, language packs, image preprocessing,
//! scanned-page detection, and invisible PDF text-layer (`Tr 3`) placement.
//!
//! Existing vector graphics, images, annotations, and outlines in the PDF are preserved verbatim
//! by appending an incremental PDF update with a `3 Tr` (invisible) text overlay stream.

pub mod engine;
pub mod model;
pub mod pdf_layer;

pub use engine::{
    AutoOcrEngine, DeterministicTestEngine, OcrEngine, TesseractCliEngine, WindowsOcrEngine,
    encode_ppm, parse_hocr,
};
pub use model::{
    BinarizeMode, DenoiseMode, DeskewMode, ExistingTextPolicy, LanguagePack, OcrConfig, OcrError,
    OcrPageResult, PageImageInput, PageScanStatus, PixelFormat, PreprocessConfig, WordBox,
    detect_scanned_page, discover_installed_packs, preprocess_image, standard_language_packs,
};
pub use pdf_layer::{
    OcrDocumentReport, OcrPageReport, TextLayerPlan, TextPlacement, apply_text_layer_to_pdf_bytes,
    build_text_layer_plan, ocr_pdf_bytes, ocr_pdf_file,
};
