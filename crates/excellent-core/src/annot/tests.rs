//! K.14's model-based tests (Appendix D.3.11): random sequences of annotation edits, each one operation, are saved,
//! the file reopened, and what it holds compared with the model; undoing them all gives the page back as it was.

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use excellent_mupdf::{AnnotData, Document};
use proptest::prelude::*;
use proptest::sample::select;

use super::*;
use crate::edit::{Change, Operation};

const PAGES: usize = 2;

fn corpus() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/base14-outline-annot.pdf")
}

/// A file in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "excellent-core-annot-{}-{n}.pdf",
            std::process::id()
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Every annotation of every page, modelled or not, as MuPDF reads it.
fn everything(doc: &Document) -> Vec<(usize, AnnotData)> {
    let mut all = Vec::new();
    for page in 0..PAGES {
        let loaded = doc.load_page(page).expect("page");
        for annot in loaded.annots().expect("annotations") {
            all.push((page, annot.data().expect("data")));
        }
    }
    all
}

fn point() -> impl Strategy<Value = PagePoint> {
    (0..1000u16, 0..1400u16).prop_map(|(x, y)| PagePoint {
        x: f32::from(x) / 2.0,
        y: f32::from(y) / 2.0,
    })
}

fn rect() -> impl Strategy<Value = PageRect> {
    (point(), 20..200u16, 20..200u16).prop_map(|(p, w, h)| PageRect {
        x0: p.x,
        y0: p.y,
        x1: p.x + f32::from(w) / 2.0,
        y1: p.y + f32::from(h) / 2.0,
    })
}

fn quad() -> impl Strategy<Value = Quad> {
    rect().prop_map(|r| Quad {
        ul: PagePoint { x: r.x0, y: r.y0 },
        ur: PagePoint { x: r.x1, y: r.y0 },
        ll: PagePoint { x: r.x0, y: r.y1 },
        lr: PagePoint { x: r.x1, y: r.y1 },
    })
}

fn shape() -> impl Strategy<Value = Shape> {
    let markup = select(vec![
        Markup::Highlight,
        Markup::Underline,
        Markup::Squiggly,
        Markup::StrikeOut,
    ]);
    let boxed = select(vec![
        Boxed::Note,
        Boxed::FreeText,
        Boxed::Typewriter,
        Boxed::Square,
        Boxed::Circle,
        Boxed::Stamp,
        Boxed::Caret,
    ]);
    prop_oneof![
        (markup, prop::collection::vec(quad(), 1..4)).prop_map(|(k, q)| Shape::Markup(k, q)),
        (boxed, rect()).prop_map(|(k, r)| Shape::Boxed(k, r)),
        prop::collection::vec(prop::collection::vec(point(), 2..6), 1..3).prop_map(Shape::Ink),
        (point(), point()).prop_map(|(a, b)| Shape::Line(a, b)),
        (
            select(vec![Path::Polygon, Path::PolyLine]),
            prop::collection::vec(point(), 3..6)
        )
            .prop_map(|(k, v)| Shape::Path(k, v)),
    ]
}

fn style() -> impl Strategy<Value = Style> {
    (
        select(vec![
            None,
            Some([1.0, 0.85, 0.0]),
            Some([0.0, 0.5, 1.0]),
            Some([0.25, 0.75, 0.5]),
        ]),
        select(vec![1.0, 0.5, 0.25]),
        select(vec![0.0, 1.0, 2.5]),
    )
        .prop_map(|(color, opacity, width)| Style {
            color,
            opacity,
            width,
        })
}

fn text() -> impl Strategy<Value = String> {
    select(vec![
        String::new(),
        "A note".to_owned(),
        "Café, naïve: “quoted” (twice) \\ done".to_owned(),
        "第二章の注".to_owned(),
        "Line one\nline two".to_owned(),
    ])
}

fn text_style() -> impl Strategy<Value = TextStyle> {
    (
        select(Font::ALL.to_vec()),
        select(vec![9.0, 12.0, 18.0]),
        select(vec![[0.0; 3], [0.0, 0.25, 0.5], [1.0, 0.0, 0.0]]),
        select(vec![Align::Left, Align::Center, Align::Right]),
    )
        .prop_map(|(font, size, color, align)| TextStyle {
            font,
            size,
            color,
            align,
        })
}

