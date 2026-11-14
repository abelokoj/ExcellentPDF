//! Page labels and go to page (plan R.6): what each page is called, and which page the reader means by what they
//! type. A label wins over a number, as in SumatraPDF: in a book whose front matter runs i–xii, "3" is the page
//! labelled 3, not the third page.

/// A document's pages, as the reader names them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pages {
    count: usize,
    /// One per page, or `None` when the pages go by their numbers.
    labels: Option<Vec<String>>,
}

impl Pages {
    /// `count` pages, labelled `labels` (one per page; anything else is ignored and the pages go by number).
    pub fn new(count: usize, labels: Option<Vec<String>>) -> Self {
        let labels = labels.filter(|labels| labels.len() == count);
        Self { count, labels }
    }

    /// How many pages.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Whether the document names its pages its own way.
    pub fn has_labels(&self) -> bool {
        self.labels.is_some()
    }

    /// Page `index`'s label: its own, or its number from 1.
    pub fn label(&self, index: usize) -> String {
        match &self.labels {
            Some(labels) => labels.get(index).cloned().unwrap_or_default(),
            None => (index + 1).to_string(),
        }
    }

    /// Whether page `index`'s label is just its number, so a status line need not show both.
    pub fn label_is_number(&self, index: usize) -> bool {
        self.label(index) == (index + 1).to_string()
    }

    /// The page the reader means by `input`: a page whose label matches (ignoring case and the spaces around it),
    /// else a page number from 1. `None` when there is no such page.
    pub fn find(&self, input: &str) -> Option<usize> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        let labelled = self.labels.as_ref().and_then(|labels| {
            labels
                .iter()
                .position(|label| label.trim().eq_ignore_ascii_case(input))
        });
        labelled.or_else(|| {
            let number: usize = input.parse().ok()?;
            (1..=self.count).contains(&number).then(|| number - 1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Pages {
        let labels = ["i", "ii", "iii", "iv", "1", "2", "3", "4", "5", "6"];
        Pages::new(10, Some(labels.map(String::from).to_vec()))
    }

    #[test]
    fn typing_a_roman_label_finds_its_page() {
        assert_eq!(book().find("iv"), Some(3));
        assert_eq!(book().find(" IV "), Some(3));
    }

    #[test]
    fn a_label_wins_over_a_number_and_numbers_fill_the_rest() {
        let pages = book();
        assert_eq!(pages.find("2"), Some(5), "the page labelled 2");
        assert_eq!(
            pages.find("9"),
            Some(8),
            "no page is labelled 9: the ninth page"
        );
        assert_eq!(pages.find("11"), None);
        assert_eq!(pages.find("0"), None);
        assert_eq!(pages.find("x"), None);
        assert_eq!(pages.find("  "), None);
    }

    #[test]
    fn without_labels_pages_go_by_number() {
        let pages = Pages::new(3, None);
        assert_eq!(pages.label(0), "1");
        assert!(pages.label_is_number(2));
        assert_eq!(pages.find("3"), Some(2));
        assert_eq!(pages.find("4"), None);
        assert!(!pages.has_labels());
        assert_eq!(
            Pages::new(3, Some(vec!["a".into()])),
            pages,
            "labels that don't fit are ignored"
        );
    }

    #[test]
    fn labels_show_with_their_numbers() {
        let pages = book();
        assert_eq!(pages.label(3), "iv");
        assert!(!pages.label_is_number(3));
        assert!(
            !pages.label_is_number(5),
            "the page labelled 2 is the sixth"
        );
    }
}
