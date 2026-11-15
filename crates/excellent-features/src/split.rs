//! The split view's models (plan §11.15). V.8, synchronized scrolling: where a pane that follows another goes when
//! the other moves, by the same page, the same position through the document, or pages the reader paired.

/// How a following pane matches the pane the reader moves (V.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Follow {
    /// The same page number, at the same place on the page.
    #[default]
    SamePage,
    /// The same distance through the document, measured in page heights: documents of different page sizes and
    /// lengths stay level.
    SamePosition,
    /// The same number of pages on from the nearest pair of pages the reader marked as matching.
    Pairs,
}

impl Follow {
    /// Every way, in the order menus list them.
    pub const ALL: [Self; 3] = [Self::SamePage, Self::SamePosition, Self::Pairs];

    /// The way `name` names (`"page"`, `"position"`, `"pairs"`); the default for anything else.
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("position") => Self::SamePosition,
            Some("pairs") => Self::Pairs,
            _ => Self::SamePage,
        }
    }

    /// Its name in `settings.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Self::SamePage => "page",
            Self::SamePosition => "position",
            Self::Pairs => "pairs",
        }
    }
}

/// Where a pane shows: the first page on screen, and how far the view's top-left is into it, in page widths and
/// heights (the canvas's anchor).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    /// The page, from 0.
    pub page: usize,
    /// Across the page, in page widths.
    pub across: f32,
    /// Down the page, in page heights.
    pub down: f32,
}

/// Where a pane whose pages are `to` (each page's height) goes to follow one at `from` whose pages are `from_pages`
/// (V.8); `pairs` are the matching pages the reader marked, `(from page, to page)`. `None` when `to` has no pages.
pub fn follow(
    how: Follow,
    from: Position,
    from_pages: &[f32],
    to: &[f32],
    pairs: &[(usize, usize)],
) -> Option<Position> {
    let last = to.len().checked_sub(1)?;
    let page = match how {
        Follow::SamePosition => {
            return Some(same_position(from, from_pages, to).unwrap_or(Position {
                page: from.page.min(last),
                ..from
            }));
        }
        Follow::SamePage => from.page,
        Follow::Pairs => paired(from.page, pairs),
    };
    Some(Position {
        page: page.min(last),
        ..from
    })
}

/// The page `page` matches by the nearest pair at or before it (else the first pair); `page` itself with none.
fn paired(page: usize, pairs: &[(usize, usize)]) -> usize {
    let before = pairs
        .iter()
        .filter(|(from, _)| *from <= page)
        .max_by_key(|(from, _)| *from);
    let nearest = before.or_else(|| pairs.iter().min_by_key(|(from, _)| *from));
    match nearest {
        Some(&(from, to)) => (to as i64 + page as i64 - from as i64).max(0) as usize,
        None => page,
    }
}

/// The same distance through `to` as `from` is through `from_pages`, as a share of each one's whole height; `None`
/// when either has no height.
fn same_position(from: Position, from_pages: &[f32], to: &[f32]) -> Option<Position> {
    let height = |pages: &[f32]| pages.iter().map(|h| h.max(0.0)).sum::<f32>();
    let (from_total, to_total) = (height(from_pages), height(to));
    if from_total <= 0.0 || to_total <= 0.0 {
        return None;
    }
    let page = from.page.min(from_pages.len().checked_sub(1)?);
    let into = height(&from_pages[..page]) + from.down.clamp(0.0, 1.0) * from_pages[page].max(0.0);
    let mut left = into / from_total * to_total;
    for (page, h) in to.iter().map(|h| h.max(0.0)).enumerate() {
        if left < h || page + 1 == to.len() {
            let down = if h > 0.0 { (left / h).min(1.0) } else { 0.0 };
            return Some(Position {
                page,
                across: from.across,
                down,
            });
        }
        left -= h;
    }
    None
}

/// The pages the reader paired across panes (V.8, matched page pairs): each mark is every pane's page at the time,
/// `None` for a pane not open then.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Marks(Vec<Vec<Option<usize>>>);

impl Marks {
    /// Pairs the pages the panes show now (by pane), replacing a mark at the same page of the first pane open.
    pub fn mark(&mut self, pages: Vec<Option<usize>>) {
        let first = |mark: &[Option<usize>]| {
            mark.iter()
                .position(Option::is_some)
                .map(|pane| (pane, mark[pane]))
        };
        if pages.iter().flatten().count() < 2 {
            return;
        }
        let key = first(&pages);
        self.0.retain(|mark| first(mark) != key);
        self.0.push(pages);
    }

