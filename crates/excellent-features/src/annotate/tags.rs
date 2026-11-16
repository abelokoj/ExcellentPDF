//! Tags on annotations (A.18): free-form words the reader gives an annotation, typed as a comma-separated list (as a
//! bookmark's are, B.6). They are kept on the annotation, in its private key, so they travel with the PDF; the app
//! indexes them in the library when the file is saved, and the annotation list finds them with `tag:`.

use excellent_core::annot::Annotation;
use excellent_core::edit::Change;

use super::shapes::changed;
use crate::bookmarks::{parse_tags, tags_text};

/// The search word that asks for a tag: `tag:method`.
pub const PREFIX: &str = "tag:";

/// `annotation`'s tags as the reader types them.
#[must_use]
pub fn text(annotation: &Annotation) -> String {
    tags_text(&annotation.tags)
}

/// The change that gives `annotation` the tags typed in `text`; none when it has them already.
#[must_use]
pub fn set_tags(annotation: &Annotation, text: &str) -> Option<Change> {
    let tags = parse_tags(text);
    changed(annotation, |a| a.tags = tags)
}

/// The tag a search word asks for, folded to lower case: `tag:Method` asks for `method`; `None` for another word, or
/// `tag:` alone.
#[must_use]
pub fn asked(word: &str) -> Option<String> {
    let rest = word
        .get(..PREFIX.len())
        .filter(|start| start.eq_ignore_ascii_case(PREFIX))
        .map(|_| &word[PREFIX.len()..])?;
    (!rest.is_empty()).then(|| rest.to_lowercase())
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Markup, Shape, Style};

    use super::*;

    #[test]
    fn tags_are_typed_with_commas_and_set_in_one_change() {
        let a = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        let Some(Change::UpdateAnnotation(tagged)) = set_tags(&a, " method,  To Read, method ")
        else {
            panic!("a change");
        };
        assert_eq!(tagged.tags, ["method", "To Read"]);
        assert_eq!(text(&tagged), "method, To Read");
        assert_eq!(set_tags(&tagged, "method, To Read"), None, "no change");
        let Some(Change::UpdateAnnotation(none)) = set_tags(&tagged, "  ") else {
            panic!("a change");
        };
        assert!(none.tags.is_empty());
    }

    #[test]
    fn a_search_word_asks_for_a_tag() {
        assert_eq!(asked("tag:Method"), Some("method".to_owned()));
        assert_eq!(asked("TAG:x"), Some("x".to_owned()));
        assert_eq!(asked("tag:"), None);
        assert_eq!(asked("tags"), None);
        assert_eq!(asked("méthod"), None);
    }
}
