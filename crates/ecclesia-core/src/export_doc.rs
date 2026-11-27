//! Batch Images <-> PDF conversion and PDF -> structured Markdown & OOXML `.docx` export
//! (plan O.9, O.10).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};

use ecclesia_mupdf::{Background, Document, Matrix, TextLayout};
use flate2::Compression;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::page_ops::pdf_doc::{
    PdfDocError, PdfVal, RawPdfWriter, StreamEncoding, fmt_f32, make_stream, rect_val,
};

/// Result alias for document export and conversion operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Paper size target when converting images to PDF (plan O.9).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub enum PdfPaperSize {
    /// Fit each page to the image's exact DPI dimensions (`72.0 * px / dpi`).
    #[default]
    FitToImage,
    /// US Letter (`612 × 792 pt`).
    Letter,
    /// ISO A4 (`595.28 × 841.89 pt`).
    A4,
    /// Custom size in points `(width_pt, height_pt)`.
    Custom {
        /// Page width in points.
        width_pt: f32,
        /// Page height in points.
        height_pt: f32,
    },
}

/// Input image for [`images_to_pdf`] (plan O.9).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ImageInput {
    /// Raw JPEG bytes (`/DCTDecode`).
    Jpeg {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Horizontal/vertical DPI used for [`PdfPaperSize::FitToImage`].
        dpi: f32,
        /// JPEG file bytes.
        bytes: Vec<u8>,
    },
    /// Uncompressed 8-bit RGB pixel buffer.
    Rgb8 {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// DPI used for [`PdfPaperSize::FitToImage`].
        dpi: f32,
        /// Interleaved RGB bytes (`width * height * 3`).
        samples: Vec<u8>,
    },
    /// Uncompressed 8-bit RGBA pixel buffer.
    Rgba8 {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// DPI used for [`PdfPaperSize::FitToImage`].
        dpi: f32,
        /// Interleaved RGBA bytes (`width * height * 4`).
        samples: Vec<u8>,
    },
    /// Encoded image bytes (PNG, GIF, BMP, TIFF, WebP, etc.) decoded via MuPDF on demand.
    Encoded {
        /// DPI used for [`PdfPaperSize::FitToImage`].
        dpi: f32,
        /// File name hint (e.g. `"photo.png"`).
        filename_hint: String,
        /// Raw image file bytes.
        bytes: Vec<u8>,
    },
}

/// Output raster image format for [`pdf_to_images`] (plan O.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RasterFormat {
    /// Lossless PNG.
    #[default]
    Png,
    /// Portable Pixmap (P6 binary RGB) for fast zero-overhead raw raster pipelines.
    Ppm,
}

/// Page selection for batch PDF-to-image conversion (plan O.9).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ExportPageRange {
    /// Every page in the document.
    #[default]
    All,
    /// Specific zero-based page indices.
    Pages(Vec<usize>),
}

/// Options controlling [`pdf_to_images`] (plan O.9).
#[derive(Debug, Clone, PartialEq)]
pub struct RasterExportOptions {
    /// Target rasterization DPI (default `150.0`).
    pub dpi: f32,
    /// Output image format.
    pub format: RasterFormat,
    /// Pages to export.
    pub pages: ExportPageRange,
}

impl Default for RasterExportOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            format: RasterFormat::Png,
            pages: ExportPageRange::All,
        }
    }
}

/// One exported page image produced by [`pdf_to_images`] (plan O.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedPageImage {
    /// Zero-based source page index.
    pub page_index: usize,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Output format.
    pub format: RasterFormat,
    /// Encoded image bytes.
    pub bytes: Vec<u8>,
}

/// Natural-sorts file paths so numeric segments compare numerically (`img2.png` < `img10.png`)
/// (plan O.9).
#[must_use]
pub fn natural_sort_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = paths.to_vec();
    out.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

