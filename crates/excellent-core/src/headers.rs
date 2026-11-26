//! Page headers, footers, and watermarks with template variables, opacity, rotation, and
//! removable OCG layer tagging (plan O.18).

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::PI;
use std::fmt::Write as _;

use flate2::Compression;

use crate::page_ops::PageApplyScope;
use crate::page_ops::pdf_doc::{
    ParsedPdf, PdfDocError, PdfVal, StreamEncoding, escape_pdf_string, fmt_f32, make_stream,
    rewrite_clean,
};

/// Result alias for header/footer/watermark operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// OCG layer name used for headers and footers so they can be found and removed cleanly.
pub const HEADER_FOOTER_OCG: &str = "ExcellentPDF_HeaderFooter";
/// OCG layer name used for watermarks so they can be found and removed cleanly.
pub const WATERMARK_OCG: &str = "ExcellentPDF_Watermark";
/// OCG layer name used for Bates numbering stamps.
pub const BATES_OCG: &str = "ExcellentPDF_Bates";

/// Whether a watermark sits behind existing page content or above it (plan O.18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum WatermarkLayer {
    /// Drawn behind existing page graphics.
    Background,
    /// Drawn on top of existing page graphics.
    #[default]
    Foreground,
}

/// Template metadata supplied for `{page}`, `{pages}`, `{date}`, `{time}`, `{title}`, `{author}`,
/// and `{filename}` variable expansion (plan O.18).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateContext {
    /// Document title (`{title}`).
    pub title: String,
    /// Document author (`{author}`).
    pub author: String,
    /// Source file name (`{filename}`).
    pub filename: String,
    /// Formatted date string (`{date}`).
    pub date: String,
    /// Formatted time string (`{time}`).
    pub time: String,
}

/// Expands header/footer/watermark template placeholders for 1-based `page_num` of `total_pages`.
#[must_use]
pub fn expand_template(
    template: &str,
    page_num: usize,
    total_pages: usize,
    ctx: &TemplateContext,
) -> String {
    template
        .replace("{page}", &page_num.to_string())
        .replace("{pages}", &total_pages.to_string())
        .replace("{total}", &total_pages.to_string())
        .replace("{date}", &ctx.date)
        .replace("{time}", &ctx.time)
        .replace("{title}", &ctx.title)
        .replace("{author}", &ctx.author)
        .replace("{filename}", &ctx.filename)
}

/// Header and footer slots across the top and bottom of a page (plan O.18).
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderFooterSpec {
    /// Top-left template.
    pub top_left: Option<String>,
    /// Top-center template.
    pub top_center: Option<String>,
    /// Top-right template.
    pub top_right: Option<String>,
    /// Bottom-left template.
    pub bottom_left: Option<String>,
    /// Bottom-center template.
    pub bottom_center: Option<String>,
    /// Bottom-right template.
    pub bottom_right: Option<String>,
    /// Font size in points.
    pub font_size_pt: f32,
    /// RGB text color (`0.0..=1.0`).
    pub color_rgb: [f32; 3],
    /// Opacity (`0.0..=1.0`).
    pub opacity: f32,
    /// Margin from page edges in points.
    pub margin_pt: f32,
    /// Target pages.
    pub scope: PageApplyScope,
}

impl Default for HeaderFooterSpec {
    fn default() -> Self {
        Self {
            top_left: None,
            top_center: None,
            top_right: None,
            bottom_left: None,
            bottom_center: Some("Page {page} of {pages}".into()),
            bottom_right: None,
            font_size_pt: 10.0,
            color_rgb: [0.25, 0.25, 0.25],
            opacity: 1.0,
            margin_pt: 28.0,
            scope: PageApplyScope::All,
        }
    }
}

