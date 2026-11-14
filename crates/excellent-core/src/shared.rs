//! The process-wide render workers and open documents, shared by every view and feature (plan K.4, K.5): the page
//! canvases, the thumbnails and, later, search and export use the same pool and the same parsed files.

use std::sync::OnceLock;

use crate::registry::Registry;
use crate::render::RenderPool;

/// The render workers, started on first use.
pub fn pool() -> &'static RenderPool {
    static POOL: OnceLock<RenderPool> = OnceLock::new();
    POOL.get_or_init(RenderPool::default)
}

/// The open documents: a file shown twice, or shown and thumbnailed, is parsed once.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}
