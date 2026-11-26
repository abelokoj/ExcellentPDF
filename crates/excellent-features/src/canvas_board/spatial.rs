//! Spatial grid index for fast viewport queries and hit-testing on large canvas boards (plan X.1).

use std::collections::{BTreeMap, BTreeSet};

use super::BoardRect;

/// Default spatial cell dimension in world units (points).
pub const DEFAULT_CELL_SIZE: f64 = 512.0;

/// Uniform spatial hash grid indexing axis-aligned [`BoardRect`] bounds for 1,000+ board cards.
#[derive(Debug, Clone)]
pub struct SpatialGridIndex {
    cell_size: f64,
    cells: BTreeMap<(i64, i64), BTreeSet<String>>,
    bounds_by_id: BTreeMap<String, BoardRect>,
}

impl Default for SpatialGridIndex {
    fn default() -> Self {
        Self::new(DEFAULT_CELL_SIZE)
    }
}

impl SpatialGridIndex {
    /// Creates an empty spatial grid with `cell_size` world units per bucket.
    ///
    /// Non-finite or non-positive sizes fall back to [`DEFAULT_CELL_SIZE`].
    #[must_use]
    pub fn new(cell_size: f64) -> Self {
        let valid = if cell_size.is_finite() && cell_size > 1.0 {
            cell_size
        } else {
            DEFAULT_CELL_SIZE
        };
        Self {
            cell_size: valid,
            cells: BTreeMap::new(),
            bounds_by_id: BTreeMap::new(),
        }
    }

    /// Number of indexed items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bounds_by_id.len()
    }

    /// Whether the index holds zero items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bounds_by_id.is_empty()
    }

    /// Inserts or updates `id` at `bounds`.
    pub fn upsert(&mut self, id: &str, bounds: BoardRect) {
        self.remove(id);
        let range = self.cell_range(&bounds);
        for gx in range.0..=range.2 {
            for gy in range.1..=range.3 {
                self.cells
                    .entry((gx, gy))
                    .or_default()
                    .insert(id.to_owned());
            }
        }
        self.bounds_by_id.insert(id.to_owned(), bounds);
    }

    /// Removes `id` from the index, returning its previous bounds if present.
    pub fn remove(&mut self, id: &str) -> Option<BoardRect> {
        let prev = self.bounds_by_id.remove(id)?;
        let range = self.cell_range(&prev);
        for gx in range.0..=range.2 {
            for gy in range.1..=range.3 {
                if let Some(bucket) = self.cells.get_mut(&(gx, gy)) {
                    bucket.remove(id);
                    if bucket.is_empty() {
                        self.cells.remove(&(gx, gy));
                    }
                }
            }
        }
        Some(prev)
    }

    /// Returns sorted item IDs whose bounds intersect `viewport`.
    #[must_use]
    pub fn query_rect(&self, viewport: &BoardRect) -> Vec<String> {
        let range = self.cell_range(viewport);
        let mut candidates = BTreeSet::new();
        for gx in range.0..=range.2 {
            for gy in range.1..=range.3 {
                if let Some(bucket) = self.cells.get(&(gx, gy)) {
                    for id in bucket {
                        candidates.insert(id.as_str());
                    }
                }
            }
        }
        candidates
            .into_iter()
            .filter(|id| {
                self.bounds_by_id
                    .get(*id)
                    .is_some_and(|rect| rect.intersects(viewport))
            })
            .map(str::to_owned)
            .collect()
    }

    /// Returns sorted item IDs containing world point `(x, y)`.
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Vec<String> {
        let probe = BoardRect {
            x,
            y,
            width: 0.0,
            height: 0.0,
        };
        let (gx, gy) = (self.cell_coord(x), self.cell_coord(y));
        let Some(bucket) = self.cells.get(&(gx, gy)) else {
            return Vec::new();
        };
        bucket
            .iter()
            .filter(|id| {
                self.bounds_by_id
                    .get(id.as_str())
                    .is_some_and(|rect| rect.contains_point(probe.x, probe.y))
            })
            .cloned()
            .collect()
    }

    /// Returns the bounding box enclosing all indexed items, or `None` when empty.
    #[must_use]
    pub fn world_bounds(&self) -> Option<BoardRect> {
        let mut iter = self.bounds_by_id.values();
        let first = *iter.next()?;
        Some(iter.fold(first, |acc, rect| acc.union(*rect)))
    }

    fn cell_coord(&self, value: f64) -> i64 {
        if !value.is_finite() {
            return 0;
        }
        (value / self.cell_size).floor() as i64
    }

    fn cell_range(&self, rect: &BoardRect) -> (i64, i64, i64, i64) {
        let norm = rect.normalized();
        (
            self.cell_coord(norm.x),
            self.cell_coord(norm.y),
            self.cell_coord(norm.x + norm.width),
            self.cell_coord(norm.y + norm.height),
        )
    }
}