fn tags() -> impl Strategy<Value = Vec<String>> {
    select(vec![
        Vec::new(),
        vec!["todo".to_owned()],
        vec!["method".to_owned(), "check later".to_owned()],
    ])
}

fn anchor() -> impl Strategy<Value = Option<Quote>> {
    select(vec![
        None,
        Some(Quote {
            prefix: "streams are made of ".to_owned(),
            exact: "blocks".to_owned(),
            suffix: ". Each block is".to_owned(),
            start: 24,
        }),
    ])
}

fn date() -> impl Strategy<Value = i64> {
    1_600_000_000..1_900_000_000i64
}

fn annotation() -> impl Strategy<Value = Annotation> {
    (
        0..PAGES,
        shape(),
        style(),
        select(vec!["Ada".to_owned(), "Grace Hopper".to_owned()]),
        date(),
        text(),
        tags(),
        anchor(),
        (
            select(vec![
                "",
                "Comment",
                "Note",
                "Help",
                "Insert",
                "Key",
                "Paragraph",
                "NewParagraph",
            ])
            .prop_map(str::to_owned),
            any::<bool>(),
        ),
        text_style(),
    )
        .prop_map(
            |(page, shape, style, author, created, contents, tags, anchor, (icon, open), text)| {
                // Only a sticky note has an icon and its own open state here, and only a text box a text style.
                let note = matches!(shape, Shape::Boxed(Boxed::Note, _));
                let text_box =
                    matches!(shape, Shape::Boxed(Boxed::FreeText | Boxed::Typewriter, _));
                Annotation {
                    id: new_id(),
                    page,
                    shape,
                    style,
                    author,
                    created,
                    modified: created,
                    contents,
                    tags,
                    anchor,
                    group: None,
                    icon: if note { icon } else { String::new() },
                    open: note && open,
                    text: if text_box { text } else { TextStyle::default() },
                }
            },
        )
}

#[derive(Debug, Clone)]
enum Action {
    Create(Box<Annotation>),
    /// The `n`th of the annotations made so far (modulo their count) restyled, rewritten and re-tagged.
    Update(usize, Style, String, Vec<String>, i64),
    Delete(usize),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => annotation().prop_map(|a| Action::Create(Box::new(a))),
        2 => (any::<usize>(), style(), text(), tags(), date())
            .prop_map(|(n, style, contents, tags, modified)| Action::Update(n, style, contents, tags, modified)),
        1 => any::<usize>().prop_map(Action::Delete),
    ]
}

/// The annotation as the file keeps it: the width only of the kinds that have one.
fn kept(mut annotation: Annotation) -> Annotation {
    if !annotation.shape.has_width() {
        annotation.style.width = 0.0;
    }
    annotation
}

/// Applies `actions` to `doc` and to the model, one operation each.
fn run(doc: &mut Document, actions: &[Action]) -> BTreeMap<String, Annotation> {
    let mut model: BTreeMap<String, Annotation> = BTreeMap::new();
    for action in actions {
        let ids: Vec<String> = model.keys().cloned().collect();
        let change = match action {
            Action::Create(annotation) => Change::CreateAnnotation((**annotation).clone()),
            Action::Update(n, style, contents, tags, modified) => {
                let Some(id) = (!ids.is_empty()).then(|| &ids[n % ids.len()]) else {
                    continue;
                };
                let mut annotation = model[id].clone();
                annotation.style = *style;
                annotation.contents.clone_from(contents);
                annotation.tags.clone_from(tags);
                annotation.modified = *modified;
                Change::UpdateAnnotation(annotation)
            }
            Action::Delete(n) => {
                let Some(id) = (!ids.is_empty()).then(|| &ids[n % ids.len()]) else {
                    continue;
                };
                Change::DeleteAnnotation {
                    page: model[id].page,
                    id: id.clone(),
                }
            }
        };
        Operation::new("Edit", vec![change.clone()])
            .apply(doc)
            .unwrap_or_else(|err| panic!("{change:?} failed: {err}"));
        match change {
            Change::CreateAnnotation(a) | Change::UpdateAnnotation(a) => {
                model.insert(a.id.clone(), kept(a));
            }
            Change::DeleteAnnotation { id, .. } => {
                model.remove(&id);
            }
            _ => unreachable!("only annotation changes are made"),
        }
    }
    model
}

