//! PDF compression profiles: MuPDF stream deflate, object deduplication, image downsampling, and
//! size comparison (plan O.8, D3, D4).

use ecclesia_mupdf::{CompressFlags, compress_pdf_bytes};

use crate::page_ops::pdf_doc::PdfDocError;

/// Result alias for compression operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Predefined PDF optimization profile (plan O.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CompressionProfile {
    /// Aggressive 72 DPI downsampling and maximum deflate for small screen/web sharing.
    Screen,
    /// Balanced 150 DPI downsampling and object deduplication for e-books.
    #[default]
    Ebook,
    /// High-resolution 300 DPI lossless compression for print workflows.
    Print,
}

/// Whether metadata streams (`/Info`, `/Metadata`) are stripped during compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MetadataRetention {
    /// Keep metadata intact.
    #[default]
    Preserve,
    /// Strip `/Info` and XMP `/Metadata`.
    Strip,
}

/// Detailed compression parameters derived from a [`CompressionProfile`] or customized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionSettings {
    /// Maximum pixel dimension / DPI threshold for uncompressed raster images.
    pub max_image_dim: u32,
    /// Zlib compression level (`1..=9`).
    pub deflate_level: u32,
    /// Whether identical streams and dictionaries are deduplicated.
    pub deduplicate_objects: DeduplicationMode,
    /// Whether metadata is preserved or stripped.
    pub metadata: MetadataRetention,
}

/// Whether identical indirect PDF objects are merged during compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DeduplicationMode {
    /// Deduplicate identical indirect objects and remap references.
    #[default]
    Enabled,
    /// Keep object identities separate.
    Disabled,
}

impl From<CompressionProfile> for CompressionSettings {
    fn from(profile: CompressionProfile) -> Self {
        match profile {
            CompressionProfile::Screen => Self {
                max_image_dim: 600,
                deflate_level: 9,
                deduplicate_objects: DeduplicationMode::Enabled,
                metadata: MetadataRetention::Strip,
            },
            CompressionProfile::Ebook => Self {
                max_image_dim: 1200,
                deflate_level: 6,
                deduplicate_objects: DeduplicationMode::Enabled,
                metadata: MetadataRetention::Preserve,
            },
            CompressionProfile::Print => Self {
                max_image_dim: 2400,
                deflate_level: 6,
                deduplicate_objects: DeduplicationMode::Enabled,
                metadata: MetadataRetention::Preserve,
            },
        }
    }
}

/// Size and optimization metrics returned alongside compressed PDF bytes (plan O.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressionStats {
    /// Input PDF byte length.
    pub original_bytes: usize,
    /// Compressed PDF byte length.
    pub compressed_bytes: usize,
    /// Indirect object count before compression.
    pub objects_before: usize,
    /// Indirect object count after compression.
    pub objects_after: usize,
    /// Number of duplicate indirect objects merged.
    pub deduplicated_objects: usize,
    /// Number of uncompressed streams compressed with Flate.
    pub streams_deflated: usize,
    /// Number of oversized uncompressed images downsampled.
    pub images_downsampled: usize,
}

impl CompressionStats {
    /// Percentage reduction (`0.0..=100.0`, or negative if input was already tiny).
    #[must_use]
    pub fn savings_percent(&self) -> f64 {
        if self.original_bytes == 0 {
            return 0.0;
        }
        let saved = self.original_bytes as f64 - self.compressed_bytes as f64;
        (saved / self.original_bytes as f64) * 100.0
    }
}

/// Compressed PDF output and statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct CompressionResult {
    /// Resulting PDF bytes.
    pub bytes: Vec<u8>,
    /// Detailed before/after statistics.
    pub stats: CompressionStats,
}

/// Compresses `pdf_bytes` using `settings` (or `CompressionProfile::into()`) through MuPDF,
/// deduplicating identical objects, downsampling oversized raster images, deflating streams,
/// and rewriting cleanly with zero incremental history (plan O.8, D3, D4).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `pdf_bytes` is malformed.
pub fn compress_pdf(pdf_bytes: &[u8], settings: CompressionSettings) -> Result<CompressionResult> {
    let flags = CompressFlags {
        max_image_dim: settings.max_image_dim,
        deflate_level: settings.deflate_level.clamp(1, 9),
        deduplicate_objects: settings.deduplicate_objects == DeduplicationMode::Enabled,
        strip_metadata: settings.metadata == MetadataRetention::Strip,
        clean_streams: false,
        linearize: false,
    };
    let (bytes, stats) =
        compress_pdf_bytes(pdf_bytes, flags).map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let compressed_bytes = bytes.len();

    Ok(CompressionResult {
        bytes,
        stats: CompressionStats {
            original_bytes: pdf_bytes.len(),
            compressed_bytes,
            objects_before: stats.objects_before,
            objects_after: stats.objects_after,
            deduplicated_objects: stats.deduplicated_objects,
            streams_deflated: stats.streams_deflated,
            images_downsampled: stats.images_downsampled,
        },
    })
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::Document;

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn compression_deflates_deduplicates_and_downsamples_cleanly() {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        let big_text = "Repeated paragraph content for compression testing. ".repeat(40);
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: format!("BT /F1 12 Tf 72 720 Td ({big_text}) Tj ET"),
            crop_box: None,
        });
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: format!("BT /F1 12 Tf 72 720 Td ({big_text}) Tj ET"),
            crop_box: None,
        });
        let raw_pdf = b.build().expect("build");

        let res = compress_pdf(&raw_pdf, CompressionProfile::Screen.into()).expect("compress");
        assert!(
            res.stats.compressed_bytes < res.stats.original_bytes,
            "compressed {} < original {}",
            res.stats.compressed_bytes,
            res.stats.original_bytes
        );
        assert!(res.stats.streams_deflated >= 1);
        assert!(res.stats.deduplicated_objects >= 1);
        assert!(res.stats.savings_percent() > 0.0);

        let doc =
            Document::from_bytes(&res.bytes, "application/pdf").expect("mupdf opens compressed");
        assert_eq!(doc.page_count().expect("pages"), 2);
    }
}
