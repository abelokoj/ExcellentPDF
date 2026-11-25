//! O.23's "Done when": a 600 DPI snapshot of a vector figure is sharp. A region of vector text is rendered at 72 and
//! at 600 DPI; at 600 its edges are a pixel or two of grey round solid ink, so far fewer of its inked pixels are grey,
//! and it is the size the resolution says, and saves as PNG.

use std::path::Path;

use excellent_features::snapshot::Region;
use excellent_mupdf::{Document, PageRect, Pixmap};

/// How many of the inked pixels are mid-grey (anti-aliased edge), as a share, and the picture's size.
fn grey_share(pixmap: &Pixmap) -> (f32, u32, u32) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let channels = pixmap.samples().len() / (w * h) as usize;
    let (mut inked, mut grey) = (0_u32, 0_u32);
    for px in pixmap.samples().chunks(channels) {
        let level = u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2]);
        if level < 3 * 230 {
            inked += 1;
            if level > 3 * 60 {
                grey += 1;
            }
        }
    }
    (grey as f32 / inked.max(1) as f32, w, h)
}

#[test]
fn a_600_dpi_snapshot_of_vector_text_is_sharp() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    let document = Document::open_pdf(&corpus).expect("open");
    // The top of the first page: its title and first lines, vector glyphs.
    let region = Region {
        page: 0,
        rect: PageRect {
            x0: 50.0,
            y0: 40.0,
            x1: 410.0,
            y1: 220.0,
        },
    };
    let (coarse, ..) = grey_share(&region.render(&document, 72).expect("72 DPI"));
    let (fine, w, h) = grey_share(&region.render(&document, 600).expect("600 DPI"));
    let png = region.render_png(&document, 600).expect("as PNG");
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "a PNG file");
    // 360 by 180 points at 600 DPI, the edges rounded out to whole pixels.
    assert_eq!(region.pixels(600), (3000, 1500));
    assert!(w.abs_diff(3000) <= 1 && h.abs_diff(1500) <= 1, "{w} x {h}");
    assert!(
        fine < 0.35 && fine < coarse * 0.6,
        "at 600 DPI {:.0} % of the ink is edge grey, at 72 DPI {:.0} %",
        fine * 100.0,
        coarse * 100.0
    );
}