/// Whether a `kind` annotation given `given` is drawn at `drawn`, as MuPDF sizes the kinds with artwork of their
/// own (`pdf-appearance.c`): a stamp fitted inside the rectangle and centred; a caret 20 × 14 points about its
/// centre; a note's icon a square of 10 to 20 points at its top-left corner. The others keep the rectangle.
fn sized(kind: Boxed, given: PageRect, drawn: PageRect) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    let same = |a: PageRect, b: PageRect| {
        near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1)
    };
    let (cx, cy) = ((given.x0 + given.x1) / 2.0, (given.y0 + given.y1) / 2.0);
    match kind {
        Boxed::Stamp => {
            let inside = drawn.x0 >= given.x0 - 0.01
                && drawn.y0 >= given.y0 - 0.01
                && drawn.x1 <= given.x1 + 0.01
                && drawn.y1 <= given.y1 + 0.01;
            let centred =
                near(drawn.x0 + drawn.x1, 2.0 * cx) && near(drawn.y0 + drawn.y1, 2.0 * cy);
            let fills = near(drawn.width(), given.width()) || near(drawn.height(), given.height());
            inside && centred && fills
        }
        Boxed::Caret => same(
            drawn,
            PageRect {
                x0: cx - 10.0,
                y0: cy - 7.0,
                x1: cx + 10.0,
                y1: cy + 7.0,
            },
        ),
        Boxed::Note => {
            let side = given.width().min(given.height()).clamp(10.0, 20.0);
            same(
                drawn,
                PageRect {
                    x0: given.x0,
                    y0: given.y0,
                    x1: given.x0 + side,
                    y1: given.y0 + side,
                },
            )
        }
        Boxed::FreeText | Boxed::Typewriter | Boxed::Square | Boxed::Circle => same(drawn, given),
    }
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(48)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Done when (K.14): random sequences of annotation edits read back, after a save and a reopen, as the model
    /// holds them; the file's other annotations are untouched; undoing every operation restores the pages.
    #[test]
    fn random_annotation_edits_survive_save_and_reopen(actions in prop::collection::vec(action(), 1..10)) {
        let mut doc = Document::open_pdf(corpus()).expect("open");
        let before = everything(&doc);
        let model = run(&mut doc, &actions);

        let scratch = Scratch::new();
        doc.save_pdf(&scratch.0).expect("save");
        let reopened = Document::open_pdf(&scratch.0).expect("reopen");
        let (mut ours, mut others) = (BTreeMap::new(), Vec::new());
        for (page, data) in everything(&reopened) {
            if let Some(expected) = model.get(&data.id) {
                let mut annotation = Annotation::from_data(page, &data).expect("a modelled kind");
                if let (Shape::Boxed(kind, given), Shape::Boxed(_, drawn)) =
                    (&expected.shape, &mut annotation.shape)
                {
                    prop_assert!(sized(*kind, *given, *drawn), "{kind:?} drawn at {drawn:?}, given {given:?}");
                    *drawn = *given;
                }
                prop_assert!(ours.insert(data.id.clone(), annotation).is_none(), "id {} twice", data.id);
            } else {
                others.push((page, data));
            }
        }
        prop_assert_eq!(&ours, &model);
        prop_assert_eq!(&others, &before);

        while doc.undo_state().expect("state").can_undo() {
            doc.undo().expect("undo");
        }
        prop_assert_eq!(everything(&doc), before);
    }
}

