//! Printing (plan O.22), the rendering: which pages a print takes, where each lands on the paper, and the pages drawn
//! in bands at the printer's resolution, as SumatraPDF prints. The printer itself (its dialog and its device) is the
//! platform's ([`Sheets`] is what it offers); this module knows nothing of the OS.
//!
//! Every page is printed as an image, rendered here at the printer's own dots per inch, so what prints is what the
//! screen shows; there is no separate "print as image" path.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};

/// A band of a page as [`Sheets::band`] gets it.
pub use excellent_mupdf::Pixmap;
use excellent_mupdf::{Background, Cookie, DeviceRect, DisplayList, Matrix, MuError, PageRect};

use crate::document::{Document, SharedMupdf};

/// The most bytes one band of a page holds: a US Letter page at 600 dpi is about 100 MB of pixels, printed in a few
/// bands of this size.
const BAND_BYTES: usize = 16 << 20;

/// How a page is sized on the paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scaling {
    /// Grown or shrunk to fill the printable area.
    Fit,
    /// Shrunk to the printable area when larger, else printed at its own size.
    #[default]
    Shrink,
    /// At its own size, cut at the paper's edges when larger.
    Actual,
}

impl Scaling {
    /// Every choice, in the order the settings offer them.
    pub const ALL: [Self; 3] = [Self::Shrink, Self::Fit, Self::Actual];

    /// Its name in `settings.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Shrink => "shrink",
            Self::Actual => "actual",
        }
    }

    /// The choice a setting names, or the default.
    pub fn parse(name: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|scaling| Some(scaling.name()) == name)
            .unwrap_or_default()
    }
}

/// The paper as the printer reports it: the printable area in device pixels and its resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    /// Printable width, in pixels.
    pub width: u32,
    /// Printable height, in pixels.
    pub height: u32,
    /// Horizontal dots per inch.
    pub dpi_x: f32,
    /// Vertical dots per inch.
    pub dpi_y: f32,
}

/// Where a page of `bounds` (points) lands on `paper`: the map from the page to the printable area and the pixels it
/// covers there, clipped to the paper. A page whose shape differs from the paper's (landscape on portrait) is turned
/// a quarter, as SumatraPDF does, and every page is centred.
pub fn placement(bounds: PageRect, paper: Paper, scaling: Scaling) -> (Matrix, DeviceRect) {
    let (width, height) = (bounds.width().max(1.0), bounds.height().max(1.0));
    let turn = (width > height) != (paper.width > paper.height);
    let (across, down) = if turn {
        (height, width)
    } else {
        (width, height)
    };
    // At its own size a point is 1/72 inch.
    let (dx, dy) = (paper.dpi_x / 72.0, paper.dpi_y / 72.0);
    let fit = (paper.width as f32 / (across * dx)).min(paper.height as f32 / (down * dy));
    let scale = match scaling {
        Scaling::Fit => fit,
        Scaling::Shrink => fit.min(1.0),
        Scaling::Actual => 1.0,
    };
    let mut ctm = Matrix::translate(-bounds.x0, -bounds.y0);
    if turn {
        // A quarter turn clockwise puts the page left of the origin; move it back.
        ctm = ctm
            .then(&Matrix::rotate_quarter_turns(1))
            .then(&Matrix::translate(height, 0.0));
    }
    let (sx, sy) = (dx * scale, dy * scale);
    ctm = ctm.then(&Matrix::scale(sx, sy));
    let left = (paper.width as f32 - across * sx) / 2.0;
    let top = (paper.height as f32 - down * sy) / 2.0;
    ctm = ctm.then(&Matrix::translate(left, top));
    let area = ctm.apply_rect(&bounds);
    let clip = DeviceRect {
        x0: area.x0.max(0),
        y0: area.y0.max(0),
        x1: area.x1.min(i32::try_from(paper.width).unwrap_or(i32::MAX)),
        y1: area.y1.min(i32::try_from(paper.height).unwrap_or(i32::MAX)),
    };
    (ctm, clip)
}

/// `area` cut into bands from the top, each of at most [`BAND_BYTES`] of RGB.
pub fn bands(area: DeviceRect) -> impl Iterator<Item = DeviceRect> {
    let row = (area.width() as usize * 3).max(1);
    let rows = i32::try_from((BAND_BYTES / row).max(1)).unwrap_or(i32::MAX);
    let mut top = area.y0;
    std::iter::from_fn(move || {
        if area.is_empty() || top >= area.y1 {
            return None;
        }
        let band = DeviceRect {
            y0: top,
            y1: area.y1.min(top.saturating_add(rows)),
            ..area
        };
        top = band.y1;
        Some(band)
    })
}

