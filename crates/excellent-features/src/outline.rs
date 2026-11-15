//! The document outline (plan §11.14, B.1–B.13), the model. Reading comes first: entries get ids from their place in
//! the tree (the path the MuPDF outline iterator walks, §11.14 step 4), the reader's page picks the entry it belongs
//! to, and the file's open flags say which entries start expanded. Editing (B.1–B.5) plans its changes here: where a
//! new entry goes, which open states to write, and the moves and deletes that act on several entries at once, each as
//! paths the way the outline is when the change is made.

use excellent_mupdf::OutlineStyle;

/// One entry of an outline, as the document has it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// Its title.
    pub title: String,
    /// The page it opens, counting from zero; `None` when it points nowhere in this document.
    pub page: Option<usize>,
    /// Whether it starts expanded.
    pub open: bool,
    /// How its title is drawn (B.7).
    pub style: OutlineStyle,
    /// The entries under it.
    pub children: Vec<Entry>,
}

/// A colour the outline menu gives entries (B.7). They are written into the document as they are; the rows draw them
/// adjusted to the theme so they stay readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Swatch {
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Purple.
    Purple,
}

impl Swatch {
    /// Every swatch, in the menu's order.
    pub const ALL: [Self; 5] = [
        Self::Red,
        Self::Orange,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    /// Its colour, `[r, g, b]`.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Red => [0xd6, 0x45, 0x45],
            Self::Orange => [0xd9, 0x82, 0x2b],
            Self::Green => [0x2f, 0x9e, 0x5b],
            Self::Blue => [0x2f, 0x6f, 0xd6],
            Self::Purple => [0x8a, 0x4f, 0xd6],
        }
    }
}

/// What a style command does to entries (B.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Restyle {
    /// Bold on, or off when every entry is bold already.
    Bold,
    /// Italic on, or off when every entry is italic already.
    Italic,
    /// This colour, or none.
    Color(Option<Swatch>),
}

/// The new styles of the entries of ids `targets` after `how`, as `(path, style)` for those whose style changes. Bold
/// and italic toggle together, as in a word processor: on for all unless all have it, then off for all.
pub fn plan_style(
    entries: &[Entry],
    targets: &[String],
    how: Restyle,
) -> Vec<(Vec<usize>, OutlineStyle)> {
    let found: Vec<(Vec<usize>, OutlineStyle)> = targets
        .iter()
        .filter_map(|id| Some((path(id)?, find(entries, id)?.style)))
        .collect();
    let all = |has: fn(&OutlineStyle) -> bool| found.iter().all(|(_, style)| has(style));
    let restyle: Box<dyn Fn(OutlineStyle) -> OutlineStyle> = match how {
        Restyle::Bold => {
            let bold = !all(|style| style.bold);
            Box::new(move |style| OutlineStyle { bold, ..style })
        }
        Restyle::Italic => {
            let italic = !all(|style| style.italic);
            Box::new(move |style| OutlineStyle { italic, ..style })
        }
        Restyle::Color(swatch) => Box::new(move |style| OutlineStyle {
            color: swatch.map(Swatch::rgb),
            ..style
        }),
    };
    found
        .into_iter()
        .filter_map(|(at, style)| {
            let new = restyle(style);
            (new != style).then_some((at, new))
        })
        .collect()
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

/// The path of the entry of id `id`; `None` when `id` isn't one.
pub fn path(id: &str) -> Option<Vec<usize>> {
    id.split('.').map(|part| part.parse().ok()).collect()
}

/// The longest title an entry takes from selected text, in characters.
const TITLE_LIMIT: usize = 200;

/// The title of an entry made from selected text (B.1 step 3): its words on one line, at most [`TITLE_LIMIT`]
/// characters and an ellipsis, cut between words where it can be; `None` when there is no text.
pub fn title_from_selection(text: &str) -> Option<String> {
    let mut title = String::new();
    let mut length = 0;
    for word in text.split_whitespace() {
        let gap = usize::from(!title.is_empty());
        let room = TITLE_LIMIT.saturating_sub(length + gap);
        let size = word.chars().count();
        if size > room {
            if title.is_empty() {
                title.extend(word.chars().take(room));
            }
            title.push('…');
            break;
        }
        if gap == 1 {
            title.push(' ');
        }
        title.push_str(word);
        length += gap + size;
    }
    (!title.is_empty()).then_some(title)
}

/// Where a new entry goes (B.1): right after the entry of id `after`, at its level; else after the last entry at the
/// top (the first entry of a new outline, step 4).
pub fn insert_at(entries: &[Entry], after: Option<&str>) -> Vec<usize> {
    let after = after
        .filter(|id| find(entries, id).is_some())
        .and_then(path);
    match after {
        Some(mut path) => {
            if let Some(last) = path.last_mut() {
                *last += 1;
            }
            path
        }
        None => vec![entries.len()],
    }
}

/// The entries with entries under them whose open state in the file differs from the reader's (B.5 step 2): their
/// paths and whether they are open now, in document order. An edit writes them first, so browsing alone never
/// changes the file and every edit keeps the outline as the reader sees it.
pub fn open_changes(entries: &[Entry], expanded: &[impl AsRef<str>]) -> Vec<(Vec<usize>, bool)> {
    let mut changes = Vec::new();
    walk(entries, &mut Vec::new(), &mut |path, entry| {
        let entry_id = id(path);
        let open = expanded.iter().any(|shown| shown.as_ref() == entry_id);
        if !entry.children.is_empty() && open != entry.open {
            changes.push((path.to_vec(), open));
        }
    });
    changes
}

/// The ids of the entries that show, in order: each one whose entries above are all expanded. A Shift+press marks
/// the ones between two of them (B.5).
pub fn shown_ids(entries: &[Entry], expanded: &[impl AsRef<str>]) -> Vec<String> {
    fn add(
        entries: &[Entry],
        expanded: &[impl AsRef<str>],
        path: &mut Vec<usize>,
        ids: &mut Vec<String>,
    ) {
        for (index, entry) in entries.iter().enumerate() {
            path.push(index);
            let entry_id = id(path);
            let open = expanded.iter().any(|shown| shown.as_ref() == entry_id);
            ids.push(entry_id);
            if open {
                add(&entry.children, expanded, path, ids);
            }
            path.pop();
        }
    }
    let mut ids = Vec::new();
    add(entries, expanded, &mut Vec::new(), &mut ids);
    ids
}

/// Which way entries move (B.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// Before the entry above them at their level (Alt+Up).
    Up,
    /// After the entry below them at their level (Alt+Down).
    Down,
    /// Under the entry above them at their level, as its last (Tab).
    In,
    /// Out of the entry they are under, right after it (Shift+Tab).
    Out,
}

