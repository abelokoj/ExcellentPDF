//! A print spool: the printed sheets as a PDF of images, one image per band, which the system's print queue takes on
//! macOS and Linux. The pages are already pixels at the printer's resolution (`excellent_core::print`), so the spool
//! only places them; it is written as the bands come, so a job never holds more than a band in memory.

use std::fmt::Write as _;
use std::io::{self, Write};

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// The paper a spool's sheets are, in points (1/72 inch), and the printable area on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Sheet {
    /// The paper's width.
    pub(super) paper_width: f32,
    /// The paper's height.
    pub(super) paper_height: f32,
    /// The printable area's left edge, from the paper's left.
    pub(super) left: f32,
    /// The printable area's top edge, from the paper's top.
    pub(super) top: f32,
    /// The printable area's width.
    pub(super) width: f32,
    /// The printable area's height.
    pub(super) height: f32,
    /// The pixels per inch the bands come at, both ways.
    pub(super) dpi: f32,
}

impl Sheet {
    /// The printable area in pixels at the sheet's resolution.
    pub(super) fn area(&self) -> super::Area {
        let pixels = |points: f32| ((points * self.dpi / 72.0).floor() as u32).max(1);
        super::Area {
            width: pixels(self.width),
            height: pixels(self.height),
            dpi_x: self.dpi,
            dpi_y: self.dpi,
        }
    }

    /// Points per pixel.
    fn point(&self) -> f32 {
        72.0 / self.dpi
    }
}

/// The catalog's and the page tree's object numbers; the page tree is written last, when its pages are known.
const CATALOG: usize = 1;
const PAGES: usize = 2;

/// A PDF being written: header first, objects as they come, the page tree and the cross-reference table at the end.
#[derive(Debug)]
pub(super) struct Spool<W: Write> {
    out: W,
    /// Bytes written so far: where the next object starts.
    at: u64,
    /// Each object's start, by number (0 is the free head and stays 0).
    offsets: Vec<u64>,
    /// The pages' object numbers, in order.
    pages: Vec<usize>,
    /// The sheet being drawn: its images and the content that places them.
    open: Option<Open>,
    sheet: Sheet,
}

#[derive(Debug, Default)]
struct Open {
    images: Vec<usize>,
    content: String,
}

