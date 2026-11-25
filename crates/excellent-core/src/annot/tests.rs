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
        (rect(), point()).prop_map(|(r, tip)| Shape::Callout(r, tip)),
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
            any::<bool>(),
        ),
        (
            text_style(),
            select(LineEnd::ALL.to_vec()),
            select(LineEnd::ALL.to_vec()),
        ),
    )
        .prop_map(
            |(
                page,
                shape,
                style,
                author,
                created,
                contents,
                tags,
                anchor,
                (icon, open, locked),
                (text, start, end),
            )| {
                // Only a sticky note has an icon and its own open state here, and only a text box a text style.
                let note = matches!(shape, Shape::Boxed(Boxed::Note, _));
                let text_box = matches!(
                    shape,
                    Shape::Boxed(Boxed::FreeText | Boxed::Typewriter, _) | Shape::Callout(..)
                );
                let shaped = matches!(shape, Shape::Boxed(Boxed::Square | Boxed::Circle, _));
                let closed = shaped || matches!(shape, Shape::Path(Path::Polygon, _));
                // Lines, polylines and polygons keep both ends; a callout its tip's.
                let ends = match shape {
                    Shape::Line(..) | Shape::Path(..) => [start, end],
                    Shape::Callout(..) => [start, LineEnd::None],
                    _ => [LineEnd::None; 2],
                };
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
                    reply_to: None,
                    layer: None,
                    state: None,
                    category: None,
                    icon: if note { icon } else { String::new() },
                    open: note && open,
                    text: if text_box { text } else { TextStyle::default() },
                    ends,
                    // Rectangles and ellipses keep a fill and dashes (A.9).
                    fill: shaped
                        .then_some([0.25, 0.5, 1.0])
                        .filter(|_| start != LineEnd::None),
                    dash: if shaped && end != LineEnd::None {
                        vec![6.0, 4.0]
                    } else {
                        Vec::new()
                    },
                    // Closed shapes keep a cloudy border (A.10).
                    cloud: closed
                        .then_some(1.0)
                        .filter(|_| matches!(start, LineEnd::Square | LineEnd::Circle)),
                    // Any kind can be locked (A.12).
                    locked,
                    // Any kind keeps PDF 2.0's blend mode (H.3).
                    multiply: open,
                    widths: Vec::new(),
                    tape: locked && closed,
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
                if let (Shape::Callout(given, _), Shape::Callout(drawn, _)) =
                    (&expected.shape, &mut annotation.shape)
                {
                    prop_assert!(sized(Boxed::FreeText, *given, *drawn), "callout drawn at {drawn:?}, given {given:?}");
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
    note.category = Some("question".into());
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
    highlight.category = Some("claim".into());
    let data = highlight.to_data();
    let read = Annotation::from_data(0, &data).expect("modelled");
    assert_eq!(read.tags, ["a"]);
    assert_eq!(read.category.as_deref(), Some("claim"));
    let mut link = data;
    link.kind = excellent_mupdf::AnnotKind::Link;
    assert_eq!(
        Annotation::from_data(0, &link),
        None,
        "links aren't modelled"
    );
}

/// A.19: a page recorded without an annotation's id doesn't draw it, and the others still draw.
#[test]
fn a_hidden_annotation_is_left_out_of_the_page() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let highlight = Annotation::new(
        0,
        Shape::Markup(
            Markup::Highlight,
            vec![Quad {
                ul: PagePoint { x: 40.0, y: 100.0 },
                ur: PagePoint { x: 200.0, y: 100.0 },
                ll: PagePoint { x: 40.0, y: 140.0 },
                lr: PagePoint { x: 200.0, y: 140.0 },
            }],
        ),
        Style {
            color: Some([1.0, 0.0, 0.0]),
            opacity: 1.0,
            width: 0.0,
        },
        "Ada",
    );
    Operation::new(
        "Highlight",
        vec![Change::CreateAnnotation(highlight.clone())],
    )
    .apply(&mut doc)
    .expect("apply");
    let page = doc.load_page(0).expect("page");
    let pixels = |hidden: &[String]| {
        page.display_list_without(hidden, None)
            .expect("list")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render")
            .samples()
            .to_vec()
    };
    let all = pixels(&[]);
    assert_eq!(
        pixels(&["someone else".to_owned()]),
        all,
        "another id hides nothing"
    );
    let without = pixels(std::slice::from_ref(&highlight.id));
    assert_ne!(without, all, "the highlight is gone");
    let contents = page
        .contents_display_list(None)
        .expect("list")
        .render(
            &excellent_mupdf::Matrix::IDENTITY,
            excellent_mupdf::Background::White,
        )
        .expect("render")
        .samples()
        .to_vec();
    assert!(
        page_annotations(&doc, 0).expect("read").len() > 1 || without == contents,
        "with the only annotation hidden, the page is its contents"
    );
}

/// A.17: an annotation's category is written to the file and read back after a save and a reopen.
#[test]
fn a_category_survives_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
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
    highlight.category = Some("evidence".into());
    Operation::new(
        "Highlight",
        vec![Change::CreateAnnotation(highlight.clone())],
    )
    .apply(&mut doc)
    .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let found = read.iter().find(|a| a.id == highlight.id).expect("kept");
    assert_eq!(found.category.as_deref(), Some("evidence"));
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

/// A.7: a callout's leader line is written from its tip, with an arrow there; moving its box keeps the tip in place,
/// through a save and a reopen.
#[test]
fn moving_a_callout_s_box_keeps_its_tip_in_place() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let tip = PagePoint { x: 100.0, y: 300.0 };
    let mut callout = Annotation::new(
        0,
        Shape::Callout(
            PageRect {
                x0: 160.0,
                y0: 200.0,
                x1: 360.0,
                y1: 230.0,
            },
            tip,
        ),
        Style {
            color: None,
            opacity: 1.0,
            width: 1.0,
        },
        "Ada",
    );
    callout.contents = "This figure".into();
    callout.ends = [LineEnd::OpenArrow, LineEnd::None];
    Operation::new("Callout", vec![Change::CreateAnnotation(callout.clone())])
        .apply(&mut doc)
        .expect("apply");
    let moved = PageRect {
        x0: 300.0,
        y0: 500.0,
        x1: 500.0,
        y1: 530.0,
    };
    callout.shape = Shape::Callout(moved, tip);
    Operation::new("Move", vec![Change::UpdateAnnotation(callout.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read");
    assert!(
        bytes.windows(19).any(|w| w == b"/IT/FreeTextCallout"),
        "the intent Acrobat reads"
    );
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0)
        .expect("read")
        .into_iter()
        .find(|a| a.id == callout.id)
        .expect("kept");
    let Shape::Callout(rect, read_tip) = read.shape else {
        panic!("a callout, not {:?}", read.shape);
    };
    assert!(
        (read_tip.x - tip.x).abs() < 0.01 && (read_tip.y - tip.y).abs() < 0.01,
        "the tip stays: {read_tip:?}"
    );
    assert!(
        (rect.x0 - moved.x0).abs() < 0.01 && (rect.y1 - moved.y1).abs() < 0.01,
        "the box moved: {rect:?}"
    );
    assert_eq!(read.ends[0], LineEnd::OpenArrow);
}

/// A.8: an arrow keeps its endings through a save and a reopen, written as `/LE`, which Acrobat draws the same.
#[test]
fn an_arrow_keeps_its_ends_through_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut arrow = Annotation::new(
        0,
        Shape::Line(
            PagePoint { x: 100.0, y: 300.0 },
            PagePoint { x: 300.0, y: 200.0 },
        ),
        Style {
            color: Some([0.8, 0.1, 0.1]),
            opacity: 1.0,
            width: 2.0,
        },
        "Ada",
    );
    arrow.ends = [LineEnd::Circle, LineEnd::ClosedArrow];
    Operation::new("Arrow", vec![Change::CreateAnnotation(arrow.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read");
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(
        has(b"/LE[/Circle/ClosedArrow]"),
        "the endings Acrobat draws"
    );
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0)
        .expect("read")
        .into_iter()
        .find(|a| a.id == arrow.id)
        .expect("kept");
    assert_eq!(read.ends, arrow.ends);
    assert_eq!(read.shape, arrow.shape);
}

#[test]
fn rectangles_and_ellipses_keep_fill_border_and_dash_through_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let rect = PageRect {
        x0: 100.0,
        y0: 200.0,
        x1: 260.0,
        y1: 300.0,
    };
    let shape = |boxed| {
        Annotation::new(
            0,
            Shape::Boxed(boxed, rect),
            Style {
                color: Some([0.1, 0.2, 0.8]),
                opacity: 1.0,
                width: 4.0,
            },
            "Ada",
        )
    };
    let mut rectangle = shape(Boxed::Square);
    rectangle.fill = Some([1.0, 0.96, 0.62]);
    rectangle.dash = vec![6.0, 4.0];
    let ellipse = shape(Boxed::Circle);
    Operation::new(
        "Shapes",
        vec![
            Change::CreateAnnotation(rectangle.clone()),
            Change::CreateAnnotation(ellipse.clone()),
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read");
    assert!(
        bytes.windows(4).any(|w| w == b"/S/D"),
        "a dashed border, as Acrobat reads it"
    );
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let find = |id: &str| read.iter().find(|a| a.id == id).expect("kept");
    let (read_rectangle, read_ellipse) = (find(&rectangle.id), find(&ellipse.id));
    assert!(matches!(
        read_rectangle.shape,
        Shape::Boxed(Boxed::Square, _)
    ));
    assert_eq!(read_rectangle.fill, rectangle.fill);
    assert_eq!(read_rectangle.dash, rectangle.dash);
    assert_eq!(read_rectangle.style.width, 4.0);
    assert!(matches!(read_ellipse.shape, Shape::Boxed(Boxed::Circle, _)));
    assert_eq!(read_ellipse.fill, None, "no fill stays none");
    assert!(read_ellipse.dash.is_empty(), "solid stays solid");
}

#[test]
fn a_cloud_keeps_its_border_effect_through_save_and_reopen() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let corners = vec![
        PagePoint { x: 100.0, y: 200.0 },
        PagePoint { x: 260.0, y: 210.0 },
        PagePoint { x: 220.0, y: 320.0 },
        PagePoint { x: 120.0, y: 300.0 },
    ];
    let mut cloud = Annotation::new(
        0,
        Shape::Path(Path::Polygon, corners.clone()),
        Style {
            color: Some([0.8, 0.1, 0.1]),
            opacity: 1.0,
            width: 2.0,
        },
        "Ada",
    );
    cloud.cloud = Some(1.0);
    let polyline = Annotation::new(
        0,
        Shape::Path(Path::PolyLine, corners.clone()),
        Style::default(),
        "Ada",
    );
    Operation::new(
        "Cloud",
        vec![
            Change::CreateAnnotation(cloud.clone()),
            Change::CreateAnnotation(polyline.clone()),
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let bytes = std::fs::read(&scratch.0).expect("read");
    assert!(
        bytes.windows(4).any(|w| w == b"/S/C"),
        "a cloudy border effect, as Acrobat reads it"
    );
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let find = |id: &str| read.iter().find(|a| a.id == id).expect("kept");
    let (read_cloud, read_polyline) = (find(&cloud.id), find(&polyline.id));
    assert_eq!(read_cloud.cloud, Some(1.0));
    assert_eq!(read_cloud.shape, cloud.shape);
    assert_eq!(read_polyline.cloud, None);
    assert_eq!(read_polyline.shape, polyline.shape);
}

#[test]
fn a_lock_and_an_author_keep_through_save_and_reopen_and_other_flags_stay() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut square = Annotation::new(
        0,
        Shape::Boxed(
            Boxed::Square,
            PageRect {
                x0: 100.0,
                y0: 100.0,
                x1: 200.0,
                y1: 160.0,
            },
        ),
        Style {
            color: Some([0.1, 0.3, 0.8]),
            opacity: 0.5,
            width: 2.0,
        },
        "Ada",
    );
    square.locked = true;
    Operation::new("Lock", vec![Change::CreateAnnotation(square.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let read = read.iter().find(|a| a.id == square.id).expect("kept");
    assert!(read.locked, "locked, as other readers see it");
    assert_eq!(read.author, "Ada");
    assert!((read.style.opacity - 0.5).abs() < 1e-3);
    // Unlocked again, it reads unlocked; MuPDF's Print flag, set on every new annotation, stays.
    let mut doc = reopened;
    let mut open = read.clone();
    open.locked = false;
    open.author = "Grace".to_owned();
    Operation::new("Unlock", vec![Change::UpdateAnnotation(open.clone())])
        .apply(&mut doc)
        .expect("apply");
    let again = page_annotations(&doc, 0).expect("read");
    let again = again.iter().find(|a| a.id == square.id).expect("kept");
    assert!(!again.locked);
    assert_eq!(again.author, "Grace");
    let bytes = {
        let scratch = Scratch::new();
        doc.save_pdf(&scratch.0).expect("save");
        std::fs::read(&scratch.0).expect("read")
    };
    assert!(
        bytes.windows(4).any(|w| w == b"/F 4") || bytes.windows(3).any(|w| w == b"/F4"),
        "the Print flag stays when the lock goes"
    );
}

#[test]
fn a_stamp_is_drawn_as_its_art_and_keeps_it_when_moved() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let rect = PageRect {
        x0: 100.0,
        y0: 100.0,
        x1: 200.0,
        y1: 140.0,
    };
    let mut stamp = Annotation::new(0, Shape::Boxed(Boxed::Stamp, rect), Style::default(), "Ada");
    stamp.icon = "CustomSeal".into();
    let art = excellent_mupdf::html_to_pdf(
        "<div style=\"background-color: #ff0000; height: 40pt\"></div>",
        "body { margin: 0 }",
        100.0,
    )
    .expect("art");
    let id = stamp.id.clone();
    Operation::new(
        "Stamp",
        vec![
            Change::CreateAnnotation(stamp.clone()),
            Change::StampArt {
                page: 0,
                id: id.clone(),
                art: excellent_mupdf::StampArt::Page(art),
            },
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    let red = |doc: &Document| {
        let pixmap = doc
            .load_page(0)
            .expect("page")
            .display_list(None)
            .expect("list")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render");
        let channels = pixmap.samples().len() / (pixmap.width() * pixmap.height()) as usize;
        pixmap
            .samples()
            .chunks(channels)
            .filter(|px| px[0] > 200 && px[1] < 60 && px[2] < 60)
            .count()
    };
    let drawn = red(&doc);
    assert!(
        drawn > 2000,
        "the art is drawn, not the name: {drawn} red pixels"
    );
    let moved = Annotation {
        shape: Shape::Boxed(
            Boxed::Stamp,
            PageRect {
                x0: 300.0,
                x1: 400.0,
                ..rect
            },
        ),
        ..stamp
    };
    Operation::new("Move", vec![Change::UpdateAnnotation(moved)])
        .apply(&mut doc)
        .expect("move");
    let kept = red(&doc);
    assert!(kept > 2000, "moving keeps the art: {kept}");
}

/// A.21: a reply and a review status are written as Acrobat writes them (`/IRT`; `/State` with `/StateModel`), read
/// back after a save and a reopen, aren't drawn or counted on the page, and go with their parent when it is deleted;
/// undo brings them back.
#[test]
fn a_thread_survives_save_and_goes_with_its_parent() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let rect = PageRect {
        x0: 300.0,
        y0: 300.0,
        x1: 340.0,
        y1: 340.0,
    };
    let parent = Annotation::new(
        0,
        Shape::Boxed(Boxed::Square, rect),
        Style::default(),
        "Ada",
    );
    let red = Style {
        color: Some([1.0, 0.0, 0.0]),
        opacity: 1.0,
        width: 1.0,
    };
    let mut reply = Annotation::new(0, Shape::Boxed(Boxed::Note, rect), red, "Grace");
    reply.reply_to = Some(parent.id.clone());
    reply.contents = "Agreed.".into();
    let mut status = Annotation::new(0, Shape::Boxed(Boxed::Note, rect), red, "Grace");
    status.reply_to = Some(parent.id.clone());
    status.state = Some(Status::Accepted);
    status.contents = "Accepted set by Grace".into();
    let pixels = |doc: &Document| {
        doc.load_page(0)
            .expect("page")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render")
            .samples()
            .to_vec()
    };
    Operation::new("Square", vec![Change::CreateAnnotation(parent.clone())])
        .apply(&mut doc)
        .expect("apply");
    let drawn = pixels(&doc);
    Operation::new(
        "Reply",
        vec![
            Change::CreateAnnotation(reply.clone()),
            Change::CreateAnnotation(status.clone()),
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    assert!(pixels(&doc) == drawn, "replies aren't drawn on the page");

    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let mut reopened = Document::open_pdf(&scratch.0).expect("reopen");
    assert!(
        page_annotations(&reopened, 0)
            .expect("read")
            .iter()
            .all(|a| !a.is_reply()),
        "replies aren't on the page"
    );
    let replies = page_replies(&reopened, 0).expect("replies");
    let read = |id: &str| replies.iter().find(|a| a.id == id).expect("kept");
    assert_eq!(
        read(&reply.id).reply_to.as_deref(),
        Some(parent.id.as_str())
    );
    assert_eq!(read(&reply.id).contents, "Agreed.");
    assert_eq!(read(&reply.id).state, None);
    assert_eq!(read(&status.id).state, Some(Status::Accepted));
    assert_eq!(read(&status.id).author, "Grace");

    Operation::new(
        "Delete",
        vec![Change::DeleteAnnotation {
            page: 0,
            id: parent.id.clone(),
        }],
    )
    .apply(&mut reopened)
    .expect("delete");
    assert!(
        page_replies(&reopened, 0).expect("replies").is_empty(),
        "the thread goes too"
    );
    reopened.undo().expect("undo");
    assert_eq!(
        page_replies(&reopened, 0).expect("replies").len(),
        2,
        "undo brings it back"
    );
}

/// A.22: an annotation on a layer is hidden with it; the file says so as other readers read it (`/OCProperties /D
/// /OFF`), reopened; undo shows it again, and taking it off the layer draws it whatever the layer.
#[test]
fn a_hidden_layer_hides_its_annotations_here_and_in_the_file() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let pixels = |doc: &Document| {
        doc.load_page(0)
            .expect("page")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render")
            .samples()
            .to_vec()
    };
    let blank = pixels(&doc);
    let rect = PageRect {
        x0: 300.0,
        y0: 300.0,
        x1: 360.0,
        y1: 360.0,
    };
    let mut square = Annotation::new(
        0,
        Shape::Boxed(Boxed::Square, rect),
        Style {
            color: Some([1.0, 0.0, 0.0]),
            opacity: 1.0,
            width: 4.0,
        },
        "Ada",
    );
    square.layer = Some("Ada".into());
    let hide = |visible: bool, locked: bool| {
        Operation::new(
            "Layer",
            vec![Change::SetLayer {
                name: "Ada".into(),
                visible,
                locked,
            }],
        )
    };
    Operation::new("Square", vec![Change::CreateAnnotation(square.clone())])
        .apply(&mut doc)
        .expect("apply");
    assert!(pixels(&doc) != blank, "a shown layer's annotation is drawn");
    let layer = |visible: bool, locked: bool| excellent_mupdf::Layer {
        name: "Ada".into(),
        visible,
        locked,
    };
    assert_eq!(doc.layers().expect("layers"), [layer(true, false)]);
    hide(false, false).apply(&mut doc).expect("hide");
    assert!(pixels(&doc) == blank, "a hidden layer's annotation isn't");

    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    assert_eq!(reopened.layers().expect("layers"), [layer(false, false)]);
    assert!(pixels(&reopened) == blank, "hidden as the file opens");
    let read = page_annotations(&reopened, 0).expect("read");
    let on = read.iter().find(|a| a.id == square.id).expect("the square");
    assert_eq!(on.layer.as_deref(), Some("Ada"));

    doc.undo().expect("undo");
    assert!(pixels(&doc) != blank, "undo shows it again");
    assert_eq!(doc.layers().expect("layers"), [layer(true, false)]);
    hide(false, true).apply(&mut doc).expect("hide and lock");
    assert_eq!(doc.layers().expect("layers"), [layer(false, true)]);
    let mut off = square;
    off.layer = None;
    Operation::new("Off the layer", vec![Change::UpdateAnnotation(off)])
        .apply(&mut doc)
        .expect("update");
    assert!(pixels(&doc) != blank, "off the layer, it is drawn");
    assert_eq!(doc.layers().expect("layers").len(), 1, "the layer stays");
}

/// A.24: annotations written as FDF read back on a copy of the document with their ids, pages, kinds and threads.
#[test]
fn annotations_go_out_and_back_as_fdf() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let mut square = Annotation::new(
        1,
        Shape::Boxed(
            Boxed::Square,
            PageRect {
                x0: 50.0,
                y0: 60.0,
                x1: 150.0,
                y1: 120.0,
            },
        ),
        Style::default(),
        "Ada",
    );
    square.contents = "Look here".into();
    let mut reply = Annotation::new(
        1,
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
        "Grace",
    );
    reply.reply_to = Some(square.id.clone());
    reply.contents = "Agreed".into();
    Operation::new(
        "Square",
        vec![
            Change::CreateAnnotation(square.clone()),
            Change::CreateAnnotation(reply.clone()),
        ],
    )
    .apply(&mut doc)
    .expect("apply");
    let fdf = doc.to_fdf().expect("write FDF");
    assert!(fdf.starts_with(b"%FDF-"), "an FDF header");

    let fresh = Document::open_pdf(corpus()).expect("open a fresh copy");
    let read = fdf_annotations(&fresh, &fdf).expect("read FDF");
    let came = read.iter().find(|a| a.id == square.id).expect("the square");
    assert_eq!((came.page, &came.shape), (1, &square.shape));
    assert_eq!(came.contents, "Look here");
    assert_eq!(came.author, "Ada");
    let answer = read.iter().find(|a| a.id == reply.id).expect("the reply");
    assert_eq!(
        answer.reply_to.as_deref(),
        Some(square.id.as_str()),
        "the thread is kept"
    );
    assert!(fdf_annotations(&fresh, b"not an fdf file").is_err());
}

/// A.25: a flattened copy shows the same pixels and has no annotations left; the open document keeps its own.
#[test]
fn a_flattened_copy_looks_the_same_and_holds_no_annotations() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let square = Annotation::new(
        0,
        Shape::Boxed(
            Boxed::Square,
            PageRect {
                x0: 50.0,
                y0: 60.0,
                x1: 150.0,
                y1: 120.0,
            },
        ),
        Style {
            color: Some([1.0, 0.0, 0.0]),
            opacity: 1.0,
            width: 3.0,
        },
        "Ada",
    );
    Operation::new("Square", vec![Change::CreateAnnotation(square.clone())])
        .apply(&mut doc)
        .expect("apply");
    let scratch = Scratch::new();
    doc.save_flattened(&scratch.0, excellent_mupdf::Forms::Keep)
        .expect("flatten");

    let flat = Document::open_pdf(&scratch.0).expect("open the copy");
    for page in 0..PAGES {
        assert!(
            page_annotations(&flat, page).expect("read").is_empty(),
            "page {page} keeps no annotations"
        );
    }
    let render = |document: &Document| {
        document
            .load_page(0)
            .expect("page")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render")
            .samples()
            .to_vec()
    };
    let (before, after) = (render(&doc), render(&flat));
    assert_eq!(before.len(), after.len(), "the same page size");
    // Baking may round an edge's antialiasing; the drawing itself must match.
    let differing = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a.abs_diff(**b) > 8)
        .count();
    assert!(
        differing * 1000 < before.len(),
        "{differing} of {} samples differ",
        before.len()
    );
    assert!(
        page_annotations(&doc, 0)
            .expect("read")
            .iter()
            .any(|a| a.id == square.id),
        "the open document keeps its annotation"
    );
}

/// H.9: a pressure pen's stroke is saved point by point and drawn as it was written, thin where it started and broad
/// where it ended, by MuPDF now and from the file once it is reopened; the widths come back with it.
#[test]
fn a_pressure_stroke_is_saved_and_drawn_as_written() {
    let mut doc = Document::open_pdf(corpus()).expect("open");
    let stroke: Vec<PagePoint> = (0..=20)
        .map(|i| PagePoint {
            x: 100.0 + i as f32 * 10.0,
            y: 400.0,
        })
        .collect();
    // From 1 point wide to 15; the style's one width, the mean, is 8.
    let widths: Vec<f32> = (0..=20).map(|i| 1.0 + i as f32 * 0.7).collect();
    let mut ink = Annotation::new(
        0,
        Shape::Ink(vec![stroke]),
        Style {
            color: Some([0.85, 0.0, 0.0]),
            opacity: 1.0,
            width: 8.0,
        },
        "Ada",
    );
    ink.widths = vec![widths.clone()];
    Operation::new("Ink", vec![Change::CreateAnnotation(ink.clone())])
        .apply(&mut doc)
        .expect("apply");
    // How many pixels down the column at `x` the stroke covers.
    let thickness = |doc: &Document, x: usize| {
        let pixmap = doc
            .load_page(0)
            .expect("page")
            .render(
                &excellent_mupdf::Matrix::IDENTITY,
                excellent_mupdf::Background::White,
            )
            .expect("render");
        let width = pixmap.width() as usize;
        let channels = pixmap.samples().len() / (width * pixmap.height() as usize);
        (370..430)
            .filter(|&y| {
                let px = &pixmap.samples()[(y * width + x) * channels..][..3];
                px[0] > 150 && px[1] < 110 && px[2] < 110
            })
            .count()
    };
    let (thin, broad) = (thickness(&doc, 110), thickness(&doc, 290));
    assert!(
        thin <= 4 && broad >= 12,
        "thin at the start ({thin} px), broad at the end ({broad} px)"
    );

    let scratch = Scratch::new();
    doc.save_pdf(&scratch.0).expect("save");
    let reopened = Document::open_pdf(&scratch.0).expect("reopen");
    let read = page_annotations(&reopened, 0).expect("read");
    let back = read.iter().find(|a| a.id == ink.id).expect("the stroke");
    assert_eq!(back.widths.len(), 1, "one stroke's widths");
    assert!(
        back.widths[0]
            .iter()
            .zip(&widths)
            .all(|(a, b)| (a - b).abs() < 1e-3),
        "{:?}",
        back.widths
    );
    let (thin, broad) = (thickness(&reopened, 110), thickness(&reopened, 290));
    assert!(
        thin <= 4 && broad >= 12,
        "from the file: {thin} px and {broad} px"
    );
}