/// What moving entries does: each move as `(from, to)`, `to` being where the entry goes once it has left, in the
/// order they are made; and the ids of the moved entries afterwards, to keep them selected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Moves {
    /// The moves, in order.
    pub steps: Vec<(Vec<usize>, Vec<usize>)>,
    /// The ids the moved entries have afterwards, in the order they were in (one under another moved one is left
    /// out: it went with it).
    pub moved: Vec<String>,
}

/// The paths of the entries of `ids`, in document order, without those under another of them: acting on an entry
/// acts on the entries under it. Ids of no entry are left out.
pub fn top_most(entries: &[Entry], ids: &[impl AsRef<str>]) -> Vec<Vec<usize>> {
    let mut paths: Vec<Vec<usize>> = ids
        .iter()
        .filter(|id| find(entries, id.as_ref()).is_some())
        .filter_map(|id| path(id.as_ref()))
        .collect();
    paths.sort();
    paths.dedup();
    let mut kept: Vec<Vec<usize>> = Vec::new();
    for path in paths {
        // Sorted, so an entry comes just before the entries under it.
        if !kept.last().is_some_and(|above| path.starts_with(above)) {
            kept.push(path);
        }
    }
    kept
}

/// The deletes that remove the entries of `ids` with the entries under them (B.2, B.5): later entries first, so each
/// path still names its entry when its turn comes.
pub fn plan_delete(entries: &[Entry], ids: &[impl AsRef<str>]) -> Vec<Vec<usize>> {
    let mut paths = top_most(entries, ids);
    paths.reverse();
    paths
}

/// The moves that shift the entries of `ids` one place `shift` (B.3, B.5); no steps when none of them can go.
/// Entries move as a group: one that can't go (the first at its level, moving up) holds back the marked entries
/// next to it, so the group keeps its order.
pub fn plan_shift(entries: &[Entry], ids: &[impl AsRef<str>], shift: Shift) -> Moves {
    let marked = top_most(entries, ids);
    let mut tree = Node::tree(entries, &marked);
    let mut order: Vec<usize> = (0..marked.len()).collect();
    if matches!(shift, Shift::Down | Shift::Out) {
        // From the last, so each one goes past entries that have already gone.
        order.reverse();
    }
    let mut steps = Vec::new();
    for mark in order {
        let Some(from) = tree.find(mark) else {
            continue;
        };
        if let Some(to) = tree.target(&from, shift) {
            let node = tree.remove(&from);
            tree.insert(&to, node);
            steps.push((from, to));
        }
    }
    let moved = if steps.is_empty() {
        Vec::new()
    } else {
        (0..marked.len())
            .filter_map(|mark| tree.find(mark))
            .map(|path| id(&path))
            .collect()
    };
    Moves { steps, moved }
}

/// An entry while moves are planned: which of the marked entries it is, if one, and the entries under it.
#[derive(Debug, Clone, Default)]
struct Node {
    mark: Option<usize>,
    children: Vec<Node>,
}

