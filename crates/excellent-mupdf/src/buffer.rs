//! MuPDF buffers, copied out and released at once.

use core::ptr;

use excellent_mupdf_sys::{ex_buffer_data, ex_drop_buffer, fz_buffer, fz_context};

/// Copies a buffer's bytes and drops the buffer.
///
/// # Safety
///
/// `buf` is a live buffer of `ctx`'s family that the caller owns; it is dropped here.
pub(crate) unsafe fn into_vec(ctx: *mut fz_context, buf: *mut fz_buffer) -> Vec<u8> {
    let (mut data, mut len) = (ptr::null(), 0);
    // SAFETY: `buf` is live (caller's contract); the out-pointers are valid.
    unsafe { ex_buffer_data(ctx, buf, &mut data, &mut len) };
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: `data` points at `len` bytes owned by `buf`, which is alive until dropped below.
        unsafe { core::slice::from_raw_parts(data, len) }.to_vec()
    };
    // SAFETY: the caller owns `buf` and doesn't use it again.
    unsafe { ex_drop_buffer(ctx, buf) };
    bytes
}

/// Copies a buffer of UTF-8 text into a `String`, replacing invalid sequences, and drops the buffer.
///
/// # Safety
///
/// As [`into_vec`].
pub(crate) unsafe fn into_string(ctx: *mut fz_context, buf: *mut fz_buffer) -> String {
    // SAFETY: forwarded from the caller.
    let bytes = unsafe { into_vec(ctx, buf) };
    String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}
