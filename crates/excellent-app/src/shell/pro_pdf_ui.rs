//! Phase 7 Pro PDF, CAD View, Extended Outline Actions, Calibrated Measurement, Compare,
//! Table/Figure Extraction, Screencast Recording, and Grading Suite UI
//! (`O.1`–`O.3`, `O.5`–`O.10`, `O.17`–`O.19`, `O.24`, `R.18`, `R.26`, `B.12`, `A.23`, `F.15`,
//! `F.16`, `L.4`, `G.9`–`G.13`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use excellent_core::bates::{BatesConfig, BatesPosition, apply_bates_to_pdf};
use excellent_core::cad::{
    CadBackgroundMode, CadDetectResult, CadEnhanceMode, CadEnhanceOverride, CadLineWeights,
    cad_enhance_active, cad_layers, detect_cad_pdf,
};
use excellent_core::compress::{CompressionProfile, CompressionSettings, compress_pdf};
use excellent_core::crypto::{
    EncryptionAlgorithm, MetadataEncryption, PdfPermissions, SanitizeOptions, SecurityPolicy,
    decrypt_pdf, encrypt_pdf, inspect_encryption, sanitize_pdf,
};
use excellent_core::edit::{Change, Operation};
use excellent_core::edit_content::{
    ReplacementImage, SignatureGuard, TextBoxAlign, TextRunEdit, edit_text_run, extract_image,
    list_page_images, replace_image,
};
use excellent_core::export_doc::{
    DocumentExportOptions, ExportPageRange, ImageInput, PageBreakMarkers, PdfPaperSize,
    RasterExportOptions, RasterFormat, collect_sorted_image_paths, images_to_pdf, pdf_to_docx,
    pdf_to_images, pdf_to_markdown,
};
use excellent_core::headers::{
    BATES_OCG, HEADER_FOOTER_OCG, HeaderFooterSpec, TemplateContext, WATERMARK_OCG,
    WatermarkContent, WatermarkLayer, WatermarkSpec, apply_headers_footers, apply_watermark,
    remove_ocg_overlays,
};
use excellent_core::page_ops::{
    MergeSource, OutlineSynthesis, PageApplyScope, PageCommand, PageLabelRange, PageLabelStyle,
    PageOrganizer, PageRotationDelta, SplitStrategy, auto_crop_to_content, merge_pdfs, split_pdf,
};
use excellent_core::print_prod::{
    inspect_separations, preflight_page, read_output_intents, summarize_pixmap_ink,
};
use excellent_core::redact::{
    PatternCase, RedactionMark, RedactionOptions, RedactionPattern, apply_redactions,
    mark_from_text_layout, verify_redaction,
};
use excellent_features::compare::{compare_pdfs, export_compare_html, export_compare_pdf};
use excellent_features::extract::{
    ExtractedFigure, ExtractedTable, extract_figures, extract_tables, render_figure_png,
    table_to_csv, table_to_markdown,
};
use excellent_features::grading::{
    AnswerKeyItem, CommentBank, CommentInsertKind, GradingStampKind, Rubric, RubricCriterion,
    RubricLevel, SavedComment, StudentSubmission, SubmissionBatch, TextMultilineMode,
    WorksheetFieldKind, WorksheetFieldSpec, build_worksheet_pdf, create_final_score_stamp,
    create_grading_stamp, discover_submission_pdfs, export_student_copy, grading_stamp_changes,
    recalculate_scores,
};
use excellent_features::measure::{
    LengthUnit, MeasureTool, ViewportScale, format_measurement, measure_angle_deg, measure_area,
    measure_distance, measure_perimeter, to_annotation,
};
use excellent_features::outline_extra::actions::{
    LaunchTrust, NamedAction, OutlineAction, UriMapMode, WindowOpenMode,
};
use excellent_features::screencast::{
    MockVideoEncoder, ScreencastConfig, ScreencastSession, VideoContainerFormat,
};
use excellent_mupdf::{Background, Matrix, PagePoint, PageRect};
use nsh_base::theme;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::ai_ui::{labeled_field, pill_button, status_caption};
use crate::list_dialog::{card, list_dialog};
use crate::panes::Panes;
use crate::settings::{self, Live, write_atomically};
use crate::settings_window::{segment_owned, segmented};
use crate::workspaces::Workspaces;

/// `settings.toml` key for global CAD drawing enhancement (`"auto"`, `"on"`, `"off"`).
pub(crate) const CAD_ENHANCE_MODE: &str = "cad_enhance_mode";
/// `settings.toml` key for CAD background style (`"light"` or `"blueprint"`).
pub(crate) const CAD_BACKGROUND_MODE: &str = "cad_background_mode";
/// `settings.toml` key for CAD line weight mode (`"respect"` or `"uniform"`).
pub(crate) const CAD_LINE_WEIGHTS: &str = "cad_line_weights";

/// Which Phase 7 Pro PDF dialog is currently open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProPdfDialog {
    /// No Pro PDF modal open.
    #[default]
    None,
    /// Page organizer, thumbnail drag reorder, split, merge, crop (`O.1`–`O.3`, `R.18`).
    Organizer,
    /// Edit text runs, replace images, true redaction, convert/export (`O.5`–`O.7`, `O.9`, `O.10`).
    EditAndExport,
    /// Compress, security/encryption/permissions & sanitize (`O.8`, `O.19`).
    SecurityCompress,
    /// Bates numbering & headers/footers/watermarks (`O.17`, `O.18`).
    BatesHeaders,
    /// Print production preflight & CAD view (`O.24`, `R.26`).
    PreflightCad,
    /// PDF Compare & Table/Figure Extraction (`F.15`, `F.16`).
    CompareExtract,
    /// Calibrated Vector Measurement (`A.23`).
    Measure,
    /// Screen & Pen Screencast Recorder (`L.4`).
    Screencast,
    /// Grading Stamps, Rubric, Comment Bank, Batch & Worksheet Maker (`G.9`–`G.13`).
    Grading,
    /// Extended Outline Actions editor/inspector (`B.12`).
    OutlineAction,
}

fn comment_bank_path() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-grading"))
        .join("comment_bank.json")
}

/// Parses global [`CadEnhanceMode`] from settings.
pub(crate) fn cad_mode_from_live(live: Live) -> CadEnhanceMode {
    live.read(|s| match s.text(CAD_ENHANCE_MODE).unwrap_or("auto") {
        "off" => CadEnhanceMode::Off,
        "on" => CadEnhanceMode::On,
        _ => CadEnhanceMode::Auto,
    })
}

/// Parses global [`CadBackgroundMode`] from settings.
pub(crate) fn cad_background_from_live(live: Live) -> CadBackgroundMode {
    live.read(|s| match s.text(CAD_BACKGROUND_MODE).unwrap_or("light") {
        "blueprint" => CadBackgroundMode::DarkBlueprint,
        _ => CadBackgroundMode::LightPaper,
    })
}

/// Parses global [`CadLineWeights`] from settings.
pub(crate) fn cad_weights_from_live(live: Live) -> CadLineWeights {
    live.read(|s| match s.text(CAD_LINE_WEIGHTS).unwrap_or("respect") {
        "uniform" => CadLineWeights::UniformThin,
        _ => CadLineWeights::RespectWeights,
    })
}

/// Sub-tab inside [`ProPdfDialog::EditAndExport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum EditExportTab {
    /// Edit text run (`O.5`).
    #[default]
    Text,
    /// Embedded image replace/extract (`O.6`).
    Images,
    /// True redaction mark + permanent apply (`O.7`).
    Redact,
    /// Batch Images<->PDF and Markdown/DOCX export (`O.9`, `O.10`).
    Convert,
}

/// Sub-tab inside [`ProPdfDialog::Grading`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GradingTab {
    /// Grading stamps + Rubric scoring (`G.9`, `G.10`).
    #[default]
    StampsAndRubric,
    /// Reusable comment bank (`G.11`).
    CommentBank,
    /// Batch student submissions (`G.12`).
    Batch,
    /// Fillable worksheet & answer-key generator (`G.13`).
    Worksheet,
}

