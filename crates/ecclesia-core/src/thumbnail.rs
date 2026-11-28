//! Thumbnails (plan R.5), the rendering: a page drawn small from its display list, on white, written as a PNG file
//! for the app to show. Runs on the render workers at [`Priority::Thumbnail`], below the tiles on screen.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ecclesia_mupdf::{Background, Matrix};

use crate::document::{Document, SharedMupdf};
use crate::render::{Job, Priority, RenderPool};

/// Renders page `page` of `document` (whose MuPDF document is `mupdf`) `width` pixels wide and writes it as a PNG to
/// `path`, atomically. The page's display list is reused when a view already recorded one, and kept for the views
/// when it wasn't. The job fails with MuPDF's error; the file write's own error is the inner result.
///
/// shortcut: the page is drawn as the file has it, without the view's rotation (R.3); rotate the thumbnail with it
/// if readers miss that.
pub fn render(
    pool: &RenderPool,
    document: &Arc<Document>,
    mupdf: SharedMupdf,
    page: usize,
    width: u32,
    path: PathBuf,
) -> Job<io::Result<PathBuf>> {
    render_cover(pool, document, mupdf, page, width, u32::MAX, false, path)
}

/// Renders page `page` as a Home cover thumbnail fitting `max_width` × `max_height`:
/// - `crop_top == true` (SumatraPDF Enhanced half-page mode): scales the page to `max_width` and
///   crops vertically from the top edge (`y0`) at `max_height`, showing the top half of the page.
/// - `crop_top == false` (whole-page mode): fits the entire uncropped page inside `max_width` × `max_height`.
#[allow(clippy::too_many_arguments)]
pub fn render_cover(
    pool: &RenderPool,
    document: &Arc<Document>,
    mupdf: SharedMupdf,
    page: usize,
    max_width: u32,
    max_height: u32,
    crop_top: bool,
    path: PathBuf,
) -> Job<io::Result<PathBuf>> {
    let document = Arc::clone(document);
    pool.submit(Priority::Thumbnail, move |cookie| {
        let list = match document.display_list(page) {
            Some(list) => list,
            None => {
                let revision = document.revision(page);
                let list = {
                    let mupdf = mupdf.lock().expect("document lock poisoned");
                    let hidden = document.hidden_annotations();
                    Arc::new(
                        mupdf
                            .load_page(page)?
                            .display_list_without(&hidden, Some(cookie))?,
                    )
                };
                document.keep_display_list(page, revision, Arc::clone(&list));
                list
            }
        };
        let bounds = list.bounds()?;
        let page_w = (bounds.x1 - bounds.x0).max(1.0);
        let page_h = (bounds.y1 - bounds.y0).max(1.0);
        let scale = cover_scale(page_w, page_h, max_width, max_height, crop_top);
        let ctm = Matrix::scale(scale, scale);
        let mut area = ctm.apply_rect(&bounds);
        let max_w = i32::try_from(max_width).unwrap_or(i32::MAX);
        let max_h = i32::try_from(max_height).unwrap_or(i32::MAX);
        area.x1 = area.x1.min(area.x0.saturating_add(max_w));
        if crop_top || max_height != u32::MAX {
            area.y1 = area.y1.min(area.y0.saturating_add(max_h));
        }
        let png = list
            .render_area(&ctm, area, Background::White, Some(cookie))?
            .to_png()?;
        Ok(write_atomically(&path, &png).map(|()| path))
    })
}

/// Computes the scale factor matching SumatraPDF Enhanced's `ThumbnailZoom`:
/// width-fitted for top-cropped half-page previews, or uniform aspect-fit for whole-page previews.
fn cover_scale(page_w: f32, page_h: f32, max_width: u32, max_height: u32, crop_top: bool) -> f32 {
    let zoom_x = max_width as f32 / page_w.max(1.0);
    if crop_top || max_height == u32::MAX {
        zoom_x
    } else {
        zoom_x.min(max_height as f32 / page_h.max(1.0))
    }
}

/// Writes `bytes` to a temporary file beside `path`, then renames it over `path`, so a reader never sees half a file.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let partial = path.with_extension("part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{pool, registry};

    #[test]
    fn a_page_becomes_a_png_file_of_the_asked_width() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let document = registry().open(&pdf).expect("the corpus file opens");
        let mupdf = document.mupdf().expect("it is awake");
        let out = std::env::temp_dir().join(format!("ecclesia-thumb-{}.png", std::process::id()));
        let job = render(pool(), &document, mupdf, 0, 120, out.clone());
        let path = job.wait().expect("it renders").expect("it is written");
        let png = fs::read(&path).expect("the file is there");
        let _ = fs::remove_file(&path);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR's width, big-endian, right after the signature and the chunk header.
        let width = u32::from_be_bytes(png[16..20].try_into().expect("four bytes"));
        assert_eq!(width, 120);
        assert!(
            document.display_list(0).is_some(),
            "the list is kept for the views"
        );
    }

    #[test]
    fn render_cover_crops_top_half_or_fits_whole_page_matching_sumatra_sizes() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let document = registry().open(&pdf).expect("the corpus file opens");
        let mupdf = document.mupdf().expect("it is awake");
        let pid = std::process::id();

        let cropped_out = std::env::temp_dir().join(format!("ecclesia-thumb-crop-{pid}.png"));
        let cropped_path = render_cover(
            pool(),
            &document,
            Arc::clone(&mupdf),
            0,
            530,
            375,
            true,
            cropped_out,
        )
        .wait()
        .expect("cropped renders")
        .expect("cropped written");
        let cropped_png = fs::read(&cropped_path).expect("cropped png");
        let _ = fs::remove_file(&cropped_path);
        let cw = u32::from_be_bytes(cropped_png[16..20].try_into().expect("width"));
        let ch = u32::from_be_bytes(cropped_png[20..24].try_into().expect("height"));
        assert_eq!(cw, 530);
        assert_eq!(ch, 375);

        let full_out = std::env::temp_dir().join(format!("ecclesia-thumb-full-{pid}.png"));
        let full_path = render_cover(pool(), &document, mupdf, 0, 530, 750, false, full_out)
            .wait()
            .expect("full renders")
            .expect("full written");
        let full_png = fs::read(&full_path).expect("full png");
        let _ = fs::remove_file(&full_path);
        let fw = u32::from_be_bytes(full_png[16..20].try_into().expect("width"));
        let fh = u32::from_be_bytes(full_png[20..24].try_into().expect("height"));
        assert!(fw <= 530 && fh <= 750);
        assert!(
            fh > ch,
            "whole-page cover should be taller than top-half crop"
        );
    }
}