/// Compares two strings using natural numeric ordering.
#[must_use]
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.char_indices().peekable();
    let mut bi = b.char_indices().peekable();
    while let (Some(&(ap, ac)), Some(&(bp, bc))) = (ai.peek(), bi.peek()) {
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let mut a_end = ap;
            while let Some(&(p, c)) = ai.peek() {
                if c.is_ascii_digit() {
                    a_end = p + c.len_utf8();
                    ai.next();
                } else {
                    break;
                }
            }
            let mut b_end = bp;
            while let Some(&(p, c)) = bi.peek() {
                if c.is_ascii_digit() {
                    b_end = p + c.len_utf8();
                    bi.next();
                } else {
                    break;
                }
            }
            let a_num = a[ap..a_end].trim_start_matches('0');
            let b_num = b[bp..b_end].trim_start_matches('0');
            let cmp = a_num
                .len()
                .cmp(&b_num.len())
                .then_with(|| a_num.cmp(b_num))
                .then_with(|| (a_end - ap).cmp(&(b_end - bp)));
            if cmp != Ordering::Equal {
                return cmp;
            }
        } else {
            let cmp = ac
                .to_ascii_lowercase()
                .cmp(&bc.to_ascii_lowercase())
                .then_with(|| ac.cmp(&bc));
            if cmp != Ordering::Equal {
                return cmp;
            }
            ai.next();
            bi.next();
        }
    }
    a.len().cmp(&b.len())
}

/// Converts `images` into a single multi-page PDF sized by `paper` and `margin_pt` (plan O.9).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `images` is empty or an image cannot be decoded/encoded.
pub fn images_to_pdf(
    images: &[ImageInput],
    paper: PdfPaperSize,
    margin_pt: f32,
) -> Result<Vec<u8>> {
    if images.is_empty() {
        return Err(PdfDocError::InvalidOperation(
            "images_to_pdf requires at least one image".into(),
        ));
    }
    let mut writer = RawPdfWriter::new();
    let catalog_id = writer.alloc();
    let pages_id = writer.alloc();
    let mut page_ids = Vec::with_capacity(images.len());
    let margin = margin_pt.max(0.0);

    for img in images {
        let (w_px, h_px, dpi, xobj_id) = write_image_xobject(&mut writer, img)?;
        let dpi = dpi.max(1.0);
        let img_w_pt = (w_px as f32 / dpi) * 72.0;
        let img_h_pt = (h_px as f32 / dpi) * 72.0;
        let (page_w, page_h) = match paper {
            PdfPaperSize::FitToImage => (img_w_pt + margin * 2.0, img_h_pt + margin * 2.0),
            PdfPaperSize::Letter => (612.0, 792.0),
            PdfPaperSize::A4 => (595.28, 841.89),
            PdfPaperSize::Custom {
                width_pt,
                height_pt,
            } => (width_pt.max(1.0), height_pt.max(1.0)),
        };
        let avail_w = (page_w - margin * 2.0).max(1.0);
        let avail_h = (page_h - margin * 2.0).max(1.0);
        let scale = (avail_w / img_w_pt.max(1.0)).min(avail_h / img_h_pt.max(1.0));
        let drawn_w = img_w_pt * scale;
        let drawn_h = img_h_pt * scale;
        let tx = (page_w - drawn_w) * 0.5;
        let ty = (page_h - drawn_h) * 0.5;

        let content = format!(
            "q\n{} 0 0 {} {} {} cm\n/Im0 Do\nQ\n",
            fmt_f32(drawn_w),
            fmt_f32(drawn_h),
            fmt_f32(tx),
            fmt_f32(ty)
        );
        let content_id = writer.alloc();
        writer.set_val(
            content_id,
            &make_stream(
                BTreeMap::new(),
                content.as_bytes(),
                StreamEncoding::Flate(Compression::default()),
            )?,
        );

        let mut xobj_map = BTreeMap::new();
        xobj_map.insert("Im0".into(), PdfVal::Ref(xobj_id, 0));
        let mut res = BTreeMap::new();
        res.insert("XObject".into(), PdfVal::Dict(xobj_map));

        let page_id = writer.alloc();
        let mut page_dict = BTreeMap::new();
        page_dict.insert("Type".into(), PdfVal::Name("Page".into()));
        page_dict.insert("Parent".into(), PdfVal::Ref(pages_id, 0));
        page_dict.insert("MediaBox".into(), rect_val([0.0, 0.0, page_w, page_h]));
        page_dict.insert("Contents".into(), PdfVal::Ref(content_id, 0));
        page_dict.insert("Resources".into(), PdfVal::Dict(res));
        writer.set_val(page_id, &PdfVal::Dict(page_dict));
        page_ids.push(page_id);
    }

    let mut pages_dict = BTreeMap::new();
    pages_dict.insert("Type".into(), PdfVal::Name("Pages".into()));
    pages_dict.insert(
        "Kids".into(),
        PdfVal::Array(page_ids.iter().map(|&id| PdfVal::Ref(id, 0)).collect()),
    );
    pages_dict.insert("Count".into(), PdfVal::Int(page_ids.len() as i64));
    writer.set_val(pages_id, &PdfVal::Dict(pages_dict));

    let mut catalog = BTreeMap::new();
    catalog.insert("Type".into(), PdfVal::Name("Catalog".into()));
    catalog.insert("Pages".into(), PdfVal::Ref(pages_id, 0));
    writer.set_val(catalog_id, &PdfVal::Dict(catalog));

    Ok(writer.finish(catalog_id, None))
}

