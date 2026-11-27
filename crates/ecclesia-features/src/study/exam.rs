//! Exam mode (plan S.10): hides answers, notes and chosen annotation categories, covers H.4 tape strips for
//! self-testing, runs an optional countdown timer, supports one-at-a-time reveal, and restores the exact previous
//! visibility state on exit.

use std::collections::{BTreeMap, BTreeSet};

use ecclesia_core::annot::{Annotation, Boxed, Shape};
use ecclesia_core::edit::Change;

use crate::annotate::categories::Category;
use crate::annotate::{edit, tape};

/// Configuration chosen when entering exam mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamConfig {
    /// Hide note annotations (sticky notes, free text, callouts and highlight comments).
    pub hide_notes: bool,
    /// Hide card/exercise answers until revealed.
    pub hide_answers: bool,
    /// Semantic categories (A.17) hidden during the exam (e.g. `Definition`, `Result`).
    pub hide_categories: Vec<Category>,
    /// Cover all unlocked H.4 tape strips when entering exam mode.
    pub cover_tape: bool,
    /// Optional countdown duration in seconds (`None` = untimed / stopwatch only).
    pub time_limit_secs: Option<u64>,
}

impl Default for ExamConfig {
    fn default() -> Self {
        Self {
            hide_notes: true,
            hide_answers: true,
            hide_categories: vec![Category::Definition, Category::Result],
            cover_tape: true,
            time_limit_secs: None,
        }
    }
}

/// Exact visibility state of the document viewer before entering exam mode, restored verbatim on exit (plan S.10).
#[derive(Debug, Clone, PartialEq)]
pub struct VisibilitySnapshot {
    /// Annotation ids that were hidden prior to exam mode.
    pub hidden_annotations: BTreeSet<String>,
    /// Layer names that were hidden prior to exam mode.
    pub hidden_layers: BTreeSet<String>,
    /// Whether margin notes / note popups were visible.
    pub notes_visible: bool,
    /// Whether flashcard / inline answers were visible.
    pub answers_visible: bool,
    /// Prior opacity of every H.4 tape annotation (keyed by annotation id).
    pub tape_opacities: BTreeMap<String, f32>,
}

impl Default for VisibilitySnapshot {
    fn default() -> Self {
        Self {
            hidden_annotations: BTreeSet::new(),
            hidden_layers: BTreeSet::new(),
            notes_visible: true,
            answers_visible: true,
            tape_opacities: BTreeMap::new(),
        }
    }
}

/// Current active visibility computed for exam mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamVisibility {
    /// Annotation ids currently hidden on the page canvas.
    pub hidden_annotations: BTreeSet<String>,
    /// Layer names currently hidden.
    pub hidden_layers: BTreeSet<String>,
    /// Whether notes are currently shown.
    pub notes_visible: bool,
    /// Whether answers are currently shown.
    pub answers_visible: bool,
}

/// Timer state for an exam session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExamTimerState {
    /// Timer is actively counting.
    Running,
    /// Timer is paused.
    Paused,
    /// Countdown reached zero.
    Expired,
}

/// Active exam mode session (plan S.10).
#[derive(Debug, Clone, PartialEq)]
pub struct ExamSession {
    config: ExamConfig,
    snapshot: VisibilitySnapshot,
    visibility: ExamVisibility,
    reveal_order: Vec<String>,
    revealed: BTreeSet<String>,
    started_at: i64,
    accumulated_secs: u64,
    resumed_at: Option<i64>,
}

impl ExamSession {
    /// Enters exam mode: captures `prior` visibility + every H.4 tape's exact opacity, computes exam visibility, and
    /// returns the session plus any H.4 tape covering [`Change`]s to apply.
    #[must_use]
    pub fn enter(
        annotations: &[Annotation],
        mut prior: VisibilitySnapshot,
        config: ExamConfig,
        now: i64,
    ) -> (Self, Vec<Change>) {
        let mut hidden_annotations = prior.hidden_annotations.clone();
        let mut tape_changes = Vec::new();
        let mut reveal_order = Vec::new();

        for annot in annotations {
            if annot.tape {
                prior
                    .tape_opacities
                    .entry(annot.id.clone())
                    .or_insert(annot.style.opacity);
                if config.cover_tape && !tape::is_hiding(annot) {
                    let mut covered = annot.clone();
                    covered.style.opacity = 1.0;
                    if let Some(change) = edit::change(annot, covered) {
                        tape_changes.push(change);
                    }
                }
                reveal_order.push(annot.id.clone());
                continue;
            }

            let is_note = matches!(
                annot.shape,
                Shape::Boxed(Boxed::Note | Boxed::FreeText | Boxed::Typewriter, _)
                    | Shape::Callout(..)
            );
            let category_hidden =
                Category::of(annot).is_some_and(|cat| config.hide_categories.contains(&cat));

            if (config.hide_notes && is_note) || category_hidden {
                hidden_annotations.insert(annot.id.clone());
                reveal_order.push(annot.id.clone());
            }
        }

        let visibility = ExamVisibility {
            hidden_annotations,
            hidden_layers: prior.hidden_layers.clone(),
            notes_visible: prior.notes_visible && !config.hide_notes,
            answers_visible: prior.answers_visible && !config.hide_answers,
        };

        let session = Self {
            config,
            snapshot: prior,
            visibility,
            reveal_order,
            revealed: BTreeSet::new(),
            started_at: now,
            accumulated_secs: 0,
            resumed_at: Some(now),
        };
        (session, tape_changes)
    }

