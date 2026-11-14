//! Tiles (plan §7.4, K.7's first part): which 256-pixel squares of which pages the viewport needs, and the cache that
//! holds them while they render and after.
//!
//! Tiles render at a zoom *bucket*: the scale in steps of 1/4096, so a tile is drawn at the size it was rendered
//! and text stays sharp (coarser buckets, stretched to the exact scale, blur it). Zooming reuses tiles another way:
//! while the new zoom's tiles render, the canvas draws the old zoom's under them, stretched, so a page never goes
//! blank (plan §7.4, item 4).
//!
//! Still to come with K.7: tiles for the next screen at lower priority, display-list budgets, eviction on OS memory
//! pressure, and the low-resolution first pass.

use std::collections::{HashMap, HashSet};

use crate::Rect;
use crate::layout::Layout;
use crate::viewport::Viewport;

/// A tile's side, in device pixels.
pub const TILE: i32 = 256;

/// Buckets per unit of scale.
const STEPS: f32 = 4096.0;

/// The bucket a scale (device pixels per point) renders at.
pub fn bucket(scale: f32) -> i32 {
    // Scales stay within the zoom limits (a few hundred at most), so this fits an i32 with room to spare.
    (scale * STEPS).round().max(1.0) as i32
}

/// The scale of bucket `bucket`.
pub fn bucket_scale(bucket: i32) -> f32 {
    bucket as f32 / STEPS
}

/// A rectangle of whole pixels, `x1` and `y1` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PixelRect {
    /// Left edge.
    pub x0: i32,
    /// Top edge.
    pub y0: i32,
    /// Right edge (exclusive).
    pub x1: i32,
    /// Bottom edge (exclusive).
    pub y1: i32,
}

/// One tile: a page, a bucket and its place in the page's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// The page's index.
    pub page: usize,
    /// The zoom bucket it renders at.
    pub bucket: i32,
    /// Its column in the page's grid.
    pub col: i32,
    /// Its row.
    pub row: i32,
}

impl TileKey {
    /// The pixels it covers, in the page's pixels at its bucket's scale, cut at the page's edge. `page` is the page's
    /// rectangle in layout points.
    pub fn area(&self, page: &Rect) -> PixelRect {
        let s = bucket_scale(self.bucket);
        let (w, h) = page_pixels(page, s);
        PixelRect {
            x0: self.col * TILE,
            y0: self.row * TILE,
            x1: ((self.col + 1) * TILE).min(w),
            y1: ((self.row + 1) * TILE).min(h),
        }
    }

    /// Where it is drawn now, in device pixels; edges on whole pixels, so neighbours meet without seams.
    pub fn on_screen(&self, layout: &Layout, viewport: &Viewport) -> Option<Rect> {
        let page = layout.page(self.page)?;
        let area = self.area(&page);
        let ratio = viewport.scale() / bucket_scale(self.bucket);
        let origin = viewport.to_screen(&page);
        Some(Rect {
            x0: (origin.x0 + area.x0 as f32 * ratio).round(),
            y0: (origin.y0 + area.y0 as f32 * ratio).round(),
            x1: (origin.x0 + area.x1 as f32 * ratio).round().min(origin.x1),
            y1: (origin.y0 + area.y1 as f32 * ratio).round().min(origin.y1),
        })
    }
}

/// A page's size in whole pixels at `scale`.
fn page_pixels(page: &Rect, scale: f32) -> (i32, i32) {
    // Page sizes are a few thousand points at most, times the largest zoom; this fits an i32.
    (
        (page.width() * scale).ceil() as i32,
        (page.height() * scale).ceil() as i32,
    )
}

