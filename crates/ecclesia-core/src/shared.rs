//! The process-wide render workers and open documents, shared by every view and feature (plan K.4, K.5): the page
//! canvases, the thumbnails and, later, search and export use the same pool and the same parsed files.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::registry::Registry;
use crate::render::RenderPool;

/// The render workers, started on first use.
pub fn pool() -> &'static RenderPool {
    static POOL: OnceLock<RenderPool> = OnceLock::new();
    POOL.get_or_init(RenderPool::default)
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// The open documents: a file shown twice, or shown and thumbnailed, is parsed once.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::new)
}

/// Keeps the open documents' operation logs (K.14) in `folder`, the app's own. Call it before the first document
/// opens; `false` when that already happened, and then logs stay in the system's temporary folder.
pub fn keep_logs_in(folder: PathBuf) -> bool {
    REGISTRY.set(Registry::with_log_dir(folder)).is_ok()
}
