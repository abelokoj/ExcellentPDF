//! A page's pictures, listed once and each sampled at most once, for the tiles that recolour around them.

use std::sync::OnceLock;

use excellent_mupdf::{Cookie, DisplayList, PageImage, PageRect, Result};

use super::region::{ImageOnPage, kept_region};
use super::stats::SampleStats;

/// The pictures of one page's display list. Empty until a tile first needs them; shared by that page's tiles, which
/// render on several workers at once.
#[derive(Debug, Default)]
pub struct PageImages {
    loaded: OnceLock<Loaded>,
}

#[derive(Debug)]
struct Loaded {
    page: PageRect,
    images: Vec<ImageOnPage>,
    sources: Vec<(PageImage, OnceLock<Option<SampleStats>>)>,
}

impl PageImages {
    /// Nothing listed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The region of the page drawn by `list` that keeps its colours at `scale` pixels a point (see
    /// [`kept_region`]). The first call lists the pictures; `cookie` cancels that.
    ///
    /// # Errors
    ///
    /// `cookie` was aborted while the pictures were listed. Any other failure lists no picture, so the whole page
    /// recolours.
    pub fn kept_region(
        &self,
        list: &DisplayList,
        scale: f32,
        cookie: &Cookie,
    ) -> Result<Option<PageRect>> {
        let loaded = match self.loaded.get() {
            Some(loaded) => loaded,
            None => {
                let loaded = load(list, cookie)?;
                // Another tile may have listed them meanwhile; either list is the same.
                let _ = self.loaded.set(loaded);
                self.loaded.get().expect("just set")
            }
        };
        Ok(kept_region(&loaded.images, loaded.page, scale, |i| {
            let (image, stats) = &loaded.sources[i];
            *stats.get_or_init(|| sample(image))
        }))
    }
}

fn load(list: &DisplayList, cookie: &Cookie) -> Result<Loaded> {
    let listed = list
        .bounds()
        .and_then(|page| Ok((page, list.images(Some(cookie))?)));
    let (page, images) = match listed {
        Ok(listed) => listed,
        Err(err) if err.is_aborted() => return Err(err),
        // shortcut: a page whose pictures can't be listed recolours whole, unlogged (core has no logger yet).
        Err(_) => (PageRect::default(), Vec::new()),
    };
    Ok(Loaded {
        page,
        images: images
            .iter()
            .map(|image| ImageOnPage {
                bounds: image.bounds(),
                width: image.width(),
                height: image.height(),
            })
            .collect(),
        sources: images
            .into_iter()
            .map(|image| (image, OnceLock::new()))
            .collect(),
    })
}

/// The picture's statistics; `None` when it can't be decoded, as Enhanced treats it.
fn sample(image: &PageImage) -> Option<SampleStats> {
    let pixmap = image.sample().ok()?;
    SampleStats::measure(
        pixmap.samples(),
        pixmap.width() as usize,
        pixmap.height() as usize,
        pixmap.stride(),
        pixmap.components(),
    )
}
