//! Study goals, focus timer, statistics and streaks (plan S.12).
//!
//! Every statistic, daily progress counter, plot series and streak is computed live from the append-only review log
//! (`study_review_log`) and reading-time log (`study_reading_log`), never stored twice.

use std::collections::BTreeMap;

use ecclesia_store::study::{ReadingLogRecord, ReviewLogRecord, StudyGoalsRecord};

use super::srs::{Rating, SECONDS_PER_DAY};

/// Aggregated study activity for a single UTC calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DailyActivity {
    /// Day index (`unix_secs.div_euclid(86_400)`).
    pub day_index: i64,
    /// Number of flashcards/vocabulary reviews completed on this day.
    pub cards_reviewed: u32,
    /// Number of `Again` (lapse) reviews on this day.
    pub again_count: u32,
    /// Pages read on this day.
    pub pages_read: u32,
    /// Total study seconds (reading time + review time) on this day.
    pub study_secs: u64,
}

impl DailyActivity {
    /// Rounded-down active study minutes on this day.
    #[must_use]
    pub const fn study_minutes(self) -> u32 {
        let mins = self.study_secs / 60;
        if mins > u32::MAX as u64 {
            u32::MAX
        } else {
            mins as u32
        }
    }

    /// Whether every enabled (> 0) target in `goals` was met on this day.
    ///
    /// If all goals are zero, any non-zero activity counts as meeting the day's goal.
    #[must_use]
    pub const fn meets_goals(self, goals: StudyGoalsRecord) -> bool {
        if goals.daily_cards == 0 && goals.daily_pages == 0 && goals.daily_minutes == 0 {
            return self.cards_reviewed > 0 || self.pages_read > 0 || self.study_secs > 0;
        }
        self.cards_reviewed >= goals.daily_cards
            && self.pages_read >= goals.daily_pages
            && self.study_minutes() >= goals.daily_minutes
    }
}

/// Progress toward today's daily study goals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalProgress {
    /// Cards reviewed today / target.
    pub cards_done: u32,
    /// Daily cards goal.
    pub cards_goal: u32,
    /// Pages read today / target.
    pub pages_done: u32,
    /// Daily pages goal.
    pub pages_goal: u32,
    /// Minutes studied today / target.
    pub minutes_done: u32,
    /// Daily minutes goal.
    pub minutes_goal: u32,
    /// Overall completion ratio clamped to `[0.0, 1.0]`.
    pub overall_ratio: f32,
    /// Whether all daily goals are satisfied today.
    pub all_met: bool,
}

/// Streak metrics computed live from daily activity (plan S.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreakInfo {
    /// Consecutive days meeting daily goals ending today (or yesterday if today is still in progress).
    pub current_days: u32,
    /// Longest streak observed across the entire history.
    pub longest_days: u32,
    /// Whether today's goals are already met.
    pub goal_met_today: bool,
    /// Whether to play the gentle streak celebration (suppressed when `reduced_motion` is enabled).
    pub celebrate: bool,
}

/// Complete live-computed statistics snapshot for the dashboard and NSH `Plot` charts (plan S.12).
#[derive(Debug, Clone, PartialEq)]
pub struct StudyStatistics {
    /// Total reviews recorded in `review_log`.
    pub total_reviews: usize,
    /// Fraction of reviews rated `Hard`, `Good` or `Easy` (`0.0` when there are no reviews).
    pub retention_rate: f64,
    /// Total pages read across `reading_log`.
    pub total_pages_read: u64,
    /// Total study seconds (reading + reviews) across both logs.
    pub total_study_secs: u64,
    /// Ordered daily time series covering the last `window_days` days ending today (for NSH `Plot`).
    pub daily_series: Vec<DailyActivity>,
    /// Today's goal progress.
    pub today: GoalProgress,
    /// Live-computed streak and gentle celebration flag.
    pub streak: StreakInfo,
}

/// Motion preference for gentle streak celebrations (plan S.12, U.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPreference {
    /// Standard gentle celebration animation allowed.
    Full,
    /// System reduced motion enabled: suppress celebration animation.
    Reduced,
}

