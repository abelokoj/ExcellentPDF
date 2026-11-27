//! Margin notes (A.11): each note shown as a card in a column beside the pages, level with what it is about, and the
//! cards pushed apart when they would overlap. Pure: where each note's annotation is on screen, and where its card
//! goes. The canvas says how a page's points map to the screen ([`ToScreen`]); the app draws the cards.

use ecclesia_core::annot::{Annotation, Shape};
use ecclesia_mupdf::{PagePoint, PageRect};
use serde_json::Value;

/// How a page's points map to the screen: `x' = a x + c y + e`, `y' = b x + d y + f`, in the app's pixels from the
/// canvas's top-left corner. Any zoom, scroll and rotation is one of these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToScreen {
    /// The matrix's six numbers, PDF's order.
    pub m: [f32; 6],
}

impl ToScreen {
    /// The map that takes the page's origin to `origin`, and one point along its x and y axes to `x` and `y`.
    #[must_use]
    pub fn through(origin: (f32, f32), x: (f32, f32), y: (f32, f32)) -> Self {
        Self {
            m: [
                x.0 - origin.0,
                x.1 - origin.1,
                y.0 - origin.0,
                y.1 - origin.1,
                origin.0,
                origin.1,
            ],
        }
    }

    /// Where page point `p` is on screen.
    #[must_use]
    pub fn apply(self, p: PagePoint) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.m;
        (a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// As JSON: `[a, b, c, d, e, f]`.
    #[must_use]
    pub fn to_json(self) -> Value {
        Value::from(self.m.to_vec())
    }

    /// Read from [`Self::to_json`]'s form; `None` for anything else.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let numbers = value.as_array()?;
        if numbers.len() != 6 {
            return None;
        }
        let mut m = [0.0; 6];
        for (slot, number) in m.iter_mut().zip(numbers) {
            *slot = number.as_f64()? as f32;
        }
        Some(Self { m })
    }
}

/// Whether `annotation` gets a card: it has a note. A text box's text is on the page already.
#[must_use]
pub fn has_card(annotation: &Annotation) -> bool {
    !annotation.contents.trim().is_empty() && !annotation.is_text_box()
}

/// The points that outline an annotation, which its card lines up with, in page points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline(Vec<PagePoint>);

impl Outline {
    /// Its top on screen under `to_screen`; `None` for an empty outline.
    #[must_use]
    pub fn top(&self, to_screen: ToScreen) -> Option<f32> {
        top(&self.0, to_screen)
    }

    /// The smallest rectangle holding it; `None` for an empty outline.
    #[must_use]
    pub fn bounds(&self) -> Option<PageRect> {
        let first = self.0.first()?;
        Some(self.0.iter().fold(
            PageRect {
                x0: first.x,
                y0: first.y,
                x1: first.x,
                y1: first.y,
            },
            |r, p| PageRect {
                x0: r.x0.min(p.x),
                y0: r.y0.min(p.y),
                x1: r.x1.max(p.x),
                y1: r.y1.max(p.y),
            },
        ))
    }
}

/// The outline of `shape`.
#[must_use]
pub fn outline(shape: &Shape) -> Outline {
    Outline(points(shape))
}

/// The points that outline `shape`.
fn points(shape: &Shape) -> Vec<PagePoint> {
    let corners = |rect: &PageRect| {
        [
            PagePoint {
                x: rect.x0,
                y: rect.y0,
            },
            PagePoint {
                x: rect.x1,
                y: rect.y0,
            },
            PagePoint {
                x: rect.x0,
                y: rect.y1,
            },
            PagePoint {
                x: rect.x1,
                y: rect.y1,
            },
        ]
    };
    match shape {
        Shape::Markup(_, quads) => quads
            .iter()
            .flat_map(|quad| [quad.ul, quad.ur, quad.ll, quad.lr])
            .collect(),
        Shape::Boxed(_, rect) => corners(rect).to_vec(),
        Shape::Ink(strokes) => strokes.iter().flatten().copied().collect(),
        Shape::Line(from, to) => vec![*from, *to],
        Shape::Path(_, points) => points.clone(),
        Shape::Callout(rect, tip) => {
            let mut points = corners(rect).to_vec();
            points.push(*tip);
            points
        }
    }
}

/// The top of `points` on screen under `to_screen`; `None` for no points.
#[must_use]
pub fn top(points: &[PagePoint], to_screen: ToScreen) -> Option<f32> {
    points
        .iter()
        .map(|&point| to_screen.apply(point).1)
        .reduce(f32::min)
}

/// About how many lines `text` takes at `per_line` characters a line, from 1 to `max`: each paragraph wraps on its
/// own. An estimate: the card clips what doesn't fit.
#[must_use]
pub fn lines(text: &str, per_line: usize, max: usize) -> usize {
    let per_line = per_line.max(1);
    let wrapped: usize = text
        .trim()
        .lines()
        .map(|line| line.chars().count().div_ceil(per_line).max(1))
        .sum();
    wrapped.clamp(1, max.max(1))
}

/// A card to lay out: the screen height of what it is about, and its own height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card {
    /// Where it would like its top: level with its annotation.
    pub anchor: f32,
    /// Its height.
    pub height: f32,
}

/// Cards in a row that touch: they move together, centred on what they are about.
struct Cluster {
    /// Its first card's place in the sorted order.
    first: usize,
    /// Its cards' heights and the gaps between them.
    height: f32,
    /// The sum over its cards of where each would put the cluster's top.
    sum: f32,
    count: f32,
    top: f32,
}