fn write_image_xobject(
    writer: &mut RawPdfWriter,
    img: &ImageInput,
) -> Result<(u32, u32, f32, u32)> {
    match img {
        ImageInput::Jpeg {
            width,
            height,
            dpi,
            bytes,
        } => {
            let xobj_id = writer.alloc();
            let mut dict = BTreeMap::new();
            dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            dict.insert("Filter".into(), PdfVal::Name("DCTDecode".into()));
            let stm = make_stream(dict, bytes, StreamEncoding::Raw)?;
            writer.set_val(xobj_id, &stm);
            Ok((*width, *height, *dpi, xobj_id))
        }
        ImageInput::Rgb8 {
            width,
            height,
            dpi,
            samples,
        } => {
            let expected = (*width as usize) * (*height as usize) * 3;
            if samples.len() != expected {
                return Err(PdfDocError::InvalidOperation(format!(
                    "expected {expected} RGB bytes, got {}",
                    samples.len()
                )));
            }
            let xobj_id = writer.alloc();
            let mut dict = BTreeMap::new();
            dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            let stm = make_stream(dict, samples, StreamEncoding::Flate(Compression::default()))?;
            writer.set_val(xobj_id, &stm);
            Ok((*width, *height, *dpi, xobj_id))
        }
        ImageInput::Rgba8 {
            width,
            height,
            dpi,
            samples,
        } => {
            let pixels = (*width as usize) * (*height as usize);
            if samples.len() != pixels * 4 {
                return Err(PdfDocError::InvalidOperation(format!(
                    "expected {} RGBA bytes, got {}",
                    pixels * 4,
                    samples.len()
                )));
            }
            let mut rgb = Vec::with_capacity(pixels * 3);
            let mut alpha = Vec::with_capacity(pixels);
            for px in samples.as_chunks::<4>().0 {
                rgb.extend_from_slice(&px[..3]);
                alpha.push(px[3]);
            }
            let smask_id = writer.alloc();
            let mut smask_dict = BTreeMap::new();
            smask_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            smask_dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            smask_dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            smask_dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            smask_dict.insert("ColorSpace".into(), PdfVal::Name("DeviceGray".into()));
            smask_dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            writer.set_val(
                smask_id,
                &make_stream(
                    smask_dict,
                    &alpha,
                    StreamEncoding::Flate(Compression::default()),
                )?,
            );

            let xobj_id = writer.alloc();
            let mut dict = BTreeMap::new();
            dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            dict.insert("SMask".into(), PdfVal::Ref(smask_id, 0));
            writer.set_val(
                xobj_id,
                &make_stream(dict, &rgb, StreamEncoding::Flate(Compression::default()))?,
            );
            Ok((*width, *height, *dpi, xobj_id))
        }
        ImageInput::Encoded {
            dpi,
            filename_hint,
            bytes,
        } => {
            let doc = Document::from_bytes(bytes, filename_hint)
                .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
            let page = doc
                .load_page(0)
                .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
            let pix = page
                .render(&Matrix::IDENTITY, Background::White)
                .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
            let w = pix.width();
            let h = pix.height();
            let samples = pix.samples();
            let comp = pix.components();
            let stride = pix.stride();
            let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
            for y in 0..h as usize {
                let row = &samples[y * stride..y * stride + (w as usize) * comp];
                for px in row.chunks_exact(comp) {
                    rgb.extend_from_slice(&px[..3]);
                }
            }
            write_image_xobject(
                writer,
                &ImageInput::Rgb8 {
                    width: w,
                    height: h,
                    dpi: *dpi,
                    samples: rgb,
                },
            )
        }
    }
}