/// The pages (from 0) that `ranges` (from 1, both ends kept, as the print dialog gives them) name in a document of
/// `count` pages, in the order given; pages past the end are left out.
pub fn pages_in(ranges: &[(u32, u32)], count: usize) -> Vec<usize> {
    ranges
        .iter()
        .flat_map(|&(from, to)| from.min(to)..=from.max(to))
        .filter_map(|page| usize::try_from(page).ok()?.checked_sub(1))
        .filter(|&page| page < count)
        .collect()
}

/// What a print job draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The pages, from 0, in the order they print.
    pub pages: Vec<usize>,
    /// How each page is sized on the paper.
    pub scaling: Scaling,
    /// Whether the pages' annotations print (form fields always do).
    pub annotations: bool,
}

/// The printer as a print job draws on it: a page at a time, each in bands.
pub trait Sheets {
    /// Starts a sheet.
    ///
    /// # Errors
    ///
    /// The printer refused.
    fn begin_page(&mut self) -> io::Result<()>;

    /// Draws `pixmap` (RGB, no alpha) with its top-left corner at `(x, y)` of the printable area.
    ///
    /// # Errors
    ///
    /// The printer refused.
    fn band(&mut self, x: i32, y: i32, pixmap: &Pixmap) -> io::Result<()>;

    /// Ends the sheet.
    ///
    /// # Errors
    ///
    /// The printer refused.
    fn end_page(&mut self) -> io::Result<()>;
}

/// A print job's progress and its cancel switch, shared between the job's thread and the UI.
#[derive(Debug, Default)]
pub struct Progress {
    printed: AtomicUsize,
    cookie: Cookie,
}

impl Progress {
    /// A job not started.
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// How many pages have gone to the printer.
    pub fn printed(&self) -> usize {
        self.printed.load(Ordering::Relaxed)
    }

    /// Stops the job at once: the page being drawn is dropped and nothing more is sent.
    pub fn cancel(&self) {
        self.cookie.abort();
    }

    /// [`Progress::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cookie.is_aborted()
    }
}

/// Why a print job stopped before its end.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The reader cancelled it.
    Cancelled,
    /// MuPDF couldn't read or draw a page.
    Mupdf(MuError),
    /// The printer refused a page.
    Printer(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("printing was cancelled"),
            Self::Mupdf(err) => write!(f, "{err}"),
            Self::Printer(err) => write!(f, "the printer refused a page: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled => None,
            Self::Mupdf(err) => Some(err),
            Self::Printer(err) => Some(err),
        }
    }
}

