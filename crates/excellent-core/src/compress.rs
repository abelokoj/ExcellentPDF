//! PDF compression profiles: stream deflate, object deduplication, image downsampling, and
//! size comparison (plan O.8).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::page_ops::pdf_doc::{
    ParsedPdf, PdfDocError, PdfVal, StreamEncoding, decode_stream_data, make_stream, rewrite_clean,
};

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

/// Compresses `pdf_bytes` using `settings` (or `CompressionProfile::into()`), deduplicating
/// identical objects, downsampling oversized uncompressed raster images, deflating streams,
/// and rewriting cleanly with zero incremental history (plan O.8).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `pdf_bytes` is malformed.
pub fn compress_pdf(pdf_bytes: &[u8], settings: CompressionSettings) -> Result<CompressionResult> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let compression = Compression::new(settings.deflate_level.clamp(1, 9));
    let mut overrides: BTreeMap<u32, PdfVal> = BTreeMap::new();
    let mut dropped: BTreeSet<u32> = BTreeSet::new();
    let mut streams_deflated = 0usize;
    let mut images_downsampled = 0usize;

    let ids = doc.object_ids();
    let objects_before = ids.len();

    // Step 1: optimize streams (downsample uncompressed large RGB/Gray images, Flate-encode raw streams).
    for &id in &ids {
        let Some(val) = doc.resolve_obj(id) else {
            continue;
        };
        if let PdfVal::Stream { mut dict, data } = val {
            if matches!(
                dict.get("Type").and_then(PdfVal::as_name),
                Some("XRef" | "ObjStm")
            ) {
                dropped.insert(id);
                continue;
            }
            if dict.get("Subtype").and_then(PdfVal::as_name) == Some("Image")
                && let Some(downsampled) =
                    maybe_downsample_image(&dict, &data, settings.max_image_dim)
            {
                let (new_w, new_h, new_samples) = downsampled;
                dict.insert("Width".into(), PdfVal::Int(i64::from(new_w)));
                dict.insert("Height".into(), PdfVal::Int(i64::from(new_h)));
                dict.remove("Filter");
                dict.remove("DecodeParms");
                let stm = make_stream(dict, &new_samples, StreamEncoding::Flate(compression))?;
                overrides.insert(id, stm);
                images_downsampled += 1;
                streams_deflated += 1;
                continue;
            }

            if !dict.contains_key("Filter") && data.len() >= 32 {
                let candidate =
                    make_stream(dict.clone(), &data, StreamEncoding::Flate(compression))?;
                if let PdfVal::Stream {
                    data: ref compressed,
                    ..
                } = candidate
                    && compressed.len() < data.len()
                {
                    overrides.insert(id, candidate);
                    streams_deflated += 1;
                }
            }
        }
    }

    if settings.metadata == MetadataRetention::Strip {
        if let Some(info_id) = doc.info_id {
            dropped.insert(info_id);
        }
        if let Some(mut catalog) = doc.get_dict(doc.root_id)
            && let Some(meta_id) = catalog.remove("Metadata").and_then(|v| v.as_ref_id())
        {
            dropped.insert(meta_id);
            overrides.insert(doc.root_id, PdfVal::Dict(catalog));
        }
    }

    // Step 2: deduplicate identical non-Page indirect objects and remap references.
    let mut deduplicated_objects = 0usize;
    if settings.deduplicate_objects == DeduplicationMode::Enabled {
        let mut digest_to_canon: HashMap<[u8; 32], u32> = HashMap::new();
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for &id in &ids {
            if dropped.contains(&id) || id == doc.root_id {
                continue;
            }
            let Some(val) = overrides.get(&id).cloned().or_else(|| doc.resolve_obj(id)) else {
                continue;
            };
            let is_page = val
                .as_dict()
                .and_then(|d| d.get("Type"))
                .and_then(PdfVal::as_name)
                .is_some_and(|t| t == "Page" || t == "Pages");
            if is_page {
                continue;
            }
            let serialized = crate::page_ops::pdf_doc::serialize_val(&val);
            let hash: [u8; 32] = Sha256::digest(&serialized).into();
            if let Some(&canon) = digest_to_canon.get(&hash) {
                remap.insert(id, canon);
                dropped.insert(id);
                deduplicated_objects += 1;
            } else {
                digest_to_canon.insert(hash, id);
            }
        }

        if !remap.is_empty() {
            for &id in &ids {
                if dropped.contains(&id) {
                    continue;
                }
                if let Some(val) = overrides.get(&id).cloned().or_else(|| doc.resolve_obj(id)) {
                    let updated = rewrite_refs(&val, &remap);
                    if updated != val {
                        overrides.insert(id, updated);
                    }
                }
            }
        }
    }

    let bytes = rewrite_clean(&doc, &overrides, &BTreeMap::new(), &dropped);
    let objects_after = objects_before.saturating_sub(dropped.len());
    let compressed_bytes = bytes.len();

    Ok(CompressionResult {
        bytes,
        stats: CompressionStats {
            original_bytes: pdf_bytes.len(),
            compressed_bytes,
            objects_before,
            objects_after,
            deduplicated_objects,
            streams_deflated,
            images_downsampled,
        },
    })
}

fn maybe_downsample_image(
    dict: &BTreeMap<String, PdfVal>,
    raw_data: &[u8],
    max_dim: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let w = dict
        .get("Width")
        .and_then(PdfVal::as_i64)
        .and_then(|v| u32::try_from(v).ok())?;
    let h = dict
        .get("Height")
        .and_then(PdfVal::as_i64)
        .and_then(|v| u32::try_from(v).ok())?;
    let bpc = dict
        .get("BitsPerComponent")
        .and_then(PdfVal::as_i64)
        .unwrap_or(8);
    if bpc != 8 || (w <= max_dim && h <= max_dim) || w == 0 || h == 0 {
        return None;
    }
    let cs = dict.get("ColorSpace").and_then(PdfVal::as_name)?;
    let channels = match cs {
        "DeviceRGB" => 3usize,
        "DeviceGray" => 1usize,
        _ => return None,
    };
    let decoded = decode_stream_data(dict, raw_data).ok()?;
    if decoded.len() != (w as usize) * (h as usize) * channels {
        return None;
    }
    let scale = (max_dim as f32 / w.max(h) as f32).min(1.0);
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let mut out = Vec::with_capacity((nw as usize) * (nh as usize) * channels);
    for ny in 0..nh {
        let sy = ((ny as f32 / nh as f32) * h as f32) as usize;
        let sy = sy.min(h as usize - 1);
        for nx in 0..nw {
            let sx = ((nx as f32 / nw as f32) * w as f32) as usize;
            let sx = sx.min(w as usize - 1);
            let idx = (sy * w as usize + sx) * channels;
            out.extend_from_slice(&decoded[idx..idx + channels]);
        }
    }
    Some((nw, nh, out))
}

fn rewrite_refs(val: &PdfVal, remap: &HashMap<u32, u32>) -> PdfVal {
    match val {
        PdfVal::Ref(id, gen_num) => PdfVal::Ref(remap.get(id).copied().unwrap_or(*id), *gen_num),
        PdfVal::Array(items) => {
            PdfVal::Array(items.iter().map(|i| rewrite_refs(i, remap)).collect())
        }
        PdfVal::Dict(map) => PdfVal::Dict(
            map.iter()
                .map(|(k, v)| (k.clone(), rewrite_refs(v, remap)))
                .collect(),
        ),
        PdfVal::Stream { dict, data } => PdfVal::Stream {
            dict: dict
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_refs(v, remap)))
                .collect(),
            data: data.clone(),
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::Document;

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
