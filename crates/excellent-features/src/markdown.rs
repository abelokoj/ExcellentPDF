//! Markdown editing with math (KaTeX) and Mermaid diagrams (plan N.14).
//!
//! Provides GitHub-compatible heading TOC extraction ([`toc`], ported from SumatraPDF Enhanced's
//! `MarkdownToc.cpp`), GFM HTML rendering with math and Mermaid blocks ([`render`]), and a
//! preview controller serving assets over an app URL scheme with automatic fallback to MuPDF when
//! a platform WebView runtime is unavailable ([`preview`]).

pub mod preview;
pub mod render;
pub mod toc;

pub use preview::{PreviewBackend, PreviewBundle, PreviewConfig, PreviewController};
pub use render::{RenderOptions, RenderedMarkdown, render_gfm_html};
pub use toc::{TocHeading, extract_toc, github_slug};
