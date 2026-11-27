//! A page's links (R.7), copied into Rust.

use core::ffi::CStr;
use core::ptr;

use ecclesia_mupdf_sys::{
    ex_drop_links, ex_link_item, ex_link_item_of, ex_load_links, fz_context, fz_link,
};

use crate::context::{check, with_context};
use crate::error::Result;
use crate::geometry::{PagePoint, PageRect};
use crate::page::Page;

/// A hostile page can't make the copy unbounded; real pages have a few hundred links at most.
const MAX_LINKS: usize = 10_000;

/// One link of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// The area that follows the link, in page points (the coordinates of [`Page::bounds`]).
    pub rect: PageRect,
    /// The link as MuPDF gives it: a web address, `file:` for a launch or another file, or `#page=…` for a place in
    /// this document.
    pub uri: String,
    /// Where it goes in this document, when it does.
    pub target: Option<Target>,
}

/// A place in this document that a link goes to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// The page, counting from zero.
    pub page: usize,
    /// The point on it, when the link names one; `x` is 0 when the link names only a height.
    pub point: Option<PagePoint>,
}

/// Drops the whole chain when it goes out of scope, even if copying it failed half way.
struct Chain {
    ctx: *mut fz_context,
    first: *mut fz_link,
}

impl Drop for Chain {
    fn drop(&mut self) {
        // SAFETY: `first` is the chain we loaded (or NULL, which the shim accepts), dropped once, with the context that
        // is current for this whole call.
        unsafe { ex_drop_links(self.ctx, self.first) };
    }
}

fn copy(raw: &ex_link_item) -> Link {
    // SAFETY: the shim gives a NUL-terminated string, never NULL, that lives as long as the chain.
    let uri = unsafe { CStr::from_ptr(raw.uri) }
        .to_string_lossy()
        .into_owned();
    // A link may name only a height (`/FitH`): its point is then at the page's left edge.
    let x = if raw.x.is_finite() { raw.x } else { 0.0 };
    let point = raw.y.is_finite().then_some(PagePoint { x, y: raw.y });
    let target = usize::try_from(raw.page)
        .ok()
        .map(|page| Target { page, point });
    Link {
        rect: raw.rect.into(),
        uri,
        target,
    }
}

/// Loads and copies the links of `page`.
pub(crate) fn load(page: &Page<'_>) -> Result<Vec<Link>> {
    let raw_page = page.raw();
    with_context(|ctx| {
        let mut first = ptr::null_mut();
        // SAFETY: the page is live and used by this thread only (the caller borrows it).
        check(|err| unsafe { ex_load_links(ctx, raw_page.as_ptr(), &mut first, err) })?;
        let chain = Chain { ctx, first };
        let mut links = Vec::new();
        let mut node = chain.first;
        while !node.is_null() && links.len() < MAX_LINKS {
            let mut raw = ex_link_item::default();
            // SAFETY: `node` is part of the live chain loaded from this page.
            check(|err| unsafe { ex_link_item_of(ctx, raw_page.as_ptr(), node, &mut raw, err) })?;
            links.push(copy(&raw));
            node = raw.next;
        }
        Ok(links)
    })
}