/// Renders PDF pages into PNG or PPM raster images at the configured DPI (plan O.9).
///
/// # Errors
///
/// Returns [`PdfDocError`] if the PDF cannot be opened or rendered, or a page index is out of range.
pub fn pdf_to_images(
    pdf_bytes: &[u8],
    options: &RasterExportOptions,
) -> Result<Vec<ExportedPageImage>> {
    let doc = Document::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let count = doc
        .page_count()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let indices: Vec<usize> = match &options.pages {
        ExportPageRange::All => (0..count).collect(),
        ExportPageRange::Pages(list) => {
            for &idx in list {
                if idx >= count {
                    return Err(PdfDocError::OutOfRange(format!(
                        "page {idx} out of range (count {count})"
                    )));
                }
            }
            list.clone()
        }
    };
    let scale = (options.dpi / 72.0).max(0.1);
    let ctm = Matrix::scale(scale, scale);

    let mut out = Vec::with_capacity(indices.len());
    for page_index in indices {
        let page = doc
            .load_page(page_index)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let pix = page
            .render(&ctm, Background::White)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let width = pix.width();
        let height = pix.height();
        let bytes = match options.format {
            RasterFormat::Png => pix
                .to_png()
                .map_err(|e| PdfDocError::StreamCodec(e.to_string()))?,
            RasterFormat::Ppm => {
                let mut buf = format!("P6\n{width} {height}\n255\n").into_bytes();
                let stride = pix.stride();
                let comp = pix.components();
                let samples = pix.samples();
                for y in 0..height as usize {
                    let row = &samples[y * stride..y * stride + (width as usize) * comp];
                    for px in row.chunks_exact(comp) {
                        buf.extend_from_slice(&px[..3]);
                    }
                }
                buf
            }
        };
        out.push(ExportedPageImage {
            page_index,
            width,
            height,
            format: options.format,
            bytes,
        });
    }
    Ok(out)
}

/// Loads all files from a directory sorted with [`natural_sort_paths`] and filters by common image
/// extensions.
#[must_use]
pub fn collect_sorted_image_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let filtered: Vec<PathBuf> = paths.iter().filter(|p| is_image_ext(p)).cloned().collect();
    natural_sort_paths(&filtered)
}

fn is_image_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| {
            matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tif" | "tiff" | "webp" | "ppm"
            )
        })
}

// ---------------------------------------------------------------------------
// O.10: PDF -> Structured Markdown & Valid OOXML .docx
// ---------------------------------------------------------------------------