/// The tiles the viewport shows at its current bucket, page by page, top to bottom.
pub fn visible_tiles(layout: &Layout, viewport: &Viewport) -> Vec<TileKey> {
    let bucket = bucket(viewport.scale());
    let s = bucket_scale(bucket);
    let visible = viewport.visible();
    let mut tiles = Vec::new();
    for (index, page) in layout.pages_in(&visible) {
        let Some(part) = page.intersect(&visible) else {
            continue;
        };
        let (w, h) = page_pixels(&page, s);
        let x0 = (((part.x0 - page.x0) * s).floor() as i32).clamp(0, w);
        let y0 = (((part.y0 - page.y0) * s).floor() as i32).clamp(0, h);
        let x1 = (((part.x1 - page.x0) * s).ceil() as i32).clamp(0, w);
        let y1 = (((part.y1 - page.y0) * s).ceil() as i32).clamp(0, h);
        for row in y0 / TILE..(y1 + TILE - 1) / TILE {
            for col in x0 / TILE..(x1 + TILE - 1) / TILE {
                tiles.push(TileKey {
                    page: index,
                    bucket,
                    col,
                    row,
                });
            }
        }
    }
    tiles
}

/// A tile being rendered (its job, `J`) or rendered (its bitmap, `B`).
#[derive(Debug)]
enum Slot<J, B> {
    Pending(J),
    Ready { bitmap: B, bytes: usize, used: u64 },
}

/// The tiles of one document. Dropping a pending tile's job cancels it.
#[derive(Debug)]
pub struct TileCache<J, B> {
    slots: HashMap<TileKey, Slot<J, B>>,
    frame: u64,
    bytes: usize,
    budget: usize,
}

impl<J, B> TileCache<J, B> {
    /// An empty cache that keeps at most `budget` bytes of bitmaps it no longer draws.
    pub fn new(budget: usize) -> Self {
        Self {
            slots: HashMap::new(),
            frame: 0,
            bytes: 0,
            budget,
        }
    }

    /// Starts a frame: what is drawn from now on counts as used in it.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// True when `key` is rendered or rendering.
    pub fn contains(&self, key: &TileKey) -> bool {
        self.slots.contains_key(key)
    }

    /// `key` is rendering through `job`.
    pub fn request(&mut self, key: TileKey, job: J) {
        self.slots.insert(key, Slot::Pending(job));
    }

    /// `key`'s bitmap, if it is rendered; it counts as used in this frame.
    pub fn ready(&mut self, key: &TileKey) -> Option<&B> {
        match self.slots.get_mut(key) {
            Some(Slot::Ready { bitmap, used, .. }) => {
                *used = self.frame;
                Some(bitmap)
            }
            _ => None,
        }
    }

    /// The rendered tiles of `page` at another bucket than `bucket`, nearest bucket last so it is drawn on top.
    pub fn others_of_page(&self, page: usize, bucket: i32) -> Vec<TileKey> {
        let mut keys: Vec<TileKey> = self
            .slots
            .iter()
            .filter(|(k, slot)| {
                k.page == page && k.bucket != bucket && matches!(slot, Slot::Ready { .. })
            })
            .map(|(k, _)| *k)
            .collect();
        keys.sort_by_key(|k| std::cmp::Reverse((k.bucket - bucket).abs()));
        keys
    }

    /// Asks `take` about every pending job: `Some(Some((bitmap, bytes)))` when it rendered, `Some(None)` when it
    /// failed or was cancelled (the slot is dropped), `None` while it runs. True when anything changed.
    pub fn poll(&mut self, mut take: impl FnMut(&mut J) -> Option<Option<(B, usize)>>) -> bool {
        let frame = self.frame;
        let mut changed = false;
        let mut failed = Vec::new();
        for (key, slot) in &mut self.slots {
            let Slot::Pending(job) = slot else { continue };
            match take(job) {
                Some(Some((bitmap, bytes))) => {
                    self.bytes += bytes;
                    *slot = Slot::Ready {
                        bitmap,
                        bytes,
                        used: frame,
                    };
                    changed = true;
                }
                Some(None) => failed.push(*key),
                None => {}
            }
        }
        for key in failed {
            self.slots.remove(&key);
            changed = true;
        }
        changed
    }

    /// Drops (and so cancels) the pending tiles not in `needed`: their pages left the viewport, or the zoom moved on.
    pub fn cancel_unneeded(&mut self, needed: &HashSet<TileKey>) {
        self.slots
            .retain(|key, slot| matches!(slot, Slot::Ready { .. }) || needed.contains(key));
    }

