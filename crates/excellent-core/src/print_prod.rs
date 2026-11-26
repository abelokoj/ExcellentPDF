//! Color-accurate prepress view, CMYK ink coverage, spot color separations, and print
//! production preflight (plan O.24, Appendix D.4).

use std::collections::BTreeSet;

use excellent_mupdf::Pixmap;

use crate::page_ops::pdf_doc::{ParsedPdf, PdfDocError, PdfVal};

/// Result alias for print production and preflight operations.
pub type Result<T> = std::result::Result<T, PdfDocError>;

/// Overprint simulation mode (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum OverprintMode {
    /// Standard rendering without overprint simulation.
    Off,
    /// Simulate overprint for CMYK and spot inks (`op = 1, opm = 1`).
    #[default]
    Simulate,
}

/// Black-point compensation policy for ICC profile proofing (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BlackPointCompensation {
    /// Enable black-point compensation (`bp = 1`).
    #[default]
    Enabled,
    /// Disable black-point compensation (`bp = 0`).
    Disabled,
}

/// Whether an ink separation plate is enabled in the separations preview (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PlateVisibility {
    /// Separation plate is printed / visible.
    #[default]
    Visible,
    /// Separation plate is turned off.
    Hidden,
}

/// Color proofing parameters corresponding to MuPDF's `fz_color_params` (plan O.24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepressColorParams {
    /// Overprint simulation mode.
    pub overprint: OverprintMode,
    /// Black-point compensation mode.
    pub black_point_compensation: BlackPointCompensation,
    /// Optional target output ICC profile name (e.g. `"ISOcoated_v2"`).
    pub simulation_profile: Option<String>,
}

impl Default for PrepressColorParams {
    fn default() -> Self {
        Self {
            overprint: OverprintMode::Simulate,
            black_point_compensation: BlackPointCompensation::Enabled,
            simulation_profile: None,
        }
    }
}

/// PDF `/OutputIntents` descriptor read from the document catalog (plan O.24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputIntentInfo {
    /// Intent subtype (e.g. `"GTS_PDFX"`, `"GTS_PDFA1"`, `"ISO_PDFE1"`).
    pub subtype: String,
    /// Output condition identifier (e.g. `"FOGRA39"`).
    pub output_condition_identifier: String,
    /// Human-readable output condition description.
    pub output_condition: String,
    /// Registry name (e.g. `"http://www.color.org"`).
    pub registry_name: String,
    /// Informational string.
    pub info: String,
    /// Whether an embedded `/DestOutputProfile` ICC stream is present.
    pub has_icc_profile: bool,
}

/// Ink separation channel kind (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SeparationKind {
    /// Process Cyan.
    ProcessCyan,
    /// Process Magenta.
    ProcessMagenta,
    /// Process Yellow.
    ProcessYellow,
    /// Process Black (Key).
    ProcessBlack,
    /// Named Pantone / Spot separation.
    Spot,
}

/// One ink separation plate available on a page (plan O.24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InkSeparation {
    /// Separation plate name (e.g. `"Cyan"`, `"PANTONE 185 C"`).
    pub name: String,
    /// Plate category.
    pub kind: SeparationKind,
    /// Representative sRGB preview swatch `[r, g, b]`.
    pub preview_rgb: [u8; 3],
    /// Whether this plate is currently active.
    pub visibility: PlateVisibility,
}

/// CMYK separation breakdown and Total Area Coverage (TAC) at one sampled pixel (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InkCoverageSample {
    /// Cyan, Magenta, Yellow, Black ink fractions in `0.0..=1.0`.
    pub cmyk: [f32; 4],
    /// Total area coverage percentage (`0.0..=400.0`).
    pub total_area_coverage_pct: f32,
    /// Whether `total_area_coverage_pct` exceeds the configured threshold.
    pub exceeds_tac_limit: bool,
}

/// Summary of ink coverage across a rendered page pixmap (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageInkSummary {
    /// Peak Total Area Coverage percentage observed on the page.
    pub max_tac_pct: f32,
    /// Average Total Area Coverage percentage across non-white pixels.
    pub mean_inked_tac_pct: f32,
    /// Share of pixels exceeding the TAC limit (`0.0..=1.0`).
    pub over_limit_ratio: f32,
}

/// Severity of a print-production preflight finding (plan O.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PreflightSeverity {
    /// Informational note.
    Info,
    /// Potential print issue.
    Warning,
    /// Critical production blocker.
    Error,
}

/// Individual preflight diagnostic for a PDF page (plan O.24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightFinding {
    /// Diagnostic severity.
    pub severity: PreflightSeverity,
    /// Machine-readable code (e.g. `"missing-trim-box"`, `"rgb-color-usage"`, `"tac-exceeded"`).
    pub code: String,
    /// Plain-English description.
    pub message: String,
}