/// Structural element inferred from PDF page text layouts (plan O.10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DocBlock {
    /// Section heading (`level` in `1..=3`).
    Heading {
        /// Heading level.
        level: u8,
        /// Heading text.
        text: String,
    },
    /// Body paragraph.
    Paragraph(String),
    /// List item.
    ListItem {
        /// Ordered (`1.`) vs unordered (`-`).
        ordered: bool,
        /// Item text without marker.
        text: String,
    },
    /// Structured table rows.
    Table(Vec<Vec<String>>),
}

/// Whether Markdown export includes page-break comments between source pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PageBreakMarkers {
    /// Emit `<!-- page N -->` separators between pages.
    Include,
    /// Omit page markers.
    #[default]
    Omit,
}

/// Options for [`pdf_to_markdown`] and [`pdf_to_docx`] (plan O.10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentExportOptions {
    /// Document title written as top-level heading / document header.
    pub title: String,
    /// Whether to emit page break markers in Markdown.
    pub page_breaks: PageBreakMarkers,
}

/// Extracts structured [`DocBlock`]s from MuPDF [`TextLayout`]s across pages (plan O.10).
#[must_use]
pub fn extract_structured_blocks(layouts: &[TextLayout]) -> Vec<Vec<DocBlock>> {
    let body_size = estimate_global_body_size(layouts);
    layouts
        .iter()
        .map(|layout| extract_page_blocks(layout, body_size))
        .collect()
}

fn estimate_global_body_size(layouts: &[TextLayout]) -> f32 {
    let mut sizes: Vec<f32> = layouts
        .iter()
        .flat_map(|l| l.chars.iter())
        .filter(|c| !c.c.is_whitespace() && c.size > 1.0)
        .map(|c| c.size)
        .collect();
    if sizes.is_empty() {
        return 11.0;
    }
    sizes.sort_by(f32::total_cmp);
    sizes[sizes.len() / 2]
}