/// Content rendered as a watermark (plan O.18).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WatermarkContent {
    /// Centered rotated text watermark.
    Text {
        /// Template text (supports `{page}`, `{date}`, etc.).
        text: String,
        /// Font size in points.
        font_size_pt: f32,
        /// RGB fill color in `0.0..=1.0`.
        color_rgb: [f32; 3],
    },
    /// Embedded PDF vector Form XObject watermark (first page of `pdf_bytes`).
    PdfPage {
        /// One-page (or multi-page, page 0 used) PDF bytes.
        pdf_bytes: Vec<u8>,
    },
    /// Embedded RGB raster image watermark.
    ImageRgb {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
        /// Interleaved RGB bytes (`width * height * 3`).
        samples: Vec<u8>,
    },
}

/// Watermark placement and appearance specification (plan O.18).
#[derive(Debug, Clone, PartialEq)]
pub struct WatermarkSpec {
    /// Watermark payload.
    pub content: WatermarkContent,
    /// Opacity (`0.0..=1.0`).
    pub opacity: f32,
    /// Rotation angle in degrees counter-clockwise (e.g. `45.0` for diagonal).
    pub angle_deg: f32,
    /// Scale factor (`1.0` = default).
    pub scale: f32,
    /// Background vs foreground layer placement.
    pub layer: WatermarkLayer,
    /// Target pages.
    pub scope: PageApplyScope,
}

pub(crate) struct OverlayInjection<'a> {
    pub(crate) ocg_name: &'a str,
    pub(crate) oc_res_key: &'a str,
    pub(crate) gs_res_key: &'a str,
    pub(crate) alpha: f32,
    pub(crate) layer: WatermarkLayer,
    pub(crate) per_page_ops: &'a BTreeMap<usize, String>,
    pub(crate) extra_xobj: Option<(&'a str, PdfVal)>,
}

