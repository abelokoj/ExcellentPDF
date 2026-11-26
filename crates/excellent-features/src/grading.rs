//! Teacher grading suite: grading stamps with embedded point values (G.9), rubric panel and
//! total score recalculation (G.10), reusable comment bank with JSON import/export (G.11),
//! batch student submission grading & CSV export (G.12), and fillable worksheet maker with
//! hidden AnswerKey OCG layer on student export (G.13).

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::PathBuf;

use excellent_core::annot::{Annotation, Boxed, Shape, Style};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect, StampArt, html_to_pdf};
use serde_json::{Value, json};

/// Error returned by grading, rubric, comment bank, batch, or worksheet operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum GradingError {
    /// JSON import/export failed.
    Json(String),
    /// Invalid rubric, field, or submission state.
    InvalidInput(String),
    /// PDF or HTML rendering failed.
    Document(String),
}

impl fmt::Display for GradingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(msg) => write!(f, "grading JSON error: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid grading input: {msg}"),
            Self::Document(msg) => write!(f, "grading document error: {msg}"),
        }
    }
}

impl std::error::Error for GradingError {}

/// Convenience result alias for grading operations.
pub type Result<T> = std::result::Result<T, GradingError>;

// ---------------------------------------------------------------------------
// G.9: Grading Stamps
// ---------------------------------------------------------------------------

const GRADING_POINTS_TAG_PREFIX: &str = "grading_pts:";

/// Quick grading stamp variant (plan G.9).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GradingStampKind {
    /// Correct checkmark (`✓`).
    Check,
    /// Incorrect mark (`✗`).
    Cross,
    /// Needs review / unclear (`?`).
    Question,
    /// Numeric point adjustment (`+1`, `-0.5`, `+5`, etc.).
    ScoreDelta(f64),
    /// Custom symbol + point value.
    Custom {
        /// Stamp label shown on the page.
        label: String,
        /// Points contributed to page/document totals.
        points: f64,
    },
}

impl GradingStampKind {
    /// Display glyph/label drawn onto the stamp appearance.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Check => "✓".into(),
            Self::Cross => "✗".into(),
            Self::Question => "?".into(),
            Self::ScoreDelta(pts) => {
                if *pts >= 0.0 {
                    format!("+{pts}")
                } else {
                    format!("{pts}")
                }
            }
            Self::Custom { label, .. } => label.clone(),
        }
    }

    /// Numeric point value carried by this stamp (`0.0` for non-numeric marks).
    #[must_use]
    pub fn points(&self) -> f64 {
        match self {
            Self::Check | Self::Cross | Self::Question => 0.0,
            Self::ScoreDelta(pts) | Self::Custom { points: pts, .. } => *pts,
        }
    }

    /// Stamp accent color RGB in `0.0..=1.0`.
    #[must_use]
    pub fn color_rgb(&self) -> [f32; 3] {
        match self {
            Self::Check => [0.10, 0.56, 0.22],
            Self::Cross => [0.78, 0.14, 0.14],
            Self::Question => [0.78, 0.45, 0.06],
            Self::ScoreDelta(p) if *p >= 0.0 => [0.10, 0.42, 0.72],
            Self::ScoreDelta(_) => [0.78, 0.14, 0.14],
            Self::Custom { points, .. } if *points >= 0.0 => [0.10, 0.42, 0.72],
            Self::Custom { .. } => [0.78, 0.14, 0.14],
        }
    }
}

/// Creates a standard PDF Stamp [`Annotation`] plus rendered [`StampArt`] so other PDF readers
/// show the grading mark while ExcellentPDF preserves its point value in `/ExcellentPDF` tags
/// (plan G.9).
///
/// # Errors
///
/// Returns [`GradingError::Document`] if MuPDF fails to render the stamp appearance PDF.
pub fn create_grading_stamp(
    page: usize,
    at: PagePoint,
    kind: &GradingStampKind,
    author: &str,
) -> Result<(Annotation, StampArt)> {
    let label = kind.label();
    let color = kind.color_rgb();
    let [r, g, b] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let html = format!(
        "<div style=\"border:2pt solid #{r:02x}{g:02x}{b:02x};border-radius:6pt;padding:2pt 6pt;\
         color:#{r:02x}{g:02x}{b:02x};font-family:sans-serif;font-weight:bold;font-size:15pt;\
         text-align:center;\">{label}</div>"
    );
    let pdf = html_to_pdf(&html, "body{margin:0;}", 72.0)
        .map_err(|e| GradingError::Document(e.to_string()))?;
    let rect = PageRect {
        x0: (at.x - 24.0).max(0.0),
        y0: (at.y - 14.0).max(0.0),
        x1: (at.x - 24.0).max(0.0) + 48.0,
        y1: (at.y - 14.0).max(0.0) + 28.0,
    };
    let mut ann = Annotation::new(
        page,
        Shape::Boxed(Boxed::Stamp, rect),
        Style {
            color: Some(color),
            opacity: 1.0,
            width: 0.0,
        },
        author,
    );
    ann.icon = format!("Grading{}", ann.id);
    ann.contents = label;
    ann.tags
        .push(format!("{GRADING_POINTS_TAG_PREFIX}{}", kind.points()));
    Ok((ann, StampArt::Page(pdf)))
}

/// Extracts the numeric point value stored on a grading stamp [`Annotation`], if any (plan G.9).
#[must_use]
pub fn stamp_points(annotation: &Annotation) -> Option<f64> {
    annotation.tags.iter().find_map(|tag| {
        tag.strip_prefix(GRADING_POINTS_TAG_PREFIX)
            .and_then(|num| num.parse::<f64>().ok())
    })
}

/// Wraps a grading stamp and its appearance into [`Change`] steps for one undoable operation.
#[must_use]
pub fn grading_stamp_changes(annotation: Annotation, art: StampArt) -> Vec<Change> {
    let page = annotation.page;
    let id = annotation.id.clone();
    vec![
        Change::CreateAnnotation(annotation),
        Change::StampArt { page, id, art },
    ]
}