/// Persistent UI state for all Phase 7 Pro PDF, CAD, Compare, Measure, Screencast, Grading & Outline Actions tools.
#[derive(Clone, Copy)]
pub(crate) struct ProPdfUiState {
    active_dialog: Signal<ProPdfDialog>,
    status: Signal<String>,
    organizer: Stored<Option<PageOrganizer>>,
    selected_page: Signal<usize>,
    org_rev: Signal<u64>,
    page_label_prefix: Signal<String>,
    split_chunk_pages: Signal<String>,
    edit_tab: Signal<EditExportTab>,
    text_find_old: Signal<String>,
    text_replace_new: Signal<String>,
    redact_pattern: Signal<String>,
    redact_marks: Stored<Vec<RedactionMark>>,
    redact_rev: Signal<u64>,
    compress_profile: Signal<CompressionProfile>,
    enc_user_pw: Signal<String>,
    enc_owner_pw: Signal<String>,
    enc_algo: Signal<EncryptionAlgorithm>,
    bates_prefix: Signal<String>,
    bates_start: Signal<String>,
    header_center: Signal<String>,
    watermark_text: Signal<String>,
    cad_override: Signal<CadEnhanceOverride>,
    cad_detect: Signal<CadDetectResult>,
    cad_layer_names: Signal<Vec<String>>,
    preflight_report: Signal<String>,
    compare_summary: Signal<String>,
    compare_other_path: Stored<Option<PathBuf>>,
    extracted_tables: Stored<Vec<ExtractedTable>>,
    extracted_figures: Stored<Vec<ExtractedFigure>>,
    extract_rev: Signal<u64>,
    measure_tool: Signal<MeasureTool>,
    measure_unit: Signal<LengthUnit>,
    measure_pts_per_unit: Signal<String>,
    measure_last: Signal<String>,
    measure_active: Signal<bool>,
    screencast_session: Stored<Option<ScreencastSession<MockVideoEncoder>>>,
    screencast_running: Signal<bool>,
    screencast_rev: Signal<u64>,
    grading_tab: Signal<GradingTab>,
    rubric: Stored<Rubric>,
    rubric_rev: Signal<u64>,
    comment_bank: Stored<CommentBank>,
    comment_draft: Signal<String>,
    comment_shortcut: Signal<String>,
    batch: Stored<Option<SubmissionBatch>>,
    batch_rev: Signal<u64>,
    worksheet_title: Signal<String>,
    worksheet_answer: Signal<String>,
    outline_uri_input: Signal<String>,
    outline_uri_preview: Signal<String>,
}

impl ProPdfUiState {
    /// Allocates reactive state for Phase 7 features.
    pub(crate) fn new() -> Self {
        let mut rubric = Rubric {
            title: "Default Rubric".to_owned(),
            ..Rubric::default()
        };
        rubric.criteria.push(RubricCriterion {
            id: "c1".to_owned(),
            title: "Accuracy & Reasoning".to_owned(),
            max_points: 10.0,
            levels: vec![
                RubricLevel {
                    label: "Excellent".to_owned(),
                    points: 10.0,
                    description: "Complete and rigorous solution".to_owned(),
                },
                RubricLevel {
                    label: "Good".to_owned(),
                    points: 7.0,
                    description: "Minor arithmetic gap".to_owned(),
                },
                RubricLevel {
                    label: "Needs Work".to_owned(),
                    points: 4.0,
                    description: "Incomplete justification".to_owned(),
                },
            ],
        });
        let bank = std::fs::read_to_string(comment_bank_path())
            .ok()
            .and_then(|json| CommentBank::import_json(&json).ok())
            .unwrap_or_default();
        Self {
            active_dialog: signal(ProPdfDialog::None),
            status: signal(String::new()),
            organizer: stored(None),
            selected_page: signal(0),
            org_rev: signal(0),
            page_label_prefix: signal("A-".to_owned()),
            split_chunk_pages: signal("5".to_owned()),
            edit_tab: signal(EditExportTab::Text),
            text_find_old: signal(String::new()),
            text_replace_new: signal(String::new()),
            redact_pattern: signal(String::new()),
            redact_marks: stored(Vec::new()),
            redact_rev: signal(0),
            compress_profile: signal(CompressionProfile::Ebook),
            enc_user_pw: signal(String::new()),
            enc_owner_pw: signal(String::new()),
            enc_algo: signal(EncryptionAlgorithm::Aes256),
            bates_prefix: signal("DOC-".to_owned()),
            bates_start: signal("1".to_owned()),
            header_center: signal("Page {page} of {pages}".to_owned()),
            watermark_text: signal("CONFIDENTIAL".to_owned()),
            cad_override: signal(CadEnhanceOverride::Auto),
            cad_detect: signal(CadDetectResult::default()),
            cad_layer_names: signal(Vec::new()),
            preflight_report: signal(String::new()),
            compare_summary: signal(String::new()),
            compare_other_path: stored(None),
            extracted_tables: stored(Vec::new()),
            extracted_figures: stored(Vec::new()),
            extract_rev: signal(0),
            measure_tool: signal(MeasureTool::Distance),
            measure_unit: signal(LengthUnit::Millimeters),
            measure_pts_per_unit: signal("72.0".to_owned()),
            measure_last: signal(String::new()),
            measure_active: signal(false),
            screencast_session: stored(None),
            screencast_running: signal(false),
            screencast_rev: signal(0),
            grading_tab: signal(GradingTab::StampsAndRubric),
            rubric: stored(rubric),
            rubric_rev: signal(0),
            comment_bank: stored(bank),
            comment_draft: signal(String::new()),
            comment_shortcut: signal("!1".to_owned()),
            batch: stored(None),
            batch_rev: signal(0),
            worksheet_title: signal("Practice Worksheet".to_owned()),
            worksheet_answer: signal("x = 3".to_owned()),
            outline_uri_input: signal("#page=1".to_owned()),
            outline_uri_preview: signal(String::new()),
        }
    }

    /// True if any Pro PDF modal is open (native canvas HWND must hide underneath).
    pub(crate) fn any_dialog_open(self) -> bool {
        self.active_dialog.get() != ProPdfDialog::None
    }

    /// True if CAD enhancement is active on the current document (`R.26`).
    pub(crate) fn is_cad_active(self, live: Live) -> bool {
        cad_enhance_active(
            self.cad_detect.get(),
            cad_mode_from_live(live),
            self.cad_override.get(),
        )
    }

    /// True if Calibrated Measurement mode is toggled on (`A.23`).
    pub(crate) fn is_measure_active(self) -> bool {
        self.measure_active.get()
    }

    /// True if Screencast Recording is running (`L.4`).
    pub(crate) fn is_screencast_active(self) -> bool {
        self.screencast_running.get()
    }

    /// Closes open Pro PDF modal.
    pub(crate) fn close(self) {
        self.active_dialog.set(ProPdfDialog::None);
    }

    /// Opens `dialog` and initializes its data from the active pane's PDF.
    pub(crate) fn open_dialog(self, dialog: ProPdfDialog, panes: Panes) {
        self.status.set(String::new());
        if let Some(path) = panes.active().tabs.shown_path() {
            match dialog {
                ProPdfDialog::Organizer => {
                    if let Ok(bytes) = std::fs::read(&path)
                        && let Ok(org) = PageOrganizer::new(bytes)
                    {
                        let count = organizer_page_count(&org);
                        self.selected_page
                            .set(panes.active().pages.current().min(count.saturating_sub(1)));
                        self.organizer.with(|slot| *slot = Some(org));
                        self.org_rev.update(|r| *r = r.wrapping_add(1));
                    }
                }
                ProPdfDialog::EditAndExport => {
                    self.edit_tab.set(EditExportTab::Text);
                }
                ProPdfDialog::PreflightCad => {
                    refresh_preflight_and_cad(self, &path, panes.active().pages.current());
                }
                ProPdfDialog::CompareExtract => {
                    refresh_extract(self, &path, panes.active().pages.current());
                }
                ProPdfDialog::Measure => {
                    self.measure_active.set(true);
                }
                _ => {}
            }
        }
        self.active_dialog.set(dialog);
    }

    /// Opens the Edit & Export modal directly on `tab`.
    pub(crate) fn open_edit_tab(self, tab: EditExportTab, panes: Panes) {
        self.status.set(String::new());
        self.edit_tab.set(tab);
        let _ = panes.active().tabs.shown_path();
        self.active_dialog.set(ProPdfDialog::EditAndExport);
    }

    /// Toggles CAD drawing enhancement override (`R.26`).
    pub(crate) fn toggle_cad_mode(self, panes: Panes, live: Live) {
        if let Some(path) = panes.active().tabs.shown_path() {
            refresh_preflight_and_cad(self, &path, panes.active().pages.current());
        }
        let next = match self.cad_override.get_untracked() {
            CadEnhanceOverride::Auto => {
                if cad_enhance_active(
                    self.cad_detect.get_untracked(),
                    cad_mode_from_live(live),
                    CadEnhanceOverride::Auto,
                ) {
                    CadEnhanceOverride::ForceOff
                } else {
                    CadEnhanceOverride::ForceOn
                }
            }
            CadEnhanceOverride::ForceOn => CadEnhanceOverride::ForceOff,
            CadEnhanceOverride::ForceOff => CadEnhanceOverride::Auto,
            _ => CadEnhanceOverride::Auto,
        };
        self.cad_override.set(next);
        self.active_dialog.set(ProPdfDialog::PreflightCad);
    }
}

fn minimal_blank_pdf_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n");
    let mut push = |id: u32, body: &str| {
        offsets.push((id, out.len()));
        out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
    };
    push(1, "<< /Type /Catalog /Pages 2 0 R >>");
    push(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    push(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>");
    let xref_off = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for (_, off) in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_off}\n%%EOF\n").as_bytes(),
    );
    out
}

