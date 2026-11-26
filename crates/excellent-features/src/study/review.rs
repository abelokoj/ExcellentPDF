//! Daily review queue and home-page review session model (plan S.5).
//!
//! Computes the home page's due count, drives a keyboard-graded (`1`–`4`) review session, and pre-warms a bounded
//! in-memory source-crop cache so a 100-card session never waits on page rendering.

use std::collections::{HashMap, VecDeque};

use excellent_store::study::ReviewLogRecord;

use super::cards::Flashcard;
use super::srs::{
    CardPhase, CardState, FsrsScheduler, IntervalPreview, Rating, ReviewOutcome, SECONDS_PER_DAY,
};

/// Default number of upcoming cards whose source crops are pre-warmed ahead of the current card.
pub const DEFAULT_PREFETCH_AHEAD: usize = 8;

/// Default maximum number of source crops kept in memory during a review session.
pub const DEFAULT_CROP_CACHE_CAPACITY: usize = 128;

/// Cache key identifying a rendered source crop for a flashcard.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CropKey {
    /// Source document path (UTF-8 lossy key).
    pub path: String,
    /// Zero-based page index.
    pub page: usize,
    /// Quantized bounding rectangle `(x0, y0, x1, y1)` in tenths of a page point.
    pub quantized_rect: Option<[i32; 4]>,
}

impl CropKey {
    /// Derives the crop cache key for `card`, or `None` when the card has no source anchor.
    #[must_use]
    pub fn for_card(card: &Flashcard) -> Option<Self> {
        let source = card.source.as_ref()?;
        let quantized_rect = source.rect.map(|r| {
            [
                (r.x0 * 10.0).round() as i32,
                (r.y0 * 10.0).round() as i32,
                (r.x1 * 10.0).round() as i32,
                (r.y1 * 10.0).round() as i32,
            ]
        });
        Some(Self {
            path: source.path.to_string_lossy().into_owned(),
            page: source.page,
            quantized_rect,
        })
    }
}

/// Cached RGBA/PNG source crop image attached to a review card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CropImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Encoded PNG or raw RGBA bytes provided by the renderer.
    pub bytes: Vec<u8>,
}

/// Bounded FIFO/LRU cache of rendered source crops so review transitions stay instantaneous (plan S.5).
#[derive(Debug, Clone)]
pub struct CropCache {
    capacity: usize,
    entries: HashMap<CropKey, CropImage>,
    order: VecDeque<CropKey>,
}

impl Default for CropCache {
    fn default() -> Self {
        Self::new(DEFAULT_CROP_CACHE_CAPACITY)
    }
}

impl CropCache {
    /// Creates a cache holding at most `capacity` crops (minimum 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Looks up a cached crop without triggering rendering.
    #[must_use]
    pub fn get(&self, key: &CropKey) -> Option<&CropImage> {
        self.entries.get(key)
    }

    /// Inserts or replaces a cached crop, evicting the oldest entry when full.
    pub fn insert(&mut self, key: CropKey, image: CropImage) {
        if let Some(slot) = self.entries.get_mut(&key) {
            *slot = image;
            return;
        }
        while self.entries.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, image);
    }

    /// Number of cached crops.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Pre-warms crops for every card in `cards` whose key is not already cached, calling `render` only on misses.
    /// Returns the number of newly rendered crops.
    pub fn prewarm(
        &mut self,
        cards: &[Flashcard],
        mut render: impl FnMut(&Flashcard) -> Option<CropImage>,
    ) -> usize {
        let mut rendered = 0;
        for card in cards {
            let Some(key) = CropKey::for_card(card) else {
                continue;
            };
            if self.entries.contains_key(&key) {
                continue;
            }
            if let Some(image) = render(card) {
                self.insert(key, image);
                rendered += 1;
            }
        }
        rendered
    }
}

/// Home-page daily review summary (plan S.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DailySummary {
    /// Total cards currently due (`due <= now`).
    pub due_now: usize,
    /// Unseen new cards in the due set.
    pub new_cards: usize,
    /// Cards past their due date by at least one full day.
    pub overdue: usize,
    /// Cards already reviewed during the current UTC day.
    pub reviewed_today: usize,
}