#[test]
fn annotation_changes_read_back_from_the_log_as_written() {
    let mut note = Annotation::new(
        1,
        Shape::Boxed(
            Boxed::Note,
            PageRect {
                x0: 10.0,
                y0: 20.5,
                x1: 30.0,
                y1: 40.0,
            },
        ),
        Style::default(),
        "Ada",
    );
    note.contents = "“Quoted”\nsecond line".into();
    note.icon = "Help".into();
    note.open = true;
    note.tags = vec!["todo".into()];
    let mut typed = Annotation::new(
        0,
        Shape::Boxed(Boxed::Typewriter, PageRect::default()),
        Style::default(),
        "Ada",
    );
    typed.text = TextStyle {
        font: Font::Mono,
        size: 10.5,
        color: [0.0, 0.25, 1.0],
        align: Align::Right,
    };
    note.anchor = Some(Quote {
        prefix: "a ".into(),
        exact: "word".into(),
        suffix: " here".into(),
        start: 2,
    });
    let ink = Annotation::new(
        0,
        Shape::Ink(vec![vec![
            PagePoint { x: 1.0, y: 2.0 },
            PagePoint { x: 3.5, y: 4.0 },
        ]]),
        Style {
            color: None,
            opacity: 0.5,
            width: 2.0,
        },
        "Grace",
    );
    let changes = vec![
        Change::CreateAnnotation(note.clone()),
        Change::CreateAnnotation(typed),
        Change::UpdateAnnotation(ink),
        Change::DeleteAnnotation {
            page: 1,
            id: note.id.clone(),
        },
    ];
    let operation = Operation::new("Annotations", changes);
    let mut record = serde_json::Map::new();
    operation.write_json(&mut record);
    let text = Value::Object(record).to_string();
    let read: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        Operation::read_json(read.as_object().expect("object")),
        Some(operation)
    );
}

#[test]
fn the_anchor_and_tags_travel_in_the_private_key() {
    let mut highlight = Annotation::new(
        0,
        Shape::Markup(Markup::Highlight, Vec::new()),
        Style::default(),
        "Ada",
    );
    assert_eq!(highlight.to_data().app_data, "", "nothing to keep, no key");
    highlight.tags = vec!["a".into()];
    let data = highlight.to_data();
    assert_eq!(
        Annotation::from_data(0, &data).expect("modelled").tags,
        ["a"]
    );
    let mut link = data;
    link.kind = excellent_mupdf::AnnotKind::Link;
    assert_eq!(
        Annotation::from_data(0, &link),
        None,
        "links aren't modelled"
    );
}