fn refresh_preflight_and_cad(state: ProPdfUiState, path: &Path, page: usize) {
    let bytes = std::fs::read(path).unwrap_or_default();
    let detect = detect_cad_pdf(&bytes).unwrap_or_default();
    state.cad_detect.set(detect);
    let intents = read_output_intents(&bytes).unwrap_or_default();
    let seps = inspect_separations(&bytes, page).unwrap_or_default();
    let mut lines = Vec::new();
    lines.push(format!(
        "CAD detect: {:?} (score {})",
        detect.reason, detect.score
    ));
    lines.push(format!("OutputIntents: {}", intents.len()));
    for oi in &intents {
        lines.push(format!(
            "  - {} ({})",
            oi.output_condition_identifier, oi.subtype
        ));
    }
    lines.push(format!("Separations on p.{}: {}", page + 1, seps.len()));
    if let Ok(doc) = excellent_mupdf::Document::open(path) {
        if let Ok(layers) = cad_layers(&doc) {
            let names: Vec<String> = layers.into_iter().map(|l| l.name).collect();
            state.cad_layer_names.set(names);
        }
        if let Ok(p) = doc.load_page(page)
            && let Ok(pix) = p.render(&Matrix::scale(1.0, 1.0), Background::White)
        {
            let ink = summarize_pixmap_ink(&pix, 280.0);
            lines.push(format!(
                "Max TAC: {:.1}% | Over-limit ratio: {:.2}%",
                ink.max_tac_pct,
                ink.over_limit_ratio * 100.0
            ));
            let pf = preflight_page(&bytes, page).unwrap_or_default();
            lines.push(format!("Preflight issues: {}", pf.len()));
        }
    }
    state.preflight_report.set(lines.join("\n"));
}

fn refresh_extract(state: ProPdfUiState, path: &Path, page: usize) {
    if let Ok(doc) = excellent_mupdf::Document::open(path)
        && let Ok(p) = doc.load_page(page)
        && let Ok(stext) = p.text(None)
    {
        let layout = stext.layout();
        let tables = extract_tables(page, &layout);
        let figures = extract_figures(page, &layout, &[], &[]);
        state.extracted_tables.with(|slot| *slot = tables);
        state.extracted_figures.with(|slot| *slot = figures);
        state.extract_rev.update(|r| *r = r.wrapping_add(1));
    }
}

/// Mounts whichever Phase 7 Pro PDF dialog is active.
pub(crate) fn pro_pdf_dialogs(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
    live: Live,
) -> impl IntoElement {
    view()
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::Organizer,
            move || organizer_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::EditAndExport,
            move || edit_export_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::SecurityCompress,
            move || security_compress_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::BatesHeaders,
            move || bates_headers_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::PreflightCad,
            move || preflight_cad_dialog(state, panes, live),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::CompareExtract,
            move || compare_extract_dialog(state, panes),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::Measure,
            move || measure_dialog(state, panes),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::Screencast,
            move || screencast_dialog(state, panes),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::Grading,
            move || grading_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.active_dialog.get() == ProPdfDialog::OutlineAction,
            move || outline_action_dialog(state),
        ))
}

fn organizer_page_count(org: &PageOrganizer) -> usize {
    excellent_mupdf::Document::from_bytes(org.bytes(), "doc.pdf")
        .and_then(|d| d.page_count())
        .unwrap_or(0)
}

fn organizer_dialog(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();

    let apply_org = move |cmd: PageCommand| {
        state.organizer.with(|opt| {
            if let Some(org) = opt {
                match org.execute("page-op", cmd) {
                    Ok(()) => state.status.set(String::new()),
                    Err(err) => state.status.set(err.to_string()),
                }
            }
        });
        state.org_rev.update(|r| *r = r.wrapping_add(1));
    };

    let split_pdf_action = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let chunk: usize = state
            .split_chunk_pages
            .get_untracked()
            .trim()
            .parse()
            .unwrap_or(5)
            .max(1);
        let dialog = OpenDialog::folders().title(t!("pro.org.split_folder"));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(dirs)) = answer.await else {
                return;
            };
            let Some(out_dir) = dirs.into_iter().next() else {
                return;
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            match split_pdf(&bytes, &SplitStrategy::EveryN(chunk)) {
                Ok(parts) => {
                    for (idx, part) in parts.iter().enumerate() {
                        let dest = out_dir.join(format!("{}-{}.pdf", part.label, idx + 1));
                        let _ = write_atomically(&dest, &part.bytes);
                    }
                    state
                        .status
                        .set(t!("pro.org.split_done", count = parts.len()));
                }
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let merge_pdfs_action = move || {
        let dialog = OpenDialog::files()
            .title(t!("pro.org.merge_pick"))
            .multiple(true)
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            let save_dialog = SaveDialog::new()
                .title(t!("pro.org.merge_save"))
                .file_name("merged.pdf")
                .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
            let Ok(Some(dest)) = pick_save_path(save_dialog).await else {
                return;
            };
            let sources: Vec<MergeSource> = paths
                .iter()
                .filter_map(|p| {
                    let bytes = std::fs::read(p).ok()?;
                    let title = p.file_stem()?.to_string_lossy().into_owned();
                    Some(MergeSource {
                        title,
                        bytes,
                        page_indices: None,
                    })
                })
                .collect();
            match merge_pdfs(&sources, OutlineSynthesis::PerFile) {
                Ok(out) => {
                    if write_atomically(&dest, &out).is_ok() {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                }
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let save_organized = move || {
        let Some(bytes) = state
            .organizer
            .with(|opt| opt.as_ref().map(|o| o.bytes().to_vec()))
        else {
            return;
        };
        let dialog = SaveDialog::new()
            .title(t!("pro.org.save"))
            .file_name("organized.pdf")
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, &bytes).is_ok()
            {
                state.status.set(t!("pro.saved"));
                workspaces.open_paths(vec![dest]);
            }
        });
    };

    let body = card(
        view()
            .w(px(640.0))
            .max_h(px(550.0))
            .gap_3()
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("pro.org.move_up"), false, move || {
                        let p = state.selected_page.get_untracked();
                        if p > 0 {
                            apply_org(PageCommand::DragReorder {
                                selected: vec![p],
                                drop_before: p - 1,
                            });
                            state.selected_page.set(p - 1);
                        }
                    }))
                    .child(pill_button(t!("pro.org.move_down"), false, move || {
                        let p = state.selected_page.get_untracked();
                        let count = state
                            .organizer
                            .with(|o| o.as_ref().map_or(0, organizer_page_count));
                        if p + 1 < count {
                            apply_org(PageCommand::DragReorder {
                                selected: vec![p],
                                drop_before: p + 2,
                            });
                            state.selected_page.set(p + 1);
                        }
                    }))
                    .child(pill_button(t!("pro.org.rotate_cw"), false, move || {
                        let p = state.selected_page.get_untracked();
                        apply_org(PageCommand::Rotate {
                            pages: vec![p],
                            delta: PageRotationDelta::Clockwise90,
                        });
                    }))
                    .child(pill_button(t!("pro.org.duplicate"), false, move || {
                        let p = state.selected_page.get_untracked();
                        apply_org(PageCommand::Duplicate(vec![p]));
                    }))
                    .child(pill_button(t!("pro.org.insert_blank"), false, move || {
                        let p = state.selected_page.get_untracked();
                        apply_org(PageCommand::InsertBlank {
                            at_index: p + 1,
                            width_pt: 612.0,
                            height_pt: 792.0,
                        });
                    }))
                    .child(pill_button(t!("pro.org.delete"), false, move || {
                        let p = state.selected_page.get_untracked();
                        apply_org(PageCommand::Delete(vec![p]));
                    }))
                    .child(pill_button(t!("pro.org.auto_crop"), false, move || {
                        if let Some(path) = panes.active().tabs.shown_path()
                            && let Ok(bytes) = std::fs::read(&path)
                            && let Ok(cropped) =
                                auto_crop_to_content(&bytes, 12.0, &PageApplyScope::All)
                            && let Ok(org) = PageOrganizer::new(cropped)
                        {
                            state.organizer.with(|slot| *slot = Some(org));
                            state.org_rev.update(|r| *r = r.wrapping_add(1));
                        }
                    }))
                    .child(pill_button(t!("pro.org.uncrop"), false, move || {
                        apply_org(PageCommand::Uncrop(PageApplyScope::All));
                    }))
                    .child(pill_button(t!("command.undo"), false, move || {
                        state.organizer.with(|o| {
                            if let Some(org) = o {
                                let _ = org.undo();
                            }
                        });
                        state.org_rev.update(|r| *r = r.wrapping_add(1));
                    }))
                    .child(pill_button(t!("command.redo"), false, move || {
                        state.organizer.with(|o| {
                            if let Some(org) = o {
                                let _ = org.redo();
                            }
                        });
                        state.org_rev.update(|r| *r = r.wrapping_add(1));
                    })),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(labeled_field(
                        move || state.page_label_prefix.get(),
                        move |v| state.page_label_prefix.set(v),
                        t!("pro.org.label_prefix_placeholder"),
                        t!("pro.org.label_prefix"),
                    ))
                    .child(pill_button(t!("pro.org.apply_labels"), false, move || {
                        let prefix = state.page_label_prefix.get_untracked();
                        apply_org(PageCommand::SetPageLabels(vec![PageLabelRange {
                            start_page: 0,
                            style: PageLabelStyle::Decimal,
                            prefix,
                            start_number: 1,
                        }]));
                    }))
                    .child(pill_button(t!("pro.org.split"), false, split_pdf_action))
                    .child(pill_button(t!("pro.org.merge"), false, merge_pdfs_action)),
            )
            .child(scroll().max_h(px(210.0)).child(dynamic(move || {
                let _rev = state.org_rev.get();
                let count = state
                    .organizer
                    .with(|o| o.as_ref().map_or(0, organizer_page_count));
                let sel = state.selected_page.get();
                let mut list = view().flex_row().flex_wrap().gap_2();
                for idx in 0..count {
                    list = list.child(pill_button(
                        t!("pro.org.page_chip", page = idx + 1),
                        sel == idx,
                        move || state.selected_page.set(idx),
                    ));
                }
                list
            })))
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("action.close"), false, close))
                    .child(pill_button(t!("pro.org.save"), true, save_organized)),
            ),
    );

    list_dialog(t!("pro.org.title"), close, body)
}

