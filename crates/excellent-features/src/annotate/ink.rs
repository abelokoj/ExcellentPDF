//! A pen stroke the page canvas finished (H.1, its `ink` event): committed on pen-up as an ink annotation, one step
//! of undo. A PDF ink annotation has one width, so it takes the stroke's mean; H.9 writes the per-point widths into
//! its appearance.

use excellent_core::annot::{Annotation, Shape, Style};
use excellent_mupdf::PagePoint;
use serde_json::Value;

use super::markup::{color, floats, opacity};

/// The thinnest line an ink annotation is given, in points: a stroke too faint to see is still saved visible.
const MIN_WIDTH: f32 = 0.25;

/// A stroke the canvas finished.
#[derive(Debug, Clone, PartialEq)]
pub struct Inked {
    /// The page, counting from zero.
    pub page: usize,
    /// Its smoothed points in page points, each with the stroke's width there.
    pub points: Vec<(PagePoint, f32)>,
    /// RGB from 0 to 1.
    pub color: [f32; 3],
    /// From 0 to 1.
    pub opacity: f32,
}

impl Inked {
    /// The stroke in `value`, the event's data: `{page, points: [[x, y, width]], color: [r, g, b], opacity}`.
    /// `None` when the page or colour is missing, a point doesn't parse, or there are no points.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
        let points = value
            .get("points")?
            .as_array()?
            .iter()
            .map(|point| match floats(point)?[..] {
                [x, y, width] if width >= 0.0 => Some((PagePoint { x, y }, width)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if points.is_empty() {
            return None;
        }
        Some(Self {
            page,
            points,
            color: color(value.get("color")?)?,
            opacity: opacity(value),
        })
    }

    /// The ink annotation it makes, by `author`.
    #[must_use]
    pub fn annotation(&self, author: &str) -> Annotation {
        let mean =
            self.points.iter().map(|(_, width)| width).sum::<f32>() / self.points.len() as f32;
        let path = self.points.iter().map(|(point, _)| *point).collect();
        Annotation::new(
            self.page,
            Shape::Ink(vec![path]),
            Style {
                color: Some(self.color),
                opacity: self.opacity,
                width: mean.max(MIN_WIDTH),
            },
            author,
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_finished_stroke_becomes_one_ink_annotation() {
        let data = json!({
            "page": 2,
            "points": [[10.0, 20.0, 1.0], [12.0, 21.0, 3.0]],
            "color": [0.1, 0.2, 0.5],
        });
        let inked = Inked::parse(&data).expect("parses");
        let annotation = inked.annotation("Ada");
        assert_eq!(annotation.page, 2);
        assert_eq!(
            annotation.shape,
            Shape::Ink(vec![vec![
                PagePoint { x: 10.0, y: 20.0 },
                PagePoint { x: 12.0, y: 21.0 }
            ]])
        );
        assert_eq!(annotation.style.width, 2.0, "the mean width");
        assert_eq!(annotation.style.color, Some([0.1, 0.2, 0.5]));
        assert_eq!(annotation.style.opacity, 1.0);
    }

    #[test]
    fn a_broken_stroke_is_refused() {
        assert_eq!(
            Inked::parse(&json!({"page": 0, "points": [], "color": [0, 0, 0]})),
            None
        );
        assert_eq!(
            Inked::parse(&json!({"page": 0, "points": [[1, 2]], "color": [0, 0, 0]})),
            None
        );
        assert_eq!(
            Inked::parse(&json!({"page": 0, "points": [[1, 2, 1]]})),
            None
        );
    }
}