fn split_table_cells(line: &str) -> Vec<String> {
    line.split("   ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect()
}

fn extract_page_blocks(layout: &TextLayout, body_size: f32) -> Vec<DocBlock> {
    let mut blocks: Vec<DocBlock> = Vec::new();
    let mut pending_table_rows: Vec<Vec<String>> = Vec::new();
    let flush_table = |blocks: &mut Vec<DocBlock>, rows: &mut Vec<Vec<String>>| {
        if rows.len() >= 2 {
            blocks.push(DocBlock::Table(std::mem::take(rows)));
        } else {
            for row in rows.drain(..) {
                blocks.push(DocBlock::Paragraph(row.join("   ")));
            }
        }
    };

    for block in &layout.blocks {
        let lines = layout.lines.get(block.lines.clone()).unwrap_or(&[]);
        if lines.is_empty() {
            continue;
        }
        let mut line_strings = Vec::with_capacity(lines.len());
        let mut size_sum = 0.0f32;
        let mut size_cnt = 0usize;
        for line in lines {
            let chars = layout.chars.get(line.chars.clone()).unwrap_or(&[]);
            let mut s = String::new();
            for ch in chars {
                if !ch.c.is_control() || ch.c == '\t' {
                    s.push(if ch.c == '\t' { ' ' } else { ch.c });
                }
                if !ch.c.is_whitespace() {
                    size_sum += ch.size;
                    size_cnt += 1;
                }
            }
            line_strings.push(s);
        }
        let joined = line_strings
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if joined.is_empty() {
            continue;
        }

        let avg_size = if size_cnt > 0 {
            size_sum / size_cnt as f32
        } else {
            body_size
        };

        // Detect table rows via multi-space column gaps within a block or across adjacent single-line blocks.
        if line_strings.len() == 1 {
            let cells = split_table_cells(&line_strings[0]);
            if cells.len() >= 2 {
                pending_table_rows.push(cells);
                continue;
            }
        }
        if line_strings.len() >= 2 {
            let rows: Vec<Vec<String>> = line_strings
                .iter()
                .map(|l| split_table_cells(l))
                .filter(|cells| cells.len() >= 2)
                .collect();
            if rows.len() >= 2 && rows.len() * 2 >= line_strings.len() {
                flush_table(&mut blocks, &mut pending_table_rows);
                blocks.push(DocBlock::Table(rows));
                continue;
            }
        }
        flush_table(&mut blocks, &mut pending_table_rows);

        if let Some(rest) = joined
            .strip_prefix("• ")
            .or_else(|| joined.strip_prefix("- "))
            .or_else(|| joined.strip_prefix("* "))
        {
            blocks.push(DocBlock::ListItem {
                ordered: false,
                text: rest.trim().to_owned(),
            });
            continue;
        }
        if let Some((num, rest)) = joined.split_once(". ")
            && !num.is_empty()
            && num.len() <= 3
            && num.chars().all(|c| c.is_ascii_digit())
            && !rest.trim().is_empty()
        {
            blocks.push(DocBlock::ListItem {
                ordered: true,
                text: rest.trim().to_owned(),
            });
            continue;
        }

        let ratio = avg_size / body_size.max(1.0);
        if ratio >= 1.15 && joined.len() <= 140 && line_strings.len() <= 3 {
            let level = if ratio >= 1.5 {
                1
            } else if ratio >= 1.3 {
                2
            } else {
                3
            };
            blocks.push(DocBlock::Heading {
                level,
                text: joined,
            });
        } else {
            blocks.push(DocBlock::Paragraph(joined));
        }
    }
    flush_table(&mut blocks, &mut pending_table_rows);
    blocks
}

/// Converts raw PDF bytes into structured GitHub-Flavored Markdown (plan O.10).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if MuPDF cannot read the PDF.
pub fn pdf_to_markdown(pdf_bytes: &[u8], options: &DocumentExportOptions) -> Result<String> {
    let layouts = extract_pdf_layouts(pdf_bytes)?;
    Ok(layouts_to_markdown(&layouts, options))
}

/// Converts extracted [`TextLayout`]s into structured GitHub-Flavored Markdown (plan O.10).
#[must_use]
pub fn layouts_to_markdown(layouts: &[TextLayout], options: &DocumentExportOptions) -> String {
    let pages = extract_structured_blocks(layouts);
    blocks_to_markdown(&pages, options)
}

/// Converts structured [`DocBlock`] pages into GitHub-Flavored Markdown (plan O.10).
#[must_use]
pub fn blocks_to_markdown(pages: &[Vec<DocBlock>], options: &DocumentExportOptions) -> String {
    let mut out = String::new();
    if !options.title.trim().is_empty() {
        let _ = write!(out, "# {}\n\n", options.title.trim());
    }
    for (idx, blocks) in pages.iter().enumerate() {
        if idx > 0 && options.page_breaks == PageBreakMarkers::Include {
            let _ = write!(out, "<!-- page {} -->\n\n", idx + 1);
        }
        let mut ord_counter = 1usize;
        for blk in blocks {
            match blk {
                DocBlock::Heading { level, text } => {
                    ord_counter = 1;
                    for _ in 0..(*level).clamp(1, 3) {
                        out.push('#');
                    }
                    let _ = write!(out, " {text}\n\n");
                }
                DocBlock::Paragraph(text) => {
                    ord_counter = 1;
                    let _ = write!(out, "{text}\n\n");
                }
                DocBlock::ListItem { ordered, text } => {
                    if *ordered {
                        let _ = writeln!(out, "{ord_counter}. {text}");
                        ord_counter += 1;
                    } else {
                        ord_counter = 1;
                        let _ = writeln!(out, "- {text}");
                    }
                }
                DocBlock::Table(rows) => {
                    ord_counter = 1;
                    if let Some(header) = rows.first() {
                        let cols = rows.iter().map(Vec::len).max().unwrap_or(header.len());
                        write_md_row(&mut out, header, cols);
                        out.push('|');
                        for _ in 0..cols {
                            out.push_str(" --- |");
                        }
                        out.push('\n');
                        for row in &rows[1..] {
                            write_md_row(&mut out, row, cols);
                        }
                        out.push('\n');
                    }
                }
            }
        }
        if !out.ends_with("\n\n") && !out.is_empty() {
            out.push('\n');
        }
    }
    out
}

fn write_md_row(out: &mut String, row: &[String], cols: usize) {
    out.push('|');
    for i in 0..cols {
        let cell = row.get(i).map_or("", String::as_str).replace('|', "\\|");
        let _ = write!(out, " {cell} |");
    }
    out.push('\n');
}

/// Converts raw PDF bytes into a valid OOXML `.docx` ZIP package (plan O.10).
///
/// # Errors
///
/// Returns [`PdfDocError`] if MuPDF cannot read the PDF or ZIP writing fails.
pub fn pdf_to_docx(pdf_bytes: &[u8], options: &DocumentExportOptions) -> Result<Vec<u8>> {
    let layouts = extract_pdf_layouts(pdf_bytes)?;
    let pages = extract_structured_blocks(&layouts);
    blocks_to_docx(&pages, options)
}

/// Converts structured [`DocBlock`] pages into a valid OOXML `.docx` ZIP package (plan O.10).
///
/// # Errors
///
/// Returns [`PdfDocError::StreamCodec`] if ZIP serialization fails.
pub fn blocks_to_docx(pages: &[Vec<DocBlock>], options: &DocumentExportOptions) -> Result<Vec<u8>> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>",
    );
    if !options.title.trim().is_empty() {
        let _ = write!(
            xml,
            "<w:p><w:pPr><w:pStyle w:val=\"Title\"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val=\"36\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            xml_escape(options.title.trim())
        );
    }

    for blocks in pages {
        for blk in blocks {
            match blk {
                DocBlock::Heading { level, text } => {
                    let lvl = (*level).clamp(1, 3);
                    let sz = match lvl {
                        1 => 32,
                        2 => 26,
                        _ => 22,
                    };
                    let _ = write!(
                        xml,
                        "<w:p><w:pPr><w:pStyle w:val=\"Heading{lvl}\"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val=\"{sz}\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                        xml_escape(text)
                    );
                }
                DocBlock::Paragraph(text) => {
                    let _ = write!(
                        xml,
                        "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                        xml_escape(text)
                    );
                }
                DocBlock::ListItem { ordered, text } => {
                    let prefix = if *ordered { "1. " } else { "• " };
                    let _ = write!(
                        xml,
                        "<w:p><w:r><w:t xml:space=\"preserve\">{}{}</w:t></w:r></w:p>",
                        xml_escape(prefix),
                        xml_escape(text)
                    );
                }
                DocBlock::Table(rows) => {
                    xml.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/></w:tblPr>");
                    for row in rows {
                        xml.push_str("<w:tr>");
                        for cell in row {
                            let _ = write!(
                                xml,
                                "<w:tc><w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>",
                                xml_escape(cell)
                            );
                        }
                        xml.push_str("</w:tr>");
                    }
                    xml.push_str("</w:tbl>");
                }
            }
        }
    }
    xml.push_str(
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:body></w:document>",
    );

    let styles_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:style w:type=\"paragraph\" w:styleId=\"Normal\" w:default=\"1\"><w:name w:val=\"Normal\"/></w:style>\
        <w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/></w:style>\
        <w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/></w:style>\
        <w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/></w:style>\
        <w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/></w:style>\
        </w:styles>";

    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
        <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
        </Types>";

    let root_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>";

    let doc_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
        </Relationships>";

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut write_entry = |name: &str, data: &str| -> Result<()> {
            zip.start_file(name, opts)
                .map_err(|e| PdfDocError::StreamCodec(e.to_string()))?;
            zip.write_all(data.as_bytes())
                .map_err(|e| PdfDocError::StreamCodec(e.to_string()))
        };
        write_entry("[Content_Types].xml", content_types)?;
        write_entry("_rels/.rels", root_rels)?;
        write_entry("word/document.xml", &xml)?;
        write_entry("word/styles.xml", styles_xml)?;
        write_entry("word/_rels/document.xml.rels", doc_rels)?;
        zip.finish()
            .map_err(|e| PdfDocError::StreamCodec(e.to_string()))?;
    }
    Ok(cursor.into_inner())
}

