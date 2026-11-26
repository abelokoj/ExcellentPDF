//! Optional AI workspace features, handwriting math-to-LaTeX, and chapter quizzes (plan I.3–I.11, H.25, S.9).
//!
//! - [`chat`]: Cited AI chat panel state and canvas jump targets (`I.3`).
//! - [`actions`]: Document/chapter summaries (`I.4`), plain-language rewrites (`I.5`), step-by-step equation explanations (`I.6`), and contextual term definitions (`I.7`).
//! - [`study`]: Glossary, study-guide, and flashcard draft generation bridged to `study::cards` (`I.8`).
//! - [`extract`]: Claim/method/finding/limitation extraction (`I.9`), cross-document comparison evidence tables (`I.10`), and suggested ghost highlights & tags (`I.11`).
//! - [`math`]: Handwritten math stroke cluster -> LaTeX converter & SVG/bitmap preview renderer (`H.25`).
//! - [`quiz`]: Chapter quiz generator integrated with `study::cards` and `study::exam` (`S.9`).

pub mod actions;
pub mod chat;
pub mod extract;
pub mod math;
pub mod quiz;
pub mod study;

pub use actions::{
    ActionOutput, EquationStep, SummaryLength, build_note_annotation, define_term_in_context,
    explain_equation_steps, rewrite_plain_language, summarize_chapter, summarize_document,
};
pub use chat::{ChatPanelModel, ChatTurn, CitationJumpTarget};
pub use extract::{
    EvidenceCell, EvidenceRow, EvidenceTable, ExtractedItem, ExtractionKind, SuggestedHighlight,
    apply_accepted_highlights, build_evidence_table, extract_structured_items,
    suggest_highlights_from_extractions,
};
pub use math::{
    MathBackend, MathConfirmationState, MathConversionResult, convert_strokes_to_latex,
    render_math_strokes_rgba, render_math_strokes_svg, validate_latex_braces,
};
pub use quiz::{
    ChapterQuizDraft, QuizExamRunner, QuizOutcome, QuizQuestionDraft, QuizQuestionKind,
    generate_chapter_quiz,
};
pub use study::{
    DraftDecision, FlashcardDraft, GlossaryEntryDraft, StudyDraftBatch, StudyGuideSectionDraft,
    generate_study_materials,
};