/// Computes the home-page [`DailySummary`] from all scheduled cards and today's review log.
#[must_use]
pub fn daily_summary(
    cards: &[(Flashcard, CardState)],
    review_log: &[ReviewLogRecord],
    now: i64,
) -> DailySummary {
    let today_start = now.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY;
    let mut summary = DailySummary::default();
    for (_, state) in cards {
        if state.due <= now {
            summary.due_now += 1;
            if state.phase == CardPhase::New {
                summary.new_cards += 1;
            } else if now.saturating_sub(state.due) >= SECONDS_PER_DAY {
                summary.overdue += 1;
            }
        }
    }
    summary.reviewed_today = review_log
        .iter()
        .filter(|entry| entry.reviewed_at >= today_start && entry.reviewed_at <= now)
        .count();
    summary
}

/// One card queued for review.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    /// Flashcard content and source anchor.
    pub card: Flashcard,
    /// Current FSRS state prior to grading.
    pub state: CardState,
}

/// Maps keyboard shortcuts `1`–`4` to FSRS [`Rating`] grades (plan S.5).
#[must_use]
pub const fn rating_from_key(digit: char) -> Option<Rating> {
    match digit {
        '1' => Some(Rating::Again),
        '2' => Some(Rating::Hard),
        '3' => Some(Rating::Good),
        '4' => Some(Rating::Easy),
        _ => None,
    }
}

/// Interactive daily review session over a due queue with cached source crops (plan S.5).
#[derive(Debug, Clone)]
pub struct ReviewSession {
    queue: Vec<QueueEntry>,
    cursor: usize,
    answer_revealed: bool,
    scheduler: FsrsScheduler,
    completed: usize,
    again_count: usize,
    crops: CropCache,
}

impl ReviewSession {
    /// Builds a review session from cards due at or before `now`, ordered oldest-due first.
    #[must_use]
    pub fn from_due(
        cards: Vec<(Flashcard, CardState)>,
        now: i64,
        scheduler: FsrsScheduler,
    ) -> Self {
        let mut queue: Vec<QueueEntry> = cards
            .into_iter()
            .filter(|(_, state)| state.due <= now)
            .map(|(card, state)| QueueEntry { card, state })
            .collect();
        queue.sort_by(|a, b| {
            a.state
                .due
                .cmp(&b.state.due)
                .then_with(|| a.card.created.cmp(&b.card.created))
                .then_with(|| a.card.id.cmp(&b.card.id))
        });
        Self {
            queue,
            cursor: 0,
            answer_revealed: false,
            scheduler,
            completed: 0,
            again_count: 0,
            crops: CropCache::default(),
        }
    }

    /// Builds a session with an explicitly sized crop cache.
    #[must_use]
    pub fn with_crop_cache(mut self, cache: CropCache) -> Self {
        self.crops = cache;
        self
    }

    /// Pre-warms crops for the next `window` cards starting at the current cursor.
    pub fn prefetch_crops(
        &mut self,
        window: usize,
        mut render: impl FnMut(&Flashcard) -> Option<CropImage>,
    ) -> usize {
        let end = self.cursor.saturating_add(window).min(self.queue.len());
        let mut rendered = 0;
        for entry in &self.queue[self.cursor..end] {
            let Some(key) = CropKey::for_card(&entry.card) else {
                continue;
            };
            if self.crops.get(&key).is_some() {
                continue;
            }
            if let Some(image) = render(&entry.card) {
                self.crops.insert(key, image);
                rendered += 1;
            }
        }
        rendered
    }

    /// Returns the currently displayed card and its state, or `None` when the session is finished.
    #[must_use]
    pub fn current(&self) -> Option<&QueueEntry> {
        self.queue.get(self.cursor)
    }

    /// Returns the cached source crop for the current card (never blocks on rendering).
    #[must_use]
    pub fn current_crop(&self) -> Option<&CropImage> {
        let entry = self.current()?;
        let key = CropKey::for_card(&entry.card)?;
        self.crops.get(&key)
    }

    /// Whether the current card's answer has been revealed.
    #[must_use]
    pub fn is_answer_revealed(&self) -> bool {
        self.answer_revealed
    }

    /// Reveals the answer of the current card.
    pub fn reveal_answer(&mut self) {
        if self.current().is_some() {
            self.answer_revealed = true;
        }
    }

    /// Previews next intervals for the current card's four grading buttons.
    #[must_use]
    pub fn interval_preview(&self, now: i64) -> Option<IntervalPreview> {
        let entry = self.current()?;
        Some(self.scheduler.preview(entry.state, now))
    }

