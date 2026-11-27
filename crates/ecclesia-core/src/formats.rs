//! The formats the app opens besides PDF (plan R.28), all through MuPDF: which files they are, and the pages a
//! reflowable one is laid out on.
//!
//! Fixed-layout formats (PDF, XPS, CBZ, SVG, images) have pages of their own. Reflowable ones (EPUB, FB2, MOBI,
//! HTML, Markdown) lay their text out on pages the size of the reader's window, at the reader's font size, and again
//! when either changes.

use ecclesia_mupdf::Reflow;

/// The extensions of the files the app opens, as MuPDF reads them (plan D3). DjVu, CHM and CBR are R.29's.
pub const EXTENSIONS: &[&str] = &[
    "pdf", "xps", "oxps", "epub", "fb2", "mobi", "prc", "cbz", "svg", "html", "htm", "xhtml", "md",
    // Pictures, each a one-page document.
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "tif", "tiff", "jp2", "j2k", "jpx", "jxr",
    "wdp", "hdp", "pnm", "pbm", "pgm", "ppm", "pam", "psd",
];

/// Whether a file named with `extension` (lower case, without the dot) and starting with `head` (its first bytes,
/// up to a kilobyte) is one the app opens. Judged by the file's signature, so a renamed file is still found and a
/// file merely named `.pdf` is not; only Markdown, which has no signature, goes by its name (and must be text).
pub fn is_document(head: &[u8], extension: Option<&str>) -> bool {
    has_signature(head) || (extension == Some("md") && is_text(head))
}

/// Whether `head` (a file's first bytes, up to a kilobyte) starts a PDF. PDF allows junk before its header; MuPDF
/// looks through the first kilobyte.
pub fn is_pdf(head: &[u8]) -> bool {
    head.windows(5).take(1024).any(|window| window == b"%PDF-")
}

/// Whether `head` starts the way one of the [`EXTENSIONS`]' formats does.
fn has_signature(head: &[u8]) -> bool {
    let starts = |magic: &[u8]| head.starts_with(magic);
    let pdf = is_pdf(head);
    // ZIP: EPUB, CBZ, XPS and OpenXPS.
    let zip = starts(b"PK\x03\x04");
    // Mobipocket and PalmDOC name their type at byte 60.
    let palm = matches!(head.get(60..68), Some(b"BOOKMOBI" | b"TEXtREAd"));
    let pictures = [
        b"\x89PNG\r\n\x1a\n".as_slice(),
        b"\xff\xd8\xff",
        b"GIF87a",
        b"GIF89a",
        b"BM",
        b"II*\0",
        b"MM\0*",
        // JPEG 2000 (a JP2 box, or a bare codestream), JPEG XR, Photoshop.
        b"\0\0\0\x0cjP  ",
        b"\xff\x4f\xff\x51",
        b"II\xbc",
        b"8BPS",
    ];
    let picture = pictures.iter().any(|magic| starts(magic));
    // Netpbm: "P1" to "P7", then white space.
    let netpbm = matches!(head, [b'P', b'1'..=b'7', space, ..] if space.is_ascii_whitespace());
    pdf || zip || palm || picture || netpbm || is_markup(head)
}

/// XML or HTML: FB2, SVG, XHTML and HTML.
fn is_markup(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let lower = text
        .get(..text.len().min(16))
        .unwrap_or("")
        .to_ascii_lowercase();
    ["<?xml", "<svg", "<fictionbook", "<!doctype html", "<html"]
        .iter()
        .any(|start| lower.starts_with(start))
}

/// UTF-8 text without NULs; a character cut off at the end of `head` is fine.
fn is_text(head: &[u8]) -> bool {
    let valid = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none(),
    };
    valid && !head.contains(&0)
}

/// The reflow text size the app starts with, in points.
pub const DEFAULT_FONT_SIZE: f32 = 12.0;
/// The smallest and largest reflow text sizes the reader can choose, in points.
pub const FONT_SIZES: std::ops::RangeInclusive<f32> = 8.0..=32.0;