fn edit_export_dialog(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();
    let tabs = segmented()
        .child(segment_owned(
            t!("pro.edit.tab_text"),
            move || state.edit_tab.get() == EditExportTab::Text,
            move || state.edit_tab.set(EditExportTab::Text),
        ))
        .child(segment_owned(
            t!("pro.edit.tab_images"),
            move || state.edit_tab.get() == EditExportTab::Images,
            move || state.edit_tab.set(EditExportTab::Images),
        ))
        .child(segment_owned(
            t!("pro.edit.tab_redact"),
            move || state.edit_tab.get() == EditExportTab::Redact,
            move || state.edit_tab.set(EditExportTab::Redact),
        ))
        .child(segment_owned(
            t!("pro.edit.tab_convert"),
            move || state.edit_tab.get() == EditExportTab::Convert,
            move || state.edit_tab.set(EditExportTab::Convert),
        ));

    let body = card(
        view()
            .w(px(620.0))
            .max_h(px(540.0))
            .gap_3()
            .child(tabs)
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(when(
                move || state.edit_tab.get() == EditExportTab::Text,
                move || edit_text_view(state, panes, workspaces),
            ))
            .child(when(
                move || state.edit_tab.get() == EditExportTab::Images,
                move || edit_images_view(state, panes, workspaces),
            ))
            .child(when(
                move || state.edit_tab.get() == EditExportTab::Redact,
                move || edit_redact_view(state, panes, workspaces),
            ))
            .child(when(
                move || state.edit_tab.get() == EditExportTab::Convert,
                move || export_convert_view(state, panes, workspaces),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.edit.title"), close, body)
}

fn edit_text_view(state: ProPdfUiState, panes: Panes, workspaces: Workspaces) -> impl IntoElement {
    let apply_text = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let old_txt = state.text_find_old.get_untracked();
        let new_txt = state.text_replace_new.get_untracked();
        let page = panes.active().pages.current();
        let bytes = std::fs::read(&path).unwrap_or_default();
        let edit = TextRunEdit {
            page,
            target_quads: Vec::new(),
            original_text: old_txt,
            replacement_text: new_txt,
            font_name: "Helvetica".to_owned(),
            font_size_pt: 11.0,
            color_rgb: [0.0, 0.0, 0.0],
            box_rect: PageRect {
                x0: 36.0,
                y0: 36.0,
                x1: 560.0,
                y1: 760.0,
            },
            align: TextBoxAlign::Left,
        };
        match edit_text_run(&bytes, &edit, SignatureGuard::AllowInvalidatingSignatures) {
            Ok(outcome) => {
                let save = SaveDialog::new()
                    .title(t!("pro.edit.save_text"))
                    .file_name("edited-text.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &outcome.bytes).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    view()
        .gap_2()
        .child(labeled_field(
            move || state.text_find_old.get(),
            move |v| state.text_find_old.set(v),
            t!("pro.edit.old_text_placeholder"),
            t!("pro.edit.old_text"),
        ))
        .child(labeled_field(
            move || state.text_replace_new.get(),
            move |v| state.text_replace_new.set(v),
            t!("pro.edit.new_text_placeholder"),
            t!("pro.edit.new_text"),
        ))
        .child(pill_button(t!("pro.edit.apply_text"), true, apply_text))
}

fn edit_images_view(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let list_and_replace = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let page = panes.active().pages.current();
        let bytes = std::fs::read(&path).unwrap_or_default();
        let imgs = list_page_images(&bytes, page).unwrap_or_default();
        state.status.set(t!("pro.images.found", count = imgs.len()));
        if let Some(first) = imgs.first().cloned() {
            let pick = OpenDialog::files()
                .title(t!("pro.images.pick_replacement"))
                .filter(FileFilter::new(t!("filter.images"), ["jpg", "jpeg"]));
            let answer = pick_paths(pick);
            spawn(async move {
                let Ok(Some(paths)) = answer.await else {
                    return;
                };
                let Some(img_path) = paths.into_iter().next() else {
                    return;
                };
                let Ok(img_bytes) = std::fs::read(&img_path) else {
                    return;
                };
                let Ok(raw) = extract_image(&bytes, first.object_id) else {
                    return;
                };
                let rep = ReplacementImage::Jpeg {
                    width: raw.width.max(1),
                    height: raw.height.max(1),
                    bytes: img_bytes,
                };
                if let Ok(out) = replace_image(&bytes, page, &first.resource_name, &rep) {
                    let save = SaveDialog::new()
                        .title(t!("pro.images.save"))
                        .file_name("replaced-image.pdf")
                        .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                    if let Ok(Some(dest)) = pick_save_path(save).await
                        && write_atomically(&dest, &out).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                }
            });
        }
    };

    view()
        .gap_2()
        .child(status_caption(move || t!("pro.images.hint")))
        .child(pill_button(
            t!("pro.images.replace_first"),
            true,
            list_and_replace,
        ))
}

fn edit_redact_view(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let mark_pattern = move |pat: Option<RedactionPattern>| {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let page = panes.active().pages.current();
        let pattern = pat.unwrap_or_else(|| RedactionPattern::Literal {
            text: state.redact_pattern.get_untracked(),
            case: PatternCase::Insensitive,
        });
        if let Ok(doc) = excellent_mupdf::Document::open(&path)
            && let Ok(p) = doc.load_page(page)
            && let Ok(stext) = p.text(None)
        {
            let marks = mark_from_text_layout(page, &stext.layout(), &pattern);
            state.redact_marks.with(|v| v.extend(marks));
            state.redact_rev.update(|r| *r = r.wrapping_add(1));
        }
    };

    let apply_perm = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let marks = state.redact_marks.with(|v| v.clone());
        if marks.is_empty() {
            return;
        }
        let marks_count = marks.len();
        let bytes = std::fs::read(&path).unwrap_or_default();
        match apply_redactions(&bytes, &marks, &RedactionOptions::default()) {
            Ok(out_bytes) => {
                let pat = RedactionPattern::Literal {
                    text: state.redact_pattern.get_untracked(),
                    case: PatternCase::Insensitive,
                };
                let verified = verify_redaction(&out_bytes, &[pat]).is_ok_and(|rep| rep.is_clean());
                let save = SaveDialog::new()
                    .title(t!("pro.redact.save"))
                    .file_name("redacted.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &out_bytes).is_ok()
                    {
                        state.status.set(t!(
                            "pro.redact.applied",
                            count = marks_count,
                            verified = verified
                        ));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    view()
        .gap_2()
        .child(labeled_field(
            move || state.redact_pattern.get(),
            move |v| state.redact_pattern.set(v),
            t!("pro.redact.pattern_placeholder"),
            t!("pro.redact.pattern_label"),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(t!("pro.redact.mark_text"), false, move || {
                    mark_pattern(None)
                }))
                .child(pill_button("SSN", false, move || {
                    mark_pattern(Some(RedactionPattern::Ssn))
                }))
                .child(pill_button("Email", false, move || {
                    mark_pattern(Some(RedactionPattern::Email))
                }))
                .child(pill_button(t!("pro.redact.apply"), true, apply_perm)),
        )
        .child(status_caption(move || {
            let _r = state.redact_rev.get();
            let count = state.redact_marks.with(|v| v.len());
            t!("pro.redact.marks_count", count = count)
        }))
}

fn export_convert_view(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let imgs_to_pdf = move || {
        let dialog = OpenDialog::files()
            .title(t!("pro.export.pick_images"))
            .multiple(true)
            .filter(FileFilter::new(
                t!("filter.images"),
                ["png", "jpg", "jpeg", "bmp", "webp"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let sorted = collect_sorted_image_paths(&paths);
            let inputs: Vec<ImageInput> = sorted
                .into_iter()
                .filter_map(|p| {
                    let bytes = std::fs::read(&p).ok()?;
                    let hint = p.file_name()?.to_string_lossy().into_owned();
                    Some(ImageInput::Encoded {
                        dpi: 150.0,
                        filename_hint: hint,
                        bytes,
                    })
                })
                .collect();
            if let Ok(pdf_bytes) = images_to_pdf(&inputs, PdfPaperSize::FitToImage, 0.0) {
                let save = SaveDialog::new()
                    .title(t!("pro.export.save_pdf"))
                    .file_name("images.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                if let Ok(Some(dest)) = pick_save_path(save).await
                    && write_atomically(&dest, &pdf_bytes).is_ok()
                {
                    state.status.set(t!("pro.saved"));
                    workspaces.open_paths(vec![dest]);
                }
            }
        });
    };

    let pdf_to_pngs = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let dialog = OpenDialog::folders().title(t!("pro.export.pick_folder"));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(dirs)) = answer.await else {
                return;
            };
            let Some(out_dir) = dirs.into_iter().next() else {
                return;
            };
            let opts = RasterExportOptions {
                format: RasterFormat::Png,
                dpi: 150.0,
                pages: ExportPageRange::All,
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            if let Ok(pages) = pdf_to_images(&bytes, &opts) {
                for img in &pages {
                    let dest = out_dir.join(format!("page-{:04}.png", img.page_index + 1));
                    let _ = write_atomically(&dest, &img.bytes);
                }
                state
                    .status
                    .set(t!("pro.export.images_done", count = pages.len()));
            }
        });
    };

    let to_md = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let save = SaveDialog::new()
            .title(t!("pro.export.markdown"))
            .file_name("exported.md")
            .filter(FileFilter::new("Markdown", ["md"]));
        let answer = pick_save_path(save);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Document".to_owned());
            if let Ok(md) = pdf_to_markdown(
                &bytes,
                &DocumentExportOptions {
                    title,
                    page_breaks: PageBreakMarkers::Include,
                },
            ) && write_atomically(&dest, md.as_bytes()).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    let to_docx = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let save = SaveDialog::new()
            .title(t!("pro.export.docx"))
            .file_name("exported.docx")
            .filter(FileFilter::new("Word Document", ["docx"]));
        let answer = pick_save_path(save);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Document".to_owned());
            if let Ok(docx) = pdf_to_docx(
                &bytes,
                &DocumentExportOptions {
                    title,
                    page_breaks: PageBreakMarkers::Include,
                },
            ) && write_atomically(&dest, &docx).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(pill_button(
            t!("pro.export.images_to_pdf"),
            false,
            imgs_to_pdf,
        ))
        .child(pill_button(
            t!("pro.export.pdf_to_images"),
            false,
            pdf_to_pngs,
        ))
        .child(pill_button(t!("pro.export.markdown"), false, to_md))
        .child(pill_button(t!("pro.export.docx"), true, to_docx))
}

fn security_compress_dialog(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();

    let run_compress = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let profile = state.compress_profile.get_untracked();
        let save = SaveDialog::new()
            .title(t!("pro.compress.save"))
            .file_name("compressed.pdf")
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(save);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            let settings = CompressionSettings::from(profile);
            match compress_pdf(&bytes, settings) {
                Ok(rep) => {
                    if write_atomically(&dest, &rep.bytes).is_ok() {
                        state.status.set(t!(
                            "pro.compress.done",
                            before = rep.stats.original_bytes,
                            after = rep.stats.compressed_bytes
                        ));
                        workspaces.open_paths(vec![dest]);
                    }
                }
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let run_encrypt = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let user_password = state.enc_user_pw.get_untracked();
        let owner_pw = state.enc_owner_pw.get_untracked();
        let algorithm = state.enc_algo.get_untracked();
        let policy = SecurityPolicy {
            algorithm,
            user_password,
            owner_password: if owner_pw.is_empty() {
                "owner".to_owned()
            } else {
                owner_pw
            },
            permissions: PdfPermissions::allow_all(),
            encrypt_metadata: MetadataEncryption::Encrypted,
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        match encrypt_pdf(&bytes, &policy) {
            Ok(out) => {
                let save = SaveDialog::new()
                    .title(t!("pro.security.save"))
                    .file_name("protected.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &out).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let run_decrypt = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let pw = state.enc_owner_pw.get_untracked();
        let user = state.enc_user_pw.get_untracked();
        let pass = if !pw.is_empty() { pw } else { user };
        let bytes = std::fs::read(&path).unwrap_or_default();
        match decrypt_pdf(&bytes, &pass) {
            Ok(out) => {
                let save = SaveDialog::new()
                    .title(t!("pro.security.save"))
                    .file_name("unlocked.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &out).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let run_sanitize = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        match sanitize_pdf(&bytes, SanitizeOptions::default()) {
            Ok(out) => {
                let save = SaveDialog::new()
                    .title(t!("pro.security.sanitize"))
                    .file_name("sanitized.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &out).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let inspect_btn = move || {
        if let Some(path) = panes.active().tabs.shown_path()
            && let Ok(bytes) = std::fs::read(&path)
        {
            match inspect_encryption(&bytes) {
                Ok(Some(desc)) => state.status.set(format!(
                    "Encrypted ({:?}, {} bits)",
                    desc.algorithm, desc.key_bits
                )),
                Ok(None) => state.status.set("Encrypted: false".to_owned()),
                Err(err) => state.status.set(err.to_string()),
            }
        }
    };

    let body = card(
        view()
            .w(px(560.0))
            .gap_3()
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(
                segmented()
                    .child(segment_owned(
                        "Screen (72 dpi)".to_owned(),
                        move || state.compress_profile.get() == CompressionProfile::Screen,
                        move || state.compress_profile.set(CompressionProfile::Screen),
                    ))
                    .child(segment_owned(
                        "Ebook (150 dpi)".to_owned(),
                        move || state.compress_profile.get() == CompressionProfile::Ebook,
                        move || state.compress_profile.set(CompressionProfile::Ebook),
                    ))
                    .child(segment_owned(
                        "Print (300 dpi)".to_owned(),
                        move || state.compress_profile.get() == CompressionProfile::Print,
                        move || state.compress_profile.set(CompressionProfile::Print),
                    )),
            )
            .child(pill_button(t!("pro.compress.run"), true, run_compress))
            .child(labeled_field(
                move || state.enc_user_pw.get(),
                move |v| state.enc_user_pw.set(v),
                t!("pro.security.user_pw_placeholder"),
                t!("pro.security.user_pw"),
            ))
            .child(labeled_field(
                move || state.enc_owner_pw.get(),
                move |v| state.enc_owner_pw.set(v),
                t!("pro.security.owner_pw_placeholder"),
                t!("pro.security.owner_pw"),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("pro.security.inspect"), false, inspect_btn))
                    .child(pill_button(t!("pro.security.encrypt"), false, run_encrypt))
                    .child(pill_button(t!("pro.security.decrypt"), false, run_decrypt))
                    .child(pill_button(
                        t!("pro.security.sanitize"),
                        false,
                        run_sanitize,
                    )),
            )
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.sec_comp.title"), close, body)
}

fn bates_headers_dialog(
    state: ProPdfUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();

    let apply_bates = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let cfg = BatesConfig {
            prefix: state.bates_prefix.get_untracked(),
            suffix: String::new(),
            digits: 6,
            start_number: state
                .bates_start
                .get_untracked()
                .trim()
                .parse()
                .unwrap_or(1),
            position: BatesPosition::BottomRight,
            font_size_pt: 10.0,
            color_rgb: [0.0, 0.0, 0.0],
            margin_pt: 24.0,
            scope: PageApplyScope::All,
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        match apply_bates_to_pdf(&bytes, &cfg) {
            Ok(out) => {
                let save = SaveDialog::new()
                    .title(t!("pro.bates.save"))
                    .file_name("bates-stamped.pdf")
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &out.bytes).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let apply_hdr_wm = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let mut bytes = std::fs::read(&path).unwrap_or_default();
        let hdr_text = state.header_center.get_untracked();
        let wm_text = state.watermark_text.get_untracked();
        let ctx = TemplateContext {
            title: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            author: "Reader".to_owned(),
            filename: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            date: "2026".to_owned(),
            time: String::new(),
        };
        if !hdr_text.trim().is_empty()
            && let Ok(next) = apply_headers_footers(
                &bytes,
                &HeaderFooterSpec {
                    top_left: None,
                    top_center: Some(hdr_text),
                    top_right: None,
                    bottom_left: None,
                    bottom_center: None,
                    bottom_right: None,
                    font_size_pt: 9.0,
                    color_rgb: [0.2, 0.2, 0.2],
                    opacity: 1.0,
                    margin_pt: 24.0,
                    scope: PageApplyScope::All,
                },
                &ctx,
            )
        {
            bytes = next;
        }
        if !wm_text.trim().is_empty()
            && let Ok(next) = apply_watermark(
                &bytes,
                &WatermarkSpec {
                    content: WatermarkContent::Text {
                        text: wm_text,
                        font_size_pt: 44.0,
                        color_rgb: [0.5, 0.5, 0.5],
                    },
                    opacity: 0.25,
                    angle_deg: 45.0,
                    scale: 1.0,
                    layer: WatermarkLayer::Foreground,
                    scope: PageApplyScope::All,
                },
                &ctx,
            )
        {
            bytes = next;
        }
        let save = SaveDialog::new()
            .title(t!("pro.headers.save"))
            .file_name("stamped-overlays.pdf")
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(save);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, &bytes).is_ok()
            {
                state.status.set(t!("pro.saved"));
                workspaces.open_paths(vec![dest]);
            }
        });
    };

    let remove_overlays = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        let mut clean = bytes;
        let mut any_ok = false;
        for layer in [HEADER_FOOTER_OCG, WATERMARK_OCG, BATES_OCG] {
            if let Ok(next) = remove_ocg_overlays(&clean, layer) {
                clean = next;
                any_ok = true;
            }
        }
        if any_ok {
            let save = SaveDialog::new()
                .title(t!("pro.headers.save"))
                .file_name("overlays-removed.pdf")
                .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
            let answer = pick_save_path(save);
            spawn(async move {
                if let Ok(Some(dest)) = answer.await
                    && write_atomically(&dest, &clean).is_ok()
                {
                    state.status.set(t!("pro.saved"));
                    workspaces.open_paths(vec![dest]);
                }
            });
        }
    };

    let body = card(
        view()
            .w(px(540.0))
            .gap_3()
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(labeled_field(
                move || state.bates_prefix.get(),
                move |v| state.bates_prefix.set(v),
                t!("pro.bates.prefix_placeholder"),
                t!("pro.bates.prefix"),
            ))
            .child(labeled_field(
                move || state.bates_start.get(),
                move |v| state.bates_start.set(v),
                t!("pro.bates.start_placeholder"),
                t!("pro.bates.start"),
            ))
            .child(labeled_field(
                move || state.header_center.get(),
                move |v| state.header_center.set(v),
                t!("pro.headers.placeholder"),
                t!("pro.headers.label"),
            ))
            .child(labeled_field(
                move || state.watermark_text.get(),
                move |v| state.watermark_text.set(v),
                t!("pro.watermark.placeholder"),
                t!("pro.watermark.label"),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("pro.bates.apply"), true, apply_bates))
                    .child(pill_button(t!("pro.headers.apply"), false, apply_hdr_wm))
                    .child(pill_button(
                        t!("pro.headers.remove"),
                        false,
                        remove_overlays,
                    )),
            )
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.bates_hdr.title"), close, body)
}

fn preflight_cad_dialog(state: ProPdfUiState, panes: Panes, live: Live) -> impl IntoElement {
    let close = move || state.close();
    let body = card(
        view()
            .w(px(560.0))
            .gap_3()
            .child(
                segmented()
                    .child(segment_owned(
                        t!("pro.cad.auto"),
                        move || state.cad_override.get() == CadEnhanceOverride::Auto,
                        move || state.cad_override.set(CadEnhanceOverride::Auto),
                    ))
                    .child(segment_owned(
                        t!("pro.cad.force_on"),
                        move || state.cad_override.get() == CadEnhanceOverride::ForceOn,
                        move || state.cad_override.set(CadEnhanceOverride::ForceOn),
                    ))
                    .child(segment_owned(
                        t!("pro.cad.force_off"),
                        move || state.cad_override.get() == CadEnhanceOverride::ForceOff,
                        move || state.cad_override.set(CadEnhanceOverride::ForceOff),
                    )),
            )
            .child(status_caption(move || {
                let active = state.is_cad_active(live);
                let bg = cad_background_from_live(live);
                let lw = cad_weights_from_live(live);
                let layer_count = state.cad_layer_names.with(Vec::len);
                format!(
                    "CAD active: {active} | BG: {bg:?} | Weights: {lw:?} | Layers: {layer_count}"
                )
            }))
            .child(
                text(move || state.preflight_report.get())
                    .p_2()
                    .rounded(px(6.0))
                    .bg(move || theme().muted)
                    .text_size(px(12.0)),
            )
            .child(
                view()
                    .flex_row()
                    .justify_between()
                    .child(pill_button(t!("pro.preflight.refresh"), false, move || {
                        if let Some(path) = panes.active().tabs.shown_path() {
                            refresh_preflight_and_cad(state, &path, panes.active().pages.current());
                        }
                    }))
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );
    list_dialog(t!("pro.preflight_cad.title"), close, body)
}

fn compare_extract_dialog(state: ProPdfUiState, panes: Panes) -> impl IntoElement {
    let close = move || state.close();

    let run_compare = move || {
        let Some(base_path) = panes.active().tabs.shown_path() else {
            return;
        };
        let dialog = OpenDialog::files()
            .title(t!("pro.compare.pick"))
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(other_path) = paths.into_iter().next() else {
                return;
            };
            state
                .compare_other_path
                .with(|slot| *slot = Some(other_path.clone()));
            let Ok(bytes_a) = std::fs::read(&base_path) else {
                return;
            };
            let Ok(bytes_b) = std::fs::read(&other_path) else {
                return;
            };
            match compare_pdfs(&bytes_a, &bytes_b, Some(96.0)) {
                Ok(report) => {
                    let changed: usize = report
                        .text_diffs
                        .iter()
                        .filter(|p| !p.is_identical())
                        .count();
                    state.compare_summary.set(t!(
                        "pro.compare.summary",
                        pages = report.left_pages.max(report.right_pages),
                        changed = changed
                    ));
                    let html = export_compare_html(&report);
                    let out = std::env::temp_dir().join("excellentpdf-compare.html");
                    let _ = write_atomically(&out, html.as_bytes());
                    if let Ok(pdf_rep) = export_compare_pdf(&report) {
                        let out_pdf = std::env::temp_dir().join("excellentpdf-compare.pdf");
                        let _ = write_atomically(&out_pdf, &pdf_rep);
                    }
                }
                Err(err) => state.compare_summary.set(err.to_string()),
            }
        });
    };

    let export_first_table = move || {
        let tables = state.extracted_tables.with(|v| v.clone());
        let Some(tbl) = tables.first() else {
            return;
        };
        let csv = table_to_csv(tbl);
        let _md = table_to_markdown(tbl);
        let dialog = SaveDialog::new()
            .title(t!("pro.extract.save_csv"))
            .file_name("table-1.csv")
            .filter(FileFilter::new(t!("filter.csv"), ["csv"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, csv.as_bytes()).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    let export_first_figure = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let figs = state.extracted_figures.with(|v| v.clone());
        let Some(fig) = figs.first().cloned() else {
            return;
        };
        let dialog = SaveDialog::new()
            .title(t!("pro.extract.save_png"))
            .file_name("figure-1.png")
            .filter(FileFilter::new("PNG", ["png"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && let Ok(doc) = excellent_mupdf::Document::open(&path)
                && let Ok(p) = doc.load_page(fig.page)
                && let Ok(list) = p.display_list(None)
                && let Ok(png) = render_figure_png(&list, fig.bounds, 150.0)
                && write_atomically(&dest, &png).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    let body = card(
        view()
            .w(px(580.0))
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("pro.compare.pick"), true, run_compare))
                    .child(pill_button(
                        t!("pro.extract.save_csv"),
                        false,
                        export_first_table,
                    ))
                    .child(pill_button(
                        t!("pro.extract.save_png"),
                        false,
                        export_first_figure,
                    )),
            )
            .child(when(
                move || !state.compare_summary.get().is_empty(),
                move || status_caption(move || state.compare_summary.get()),
            ))
            .child(status_caption(move || {
                let _r = state.extract_rev.get();
                let tcount = state.extracted_tables.with(|v| v.len());
                let fcount = state.extracted_figures.with(|v| v.len());
                t!("pro.extract.counts", tables = tcount, figures = fcount)
            }))
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.compare_extract.title"), close, body)
}

fn measure_dialog(state: ProPdfUiState, panes: Panes) -> impl IntoElement {
    let close = move || state.close();

    let sample_and_stamp = move || {
        let ppu: f64 = state
            .measure_pts_per_unit
            .get_untracked()
            .trim()
            .parse()
            .unwrap_or(72.0_f64)
            .max(0.01);
        let unit = state.measure_unit.get_untracked();
        let tool = state.measure_tool.get_untracked();
        let scale = ViewportScale {
            viewport_rect: Some(PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 612.0,
                y1: 792.0,
            }),
            points_per_unit: ppu,
            unit,
            scale_label: "Custom".to_owned(),
            precision: 2,
        };
        let pts = [
            PagePoint { x: 100.0, y: 120.0 },
            PagePoint { x: 244.0, y: 120.0 },
            PagePoint { x: 244.0, y: 220.0 },
        ];
        let val = match tool {
            MeasureTool::Distance => measure_distance(pts[0], pts[1], &scale),
            MeasureTool::Perimeter => measure_perimeter(&pts, &scale),
            MeasureTool::Area => measure_area(&pts, &scale),
            MeasureTool::Angle => measure_angle_deg(pts[0], pts[1], pts[2]),
            _ => 0.0,
        };
        let formatted = format_measurement(tool, val, &scale);
        state.measure_last.set(formatted);
        let page = panes.active().pages.current();
        if let Some(annot) = to_annotation(page, tool, &pts, &scale, "Reader") {
            panes.active().editing.apply(&Operation {
                name: t!("pro.measure.stamp_op"),
                changes: vec![Change::CreateAnnotation(annot)],
            });
        }
    };

    let body = card(
        view()
            .w(px(500.0))
            .gap_3()
            .child(
                segmented()
                    .child(segment_owned(
                        t!("pro.measure.distance"),
                        move || state.measure_tool.get() == MeasureTool::Distance,
                        move || state.measure_tool.set(MeasureTool::Distance),
                    ))
                    .child(segment_owned(
                        t!("pro.measure.perimeter"),
                        move || state.measure_tool.get() == MeasureTool::Perimeter,
                        move || state.measure_tool.set(MeasureTool::Perimeter),
                    ))
                    .child(segment_owned(
                        t!("pro.measure.area"),
                        move || state.measure_tool.get() == MeasureTool::Area,
                        move || state.measure_tool.set(MeasureTool::Area),
                    ))
                    .child(segment_owned(
                        t!("pro.measure.angle"),
                        move || state.measure_tool.get() == MeasureTool::Angle,
                        move || state.measure_tool.set(MeasureTool::Angle),
                    )),
            )
            .child(
                segmented()
                    .child(segment_owned(
                        "mm".to_owned(),
                        move || state.measure_unit.get() == LengthUnit::Millimeters,
                        move || state.measure_unit.set(LengthUnit::Millimeters),
                    ))
                    .child(segment_owned(
                        "cm".to_owned(),
                        move || state.measure_unit.get() == LengthUnit::Centimeters,
                        move || state.measure_unit.set(LengthUnit::Centimeters),
                    ))
                    .child(segment_owned(
                        "m".to_owned(),
                        move || state.measure_unit.get() == LengthUnit::Meters,
                        move || state.measure_unit.set(LengthUnit::Meters),
                    ))
                    .child(segment_owned(
                        "in".to_owned(),
                        move || state.measure_unit.get() == LengthUnit::Inches,
                        move || state.measure_unit.set(LengthUnit::Inches),
                    ))
                    .child(segment_owned(
                        "ft".to_owned(),
                        move || state.measure_unit.get() == LengthUnit::Feet,
                        move || state.measure_unit.set(LengthUnit::Feet),
                    )),
            )
            .child(labeled_field(
                move || state.measure_pts_per_unit.get(),
                move |v| state.measure_pts_per_unit.set(v),
                "72.0".to_owned(),
                t!("pro.measure.scale_label"),
            ))
            .child(when(
                move || !state.measure_last.get().is_empty(),
                move || status_caption(move || state.measure_last.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("pro.measure.stamp"), true, sample_and_stamp))
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );

    list_dialog(t!("pro.measure.title"), close, body)
}

fn screencast_dialog(state: ProPdfUiState, panes: Panes) -> impl IntoElement {
    let close = move || state.close();

    let start_rec = move || {
        let cfg = ScreencastConfig {
            width: 1280,
            height: 720,
            fps: 30,
            audio_sample_rate: 48_000,
            audio_channels: 1,
            container: VideoContainerFormat::RawBundle,
        };
        let page = panes.active().pages.current();
        if let Ok(session) =
            ScreencastSession::start("screencast-1", 0.0, page, cfg, MockVideoEncoder::default())
        {
            state.screencast_session.with(|slot| *slot = Some(session));
            state.screencast_running.set(true);
            state.status.set(t!("pro.screencast.recording"));
            state.screencast_rev.update(|r| *r = r.wrapping_add(1));
        }
    };

    let finish_rec = move || {
        let Some(mut session) = state.screencast_session.with(Option::take) else {
            return;
        };
        if let Some(path) = panes.active().tabs.shown_path()
            && let Ok(doc) = excellent_mupdf::Document::open(&path)
            && let Ok(p) = doc.load_page(panes.active().pages.current())
            && let Ok(pix) = p.render(&Matrix::scale(1.0, 1.0), Background::White)
        {
            let rgba = pix.samples().to_vec();
            let _ = session.record_frame(500.0, rgba);
        }
        state.screencast_running.set(false);
        match session.finish(1000.0) {
            Ok(out) => {
                let save = SaveDialog::new()
                    .title(t!("pro.screencast.save"))
                    .file_name("lecture-recording.wav")
                    .filter(FileFilter::new("Audio/Bundle", ["wav"]));
                let answer = pick_save_path(save);
                let byte_count = out.container_bytes.len();
                let bytes = out.container_bytes;
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await {
                        let _ = write_atomically(&dest, &bytes);
                    }
                    state
                        .status
                        .set(t!("pro.screencast.finished", bytes = byte_count));
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
        state.screencast_rev.update(|r| *r = r.wrapping_add(1));
    };

    let body = card(
        view()
            .w(px(460.0))
            .gap_3()
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(pill_button(t!("pro.screencast.start"), true, start_rec))
                    .child(pill_button(t!("pro.screencast.stop"), false, finish_rec)),
            )
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.screencast.title"), close, body)
}

fn grading_dialog(state: ProPdfUiState, panes: Panes, workspaces: Workspaces) -> impl IntoElement {
    let close = move || state.close();
    let tabs = segmented()
        .child(segment_owned(
            t!("pro.grading.tab_rubric"),
            move || state.grading_tab.get() == GradingTab::StampsAndRubric,
            move || state.grading_tab.set(GradingTab::StampsAndRubric),
        ))
        .child(segment_owned(
            t!("pro.grading.tab_comments"),
            move || state.grading_tab.get() == GradingTab::CommentBank,
            move || state.grading_tab.set(GradingTab::CommentBank),
        ))
        .child(segment_owned(
            t!("pro.grading.tab_batch"),
            move || state.grading_tab.get() == GradingTab::Batch,
            move || state.grading_tab.set(GradingTab::Batch),
        ))
        .child(segment_owned(
            t!("pro.grading.tab_worksheet"),
            move || state.grading_tab.get() == GradingTab::Worksheet,
            move || state.grading_tab.set(GradingTab::Worksheet),
        ));

    let body = card(
        view()
            .w(px(620.0))
            .max_h(px(550.0))
            .gap_3()
            .child(tabs)
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(when(
                move || state.grading_tab.get() == GradingTab::StampsAndRubric,
                move || grading_stamps_rubric_view(state, panes),
            ))
            .child(when(
                move || state.grading_tab.get() == GradingTab::CommentBank,
                move || grading_comments_view(state, panes),
            ))
            .child(when(
                move || state.grading_tab.get() == GradingTab::Batch,
                move || grading_batch_view(state, workspaces),
            ))
            .child(when(
                move || state.grading_tab.get() == GradingTab::Worksheet,
                move || grading_worksheet_view(state, workspaces),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("pro.grading.title"), close, body)
}

fn grading_stamps_rubric_view(state: ProPdfUiState, panes: Panes) -> impl IntoElement {
    let stamp = move |kind: GradingStampKind| {
        let pane = panes.active();
        let page = pane.pages.current();
        if let Ok((ann, art)) =
            create_grading_stamp(page, PagePoint { x: 480.0, y: 120.0 }, &kind, "Instructor")
        {
            let changes = grading_stamp_changes(ann, art);
            pane.editing.apply(&Operation {
                name: t!("pro.grading.stamp_op"),
                changes,
            });
            state.status.set(t!("pro.grading.stamped"));
        }
    };

    let stamp_final_score = move || {
        let mut selected = BTreeMap::new();
        selected.insert("c1".to_owned(), 0usize);
        let summary = state.rubric.with(|r| recalculate_scores(r, &selected, &[]));
        let pane = panes.active();
        let bounds = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        if let Ok((ann, art)) = create_final_score_stamp(bounds, &summary, "Instructor") {
            let changes = grading_stamp_changes(ann, art);
            pane.editing.apply(&Operation {
                name: t!("pro.grading.final_score_op"),
                changes,
            });
            state.status.set(t!("pro.grading.stamped"));
        }
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button("✓", false, move || {
                    stamp(GradingStampKind::Check)
                }))
                .child(pill_button("✗", false, move || {
                    stamp(GradingStampKind::Cross)
                }))
                .child(pill_button("?", false, move || {
                    stamp(GradingStampKind::Question)
                }))
                .child(pill_button("+1", false, move || {
                    stamp(GradingStampKind::ScoreDelta(1.0))
                }))
                .child(pill_button("-1", false, move || {
                    stamp(GradingStampKind::ScoreDelta(-1.0))
                }))
                .child(pill_button(
                    t!("pro.grading.stamp_score"),
                    true,
                    stamp_final_score,
                )),
        )
        .child(status_caption(move || {
            let _rev = state.rubric_rev.get();
            let mut selected = BTreeMap::new();
            selected.insert("c1".to_owned(), 0usize);
            let sum = state.rubric.with(|r| recalculate_scores(r, &selected, &[]));
            format!("Total: {:.1} / {:.1}", sum.total_points, sum.max_points)
        }))
}

fn grading_comments_view(state: ProPdfUiState, panes: Panes) -> impl IntoElement {
    let add_comment = move || {
        let txt = state.comment_draft.get_untracked();
        let sc = state.comment_shortcut.get_untracked();
        if txt.trim().is_empty() {
            return;
        }
        state.comment_bank.with(|bank| {
            bank.upsert(SavedComment {
                id: format!("c-{}", bank.len() + 1),
                shortcut: sc,
                title: txt.chars().take(24).collect(),
                body: txt.clone(),
                insert_kind: CommentInsertKind::StickyNote,
                usage_count: 0,
            });
            if let Ok(json) = bank.export_json() {
                let p = comment_bank_path();
                if let Some(par) = p.parent() {
                    let _ = std::fs::create_dir_all(par);
                }
                let _ = write_atomically(&p, json.as_bytes());
            }
        });
        state.comment_draft.set(String::new());
        state.rubric_rev.update(|r| *r = r.wrapping_add(1));
    };

    let insert_first = move || {
        let sc = state.comment_shortcut.get_untracked();
        let pane = panes.active();
        let page = pane.pages.current();
        let built = state.comment_bank.with(|bank| {
            bank.insert_by_shortcut(&sc, page, PagePoint { x: 100.0, y: 140.0 }, "Instructor")
        });
        if let Some(annot) = built {
            pane.editing.apply(&Operation {
                name: t!("pro.grading.comment_op"),
                changes: vec![Change::CreateAnnotation(annot)],
            });
            state.status.set(t!("pro.grading.stamped"));
        }
    };

    view()
        .gap_2()
        .child(labeled_field(
            move || state.comment_draft.get(),
            move |v| state.comment_draft.set(v),
            t!("pro.grading.comment_placeholder"),
            t!("pro.grading.comment_label"),
        ))
        .child(labeled_field(
            move || state.comment_shortcut.get(),
            move |v| state.comment_shortcut.set(v),
            "!1".to_owned(),
            t!("pro.grading.shortcut_label"),
        ))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(pill_button(
                    t!("pro.grading.add_comment"),
                    false,
                    add_comment,
                ))
                .child(pill_button(
                    t!("pro.grading.insert_comment"),
                    true,
                    insert_first,
                )),
        )
}

fn grading_batch_view(state: ProPdfUiState, workspaces: Workspaces) -> impl IntoElement {
    let pick_folder = move || {
        let dialog = OpenDialog::folders().title(t!("pro.grading.pick_batch"));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(dirs)) = answer.await else {
                return;
            };
            let Some(dir) = dirs.into_iter().next() else {
                return;
            };
            let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok().map(|x| x.path()))
                .collect();
            let pdfs = discover_submission_pdfs(&entries);
            let subs: Vec<StudentSubmission> = pdfs
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    let bytes = std::fs::read(p).ok()?;
                    let name = p.file_stem()?.to_string_lossy().into_owned();
                    Some(StudentSubmission {
                        student_id: format!("S{:03}", i + 1),
                        student_name: name,
                        source_path: p.clone(),
                        pdf_bytes: bytes,
                        rubric_levels: BTreeMap::new(),
                        annotations: Vec::new(),
                    })
                })
                .collect();
            let count = subs.len();
            let rubric = state.rubric.with(|r| r.clone());
            let batch = SubmissionBatch::new(rubric, subs);
            if let Some(cur) = batch.current() {
                workspaces.open_paths(vec![cur.source_path.clone()]);
            }
            state.batch.with(|slot| *slot = Some(batch));
            state
                .status
                .set(t!("pro.grading.batch_loaded", count = count));
            state.batch_rev.update(|r| *r = r.wrapping_add(1));
        });
    };

    let next_sub = move || {
        state.batch.with(|opt| {
            if let Some(b) = opt
                && b.next_student().is_some()
                && let Some(sub) = b.current()
            {
                workspaces.open_paths(vec![sub.source_path.clone()]);
            }
        });
        state.batch_rev.update(|r| *r = r.wrapping_add(1));
    };

    let export_csv = move || {
        let Some(export) = state
            .batch
            .with(|opt| opt.as_ref().map(SubmissionBatch::export_graded))
        else {
            return;
        };
        let save = SaveDialog::new()
            .title(t!("pro.grading.export_csv"))
            .file_name("grades.csv")
            .filter(FileFilter::new(t!("filter.csv"), ["csv"]));
        let answer = pick_save_path(save);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, export.csv.as_bytes()).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(pill_button(t!("pro.grading.pick_batch"), true, pick_folder))
        .child(pill_button(t!("pro.grading.next_student"), false, next_sub))
        .child(pill_button(t!("pro.grading.export_csv"), false, export_csv))
}

fn grading_worksheet_view(state: ProPdfUiState, workspaces: Workspaces) -> impl IntoElement {
    let build_and_save = move |student_copy: bool| {
        let _title = state.worksheet_title.get_untracked();
        let a = state.worksheet_answer.get_untracked();
        let fields = vec![WorksheetFieldSpec {
            name: "q1".to_owned(),
            page: 0,
            rect: PageRect {
                x0: 72.0,
                y0: 560.0,
                x1: 540.0,
                y1: 620.0,
            },
            kind: WorksheetFieldKind::Text {
                default_value: String::new(),
                multiline: TextMultilineMode::MultiLine,
            },
        }];
        let keys = vec![AnswerKeyItem {
            page: 0,
            position: PagePoint { x: 72.0, y: 540.0 },
            answer_text: a,
        }];
        let base_pdf = minimal_blank_pdf_bytes();
        match build_worksheet_pdf(&base_pdf, &fields, &keys) {
            Ok(teacher_pdf) => {
                let bytes = if student_copy {
                    export_student_copy(&teacher_pdf).unwrap_or(teacher_pdf)
                } else {
                    teacher_pdf
                };
                let save = SaveDialog::new()
                    .title(t!("pro.grading.save_worksheet"))
                    .file_name(if student_copy {
                        "worksheet-student.pdf"
                    } else {
                        "worksheet-key.pdf"
                    })
                    .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
                let answer = pick_save_path(save);
                spawn(async move {
                    if let Ok(Some(dest)) = answer.await
                        && write_atomically(&dest, &bytes).is_ok()
                    {
                        state.status.set(t!("pro.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                });
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    view()
        .gap_2()
        .child(labeled_field(
            move || state.worksheet_title.get(),
            move |v| state.worksheet_title.set(v),
            t!("pro.grading.ws_title_placeholder"),
            t!("pro.grading.ws_title"),
        ))
        .child(labeled_field(
            move || state.worksheet_answer.get(),
            move |v| state.worksheet_answer.set(v),
            t!("pro.grading.ws_answer_placeholder"),
            t!("pro.grading.ws_answer"),
        ))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(pill_button(
                    t!("pro.grading.export_teacher"),
                    false,
                    move || build_and_save(false),
                ))
                .child(pill_button(
                    t!("pro.grading.export_student"),
                    true,
                    move || build_and_save(true),
                )),
        )
}

fn outline_action_dialog(state: ProPdfUiState) -> impl IntoElement {
    let close = move || state.close();
    let inspect = move || {
        let raw = state.outline_uri_input.get_untracked();
        match OutlineAction::parse(raw.trim()) {
            Ok(parsed) => {
                let canonical = parsed.to_link_uri();
                let summary = match &parsed {
                    OutlineAction::InternalPage { page } => {
                        format!("GoTo p.{} ({canonical})", page + 1)
                    }
                    OutlineAction::RemoteGoto { file, page, .. } => {
                        format!(
                            "GoToR {} p.{} ({canonical})",
                            file.display(),
                            page.unwrap_or(0) + 1
                        )
                    }
                    OutlineAction::Launch { target, trust, .. } => {
                        format!("Launch {} ({trust:?})", target.display())
                    }
                    OutlineAction::Named(NamedAction::Custom(s)) => format!("Named({s})"),
                    OutlineAction::Named(n) => format!("Named({n:?})"),
                    OutlineAction::Uri { url, map_mode } => {
                        format!("URI {url} ({map_mode:?})")
                    }
                    _ => format!("{parsed:?}"),
                };
                state.outline_uri_preview.set(summary);
            }
            Err(err) => state.outline_uri_preview.set(err.to_string()),
        }
    };
    let preset = move |action: OutlineAction| {
        let uri = action.to_link_uri();
        state.outline_uri_input.set(uri);
        inspect();
    };

    let body = card(
        view()
            .w(px(520.0))
            .gap_3()
            .child(labeled_field(
                move || state.outline_uri_input.get(),
                move |v| state.outline_uri_input.set(v),
                "#page=1".to_owned(),
                t!("pro.outline_action.uri_label"),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button("NextPage", false, move || {
                        preset(OutlineAction::Named(NamedAction::NextPage))
                    }))
                    .child(pill_button("GoToR", false, move || {
                        preset(OutlineAction::RemoteGoto {
                            file: PathBuf::from("companion.pdf"),
                            page: Some(0),
                            named_dest: None,
                            window: WindowOpenMode::NewWindow,
                        })
                    }))
                    .child(pill_button("Launch", false, move || {
                        preset(OutlineAction::Launch {
                            target: PathBuf::from("notes.txt"),
                            parameters: None,
                            window: WindowOpenMode::NewWindow,
                            trust: LaunchTrust::RequireConfirmation,
                        })
                    }))
                    .child(pill_button("URI", false, move || {
                        preset(OutlineAction::Uri {
                            url: "https://example.org".to_owned(),
                            map_mode: UriMapMode::Plain,
                        })
                    })),
            )
            .child(when(
                move || !state.outline_uri_preview.get().is_empty(),
                move || status_caption(move || state.outline_uri_preview.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("pro.outline_action.inspect"), true, inspect))
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );
    list_dialog(t!("pro.outline_action.title"), close, body)
}