    /// Grades the current card, updates its FSRS state, advances to the next card, and returns the [`ReviewOutcome`]
    /// to persist. Returns `None` if the session is already finished.
    pub fn grade(&mut self, rating: Rating, now: i64, duration_ms: u64) -> Option<ReviewOutcome> {
        let entry = self.queue.get_mut(self.cursor)?;
        let outcome = self
            .scheduler
            .review(&entry.card.id, entry.state, rating, now, duration_ms);
        entry.state = outcome.state;
        self.completed += 1;
        if rating == Rating::Again {
            self.again_count += 1;
        }
        self.cursor += 1;
        self.answer_revealed = false;
        Some(outcome)
    }

    /// Grades the current card using keyboard digit `'1'`..=`'4'`.
    pub fn grade_key(&mut self, digit: char, now: i64, duration_ms: u64) -> Option<ReviewOutcome> {
        let rating = rating_from_key(digit)?;
        self.grade(rating, now, duration_ms)
    }

    /// Remaining cards in the session.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.queue.len().saturating_sub(self.cursor)
    }

    /// Total cards originally queued in the session.
    #[must_use]
    pub fn total(&self) -> usize {
        self.queue.len()
    }

    /// Number of cards graded so far.
    #[must_use]
    pub fn completed(&self) -> usize {
        self.completed
    }

    /// Number of `Again` grades in this session.
    #[must_use]
    pub fn again_count(&self) -> usize {
        self.again_count
    }

    /// Whether every card in the session has been reviewed.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.queue.len()
    }

    /// Access to the session's crop cache.
    #[must_use]
    pub fn crop_cache(&self) -> &CropCache {
        &self.crops
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use excellent_mupdf::PageRect;

    use super::*;
    use crate::study::cards::{CardKind, SourceAnchor};

    fn make_card(idx: usize) -> (Flashcard, CardState) {
        let card = Flashcard {
            id: format!("card-{idx:03}"),
            deck_id: "deck-1".to_owned(),
            kind: CardKind::Basic,
            front: format!("Question {idx}"),
            back: format!("Answer {idx}"),
            source: Some(SourceAnchor {
                path: PathBuf::from("C:/papers/book.pdf"),
                page: idx / 5,
                quote: None,
                rect: Some(PageRect {
                    x0: 10.0,
                    y0: (idx % 5) as f32 * 40.0,
                    x1: 200.0,
                    y1: (idx % 5) as f32 * 40.0 + 30.0,
                }),
                annot_id: None,
            }),
            tags: Vec::new(),
            media: Vec::new(),
            created: 1_700_000_000 + idx as i64,
            modified: 1_700_000_000 + idx as i64,
        };
        let state = CardState::new_at(1_700_000_000);
        (card, state)
    }

    /// Done when (S.5): a review session of 100 cards never waits on rendering once crops are cached/prefetched.
    #[test]
    fn review_session_of_100_cards_uses_cached_crops_without_blocking() {
        let now = 1_700_010_000;
        let deck: Vec<_> = (0..100).map(make_card).collect();
        let summary = daily_summary(&deck, &[], now);
        assert_eq!(summary.due_now, 100);
        assert_eq!(summary.new_cards, 100);

        let mut session = ReviewSession::from_due(deck, now, FsrsScheduler::new())
            .with_crop_cache(CropCache::new(128));

        let mut render_calls = 0_usize;
        let warmed = session.prefetch_crops(100, |card| {
            render_calls += 1;
            Some(CropImage {
                width: 320,
                height: 80,
                bytes: card.id.as_bytes().to_vec(),
            })
        });
        assert_eq!(warmed, 100);
        assert_eq!(render_calls, 100);

        // Second prefetch does zero redundant renders.
        assert_eq!(session.prefetch_crops(100, |_| panic!("must be cached")), 0);

        // Review all 100 cards with keyboard grades 1..=4; every card's crop is immediately available.
        let keys = ['3', '4', '2', '1'];
        for i in 0..100 {
            assert!(!session.is_answer_revealed());
            let crop = session
                .current_crop()
                .expect("crop must be immediately present in cache");
            assert_eq!(crop.bytes, format!("card-{i:03}").as_bytes());
            session.reveal_answer();
            assert!(session.is_answer_revealed());
            let outcome = session
                .grade_key(keys[i % 4], now + i as i64 * 10, 1_200)
                .expect("graded");
            assert_eq!(outcome.log.card_id, format!("card-{i:03}"));
        }
        assert!(session.is_finished());
        assert_eq!(session.completed(), 100);
        assert_eq!(session.again_count(), 25);
        assert_eq!(session.remaining(), 0);
    }
}
