//! Cancelling long MuPDF work from another thread.

use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};

use excellent_mupdf_sys::{FZ_ERROR_ABORT, fz_cookie};

use crate::error::{MuError, Result};

/// A cancel switch for one operation, such as building a display list or rendering a tile.
///
/// Pass it to the operation on a worker thread; call [`Cookie::abort`] from any thread to stop that work early. The
/// operation then returns an error for which [`MuError::is_aborted`] is true.
#[derive(Default)]
pub struct Cookie {
    raw: UnsafeCell<fz_cookie>,
}

// SAFETY: MuPDF reads `abort` and writes the progress fields from the one thread running the operation; other threads
// only touch `abort`, through atomic operations. Sharing a cookie between the working thread and the cancelling one
// is how MuPDF documents it (fitz/device.h).
unsafe impl Send for Cookie {}
// SAFETY: as for `Send`.
unsafe impl Sync for Cookie {}

impl core::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Cookie")
            .field("aborted", &self.is_aborted())
            .finish()
    }
}

impl Cookie {
    /// A cookie that hasn't been aborted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the operation using this cookie, or keeps it from starting.
    pub fn abort(&self) {
        self.abort_flag().store(1, Ordering::Relaxed);
    }

    /// [`Cookie::abort`] was called.
    pub fn is_aborted(&self) -> bool {
        self.abort_flag().load(Ordering::Relaxed) != 0
    }

    /// `Ok` unless aborted. Operations call this after MuPDF returns, since an aborted run stops quietly, and a job
    /// queue can call it before starting work.
    ///
    /// # Errors
    ///
    /// The cookie was aborted.
    pub fn check(&self) -> Result<()> {
        if self.is_aborted() {
            Err(MuError::new(FZ_ERROR_ABORT, "cancelled"))
        } else {
            Ok(())
        }
    }

    /// The pointer MuPDF takes. Valid while `self` is borrowed.
    pub(crate) fn as_ptr(&self) -> *mut fz_cookie {
        self.raw.get()
    }

    fn abort_flag(&self) -> &AtomicI32 {
        // SAFETY: `abort` is an aligned `c_int` (i32 on every target) inside `self`, so it lives as long as the
        // returned borrow; all Rust access to it is atomic, and MuPDF only reads it.
        unsafe { AtomicI32::from_ptr(ptr::addr_of_mut!((*self.raw.get()).abort)) }
    }
}

/// The raw cookie for an optional `Cookie`: null when there is none.
pub(crate) fn raw(cookie: Option<&Cookie>) -> *mut fz_cookie {
    cookie.map_or(ptr::null_mut(), Cookie::as_ptr)
}

/// After an operation: aborted means its result is incomplete, so report that instead.
pub(crate) fn finish<T>(cookie: Option<&Cookie>, value: T) -> Result<T> {
    cookie.map_or(Ok(()), Cookie::check)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_is_seen_and_reported() {
        let cookie = Cookie::new();
        assert!(cookie.check().is_ok());
        cookie.abort();
        assert!(cookie.is_aborted());
        assert!(cookie.check().expect_err("aborted").is_aborted());
        assert!(finish(Some(&cookie), ()).is_err());
        assert!(finish(None, ()).is_ok());
    }
}