/// Applies headers and footers to `pdf_bytes` inside an OCG-tagged stream so they can be
/// removed cleanly later (plan O.18).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `pdf_bytes` is malformed or any target page is out of range.
pub fn apply_headers_footers(
    pdf_bytes: &[u8],
    spec: &HeaderFooterSpec,
    ctx: &TemplateContext,
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let target_indices = resolve_scope(&spec.scope, pages.len())?;
    let total = pages.len();
    let font_size = spec.font_size_pt.max(4.0);
    let [r, g, b] = spec.color_rgb.map(|c| c.clamp(0.0, 1.0));
    let alpha = spec.opacity.clamp(0.0, 1.0);
    let margin = spec.margin_pt.max(4.0);

    let mut per_page_ops: BTreeMap<usize, String> = BTreeMap::new();
    for idx in target_indices {
        let pid = pages[idx];
        let media = doc
            .effective_box(pid, "MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let w = media[2] - media[0];
        let mut ops = String::new();
        let _ = writeln!(
            ops,
            "q\n/ExGsHf gs\n{} {} {} rg",
            fmt_f32(r),
            fmt_f32(g),
            fmt_f32(b)
        );

        let page_num = idx + 1;
        let slots = [
            (spec.top_left.as_deref(), true, 0u8),
            (spec.top_center.as_deref(), true, 1u8),
            (spec.top_right.as_deref(), true, 2u8),
            (spec.bottom_left.as_deref(), false, 0u8),
            (spec.bottom_center.as_deref(), false, 1u8),
            (spec.bottom_right.as_deref(), false, 2u8),
        ];
        for (tmpl_opt, is_top, align) in slots {
            let Some(tmpl) = tmpl_opt else {
                continue;
            };
            let text = expand_template(tmpl, page_num, total, ctx);
            if text.is_empty() {
                continue;
            }
            let est_w = text.chars().count() as f32 * font_size * 0.5;
            let x = match align {
                0 => media[0] + margin,
                1 => media[0] + ((w - est_w) * 0.5).max(margin),
                _ => (media[2] - margin - est_w).max(media[0] + margin),
            };
            let y = if is_top {
                media[3] - margin - font_size
            } else {
                media[1] + margin
            };
            let _ = writeln!(
                ops,
                "BT /ExF1 {} Tf 1 0 0 1 {} {} Tm ({}) Tj ET",
                fmt_f32(font_size),
                fmt_f32(x),
                fmt_f32(y),
                escape_pdf_string(&text)
            );
        }
        ops.push_str("Q\n");
        per_page_ops.insert(idx, ops);
    }

    inject_ocg_overlays(
        &doc,
        OverlayInjection {
            ocg_name: HEADER_FOOTER_OCG,
            oc_res_key: "ExOcHf",
            gs_res_key: "ExGsHf",
            alpha,
            layer: WatermarkLayer::Foreground,
            per_page_ops: &per_page_ops,
            extra_xobj: None,
        },
    )
}

/// Applies a watermark (text, PDF overlay, or raster image) to `pdf_bytes` inside an OCG-tagged
/// stream (plan O.18).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `pdf_bytes` or image/PDF overlay bytes are invalid.
pub fn apply_watermark(
    pdf_bytes: &[u8],
    spec: &WatermarkSpec,
    ctx: &TemplateContext,
) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let target_indices = resolve_scope(&spec.scope, pages.len())?;
    let total = pages.len();
    let alpha = spec.opacity.clamp(0.0, 1.0);
    let rad = spec.angle_deg * PI / 180.0;
    let cos = rad.cos();
    let sin = rad.sin();
    let scale = spec.scale.max(0.05);

    let mut per_page_ops: BTreeMap<usize, String> = BTreeMap::new();
    let mut xobj_payload: Option<(&str, PdfVal)> = None;

    match &spec.content {
        WatermarkContent::Text {
            text,
            font_size_pt,
            color_rgb,
        } => {
            let font_size = font_size_pt.max(6.0) * scale;
            let [r, g, b] = color_rgb.map(|c| c.clamp(0.0, 1.0));
            for idx in target_indices {
                let pid = pages[idx];
                let media = doc
                    .effective_box(pid, "MediaBox")
                    .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let cx = (media[0] + media[2]) * 0.5;
                let cy = (media[1] + media[3]) * 0.5;
                let expanded = expand_template(text, idx + 1, total, ctx);
                let est_w = expanded.chars().count() as f32 * font_size * 0.52;
                let dx = -est_w * 0.5;
                let dy = -font_size * 0.35;
                let tx = cx + dx * cos - dy * sin;
                let ty = cy + dx * sin + dy * cos;
                let ops = format!(
                    "q\n/ExGsWm gs\n{} {} {} rg\nBT\n/ExF1 {} Tf\n{} {} {} {} {} {} Tm\n({}) Tj\nET\nQ\n",
                    fmt_f32(r),
                    fmt_f32(g),
                    fmt_f32(b),
                    fmt_f32(font_size),
                    fmt_f32(cos),
                    fmt_f32(sin),
                    fmt_f32(-sin),
                    fmt_f32(cos),
                    fmt_f32(tx),
                    fmt_f32(ty),
                    escape_pdf_string(&expanded)
                );
                per_page_ops.insert(idx, ops);
            }
        }
        WatermarkContent::ImageRgb {
            width,
            height,
            samples,
        } => {
            let expected = (*width as usize) * (*height as usize) * 3;
            if samples.len() != expected {
                return Err(PdfDocError::InvalidOperation(format!(
                    "expected {expected} RGB bytes for watermark, got {}",
                    samples.len()
                )));
            }
            let mut img_dict = BTreeMap::new();
            img_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            img_dict.insert("Subtype".into(), PdfVal::Name("Image".into()));
            img_dict.insert("Width".into(), PdfVal::Int(i64::from(*width)));
            img_dict.insert("Height".into(), PdfVal::Int(i64::from(*height)));
            img_dict.insert("ColorSpace".into(), PdfVal::Name("DeviceRGB".into()));
            img_dict.insert("BitsPerComponent".into(), PdfVal::Int(8));
            let img_val = make_stream(
                img_dict,
                samples,
                StreamEncoding::Flate(Compression::default()),
            )?;
            xobj_payload = Some(("ExWmX0", img_val));

            let base_w = (*width as f32).max(1.0) * scale;
            let base_h = (*height as f32).max(1.0) * scale;
            for idx in target_indices {
                let pid = pages[idx];
                let media = doc
                    .effective_box(pid, "MediaBox")
                    .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let cx = (media[0] + media[2]) * 0.5;
                let cy = (media[1] + media[3]) * 0.5;
                let tx = cx - 0.5 * base_w * cos + 0.5 * base_h * sin;
                let ty = cy - 0.5 * base_w * sin - 0.5 * base_h * cos;
                let ops = format!(
                    "q\n/ExGsWm gs\n{} {} {} {} {} {} cm\n/ExWmX0 Do\nQ\n",
                    fmt_f32(base_w * cos),
                    fmt_f32(base_w * sin),
                    fmt_f32(-base_h * sin),
                    fmt_f32(base_h * cos),
                    fmt_f32(tx),
                    fmt_f32(ty)
                );
                per_page_ops.insert(idx, ops);
            }
        }
        WatermarkContent::PdfPage { pdf_bytes: wm_pdf } => {
            let wm_doc = ParsedPdf::parse(wm_pdf)?;
            let wm_pages = wm_doc.page_objects()?;
            let &wm_pid = wm_pages.first().ok_or_else(|| {
                PdfDocError::InvalidOperation("watermark PDF has zero pages".into())
            })?;
            let wm_mb = wm_doc
                .effective_box(wm_pid, "MediaBox")
                .unwrap_or([0.0, 0.0, 200.0, 100.0]);
            let wm_streams = wm_doc.page_decoded_content_streams(wm_pid)?;
            let mut combined = Vec::new();
            for s in wm_streams {
                combined.extend_from_slice(&s);
                combined.push(b'\n');
            }
            let mut form_dict = BTreeMap::new();
            form_dict.insert("Type".into(), PdfVal::Name("XObject".into()));
            form_dict.insert("Subtype".into(), PdfVal::Name("Form".into()));
            form_dict.insert("FormType".into(), PdfVal::Int(1));
            form_dict.insert(
                "BBox".into(),
                PdfVal::Array(
                    wm_mb
                        .into_iter()
                        .map(|v| PdfVal::Real(f64::from(v)))
                        .collect(),
                ),
            );
            form_dict.insert(
                "Resources".into(),
                wm_doc
                    .effective_resources(wm_pid)
                    .unwrap_or_else(|| PdfVal::Dict(BTreeMap::new())),
            );
            let form_val = make_stream(
                form_dict,
                &combined,
                StreamEncoding::Flate(Compression::default()),
            )?;
            xobj_payload = Some(("ExWmX0", form_val));

            let wm_w = (wm_mb[2] - wm_mb[0]).max(1.0);
            let wm_h = (wm_mb[3] - wm_mb[1]).max(1.0);
            for idx in target_indices {
                let pid = pages[idx];
                let media = doc
                    .effective_box(pid, "MediaBox")
                    .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let cx = (media[0] + media[2]) * 0.5;
                let cy = (media[1] + media[3]) * 0.5;
                let dx = -0.5 * wm_w * scale;
                let dy = -0.5 * wm_h * scale;
                let tx = cx + dx * cos - dy * sin;
                let ty = cy + dx * sin + dy * cos;
                let ops = format!(
                    "q\n/ExGsWm gs\n{} {} {} {} {} {} cm\n/ExWmX0 Do\nQ\n",
                    fmt_f32(scale * cos),
                    fmt_f32(scale * sin),
                    fmt_f32(-scale * sin),
                    fmt_f32(scale * cos),
                    fmt_f32(tx),
                    fmt_f32(ty)
                );
                per_page_ops.insert(idx, ops);
            }
        }
    }

    inject_ocg_overlays(
        &doc,
        OverlayInjection {
            ocg_name: WATERMARK_OCG,
            oc_res_key: "ExOcWm",
            gs_res_key: "ExGsWm",
            alpha,
            layer: spec.layer,
            per_page_ops: &per_page_ops,
            extra_xobj: xobj_payload,
        },
    )
}

