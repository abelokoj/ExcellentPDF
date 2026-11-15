//! Tables of contents, copied into Rust.

use core::ffi::{CStr, c_int};
use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    ex_drop_outline, ex_load_outline, ex_outline_item, ex_outline_item_of, fz_context, fz_document,
    fz_outline,
};

use crate::context::{check, with_context};
use crate::error::Result;

/// Deeper entries are left out, so a hostile file can't exhaust the stack. Real outlines are a few levels deep.
const MAX_DEPTH: usize = 64;

/// How an outline entry's title is drawn (B.7): the PDF's `/F` and `/C`, as Acrobat shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OutlineStyle {
    /// Bold (`/F` bit 2).
    pub bold: bool,
    /// Italic (`/F` bit 1).
    pub italic: bool,
    /// Its own colour, `[r, g, b]`; `None` draws it in the theme's text colour. Black counts as none, as in SumatraPDF
    /// Enhanced: a black entry would vanish on a dark theme.
    pub color: Option<[u8; 3]>,
}

/// `/F`'s bits, in the PDF spec's order (MuPDF's `FZ_OUTLINE_FLAG_*` names have them the other way round).
const ITALIC: c_int = 1;
const BOLD: c_int = 2;

impl OutlineStyle {
    /// The style as the shim reads it: `/F` bits and `0xRRGGBB` or -1.
    pub(crate) fn from_raw(flags: c_int, color: c_int) -> Self {
        let color = u32::try_from(color)
            .ok()
            .map(|rgb| rgb.to_be_bytes())
            .map(|[_, r, g, b]| [r, g, b])
            .filter(|rgb| *rgb != [0, 0, 0]);
        Self {
            bold: flags & BOLD != 0,
            italic: flags & ITALIC != 0,
            color,
        }
    }

    /// The style as the shim writes it: `/F` bits and `0xRRGGBB` or -1.
    pub(crate) fn to_raw(self) -> (c_int, c_int) {
        let flags = if self.bold { BOLD } else { 0 } | if self.italic { ITALIC } else { 0 };
        let color = self
            .color
            .filter(|rgb| *rgb != [0, 0, 0])
            .map_or(-1, |[r, g, b]| {
                c_int::from(r) << 16 | c_int::from(g) << 8 | c_int::from(b)
            });
        (flags, color)
    }
}

/// One entry of a document's outline.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutlineItem {
    /// The entry's title.
    pub title: String,
    /// Where it links, when that is outside the document (a web address) or MuPDF keeps the link text.
    pub uri: Option<String>,
    /// The page it opens, counting from zero; `None` when it points nowhere in this document.
    pub page: Option<usize>,
    /// Whether the entry starts expanded.
    pub open: bool,
    /// How its title is drawn (B.7).
    pub style: OutlineStyle,
    /// Its child entries.
    pub children: Vec<OutlineItem>,
}

/// Drops the whole outline tree when it goes out of scope, even if copying it failed half way.
struct Tree {
    ctx: *mut fz_context,
    first: *mut fz_outline,
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: `first` is the outline we loaded (or NULL, which the shim accepts), dropped once, with the context
        // that is current for this whole call.
        unsafe { ex_drop_outline(self.ctx, self.first) };
    }
}

fn text(ptr: *const core::ffi::c_char) -> Option<String> {
    // SAFETY: the shim gives NUL-terminated UTF-8 strings that live as long as the outline, or NULL.
    NonNull::new(ptr.cast_mut()).map(|p| {
        unsafe { CStr::from_ptr(p.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

/// Copies the entries from `first` along its `next` chain, and their children.
fn copy(
    ctx: *mut fz_context,
    doc: NonNull<fz_document>,
    first: *mut fz_outline,
    depth: usize,
) -> Result<Vec<OutlineItem>> {
    let mut items = Vec::new();
    let mut node = first;
    while !node.is_null() {
        let mut raw = ex_outline_item::default();
        // SAFETY: `node` is part of the live outline tree of `doc`.
        check(|err| unsafe { ex_outline_item_of(ctx, doc.as_ptr(), node, &mut raw, err) })?;
        let children = if depth < MAX_DEPTH {
            copy(ctx, doc, raw.down, depth + 1)?
        } else {
            Vec::new()
        };
        items.push(OutlineItem {
            title: text(raw.title).unwrap_or_default(),
            uri: text(raw.uri),
            page: usize::try_from(raw.page).ok(),
            open: raw.is_open != 0,
            style: OutlineStyle::from_raw(raw.flags, raw.color),
            children,
        });
        node = raw.next;
    }
    Ok(items)
}

/// Loads and copies a document's outline.
pub(crate) fn load(doc: NonNull<fz_document>) -> Result<Vec<OutlineItem>> {
    with_context(|ctx| {
        let mut first = ptr::null_mut();
        // SAFETY: the document is live and used by this thread only (the caller borrows it).
        check(|err| unsafe { ex_load_outline(ctx, doc.as_ptr(), &mut first, err) })?;
        let tree = Tree { ctx, first };
        copy(ctx, doc, tree.first, 0)
    })
}