/// A.2: a strikeout grouped with a caret ("replace text") is written as `/IRT` with `/RT /Group`, survives a save and
/// a reopen, and ungrouping it takes both keys away.
#[test]
fn a_grouped_pair_survives_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut caret = Annotation::new(
        0,
        Shape::Boxed(
            Boxed::Caret,
            PageRect {
                x0: 90.0,
                y0: 100.0,
                x1: 110.0,
                y1: 114.0,
            },
        ),
        Style::default(),
        "Ada",
    );
    caret.contents = "new words".into();
    let mut strike = Annotation::new(
        0,
        Shape::Markup(
            Markup::StrikeOut,
            vec![Quad {
                ul: PagePoint { x: 40.0, y: 100.0 },
                ur: PagePoint { x: 100.0, y: 100.0 },
                ll: PagePoint { x: 40.0, y: 112.0 },
                lr: PagePoint { x: 100.0, y: 112.0 },
            }],
        ),
        Style::default(),
        "Ada",
    );
    strike.group = Some(caret.id.clone());
    Operation::new(
        "Replace",
        vec![
            Change::CreateAnnotation(caret.clone()),
            Change::CreateAnnotation(strike.clone()),
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let found = |id: &str| read.iter().find(|a| a.id == id).expect("kept").clone();
    assert_eq!(found(&strike.id).group.as_deref(), Some(caret.id.as_str()));
    assert_eq!(found(&caret.id).group, None);
    assert_eq!(found(&caret.id).contents, "new words");

    strike.group = None;
    Operation::new("Ungroup", vec![Change::UpdateAnnotation(strike.clone())])
        .apply(&mut doc)
        .expect("apply");
    assert_eq!(
        page_annotations(&doc, 0)
            .expect("read")
            .iter()
            .find(|a| a.id == strike.id)
            .expect("kept")
            .group,
        None
    );
}

/// A.4: a highlight's note is saved as its `/Contents` with a `/Popup`, so other readers open it in their comment
/// panes; a note written again keeps the one popup.
#[test]
fn a_note_on_a_highlight_gets_a_popup_and_survives_save_and_reopen() {
    let popups = |bytes: &[u8]| bytes.windows(14).filter(|w| w == b"/Subtype/Popup").count();
    let before = popups(&std::fs::read(corpus()).expect("read the corpus"));
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let annotations = page_annotations(&doc, 0).expect("read").len();
    let mut highlight = Annotation::new(
        0,
        Shape::Markup(
            Markup::Highlight,
            vec![Quad {
                ul: PagePoint { x: 40.0, y: 100.0 },
                ur: PagePoint { x: 100.0, y: 100.0 },
                ll: PagePoint { x: 40.0, y: 112.0 },
                lr: PagePoint { x: 100.0, y: 112.0 },
            }],
        ),
        Style::default(),
        "Ada",
    );
    Operation::new(
        "Highlight",
        vec![Change::CreateAnnotation(highlight.clone())],
    )
    .apply(&mut doc)
    .expect("apply");
    highlight.contents = "Check the figure.".into();
    Operation::new("Note", vec![Change::UpdateAnnotation(highlight.clone())])
        .apply(&mut doc)
        .expect("apply");
    highlight.contents = "Check the figure, then the table.".into();
    Operation::new("Note", vec![Change::UpdateAnnotation(highlight.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read the file");
    assert_eq!(popups(&bytes), before + 1, "one popup, written once");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    assert_eq!(
        read.len(),
        annotations + 1,
        "the popup is no annotation of its own"
    );
    let noted = read.iter().find(|a| a.id == highlight.id).expect("kept");
    assert_eq!(noted.contents, "Check the figure, then the table.");
}

#[test]
fn a_sticky_note_keeps_its_icon_and_open_state_through_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut note = Annotation::new(
        0,
        Shape::Boxed(
            Boxed::Note,
            PageRect {
                x0: 50.0,
                y0: 60.0,
                x1: 70.0,
                y1: 80.0,
            },
        ),
        Style::default(),
        "Ada",
    );
    note.icon = "Help".into();
    Operation::new("Sticky note", vec![Change::CreateAnnotation(note.clone())])
        .apply(&mut doc)
        .expect("apply");
    note.contents = "Why this number?".into();
    note.open = true;
    Operation::new("Note", vec![Change::UpdateAnnotation(note.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read the file");
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"/Name/Help"), "the icon Acrobat draws");
    assert!(has(b"/Open true"), "the popup opens in other readers");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let kept = read.iter().find(|a| a.id == note.id).expect("kept");
    assert_eq!(
        (kept.icon.as_str(), kept.open, kept.contents.as_str()),
        ("Help", true, "Why this number?")
    );
}

/// A.6: typewriter text typed in Chinese keeps its intent, font, size, colour and alignment through a save and a
/// reopen, written as Acrobat reads them (`/IT /FreeTextTypeWriter`, `/DA`, `/Q`).
#[test]
fn typewriter_text_keeps_its_style_through_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut typed = Annotation::new(
        0,
        Shape::Boxed(
            Boxed::Typewriter,
            PageRect {
                x0: 72.0,
                y0: 72.0,
                x1: 272.0,
                y1: 92.0,
            },
        ),
        Style {
            color: None,
            opacity: 1.0,
            width: 0.0,
        },
        "Ada",
    );
    typed.contents = "中文输入 and English".into();
    typed.text = TextStyle {
        font: Font::Mono,
        size: 14.0,
        color: [0.0, 0.25, 0.5],
        align: Align::Center,
    };
    Operation::new("Type", vec![Change::CreateAnnotation(typed.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read");
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"/IT/FreeTextTypeWriter"), "the intent Acrobat reads");
    assert!(has(b"/Cour 14 Tf"), "the font and size in /DA");
    assert!(has(b"/Q 1"), "centred");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0)
        .expect("read")
        .into_iter()
        .find(|a| a.id == typed.id)
        .expect("kept");
    assert!(matches!(read.shape, Shape::Boxed(Boxed::Typewriter, _)));
    assert_eq!(read.text, typed.text);
    assert_eq!(read.contents, typed.contents);
}
