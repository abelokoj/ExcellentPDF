//! MuPDF's object types, as opaque FFI types.
//!
//! They can only be used behind pointers. The marker makes them `!Send`, `!Sync` and `!Unpin`: a MuPDF object
//! belongs to the context (and so the thread) that made it, and the safe crate decides what may cross threads.

use core::marker::{PhantomData, PhantomPinned};

macro_rules! opaque {
    ($($(#[$doc:meta])* $name:ident;)*) => {$(
        $(#[$doc])*
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    )*};
}

opaque! {
    /// `fz_context`: MuPDF's per-thread state (allocator, error stack, caches).
    fz_context;
    /// `fz_document`: an open document of any format.
    fz_document;
    /// `fz_page`: a loaded page of a document.
    fz_page;
    /// `fz_pixmap`: a raster image.
    fz_pixmap;
    /// `fz_image`: an image a document holds, decoded on demand.
    fz_image;
    /// `fz_stext_page`: the structured text of a page.
    fz_stext_page;
    /// `fz_buffer`: a growable byte buffer.
    fz_buffer;
    /// `fz_display_list`: a page's drawing commands, recorded once and replayed.
    fz_display_list;
    /// `fz_outline`: one entry of a document's table of contents, with links to the next and the first child.
    fz_outline;
    /// `fz_link`: one link of a page, with a pointer to the next.
    fz_link;
    /// `fz_document_writer`: writes pages to a file in some output format.
    fz_document_writer;
    /// `pdf_annot`: an annotation on a PDF page.
    pdf_annot;
}