/// Removes all content blocks tagged with `ocg_name` (e.g. [`WATERMARK_OCG`],
/// [`HEADER_FOOTER_OCG`], or [`BATES_OCG`]) so removing a watermark restores the original rendering
/// (plan O.18).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `pdf_bytes` cannot be parsed.
pub fn remove_ocg_overlays(pdf_bytes: &[u8], ocg_name: &str) -> Result<Vec<u8>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let mut overrides = BTreeMap::new();
    let mut next_id = doc.next_obj_id();

    let marker_start = format!("% ExcellentPDF-OCG-BEGIN {ocg_name}");
    let marker_end = format!("% ExcellentPDF-OCG-END {ocg_name}");

    for pid in pages {
        let streams = doc.page_decoded_content_streams(pid)?;
        let mut kept_all = String::new();
        let mut changed = false;
        for stm in streams {
            let text = String::from_utf8_lossy(&stm);
            if text.contains(&marker_start) {
                changed = true;
                let mut skipping = false;
                for line in text.lines() {
                    let t = line.trim();
                    if t == marker_start {
                        skipping = true;
                        continue;
                    }
                    if t == marker_end {
                        skipping = false;
                        continue;
                    }
                    if !skipping {
                        kept_all.push_str(line);
                        kept_all.push('\n');
                    }
                }
            } else {
                kept_all.push_str(&text);
                if !kept_all.ends_with('\n') {
                    kept_all.push('\n');
                }
            }
        }
        if changed {
            let cid = next_id;
            next_id += 1;
            overrides.insert(
                cid,
                make_stream(
                    BTreeMap::new(),
                    kept_all.as_bytes(),
                    StreamEncoding::Flate(Compression::default()),
                )?,
            );
            if let Some(mut pdict) = doc.get_dict(pid) {
                pdict.insert("Contents".into(), PdfVal::Ref(cid, 0));
                overrides.insert(pid, PdfVal::Dict(pdict));
            }
        }
    }

    Ok(rewrite_clean(
        &doc,
        &overrides,
        &BTreeMap::new(),
        &BTreeSet::new(),
    ))
}