/// The widest a reflowed page gets, in ems: about 70 characters to a line, past which lines get hard to follow.
const WIDEST: f32 = 36.0;
/// The narrowest and shortest a reflowed page gets, in ems, however small the window.
const NARROWEST: f32 = 14.0;
const SHORTEST: f32 = 12.0;

/// The pages a reflowable document is laid out on in a window with `width` × `height` points of room for a page
/// (its size at zoom 1, less the gaps the layout puts around a page): as tall as that room, so a page is a screen,
/// and as wide as it up to a comfortable line length, with the text at `font_size` points (clamped to
/// [`FONT_SIZES`]).
pub fn reflow_pages(width: f32, height: f32, font_size: f32) -> Reflow {
    let em = font_size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
    Reflow {
        width: width.max(0.0).clamp(NARROWEST * em, WIDEST * em),
        height: height.max(0.0).max(SHORTEST * em),
        font_size: em,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_known_by_their_contents() {
        for head in [
            b"%PDF-1.7\n".as_slice(),
            b"\r\n%PDF-2.0",
            b"PK\x03\x04mimetypeapplication/epub+zip",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR",
            b"\xff\xd8\xff\xe0\0\x10JFIF",
            b"GIF89a",
            b"II*\0\x08\0",
            b"\0\0\0\x0cjP  \r\n\x87\n",
            b"P6\n120 160\n255\n",
            b"\xef\xbb\xbf<?xml version=\"1.0\"?>",
            b"<FictionBook xmlns=",
            b"<svg xmlns=",
            b"  <!DOCTYPE html>\n<html>",
            b"<HTML><BODY>",
        ] {
            assert!(
                is_document(head, None),
                "{:?}",
                String::from_utf8_lossy(head)
            );
        }
        let mut mobi = vec![0; 60];
        mobi.extend_from_slice(b"BOOKMOBI");
        assert!(is_document(&mobi, Some("mobi")));

        for head in [
            b"MZ\x90\0".as_slice(),
            b"hello",
            b"AT&TFORM\0\0",
            b"Rar!\x1a\x07",
            b"P9 ",
        ] {
            assert!(
                !is_document(head, Some("pdf")),
                "{:?}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn markdown_goes_by_its_name_and_must_be_text() {
        assert!(is_document(b"# Notes\n\nSome text.\n", Some("md")));
        assert!(!is_document(b"# Notes\n\nSome text.\n", Some("txt")));
        assert!(!is_document(b"# Notes\0\x01\x02", Some("md")));
        // A two-byte character cut off by the end of the head.
        assert!(is_document(
            "caf\u{e9}".as_bytes().split_last().expect("bytes").1,
            Some("md")
        ));
    }

    #[test]
    fn every_extension_is_lower_case_and_listed_once() {
        for (i, ext) in EXTENSIONS.iter().enumerate() {
            assert_eq!(*ext, ext.to_ascii_lowercase());
            assert!(!EXTENSIONS[..i].contains(ext), "{ext} twice");
        }
        assert!(!EXTENSIONS.contains(&"djvu"), "DjVu is R.29's");
    }

    #[test]
    fn reflowed_pages_fill_the_window_up_to_a_readable_line() {
        // A small window: the page is the room it has.
        let page = reflow_pages(384.0, 684.0, 12.0);
        assert_eq!(
            (page.width, page.height, page.font_size),
            (384.0, 684.0, 12.0)
        );
        // A wide window: lines stop at 36 ems.
        assert_eq!(reflow_pages(2000.0, 700.0, 12.0).width, 36.0 * 12.0);
        // A tiny window still gets a page that holds some text.
        let tiny = reflow_pages(10.0, 10.0, 12.0);
        assert_eq!((tiny.width, tiny.height), (14.0 * 12.0, 12.0 * 12.0));
        // Out-of-range sizes are clamped.
        assert_eq!(reflow_pages(500.0, 700.0, 2.0).font_size, 8.0);
        assert_eq!(reflow_pages(500.0, 700.0, 99.0).font_size, 32.0);
    }
}
