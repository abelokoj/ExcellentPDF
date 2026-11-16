//! Semantic categories (A.17): what an annotation means to the reader's argument (a claim, its evidence, a question,
//! a definition, a method, a result, something to do), each with its own colour and icon. An annotation has one
//! category or none; it is kept by its key in the annotation's private key (`Annotation::category`), so it travels
//! with the PDF and the operation log, and the annotation list filters by it.

use excellent_core::annot::Annotation;
use excellent_core::edit::Change;

use super::shapes::changed;

/// A semantic category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// A claim the text makes.
    Claim,
    /// Evidence for or against a claim.
    Evidence,
    /// A question the reader has.
    Question,
    /// A term's definition.
    Definition,
    /// How something was done.
    Method,
    /// What came of it.
    Result,
    /// Something for the reader to do.
    ToDo,
}

impl Category {
    /// Every category, in the order the pickers offer them.
    pub const ALL: [Self; 7] = [
        Self::Claim,
        Self::Evidence,
        Self::Question,
        Self::Definition,
        Self::Method,
        Self::Result,
        Self::ToDo,
    ];

    /// Its key, as the file keeps it and its label's key names it (`category.<key>`).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Evidence => "evidence",
            Self::Question => "question",
            Self::Definition => "definition",
            Self::Method => "method",
            Self::Result => "result",
            Self::ToDo => "todo",
        }
    }

    /// The category whose key is `key`; `None` for a key this version doesn't know.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.key() == key)
    }

    /// Its colour, `0xRRGGBB`: seven hues far enough apart to tell at a glance, each readable as a dot on light and
    /// dark chrome.
    #[must_use]
    pub fn color(self) -> u32 {
        match self {
            Self::Claim => 0xE5_48_4D,
            Self::Evidence => 0x30_A4_6C,
            Self::Question => 0xFF_B2_24,
            Self::Definition => 0x00_90_FF,
            Self::Method => 0x6E_56_CF,
            Self::Result => 0x12_A5_94,
            Self::ToDo => 0xF7_6B_15,
        }
    }

    /// Its icon's name in the Lucide set, for the views that draw icons.
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::Claim => "megaphone",
            Self::Evidence => "file-check",
            Self::Question => "circle-help",
            Self::Definition => "book-open",
            Self::Method => "flask-conical",
            Self::Result => "chart-column",
            Self::ToDo => "square-check",
        }
    }

    /// The category of `annotation`; `None` when it has none, or one this version doesn't know.
    #[must_use]
    pub fn of(annotation: &Annotation) -> Option<Self> {
        annotation.category.as_deref().and_then(Self::from_key)
    }
}

/// The change that puts `annotation` in `category`, or in none; none when it is there already.
#[must_use]
pub fn set_category(annotation: &Annotation, category: Option<Category>) -> Option<Change> {
    changed(annotation, |a| {
        a.category = category.map(|category| category.key().to_owned());
    })
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Markup, Shape, Style};

    use super::*;

    fn highlight() -> Annotation {
        Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        )
    }

    #[test]
    fn each_category_has_its_own_key_colour_and_icon() {
        for (at, category) in Category::ALL.into_iter().enumerate() {
            assert_eq!(Category::from_key(category.key()), Some(category));
            for other in &Category::ALL[at + 1..] {
                assert_ne!(category.key(), other.key());
                assert_ne!(category.color(), other.color());
                assert_ne!(category.icon(), other.icon());
            }
        }
        assert_eq!(Category::from_key("rumour"), None);
    }

    #[test]
    fn a_category_is_set_changed_and_taken_away_one_change_each() {
        let mut a = highlight();
        assert_eq!(Category::of(&a), None);
        let Some(Change::UpdateAnnotation(claim)) = set_category(&a, Some(Category::Claim)) else {
            panic!("a change");
        };
        assert_eq!(claim.category.as_deref(), Some("claim"));
        assert_eq!(Category::of(&claim), Some(Category::Claim));
        assert_eq!(
            set_category(&claim, Some(Category::Claim)),
            None,
            "no change"
        );
        let Some(Change::UpdateAnnotation(none)) = set_category(&claim, None) else {
            panic!("a change");
        };
        assert_eq!(none.category, None);
        a.category = Some("rumour".to_owned());
        assert_eq!(
            Category::of(&a),
            None,
            "a later version's category reads as none"
        );
    }
}