    /// The pairs from pane `from` to pane `to`, `(from page, to page)`, in the order of `from`'s pages.
    pub fn pairs(&self, from: usize, to: usize) -> Vec<(usize, usize)> {
        let mut pairs: Vec<(usize, usize)> = self
            .0
            .iter()
            .filter_map(|mark| Some((*mark.get(from)?.as_ref()?, *mark.get(to)?.as_ref()?)))
            .collect();
        pairs.sort_unstable();
        pairs.dedup_by_key(|pair| pair.0);
        pairs
    }

    /// How many marks there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Forgets them all.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(page: usize, down: f32) -> Position {
        Position {
            page,
            across: 0.1,
            down,
        }
    }

    #[test]
    fn same_position_stays_level_across_different_page_sizes() {
        // A letter-size paper of four pages beside a book of five uneven pages: halfway down the third page is
        // 62.5 % through, which in the book is the very top of its fifth page.
        let paper = [792.0; 4];
        let book = [200.0, 50.0, 50.0, 200.0, 300.0];
        let there = follow(Follow::SamePosition, at(2, 0.5), &paper, &book, &[]).expect("pages");
        assert_eq!(there, at(4, 0.0));
        // A quarter through each way, and the end stays the end.
        let there = follow(Follow::SamePosition, at(1, 0.0), &paper, &book, &[]).expect("pages");
        assert_eq!(there, at(1, 0.0));
        let there = follow(Follow::SamePosition, at(3, 1.0), &paper, &book, &[]).expect("pages");
        assert_eq!(there, at(4, 1.0));
        // And back: the book's top of page 5 is the paper's middle of page 3.
        let back = follow(Follow::SamePosition, at(4, 0.0), &book, &paper, &[]).expect("pages");
        assert_eq!(back, at(2, 0.5));
    }

    #[test]
    fn same_page_keeps_the_place_on_the_page_and_stops_at_the_last() {
        let (a, b) = ([792.0; 10], [300.0; 4]);
        assert_eq!(
            follow(Follow::SamePage, at(2, 0.3), &a, &b, &[]),
            Some(at(2, 0.3))
        );
        assert_eq!(
            follow(Follow::SamePage, at(8, 0.3), &a, &b, &[]),
            Some(at(3, 0.3))
        );
        assert_eq!(follow(Follow::SamePage, at(0, 0.0), &a, &[], &[]), None);
    }

    #[test]
    fn pairs_count_on_from_the_nearest_pair() {
        let (a, b) = ([792.0; 20], [792.0; 20]);
        let pairs = [(0, 0), (4, 7)];
        let go = |page| follow(Follow::Pairs, at(page, 0.5), &a, &b, &pairs).map(|p| p.page);
        assert_eq!(go(2), Some(2));
        assert_eq!(go(4), Some(7));
        assert_eq!(go(10), Some(13));
        assert_eq!(go(18), Some(19));
        // Before the first pair, it counts back from it; with none, it is the same page.
        let late = [(5, 2)];
        assert_eq!(
            follow(Follow::Pairs, at(1, 0.0), &a, &b, &late).map(|p| p.page),
            Some(0)
        );
        assert_eq!(
            follow(Follow::Pairs, at(6, 0.0), &a, &b, &[]).map(|p| p.page),
            Some(6)
        );
    }

    #[test]
    fn marks_pair_the_panes_open_at_the_time() {
        let mut marks = Marks::default();
        marks.mark(vec![Some(1), Some(4), None, None]);
        marks.mark(vec![Some(9), None, Some(2), None]);
        marks.mark(vec![Some(3), None, None, None]);
        assert_eq!(marks.len(), 2, "a mark needs two panes");
        assert_eq!(marks.pairs(0, 1), [(1, 4)]);
        assert_eq!(marks.pairs(1, 0), [(4, 1)]);
        assert_eq!(marks.pairs(0, 2), [(9, 2)]);
        assert!(marks.pairs(1, 2).is_empty());
        // Marking the first pane's page again moves its pair.
        marks.mark(vec![Some(1), Some(6), None, None]);
        assert_eq!(marks.pairs(0, 1), [(1, 6)]);
        marks.clear();
        assert!(marks.is_empty());
        assert_eq!(Follow::parse(Some(Follow::Pairs.name())), Follow::Pairs);
        assert_eq!(Follow::parse(Some("nonsense")), Follow::SamePage);
    }
}
