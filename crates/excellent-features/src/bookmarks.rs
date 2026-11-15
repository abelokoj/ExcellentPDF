//! Personal bookmarks (plan B.6), the model: the reader's tags as typed, and which bookmarks a search finds. The
//! bookmarks themselves are kept in the library (`excellent_store::bookmarks`); a new one is titled as an outline
//! entry is ([`crate::outline::title_from_selection`]).

/// The tags typed in `text`, separated by commas: each trimmed, empty ones dropped, and each once (the first spelling
/// kept when two differ only in case).
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split(',').map(str::trim).filter(|tag| !tag.is_empty()) {
        if !tags
            .iter()
            .any(|kept| kept.to_lowercase() == tag.to_lowercase())
        {
            tags.push(tag.to_owned());
        }
    }
    tags
}

/// `tags` as the reader types them: [`parse_tags`] gives them back.
pub fn tags_text(tags: &[String]) -> String {
    tags.join(", ")
}

/// Whether a bookmark with this `title`, `note` and `tags` is found by `query`: every word of it is in one of them,
/// whatever its case. An empty query finds every bookmark.
pub fn matches(query: &str, title: &str, note: &str, tags: &[String]) -> bool {
    let fields: Vec<String> = [title, note]
        .into_iter()
        .chain(tags.iter().map(String::as_str))
        .map(str::to_lowercase)
        .collect();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|word| fields.iter().any(|field| field.contains(&word)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|&tag| tag.to_owned()).collect()
    }

    #[test]
    fn tags_are_read_as_typed_and_written_back_the_same() {
        assert_eq!(
            parse_tags(" method,, To read , method, to READ ,"),
            ["method", "To read"]
        );
        assert!(parse_tags(" , ").is_empty());
        let tags = owned(&["method", "to read"]);
        assert_eq!(parse_tags(&tags_text(&tags)), tags);
    }

    #[test]
    fn a_search_finds_every_word_in_the_title_note_or_tags() {
        let tags = owned(&["Method", "chapter 2"]);
        let found = |query| matches(query, "Proof of the lemma", "Check the bound", &tags);
        assert!(found(""));
        assert!(found("LEMMA"));
        assert!(found("lemma bound method"), "words from each field");
        assert!(found("chap"));
        assert!(!found("lemma theorem"), "every word must be found");
    }
}
