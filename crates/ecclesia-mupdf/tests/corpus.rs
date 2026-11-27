//! T4 (plan K.3): page 1 of every corpus file renders to PNG and gives its text; the other types work end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use ecclesia_mupdf::{
    AnnotKind, Background, Cookie, DeviceRect, DisplayList, Document, Matrix, OutlineItem, Pixmap,
    Reflow, Writer,
};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn corpus(name: &str) -> PathBuf {
    corpus_dir().join(name)
}

fn out_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ecclesia-mupdf-corpus");
    fs::create_dir_all(&dir).expect("can create the test output folder");
    dir
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Dark pixels in an opaque RGB pixmap: drawn ink.
fn has_ink(pixmap: &Pixmap) -> bool {
    pixmap.samples().iter().any(|&b| b < 128)
}

#[test]
fn every_corpus_file_renders_page_one_to_png_and_gives_text() {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("tests/corpus exists")
        .map(|entry| entry.expect("readable corpus entry").path())
        .filter(|path| path.file_name().is_some_and(|name| name != "README.md"))
        .collect();
    files.sort();
    assert!(files.len() >= 2, "corpus has files: {files:?}");
    // R.28: every format MuPDF opens besides PDF has a file here.
    for format in [
        "cbz", "epub", "fb2", "html", "md", "mobi", "png", "svg", "xps",
    ] {
        assert!(
            files
                .iter()
                .any(|path| path.extension().is_some_and(|ext| ext == format)),
            "the corpus has a .{format} file"
        );
    }

    for path in files {
        let name = path
            .file_name()
            .expect("file")
            .to_string_lossy()
            .into_owned();
        let doc = Document::open(&path).unwrap_or_else(|e| panic!("{name}: open: {e}"));
        assert!(doc.page_count().expect("count") > 0, "{name}: has pages");
        let page = doc
            .load_page(0)
            .unwrap_or_else(|e| panic!("{name}: load page 1: {e}"));

        let pixmap = page
            .render(&Matrix::scale(1.5, 1.5), Background::White)
            .unwrap_or_else(|e| panic!("{name}: render: {e}"));
        assert!(has_ink(&pixmap), "{name}: page 1 draws something");
        let png = pixmap.to_png().expect("PNG");
        assert!(png.starts_with(PNG_SIGNATURE), "{name}: is a PNG");
        fs::write(out_dir().join(format!("{name}.png")), &png).expect("write PNG");

        // Pictures and the hand-written XPS page (a shape, no font) have no text.
        let pictures = ["png", "cbz", "xps"];
        if !path
            .extension()
            .is_some_and(|ext| pictures.iter().any(|p| ext == *p))
        {
            let text = page.text(None).and_then(|t| t.text()).expect("text");
            assert!(!text.trim().is_empty(), "{name}: page 1 has text");
        }
    }
}

/// The first "Mark c.nn" on page `index`: every sentence of the `formats.*` books starts with one.
fn page_text(doc: &Document, index: usize) -> String {
    doc.load_page(index)
        .expect("page")
        .text(None)
        .and_then(|t| t.text())
        .expect("text")
}

fn first_mark(doc: &Document, index: usize) -> String {
    let text = page_text(doc, index);
    let words: Vec<&str> = text.split_whitespace().collect();
    let at = words
        .windows(2)
        .position(|pair| pair[0] == "Mark")
        .unwrap_or_else(|| panic!("page {index} has a mark: {text:?}"));
    words[at + 1].to_owned()
}

#[test]
fn reflowing_keeps_the_reading_position() {
    for name in [
        "formats.epub",
        "formats.fb2",
        "formats.mobi",
        "formats.html",
        "formats.md",
    ] {
        let mut doc = Document::open(corpus(name)).expect("open");
        assert!(doc.is_reflowable().expect("reflowable"), "{name} reflows");
        let small = Reflow {
            width: 420.0,
            height: 595.0,
            font_size: 9.0,
        };
        assert_eq!(doc.reflow(small, None).expect("reflow"), None);
        let before = doc.page_count().expect("count");
        assert!(before >= 3, "{name}: {before} pages at 9 pt");
        let page = before / 2;
        let mark = first_mark(&doc, page);

        let large = Reflow {
            font_size: 18.0,
            ..small
        };
        let kept = doc
            .reflow(large, Some(page))
            .expect("reflow")
            .expect("the page's text is found again");
        assert!(
            doc.page_count().expect("count") > before,
            "{name}: larger text, more pages"
        );
        // Larger text never moves text to an earlier page.
        assert!(kept >= page, "{name}: page {page} moved to {kept}");
        // The page's first words (perhaps the end of the sentence before the mark) open page `kept`, so its first
        // mark is on that page or, cut by the larger text, opens the next.
        let text = page_text(&doc, kept);
        assert!(
            text.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["Mark", mark.as_str()])
                || first_mark(&doc, kept + 1) == mark,
            "{name}: {mark} on page {kept}: {text:?}"
        );
    }

    let mut pdf = Document::open(corpus("zlib-manual.pdf")).expect("open");
    assert!(!pdf.is_reflowable().expect("reflowable"));
    let size = Reflow {
        width: 100.0,
        height: 100.0,
        font_size: 9.0,
    };
    pdf.reflow(size, None).expect("a fixed layout ignores it");
    assert_eq!(pdf.page_count().expect("count"), 2);
    assert!(pdf.reflow(Reflow { width: 0.0, ..size }, None).is_err());
}

