//! Sticky notes (A.5): a PDF text annotation, a small icon at a point on the page that carries a note. Its icon is
//! one of the seven the PDF standard names, so every reader draws it; whether other readers show its note open is
//! its `/Open`. The note's text is written as A.4's (`notes`).

use excellent_core::annot::{Annotation, Boxed, Shape, Style, now};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect};

/// A sticky note's side on the page, in points: the size Acrobat draws its icons.
pub const SIZE: f32 = 20.0;

/// The icons the PDF standard names for a text annotation (ISO 32000-2, 12.5.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Icon {
    /// A speech bubble.
    Comment,
    /// A note pad: readers' default.
    #[default]
    Note,
    /// A question mark.
    Help,
    /// A caret.
    Insert,
    /// A key.
    Key,
    /// A pilcrow.
    Paragraph,
    /// A pilcrow with a plus.
    NewParagraph,
}

impl Icon {
    /// Every icon, in the order the editor offers them.
    pub const ALL: [Self; 7] = [
        Self::Comment,
        Self::Note,
        Self::Help,
        Self::Insert,
        Self::Key,
        Self::Paragraph,
        Self::NewParagraph,
    ];

    /// Its name in the PDF (`/Name`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Comment => "Comment",
            Self::Note => "Note",
            Self::Help => "Help",
            Self::Insert => "Insert",
            Self::Key => "Key",
            Self::Paragraph => "Paragraph",
            Self::NewParagraph => "NewParagraph",
        }
    }

    /// The icon a PDF names, or the default for none or a name readers don't know.
    #[must_use]
    pub fn of(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|icon| icon.name() == name)
            .unwrap_or_default()
    }
}

/// Whether `annotation` is a sticky note.
#[must_use]
pub fn is_sticky(annotation: &Annotation) -> bool {
    matches!(annotation.shape, Shape::Boxed(Boxed::Note, _))
}

/// A new sticky note on page `page` with its top left at `at`, moved inside the page (`bounds`) when it would leave
/// it, as Acrobat places one; its note closed and empty.
#[must_use]
pub fn place(
    page: usize,
    at: PagePoint,
    bounds: PageRect,
    icon: Icon,
    style: Style,
    author: &str,
) -> Annotation {
    let fit = |start: f32, low: f32, high: f32| start.min(high - SIZE).max(low);
    let (x0, y0) = (
        fit(at.x, bounds.x0, bounds.x1),
        fit(at.y, bounds.y0, bounds.y1),
    );
    let mut note = Annotation::new(
        page,
        Shape::Boxed(
            Boxed::Note,
            PageRect {
                x0,
                y0,
                x1: x0 + SIZE,
                y1: y0 + SIZE,
            },
        ),
        style,
        author,
    );
    note.icon = icon.name().to_owned();
    note
}

/// The change that gives the sticky note `annotation` the icon `icon`; none when it has it already.
#[must_use]
pub fn set_icon(annotation: &Annotation, icon: Icon) -> Option<Change> {
    (Icon::of(&annotation.icon) != icon || annotation.icon.is_empty()).then(|| {
        Change::UpdateAnnotation(Annotation {
            icon: icon.name().to_owned(),
            modified: now(),
            ..annotation.clone()
        })
    })
}

/// The change that makes other readers show `annotation`'s note open, or closed; none when it already is.
#[must_use]
pub fn set_open(annotation: &Annotation, open: bool) -> Option<Change> {
    (annotation.open != open).then(|| {
        Change::UpdateAnnotation(Annotation {
            open,
            modified: now(),
            ..annotation.clone()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    fn rect(annotation: &Annotation) -> PageRect {
        match annotation.shape {
            Shape::Boxed(Boxed::Note, rect) => rect,
            _ => panic!("a sticky note"),
        }
    }

    #[test]
    fn a_note_is_placed_at_the_click_inside_the_page() {
        let note = place(
            2,
            PagePoint { x: 100.0, y: 50.0 },
            PAGE,
            Icon::Help,
            Style::default(),
            "me",
        );
        assert!(is_sticky(&note));
        assert_eq!((note.page, note.icon.as_str()), (2, "Help"));
        assert!(!note.open && note.contents.is_empty());
        let at = rect(&note);
        assert_eq!((at.x0, at.y0, at.x1, at.y1), (100.0, 50.0, 120.0, 70.0));
        let corner = place(
            0,
            PagePoint { x: 611.0, y: 790.0 },
            PAGE,
            Icon::Note,
            Style::default(),
            "me",
        );
        let at = rect(&corner);
        assert_eq!((at.x1, at.y1), (612.0, 792.0), "kept on the page");
    }

    #[test]
    fn icons_are_the_standard_s_seven_and_read_back_by_name() {
        let names: Vec<&str> = Icon::ALL.iter().map(|icon| icon.name()).collect();
        assert_eq!(
            names,
            [
                "Comment",
                "Note",
                "Help",
                "Insert",
                "Key",
                "Paragraph",
                "NewParagraph"
            ]
        );
        for icon in Icon::ALL {
            assert_eq!(Icon::of(icon.name()), icon);
        }
        assert_eq!(Icon::of(""), Icon::Note, "readers' default");
        assert_eq!(Icon::of("Star"), Icon::Note);
    }

    #[test]
    fn changing_the_icon_or_open_state_is_one_change_only_when_it_changes() {
        let note = place(
            0,
            PagePoint { x: 0.0, y: 0.0 },
            PAGE,
            Icon::Note,
            Style::default(),
            "me",
        );
        assert!(set_icon(&note, Icon::Note).is_none());
        let Some(Change::UpdateAnnotation(keyed)) = set_icon(&note, Icon::Key) else {
            panic!("a new icon is a change");
        };
        assert_eq!(
            (keyed.id.as_str(), keyed.icon.as_str()),
            (note.id.as_str(), "Key")
        );
        assert!(set_open(&keyed, false).is_none());
        let Some(Change::UpdateAnnotation(opened)) = set_open(&keyed, true) else {
            panic!("opening is a change");
        };
        assert!(opened.open);
        let unnamed = Annotation {
            icon: String::new(),
            ..note
        };
        assert!(
            set_icon(&unnamed, Icon::Note).is_some(),
            "naming the default writes it into the file"
        );
    }
}