    /// Active visibility state during the exam.
    #[must_use]
    pub fn visibility(&self) -> &ExamVisibility {
        &self.visibility
    }

    /// Active exam configuration.
    #[must_use]
    pub fn config(&self) -> &ExamConfig {
        &self.config
    }

    /// Reveals one specific hidden annotation or H.4 tape strip by `id`, returning the tape [`Change`] when it is tape.
    pub fn reveal_one(&mut self, id: &str, annotations: &[Annotation]) -> Option<Change> {
        if let Some(annot) = annotations.iter().find(|a| a.id == id && a.tape) {
            self.revealed.insert(id.to_owned());
            if tape::is_hiding(annot) {
                return tape::tapped(annot);
            }
            return None;
        }
        if self.visibility.hidden_annotations.remove(id) {
            self.revealed.insert(id.to_owned());
        }
        None
    }

    /// Reveals the next unrevealed item in reading/creation order, returning its id and optional H.4 tape [`Change`].
    pub fn reveal_next(&mut self, annotations: &[Annotation]) -> Option<(String, Option<Change>)> {
        let next_id = self
            .reveal_order
            .iter()
            .find(|id| !self.revealed.contains(*id))?
            .clone();
        let change = self.reveal_one(&next_id, annotations);
        Some((next_id, change))
    }

    /// Number of items still hidden/unrevealed in the exam queue.
    #[must_use]
    pub fn remaining_hidden(&self) -> usize {
        self.reveal_order
            .iter()
            .filter(|id| !self.revealed.contains(*id))
            .count()
    }

    /// Pauses the exam timer at `now`.
    pub fn pause_timer(&mut self, now: i64) {
        if let Some(start) = self.resumed_at.take() {
            let delta = u64::try_from(now.saturating_sub(start)).unwrap_or(0);
            self.accumulated_secs = self.accumulated_secs.saturating_add(delta);
        }
    }

    /// Resumes the exam timer at `now`.
    pub fn resume_timer(&mut self, now: i64) {
        if self.resumed_at.is_none() && self.timer_state(now) != ExamTimerState::Expired {
            self.resumed_at = Some(now);
        }
    }

    /// Elapsed active exam seconds as of `now`.
    #[must_use]
    pub fn elapsed_secs(&self, now: i64) -> u64 {
        let running = self.resumed_at.map_or(0, |start| {
            u64::try_from(now.saturating_sub(start)).unwrap_or(0)
        });
        self.accumulated_secs.saturating_add(running)
    }

    /// Remaining countdown seconds as of `now`, or `None` if untimed.
    #[must_use]
    pub fn remaining_secs(&self, now: i64) -> Option<u64> {
        let limit = self.config.time_limit_secs?;
        Some(limit.saturating_sub(self.elapsed_secs(now)))
    }

    /// Current timer state as of `now`.
    #[must_use]
    pub fn timer_state(&self, now: i64) -> ExamTimerState {
        if self.remaining_secs(now) == Some(0) {
            ExamTimerState::Expired
        } else if self.resumed_at.is_some() {
            ExamTimerState::Running
        } else {
            ExamTimerState::Paused
        }
    }

    /// Timestamp when the exam began.
    #[must_use]
    pub fn started_at(&self) -> i64 {
        self.started_at
    }

    /// Exits exam mode and returns the exact prior [`VisibilitySnapshot`] plus [`Change`]s restoring every H.4 tape's
    /// exact previous opacity (plan S.10 **Done when**).
    #[must_use]
    pub fn exit(self, current_annotations: &[Annotation]) -> (VisibilitySnapshot, Vec<Change>) {
        let mut restore_changes = Vec::new();
        for annot in current_annotations {
            if let Some(&prev_opacity) = self.snapshot.tape_opacities.get(&annot.id)
                && (annot.style.opacity - prev_opacity).abs() > f32::EPSILON
            {
                let mut restored = annot.clone();
                restored.style.opacity = prev_opacity;
                if let Some(change) = edit::change(annot, restored) {
                    restore_changes.push(change);
                }
            }
        }
        (self.snapshot, restore_changes)
    }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::{Markup, Style};
    use ecclesia_mupdf::PageRect;