pub(crate) fn inject_ocg_overlays(doc: &ParsedPdf, inj: OverlayInjection<'_>) -> Result<Vec<u8>> {
    let pages = doc.page_objects()?;
    let mut next_id = doc.next_obj_id();
    let mut overrides = BTreeMap::new();
    let mut extra = BTreeMap::new();

    // Shared font (/ExF1 Helvetica), ExtGState (/ExGs*), OCG, and optional XObject.
    let font_id = next_id;
    next_id += 1;
    let mut font_dict = BTreeMap::new();
    font_dict.insert("Type".into(), PdfVal::Name("Font".into()));
    font_dict.insert("Subtype".into(), PdfVal::Name("Type1".into()));
    font_dict.insert("BaseFont".into(), PdfVal::Name("Helvetica".into()));
    font_dict.insert("Encoding".into(), PdfVal::Name("WinAnsiEncoding".into()));
    extra.insert(font_id, PdfVal::Dict(font_dict));

    let gs_id = next_id;
    next_id += 1;
    let mut gs_dict = BTreeMap::new();
    gs_dict.insert("Type".into(), PdfVal::Name("ExtGState".into()));
    gs_dict.insert("CA".into(), PdfVal::Real(f64::from(inj.alpha)));
    gs_dict.insert("ca".into(), PdfVal::Real(f64::from(inj.alpha)));
    extra.insert(gs_id, PdfVal::Dict(gs_dict));

    let ocg_id = next_id;
    next_id += 1;
    let mut ocg_dict = BTreeMap::new();
    ocg_dict.insert("Type".into(), PdfVal::Name("OCG".into()));
    ocg_dict.insert("Name".into(), PdfVal::Str(inj.ocg_name.as_bytes().to_vec()));
    extra.insert(ocg_id, PdfVal::Dict(ocg_dict));

    let xobj_entry = inj.extra_xobj.map(|(name, val)| {
        let id = next_id;
        next_id += 1;
        extra.insert(id, val);
        (name.to_owned(), id)
    });

    // Register OCG inside Catalog /OCProperties.
    let mut catalog = doc
        .get_dict(doc.root_id)
        .ok_or_else(|| PdfDocError::InvalidPdf("catalog missing".into()))?;
    let mut ocp = catalog
        .get("OCProperties")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();
    let mut ocgs = ocp
        .get("OCGs")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| match v {
            PdfVal::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default();
    ocgs.push(PdfVal::Ref(ocg_id, 0));
    ocp.insert("OCGs".into(), PdfVal::Array(ocgs));

    let mut def = ocp
        .get("D")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();
    let mut on_arr = def
        .get("ON")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| match v {
            PdfVal::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default();
    on_arr.push(PdfVal::Ref(ocg_id, 0));
    def.insert("ON".into(), PdfVal::Array(on_arr));
    let mut order_arr = def
        .get("Order")
        .and_then(|v| doc.resolve_val(v))
        .and_then(|v| match v {
            PdfVal::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default();
    order_arr.push(PdfVal::Ref(ocg_id, 0));
    def.insert("Order".into(), PdfVal::Array(order_arr));
    ocp.insert("D".into(), PdfVal::Dict(def));
    catalog.insert("OCProperties".into(), PdfVal::Dict(ocp));
    overrides.insert(doc.root_id, PdfVal::Dict(catalog));

    let marker_begin = format!("% ExcellentPDF-OCG-BEGIN {}", inj.ocg_name);
    let marker_end = format!("% ExcellentPDF-OCG-END {}", inj.ocg_name);

    for (page_idx, ops) in inj.per_page_ops {
        let &pid = pages
            .get(*page_idx)
            .ok_or_else(|| PdfDocError::OutOfRange(format!("page {page_idx} out of range")))?;
        let mut pdict = doc
            .get_dict(pid)
            .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;

        let existing_streams = doc.page_decoded_content_streams(pid)?;
        let mut existing = String::new();
        for stm in existing_streams {
            existing.push_str(&String::from_utf8_lossy(&stm));
            if !existing.ends_with('\n') {
                existing.push('\n');
            }
        }
        let tagged_overlay = format!(
            "{marker_begin}\n/OC /{} BDC\n{ops}EMC\n{marker_end}\n",
            inj.oc_res_key
        );
        let combined = match inj.layer {
            WatermarkLayer::Background => format!("{tagged_overlay}q\n{existing}Q\n"),
            WatermarkLayer::Foreground => format!("q\n{existing}Q\n{tagged_overlay}"),
        };

        let cid = next_id;
        next_id += 1;
        extra.insert(
            cid,
            make_stream(
                BTreeMap::new(),
                combined.as_bytes(),
                StreamEncoding::Flate(Compression::default()),
            )?,
        );
        pdict.insert("Contents".into(), PdfVal::Ref(cid, 0));

        // Merge /Resources.
        let mut res = doc
            .effective_resources(pid)
            .and_then(|v| v.as_dict().cloned())
            .unwrap_or_default();
        let mut fonts = res
            .get("Font")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_dict().cloned())
            .unwrap_or_default();
        fonts.insert("ExF1".into(), PdfVal::Ref(font_id, 0));
        res.insert("Font".into(), PdfVal::Dict(fonts));

        let mut ext_gs = res
            .get("ExtGState")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_dict().cloned())
            .unwrap_or_default();
        ext_gs.insert(inj.gs_res_key.into(), PdfVal::Ref(gs_id, 0));
        res.insert("ExtGState".into(), PdfVal::Dict(ext_gs));

        let mut props = res
            .get("Properties")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_dict().cloned())
            .unwrap_or_default();
        props.insert(inj.oc_res_key.into(), PdfVal::Ref(ocg_id, 0));
        res.insert("Properties".into(), PdfVal::Dict(props));

        if let Some((ref xname, xid)) = xobj_entry {
            let mut xobjs = res
                .get("XObject")
                .and_then(|v| doc.resolve_val(v))
                .and_then(|v| v.as_dict().cloned())
                .unwrap_or_default();
            xobjs.insert(xname.clone(), PdfVal::Ref(xid, 0));
            res.insert("XObject".into(), PdfVal::Dict(xobjs));
        }

        pdict.insert("Resources".into(), PdfVal::Dict(res));
        overrides.insert(pid, PdfVal::Dict(pdict));
    }

    Ok(rewrite_clean(doc, &overrides, &extra, &BTreeSet::new()))
}

pub(crate) fn resolve_scope(scope: &PageApplyScope, count: usize) -> Result<Vec<usize>> {
    match scope {
        PageApplyScope::Single(idx) => {
            if *idx >= count {
                return Err(PdfDocError::OutOfRange(format!(
                    "page {idx} out of range (count {count})"
                )));
            }
            Ok(vec![*idx])
        }
        PageApplyScope::All => Ok((0..count).collect()),
        PageApplyScope::OddPrintedPages => Ok((0..count).filter(|i| i % 2 == 0).collect()),
        PageApplyScope::EvenPrintedPages => Ok((0..count).filter(|i| i % 2 == 1).collect()),
        PageApplyScope::Explicit(indices) => {
            for &idx in indices {
                if idx >= count {
                    return Err(PdfDocError::OutOfRange(format!(
                        "page {idx} out of range (count {count})"
                    )));
                }
            }
            Ok(indices.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{Background, Document, Matrix};

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn headers_footers_and_removable_watermark_round_trip() {
        let mut b = PdfBuilder::new();
        b.enable_standard_fonts();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "BT /F1 14 Tf 72 700 Td (Original Document Body) Tj ET".into(),
            crop_box: None,
        });
        let original = b.build().expect("base");
        let orig_pix = Document::from_bytes(&original, "application/pdf")
            .expect("orig")
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render");

        let ctx = TemplateContext {
            title: "Thesis".into(),
            author: "Ada".into(),
            filename: "thesis.pdf".into(),
            date: "2026-11-25".into(),
            time: "18:00".into(),
        };
        let with_hf =
            apply_headers_footers(&original, &HeaderFooterSpec::default(), &ctx).expect("apply hf");
        let hf_doc = Document::from_bytes(&with_hf, "application/pdf").expect("open hf");
        let hf_text = hf_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("stext")
            .text()
            .expect("t");
        assert!(hf_text.contains("Page 1 of 1"), "got {hf_text}");

        let with_wm = apply_watermark(
            &with_hf,
            &WatermarkSpec {
                content: WatermarkContent::Text {
                    text: "CONFIDENTIAL {date}".into(),
                    font_size_pt: 48.0,
                    color_rgb: [0.8, 0.1, 0.1],
                },
                opacity: 0.35,
                angle_deg: 45.0,
                scale: 1.0,
                layer: WatermarkLayer::Background,
                scope: PageApplyScope::All,
            },
            &ctx,
        )
        .expect("apply wm");

        let wm_doc = Document::from_bytes(&with_wm, "application/pdf").expect("open wm");
        assert!(
            wm_doc
                .layers()
                .expect("layers")
                .iter()
                .any(|l| l.name == WATERMARK_OCG)
        );
        let wm_text = wm_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("s")
            .text()
            .expect("t");
        assert!(wm_text.contains("CONFIDENTIAL 2026-11-25"), "got {wm_text}");

        let stripped_wm = remove_ocg_overlays(&with_wm, WATERMARK_OCG).expect("rm wm");
        let stripped_all = remove_ocg_overlays(&stripped_wm, HEADER_FOOTER_OCG).expect("rm hf");
        let restored_pix = Document::from_bytes(&stripped_all, "application/pdf")
            .expect("restored")
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render");
        assert_eq!(orig_pix.samples(), restored_pix.samples());
    }
}
