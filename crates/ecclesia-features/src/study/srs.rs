//! Spaced repetition engine (plan S.4): pure Rust FSRS-5 scheduler matching the open-spaced-repetition FSRS-5
//! reference specification and test vectors, append-only review log support, on-demand parameter optimization, and
//! lossless migration of SumatraPDF Enhanced's `SumatraPDF-vocabulary.json` Leitner / SM-2 scheduling state.

use ecclesia_store::study::{ReviewLogRecord, SrsRecord};

/// Seconds in one calendar day (`86_400`).
pub const SECONDS_PER_DAY: i64 = 86_400;

/// FSRS-5 forgetting-curve decay exponent (`-0.5`).
pub const FSRS_DECAY: f64 = -0.5;

/// FSRS-5 forgetting-curve factor (`19 / 81`), chosen so `retrievability(S, S) == 0.9`.
pub const FSRS_FACTOR: f64 = 19.0 / 81.0;

/// Default desired retention (`0.90`).
pub const DEFAULT_RETENTION: f64 = 0.9;

/// Default maximum scheduled interval in days (100 years, matching `fsrs-rs`).
pub const DEFAULT_MAX_INTERVAL: u32 = 36_500;

/// Minimum allowed stability in days.
const MIN_STABILITY: f64 = 0.1;

/// Official FSRS-5 default 19-parameter weight vector (`open-spaced-repetition/fsrs-rs` v2.0).
pub const DEFAULT_WEIGHTS: [f64; 19] = [
    0.40255, 1.18385, 3.173, 15.69105, 7.1949, 0.5345, 1.4604, 0.0046, 1.54575, 0.1192, 1.01925,
    1.9395, 0.11, 0.29605, 2.2698, 0.2315, 2.9898, 0.51655, 0.6621,
];

/// Recall grade supplied by the learner (keyboard keys `1`–`4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Rating {
    /// Complete blackout / incorrect recall (`1`).
    Again = 1,
    /// Recalled with significant difficulty (`2`).
    Hard = 2,
    /// Recalled with normal effort (`3`).
    Good = 3,
    /// Recalled effortlessly (`4`).
    Easy = 4,
}

impl Rating {
    /// All four ratings in ascending order (`Again`, `Hard`, `Good`, `Easy`).
    pub const ALL: [Self; 4] = [Self::Again, Self::Hard, Self::Good, Self::Easy];

    /// Numeric FSRS rating (`1..=4`).
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Parses a numeric FSRS rating (`1..=4`).
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Again),
            2 => Some(Self::Hard),
            3 => Some(Self::Good),
            4 => Some(Self::Easy),
            _ => None,
        }
    }
}

/// Lifecycle state of a scheduled card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CardPhase {
    /// Never reviewed.
    #[default]
    New = 0,
    /// Initial learning steps.
    Learning = 1,
    /// Graduated long-term review.
    Review = 2,
    /// Relearning after a lapse.
    Relearning = 3,
}

impl CardPhase {
    /// Numeric state code stored in SQLite.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Parses a stored state code.
    #[must_use]
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Learning,
            2 => Self::Review,
            3 => Self::Relearning,
            _ => Self::New,
        }
    }
}

/// FSRS-5 memory and scheduling state for one card or vocabulary word.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardState {
    /// Lifecycle phase.
    pub phase: CardPhase,
    /// Memory stability in days (interval at which retrievability drops to 90 %).
    pub stability: f64,
    /// Item difficulty in `[1.0, 10.0]` (`0.0` while [`CardPhase::New`]).
    pub difficulty: f64,
    /// Days elapsed between the previous review and the latest review.
    pub elapsed_days: u32,
    /// Scheduled interval in days.
    pub scheduled_days: u32,
    /// Total number of reviews completed.
    pub reps: u32,
    /// Number of lapses (`Again` on a previously reviewed card).
    pub lapses: u32,
    /// Next due timestamp (UTC seconds since 1970).
    pub due: i64,
    /// Timestamp of the last review (UTC seconds since 1970).
    pub last_review: Option<i64>,
    /// Whether the card has reached learned mastery (interval >= 60 days or marked learned).
    pub learned: bool,
}