// ---------------------------------------------------------------------------
// G.10: Rubric Panel & Scores
// ---------------------------------------------------------------------------

/// One achievement level inside a [`RubricCriterion`] (plan G.10).
#[derive(Debug, Clone, PartialEq)]
pub struct RubricLevel {
    /// Level label (e.g. `"Exemplary"`, `"Proficient"`, `"Developing"`).
    pub label: String,
    /// Points awarded at this level.
    pub points: f64,
    /// Optional descriptor shown to the grader.
    pub description: String,
}

/// One graded criterion inside a [`Rubric`] (plan G.10).
#[derive(Debug, Clone, PartialEq)]
pub struct RubricCriterion {
    /// Criterion identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Maximum possible points for this criterion.
    pub max_points: f64,
    /// Ordered selectable levels.
    pub levels: Vec<RubricLevel>,
}

/// Complete grading rubric (plan G.10).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rubric {
    /// Rubric title.
    pub title: String,
    /// Grading criteria.
    pub criteria: Vec<RubricCriterion>,
}

impl Rubric {
    /// Total maximum rubric points.
    #[must_use]
    pub fn max_points(&self) -> f64 {
        self.criteria.iter().map(|c| c.max_points).sum()
    }
}

/// Recalculated score summary across rubric criteria and page grading stamps (plan G.10).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreSummary {
    /// Points summed from selected rubric levels.
    pub rubric_points: f64,
    /// Points summed from grading stamps per zero-based page.
    pub per_page_stamp_points: BTreeMap<usize, f64>,
    /// Total points across rubric and stamps.
    pub total_points: f64,
    /// Maximum possible points from the rubric.
    pub max_points: f64,
}

/// Computes per-page stamp sums and overall total score whenever a stamp or rubric selection
/// changes (plan G.10).
#[must_use]
pub fn recalculate_scores(
    rubric: &Rubric,
    selected_levels: &BTreeMap<String, usize>,
    annotations: &[Annotation],
) -> ScoreSummary {
    let mut rubric_points = 0.0f64;
    for criterion in &rubric.criteria {
        if let Some(&level_idx) = selected_levels.get(&criterion.id)
            && let Some(level) = criterion.levels.get(level_idx)
        {
            rubric_points += level.points;
        }
    }

    let mut per_page: BTreeMap<usize, f64> = BTreeMap::new();
    let mut stamp_sum = 0.0f64;
    for ann in annotations {
        if let Some(pts) = stamp_points(ann) {
            *per_page.entry(ann.page).or_insert(0.0) += pts;
            stamp_sum += pts;
        }
    }

    ScoreSummary {
        rubric_points,
        per_page_stamp_points: per_page,
        total_points: rubric_points + stamp_sum,
        max_points: rubric.max_points(),
    }
}

/// Creates a final score summary stamp positioned at the top-right of page 1 (index 0) (plan G.10).
///
/// # Errors
///
/// Returns [`GradingError::Document`] if MuPDF cannot render the stamp PDF.
pub fn create_final_score_stamp(
    page0_bounds: PageRect,
    score: &ScoreSummary,
    author: &str,
) -> Result<(Annotation, StampArt)> {
    let label = format!("{:.1} / {:.1}", score.total_points, score.max_points);
    let html = format!(
        "<div style=\"border:2.5pt solid #166534;border-radius:8pt;padding:4pt 10pt;\
         background:#f0fdf4;color:#166534;font-family:sans-serif;font-weight:bold;font-size:14pt;\
         text-align:center;\">SCORE: {label}</div>"
    );
    let pdf = html_to_pdf(&html, "body{margin:0;}", 150.0)
        .map_err(|e| GradingError::Document(e.to_string()))?;
    let rect = PageRect {
        x0: (page0_bounds.x1 - 160.0).max(page0_bounds.x0 + 12.0),
        y0: page0_bounds.y0 + 16.0,
        x1: (page0_bounds.x1 - 16.0).max(page0_bounds.x0 + 150.0),
        y1: page0_bounds.y0 + 52.0,
    };
    let mut ann = Annotation::new(
        0,
        Shape::Boxed(Boxed::Stamp, rect),
        Style {
            color: Some([0.09, 0.40, 0.20]),
            opacity: 1.0,
            width: 0.0,
        },
        author,
    );
    ann.icon = format!("FinalScore{}", ann.id);
    ann.contents = format!("Score: {label}");
    Ok((ann, StampArt::Page(pdf)))
}

// ---------------------------------------------------------------------------
// G.11: Comment Bank
// ---------------------------------------------------------------------------

/// How a saved comment is inserted onto the PDF page (plan G.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CommentInsertKind {
    /// Insert as a sticky note (`Boxed::Note`).
    #[default]
    StickyNote,
    /// Insert as a visible text box (`Boxed::FreeText`).
    TextBox,
}

/// Reusable saved feedback entry in the teacher's comment bank (plan G.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedComment {
    /// Unique identifier.
    pub id: String,
    /// Quick shortcut mnemonic (e.g. `"c1"`, `"units"`).
    pub shortcut: String,
    /// Short title shown in the picker.
    pub title: String,
    /// Full reusable feedback text.
    pub body: String,
    /// Default annotation placement kind.
    pub insert_kind: CommentInsertKind,
    /// Number of times this comment has been inserted.
    pub usage_count: u64,
}

/// Reusable teacher comment bank supporting shortcut lookup, usage tracking, and JSON
/// import/export (plan G.11).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommentBank {
    entries: Vec<SavedComment>,
}

impl CommentBank {
    /// Creates an empty comment bank.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of saved comments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the comment bank is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns all saved comments in bank order.
    #[must_use]
    pub fn entries(&self) -> &[SavedComment] {
        &self.entries
    }

    /// Adds or replaces a comment by `id`.
    pub fn upsert(&mut self, comment: SavedComment) {
        if let Some(existing) = self.entries.iter_mut().find(|c| c.id == comment.id) {
            *existing = comment;
        } else {
            self.entries.push(comment);
        }
    }