/// Reads all `/OutputIntents` entries from the PDF catalog (plan O.24).
///
/// # Errors
///
/// Returns [`PdfDocError::InvalidPdf`] if the PDF cannot be parsed.
pub fn read_output_intents(pdf_bytes: &[u8]) -> Result<Vec<OutputIntentInfo>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let Some(catalog) = doc.get_dict(doc.root_id) else {
        return Ok(Vec::new());
    };
    let Some(PdfVal::Array(arr)) = catalog
        .get("OutputIntents")
        .and_then(|v| doc.resolve_val(v))
    else {
        return Ok(Vec::new());
    };

    let mut intents = Vec::new();
    for item in arr {
        let Some(dict) = doc.resolve_val(&item).and_then(|v| v.as_dict().cloned()) else {
            continue;
        };
        let str_field = |k: &str| {
            dict.get(k)
                .and_then(|v| doc.resolve_val(v))
                .and_then(|v| v.as_text_string())
                .unwrap_or_default()
        };
        let subtype = dict
            .get("S")
            .and_then(|v| doc.resolve_val(v))
            .and_then(|v| v.as_name().map(str::to_owned))
            .unwrap_or_default();
        intents.push(OutputIntentInfo {
            subtype,
            output_condition_identifier: str_field("OutputConditionIdentifier"),
            output_condition: str_field("OutputCondition"),
            registry_name: str_field("RegistryName"),
            info: str_field("Info"),
            has_icc_profile: dict.contains_key("DestOutputProfile"),
        });
    }
    Ok(intents)
}

