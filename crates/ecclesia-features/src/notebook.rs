//! Notebook features (plan §27.8, §27.9 L.3, and §27.18 U.8).
//!
//! Pure Rust models and PDF/archive operations for procedural paper templates (N.1),
//! inserting pages anywhere (N.2), expanding and restoring margins (N.3), infinite canvas
//! whiteboards (N.5), typed text blocks (N.6) with canvas IME composition (U.8), images/stickers/
//! sticky notes (N.7), covers and sections (N.8), inter-page and deep links (N.9), quick-note
//! inbox (N.10), and vector slide condensation to note pages (L.3).

pub mod ime;
pub mod infinite;
pub mod links;
pub mod media;
pub mod packs;
pub mod pages;
pub mod paper;
pub mod planners;
pub mod quicknote;
pub mod sections;
pub mod slides;
pub mod textblocks;
pub mod versions;