    /// Inserts the comment matching `shortcut` onto `page` at `at`, incrementing its
    /// `usage_count` (plan G.11).
    #[must_use]
    pub fn insert_by_shortcut(
        &mut self,
        shortcut: &str,
        page: usize,
        at: PagePoint,
        author: &str,
    ) -> Option<Annotation> {
        let entry = self
            .entries
            .iter_mut()
            .find(|c| c.shortcut.eq_ignore_ascii_case(shortcut))?;
        entry.usage_count = entry.usage_count.saturating_add(1);
        let shape = match entry.insert_kind {
            CommentInsertKind::StickyNote => Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: at.x,
                    y0: at.y,
                    x1: at.x + 24.0,
                    y1: at.y + 24.0,
                },
            ),
            CommentInsertKind::TextBox => Shape::Boxed(
                Boxed::FreeText,
                PageRect {
                    x0: at.x,
                    y0: at.y,
                    x1: at.x + 180.0,
                    y1: at.y + 44.0,
                },
            ),
        };
        let mut ann = Annotation::new(
            page,
            shape,
            Style {
                color: Some([0.96, 0.80, 0.20]),
                opacity: 1.0,
                width: 1.0,
            },
            author,
        );
        ann.contents = entry.body.clone();
        Some(ann)
    }

    /// Exports the comment bank as pretty JSON (plan G.11).
    ///
    /// # Errors
    ///
    /// Returns [`GradingError::Json`] if serialization fails.
    pub fn export_json(&self) -> Result<String> {
        let items: Vec<Value> = self
            .entries
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "shortcut": c.shortcut,
                    "title": c.title,
                    "body": c.body,
                    "kind": match c.insert_kind {
                        CommentInsertKind::StickyNote => "sticky_note",
                        CommentInsertKind::TextBox => "text_box",
                    },
                    "usage_count": c.usage_count,
                })
            })
            .collect();
        serde_json::to_string_pretty(&items).map_err(|e| GradingError::Json(e.to_string()))
    }

    /// Imports comments from JSON produced by [`CommentBank::export_json`] (plan G.11).
    ///
    /// # Errors
    ///
    /// Returns [`GradingError::Json`] if the JSON is malformed.
    pub fn import_json(raw: &str) -> Result<Self> {
        let val: Value =
            serde_json::from_str(raw).map_err(|e| GradingError::Json(e.to_string()))?;
        let arr = val
            .as_array()
            .ok_or_else(|| GradingError::Json("expected JSON array of comments".into()))?;
        let mut bank = Self::new();
        for item in arr {
            let obj = item
                .as_object()
                .ok_or_else(|| GradingError::Json("comment entry must be an object".into()))?;
            let get_str = |k: &str| {
                obj.get(k)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| GradingError::Json(format!("missing string field '{k}'")))
            };
            let kind = match obj.get("kind").and_then(Value::as_str) {
                Some("text_box") => CommentInsertKind::TextBox,
                _ => CommentInsertKind::StickyNote,
            };
            let usage_count = obj.get("usage_count").and_then(Value::as_u64).unwrap_or(0);
            bank.upsert(SavedComment {
                id: get_str("id")?,
                shortcut: get_str("shortcut")?,
                title: get_str("title")?,
                body: get_str("body")?,
                insert_kind: kind,
                usage_count,
            });
        }
        Ok(bank)
    }
}

// ---------------------------------------------------------------------------
// G.12: Batch Open & Export Student Submissions
// ---------------------------------------------------------------------------

/// One student submission inside a grading batch (plan G.12).
#[derive(Debug, Clone, PartialEq)]
pub struct StudentSubmission {
    /// Student identifier.
    pub student_id: String,
    /// Student display name.
    pub student_name: String,
    /// Source PDF file path.
    pub source_path: PathBuf,
    /// Graded PDF bytes (or source PDF bytes before grading).
    pub pdf_bytes: Vec<u8>,
    /// Selected rubric criterion levels (`criterion_id -> level_index`).
    pub rubric_levels: BTreeMap<String, usize>,
    /// Annotations/stamps applied to this submission.
    pub annotations: Vec<Annotation>,
}

/// Ordered folder/batch of student submissions supporting next/previous navigation and batch CSV
/// + PDF export (plan G.12).
#[derive(Debug, Clone)]
pub struct SubmissionBatch {
    rubric: Rubric,
    submissions: Vec<StudentSubmission>,
    active_index: usize,
}

impl SubmissionBatch {
    /// Creates a batch over `submissions` graded against `rubric`.
    #[must_use]
    pub fn new(rubric: Rubric, submissions: Vec<StudentSubmission>) -> Self {
        Self {
            rubric,
            submissions,
            active_index: 0,
        }
    }

    /// Number of submissions in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.submissions.len()
    }

    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.submissions.is_empty()
    }

    /// Zero-based active student index.
    #[must_use]
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    /// Currently active student submission, if any.
    #[must_use]
    pub fn current(&self) -> Option<&StudentSubmission> {
        self.submissions.get(self.active_index)
    }

    /// Mutable reference to the currently active student submission.
    pub fn current_mut(&mut self) -> Option<&mut StudentSubmission> {
        self.submissions.get_mut(self.active_index)
    }

    /// Advances to the next student submission, returning the new index if moved.
    pub fn next_student(&mut self) -> Option<usize> {
        if self.active_index + 1 < self.submissions.len() {
            self.active_index += 1;
            Some(self.active_index)
        } else {
            None
        }
    }

    /// Moves to the previous student submission, returning the new index if moved.
    pub fn prev_student(&mut self) -> Option<usize> {
        if self.active_index > 0 {
            self.active_index -= 1;
            Some(self.active_index)
        } else {
            None
        }
    }

    /// Computes scores for every submission and exports a formula-safe CSV summary plus per-student
    /// `(output_filename, pdf_bytes)` pairs (plan G.12).
    #[must_use]
    pub fn export_graded(&self) -> BatchGradingExport {
        let mut csv = String::from(
            "student_id,student_name,rubric_points,stamp_points,total_points,max_points,file\r\n",
        );
        let mut files = Vec::with_capacity(self.submissions.len());
        for sub in &self.submissions {
            let summary = recalculate_scores(&self.rubric, &sub.rubric_levels, &sub.annotations);
            let stamp_pts: f64 = summary.per_page_stamp_points.values().sum();
            let file_name = sub
                .source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("submission.pdf")
                .to_owned();
            let _ = write!(
                csv,
                "{},{},{:.2},{:.2},{:.2},{:.2},{}\r\n",
                csv_safe(&sub.student_id),
                csv_safe(&sub.student_name),
                summary.rubric_points,
                stamp_pts,
                summary.total_points,
                summary.max_points,
                csv_safe(&file_name)
            );
            files.push((file_name, sub.pdf_bytes.clone(), summary));
        }
        BatchGradingExport { csv, files }
    }
}