impl Node {
    /// The root over `entries`, each entry at one of the `marked` paths numbered by its place there.
    fn tree(entries: &[Entry], marked: &[Vec<usize>]) -> Self {
        fn build(entries: &[Entry], marked: &[Vec<usize>], path: &mut Vec<usize>) -> Vec<Node> {
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    path.push(index);
                    let node = Node {
                        mark: marked.iter().position(|m| m == path),
                        children: build(&entry.children, marked, path),
                    };
                    path.pop();
                    node
                })
                .collect()
        }
        Self {
            mark: None,
            children: build(entries, marked, &mut Vec::new()),
        }
    }

    /// The path of the marked entry `mark`.
    fn find(&self, mark: usize) -> Option<Vec<usize>> {
        self.children.iter().enumerate().find_map(|(index, child)| {
            if child.mark == Some(mark) {
                return Some(vec![index]);
            }
            let mut below = child.find(mark)?;
            below.insert(0, index);
            Some(below)
        })
    }

    fn at(&self, path: &[usize]) -> Option<&Self> {
        path.iter()
            .try_fold(self, |node, &index| node.children.get(index))
    }

    /// The entry at `path`, which must name one.
    fn at_mut(&mut self, path: &[usize]) -> &mut Self {
        path.iter()
            .fold(self, |node, &index| &mut node.children[index])
    }

    /// Where the entry at `from` goes, once it has left, to move one place `shift`; `None` when it stays.
    fn target(&self, from: &[usize], shift: Shift) -> Option<Vec<usize>> {
        let (&index, parent) = from.split_last()?;
        let siblings = &self.at(parent)?.children;
        let unmarked = |at: usize| siblings.get(at).is_some_and(|node| node.mark.is_none());
        let mut to = parent.to_vec();
        match shift {
            Shift::Up => {
                let above = index.checked_sub(1).filter(|&above| unmarked(above))?;
                to.push(above);
            }
            Shift::Down => {
                // Once the entry has left, the one below it is at `index`: it goes right after that one.
                if !unmarked(index + 1) {
                    return None;
                }
                to.push(index + 1);
            }
            Shift::In => {
                let above = index.checked_sub(1).filter(|&above| unmarked(above))?;
                to.push(above);
                to.push(siblings[above].children.len());
            }
            Shift::Out => {
                let parent_index = to.pop()?;
                to.push(parent_index + 1);
            }
        }
        Some(to)
    }

    fn remove(&mut self, path: &[usize]) -> Self {
        let (&index, parent) = path.split_last().expect("a move starts at an entry");
        self.at_mut(parent).children.remove(index)
    }

    fn insert(&mut self, path: &[usize], node: Self) {
        let (&index, parent) = path.split_last().expect("a move ends at a place");
        self.at_mut(parent).children.insert(index, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, page: Option<usize>, children: Vec<Entry>) -> Entry {
        Entry {
            title: title.to_owned(),
            page,
            open: !children.is_empty(),
            style: OutlineStyle::default(),
            children,
        }
    }

    #[test]
    fn style_toggles_together_and_skips_what_stays() {
        let mut entries = book();
        entries[0].style.bold = true;
        let targets = vec!["0".to_owned(), "1.0".to_owned()];
        // One of two is bold: both turn bold, and only the other changes.
        let plan = plan_style(&entries, &targets, Restyle::Bold);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0, vec![1, 0]);
        assert!(plan[0].1.bold);
        // Both bold: both turn off.
        entries[1].children[0].style.bold = true;
        let plan = plan_style(&entries, &targets, Restyle::Bold);
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().all(|(_, style)| !style.bold));
        // A colour keeps the rest of the style; an unknown id is skipped.
        let targets = vec!["0".to_owned(), "9".to_owned()];
        let plan = plan_style(&entries, &targets, Restyle::Color(Some(Swatch::Blue)));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].1.color, Some(Swatch::Blue.rgb()));
        assert!(plan[0].1.bold);
        // No colour on an entry without one changes nothing.
        assert!(plan_style(&entries, &["2".to_owned()], Restyle::Color(None)).is_empty());
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

    #[test]
    fn paths_titles_and_where_a_new_entry_goes() {
        let book = book();
        assert_eq!(path("2.1"), Some(vec![2, 1]));
        for bad in ["", "x", "1.", "-1"] {
            assert_eq!(path(bad), None, "{bad}");
        }
        assert_eq!(
            title_from_selection("  Chapter\n 3:\tThe  End ").as_deref(),
            Some("Chapter 3: The End")
        );
        assert_eq!(title_from_selection(" \n\t "), None);
        let long = title_from_selection(&"word ".repeat(100)).expect("a title");
        assert!(long.ends_with("word…"), "{long}");
        assert!(long.chars().count() <= TITLE_LIMIT + 1);
        let one_word = title_from_selection(&"x".repeat(500)).expect("a title");
        assert_eq!(one_word.chars().count(), TITLE_LIMIT + 1);

        assert_eq!(insert_at(&book, Some("1.0")), [1, 1]);
        assert_eq!(insert_at(&book, Some("3")), [4]);
        assert_eq!(insert_at(&book, None), [4]);
        assert_eq!(insert_at(&book, Some("9.9")), [4]);
        assert_eq!(insert_at(&[], None), [0]);
    }

    #[test]
    fn only_open_state_the_reader_changed_is_written() {
        let book = book();
        let none: [&str; 0] = [];
        assert_eq!(open_changes(&book, &none), [(vec![1], false)]);
        assert!(open_changes(&book, &["1"]).is_empty());
        // A leaf has no open state to write.
        assert!(open_changes(&book, &["1", "0", "3"]).is_empty());
        assert_eq!(shown_ids(&book, &none), ["0", "1", "2", "3"]);
        assert_eq!(shown_ids(&book, &["1"]), ["0", "1", "1.0", "1.1", "2", "3"]);
    }

    /// The titles of `entries` after `moves`, as MuPDF makes them: each entry leaves, then goes to its place.
    fn after(mut entries: Vec<Entry>, moves: &Moves) -> Vec<String> {
        fn level<'a>(entries: &'a mut Vec<Entry>, path: &[usize]) -> &'a mut Vec<Entry> {
            path.iter()
                .fold(entries, |level, &index| &mut level[index].children)
        }
        for (from, to) in &moves.steps {
            let (&index, parent) = from.split_last().expect("a path");
            let entry = level(&mut entries, parent).remove(index);
            let (&index, parent) = to.split_last().expect("a path");
            level(&mut entries, parent).insert(index, entry);
        }
        let mut titles = Vec::new();
        walk(&entries, &mut Vec::new(), &mut |path, entry| {
            titles.push(format!("{} {}", id(path), entry.title));
        });
        titles
    }

    #[test]
    fn entries_move_one_place_as_a_group() {
        let moves = plan_shift(&book(), &["1.1"], Shift::Up);
        assert_eq!(moves.moved, ["1.0"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Intro",
                "1 Body",
                "1.0 Second",
                "1.1 First",
                "2 Web link",
                "3 End"
            ]
        );
        // The first entry can't go up, and holds back the one next to it.
        assert_eq!(
            plan_shift(&book(), &["0", "1"], Shift::Up),
            Moves::default()
        );

        let moves = plan_shift(&book(), &["3", "2"], Shift::Up);
        assert_eq!(moves.moved, ["1", "2"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Intro",
                "1 Web link",
                "2 End",
                "3 Body",
                "3.0 First",
                "3.1 Second"
            ]
        );

        let moves = plan_shift(&book(), &["0", "1"], Shift::Down);
        assert_eq!(moves.moved, ["1", "2"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Web link",
                "1 Intro",
                "2 Body",
                "2.0 First",
                "2.1 Second",
                "3 End"
            ]
        );
        assert_eq!(plan_shift(&book(), &["3"], Shift::Down), Moves::default());
    }

    #[test]
    fn entries_go_in_and_out_with_the_entries_under_them() {
        let moves = plan_shift(&book(), &["2"], Shift::In);
        assert_eq!(moves.moved, ["1.2"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Intro",
                "1 Body",
                "1.0 First",
                "1.1 Second",
                "1.2 Web link",
                "2 End"
            ]
        );
        // Body takes its entries with it, under Intro.
        let moves = plan_shift(&book(), &["1", "1.1"], Shift::In);
        assert_eq!(moves.moved, ["0.0"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Intro",
                "0.0 Body",
                "0.0.0 First",
                "0.0.1 Second",
                "1 Web link",
                "2 End"
            ]
        );

        let moves = plan_shift(&book(), &["1.0", "1.1"], Shift::Out);
        assert_eq!(moves.moved, ["2", "3"]);
        assert_eq!(
            after(book(), &moves),
            [
                "0 Intro",
                "1 Body",
                "2 First",
                "3 Second",
                "4 Web link",
                "5 End"
            ]
        );
        assert_eq!(plan_shift(&book(), &["0"], Shift::Out), Moves::default());
        assert_eq!(plan_shift(&book(), &["0"], Shift::In), Moves::default());
    }

    #[test]
    fn deletes_go_from_the_last_and_skip_entries_already_going() {
        let book = book();
        assert_eq!(
            plan_delete(&book, &["1", "1.0", "3", "9"]),
            [vec![3], vec![1]]
        );
        assert_eq!(
            top_most(&book, &["1.1", "1.0", "1.1"]),
            [vec![1, 0], vec![1, 1]]
        );
    }
}