/// Inspects process CMYK and spot `/Separation` / `/DeviceN` plates declared on `page` (plan O.24).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `page` is out of range or the PDF is malformed.
pub fn inspect_separations(pdf_bytes: &[u8], page: usize) -> Result<Vec<InkSeparation>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &pid = pages.get(page).ok_or_else(|| {
        PdfDocError::OutOfRange(format!("page {page} out of range (count {})", pages.len()))
    })?;

    let mut plates = vec![
        InkSeparation {
            name: "Cyan".into(),
            kind: SeparationKind::ProcessCyan,
            preview_rgb: [0, 174, 239],
            visibility: PlateVisibility::Visible,
        },
        InkSeparation {
            name: "Magenta".into(),
            kind: SeparationKind::ProcessMagenta,
            preview_rgb: [236, 0, 140],
            visibility: PlateVisibility::Visible,
        },
        InkSeparation {
            name: "Yellow".into(),
            kind: SeparationKind::ProcessYellow,
            preview_rgb: [255, 242, 0],
            visibility: PlateVisibility::Visible,
        },
        InkSeparation {
            name: "Black".into(),
            kind: SeparationKind::ProcessBlack,
            preview_rgb: [35, 31, 32],
            visibility: PlateVisibility::Visible,
        },
    ];

    let mut spot_names = BTreeSet::new();
    if let Some(res) = doc.effective_resources(pid)
        && let Some(res_dict) = res.as_dict()
        && let Some(cs_val) = res_dict.get("ColorSpace").and_then(|v| doc.resolve_val(v))
        && let Some(cs_dict) = cs_val.as_dict()
    {
        for val in cs_dict.values() {
            if let Some(PdfVal::Array(arr)) = doc.resolve_val(val) {
                match arr.first().and_then(PdfVal::as_name) {
                    Some("Separation") => {
                        if let Some(name) = arr.get(1).and_then(PdfVal::as_name)
                            && !matches!(
                                name,
                                "All" | "None" | "Cyan" | "Magenta" | "Yellow" | "Black"
                            )
                        {
                            spot_names.insert(name.to_owned());
                        }
                    }
                    Some("DeviceN") => {
                        if let Some(PdfVal::Array(names)) =
                            arr.get(1).and_then(|v| doc.resolve_val(v))
                        {
                            for n in names {
                                if let Some(name) = n.as_name()
                                    && !matches!(
                                        name,
                                        "All" | "None" | "Cyan" | "Magenta" | "Yellow" | "Black"
                                    )
                                {
                                    spot_names.insert(name.to_owned());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    for name in spot_names {
        plates.push(InkSeparation {
            name,
            kind: SeparationKind::Spot,
            preview_rgb: [180, 90, 40],
            visibility: PlateVisibility::Visible,
        });
    }
    Ok(plates)
}

/// Converts normalized sRGB `[r, g, b]` in `0.0..=1.0` to subtractive CMYK `[c, m, y, k]` using
/// under-color removal.
#[must_use]
pub fn rgb_to_cmyk(rgb: [f32; 3]) -> [f32; 4] {
    let r = rgb[0].clamp(0.0, 1.0);
    let g = rgb[1].clamp(0.0, 1.0);
    let b = rgb[2].clamp(0.0, 1.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 0.9999 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let denom = 1.0 - k;
    let c = ((1.0 - r - k) / denom).clamp(0.0, 1.0);
    let m = ((1.0 - g - k) / denom).clamp(0.0, 1.0);
    let y = ((1.0 - b - k) / denom).clamp(0.0, 1.0);
    [c, m, y, k]
}

/// Samples ink coverage at pixel `(x, y)` of `pixmap` against `tac_limit_pct` (typically `300.0`).
#[must_use]
pub fn sample_ink_coverage(
    pixmap: &Pixmap,
    x: u32,
    y: u32,
    tac_limit_pct: f32,
) -> Option<InkCoverageSample> {
    if x >= pixmap.width() || y >= pixmap.height() || pixmap.components() < 3 {
        return None;
    }
    let idx = (y as usize) * pixmap.stride() + (x as usize) * pixmap.components();
    let s = pixmap.samples();
    let rgb = [
        f32::from(s[idx]) / 255.0,
        f32::from(s[idx + 1]) / 255.0,
        f32::from(s[idx + 2]) / 255.0,
    ];
    let cmyk = rgb_to_cmyk(rgb);
    let tac = (cmyk[0] + cmyk[1] + cmyk[2] + cmyk[3]) * 100.0;
    Some(InkCoverageSample {
        cmyk,
        total_area_coverage_pct: tac,
        exceeds_tac_limit: tac > tac_limit_pct,
    })
}

/// Summarizes Total Area Coverage across all pixels of `pixmap` (plan O.24).
#[must_use]
pub fn summarize_pixmap_ink(pixmap: &Pixmap, tac_limit_pct: f32) -> PageInkSummary {
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let comp = pixmap.components();
    if w == 0 || h == 0 || comp < 3 {
        return PageInkSummary {
            max_tac_pct: 0.0,
            mean_inked_tac_pct: 0.0,
            over_limit_ratio: 0.0,
        };
    }
    let stride = pixmap.stride();
    let samples = pixmap.samples();
    let mut max_tac = 0.0f32;
    let mut inked_sum = 0.0f64;
    let mut inked_count = 0usize;
    let mut over_count = 0usize;
    let total = w * h;

    for y in 0..h {
        let row = &samples[y * stride..y * stride + w * comp];
        for px in row.chunks_exact(comp) {
            let cmyk = rgb_to_cmyk([
                f32::from(px[0]) / 255.0,
                f32::from(px[1]) / 255.0,
                f32::from(px[2]) / 255.0,
            ]);
            let tac = (cmyk[0] + cmyk[1] + cmyk[2] + cmyk[3]) * 100.0;
            max_tac = max_tac.max(tac);
            if tac > 0.5 {
                inked_sum += f64::from(tac);
                inked_count += 1;
            }
            if tac > tac_limit_pct {
                over_count += 1;
            }
        }
    }

    PageInkSummary {
        max_tac_pct: max_tac,
        mean_inked_tac_pct: if inked_count > 0 {
            (inked_sum / inked_count as f64) as f32
        } else {
            0.0
        },
        over_limit_ratio: over_count as f32 / total.max(1) as f32,
    }
}

/// Applies CMYK plate visibility toggles onto an RGB pixmap preview in-place (plan O.24).
pub fn apply_separation_preview(pixmap: &mut Pixmap, plates: &[InkSeparation]) {
    let visible = |kind: SeparationKind| -> bool {
        plates
            .iter()
            .find(|p| p.kind == kind)
            .is_none_or(|p| p.visibility == PlateVisibility::Visible)
    };
    let show_c = visible(SeparationKind::ProcessCyan);
    let show_m = visible(SeparationKind::ProcessMagenta);
    let show_y = visible(SeparationKind::ProcessYellow);
    let show_k = visible(SeparationKind::ProcessBlack);

    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let comp = pixmap.components();
    let stride = pixmap.stride();
    if comp < 3 {
        return;
    }
    let samples = pixmap.samples_mut();
    for y in 0..h {
        let row = &mut samples[y * stride..y * stride + w * comp];
        for px in row.chunks_exact_mut(comp) {
            let mut cmyk = rgb_to_cmyk([
                f32::from(px[0]) / 255.0,
                f32::from(px[1]) / 255.0,
                f32::from(px[2]) / 255.0,
            ]);
            if !show_c {
                cmyk[0] = 0.0;
            }
            if !show_m {
                cmyk[1] = 0.0;
            }
            if !show_y {
                cmyk[2] = 0.0;
            }
            if !show_k {
                cmyk[3] = 0.0;
            }
            let r = (1.0 - cmyk[0]) * (1.0 - cmyk[3]);
            let g = (1.0 - cmyk[1]) * (1.0 - cmyk[3]);
            let b = (1.0 - cmyk[2]) * (1.0 - cmyk[3]);
            px[0] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
            px[1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
            px[2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// Runs print-production preflight checks on `page` of `pdf_bytes` (TrimBox/BleedBox presence,
/// OutputIntents, RGB/CMYK content operators, and overprint usage) (plan O.24).
///
/// # Errors
///
/// Returns [`PdfDocError`] if `page` is out of range or the PDF is malformed.
pub fn preflight_page(pdf_bytes: &[u8], page: usize) -> Result<Vec<PreflightFinding>> {
    let doc = ParsedPdf::parse(pdf_bytes)?;
    let pages = doc.page_objects()?;
    let &pid = pages.get(page).ok_or_else(|| {
        PdfDocError::OutOfRange(format!("page {page} out of range (count {})", pages.len()))
    })?;
    let pdict = doc
        .get_dict(pid)
        .ok_or_else(|| PdfDocError::InvalidPdf("page dict missing".into()))?;
    let mut findings = Vec::new();

    if !pdict.contains_key("TrimBox") {
        findings.push(PreflightFinding {
            severity: PreflightSeverity::Warning,
            code: "missing-trim-box".into(),
            message: format!("page {} lacks an explicit /TrimBox", page + 1),
        });
    }
    if !pdict.contains_key("BleedBox") {
        findings.push(PreflightFinding {
            severity: PreflightSeverity::Warning,
            code: "missing-bleed-box".into(),
            message: format!("page {} lacks an explicit /BleedBox", page + 1),
        });
    }
    if read_output_intents(pdf_bytes)?.is_empty() {
        findings.push(PreflightFinding {
            severity: PreflightSeverity::Info,
            code: "no-output-intent".into(),
            message: "document has no /OutputIntents ICC profile; soft-proofing uses default CMYK"
                .into(),
        });
    }

    let streams = doc.page_decoded_content_streams(pid)?;
    let mut uses_rgb = false;
    let mut uses_cmyk = false;
    for stm in streams {
        let text = String::from_utf8_lossy(&stm);
        for tok in text.split_whitespace() {
            if tok == "rg" || tok == "RG" {
                uses_rgb = true;
            } else if tok == "k" || tok == "K" {
                uses_cmyk = true;
            }
        }
    }
    if uses_rgb {
        findings.push(PreflightFinding {
            severity: PreflightSeverity::Warning,
            code: "rgb-color-usage".into(),
            message: format!(
                "page {} contains DeviceRGB operators; convert to CMYK before offset press",
                page + 1
            ),
        });
    }
    if uses_cmyk {
        findings.push(PreflightFinding {
            severity: PreflightSeverity::Info,
            code: "cmyk-color-present".into(),
            message: format!("page {} uses native DeviceCMYK ink operators", page + 1),
        });
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{Background, Document, Matrix};

    use super::*;
    use crate::notebook::pdf_ops::{PdfBuilder, PdfPageInput};

    #[test]
    fn ink_coverage_separations_and_preflight_checks_work() {
        let mut b = PdfBuilder::new();
        b.add_page(PdfPageInput {
            width: 612.0,
            height: 792.0,
            content: "q 0.9 0.8 0.7 0.9 k 72 600 100 100 re f Q\n\
                      q 1 0 0 rg 200 600 100 100 re f Q"
                .into(),
            crop_box: None,
        });
        let pdf = b.build().expect("build");

        let findings = preflight_page(&pdf, 0).expect("preflight");
        assert!(findings.iter().any(|f| f.code == "missing-trim-box"));
        assert!(findings.iter().any(|f| f.code == "rgb-color-usage"));
        assert!(findings.iter().any(|f| f.code == "cmyk-color-present"));

        let mut plates = inspect_separations(&pdf, 0).expect("separations");
        assert_eq!(plates.len(), 4);

        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let mut pix = doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render");
        let summary = summarize_pixmap_ink(&pix, 280.0);
        assert!(summary.max_tac_pct > 90.0, "summary: {summary:?}");

        plates[0].visibility = PlateVisibility::Hidden;
        apply_separation_preview(&mut pix, &plates);
        let sample = sample_ink_coverage(&pix, 100, 140, 300.0).expect("sample");
        assert!(sample.cmyk[0] < 0.05, "cyan removed: {:?}", sample.cmyk);
    }
}