/// Computes every study statistic, goal progress, plot series and streak strictly from the raw review log and reading
/// log without any cached/duplicated totals (plan S.12 **Done when**).
#[must_use]
pub fn compute_statistics(
    review_log: &[ReviewLogRecord],
    reading_log: &[ReadingLogRecord],
    goals: StudyGoalsRecord,
    now: i64,
    window_days: usize,
    motion: MotionPreference,
) -> StudyStatistics {
    let today_day = now.div_euclid(SECONDS_PER_DAY);
    let mut by_day: BTreeMap<i64, DailyActivity> = BTreeMap::new();

    let mut passed_reviews = 0_usize;
    let mut total_study_secs = 0_u64;
    let mut total_pages_read = 0_u64;

    for entry in review_log {
        let day = entry.reviewed_at.div_euclid(SECONDS_PER_DAY);
        let slot = by_day.entry(day).or_insert_with(|| DailyActivity {
            day_index: day,
            ..DailyActivity::default()
        });
        slot.cards_reviewed = slot.cards_reviewed.saturating_add(1);
        if entry.rating == Rating::Again.code() {
            slot.again_count = slot.again_count.saturating_add(1);
        } else {
            passed_reviews += 1;
        }
        let secs = entry.duration_ms / 1_000;
        slot.study_secs = slot.study_secs.saturating_add(secs);
        total_study_secs = total_study_secs.saturating_add(secs);
    }

    for entry in reading_log {
        let day = entry.recorded_at.div_euclid(SECONDS_PER_DAY);
        let slot = by_day.entry(day).or_insert_with(|| DailyActivity {
            day_index: day,
            ..DailyActivity::default()
        });
        slot.pages_read = slot.pages_read.saturating_add(entry.pages_read);
        slot.study_secs = slot.study_secs.saturating_add(entry.duration_secs);
        total_pages_read = total_pages_read.saturating_add(u64::from(entry.pages_read));
        total_study_secs = total_study_secs.saturating_add(entry.duration_secs);
    }

    let today_activity = by_day.get(&today_day).copied().unwrap_or(DailyActivity {
        day_index: today_day,
        ..DailyActivity::default()
    });
    let today = compute_goal_progress(today_activity, goals);

    let (current_days, longest_days) = compute_streaks(&by_day, goals, today_day);
    let celebrate = today.all_met && current_days > 0 && motion == MotionPreference::Full;
    let streak = StreakInfo {
        current_days,
        longest_days,
        goal_met_today: today.all_met,
        celebrate,
    };

    let span = window_days.max(1) as i64;
    let start_day = today_day - span + 1;
    let daily_series: Vec<DailyActivity> = (start_day..=today_day)
        .map(|day| {
            by_day.get(&day).copied().unwrap_or(DailyActivity {
                day_index: day,
                ..DailyActivity::default()
            })
        })
        .collect();

    let total_reviews = review_log.len();
    let retention_rate = if total_reviews == 0 {
        0.0
    } else {
        passed_reviews as f64 / total_reviews as f64
    };

    StudyStatistics {
        total_reviews,
        retention_rate,
        total_pages_read,
        total_study_secs,
        daily_series,
        today,
        streak,
    }
}

fn compute_goal_progress(activity: DailyActivity, goals: StudyGoalsRecord) -> GoalProgress {
    let mut parts = 0_u32;
    let mut sum = 0.0_f32;
    if goals.daily_cards > 0 {
        parts += 1;
        sum += (activity.cards_reviewed as f32 / goals.daily_cards as f32).min(1.0);
    }
    if goals.daily_pages > 0 {
        parts += 1;
        sum += (activity.pages_read as f32 / goals.daily_pages as f32).min(1.0);
    }
    if goals.daily_minutes > 0 {
        parts += 1;
        sum += (activity.study_minutes() as f32 / goals.daily_minutes as f32).min(1.0);
    }
    let all_met = activity.meets_goals(goals);
    let overall_ratio = if parts == 0 {
        if all_met { 1.0 } else { 0.0 }
    } else {
        (sum / parts as f32).clamp(0.0, 1.0)
    };
    GoalProgress {
        cards_done: activity.cards_reviewed,
        cards_goal: goals.daily_cards,
        pages_done: activity.pages_read,
        pages_goal: goals.daily_pages,
        minutes_done: activity.study_minutes(),
        minutes_goal: goals.daily_minutes,
        overall_ratio,
        all_met,
    }
}