    use super::*;

    fn apply_changes(annotations: &mut [Annotation], changes: Vec<Change>) {
        for change in changes {
            if let Change::UpdateAnnotation(updated) = change
                && let Some(slot) = annotations.iter_mut().find(|a| a.id == updated.id)
            {
                *slot = updated;
            }
        }
    }

    /// Done when (S.10): leaving exam mode restores the exact previous visibility and tape states.
    #[test]
    fn exam_mode_hides_reveals_times_and_restores_exact_visibility() {
        let mut note = Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 30.0,
                    y1: 30.0,
                },
            ),
            Style::default(),
            "Ada",
        );
        note.id = "note-1".to_owned();

        let mut def_highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        def_highlight.id = "def-1".to_owned();
        def_highlight.category = Some(Category::Definition.key().to_owned());

        let mut claim_highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        claim_highlight.id = "claim-1".to_owned();
        claim_highlight.category = Some(Category::Claim.key().to_owned());

        // Tape already revealed before entering exam mode.
        let mut revealed_tape = tape::tape(
            0,
            PageRect {
                x0: 40.0,
                y0: 40.0,
                x1: 140.0,
                y1: 56.0,
            },
            "Ada",
        );
        revealed_tape.id = "tape-1".to_owned();
        revealed_tape.style.opacity = tape::REVEALED;

        let mut annotations = vec![note, def_highlight, claim_highlight, revealed_tape];
        let mut prior = VisibilitySnapshot::default();
        prior.hidden_annotations.insert("pre-hidden".to_owned());
        prior.hidden_layers.insert("Instructor".to_owned());

        let config = ExamConfig {
            time_limit_secs: Some(600),
            ..ExamConfig::default()
        };

        let (mut session, enter_changes) =
            ExamSession::enter(&annotations, prior.clone(), config, 1_000);
        apply_changes(&mut annotations, enter_changes);

        // Note, definition highlight and pre-hidden annotation are hidden; claim stays visible; tape is covered.
        assert!(session.visibility().hidden_annotations.contains("note-1"));
        assert!(session.visibility().hidden_annotations.contains("def-1"));
        assert!(
            session
                .visibility()
                .hidden_annotations
                .contains("pre-hidden")
        );
        assert!(!session.visibility().hidden_annotations.contains("claim-1"));
        assert!(!session.visibility().notes_visible);
        assert!(!session.visibility().answers_visible);
        assert!(tape::is_hiding(&annotations[3]), "tape covered on entry");

        // Timer tracks elapsed, pause, resume and expiry.
        assert_eq!(session.elapsed_secs(1_120), 120);
        assert_eq!(session.remaining_secs(1_120), Some(480));
        session.pause_timer(1_120);
        assert_eq!(session.timer_state(1_500), ExamTimerState::Paused);
        assert_eq!(session.elapsed_secs(1_500), 120);
        session.resume_timer(1_500);
        assert_eq!(session.elapsed_secs(2_100), 720);
        assert_eq!(session.timer_state(2_100), ExamTimerState::Expired);

        // Reveal one at a time.
        let (first_id, first_change) = session.reveal_next(&annotations).expect("first reveal");
        assert_eq!(first_id, "note-1");
        assert!(first_change.is_none());
        assert!(!session.visibility().hidden_annotations.contains("note-1"));

        let (second_id, _) = session.reveal_next(&annotations).expect("second reveal");
        assert_eq!(second_id, "def-1");
        let (third_id, third_change) = session.reveal_next(&annotations).expect("third reveal");
        assert_eq!(third_id, "tape-1");
        apply_changes(&mut annotations, third_change.into_iter().collect());
        assert!(!tape::is_hiding(&annotations[3]));

        // Cover tape again mid-exam so exit genuinely has to restore its prior revealed opacity.
        annotations[3].style.opacity = 1.0;

        let (restored, exit_changes) = session.exit(&annotations);
        apply_changes(&mut annotations, exit_changes);

        assert_eq!(restored.hidden_annotations, prior.hidden_annotations);
        assert_eq!(restored.hidden_layers, prior.hidden_layers);
        assert_eq!(restored.notes_visible, prior.notes_visible);
        assert_eq!(restored.answers_visible, prior.answers_visible);
        assert!(
            (annotations[3].style.opacity - tape::REVEALED).abs() < f32::EPSILON,
            "tape restored to exact prior revealed opacity"
        );
    }
}