impl CardState {
    /// Creates a brand-new card due immediately at `now`.
    #[must_use]
    pub fn new_at(now: i64) -> Self {
        Self {
            phase: CardPhase::New,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: 0,
            due: now,
            last_review: None,
            learned: false,
        }
    }

    /// Converts to a SQLite [`SrsRecord`].
    #[must_use]
    pub fn to_record(self) -> SrsRecord {
        SrsRecord {
            state: self.phase.code(),
            stability: self.stability,
            difficulty: self.difficulty,
            elapsed_days: self.elapsed_days,
            scheduled_days: self.scheduled_days,
            reps: self.reps,
            lapses: self.lapses,
            due: self.due,
            last_review: self.last_review,
            learned: self.learned,
        }
    }

    /// Reconstructs from a SQLite [`SrsRecord`].
    #[must_use]
    pub fn from_record(rec: SrsRecord) -> Self {
        Self {
            phase: CardPhase::from_code(rec.state),
            stability: rec.stability,
            difficulty: rec.difficulty,
            elapsed_days: rec.elapsed_days,
            scheduled_days: rec.scheduled_days,
            reps: rec.reps,
            lapses: rec.lapses,
            due: rec.due,
            last_review: rec.last_review,
            learned: rec.learned,
        }
    }
}

/// Configurable FSRS-5 parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct FsrsParameters {
    /// The 19 FSRS-5 weights.
    pub weights: [f64; 19],
    /// Target recall probability at review time (clamped to `(0.0, 1.0)`, default `0.9`).
    pub desired_retention: f64,
    /// Maximum scheduled interval in days (default `36_500`).
    pub maximum_interval: u32,
}

impl Default for FsrsParameters {
    fn default() -> Self {
        Self {
            weights: DEFAULT_WEIGHTS,
            desired_retention: DEFAULT_RETENTION,
            maximum_interval: DEFAULT_MAX_INTERVAL,
        }
    }
}

/// Result of scheduling one review.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewOutcome {
    /// Updated card state to persist.
    pub state: CardState,
    /// Review log entry to append to `study_review_log`.
    pub log: ReviewLogRecord,
}

/// Preview of next intervals for all four ratings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalPreview {
    /// Interval in days if graded [`Rating::Again`].
    pub again_days: u32,
    /// Interval in days if graded [`Rating::Hard`].
    pub hard_days: u32,
    /// Interval in days if graded [`Rating::Good`].
    pub good_days: u32,
    /// Interval in days if graded [`Rating::Easy`].
    pub easy_days: u32,
}

/// Pure Rust FSRS-5 spaced-repetition scheduler (plan S.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FsrsScheduler {
    params: FsrsParameters,
}

impl FsrsScheduler {
    /// Creates a scheduler with the default FSRS-5 weights and `0.9` desired retention.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a scheduler with custom parameters.
    #[must_use]
    pub fn with_parameters(mut params: FsrsParameters) -> Self {
        params.desired_retention = params.desired_retention.clamp(0.01, 0.99);
        params.maximum_interval = params.maximum_interval.max(1);
        Self { params }
    }

    /// Returns the active parameters.
    #[must_use]
    pub fn parameters(&self) -> &FsrsParameters {
        &self.params
    }

    /// Computes probability of recall `R(t, S) = (1 + FACTOR * t / S)^DECAY`.
    #[must_use]
    pub fn retrievability(elapsed_days: f64, stability: f64) -> f64 {
        if stability <= 0.0 {
            return 0.0;
        }
        let t = elapsed_days.max(0.0);
        (1.0 + FSRS_FACTOR * t / stability).powf(FSRS_DECAY)
    }

    /// Initial stability `S_0(G)` for a card's first review.
    #[must_use]
    pub fn init_stability(&self, rating: Rating) -> f64 {
        self.params.weights[usize::from(rating.code() - 1)].max(MIN_STABILITY)
    }

    /// Initial difficulty `D_0(G)` in `[1.0, 10.0]` for a card's first review (FSRS-5 formula).
    #[must_use]
    pub fn init_difficulty(&self, rating: Rating) -> f64 {
        let w = &self.params.weights;
        let g = f64::from(rating.code());
        (w[4] - (w[5] * (g - 1.0)).exp() + 1.0).clamp(1.0, 10.0)
    }

