//! Document outlines (plan §11.14), read on the render workers so a long one never stalls the UI thread.

pub use excellent_mupdf::OutlineItem;

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
}
