//! The per-thread MuPDF contexts (plan §5.3).
//!
//! One base context is made per process, with real locks and the built-in font loader; it is never dropped. Each
//! thread clones it on its first MuPDF call and drops the clone when the thread exits. Objects made on one thread may
//! be used with any context of the family, which is what lets [`crate::DisplayList`] cross threads.

use core::ffi::c_int;
use core::ptr::{self, NonNull};
use std::sync::{Mutex, OnceLock};

use ecclesia_mupdf_sys::{
    ex_clone_context, ex_drop_context, ex_error, ex_new_context, ex_set_font_loader,
    ex_set_system_font_lookup, fz_context,
};

use crate::error::{MuError, Result};
use crate::{fonts, system_fonts};

/// The base context. Only used under [`BASE`]'s mutex: to clone it, and for drops on a thread whose own context is
/// already gone.
struct Base(NonNull<fz_context>);

// SAFETY: the base context is only touched while holding the mutex around it, so one thread at a time.
unsafe impl Send for Base {}

static BASE: OnceLock<core::result::Result<Mutex<Base>, MuError>> = OnceLock::new();

fn base() -> Result<&'static Mutex<Base>> {
    BASE.get_or_init(|| {
        // SAFETY: the loaders only read static/cached data and are safe to call from any thread.
        unsafe {
            ex_set_font_loader(Some(fonts::load));
            ex_set_system_font_lookup(Some(system_fonts::lookup));
        }
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` is a valid out-pointer; `check` gives the error on failure.
        check(|err| unsafe { ex_new_context(&mut raw, err) })?;
        let raw = NonNull::new(raw).expect("ex_new_context succeeded with a context");
        Ok(Mutex::new(Base(raw)))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// This thread's clone of the base context.
struct ThreadContext(NonNull<fz_context>);

impl ThreadContext {
    fn new() -> Result<Self> {
        let base = base()?.lock().expect("MuPDF base context lock poisoned");
        let mut raw = ptr::null_mut();
        // SAFETY: the base is live (never dropped) and held exclusively through the mutex.
        check(|err| unsafe { ex_clone_context(base.0.as_ptr(), &mut raw, err) })?;
        Ok(Self(
            NonNull::new(raw).expect("ex_clone_context succeeded with a context"),
        ))
    }
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        // SAFETY: the clone belongs to this thread, and objects dropped later fall back to the base.
        unsafe { ex_drop_context(self.0.as_ptr()) }
    }
}

thread_local! {
    static CONTEXT: Result<ThreadContext> = ThreadContext::new();
}

/// Runs `f` with this thread's context.
///
/// While the thread is exiting and its context is already gone, `f` runs with the base context instead, under its
/// lock, so objects dropped late are still released.
pub(crate) fn with_context<R>(f: impl FnOnce(*mut fz_context) -> Result<R>) -> Result<R> {
    let mut f = Some(f);
    let on_thread = CONTEXT.try_with(|context| {
        let f = f.take().expect("the closure runs once");
        match context {
            Ok(context) => f(context.0.as_ptr()),
            Err(err) => Err(err.clone()),
        }
    });
    match on_thread {
        Ok(result) => result,
        Err(_) => {
            let f = f
                .take()
                .expect("the closure did not run: the thread's context is gone");
            let base = base()?.lock().expect("MuPDF base context lock poisoned");
            f(base.0.as_ptr())
        }
    }
}

/// Releases a MuPDF object in `Drop`. Drops never fail: if no context can be had at all (MuPDF never started), the
/// object is leaked.
pub(crate) fn drop_with_context(f: impl FnOnce(*mut fz_context)) {
    let _ = with_context(|ctx| {
        f(ctx);
        Ok(())
    });
}

/// Calls a shim function that reports failure as a non-zero code and an [`ex_error`].
pub(crate) fn check(f: impl FnOnce(&mut ex_error) -> c_int) -> Result<()> {
    let mut err = ex_error::default();
    match f(&mut err) {
        0 => Ok(()),
        _ => Err(MuError::from_shim(&err)),
    }
}