/// Output produced by [`SubmissionBatch::export_graded`] (plan G.12).
#[derive(Debug, Clone, PartialEq)]
pub struct BatchGradingExport {
    /// Formula-safe RFC 4180 CSV summary of all student scores.
    pub csv: String,
    /// Graded `(filename, pdf_bytes, score_summary)` entries.
    pub files: Vec<(String, Vec<u8>, ScoreSummary)>,
}

fn csv_safe(val: &str) -> String {
    let safe = if val.starts_with(['=', '+', '-', '@']) {
        format!("'{val}")
    } else {
        val.to_owned()
    };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// Collects sorted PDF paths inside `folder` for opening a submission batch (plan G.12).
#[must_use]
pub fn discover_submission_pdfs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut pdfs: Vec<PathBuf> = paths
        .iter()
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        })
        .cloned()
        .collect();
    pdfs.sort();
    pdfs
}

// ---------------------------------------------------------------------------
// G.13: Worksheet Maker (AcroForm Fillable Fields + AnswerKey OCG Layer)
// ---------------------------------------------------------------------------

/// Fillable AcroForm field kind created by the worksheet maker (plan G.13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorksheetFieldKind {
    /// Single-line or multi-line fillable text field (`/FT /Tx`).
    Text {
        /// Initial default value (`/V`).
        default_value: String,
        /// Multi-line flag.
        multiline: TextMultilineMode,
    },
    /// Fillable checkbox (`/FT /Btn`).
    Checkbox {
        /// Initial checked state.
        initial_state: CheckboxInitialState,
    },
    /// Dropdown / choice list (`/FT /Ch`).
    Choice {
        /// Selectable option labels (`/Opt`).
        options: Vec<String>,
        /// Optional preselected option.
        selected: Option<String>,
    },
}

/// Single-line vs multi-line text field mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TextMultilineMode {
    /// Single line of text.
    #[default]
    SingleLine,
    /// Multi-line paragraph text box (`/Ff` bit 13).
    MultiLine,
}

/// Initial state for a worksheet checkbox field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CheckboxInitialState {
    /// Unchecked (`/Off`).
    #[default]
    Unchecked,
    /// Checked (`/Yes`).
    Checked,
}

/// Fillable worksheet field specification (plan G.13).
#[derive(Debug, Clone, PartialEq)]
pub struct WorksheetFieldSpec {
    /// Unique AcroForm field name (`/T`).
    pub name: String,
    /// Zero-based page index.
    pub page: usize,
    /// Bounding rectangle in top-down page coordinates (points).
    pub rect: PageRect,
    /// Field type and initial properties.
    pub kind: WorksheetFieldKind,
}

/// Teacher answer-key overlay entry placed on the `"AnswerKey"` OCG layer (plan G.13).
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerKeyItem {
    /// Zero-based page index.
    pub page: usize,
    /// Position in top-down page coordinates (points).
    pub position: PagePoint,
    /// Answer text shown in the teacher edition and stripped/hidden in student copies.
    pub answer_text: String,
}