#[test]
fn renders_with_the_page_rect_scaled() {
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let bounds = page.bounds().expect("bounds");
    assert_eq!(
        (bounds.width(), bounds.height()),
        (612.0, 792.0),
        "US Letter"
    );
    let pixmap = page
        .render(&Matrix::scale(2.0, 2.0), Background::Transparent)
        .expect("render");
    assert_eq!((pixmap.width(), pixmap.height()), (1224, 1584));
    assert!(pixmap.has_alpha());
    assert_eq!(pixmap.components(), 4);
    assert_eq!(pixmap.samples().len(), pixmap.stride() * 1584);
}

#[test]
fn unembedded_base14_fonts_are_drawn() {
    let doc = Document::open(corpus("base14-outline-annot.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let pixmap = page
        .render(&Matrix::IDENTITY, Background::White)
        .expect("render");
    // Built-in fonts first, then the system font folder: either way the text must be drawn, not left blank.
    assert!(has_ink(&pixmap), "Helvetica text was drawn");
    let text = page.text(None).and_then(|t| t.text()).expect("text");
    assert!(text.contains("Base fourteen page one"), "{text}");
}

#[test]
fn a_text_layout_covers_its_characters_in_order() {
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let stext = page.text(None).expect("text");
    let layout = stext.layout();
    assert!(!layout.chars.is_empty(), "the manual's first page has text");
    // Lines cover the characters, and blocks the lines, back to back from the start.
    let mut next = 0;
    for line in &layout.lines {
        assert_eq!(line.chars.start, next, "lines are contiguous");
        assert!(line.block < layout.blocks.len());
        next = line.chars.end;
    }
    assert_eq!(next, layout.chars.len());
    let mut next = 0;
    for block in &layout.blocks {
        assert_eq!(block.lines.start, next, "blocks are contiguous");
        next = block.lines.end;
    }
    assert_eq!(next, layout.lines.len());
    // "ver-" / "sion": MuPDF joined the word and marked the line.
    assert!(layout.lines.iter().any(|line| line.joined));
    let first = &layout.chars[0].quad;
    assert!(
        first.ur.x > first.ul.x && first.ll.y > first.ul.y,
        "a real box: {first:?}"
    );
}

#[test]
fn reads_the_outline() {
    let doc = Document::open(corpus("base14-outline-annot.pdf")).expect("open");
    let outline = doc.outline().expect("outline");
    let titles: Vec<(&str, Option<usize>)> = outline
        .iter()
        .map(|item| (item.title.as_str(), item.page))
        .collect();
    assert_eq!(titles, [("Page one", Some(0)), ("Page two", Some(1))]);
    assert!(outline.iter().all(|item| item.children.is_empty()));

    let none = Document::open(corpus("zlib-manual.pdf")).expect("open");
    assert_eq!(none.outline().expect("outline"), Vec::<OutlineItem>::new());
}

#[test]
fn reads_annotations() {
    let doc = Document::open(corpus("base14-outline-annot.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let annots = page.annots().expect("annots");
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].kind().expect("kind"), AnnotKind::Text);
    assert_eq!(annots[0].contents().expect("contents"), "A sticky note");
    let rect = annots[0].rect().expect("rect");
    assert!(rect.width() > 0.0 && rect.height() > 0.0, "{rect:?}");
    assert!(
        doc.load_page(1)
            .expect("page 2")
            .annots()
            .expect("annots")
            .is_empty()
    );
}

#[test]
fn display_list_renders_like_the_page_on_any_thread() {
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let ctm = Matrix::scale(0.75, 0.75);
    let direct = page.render(&ctm, Background::White).expect("render");
    let list = Arc::new(page.display_list(None).expect("list"));
    drop(page);

    let renders: Vec<Pixmap> = (0..3)
        .map(|_| {
            let list = Arc::clone(&list);
            thread::spawn(move || {
                list.render(&ctm, Background::White)
                    .expect("render on a worker")
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .collect();
    for pixmap in &renders {
        assert_eq!(
            pixmap.samples(),
            direct.samples(),
            "same pixels from the list"
        );
    }

    // A tile: the top-left 100×50 pixels are the same as that corner of the whole render.
    let tile = list
        .render_area(
            &ctm,
            DeviceRect {
                x0: 0,
                y0: 0,
                x1: 100,
                y1: 50,
            },
            Background::White,
            None,
        )
        .expect("tile");
    assert_eq!((tile.width(), tile.height()), (100, 50));
    for row in 0..50 {
        let whole = &direct.samples()[row * direct.stride()..][..300];
        let part = &tile.samples()[row * tile.stride()..][..300];
        assert_eq!(whole, part, "row {row}");
    }
}

#[test]
fn an_aborted_cookie_cancels_the_work() {
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("open");
    let page = doc.load_page(0).expect("page");
    let list = page.display_list(None).expect("list");
    let area = DeviceRect {
        x0: 0,
        y0: 0,
        x1: 64,
        y1: 64,
    };

    let live = Cookie::new();
    assert!(page.display_list(Some(&live)).is_ok());
    assert!(page.text(Some(&live)).is_ok());
    assert!(
        list.render_area(&Matrix::IDENTITY, area, Background::White, Some(&live))
            .is_ok()
    );

    let cancelled = Cookie::new();
    cancelled.abort();
    let aborted =
        |result: Result<(), ecclesia_mupdf::MuError>| result.expect_err("cancelled").is_aborted();
    assert!(aborted(page.display_list(Some(&cancelled)).map(drop)));
    assert!(aborted(page.text(Some(&cancelled)).map(drop)));
    assert!(aborted(
        list.render_area(&Matrix::IDENTITY, area, Background::White, Some(&cancelled))
            .map(drop)
    ));
}

#[test]
fn opens_from_bytes() {
    let bytes = fs::read(corpus("zlib-manual.pdf")).expect("read");
    let doc = Document::from_bytes(&bytes, "application/pdf").expect("open from memory");
    drop(bytes);
    assert_eq!(doc.page_count().expect("count"), 2, "the bytes were copied");
    assert!(
        Document::from_bytes(b"not a pdf", "application/pdf")
            .and_then(|d| d.page_count())
            .is_err()
    );
}

#[test]
fn writes_pages_to_a_new_pdf() {
    let source = Document::open(corpus("zlib-manual.pdf")).expect("open");
    let out = out_dir().join("rewritten.pdf");
    let mut writer = Writer::new(&out, Some("pdf"), None).expect("writer");
    for index in 0..source.page_count().expect("count") {
        writer
            .write_page(&source.load_page(index).expect("page"))
            .expect("write");
    }
    writer.close().expect("close");

    let copy = Document::open(&out).expect("reopen");
    assert_eq!(copy.page_count().expect("count"), 2);
    let text = copy
        .load_page(0)
        .and_then(|p| p.text(None))
        .and_then(|t| t.text())
        .expect("text");
    assert!(text.contains("zlib"), "{text:.200}");
}

#[test]
fn errors_carry_mupdf_messages() {
    let missing = Document::open(corpus("missing.pdf")).expect_err("no such file");
    assert!(!missing.message().is_empty());
    let junk = out_dir().join("junk.pdf");
    fs::write(&junk, b"this is not a PDF").expect("write junk");
    let not_pdf = Document::open(&junk)
        .and_then(|doc| doc.page_count())
        .expect_err("junk is not a PDF");
    assert!(!not_pdf.to_string().is_empty());
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("open");
    assert!(doc.load_page(99).is_err(), "page past the end");
}

#[test]
fn thread_rules_are_in_the_types() {
    fn send<T: Send>() {}
    fn send_sync<T: Send + Sync>() {}
    send::<Document>();
    send_sync::<DisplayList>();
    send_sync::<Pixmap>();
    // `Page`, `StextPage`, `Annot` and `Writer` are not `Send`: they hold raw MuPDF pointers and have no `Send`
    // impl, which the compiler enforces (a `send::<Page<'_>>()` call does not compile).
}

/// A one-page PDF that reads right to left, with a crop box inside its media box and a quarter turn.
const RTL_PDF: &[u8] = b"%PDF-1.7
1 0 obj <</Type /Catalog /Pages 2 0 R /ViewerPreferences <</Direction /R2L>>>> endobj
2 0 obj <</Type /Pages /Kids [3 0 R] /Count 1>> endobj
3 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /CropBox [10 20 110 220] /Rotate 90>> endobj
trailer <</Root 1 0 R>>
%%EOF
";

#[test]
fn page_boxes_are_read_without_loading_and_match_the_page() {
    let doc = Document::from_bytes(RTL_PDF, "application/pdf").expect("open");
    let size = |page_box| {
        let r = doc.page_box(0, page_box).expect("box");
        (r.width(), r.height())
    };
    assert_eq!(
        size(ecclesia_mupdf::PageBox::Media),
        (300.0, 200.0),
        "turned a quarter"
    );
    assert_eq!(size(ecclesia_mupdf::PageBox::Crop), (200.0, 100.0));
    let crop = doc
        .page_box(0, ecclesia_mupdf::PageBox::Crop)
        .expect("crop box");
    assert_eq!(
        crop,
        doc.load_page(0).expect("page").bounds().expect("bounds")
    );
    assert!(doc.reads_right_to_left().expect("direction"));

    let manual = Document::open(corpus("zlib-manual.pdf")).expect("open");
    assert!(!manual.reads_right_to_left().expect("direction"));
    for index in 0..manual.page_count().expect("count").min(5) {
        assert_eq!(
            manual
                .page_box(index, ecclesia_mupdf::PageBox::Crop)
                .expect("box"),
            manual
                .load_page(index)
                .expect("page")
                .bounds()
                .expect("bounds"),
            "page {index}"
        );
    }
}

/// A six-page PDF numbered i–iv, then 1–2 (R.6).
const LABELLED_PDF: &[u8] = b"%PDF-1.7
1 0 obj <</Type /Catalog /Pages 2 0 R /PageLabels <</Nums [0 <</S /r>> 4 <</S /D>>]>>>> endobj
2 0 obj <</Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R 7 0 R 8 0 R] /Count 6>> endobj
3 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
4 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
5 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
6 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
7 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
8 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
trailer <</Root 1 0 R>>
%%EOF
";

#[test]
fn page_labels_are_read_and_absent_labels_are_none() {
    let doc = Document::from_bytes(LABELLED_PDF, "application/pdf").expect("open");
    assert_eq!(
        doc.page_labels().expect("labels"),
        Some(
            ["i", "ii", "iii", "iv", "1", "2"]
                .map(String::from)
                .to_vec()
        )
    );
    let manual = Document::open(corpus("zlib-manual.pdf")).expect("open");
    assert_eq!(manual.page_labels().expect("labels"), None);
}

/// A three-page PDF whose first page links to page 3 (with a point), a web page, a launch action and a script (R.7).
const LINKED_PDF: &[u8] = b"%PDF-1.7
1 0 obj <</Type /Catalog /Pages 2 0 R>> endobj
2 0 obj <</Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3>> endobj
3 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Annots [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R]>> endobj
4 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
5 0 obj <</Type /Page /Parent 2 0 R /MediaBox [0 0 200 300]>> endobj
6 0 obj <</Type /Annot /Subtype /Link /Rect [10 10 100 50] /Dest [5 0 R /XYZ 20 200 0]>> endobj
7 0 obj <</Type /Annot /Subtype /Link /Rect [10 60 100 100] /A <</S /URI /URI (https://example.com/a)>>>> endobj
8 0 obj <</Type /Annot /Subtype /Link /Rect [10 110 100 150] /A <</S /Launch /F (calc.exe)>>>> endobj
9 0 obj <</Type /Annot /Subtype /Link /Rect [10 160 100 200] /A <</S /URI /URI (javascript:app.alert(1))>>>> endobj
10 0 obj <</Type /Annot /Subtype /Link /Rect [10 210 100 250] /A <</S /JavaScript /JS (app.alert(2))>>>> endobj
trailer <</Root 1 0 R>>
%%EOF
";

#[test]
fn links_are_read_and_resolved() {
    let doc = Document::from_bytes(LINKED_PDF, "application/pdf").expect("open");
    let page = doc.load_page(0).expect("page");
    let links = page.links().expect("links");
    // MuPDF makes a launch action a `file:` link and leaves out /JavaScript actions: four links of five.
    assert_eq!(links.len(), 4);
    let first = &links[0];
    // PDF space is upside down: the rect 10..50 from the bottom of a 300-point page is 250..290 from the top.
    assert_eq!((first.rect.y0, first.rect.y1), (250.0, 290.0));
    let target = first.target.expect("an internal link");
    assert_eq!(target.page, 2);
    let point = target.point.expect("the link names a point");
    assert_eq!((point.x, point.y), (20.0, 100.0));
    assert_eq!(links[1].uri, "https://example.com/a");
    assert_eq!(links[1].target, None);
    assert!(links[2].uri.starts_with("file:calc.exe"));
    assert_eq!(links[3].uri, "javascript:app.alert(1)");
    assert!(links[2..].iter().all(|link| link.target.is_none()));
    assert!(
        doc.load_page(1)
            .expect("page")
            .links()
            .expect("links")
            .is_empty()
    );
}

/// R.7: links in an EPUB (to a chapter, and to the web) and on an XPS page are read like a PDF's.
#[test]
fn epub_and_xps_links_are_read_and_resolved() {
    let doc = Document::open(corpus("formats.epub")).expect("open");
    let links = doc.load_page(0).expect("page").links().expect("links");
    assert_eq!(links.len(), 2, "{links:?}");
    // MuPDF lists a page's links in no promised order.
    let chapter = links
        .iter()
        .find_map(|link| link.uri.ends_with("c2.xhtml").then_some(link.target?.page))
        .unwrap_or_else(|| panic!("the chapter link goes inside the book: {links:?}"));
    assert!(
        first_mark(&doc, chapter).starts_with("3."),
        "page {chapter} starts the last chapter"
    );
    let web = links
        .iter()
        .find(|link| link.uri == "https://example.com/")
        .expect("the web link");
    assert_eq!(web.target, None);

    let xps = Document::open(corpus("formats.xps")).expect("open");
    let links = xps.load_page(0).expect("page").links().expect("links");
    assert_eq!(links.len(), 1, "{links:?}");
    assert_eq!(links[0].uri, "https://example.com/");
    assert!(links[0].rect.x1 > links[0].rect.x0);
}

/// R.14: a page's pictures are listed with where they land and their own size, and sample in RGB.
#[test]
fn a_page_lists_its_pictures_and_samples_them_in_rgb() {
    let doc = Document::open(corpus("zlib-manual.pdf")).expect("opens");
    let png = doc
        .load_page(0)
        .expect("page 1")
        .render(&Matrix::scale(0.5, 0.5), Background::White)
        .expect("renders")
        .to_png()
        .expect("PNG");
    // An image file opens as one page drawn by one picture.
    let picture = Document::from_bytes(&png, "png").expect("the PNG opens");
    let list = picture
        .load_page(0)
        .expect("its page")
        .display_list(None)
        .expect("list");
    let images = list.images(None).expect("listed");
    // Where a picture goes missing (macOS CI): whether the list and the page draw anything at all.
    let inked = |p: Result<Pixmap, _>| p.map(|p| p.samples().iter().filter(|&&b| b < 128).count());
    assert_eq!(
        images.len(),
        1,
        "one picture; the list's bounds {:?}, the PNG {} bytes, pages {:?}; dark samples drawn by the list {:?}, by          the page {:?}",
        list.bounds(),
        png.len(),
        picture.page_count(),
        inked(list.render(&Matrix::IDENTITY, Background::White)),
        inked(
            picture
                .load_page(0)
                .expect("its page")
                .render(&Matrix::IDENTITY, Background::White)
        ),
    );
    let image = &images[0];
    let page = list.bounds().expect("bounds");
    assert!(image.width() > 64 && image.height() > 64, "its own pixels");
    let bounds = image.bounds();
    assert!(
        (bounds.x0 - page.x0).abs() < 1.0 && (bounds.x1 - page.x1).abs() < 1.0,
        "it fills the page: {bounds:?} on {page:?}"
    );
    // Already decoded, it comes back at full size: MuPDF subsamples only formats that allow it.
    let sample = image.sample().expect("decodes");
    assert_eq!(
        (sample.width(), sample.height()),
        (image.width(), image.height())
    );
    assert_eq!(sample.components(), 3, "RGB, no alpha");
    assert!(
        sample.samples().iter().any(|&b| b < 128),
        "the page's text is in it"
    );
}
