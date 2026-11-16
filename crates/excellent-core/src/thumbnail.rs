//! Thumbnails (plan R.5), the rendering: a page drawn small from its display list, on white, written as a PNG file
//! for the app to show. Runs on the render workers at [`Priority::Thumbnail`], below the tiles on screen.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use excellent_mupdf::{Background, Matrix};

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
        let scale = width as f32 / (bounds.x1 - bounds.x0).max(1.0);
        let ctm = Matrix::scale(scale, scale);
        let area = ctm.apply_rect(&bounds);
        let png = list
            .render_area(&ctm, area, Background::White, Some(cookie))?
            .to_png()?;
        Ok(write_atomically(&path, &png).map(|()| path))
    })
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
        let out = std::env::temp_dir().join(format!("excellent-thumb-{}.png", std::process::id()));
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
}
