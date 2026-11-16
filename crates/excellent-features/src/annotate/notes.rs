//! Notes on highlights (A.4): the text a markup carries as its PDF `/Contents`, which the shim gives a `/Popup` so
//! other readers open it in their comment panes. Which markup a click on the page opens, and the change a note's new
//! text makes. The text is plain: what the reader types, Markdown-light marks and all, is kept as typed.

use excellent_core::annot::{Annotation, Boxed, Shape, now};
use excellent_core::edit::Change;
use excellent_mupdf::PagePoint;
use serde_json::Value;

/// How far outside a markup's lines a click still opens it, in page points: a line's descenders and a hand's slip.
const SLOP: f32 = 1.5;

/// The note being read or written: the markup's page and id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Its page, counting from zero.
    pub page: usize,
    /// Its id (`/NM`).
    pub id: String,
}

/// A click the page canvas reports (its `pagePress` event: `{page, x, y}`), in page points.
#[must_use]
pub fn parse_press(data: &Value) -> Option<(usize, PagePoint)> {
    let page = usize::try_from(data.get("page")?.as_u64()?).ok()?;
    let coordinate = |key: &str| data.get(key)?.as_f64().map(|value| value as f32);
    Some((
        page,
        PagePoint {
            x: coordinate("x")?,
            y: coordinate("y")?,
        },
    ))
}

/// The markup, sticky note (A.5), text box (A.6) or line (A.8) under `point`, whose note or text a click opens: the one drawn last when several are, as
/// the page shows it on top.
#[must_use]
pub fn note_at(annotations: &[Annotation], point: PagePoint) -> Option<&Annotation> {
    let near = |bounds: excellent_mupdf::PageRect| {
        (bounds.x0 - SLOP..=bounds.x1 + SLOP).contains(&point.x)
            && (bounds.y0 - SLOP..=bounds.y1 + SLOP).contains(&point.y)
    };
    annotations
        .iter()
        .rev()
        .find(|annotation| match &annotation.shape {
            Shape::Markup(_, quads) => quads.iter().any(|quad| near(quad.bounds())),
            Shape::Boxed(Boxed::Note | Boxed::FreeText | Boxed::Typewriter, rect) => near(*rect),
            Shape::Callout(rect, _) => near(*rect),
            Shape::Line(..) | Shape::Path(..) | Shape::Boxed(Boxed::Square | Boxed::Circle, _) => {
                super::shapes::hits(annotation, point, SLOP)
            }
            _ => false,
        })
}

/// The markup a set of changes makes or extends, last of all: the one a note written straight after marking goes on.
#[must_use]
pub fn marked(changes: &[Change]) -> Option<Target> {
    changes.iter().rev().find_map(|change| match change {
        Change::CreateAnnotation(annotation) | Change::UpdateAnnotation(annotation)
            if matches!(annotation.shape, Shape::Markup(..)) =>
        {
            Some(Target {
                page: annotation.page,
                id: annotation.id.clone(),
            })
        }
        _ => None,
    })
}

/// The note as kept: line ends made `\n`, and no blank lines or spaces at its end.
#[must_use]
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end()
        .to_owned()
}

/// The change that gives `annotation` the note `text`, changed now; none when the note stays as it is.
#[must_use]
pub fn write(annotation: &Annotation, text: &str) -> Option<Change> {
    let text = normalize(text);
    (text != annotation.contents).then(|| {
        Change::UpdateAnnotation(Annotation {
            contents: text,
            modified: now(),
            ..annotation.clone()
        })
    })
}