impl From<MuError> for Error {
    fn from(err: MuError) -> Self {
        if err.is_aborted() {
            Self::Cancelled
        } else {
            Self::Mupdf(err)
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Printer(err)
    }
}

/// Prints `job` from `document` (whose MuPDF document is `mupdf`) onto `paper` through `sheets`, a page at a time, on
/// the calling thread: run it on a thread of its own. `progress` counts the pages sent and cancels the job.
///
/// # Errors
///
/// Cancelled, a page that MuPDF can't draw, or a printer that refuses one. Pages already sent stay sent; the caller
/// aborts the printer's document.
pub fn print(
    document: &Document,
    mupdf: &SharedMupdf,
    job: &Job,
    paper: Paper,
    sheets: &mut impl Sheets,
    progress: &Progress,
) -> Result<(), Error> {
    let cookie = Some(&progress.cookie);
    for &page in &job.pages {
        progress.cookie.check()?;
        let list = display_list(document, mupdf, page, job.annotations, &progress.cookie)?;
        let (ctm, area) = placement(list.bounds()?, paper, job.scaling);
        sheets.begin_page()?;
        for band in bands(area) {
            let pixmap = list.render_area(&ctm, band, Background::White, cookie)?;
            sheets.band(band.x0, band.y0, &pixmap)?;
        }
        sheets.end_page()?;
        progress.printed.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// Page `page`'s display list: the views' own when annotations print and one is recorded, else recorded now (and not
/// kept: a list without annotations is no use to the views).
fn display_list(
    document: &Document,
    mupdf: &SharedMupdf,
    page: usize,
    annotations: bool,
    cookie: &Cookie,
) -> Result<Arc<DisplayList>, Error> {
    if annotations && let Some(list) = document.display_list(page) {
        return Ok(list);
    }
    let mupdf = mupdf.lock().unwrap_or_else(PoisonError::into_inner);
    let page = mupdf.load_page(page)?;
    let list = if annotations {
        page.display_list(Some(cookie))?
    } else {
        page.contents_display_list(Some(cookie))?
    };
    Ok(Arc::new(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::registry;
    use std::path::Path;

    const LETTER_300: Paper = Paper {
        width: 2400,
        height: 3150,
        dpi_x: 300.0,
        dpi_y: 300.0,
    };

    fn page(width: f32, height: f32) -> PageRect {
        PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        }
    }

    #[test]
    fn a_page_is_fit_shrunk_or_kept_at_its_size_and_centred() {
        // US Letter at 300 dpi is 2550 × 3300 pixels: larger than the printable area.
        let (_, fit) = placement(page(612.0, 792.0), LETTER_300, Scaling::Fit);
        assert!((2399..=2400).contains(&fit.width()), "{fit:?}");
        assert!(fit.y0 > 0 && fit.y0 + fit.y1 == 3150, "centred: {fit:?}");
        let (_, shrunk) = placement(page(612.0, 792.0), LETTER_300, Scaling::Shrink);
        assert_eq!(shrunk, fit, "a large page shrinks as it fits");
        let (_, actual) = placement(page(612.0, 792.0), LETTER_300, Scaling::Actual);
        assert_eq!(
            actual,
            DeviceRect {
                x0: 0,
                y0: 0,
                x1: 2400,
                y1: 3150
            },
            "cut at the edges"
        );
        // A small page grows only when fitting.
        let (_, small) = placement(page(144.0, 144.0), LETTER_300, Scaling::Shrink);
        assert_eq!(small.width(), 600);
        let (_, grown) = placement(page(144.0, 144.0), LETTER_300, Scaling::Fit);
        assert!((2399..=2400).contains(&grown.width()), "{grown:?}");
    }

    #[test]
    fn a_landscape_page_turns_onto_portrait_paper() {
        let (ctm, area) = placement(page(792.0, 612.0), LETTER_300, Scaling::Fit);
        assert!(area.height() > area.width(), "turned: {area:?}");
        // The page's top-left corner goes to the paper's top right.
        let corner = ctm.apply(excellent_mupdf::PagePoint { x: 0.0, y: 0.0 });
        assert!(corner.x > 1200.0 && corner.y < 1575.0, "{corner:?}");
    }

    #[test]
    fn bands_cover_the_area_once_each_within_the_size() {
        let area = DeviceRect {
            x0: 10,
            y0: 20,
            x1: 5010,
            y1: 6620,
        };
        let bands: Vec<_> = bands(area).collect();
        assert!(bands.len() > 1);
        assert_eq!(bands[0].y0, 20);
        assert_eq!(bands.last().map(|band| band.y1), Some(6620));
        for pair in bands.windows(2) {
            assert_eq!(pair[0].y1, pair[1].y0, "no gap or overlap");
        }
        for band in &bands {
            assert!(band.width() as usize * 3 * band.height() as usize <= BAND_BYTES);
        }
        assert_eq!(super::bands(DeviceRect::default()).count(), 0);
    }

    #[test]
    fn page_ranges_from_the_dialog_become_pages_from_zero() {
        assert_eq!(pages_in(&[(1, 3), (7, 7)], 10), vec![0, 1, 2, 6]);
        assert_eq!(
            pages_in(&[(9, 12)], 10),
            vec![8, 9],
            "past the end is left out"
        );
        assert_eq!(pages_in(&[(3, 2), (0, 1)], 10), vec![1, 2, 0]);
        assert_eq!(Scaling::parse(Some("fit")), Scaling::Fit);
        assert_eq!(Scaling::parse(Some("odd")), Scaling::Shrink);
    }

    /// Sheets that keep what they are given.
    #[derive(Default)]
    struct Kept {
        pages: usize,
        rows: Vec<u32>,
        open: bool,
    }

    impl Sheets for Kept {
        fn begin_page(&mut self) -> io::Result<()> {
            assert!(!self.open);
            self.open = true;
            Ok(())
        }

        fn band(&mut self, _x: i32, _y: i32, pixmap: &Pixmap) -> io::Result<()> {
            assert!(self.open && pixmap.components() == 3 && !pixmap.has_alpha());
            self.rows.push(pixmap.height());
            Ok(())
        }

        fn end_page(&mut self) -> io::Result<()> {
            self.open = false;
            self.pages += 1;
            Ok(())
        }
    }

    #[test]
    fn a_job_draws_each_page_in_bands_and_stops_when_cancelled() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let document = registry().open(&pdf).expect("the corpus file opens");
        let mupdf = document.mupdf().expect("it is awake");
        let job = Job {
            pages: vec![0, 1],
            scaling: Scaling::Fit,
            annotations: false,
        };
        let paper = Paper {
            width: 850,
            height: 1100,
            dpi_x: 100.0,
            dpi_y: 100.0,
        };
        let progress = Progress::new();
        let mut sheets = Kept::default();
        print(&document, &mupdf, &job, paper, &mut sheets, &progress).expect("it prints");
        assert_eq!((sheets.pages, progress.printed()), (2, 2));
        assert!(sheets.rows.iter().sum::<u32>() >= 2 * 1000);

        let progress = Progress::new();
        progress.cancel();
        let mut sheets = Kept::default();
        let stopped = print(&document, &mupdf, &job, paper, &mut sheets, &progress);
        assert!(matches!(stopped, Err(Error::Cancelled)));
        assert_eq!(sheets.pages, 0);
    }
}