    /// Short-term stability update `S'(S, G)` used when `elapsed_days == 0` (same-day review in FSRS-5).
    #[must_use]
    pub fn short_term_stability(&self, stability: f64, rating: Rating) -> f64 {
        let w = &self.params.weights;
        let g = f64::from(rating.code());
        (stability * (w[17] * (g - 3.0 + w[18])).exp()).max(MIN_STABILITY)
    }

    /// Next difficulty `D'(D, G)` with FSRS-5 linear damping and mean reversion toward `D_0(Easy)`.
    #[must_use]
    pub fn next_difficulty(&self, difficulty: f64, rating: Rating) -> f64 {
        let w = &self.params.weights;
        let g = f64::from(rating.code());
        let delta_d = -w[6] * (g - 3.0);
        let damped = difficulty + delta_d * (10.0 - difficulty) / 9.0;
        let target = self.init_difficulty(Rating::Easy);
        (w[7] * target + (1.0 - w[7]) * damped).clamp(1.0, 10.0)
    }

    /// Stability after a successful recall (`Hard`, `Good` or `Easy`).
    #[must_use]
    pub fn next_recall_stability(
        &self,
        difficulty: f64,
        stability: f64,
        retrievability: f64,
        rating: Rating,
    ) -> f64 {
        let w = &self.params.weights;
        let hard_penalty = if rating == Rating::Hard { w[15] } else { 1.0 };
        let easy_bonus = if rating == Rating::Easy { w[16] } else { 1.0 };
        let factor = w[8].exp()
            * (11.0 - difficulty)
            * stability.powf(-w[9])
            * (((1.0 - retrievability) * w[10]).exp() - 1.0)
            * hard_penalty
            * easy_bonus;
        (stability * (factor + 1.0)).max(MIN_STABILITY)
    }

    /// Stability after a lapse (`Again`), capped at the card's previous stability.
    #[must_use]
    pub fn next_forget_stability(
        &self,
        difficulty: f64,
        stability: f64,
        retrievability: f64,
    ) -> f64 {
        let w = &self.params.weights;
        let post = w[11]
            * difficulty.powf(-w[12])
            * ((stability + 1.0).powf(w[13]) - 1.0)
            * ((1.0 - retrievability) * w[14]).exp();
        post.min(stability).max(MIN_STABILITY)
    }

    /// Converts stability into a rounded integer day interval for `desired_retention`, clamped to `[1, maximum_interval]`.
    #[must_use]
    pub fn next_interval(&self, stability: f64) -> u32 {
        let raw = (stability / FSRS_FACTOR)
            * (self.params.desired_retention.powf(1.0 / FSRS_DECAY) - 1.0);
        let rounded = raw
            .round()
            .clamp(1.0, f64::from(self.params.maximum_interval));
        rounded as u32
    }