    /// Drops the least recently drawn bitmaps until the ones not drawn in this frame fit the budget.
    pub fn evict(&mut self) {
        if self.bytes <= self.budget {
            return;
        }
        let mut idle: Vec<(u64, TileKey, usize)> = self
            .slots
            .iter()
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { used, bytes, .. } if *used < self.frame => {
                    Some((*used, *key, *bytes))
                }
                _ => None,
            })
            .collect();
        idle.sort_unstable_by_key(|(used, ..)| *used);
        for (_, key, bytes) in idle {
            if self.bytes <= self.budget {
                break;
            }
            self.slots.remove(&key);
            self.bytes -= bytes;
        }
    }

    /// Drops everything: another document, or the bitmaps' device was lost.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.bytes = 0;
    }

    /// Bytes of rendered bitmaps held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PageSize;

    fn key(page: usize, bucket: i32) -> TileKey {
        TileKey {
            page,
            bucket,
            col: 0,
            row: 0,
        }
    }

    #[test]
    fn buckets_are_close_enough_to_draw_unstretched() {
        for scale in [0.1_f32, 0.75, 1.0, 1.333, 2.0, 7.3, 80.0] {
            let s = bucket_scale(bucket(scale));
            // Off by less than half a pixel across a whole tile.
            assert!(
                (s / scale - 1.0).abs() * TILE as f32 <= 0.5,
                "{scale} -> {s}"
            );
        }
        assert_eq!(bucket(1.0), 4096);
        assert_eq!(bucket(2.0), 8192);
    }

    #[test]
    fn visible_tiles_cover_the_screen_and_meet_without_gaps() {
        let layout = Layout::continuous(&[PageSize {
            width: 612.0,
            height: 792.0,
        }]);
        let mut v = Viewport::new(1.0);
        v.resize(700.0, 500.0, &layout);
        let tiles = visible_tiles(&layout, &v);
        // Scale 1: the page is 612 × 792 pixels; 500 rows show (minus the top margin).
        assert!(tiles.iter().all(|t| t.bucket == 4096 && t.page == 0));
        assert_eq!(tiles.iter().map(|t| t.col).max(), Some(2));
        assert_eq!(tiles.iter().map(|t| t.row).max(), Some(1));
        let a = tiles[0].on_screen(&layout, &v).expect("on screen");
        let b = tiles[1].on_screen(&layout, &v).expect("on screen");
        assert_eq!(a.x1, b.x0, "neighbours meet");
        let page = layout.page(0).expect("page");
        let last = TileKey {
            page: 0,
            bucket: 4096,
            col: 2,
            row: 0,
        };
        assert_eq!(last.area(&page).x1, 612, "cut at the page's edge");
    }

    #[test]
    fn the_cache_cancels_what_is_not_needed_and_evicts_the_oldest() {
        let mut cache: TileCache<&'static str, u8> = TileCache::new(100);
        for (page, job) in [(0, "a"), (1, "b"), (2, "c")] {
            cache.request(key(page, 0), job);
        }
        cache.cancel_unneeded(&[key(0, 0), key(1, 0)].into_iter().collect());
        assert!(
            !cache.contains(&key(2, 0)),
            "a pending tile no longer needed is dropped"
        );

        cache.begin_frame();
        assert!(cache.poll(|job| match *job {
            "a" => Some(Some((1, 80))),
            _ => Some(Some((2, 80))),
        }));
        assert_eq!(cache.bytes(), 160);
        cache.begin_frame();
        assert!(cache.ready(&key(1, 0)).is_some(), "drawn in this frame");
        cache.evict();
        assert!(!cache.contains(&key(0, 0)), "the idle tile went");
        assert!(cache.contains(&key(1, 0)), "the drawn one stayed");
        assert_eq!(cache.bytes(), 80);
    }

    #[test]
    fn other_buckets_nearest_last_and_failures_drop_the_slot() {
        let mut cache: TileCache<bool, u8> = TileCache::new(usize::MAX);
        cache.request(key(0, 2), true);
        cache.request(key(0, 7), true);
        cache.request(key(0, 4), false);
        cache.poll(|ok| if *ok { Some(Some((0, 1))) } else { Some(None) });
        assert!(!cache.contains(&key(0, 4)));
        let others: Vec<i32> = cache
            .others_of_page(0, 5)
            .iter()
            .map(|k| k.bucket)
            .collect();
        assert_eq!(others, [2, 7]);
    }
}
