//! Document outlines (plan §11.14), read on the render workers so a long one never stalls the UI thread.

use excellent_mupdf::TextLayout;
pub use excellent_mupdf::{OutlineItem, OutlineStyle};

use crate::document::SharedMupdf;
use crate::render::{Job, Priority, RenderPool};

/// Reads the outline of the document `mupdf`: empty when it has none. Runs above thumbnails and below the pages on
/// screen.
pub fn load(pool: &RenderPool, mupdf: SharedMupdf) -> Job<Vec<OutlineItem>> {
    pool.submit(Priority::Nearby, move |_cookie| {
        let mupdf = mupdf.lock().expect("document lock poisoned");
        mupdf.outline()
    })
}

/// A heading to look for (B.9): its title, and the page to look from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The title it reads as.
    pub title: String,
    /// The first page to look on, counting from zero; the pages before it are looked on last.
    pub from: usize,
}

/// Looks for each of `wanted`'s headings in the document `mupdf`, reading each page's text once: where `top` finds
/// one, as a page and the heading's top in page points, or `None`. Runs at background priority and can be cancelled.
pub fn find_headings(
    pool: &RenderPool,
    mupdf: SharedMupdf,
    wanted: Vec<Wanted>,
    top: fn(&TextLayout, &str) -> Option<f32>,
) -> Job<Vec<Option<(usize, f32)>>> {
    pool.submit(Priority::Background, move |cookie| {
        let mupdf = mupdf.lock().expect("document lock poisoned");
        let count = mupdf.page_count()?;
        // The first match at or after each one's page, and the first before it, kept in case there is none after.
        let mut after: Vec<Option<(usize, f32)>> = vec![None; wanted.len()];
        let mut before: Vec<Option<(usize, f32)>> = vec![None; wanted.len()];
        for index in 0..count {
            if after.iter().all(Option::is_some) {
                break;
            }
            cookie.check()?;
            let layout = mupdf.load_page(index)?.text(Some(cookie))?.layout();
            for (n, heading) in wanted.iter().enumerate() {
                let slot = if index >= heading.from {
                    &mut after[n]
                } else {
                    &mut before[n]
                };
                if slot.is_none()
                    && let Some(y) = top(&layout, &heading.title)
                {
                    *slot = Some((index, y));
                }
            }
        }
        Ok(after
            .into_iter()
            .zip(before)
            .map(|(after, before)| after.or(before))
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::shared::{pool, registry};

    #[test]
    fn an_outline_is_read_on_the_workers() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let document = registry().open(&pdf).expect("the corpus file opens");
        let mupdf = document.mupdf().expect("it is awake");
        let outline = load(pool(), mupdf).wait().expect("it is read");
        let titles: Vec<&str> = outline.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["Page one", "Page two"]);
    }

    #[test]
    fn headings_are_found_on_the_workers() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let document = registry().open(&pdf).expect("the corpus file opens");
        let mupdf = document.mupdf().expect("it is awake");
        // Any non-empty line is "a heading" here: the first page with text is found, from either end.
        let any = |layout: &TextLayout, _: &str| layout.lines.first().map(|line| line.bbox.y0);
        let wanted = vec![
            Wanted {
                title: "x".into(),
                from: 0,
            },
            Wanted {
                title: "x".into(),
                from: 99,
            },
        ];
        let found = find_headings(pool(), mupdf, wanted, any)
            .wait()
            .expect("it runs");
        assert!(found[0].is_some_and(|(page, _)| page == 0), "{found:?}");
        assert!(found[1].is_some(), "{found:?}");
    }
}