    /// Applies one review of `card` at timestamp `now` (UTC seconds) with grade `rating`.
    #[must_use]
    pub fn review(
        &self,
        card_id: &str,
        card: CardState,
        rating: Rating,
        now: i64,
        duration_ms: u64,
    ) -> ReviewOutcome {
        let elapsed_days = match card.last_review {
            Some(prev) if now > prev => {
                u32::try_from((now - prev) / SECONDS_PER_DAY).unwrap_or(u32::MAX)
            }
            _ => 0,
        };

        let (stability, difficulty, next_phase) = if card.phase == CardPhase::New
            || card.stability <= 0.0
        {
            let s = self.init_stability(rating);
            let d = self.init_difficulty(rating);
            let phase = match rating {
                Rating::Again | Rating::Hard => CardPhase::Learning,
                Rating::Good | Rating::Easy => CardPhase::Review,
            };
            (s, d, phase)
        } else {
            let d = self.next_difficulty(card.difficulty.clamp(1.0, 10.0), rating);
            let s = if elapsed_days == 0 {
                self.short_term_stability(card.stability, rating)
            } else {
                let r = Self::retrievability(f64::from(elapsed_days), card.stability);
                if rating == Rating::Again {
                    self.next_forget_stability(card.difficulty.clamp(1.0, 10.0), card.stability, r)
                } else {
                    self.next_recall_stability(
                        card.difficulty.clamp(1.0, 10.0),
                        card.stability,
                        r,
                        rating,
                    )
                }
            };
            let phase = if rating == Rating::Again {
                CardPhase::Relearning
            } else {
                CardPhase::Review
            };
            (s, d, phase)
        };

        let scheduled_days = if rating == Rating::Again {
            1
        } else {
            self.next_interval(stability)
        };
        let due = now.saturating_add(i64::from(scheduled_days) * SECONDS_PER_DAY);
        let lapses = if rating == Rating::Again && card.phase != CardPhase::New {
            card.lapses.saturating_add(1)
        } else {
            card.lapses
        };
        let learned = rating != Rating::Again && scheduled_days >= 60;

        let next_state = CardState {
            phase: next_phase,
            stability,
            difficulty,
            elapsed_days,
            scheduled_days,
            reps: card.reps.saturating_add(1),
            lapses,
            due,
            last_review: Some(now),
            learned,
        };
        let log = ReviewLogRecord {
            id: 0,
            card_id: card_id.to_owned(),
            rating: rating.code(),
            state: card.phase.code(),
            due,
            stability,
            difficulty,
            elapsed_days,
            scheduled_days,
            reviewed_at: now,
            duration_ms,
        };
        ReviewOutcome {
            state: next_state,
            log,
        }
    }

    /// Previews the next interval in days for each of the four ratings without mutating state.
    #[must_use]
    pub fn preview(&self, card: CardState, now: i64) -> IntervalPreview {
        IntervalPreview {
            again_days: self
                .review("", card, Rating::Again, now, 0)
                .state
                .scheduled_days,
            hard_days: self
                .review("", card, Rating::Hard, now, 0)
                .state
                .scheduled_days,
            good_days: self
                .review("", card, Rating::Good, now, 0)
                .state
                .scheduled_days,
            easy_days: self
                .review("", card, Rating::Easy, now, 0)
                .state
                .scheduled_days,
        }
    }
}

/// One card's chronological review sequence used by [`optimize_parameters`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationItem {
    /// Ordered `(elapsed_days_since_previous, rating)` pairs.
    pub reviews: Vec<(u32, Rating)>,
}

/// Result of running the on-demand FSRS parameter optimizer (plan S.4).
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizationReport {
    /// Tuned parameters.
    pub parameters: FsrsParameters,
    /// Mean binary cross-entropy loss before optimization.
    pub initial_loss: f64,
    /// Mean binary cross-entropy loss after optimization.
    pub final_loss: f64,
    /// Number of review transitions evaluated.
    pub sample_count: usize,
}

/// Evaluates mean binary cross-entropy (log loss) of `weights` over `items`.
#[must_use]
pub fn evaluate_log_loss(weights: &[f64; 19], items: &[OptimizationItem]) -> (f64, usize) {
    let scheduler = FsrsScheduler::with_parameters(FsrsParameters {
        weights: *weights,
        ..FsrsParameters::default()
    });
    let mut total_loss = 0.0;
    let mut count = 0_usize;
    for item in items {
        let mut s = 0.0;
        let mut d = 0.0;
        let mut initialized = false;
        for &(delta_t, rating) in &item.reviews {
            if !initialized {
                s = scheduler.init_stability(rating);
                d = scheduler.init_difficulty(rating);
                initialized = true;
            } else {
                let r =
                    FsrsScheduler::retrievability(f64::from(delta_t), s).clamp(1e-6, 1.0 - 1e-6);
                let y = if rating == Rating::Again { 0.0 } else { 1.0 };
                total_loss += -(y * r.ln() + (1.0 - y) * (1.0 - r).ln());
                count += 1;
                let next_d = scheduler.next_difficulty(d, rating);
                let next_s = if delta_t == 0 {
                    scheduler.short_term_stability(s, rating)
                } else if rating == Rating::Again {
                    scheduler.next_forget_stability(d, s, r)
                } else {
                    scheduler.next_recall_stability(d, s, r, rating)
                };
                s = next_s;
                d = next_d;
            }
        }
    }
    if count == 0 {
        (0.0, 0)
    } else {
        (total_loss / count as f64, count)
    }
}

