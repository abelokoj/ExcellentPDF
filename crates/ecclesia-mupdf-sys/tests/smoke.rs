//! T3 smoke test: the shim opens a corpus PDF, counts its pages, and exercises every other shim call once.

use core::ffi::CStr;
use core::ptr;
use std::ffi::CString;
use std::path::PathBuf;

use ecclesia_mupdf_sys::*;

/// `tests/corpus/zlib-manual.pdf`: zlib's manual page, 2 US Letter pages (groff, Ghostscript).
fn corpus(name: &str) -> CString {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    CString::new(path.to_str().expect("corpus path is UTF-8")).expect("corpus path has no NUL")
}

fn message(err: &ex_error) -> String {
    // SAFETY: the shim always NUL-terminates `message` (snprintf), and a zeroed one is an empty string.
    unsafe { CStr::from_ptr(err.message.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn opens_counts_renders_and_extracts_text() {
    let mut err = ex_error::default();
    let mut ctx = ptr::null_mut();
    // SAFETY: out-pointers are valid; on success `ctx` is a new context we drop at the end.
    let rc = unsafe { ex_new_context(&mut ctx, &mut err) };
    assert_eq!(rc, 0, "ex_new_context: {}", message(&err));

    let path = corpus("zlib-manual.pdf");
    let mut doc = ptr::null_mut();
    // SAFETY: `ctx` is live, `path` is a NUL-terminated UTF-8 string.
    let rc = unsafe { ex_open_document(ctx, path.as_ptr(), &mut doc, &mut err) };
    assert_eq!(rc, 0, "ex_open_document: {}", message(&err));

    let mut pages = 0;
    // SAFETY: `doc` belongs to `ctx`.
    let rc = unsafe { ex_count_pages(ctx, doc, &mut pages, &mut err) };
    assert_eq!(rc, 0, "ex_count_pages: {}", message(&err));
    assert_eq!(pages, 2);

    let mut page = ptr::null_mut();
    // SAFETY: `doc` belongs to `ctx`; page 0 exists.
    let rc = unsafe { ex_load_page(ctx, doc, 0, &mut page, &mut err) };
    assert_eq!(rc, 0, "ex_load_page: {}", message(&err));

    let mut bounds = fz_rect::default();
    // SAFETY: `page` belongs to `ctx`.
    let rc = unsafe { ex_page_bounds(ctx, page, &mut bounds, &mut err) };
    assert_eq!(rc, 0, "ex_page_bounds: {}", message(&err));
    assert_eq!(
        (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0),
        (612.0, 792.0),
        "US Letter in points"
    );

    let identity = fz_matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    let mut pix = ptr::null_mut();
    // SAFETY: `page` belongs to `ctx`.
    let rc = unsafe { ex_render_page(ctx, page, identity, 0, &mut pix, &mut err) };
    assert_eq!(rc, 0, "ex_render_page: {}", message(&err));
    let mut info = ex_pixmap_info::default();
    // SAFETY: `pix` belongs to `ctx`.
    unsafe { ex_pixmap_info_of(ctx, pix, &mut info) };
    assert_eq!(
        (info.width, info.height, info.components, info.alpha),
        (612, 792, 3, 0)
    );
    let len = usize::try_from(info.stride).expect("stride is positive") * 792;
    // SAFETY: `samples` points at `stride * height` bytes owned by `pix`, which is alive until dropped below.
    let samples = unsafe { core::slice::from_raw_parts(info.samples, len) };
    assert!(
        samples.iter().any(|&b| b < 128),
        "page 1 has dark pixels (text)"
    );

    let mut stext = ptr::null_mut();
    // SAFETY: `page` belongs to `ctx`.
    let rc = unsafe { ex_new_stext_page(ctx, page, 0, ptr::null_mut(), &mut stext, &mut err) };
    assert_eq!(rc, 0, "ex_new_stext_page: {}", message(&err));
    let mut buf = ptr::null_mut();
    // SAFETY: `stext` belongs to `ctx`.
    let rc = unsafe { ex_stext_page_text(ctx, stext, &mut buf, &mut err) };
    assert_eq!(rc, 0, "ex_stext_page_text: {}", message(&err));
    let (mut data, mut len) = (ptr::null(), 0);
    // SAFETY: `buf` belongs to `ctx`; `data` stays valid until `buf` is dropped.
    unsafe { ex_buffer_data(ctx, buf, &mut data, &mut len) };
    // SAFETY: `data` points at `len` bytes owned by `buf`.
    let text = String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(data, len) });
    assert!(
        text.contains("zlib"),
        "page 1 text mentions zlib: {text:.200}"
    );

    // SAFETY: each object is dropped once, before the context that owns it.
    unsafe {
        ex_drop_buffer(ctx, buf);
        ex_drop_stext_page(ctx, stext);
        ex_drop_pixmap(ctx, pix);
        ex_drop_page(ctx, page);
        ex_drop_document(ctx, doc);
        ex_drop_context(ctx);
    }
}

#[test]
fn reports_errors_instead_of_unwinding() {
    let mut err = ex_error::default();
    let mut ctx = ptr::null_mut();
    // SAFETY: out-pointers are valid.
    assert_eq!(
        unsafe { ex_new_context(&mut ctx, &mut err) },
        0,
        "{}",
        message(&err)
    );

    let missing = corpus("does-not-exist.pdf");
    let mut doc = ptr::null_mut();
    // SAFETY: `ctx` is live, `missing` is NUL-terminated.
    let rc = unsafe { ex_open_document(ctx, missing.as_ptr(), &mut doc, &mut err) };
    assert_ne!(rc, 0);
    assert_eq!(err.code, rc);
    assert!(doc.is_null(), "out-parameter untouched on failure");
    assert!(!message(&err).is_empty());

    // The context is still usable after a caught error.
    let path = corpus("zlib-manual.pdf");
    // SAFETY: as above.
    let rc = unsafe { ex_open_document(ctx, path.as_ptr(), &mut doc, &mut err) };
    assert_eq!(rc, 0, "{}", message(&err));
    // SAFETY: dropped once, document before context.
    unsafe {
        ex_drop_document(ctx, doc);
        ex_drop_context(ctx);
    }
}
