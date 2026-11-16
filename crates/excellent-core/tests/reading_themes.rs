//! R.15's "done when": each reading theme renders the corpus with WCAG AA text contrast. A tile of real text is
//! rendered through the render pool in every built-in theme; its paper must be the theme's, and its darkest-to-paper
//! text must reach 4.5 : 1.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use excellent_core::darkmode::themes::{AA_TEXT, contrast};
use excellent_core::darkmode::{Pictures, ReadingTheme, Rgb};
use excellent_core::render::{Priority, Recolor, RenderPool};
use excellent_mupdf::{DeviceRect, Document, Matrix, Pixmap};

#[test]
fn each_reading_theme_renders_the_corpus_with_aa_text_contrast() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    let document = Arc::new(Mutex::new(Document::open(path).expect("open")));
    let pool = RenderPool::new(1);
    let list = Arc::new(
        pool.build_display_list(&document, 0, Default::default(), Priority::Visible)
            .wait()
            .expect("display list"),
    );
    // The first lines of text, at 1.5×.
    let ctm = Matrix::scale(1.5, 1.5);
    let area = DeviceRect {
        x0: 96,
        y0: 96,
        x1: 480,
        y1: 352,
    };
    for theme in ReadingTheme::ALL {
        let Some(palette) = theme.palette() else {
            continue;
        };
        let recolor = Recolor {
            colors: palette.page_colors(Pictures::Recolor),
            images: Arc::default(),
        };
        let tile = pool
            .render_tile(&list, ctm, area, Priority::Visible, Some(recolor))
            .wait()
            .expect("tile");
        let (paper, text) = paper_and_text(&tile);
        assert_eq!(paper, palette.paper, "{theme:?}: the paper is the theme's");
        let ratio = contrast(text, paper);
        assert!(
            ratio >= AA_TEXT,
            "{theme:?}: text {text:?} on {paper:?} is {ratio:.2} : 1"
        );
    }
}

/// The commonest colour of the tile (its paper) and the colour that stands out most from it (its text).
fn paper_and_text(tile: &Pixmap) -> (Rgb, Rgb) {
    let n = tile.components();
    let width = tile.width() as usize;
    let mut counts: HashMap<Rgb, usize> = HashMap::new();
    for row in tile.samples().chunks(tile.stride()) {
        for pixel in row[..width * n].chunks_exact(n) {
            *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
        }
    }
    let paper = *counts
        .iter()
        .max_by_key(|&(_, count)| count)
        .expect("the tile has pixels")
        .0;
    let text = *counts
        .keys()
        .max_by(|a, b| contrast(**a, paper).total_cmp(&contrast(**b, paper)))
        .expect("the tile has pixels");
    (paper, text)
}
