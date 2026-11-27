//! Form flattening (plan O.13): bakes interactive `/Widget` annotations and their current
//! appearance streams into static page content streams while removing `/AcroForm` and widget
//! annotations from the flattened copy.

use std::fs;
use std::path::Path;

use ecclesia_mupdf::{Document, Forms};

use super::model::{AcroForm, FormError};

/// Whether flattening bakes form fields alone or also non-widget annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlattenTarget {
    /// Bake form widget appearances into static page content and keep non-widget annotations.
    FormsOnly,
    /// Bake both form widgets and annotations into static page content (MuPDF `pdf_bake_document`).
    #[default]
    FormsAndAnnotations,
}

/// Saves a flattened copy of `doc` to `dest`, baking form widget appearances into static page
/// content streams and removing interactive `/AcroForm` widgets (plan O.13).
///
/// The open `doc` and its undo history remain untouched.
///
/// # Errors
///
/// Returns [`FormError`] if `doc` is not a PDF or MuPDF fails to bake or write `dest`.
pub fn save_flattened_forms(doc: &Document, dest: impl AsRef<Path>) -> Result<(), FormError> {
    doc.save_flattened(dest.as_ref(), Forms::Flatten)?;
    Ok(())
}

/// Flattens in-memory PDF bytes (preserving visual appearance while stripping `/AcroForm` and
/// `/Widget` annotations) and returns the flattened PDF bytes.
///
/// # Errors
///
/// Returns [`FormError`] if `pdf_bytes` cannot be opened, flattened, or read back.
pub fn flatten_pdf_bytes(pdf_bytes: &[u8], target: FlattenTarget) -> Result<Vec<u8>, FormError> {
    let dir = std::env::temp_dir().join(format!(
        "ecclesia-flatten-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    fs::create_dir_all(&dir)?;
    let in_path = dir.join("input.pdf");
    let out_path = dir.join("flattened.pdf");
    let result = (|| -> Result<Vec<u8>, FormError> {
        fs::write(&in_path, pdf_bytes)?;
        let doc = Document::open_pdf(&in_path)?;
        let forms_mode = match target {
            FlattenTarget::FormsOnly | FlattenTarget::FormsAndAnnotations => Forms::Flatten,
        };
        doc.save_flattened(&out_path, forms_mode)?;
        let out_bytes = fs::read(&out_path)?;
        // Verify flattened output has no remaining interactive AcroForm fields.
        let check = AcroForm::from_pdf_bytes(&out_bytes)?;
        if !check.fields().is_empty() {
            return Err(FormError::PdfStructure(
                "flattened PDF still contains interactive form fields".to_owned(),
            ));
        }
        Ok(out_bytes)
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}
