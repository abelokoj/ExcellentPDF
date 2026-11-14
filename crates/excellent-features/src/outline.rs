//! The document outline (plan §11.14, B.1–B.13), the model. Reading comes first: entries get ids from their place in
//! the tree (the path the MuPDF outline iterator walks, §11.14 step 4), the reader's page picks the entry it belongs
//! to, and the file's open flags say which entries start expanded. Editing (B.1–B.5) adds to this model.

/// One entry of an outline, as the document has it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// Its title.
    pub title: String,
    /// The page it opens, counting from zero; `None` when it points nowhere in this document.
    pub page: Option<usize>,
    /// Whether it starts expanded.
    pub open: bool,
    /// The entries under it.
    pub children: Vec<Entry>,
}

/// The id of the entry at `path` (the index of each entry from the top down): "0", "2.1", "2.1.4". Stable while the
/// outline isn't edited.
pub fn id(path: &[usize]) -> String {
    let parts: Vec<String> = path.iter().map(usize::to_string).collect();
    parts.join(".")
}

/// The entry of id `id`, if there is one.
pub fn find<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Entry> {
    let mut level = entries;
    let mut found = None;
    for part in id.split('.') {
        let entry = level.get(part.parse::<usize>().ok()?)?;
        found = Some(entry);
        level = &entry.children;
    }
    found
}

/// Calls `visit` with each entry's path and the entry, depth first, in document order.
fn walk(entries: &[Entry], path: &mut Vec<usize>, visit: &mut impl FnMut(&[usize], &Entry)) {
    for (index, entry) in entries.iter().enumerate() {
        path.push(index);
        visit(path, entry);
        walk(&entry.children, path, visit);
        path.pop();
    }
}

/// The ids of the entries that start expanded and have entries under them.
pub fn open_ids(entries: &[Entry]) -> Vec<String> {
    let mut ids = Vec::new();
    walk(entries, &mut Vec::new(), &mut |path, entry| {
        if entry.open && !entry.children.is_empty() {
            ids.push(id(path));
        }
    });
    ids
}

/// The entry the reader of page `page` is in: the one opening the nearest page at or before it, the last of those in
/// document order (a section starting on its chapter's page wins over the chapter). `None` before the first entry.
pub fn at_page(entries: &[Entry], page: usize) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    walk(entries, &mut Vec::new(), &mut |path, entry| {
        if let Some(start) = entry.page.filter(|&start| start <= page)
            && best.as_ref().is_none_or(|(at, _)| start >= *at)
        {
            best = Some((start, id(path)));
        }
    });
    best.map(|(_, id)| id)
}

/// The entry to show as the reader's: the one the reader chose while it still opens the page being read (several
/// entries can open one page), else the one [`at_page`] finds.
pub fn reading(entries: &[Entry], chosen: Option<&str>, page: usize) -> Option<String> {
    let still = chosen.filter(|id| find(entries, id).and_then(|entry| entry.page) == Some(page));
    still.map(str::to_owned).or_else(|| at_page(entries, page))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, page: Option<usize>, children: Vec<Entry>) -> Entry {
        Entry {
            title: title.to_owned(),
            page,
            open: !children.is_empty(),
            children,
        }
    }

    /// 1 Intro (p0); 2 Body (p3) with 2.1 (p3) and 2.2 (p7); 3 Web link (nowhere); 4 End (p10).
    fn book() -> Vec<Entry> {
        vec![
            entry("Intro", Some(0), vec![]),
            entry(
                "Body",
                Some(3),
                vec![
                    entry("First", Some(3), vec![]),
                    entry("Second", Some(7), vec![]),
                ],
            ),
            entry("Web link", None, vec![]),
            entry("End", Some(10), vec![]),
        ]
    }

    #[test]
    fn ids_are_paths_and_find_their_entries() {
        let book = book();
        assert_eq!(id(&[1, 0]), "1.0");
        assert_eq!(find(&book, "1.1").map(|e| e.title.as_str()), Some("Second"));
        assert_eq!(find(&book, "3").map(|e| e.title.as_str()), Some("End"));
        for missing in ["9", "1.5", "0.0", "", "x", "1."] {
            assert_eq!(find(&book, missing), None, "{missing}");
        }
        assert_eq!(open_ids(&book), ["1"]);
    }

    #[test]
    fn the_reader_is_in_the_nearest_entry_before_the_page() {
        let book = book();
        assert_eq!(at_page(&book, 0).as_deref(), Some("0"));
        assert_eq!(at_page(&book, 2).as_deref(), Some("0"));
        // The section starting on its chapter's page wins.
        assert_eq!(at_page(&book, 3).as_deref(), Some("1.0"));
        assert_eq!(at_page(&book, 9).as_deref(), Some("1.1"));
        assert_eq!(at_page(&book, 500).as_deref(), Some("3"));
        assert_eq!(at_page(&[entry("Late", Some(5), vec![])], 2), None);
        assert_eq!(at_page(&[], 2), None);
    }

    #[test]
    fn a_chosen_entry_stays_while_it_opens_the_page() {
        let book = book();
        // The chapter shares page 3 with its first section: chosen, it stays.
        assert_eq!(reading(&book, Some("1"), 3).as_deref(), Some("1"));
        // Read on, and the page decides again.
        assert_eq!(reading(&book, Some("1"), 8).as_deref(), Some("1.1"));
        assert_eq!(reading(&book, Some("nope"), 0).as_deref(), Some("0"));
        assert_eq!(reading(&book, None, 3).as_deref(), Some("1.0"));
    }
}