/// Where each card's top goes (in `cards`' order): as near its anchor as it can be, no two closer than `gap`, in
/// the order of their anchors, none above `ceiling`. Cards that would overlap are pushed apart round the middle of
/// what they are about, so a crowd spreads both ways rather than all downwards. Linear after the sort.
#[must_use]
pub fn layout(cards: &[Card], gap: f32, ceiling: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by(|&i, &j| cards[i].anchor.total_cmp(&cards[j].anchor).then(i.cmp(&j)));
    let mut clusters: Vec<Cluster> = Vec::new();
    for (at, &i) in order.iter().enumerate() {
        let card = cards[i];
        let mut next = Cluster {
            first: at,
            height: card.height,
            sum: card.anchor,
            count: 1.0,
            top: card.anchor.max(ceiling),
        };
        while let Some(last) = clusters.last() {
            if last.top + last.height + gap <= next.top {
                break;
            }
            let last = clusters.pop().expect("a cluster was just seen");
            // Every card of `next` sits `shift` further down in the merged cluster than in its own.
            let shift = last.height + gap;
            let sum = last.sum + next.sum - next.count * shift;
            let count = last.count + next.count;
            next = Cluster {
                first: last.first,
                height: shift + next.height,
                sum,
                count,
                top: (sum / count).max(ceiling),
            };
        }
        clusters.push(next);
    }
    let mut tops = vec![0.0; cards.len()];
    let ends = clusters
        .iter()
        .skip(1)
        .map(|cluster| cluster.first)
        .chain([order.len()]);
    for (cluster, end) in clusters.iter().zip(ends) {
        let mut y = cluster.top;
        for &i in &order[cluster.first..end] {
            tops[i] = y;
            y += cards[i].height + gap;
        }
    }
    tops
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn card(anchor: f32, height: f32) -> Card {
        Card { anchor, height }
    }

    /// No two cards closer than `gap`, in their anchors' order, none above `ceiling`.
    fn assert_laid_out(cards: &[Card], tops: &[f32], gap: f32, ceiling: f32) {
        let mut order: Vec<usize> = (0..cards.len()).collect();
        order.sort_by(|&i, &j| cards[i].anchor.total_cmp(&cards[j].anchor).then(i.cmp(&j)));
        for pair in order.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                tops[a] + cards[a].height + gap <= tops[b] + 1e-3,
                "cards {a} and {b} overlap: {} + {} and {}",
                tops[a],
                cards[a].height,
                tops[b]
            );
        }
        assert!(tops.iter().all(|&top| top >= ceiling - 1e-3));
    }

    #[test]
    fn a_card_alone_sits_level_with_its_note() {
        let cards = [card(100.0, 40.0), card(300.0, 40.0)];
        assert_eq!(layout(&cards, 8.0, 0.0), vec![100.0, 300.0]);
        assert_eq!(
            layout(&[card(-50.0, 40.0)], 8.0, 0.0),
            vec![0.0],
            "not above the column"
        );
        assert!(layout(&[], 8.0, 0.0).is_empty());
    }

    #[test]
    fn overlapping_cards_spread_both_ways_round_their_notes() {
        let cards = [card(110.0, 40.0), card(100.0, 40.0)];
        let tops = layout(&cards, 10.0, 0.0);
        assert_laid_out(&cards, &tops, 10.0, 0.0);
        // The first by anchor goes up as far as the second goes down.
        assert!(
            (tops[1] - 80.0).abs() < 1e-3 && (tops[0] - 130.0).abs() < 1e-3,
            "{tops:?}"
        );
    }

    #[test]
    fn two_hundred_notes_on_one_page_lay_out_without_overlap_within_a_frame() {
        let mut seed: u32 = 0x2545_f491;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let cards: Vec<Card> = (0..200)
            .map(|_| card(next() * 1000.0, 40.0 + next() * 60.0))
            .collect();
        let started = Instant::now();
        let tops = layout(&cards, 6.0, 0.0);
        let took = started.elapsed();
        assert_laid_out(&cards, &tops, 6.0, 0.0);
        assert!(took.as_secs_f64() < 1.0 / 60.0, "took {took:?}");
    }

    #[test]
    fn a_card_lines_up_with_the_top_of_its_annotation_under_any_rotation() {
        let points = [
            PagePoint { x: 10.0, y: 50.0 },
            PagePoint { x: 30.0, y: 20.0 },
        ];
        let plain = ToScreen::through((0.0, 100.0), (2.0, 100.0), (0.0, 102.0));
        assert_eq!(top(&points, plain), Some(140.0));
        // Turned a quarter clockwise: the page's x runs down the screen.
        let turned = ToScreen::through((500.0, 0.0), (500.0, 1.0), (499.0, 0.0));
        assert_eq!(top(&points, turned), Some(10.0));
        assert_eq!(top(&[], plain), None);
        assert_eq!(ToScreen::from_json(&plain.to_json()), Some(plain));
        assert_eq!(ToScreen::from_json(&Value::from(vec![1.0, 2.0])), None);
    }

    #[test]
    fn a_note_s_lines_are_counted_by_paragraph_and_capped() {
        assert_eq!(lines("", 30, 4), 1);
        assert_eq!(lines("short", 30, 4), 1);
        assert_eq!(lines(&"x".repeat(61), 30, 4), 3);
        assert_eq!(lines("a\nb", 30, 4), 2);
        assert_eq!(lines(&"word ".repeat(100), 30, 4), 4);
    }
}