/// Builds a teacher worksheet PDF containing fillable AcroForm widget fields (`/Tx`, `/Btn`,
/// `/Ch`) AND an `"AnswerKey"` OCG layer holding `answers` (plan G.13).
///
/// # Errors
///
/// Returns [`GradingError`] if `base_pdf` is invalid or any field targets an invalid page.
pub fn build_worksheet_pdf(
    base_pdf: &[u8],
    fields: &[WorksheetFieldSpec],
    answers: &[AnswerKeyItem],
) -> Result<Vec<u8>> {
    let parsed = ParsedLite::parse(base_pdf)?;
    let pages = parsed.page_objects()?;
    for f in fields {
        if f.page >= pages.len() {
            return Err(GradingError::InvalidInput(format!(
                "field '{}' page {} out of range",
                f.name, f.page
            )));
        }
    }
    for a in answers {
        if a.page >= pages.len() {
            return Err(GradingError::InvalidInput(format!(
                "answer key page {} out of range",
                a.page
            )));
        }
    }

    let mut next_id = parsed.next_id;
    let mut updates: BTreeMap<u32, Vec<u8>> = BTreeMap::new();

    // Allocate shared AnswerKey OCG and Helvetica font.
    let helv_id = next_id;
    next_id += 1;
    updates.insert(
        helv_id,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );

    let answer_ocg_id = next_id;
    next_id += 1;
    updates.insert(
        answer_ocg_id,
        b"<< /Type /OCG /Name (AnswerKey) >>".to_vec(),
    );

    // Create AcroForm widget objects per page.
    let mut acro_field_ids = Vec::with_capacity(fields.len());
    let mut page_annots: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    for f in fields {
        let pid = pages[f.page];
        let mb = parsed.media_box(pid).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let x0 = mb[0] + f.rect.x0;
        let x1 = mb[0] + f.rect.x1;
        let y0 = mb[3] - f.rect.y1;
        let y1 = mb[3] - f.rect.y0;
        let wid = next_id;
        next_id += 1;
        let mut dict = format!(
            "<< /Type /Annot /Subtype /Widget /P {pid} 0 R /T ({}) /Rect [{x0:.2} {y0:.2} {x1:.2} {y1:.2}] /MK << /BC [0 0 0] /BG [1 1 1] >>",
            esc_pdf(&f.name)
        );
        match &f.kind {
            WorksheetFieldKind::Text {
                default_value,
                multiline,
            } => {
                let ff = if *multiline == TextMultilineMode::MultiLine {
                    4096
                } else {
                    0
                };
                let _ = write!(
                    dict,
                    " /FT /Tx /Ff {ff} /DA (/Helv 11 Tf 0 g) /V ({}) /DV ({})",
                    esc_pdf(default_value),
                    esc_pdf(default_value)
                );
            }
            WorksheetFieldKind::Checkbox { initial_state } => {
                let state = match initial_state {
                    CheckboxInitialState::Checked => "Yes",
                    CheckboxInitialState::Unchecked => "Off",
                };
                let _ = write!(dict, " /FT /Btn /V /{state} /AS /{state}");
            }
            WorksheetFieldKind::Choice { options, selected } => {
                dict.push_str(" /FT /Ch /Ff 131072 /DA (/Helv 11 Tf 0 g) /Opt [");
                for opt in options {
                    let _ = write!(dict, " ({})", esc_pdf(opt));
                }
                dict.push(']');
                if let Some(sel) = selected {
                    let _ = write!(dict, " /V ({})", esc_pdf(sel));
                }
            }
        }
        dict.push_str(" >>");
        updates.insert(wid, dict.into_bytes());
        acro_field_ids.push(wid);
        page_annots.entry(f.page).or_default().push(wid);
    }

    // Group AnswerKey content overlays per page.
    let mut page_answers: BTreeMap<usize, Vec<&AnswerKeyItem>> = BTreeMap::new();
    for ans in answers {
        page_answers.entry(ans.page).or_default().push(ans);
    }

    for (page_idx, &pid) in pages.iter().enumerate() {
        let new_widgets = page_annots.get(&page_idx);
        let ans_items = page_answers.get(&page_idx);
        if new_widgets.is_none() && ans_items.is_none() {
            continue;
        }
        let raw_page = parsed.obj_body(pid).unwrap_or_default();
        let mut page_str = String::from_utf8_lossy(&raw_page).into_owned();

        if let Some(widgets) = new_widgets {
            let refs: String = widgets.iter().map(|id| format!(" {id} 0 R")).collect();
            page_str = inject_into_page_annots(&page_str, &refs);
        }

        if let Some(items) = ans_items {
            let mb = parsed.media_box(pid).unwrap_or([0.0, 0.0, 612.0, 792.0]);
            let mut ans_stream = String::from(
                "% ExcellentPDF-AnswerKey-BEGIN\n/OC /ExAnsKey BDC\nq 0.8 0.1 0.1 rg\n",
            );
            for item in items {
                let x = mb[0] + item.position.x;
                let y = mb[3] - item.position.y;
                let _ = writeln!(
                    ans_stream,
                    "BT /ExAnsF1 11 Tf 1 0 0 1 {x:.2} {y:.2} Tm ({}) Tj ET",
                    esc_pdf(&item.answer_text)
                );
            }
            ans_stream.push_str("Q\nEMC\n% ExcellentPDF-AnswerKey-END\n");
            let ans_cid = next_id;
            next_id += 1;
            let ans_obj = format!(
                "<< /Length {} >>\nstream\n{ans_stream}\nendstream",
                ans_stream.len()
            );
            updates.insert(ans_cid, ans_obj.into_bytes());
            page_str = append_content_ref(&page_str, ans_cid);
            page_str = inject_answer_resources(&page_str, helv_id, answer_ocg_id);
        }

        updates.insert(pid, page_str.into_bytes());
    }

    // Update /Catalog with /AcroForm and /OCProperties.
    let raw_cat = parsed.obj_body(parsed.root_id).unwrap_or_default();
    let mut cat_str = String::from_utf8_lossy(&raw_cat).into_owned();
    let fields_refs: String = acro_field_ids
        .iter()
        .map(|id| format!(" {id} 0 R"))
        .collect();
    let acro_dict = format!(
        "<< /Fields [{fields_refs} ] /NeedAppearances true /DR << /Font << /Helv {helv_id} 0 R >> >> >>"
    );
    cat_str = set_dict_entry(&cat_str, "AcroForm", &acro_dict);
    let ocp_dict = format!(
        "<< /OCGs [{answer_ocg_id} 0 R] /D << /ON [{answer_ocg_id} 0 R] /Order [{answer_ocg_id} 0 R] >> >>"
    );
    cat_str = set_dict_entry(&cat_str, "OCProperties", &ocp_dict);
    updates.insert(parsed.root_id, cat_str.into_bytes());

    Ok(append_incremental_patch(
        base_pdf,
        &updates,
        parsed.root_id,
        parsed.xref_offset,
        next_id,
    ))
}

/// Exports a student copy of a worksheet PDF by removing/hiding the `"AnswerKey"` OCG layer so
/// fillable fields remain active and zero answer key text appears (plan G.13).
///
/// # Errors
///
/// Returns [`GradingError`] if `worksheet_pdf` cannot be parsed.
pub fn export_student_copy(worksheet_pdf: &[u8]) -> Result<Vec<u8>> {
    let parsed = ParsedLite::parse(worksheet_pdf)?;
    let mut updates = BTreeMap::new();

    // Remove AnswerKey content streams from every page's /Contents array and set AnswerKey OCG OFF.
    for id in parsed.all_ids() {
        let Some(body) = parsed.obj_body(id) else {
            continue;
        };
        let text = String::from_utf8_lossy(&body);
        if text.contains("% ExcellentPDF-AnswerKey-BEGIN") {
            let blank = b"<< /Length 0 >>\nstream\n\nendstream".to_vec();
            updates.insert(id, blank);
        } else if text.contains("/Name (AnswerKey)") {
            updates.insert(id, b"<< /Type /OCG /Name (AnswerKey) >>".to_vec());
        } else if id == parsed.root_id && text.contains("/OCProperties") {
            // Move AnswerKey OCG to /OFF in /OCProperties /D as well.
            let updated_cat = text.replace("/ON [", "/OFF [");
            updates.insert(id, updated_cat.into_bytes());
        }
    }

    Ok(append_incremental_patch(
        worksheet_pdf,
        &updates,
        parsed.root_id,
        parsed.xref_offset,
        parsed.next_id,
    ))
}