/// Runs an on-demand coordinate/gradient optimizer over review histories, keeping weights strictly bounded and
/// returning the parameters with the lowest log loss (plan S.4).
#[must_use]
pub fn optimize_parameters(
    initial: &FsrsParameters,
    items: &[OptimizationItem],
    iterations: usize,
) -> OptimizationReport {
    let (initial_loss, sample_count) = evaluate_log_loss(&initial.weights, items);
    if sample_count == 0 || iterations == 0 {
        return OptimizationReport {
            parameters: initial.clone(),
            initial_loss,
            final_loss: initial_loss,
            sample_count,
        };
    }

    let bounds: [(f64, f64); 19] = [
        (0.05, 100.0),
        (0.05, 100.0),
        (0.05, 100.0),
        (0.05, 100.0),
        (1.0, 10.0),
        (0.05, 4.0),
        (0.05, 4.0),
        (0.0, 0.75),
        (0.0, 4.5),
        (0.0, 0.8),
        (0.01, 3.5),
        (0.05, 5.0),
        (0.01, 0.8),
        (0.01, 0.8),
        (0.01, 4.0),
        (0.05, 1.0),
        (1.0, 6.0),
        (0.0, 2.0),
        (0.0, 2.0),
    ];

    let mut best_w = initial.weights;
    let mut best_loss = initial_loss;
    let eps = 1e-3;
    let mut step = 0.25;

    for _ in 0..iterations {
        let mut grad = [0.0_f64; 19];
        for i in 0..19 {
            let mut plus = best_w;
            plus[i] = (plus[i] + eps).clamp(bounds[i].0, bounds[i].1);
            let mut minus = best_w;
            minus[i] = (minus[i] - eps).clamp(bounds[i].0, bounds[i].1);
            let span = plus[i] - minus[i];
            if span > 0.0 {
                let (lp, _) = evaluate_log_loss(&plus, items);
                let (lm, _) = evaluate_log_loss(&minus, items);
                grad[i] = (lp - lm) / span;
            }
        }
        let norm: f64 = grad.iter().map(|g| g * g).sum::<f64>().sqrt();
        if norm < 1e-9 {
            break;
        }
        let mut candidate = best_w;
        for i in 0..19 {
            candidate[i] = (best_w[i] - step * grad[i] / norm).clamp(bounds[i].0, bounds[i].1);
        }
        let (cand_loss, _) = evaluate_log_loss(&candidate, items);
        if cand_loss < best_loss {
            best_w = candidate;
            best_loss = cand_loss;
        } else {
            step *= 0.5;
            if step < 1e-4 {
                break;
            }
        }
    }

    OptimizationReport {
        parameters: FsrsParameters {
            weights: best_w,
            desired_retention: initial.desired_retention,
            maximum_interval: initial.maximum_interval,
        },
        initial_loss,
        final_loss: best_loss,
        sample_count,
    }
}

/// Legacy SumatraPDF Enhanced Leitner / SM-2 scheduling fields from `SumatraPDF-vocabulary.json` (plan S.4, S.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnhancedScheduling {
    /// Leitner box (`0..=4`).
    pub box_index: u32,
    /// SM-2 ease factor (`1.3..=10.0`, default `2.5`).
    pub ease: f64,
    /// Consecutive successful repetitions.
    pub repetitions: u32,
    /// Total reviews recorded.
    pub reviews: u32,
    /// Total lapses recorded.
    pub lapses: u32,
    /// Current interval in days.
    pub interval_days: u32,
    /// Scheduled due timestamp (UTC seconds since 1970).
    pub due_time: i64,
    /// Last review timestamp (`0` if never reviewed).
    pub last_reviewed: i64,
    /// Whether Enhanced marked the word learned.
    pub learned: bool,
}