fn compute_streaks(
    by_day: &BTreeMap<i64, DailyActivity>,
    goals: StudyGoalsRecord,
    today_day: i64,
) -> (u32, u32) {
    let met_days: Vec<i64> = by_day
        .values()
        .filter(|act| act.meets_goals(goals))
        .map(|act| act.day_index)
        .collect();
    if met_days.is_empty() {
        return (0, 0);
    }

    let mut longest = 1_u32;
    let mut run = 1_u32;
    for pair in met_days.windows(2) {
        if pair[1] == pair[0] + 1 {
            run = run.saturating_add(1);
            longest = longest.max(run);
        } else {
            run = 1;
        }
    }

    // Current streak counts backwards from today if met, otherwise from yesterday (grace while today is in progress).
    let anchor = if by_day.get(&today_day).is_some_and(|a| a.meets_goals(goals)) {
        today_day
    } else if by_day
        .get(&(today_day - 1))
        .is_some_and(|a| a.meets_goals(goals))
    {
        today_day - 1
    } else {
        return (0, longest);
    };

    let mut current = 0_u32;
    let mut day = anchor;
    while by_day.get(&day).is_some_and(|a| a.meets_goals(goals)) {
        current = current.saturating_add(1);
        day -= 1;
    }
    (current, longest)
}

/// Phase of the study focus timer (plan S.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPhase {
    /// Not started.
    Idle,
    /// Actively focusing on study.
    Focusing,
    /// Scheduled break between focus intervals.
    Break,
}

/// Configurable study focus timer (Pomodoro-style work/break timer, plan S.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusTimer {
    work_secs: u64,
    break_secs: u64,
    phase: FocusPhase,
    paused: bool,
    elapsed_in_phase: u64,
    phase_resumed_at: Option<i64>,
    completed_sessions: u32,
}

impl Default for FocusTimer {
    fn default() -> Self {
        Self::new(25 * 60, 5 * 60)
    }
}

impl FocusTimer {
    /// Creates an idle focus timer with `work_secs` and `break_secs` durations (minimum 1 second each).
    #[must_use]
    pub fn new(work_secs: u64, break_secs: u64) -> Self {
        Self {
            work_secs: work_secs.max(1),
            break_secs: break_secs.max(1),
            phase: FocusPhase::Idle,
            paused: false,
            elapsed_in_phase: 0,
            phase_resumed_at: None,
            completed_sessions: 0,
        }
    }

    /// Starts a focus session at `now`.
    pub fn start(&mut self, now: i64) {
        self.phase = FocusPhase::Focusing;
        self.paused = false;
        self.elapsed_in_phase = 0;
        self.phase_resumed_at = Some(now);
    }

    /// Pauses the active timer at `now`.
    pub fn pause(&mut self, now: i64) {
        if self.phase != FocusPhase::Idle && !self.paused {
            self.elapsed_in_phase = self.elapsed_secs(now);
            self.phase_resumed_at = None;
            self.paused = true;
        }
    }

    /// Resumes a paused timer at `now`.
    pub fn resume(&mut self, now: i64) {
        if self.phase != FocusPhase::Idle && self.paused {
            self.paused = false;
            self.phase_resumed_at = Some(now);
        }
    }

    /// Resets the timer back to [`FocusPhase::Idle`].
    pub fn reset(&mut self) {
        self.phase = FocusPhase::Idle;
        self.paused = false;
        self.elapsed_in_phase = 0;
        self.phase_resumed_at = None;
    }

    /// Advances the timer at `now`; transitions `Focusing -> Break` (incrementing `completed_sessions`) and
    /// `Break -> Idle` when their durations elapse. Returns whether a phase transition occurred.
    pub fn tick(&mut self, now: i64) -> bool {
        if self.phase == FocusPhase::Idle || self.paused {
            return false;
        }
        let target = self.phase_duration();
        let elapsed = self.elapsed_secs(now);
        if elapsed < target {
            return false;
        }
        match self.phase {
            FocusPhase::Focusing => {
                self.completed_sessions = self.completed_sessions.saturating_add(1);
                self.phase = FocusPhase::Break;
                self.elapsed_in_phase = 0;
                self.phase_resumed_at = Some(now);
                true
            }
            FocusPhase::Break => {
                self.reset();
                true
            }
            FocusPhase::Idle => false,
        }
    }

    /// Current phase.
    #[must_use]
    pub const fn phase(&self) -> FocusPhase {
        self.phase
    }

    /// Whether paused.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Total completed focus intervals.
    #[must_use]
    pub const fn completed_sessions(&self) -> u32 {
        self.completed_sessions
    }