// ---------------------------------------------------------------------------
// Minimal PDF incremental patcher for G.13 (self-contained in excellent-features)
// ---------------------------------------------------------------------------

struct ParsedLite<'a> {
    data: &'a [u8],
    xref: BTreeMap<u32, usize>,
    root_id: u32,
    next_id: u32,
    xref_offset: usize,
}

impl<'a> ParsedLite<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        let sx_pos = find_last(data, b"startxref")
            .ok_or_else(|| GradingError::Document("PDF missing startxref".into()))?;
        let rest = &data[sx_pos + b"startxref".len()..];
        let xref_offset = read_first_int(rest)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| GradingError::Document("invalid startxref offset".into()))?;

        let mut xref = BTreeMap::new();
        let mut root_id = None;
        let mut max_size = 1u32;
        let mut next_xref = Some(xref_offset);
        let mut guard = 0usize;

        while let Some(off) = next_xref {
            guard += 1;
            if guard > 64 || off >= data.len() {
                break;
            }
            let slice = &data[off..];
            if slice.starts_with(b"xref") {
                let mut pos = 4usize;
                while pos < slice.len() {
                    while pos < slice.len() && slice[pos].is_ascii_whitespace() {
                        pos += 1;
                    }
                    if slice[pos..].starts_with(b"trailer") {
                        break;
                    }
                    let (first, adv1) = read_int_adv(&slice[pos..])?;
                    pos += adv1;
                    let (count, adv2) = read_int_adv(&slice[pos..])?;
                    pos += adv2;
                    for i in 0..count.max(0) as u32 {
                        let (obj_off, a1) = read_int_adv(&slice[pos..])?;
                        pos += a1;
                        let (_, a2) = read_int_adv(&slice[pos..])?;
                        pos += a2;
                        while pos < slice.len() && slice[pos].is_ascii_whitespace() {
                            pos += 1;
                        }
                        if pos < slice.len() {
                            let flag = slice[pos];
                            pos += 1;
                            if flag == b'n' {
                                xref.entry(first as u32 + i).or_insert(obj_off as usize);
                            }
                        }
                    }
                }
                if let Some(tpos) = find_first(slice, b"trailer") {
                    let tstr = String::from_utf8_lossy(&slice[tpos..slice.len().min(tpos + 512)]);
                    if root_id.is_none() {
                        root_id = extract_ref(&tstr, "/Root");
                    }
                    if let Some(sz) = extract_int_key(&tstr, "/Size") {
                        max_size = max_size.max(sz as u32);
                    }
                    next_xref =
                        extract_int_key(&tstr, "/Prev").and_then(|v| usize::try_from(v).ok());
                } else {
                    next_xref = None;
                }
            } else {
                break;
            }
        }
        let root_id = root_id.ok_or_else(|| GradingError::Document("missing /Root".into()))?;
        let highest = xref.keys().copied().max().unwrap_or(0) + 1;
        Ok(Self {
            data,
            xref,
            root_id,
            next_id: max_size.max(highest),
            xref_offset,
        })
    }

    fn all_ids(&self) -> Vec<u32> {
        self.xref.keys().copied().collect()
    }

    fn obj_body(&self, id: u32) -> Option<Vec<u8>> {
        let &off = self.xref.get(&id)?;
        let slice = &self.data[off..];
        let obj_pos = find_first(slice, b"obj")?;
        let end_pos = find_first(&slice[obj_pos + 3..], b"endobj")?;
        Some(
            slice[obj_pos + 3..obj_pos + 3 + end_pos]
                .trim_ascii()
                .to_vec(),
        )
    }

    fn page_objects(&self) -> Result<Vec<u32>> {
        let cat =
            String::from_utf8_lossy(&self.obj_body(self.root_id).unwrap_or_default()).into_owned();
        let pages_id = extract_ref(&cat, "/Pages")
            .ok_or_else(|| GradingError::Document("missing /Pages".into()))?;
        let mut out = Vec::new();
        self.collect_pages(pages_id, &mut out, 0);
        Ok(out)
    }

    fn collect_pages(&self, id: u32, out: &mut Vec<u32>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Some(body) = self.obj_body(id) else {
            return;
        };
        let s = String::from_utf8_lossy(&body);
        if s.contains("/Kids") {
            if let Some(kpos) = s.find("/Kids")
                && let Some(lb) = s[kpos..].find('[')
                && let Some(rb) = s[kpos + lb..].find(']')
            {
                let inner = &s[kpos + lb + 1..kpos + lb + rb];
                let tokens: Vec<&str> = inner.split_whitespace().collect();
                for w in tokens.windows(3) {
                    if w[2] == "R"
                        && let Ok(kid) = w[0].parse::<u32>()
                    {
                        self.collect_pages(kid, out, depth + 1);
                    }
                }
            }
        } else {
            out.push(id);
        }
    }

    fn media_box(&self, pid: u32) -> Option<[f32; 4]> {
        let body = self.obj_body(pid)?;
        let s = String::from_utf8_lossy(&body);
        let pos = s.find("/MediaBox")?;
        let lb = s[pos..].find('[')?;
        let rb = s[pos + lb..].find(']')?;
        let nums: Vec<f32> = s[pos + lb + 1..pos + lb + rb]
            .split_whitespace()
            .filter_map(|t| t.parse::<f32>().ok())
            .collect();
        (nums.len() == 4).then(|| [nums[0], nums[1], nums[2], nums[3]])
    }
}

