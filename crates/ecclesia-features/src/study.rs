//! Study tools (plan §27.10): flashcards from highlights, notes and lasso selections ([`cards`], S.1–S.3),
//! FSRS-5 spaced repetition ([`srs`], S.4), daily review queue and crop cache ([`review`], S.5), study export
//! ([`export`], S.8), exam mode ([`exam`], S.10), Anki `.apkg` import/export ([`anki`], S.11), and live-computed
//! goals, focus timer, statistics and streaks ([`stats`], S.12).

pub mod anki;
pub mod assignments;
pub mod cards;
pub mod exam;
pub mod export;
pub mod language;
pub mod review;
pub mod srs;
pub mod stats;
pub mod summaries;