    /// Elapsed seconds in the current phase as of `now`.
    #[must_use]
    pub fn elapsed_secs(&self, now: i64) -> u64 {
        let active = self
            .phase_resumed_at
            .map_or(0, |s| u64::try_from(now.saturating_sub(s)).unwrap_or(0));
        self.elapsed_in_phase.saturating_add(active)
    }

    /// Remaining seconds in the current phase as of `now`.
    #[must_use]
    pub fn remaining_secs(&self, now: i64) -> u64 {
        self.phase_duration().saturating_sub(self.elapsed_secs(now))
    }

    const fn phase_duration(&self) -> u64 {
        match self.phase {
            FocusPhase::Focusing => self.work_secs,
            FocusPhase::Break => self.break_secs,
            FocusPhase::Idle => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(day: i64, rating: Rating, duration_ms: u64) -> ReviewLogRecord {
        ReviewLogRecord {
            id: 0,
            card_id: "c1".to_owned(),
            rating: rating.code(),
            state: 2,
            due: day * SECONDS_PER_DAY,
            stability: 5.0,
            difficulty: 5.0,
            elapsed_days: 1,
            scheduled_days: 5,
            reviewed_at: day * SECONDS_PER_DAY + 3_600,
            duration_ms,
        }
    }

    fn reading(day: i64, pages: u32, duration_secs: u64) -> ReadingLogRecord {
        ReadingLogRecord {
            id: 0,
            path: Some("C:/book.pdf".to_owned()),
            pages_read: pages,
            duration_secs,
            recorded_at: day * SECONDS_PER_DAY + 7_200,
        }
    }

    /// Done when (S.12): statistics and streaks are computed strictly from the review log and reading time.
    #[test]
    fn statistics_and_streaks_are_computed_live_from_logs() {
        let day0 = 19_700_i64;
        let goals = StudyGoalsRecord {
            daily_cards: 2,
            daily_pages: 5,
            daily_minutes: 10,
        };
        let reviews = vec![
            review(day0, Rating::Good, 60_000),
            review(day0, Rating::Easy, 60_000),
            review(day0 + 1, Rating::Good, 60_000),
            review(day0 + 1, Rating::Again, 60_000),
            review(day0 + 2, Rating::Good, 120_000),
            review(day0 + 2, Rating::Hard, 120_000),
        ];
        let readings = vec![
            reading(day0, 6, 600),
            reading(day0 + 1, 5, 540),
            reading(day0 + 2, 8, 600),
        ];
        let now = (day0 + 2) * SECONDS_PER_DAY + 10_000;

        let stats = compute_statistics(&reviews, &readings, goals, now, 7, MotionPreference::Full);
        assert_eq!(stats.total_reviews, 6);
        assert!((stats.retention_rate - (5.0 / 6.0)).abs() < 1e-12);
        assert_eq!(stats.total_pages_read, 19);
        assert_eq!(stats.daily_series.len(), 7);
        assert!(stats.today.all_met);
        assert_eq!(stats.streak.current_days, 3);
        assert_eq!(stats.streak.longest_days, 3);
        assert!(stats.streak.celebrate);

        // Reduced motion suppresses celebration while keeping exact streak counts.
        let reduced = compute_statistics(
            &reviews,
            &readings,
            goals,
            now,
            7,
            MotionPreference::Reduced,
        );
        assert!(!reduced.streak.celebrate);
        assert_eq!(reduced.streak.current_days, 3);
    }

    #[test]
    fn focus_timer_transitions_through_work_pause_break_and_idle() {
        let mut timer = FocusTimer::new(1_500, 300);
        assert_eq!(timer.phase(), FocusPhase::Idle);
        timer.start(1_000);
        assert_eq!(timer.phase(), FocusPhase::Focusing);
        assert_eq!(timer.remaining_secs(1_500), 1_000);

        timer.pause(1_500);
        assert!(timer.is_paused());
        assert!(!timer.tick(5_000));
        assert_eq!(timer.remaining_secs(5_000), 1_000);

        timer.resume(5_000);
        assert!(timer.tick(6_000), "transitions to break");
        assert_eq!(timer.phase(), FocusPhase::Break);
        assert_eq!(timer.completed_sessions(), 1);

        assert!(timer.tick(6_300), "break ends back to idle");
        assert_eq!(timer.phase(), FocusPhase::Idle);
    }
}