fn append_incremental_patch(
    base: &[u8],
    updates: &BTreeMap<u32, Vec<u8>>,
    root_id: u32,
    prev_xref: usize,
    size: u32,
) -> Vec<u8> {
    let mut out = base.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut offsets = BTreeMap::new();
    for (&id, body) in updates {
        offsets.insert(id, out.len());
        let _ = writeln!(out, "{id} 0 obj");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n");
    for (&id, &off) in &offsets {
        let _ = writeln!(out, "{id} 1");
        let _ = writeln!(out, "{off:010} 00000 n ");
    }
    let _ = writeln!(
        out,
        "trailer\n<< /Size {size} /Root {root_id} 0 R /Prev {prev_xref} >>\nstartxref\n{xref_pos}\n%%EOF"
    );
    out
}

fn inject_into_page_annots(page_dict: &str, new_refs: &str) -> String {
    if let Some(pos) = page_dict.find("/Annots")
        && let Some(lb) = page_dict[pos..].find('[')
        && let Some(rb) = page_dict[pos + lb..].find(']')
    {
        let insert_at = pos + lb + rb;
        let mut out = String::with_capacity(page_dict.len() + new_refs.len());
        out.push_str(&page_dict[..insert_at]);
        out.push_str(new_refs);
        out.push_str(&page_dict[insert_at..]);
        return out;
    }
    set_dict_entry(page_dict, "Annots", &format!("[{new_refs} ]"))
}

fn append_content_ref(page_dict: &str, new_cid: u32) -> String {
    if let Some(pos) = page_dict.find("/Contents") {
        let after = page_dict[pos + 9..].trim_start();
        let offset = page_dict.len() - after.len();
        if after.starts_with('[')
            && let Some(rb) = after.find(']')
        {
            let mut out = String::new();
            out.push_str(&page_dict[..offset + rb]);
            let _ = write!(out, " {new_cid} 0 R");
            out.push_str(&page_dict[offset + rb..]);
            return out;
        }
        let tokens: Vec<&str> = after.split_whitespace().collect();
        if tokens.len() >= 3 && tokens[2].starts_with('R') {
            let old_ref = format!("{} {} R", tokens[0], tokens[1]);
            let rest_start = offset + after.find(&old_ref).unwrap_or(0) + old_ref.len();
            let mut out = String::new();
            out.push_str(&page_dict[..offset]);
            let _ = write!(out, "[{old_ref} {new_cid} 0 R]");
            out.push_str(&page_dict[rest_start..]);
            return out;
        }
    }
    set_dict_entry(page_dict, "Contents", &format!("{new_cid} 0 R"))
}

fn inject_answer_resources(page_dict: &str, helv_id: u32, ocg_id: u32) -> String {
    let mut s = page_dict.to_owned();
    if let Some(pos) = s.rfind(">>") {
        let mut extra = String::new();
        if !s.contains("/Properties") {
            let _ = write!(extra, " /Properties << /ExAnsKey {ocg_id} 0 R >>");
        }
        if !s.contains("/ExAnsF1") {
            let _ = write!(extra, " /Font << /ExAnsF1 {helv_id} 0 R >>");
        }
        if s.contains("/Resources <<") {
            if let Some(rpos) = s.find("/Resources <<") {
                let ins = rpos + "/Resources <<".len();
                let mut out = String::new();
                out.push_str(&s[..ins]);
                out.push_str(&extra);
                out.push_str(&s[ins..]);
                return out;
            }
        } else {
            let mut out = String::new();
            out.push_str(&s[..pos]);
            let _ = write!(out, " /Resources <<{extra} >> ");
            out.push_str(&s[pos..]);
            s = out;
        }
    }
    s
}

fn set_dict_entry(dict_str: &str, key: &str, val: &str) -> String {
    if let Some(pos) = dict_str.rfind(">>") {
        let mut out = String::with_capacity(dict_str.len() + key.len() + val.len() + 8);
        out.push_str(&dict_str[..pos]);
        let _ = write!(out, " /{key} {val} ");
        out.push_str(&dict_str[pos..]);
        out
    } else {
        dict_str.to_owned()
    }
}

fn esc_pdf(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn find_first(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn read_first_int(slice: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(slice).ok()?;
    s.split_whitespace().next()?.parse::<i64>().ok()
}

fn read_int_adv(slice: &[u8]) -> Result<(i64, usize)> {
    let mut i = 0usize;
    while i < slice.len() && slice[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    while i < slice.len() && (slice[i].is_ascii_digit() || slice[i] == b'-') {
        i += 1;
    }
    let v = std::str::from_utf8(&slice[start..i])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| GradingError::Document("failed to parse PDF integer".into()))?;
    Ok((v, i))
}

fn extract_ref(s: &str, key: &str) -> Option<u32> {
    let pos = s.find(key)?;
    s[pos + key.len()..]
        .split_whitespace()
        .next()?
        .parse::<u32>()
        .ok()
}

fn extract_int_key(s: &str, key: &str) -> Option<i64> {
    let pos = s.find(key)?;
    s[pos + key.len()..]
        .split_whitespace()
        .next()?
        .parse::<i64>()
        .ok()
}

#[cfg(test)]
mod tests {
    use excellent_core::notebook::pdf_ops::build_blank_pdf;
    use excellent_mupdf::Document;

    use super::*;

    fn sample_rubric() -> Rubric {
        Rubric {
            title: "Lab Report".into(),
            criteria: vec![
                RubricCriterion {
                    id: "method".into(),
                    title: "Methodology".into(),
                    max_points: 10.0,
                    levels: vec![
                        RubricLevel {
                            label: "Full".into(),
                            points: 10.0,
                            description: "Complete".into(),
                        },
                        RubricLevel {
                            label: "Partial".into(),
                            points: 6.0,
                            description: "Minor gaps".into(),
                        },
                    ],
                },
                RubricCriterion {
                    id: "analysis".into(),
                    title: "Analysis".into(),
                    max_points: 10.0,
                    levels: vec![RubricLevel {
                        label: "Full".into(),
                        points: 10.0,
                        description: "Rigorous".into(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn grading_stamps_rubric_recalculation_and_comment_bank_round_trip() {
        let (check, _) = create_grading_stamp(
            0,
            PagePoint { x: 100.0, y: 100.0 },
            &GradingStampKind::Check,
            "Prof. Ada",
        )
        .expect("check");
        let (plus, _) = create_grading_stamp(
            0,
            PagePoint { x: 140.0, y: 120.0 },
            &GradingStampKind::ScoreDelta(2.5),
            "Prof. Ada",
        )
        .expect("plus");
        let (minus, _) = create_grading_stamp(
            1,
            PagePoint { x: 140.0, y: 200.0 },
            &GradingStampKind::ScoreDelta(-0.5),
            "Prof. Ada",
        )
        .expect("minus");
        assert_eq!(stamp_points(&check), Some(0.0));
        assert_eq!(stamp_points(&plus), Some(2.5));

        let rubric = sample_rubric();
        let mut levels = BTreeMap::new();
        levels.insert("method".into(), 1); // 6.0
        levels.insert("analysis".into(), 0); // 10.0
        let anns = vec![check, plus.clone(), minus];
        let score = recalculate_scores(&rubric, &levels, &anns);
        assert!((score.rubric_points - 16.0).abs() < 0.01);
        assert!((score.total_points - 18.0).abs() < 0.01);

        // Removing a stamp updates totals immediately.
        let score2 = recalculate_scores(&rubric, &levels, &[plus]);
        assert!((score2.total_points - 18.5).abs() < 0.01);

        let mut bank = CommentBank::new();
        bank.upsert(SavedComment {
            id: "c-units".into(),
            shortcut: "units".into(),
            title: "Missing units".into(),
            body: "Include SI units on every axis.".into(),
            insert_kind: CommentInsertKind::StickyNote,
            usage_count: 0,
        });
        let ann = bank
            .insert_by_shortcut("units", 0, PagePoint { x: 50.0, y: 50.0 }, "Prof. Ada")
            .expect("inserted");
        assert_eq!(ann.contents, "Include SI units on every axis.");
        assert_eq!(bank.entries()[0].usage_count, 1);

        let json = bank.export_json().expect("export");
        let imported = CommentBank::import_json(&json).expect("import");
        assert_eq!(imported, bank);
    }

    #[test]
    fn batch_fifty_submissions_export_with_matching_csv_totals() {
        let rubric = sample_rubric();
        let blank = build_blank_pdf(&[(612.0, 792.0)]).expect("blank");
        let mut subs = Vec::with_capacity(50);
        for i in 0..50 {
            let (stamp, _) = create_grading_stamp(
                0,
                PagePoint { x: 80.0, y: 80.0 },
                &GradingStampKind::ScoreDelta(f64::from(i % 4) * 0.5),
                "Grader",
            )
            .expect("stamp");
            let mut levels = BTreeMap::new();
            levels.insert("method".into(), 0); // 10.0
            subs.push(StudentSubmission {
                student_id: format!("S{:03}", i + 1),
                student_name: format!("Student {}", i + 1),
                source_path: PathBuf::from(format!("student_{:03}.pdf", i + 1)),
                pdf_bytes: blank.clone(),
                rubric_levels: levels,
                annotations: vec![stamp],
            });
        }
        let mut batch = SubmissionBatch::new(rubric, subs);
        assert_eq!(batch.len(), 50);
        assert_eq!(batch.next_student(), Some(1));
        assert_eq!(batch.prev_student(), Some(0));
        let exported = batch.export_graded();
        assert_eq!(exported.files.len(), 50);
        let lines: Vec<&str> = exported.csv.lines().collect();
        assert_eq!(lines.len(), 51);
        assert!(lines[1].contains("S001,Student 1,10.00,0.00,10.00,20.00"));
        assert!(lines[2].contains("S002,Student 2,10.00,0.50,10.50,20.00"));
    }

    #[test]
    fn worksheet_maker_builds_fillable_fields_and_strips_answers_for_students() {
        let blank = build_blank_pdf(&[(612.0, 792.0)]).expect("blank");
        let teacher = build_worksheet_pdf(
            &blank,
            &[
                WorksheetFieldSpec {
                    name: "q1_answer".into(),
                    page: 0,
                    rect: PageRect {
                        x0: 72.0,
                        y0: 100.0,
                        x1: 300.0,
                        y1: 124.0,
                    },
                    kind: WorksheetFieldKind::Text {
                        default_value: String::new(),
                        multiline: TextMultilineMode::SingleLine,
                    },
                },
                WorksheetFieldSpec {
                    name: "q2_agree".into(),
                    page: 0,
                    rect: PageRect {
                        x0: 72.0,
                        y0: 150.0,
                        x1: 90.0,
                        y1: 168.0,
                    },
                    kind: WorksheetFieldKind::Checkbox {
                        initial_state: CheckboxInitialState::Unchecked,
                    },
                },
                WorksheetFieldSpec {
                    name: "q3_choice".into(),
                    page: 0,
                    rect: PageRect {
                        x0: 72.0,
                        y0: 200.0,
                        x1: 220.0,
                        y1: 224.0,
                    },
                    kind: WorksheetFieldKind::Choice {
                        options: vec!["A".into(), "B".into(), "C".into()],
                        selected: None,
                    },
                },
            ],
            &[AnswerKeyItem {
                page: 0,
                position: PagePoint { x: 320.0, y: 116.0 },
                answer_text: "KEY: 42".into(),
            }],
        )
        .expect("worksheet");

        let t_doc = Document::from_bytes(&teacher, "application/pdf").expect("teacher open");
        let t_text = t_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("s")
            .text()
            .expect("t");
        assert!(
            t_text.contains("KEY: 42"),
            "teacher copy has answer: {t_text}"
        );
        assert!(
            t_doc
                .layers()
                .expect("layers")
                .iter()
                .any(|l| l.name == "AnswerKey")
        );

        let student = export_student_copy(&teacher).expect("student copy");
        let s_doc = Document::from_bytes(&student, "application/pdf").expect("student open");
        let s_text = s_doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("s")
            .text()
            .expect("t");
        assert!(
            !s_text.contains("KEY: 42"),
            "student copy stripped answers: {s_text}"
        );
        let raw_s = String::from_utf8_lossy(&student);
        assert!(raw_s.contains("/T (q1_answer)"));
        assert!(raw_s.contains("/T (q2_agree)"));
        assert!(raw_s.contains("/T (q3_choice)"));
    }
}