/// The first line of a note, for a heading or a list: at most `chars` characters, an ellipsis when cut.
#[must_use]
pub fn first_line(text: &str, chars: usize) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= chars {
        return line.to_owned();
    }
    let cut: String = line.chars().take(chars.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Markup, Style};
    use excellent_mupdf::Quad;

    use super::*;

    fn point(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> Quad {
        Quad {
            ul: point(x0, y0),
            ur: point(x1, y0),
            ll: point(x0, y1),
            lr: point(x1, y1),
        }
    }

    fn markup(quads: Vec<Quad>) -> Annotation {
        Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, quads),
            Style::default(),
            "me",
        )
    }

    #[test]
    fn a_click_opens_the_note_under_it_the_top_one_first() {
        let below = markup(vec![line(10.0, 10.0, 100.0, 20.0)]);
        let above = markup(vec![
            line(50.0, 10.0, 150.0, 20.0),
            line(10.0, 22.0, 60.0, 32.0),
        ]);
        let annotations = [below.clone(), above.clone()];
        assert_eq!(
            note_at(&annotations, point(20.0, 15.0)).map(|a| &a.id),
            Some(&below.id)
        );
        assert_eq!(
            note_at(&annotations, point(60.0, 15.0)).map(|a| &a.id),
            Some(&above.id)
        );
        assert_eq!(
            note_at(&annotations, point(30.0, 31.0)).map(|a| &a.id),
            Some(&above.id)
        );
        assert_eq!(
            note_at(&annotations, point(101.0, 21.0)).map(|a| &a.id),
            Some(&above.id)
        );
        assert!(note_at(&annotations, point(200.0, 15.0)).is_none());
        let sticky = Annotation::new(
            0,
            Shape::Boxed(
                Boxed::Note,
                excellent_mupdf::PageRect {
                    x0: 300.0,
                    y0: 0.0,
                    x1: 320.0,
                    y1: 20.0,
                },
            ),
            Style::default(),
            "me",
        );
        let stamp = Annotation {
            shape: Shape::Boxed(Boxed::Stamp, Default::default()),
            ..sticky.clone()
        };
        let annotations = [sticky.clone(), stamp];
        assert_eq!(
            note_at(&annotations, point(310.0, 10.0)).map(|a| &a.id),
            Some(&sticky.id)
        );
        assert!(
            note_at(&annotations, point(0.0, 0.0)).is_none(),
            "no note on a stamp"
        );
    }

    #[test]
    fn a_note_is_written_as_one_change_only_when_it_changes() {
        let highlight = markup(vec![line(0.0, 0.0, 10.0, 10.0)]);
        let Some(Change::UpdateAnnotation(noted)) =
            write(&highlight, "Check this.\r\nAnd this.\n\n")
        else {
            panic!("a new note is a change");
        };
        assert_eq!(noted.contents, "Check this.\nAnd this.");
        assert_eq!(noted.id, highlight.id);
        assert_eq!(noted.shape, highlight.shape);
        assert!(
            write(&noted, "Check this.\nAnd this.  ").is_none(),
            "the same note"
        );
        let Some(Change::UpdateAnnotation(cleared)) = write(&noted, "  \n") else {
            panic!("clearing is a change");
        };
        assert!(cleared.contents.is_empty());
    }

    #[test]
    fn the_markup_just_made_is_the_one_noted() {
        let first = markup(vec![line(0.0, 0.0, 10.0, 10.0)]);
        let second = Annotation {
            page: 2,
            ..markup(vec![line(0.0, 0.0, 10.0, 10.0)])
        };
        let changes = [
            Change::CreateAnnotation(first),
            Change::UpdateAnnotation(second.clone()),
            Change::DeleteAnnotation {
                page: 2,
                id: "folded".to_owned(),
            },
        ];
        assert_eq!(
            marked(&changes),
            Some(Target {
                page: 2,
                id: second.id
            })
        );
        assert_eq!(marked(&[]), None);
    }

    #[test]
    fn the_first_line_is_cut_at_a_whole_character() {
        assert_eq!(first_line("\n  Short one \nmore", 20), "Short one");
        assert_eq!(first_line("ééééééééé", 5), "éééé…");
        assert_eq!(first_line("", 5), "");
    }

    #[test]
    fn a_press_is_read() {
        let data = serde_json::json!({ "page": 3, "x": 12.5, "y": 40 });
        assert_eq!(parse_press(&data), Some((3, point(12.5, 40.0))));
        assert_eq!(
            parse_press(&serde_json::json!({ "page": -1, "x": 0, "y": 0 })),
            None
        );
    }
}