fn extract_pdf_layouts(pdf_bytes: &[u8]) -> Result<Vec<TextLayout>> {
    let doc = Document::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let count = doc
        .page_count()
        .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
    let mut layouts = Vec::with_capacity(count);
    for i in 0..count {
        let page = doc
            .load_page(i)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        let stext = page
            .text(None)
            .map_err(|e| PdfDocError::InvalidPdf(e.to_string()))?;
        layouts.push(stext.layout());
    }
    Ok(layouts)
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use zip::ZipArchive;

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn natural_sort_and_two_hundred_images_round_trip_to_pdf_and_png() {
        let unsorted = vec![
            PathBuf::from("photo10.png"),
            PathBuf::from("photo2.png"),
            PathBuf::from("photo1.png"),
            PathBuf::from("photo20.png"),
        ];
        let sorted = collect_sorted_image_paths(&unsorted);
        assert_eq!(
            sorted,
            vec![
                PathBuf::from("photo1.png"),
                PathBuf::from("photo2.png"),
                PathBuf::from("photo10.png"),
                PathBuf::from("photo20.png"),
            ]
        );

        let inputs: Vec<ImageInput> = (0..200)
            .map(|i| ImageInput::Rgb8 {
                width: 4,
                height: 4,
                dpi: 72.0,
                samples: vec![(i % 255) as u8; 48],
            })
            .collect();
        let pdf = images_to_pdf(&inputs, PdfPaperSize::A4, 18.0).expect("200 images to pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("mupdf open");
        assert_eq!(doc.page_count().expect("count"), 200);

        let imgs = pdf_to_images(
            &pdf,
            &RasterExportOptions {
                dpi: 72.0,
                format: RasterFormat::Png,
                pages: ExportPageRange::Pages(vec![0, 199]),
            },
        )
        .expect("pdf to png");
        assert_eq!(imgs.len(), 2);
        assert_eq!(&imgs[0].bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn pdf_exports_structured_markdown_and_valid_docx() {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT\n\
                /F1 20 Tf 1 0 0 1 72 720 Tm (Methods) Tj\n\
                /F1 11 Tf 1 0 0 1 72 680 Tm (Body text explaining the experiment.) Tj\n\
                1 0 0 1 72 640 Tm (ColA   ColB) Tj\n\
                1 0 0 1 72 620 Tm (10   20) Tj\n\
                ET"
            .into(),
            crop_box: None,
        });
        let pdf = b.build().expect("pdf");
        let opts = DocumentExportOptions {
            title: "Paper Export".into(),
            page_breaks: PageBreakMarkers::Include,
        };
        let md = pdf_to_markdown(&pdf, &opts).expect("md");
        assert!(md.contains("# Paper Export"), "title: {md}");
        assert!(md.contains("# Methods"), "heading: {md}");
        assert!(md.contains("| ColA | ColB |"), "table: {md}");

        let docx = pdf_to_docx(&pdf, &opts).expect("docx");
        let mut archive = ZipArchive::new(Cursor::new(docx)).expect("zip");
        let mut doc_xml = String::new();
        std::io::Read::read_to_string(
            &mut archive
                .by_name("word/document.xml")
                .expect("word/document.xml"),
            &mut doc_xml,
        )
        .expect("read xml");
        assert!(doc_xml.contains("Methods"));
        assert!(doc_xml.contains("<w:tbl>"));
    }
}