impl<W: Write> Spool<W> {
    /// Starts a spool of `sheet`-sized pages on `out`.
    ///
    /// # Errors
    ///
    /// `out` refused the header.
    pub(super) fn new(out: W, sheet: Sheet) -> io::Result<Self> {
        let mut spool = Self {
            out,
            at: 0,
            // Free head, catalog and page tree.
            offsets: vec![0; PAGES + 1],
            pages: Vec::new(),
            open: None,
            sheet,
        };
        // The second line's high bytes mark the file as binary, as the PDF reference advises.
        spool.put(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n")?;
        spool.offsets[CATALOG] = spool.at;
        spool.put(
            format!("{CATALOG} 0 obj\n<< /Type /Catalog /Pages {PAGES} 0 R >>\nendobj\n")
                .as_bytes(),
        )?;
        Ok(spool)
    }

    /// Starts a sheet. One already open is ended first.
    ///
    /// # Errors
    ///
    /// Ending an open sheet failed.
    pub(super) fn begin_page(&mut self) -> io::Result<()> {
        if self.open.is_some() {
            self.end_page()?;
        }
        self.open = Some(Open::default());
        Ok(())
    }

    /// Places `width` × `height` pixels of RGB (rows `stride` bytes apart) with the top-left corner at `(x, y)` of the
    /// printable area. Without an open sheet, one is begun.
    ///
    /// # Errors
    ///
    /// `out` refused the image.
    ///
    /// # Panics
    ///
    /// `rgb` is shorter than `height` rows of `stride` bytes, or a row shorter than `width` pixels.
    pub(super) fn band(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        stride: usize,
        rgb: &[u8],
    ) -> io::Result<()> {
        let (w, h) = (width as usize, height as usize);
        assert!(
            stride >= w * 3 && rgb.len() >= stride * h.saturating_sub(1) + w * 3,
            "band of {w}×{h} pixels doesn't fit {} bytes at stride {stride}",
            rgb.len()
        );
        if w == 0 || h == 0 {
            return Ok(());
        }
        let mut packed = ZlibEncoder::new(Vec::new(), Compression::fast());
        for row in rgb.chunks(stride).take(h) {
            packed.write_all(&row[..w * 3])?;
        }
        let packed = packed.finish()?;
        let image = self.object(
            &format!(
                "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB \
                 /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>",
                packed.len()
            ),
            Some(&packed),
        )?;
        let point = self.sheet.point();
        let (wide, high) = (width as f32 * point, height as f32 * point);
        let left = self.sheet.left + x as f32 * point;
        // PDF measures up from the paper's foot.
        let bottom = self.sheet.paper_height - (self.sheet.top + y as f32 * point) - high;
        let open = self.open.get_or_insert_with(Open::default);
        let name = open.images.len();
        open.images.push(image);
        // Writing to a String can't fail.
        let _ = writeln!(
            open.content,
            "q {wide:.4} 0 0 {high:.4} {left:.4} {bottom:.4} cm /I{name} Do Q"
        );
        Ok(())
    }

    /// Ends the sheet: its content and its page are written. Without an open sheet, nothing happens.
    ///
    /// # Errors
    ///
    /// `out` refused the page.
    pub(super) fn end_page(&mut self) -> io::Result<()> {
        let Some(open) = self.open.take() else {
            return Ok(());
        };
        let content = self.object(
            &format!("<< /Length {} >>", open.content.len()),
            Some(open.content.as_bytes()),
        )?;
        let images: String = open
            .images
            .iter()
            .enumerate()
            .map(|(name, number)| format!("/I{name} {number} 0 R "))
            .collect();
        let Sheet {
            paper_width,
            paper_height,
            ..
        } = self.sheet;
        let page = self.object(
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {paper_width:.4} {paper_height:.4}] \
                 /Resources << /XObject << {images}>> >> /Contents {content} 0 R >>"
            ),
            None,
        )?;
        self.pages.push(page);
        Ok(())
    }

    /// The number of sheets ended so far.
    #[cfg(test)]
    pub(super) fn pages(&self) -> usize {
        self.pages.len()
    }

    /// Ends the file (a sheet still open is ended) and hands back `out`.
    ///
    /// # Errors
    ///
    /// `out` refused the end of the file.
    pub(super) fn finish(mut self) -> io::Result<W> {
        self.end_page()?;
        let kids: String = self
            .pages
            .iter()
            .map(|page| format!("{page} 0 R "))
            .collect();
        self.offsets[PAGES] = self.at;
        let tree = format!(
            "{PAGES} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {} >>\nendobj\n",
            self.pages.len()
        );
        self.put(tree.as_bytes())?;
        let table = self.at;
        let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len());
        for offset in &self.offsets[1..] {
            // Each entry is exactly 20 bytes, its line break included.
            let _ = writeln!(xref, "{offset:010} 00000 n ");
        }
        let _ = writeln!(
            xref,
            "trailer\n<< /Size {} /Root {CATALOG} 0 R >>\nstartxref\n{table}\n%%EOF",
            self.offsets.len()
        );
        self.put(xref.as_bytes())?;
        self.out.flush()?;
        Ok(self.out)
    }

    /// Writes the next object, its dictionary and its stream if any; its number.
    fn object(&mut self, dictionary: &str, stream: Option<&[u8]>) -> io::Result<usize> {
        let number = self.offsets.len();
        self.offsets.push(self.at);
        self.put(format!("{number} 0 obj\n{dictionary}\n").as_bytes())?;
        if let Some(stream) = stream {
            self.put(b"stream\n")?;
            self.put(stream)?;
            self.put(b"\nendstream\n")?;
        }
        self.put(b"endobj\n")?;
        Ok(number)
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.at += bytes.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::ZlibDecoder;

    use super::*;

    const LETTER: Sheet = Sheet {
        paper_width: 612.0,
        paper_height: 792.0,
        left: 18.0,
        top: 18.0,
        width: 576.0,
        height: 756.0,
        dpi: 144.0,
    };

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// The cross-reference table's offsets, from the bytes after `startxref`.
    fn offsets(pdf: &[u8]) -> Vec<usize> {
        let all = text(pdf);
        let start: usize = all
            .rsplit("startxref\n")
            .next()
            .and_then(|tail| tail.lines().next())
            .and_then(|line| line.parse().ok())
            .expect("startxref names the table");
        // The header's binary bytes aren't UTF-8: cut by bytes, not by the lossy text.
        let table = text(&pdf[start..]);
        assert!(
            table.starts_with("xref\n0 "),
            "startxref points at the table"
        );
        table
            .lines()
            .skip(3)
            .take_while(|line| line.ends_with(" n "))
            .map(|line| line[..10].parse().expect("ten-digit offset"))
            .collect()
    }

    #[test]
    fn every_object_starts_where_the_table_says() {
        let mut spool = Spool::new(Vec::new(), LETTER).expect("in memory");
        let white = vec![255u8; 4 * 3 * 2];
        for _ in 0..2 {
            spool.begin_page().expect("in memory");
            spool.band(0, 0, 4, 2, 12, &white).expect("in memory");
            spool.band(0, 2, 4, 2, 12, &white).expect("in memory");
            spool.end_page().expect("in memory");
        }
        assert_eq!(spool.pages(), 2);
        let pdf = spool.finish().expect("in memory");
        let found = offsets(&pdf);
        // Catalog, page tree, then per page two images, a content stream and the page.
        assert_eq!(found.len(), 2 + 2 * 4);
        for (number, offset) in (1..).zip(found) {
            assert!(
                text(&pdf[offset..]).starts_with(&format!("{number} 0 obj\n")),
                "object {number} at {offset}"
            );
        }
        assert!(text(&pdf).contains("/Count 2"));
        assert!(pdf.starts_with(b"%PDF-1.4\n"));
    }

    #[test]
    fn a_band_keeps_its_pixels_and_lands_below_the_margin() {
        let mut spool = Spool::new(Vec::new(), LETTER).expect("in memory");
        // Two rows of three pixels, rows padded to 12 bytes.
        let rgb: Vec<u8> = (0..24).collect();
        spool.band(2, 4, 3, 2, 12, &rgb).expect("in memory");
        let pdf = spool.finish().expect("in memory");
        let all = text(&pdf);
        // 144 dpi: a pixel is half a point. Left 18 + 1; top 18 + 2, one point high: bottom 792 - 20 - 1.
        assert!(
            all.contains("q 1.5000 0 0 1.0000 19.0000 771.0000 cm /I0 Do Q"),
            "{all}"
        );
        let start = pdf
            .windows(7)
            .position(|w| w == b"stream\n")
            .expect("an image stream")
            + 7;
        let mut pixels = Vec::new();
        ZlibDecoder::new(&pdf[start..])
            .read_to_end(&mut pixels)
            .expect("the stream inflates");
        let expected: Vec<u8> = (0..9).chain(12..21).collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn the_area_is_the_printable_part_in_pixels() {
        let area = LETTER.area();
        assert_eq!((area.width, area.height), (1152, 1512));
    }
}