/// Migrates SumatraPDF Enhanced's Leitner / SM-2 state into FSRS-5 [`CardState`], preserving the exact `due`
/// timestamp, review counts, lapses and learned flag (plan S.4, S.7).
#[must_use]
pub fn migrate_enhanced_state(legacy: EnhancedScheduling) -> CardState {
    const LEITNER_INTERVALS: [u32; 5] = [1, 3, 7, 14, 30];
    let has_history = legacy.reviews > 0 || legacy.last_reviewed > 0 || legacy.interval_days > 0;
    if !has_history && !legacy.learned {
        return CardState {
            phase: CardPhase::New,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: legacy.lapses,
            due: legacy.due_time,
            last_review: None,
            learned: false,
        };
    }

    let effective_interval = if legacy.interval_days > 0 {
        legacy.interval_days
    } else if legacy.learned {
        30
    } else {
        LEITNER_INTERVALS[usize::try_from(legacy.box_index.min(4)).unwrap_or(0)]
    };

    // At desired retention 0.9, FSRS-5 stability equals the scheduled interval in days.
    let stability = f64::from(effective_interval).max(MIN_STABILITY);

    // Map SM-2 ease (1.3 = very hard, 2.5 = neutral, 3.5+ = easy) into FSRS difficulty [1.0, 10.0].
    let ease = if legacy.ease.is_finite() {
        legacy.ease.clamp(1.3, 10.0)
    } else {
        2.5
    };
    let difficulty = (5.0 - (ease - 2.5) * 3.0).clamp(1.0, 10.0);

    let phase = if legacy.learned || legacy.repetitions >= 1 || effective_interval >= 3 {
        CardPhase::Review
    } else if legacy.lapses > 0 {
        CardPhase::Relearning
    } else {
        CardPhase::Learning
    };

    CardState {
        phase,
        stability,
        difficulty,
        elapsed_days: 0,
        scheduled_days: effective_interval,
        reps: legacy.reviews.max(legacy.repetitions),
        lapses: legacy.lapses,
        due: legacy.due_time,
        last_review: (legacy.last_reviewed > 0).then_some(legacy.last_reviewed),
        learned: legacy.learned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies exact FSRS-5 reference vector outputs from open-spaced-repetition (`fsrs-rs` v2.0 / `py-fsrs` FSRS-5)
    /// for the canonical 13-review sequence `[Good×6, Again×2, Good×5]` with default weights and 0.9 retention.
    #[test]
    fn fsrs5_matches_reference_test_vectors() {
        let scheduler = FsrsScheduler::new();

        // Fundamental FSRS-5 identity: R(S, S) == 0.9 for any positive S.
        for s in [0.4, 3.173, 15.69105, 120.0] {
            let r = FsrsScheduler::retrievability(s, s);
            assert!((r - 0.9).abs() < 1e-12, "R({s}, {s}) = {r}");
        }

        // Exact initial stabilities and difficulties against FSRS-5 closed-form formulas.
        assert!((scheduler.init_stability(Rating::Again) - 0.40255).abs() < 1e-12);
        assert!((scheduler.init_stability(Rating::Hard) - 1.18385).abs() < 1e-12);
        assert!((scheduler.init_stability(Rating::Good) - 3.173).abs() < 1e-12);
        assert!((scheduler.init_stability(Rating::Easy) - 15.69105).abs() < 1e-12);

        let d0_again = scheduler.init_difficulty(Rating::Again);
        let d0_good = scheduler.init_difficulty(Rating::Good);
        let d0_easy = scheduler.init_difficulty(Rating::Easy);
        assert!((d0_again - 7.1949).abs() < 1e-9);
        assert!((d0_good - 5.2824344228).abs() < 1e-6, "d0_good={d0_good}");
        assert!((d0_easy - 3.2245015926).abs() < 1e-6, "d0_easy={d0_easy}");

        // Canonical review sequence advancing each review by its scheduled interval.
        let ratings = [
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Again,
            Rating::Again,
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Good,
            Rating::Good,
        ];
        // Reference FSRS-5 intervals for default 19 weights at 0.9 retention (stepping t = scheduled_days).
        let expected_intervals = [3, 11, 35, 101, 269, 669, 1, 1, 3, 8, 18, 39, 81];

        let mut now = 1_700_000_000_i64;
        let mut card = CardState::new_at(now);
        let mut intervals = Vec::new();
        for rating in ratings {
            let outcome = scheduler.review("vec-card", card, rating, now, 1_500);
            intervals.push(outcome.state.scheduled_days);
            card = outcome.state;
            now = card.due;
        }
        assert_eq!(
            intervals, expected_intervals,
            "FSRS-5 reference intervals mismatch"
        );
        // Verify exact closed-form step-2 stability and difficulty:
        // R(3, 3.173) = (1 + (19/81) * 3 / 3.173)^(-0.5) ≈ 0.90469185
        // S_2 ≈ 10.7397 -> interval 11; D_2 = 0.0046 * D0(4) + 0.9954 * D0(3) ≈ 5.272968.
        let step1 = scheduler
            .review("s", CardState::new_at(0), Rating::Good, 0, 0)
            .state;
        let step2 = scheduler
            .review("s", step1, Rating::Good, 3 * SECONDS_PER_DAY, 0)
            .state;
        assert!(
            (step2.stability - 10.739_7).abs() < 1e-3,
            "S_2={}",
            step2.stability
        );
        assert!(
            (step2.difficulty - 5.272_968).abs() < 1e-4,
            "D_2={}",
            step2.difficulty
        );
        assert_eq!(card.reps, 13);
        assert_eq!(card.lapses, 2);
        assert!(card.learned);
    }

    #[test]
    fn rating_intervals_are_monotonic_and_same_day_uses_short_term_formula() {
        let scheduler = FsrsScheduler::new();
        let now = 1_700_000_000;
        let new_card = CardState::new_at(now);
        let preview = scheduler.preview(new_card, now);
        assert!(preview.again_days <= preview.hard_days);
        assert!(preview.hard_days <= preview.good_days);
        assert!(preview.good_days < preview.easy_days);

        let first = scheduler
            .review("c1", new_card, Rating::Good, now, 500)
            .state;
        // Same-day review (elapsed_days == 0) uses short_term_stability.
        let same_day = scheduler
            .review("c1", first, Rating::Good, now + 3_600, 400)
            .state;
        let expected_short = scheduler.short_term_stability(first.stability, Rating::Good);
        assert!((same_day.stability - expected_short).abs() < 1e-12);
    }

    #[test]
    fn optimizer_improves_log_loss_on_synthetic_review_logs() {
        // Learner consistently forgets quickly (hard deck): optimizer should reduce log loss.
        let mut items = Vec::new();
        for _ in 0..12 {
            items.push(OptimizationItem {
                reviews: vec![
                    (0, Rating::Good),
                    (3, Rating::Again),
                    (1, Rating::Good),
                    (2, Rating::Again),
                    (1, Rating::Good),
                ],
            });
        }
        let report = optimize_parameters(&FsrsParameters::default(), &items, 25);
        assert_eq!(report.sample_count, 48);
        assert!(
            report.final_loss < report.initial_loss,
            "expected loss {} < {}",
            report.final_loss,
            report.initial_loss
        );
    }

    #[test]
    fn enhanced_scheduling_migration_preserves_due_dates_and_progress() {
        let migrated = migrate_enhanced_state(EnhancedScheduling {
            box_index: 3,
            ease: 2.35,
            repetitions: 4,
            reviews: 7,
            lapses: 2,
            interval_days: 14,
            due_time: 1_701_000_000,
            last_reviewed: 1_700_000_000,
            learned: false,
        });
        assert_eq!(migrated.due, 1_701_000_000);
        assert_eq!(migrated.last_review, Some(1_700_000_000));
        assert_eq!(migrated.reps, 7);
        assert_eq!(migrated.lapses, 2);
        assert_eq!(migrated.scheduled_days, 14);
        assert!((migrated.stability - 14.0).abs() < 1e-9);
        assert!((1.0..=10.0).contains(&migrated.difficulty));
        assert_eq!(migrated.phase, CardPhase::Review);

        // Next FSRS review advances cleanly from the migrated state.
        let outcome =
            FsrsScheduler::new().review("migrated", migrated, Rating::Good, 1_701_000_000, 1_000);
        assert!(outcome.state.scheduled_days > 14);
    }
}
