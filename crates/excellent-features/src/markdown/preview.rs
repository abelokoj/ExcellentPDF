//! Markdown preview controller over app URL scheme with MuPDF fallback (plan N.14 steps 3–4).
//!
//! Serves rendered HTML, KaTeX, and Mermaid pack assets over `excellentpdf-md://` (never `file://`),
//! builds `web_view::evaluate` scroll-sync commands, and switches cleanly to MuPDF rendering when
//! the host reports that no WebView runtime is available.

use excellent_core::notebook::Result;
use excellent_mupdf::Document;
use serde_json::{Value, json};

use crate::markdown::render::{RenderOptions, RenderedMarkdown, escape_html, render_gfm_html};
use crate::markdown::toc::TocHeading;

/// Custom app URL scheme used to serve Markdown previews and KaTeX/Mermaid pack assets (N.14 step 4).
pub const PREVIEW_SCHEME: &str = "excellentpdf-md://";

/// Active rendering backend for a Markdown preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PreviewBackend {
    /// Platform WebView (WebView2 / WKWebView / WebKitGTK) with KaTeX and Mermaid packs.
    #[default]
    WebView,
    /// Fallback when no WebView runtime exists: render directly through MuPDF without scripts.
    MuPdfFallback,
}

/// Configuration for the Markdown preview controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewConfig {
    /// Document title shown in `<title>`.
    pub title: String,
    /// Whether the KaTeX/Mermaid pack (K.21) is installed locally.
    pub math_mermaid_pack_installed: bool,
    /// Page width in points used when rendering the MuPDF fallback.
    pub fallback_page_width_pt: u32,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            title: "Markdown Preview".into(),
            math_mermaid_pack_installed: true,
            fallback_page_width_pt: 540,
        }
    }
}

/// Complete WebView HTML document plus TOC metadata ready to serve over `excellentpdf-md://`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewBundle {
    /// Full standalone HTML document referencing only `excellentpdf-md://` pack URLs.
    pub document_html: String,
    /// Heading TOC matching every heading anchor in `document_html`.
    pub headings: Vec<TocHeading>,
    /// Active backend used to produce this preview.
    pub backend: PreviewBackend,
    /// Whether math blocks were detected.
    pub has_math: bool,
    /// Whether Mermaid blocks were detected.
    pub has_mermaid: bool,
}

/// Stateful Markdown preview coordinator handling WebView rendering, scroll sync, and fallback.
#[derive(Debug, Clone)]
pub struct PreviewController {
    config: PreviewConfig,
    backend: PreviewBackend,
}

impl PreviewController {
    /// Creates a preview controller starting on [`PreviewBackend::WebView`].
    #[must_use]
    pub fn new(config: PreviewConfig) -> Self {
        Self {
            config,
            backend: PreviewBackend::WebView,
        }
    }

    /// Returns the active preview backend.
    #[must_use]
    pub fn backend(&self) -> PreviewBackend {
        self.backend
    }

    /// Called when NSH signals that the platform cannot display a WebView runtime (N.14 step 3).
    pub fn mark_webview_unavailable(&mut self) {
        self.backend = PreviewBackend::MuPdfFallback;
    }

    /// Builds the preview HTML document and TOC for `markdown`.
    #[must_use]
    pub fn build_preview(&self, markdown: &str) -> PreviewBundle {
        let rich =
            self.backend == PreviewBackend::WebView && self.config.math_mermaid_pack_installed;
        let opts = if rich {
            RenderOptions::rich_preview()
        } else {
            RenderOptions::plain_fallback()
        };
        let rendered = render_gfm_html(markdown, opts);
        let document_html = self.wrap_html(&rendered, rich);
        PreviewBundle {
            document_html,
            headings: rendered.headings,
            backend: self.backend,
            has_math: rendered.has_math,
            has_mermaid: rendered.has_mermaid,
        }
    }

    /// Renders `markdown` through MuPDF into a viewable PDF buffer for [`PreviewBackend::MuPdfFallback`].
    ///
    /// # Errors
    ///
    /// Returns [`excellent_core::notebook::NotebookError::MuPdf`] if MuPDF cannot lay out the fallback HTML.
    pub fn render_mupdf_fallback_pdf(&self, markdown: &str) -> Result<Vec<u8>> {
        let rendered = render_gfm_html(markdown, RenderOptions::plain_fallback());
        let css = "body{font-family:sans-serif;font-size:11pt;line-height:1.45;margin:24pt;color:#1f2937}\
            h1,h2,h3{color:#111827;margin-top:14pt;margin-bottom:6pt}\
            pre,code{font-family:monospace;background:#f3f4f6}\
            pre{padding:8pt}\
            table{border-collapse:collapse;width:100%}\
            th,td{border:0.5pt solid #cbd5e1;padding:4pt 6pt}";
        let pdf = excellent_mupdf::html_to_pdf(
            &rendered.html,
            css,
            self.config.fallback_page_width_pt.max(200) as f32,
        )?;
        // Verify MuPDF opens the fallback PDF cleanly.
        let _doc = Document::from_bytes(&pdf, "application/pdf")?;
        Ok(pdf)
    }

    /// Generates a JavaScript snippet passed to NSH's `web_view::evaluate` to scroll the preview
    /// to source line `line` (1-based).
    #[must_use]
    pub fn scroll_sync_script(line: usize) -> String {
        format!(
            "(function(){{const el=document.querySelector('[data-line=\"{line}\"]');if(el)el.scrollIntoView({{block:'start',behavior:'smooth'}});}})();"
        )
    }

    /// Serializes `headings` as JSON for the WebView/sidebar TOC bridge.
    #[must_use]
    pub fn headings_json(headings: &[TocHeading]) -> Value {
        Value::Array(
            headings
                .iter()
                .map(|h| {
                    json!({
                        "level": h.level,
                        "title": h.title,
                        "slug": h.slug,
                        "line": h.line,
                    })
                })
                .collect(),
        )
    }

    fn wrap_html(&self, rendered: &RenderedMarkdown, include_pack_scripts: bool) -> String {
        let mut scripts = String::new();
        if include_pack_scripts && rendered.has_math {
            scripts.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"{PREVIEW_SCHEME}pack/katex/katex.min.css\" />\n\
                 <script defer src=\"{PREVIEW_SCHEME}pack/katex/katex.min.js\"></script>\n\
                 <script defer src=\"{PREVIEW_SCHEME}pack/katex/auto-render.min.js\"></script>\n"
            ));
        }
        if include_pack_scripts && rendered.has_mermaid {
            scripts.push_str(&format!(
                "<script defer src=\"{PREVIEW_SCHEME}pack/mermaid/mermaid.min.js\"></script>\n"
            ));
        }

        format!(
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\" /><title>{}</title>\n\
             <style>body{{font-family:system-ui,sans-serif;max-width:780px;margin:24px auto;padding:0 20px;line-height:1.6}}\
             pre{{background:#f6f8fa;padding:12px;border-radius:6px;overflow:auto}}\
             table{{border-collapse:collapse}}th,td{{border:1px solid #d0d7de;padding:6px 10px}}</style>\n\
             {scripts}</head><body>\n{}\n</body></html>",
            escape_html(&self.config.title),
            rendered.html
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_serves_over_app_scheme_and_falls_back_cleanly_to_mupdf() {
        let sample = "# README\n\nEnergy $E = mc^2$.\n\n```mermaid\ngraph LR\n  A-->B\n```\n";
        let mut ctrl = PreviewController::new(PreviewConfig::default());
        let bundle = ctrl.build_preview(sample);

        assert_eq!(bundle.backend, PreviewBackend::WebView);
        assert!(
            bundle
                .document_html
                .contains("excellentpdf-md://pack/katex/katex.min.js")
        );
        assert!(
            bundle
                .document_html
                .contains("excellentpdf-md://pack/mermaid/mermaid.min.js")
        );
        assert!(
            !bundle.document_html.contains("file://"),
            "preview must never reference file:// URLs"
        );
        assert!(PreviewController::scroll_sync_script(4).contains("data-line=\"4\""));

        // Simulate missing WebView runtime (e.g. WebView2 unavailable on Windows).
        ctrl.mark_webview_unavailable();
        assert_eq!(ctrl.backend(), PreviewBackend::MuPdfFallback);
        let fallback_bundle = ctrl.build_preview(sample);
        assert!(!fallback_bundle.document_html.contains("katex.min.js"));
        assert!(!fallback_bundle.document_html.contains("mermaid.min.js"));

        let pdf = ctrl
            .render_mupdf_fallback_pdf(sample)
            .expect("mupdf fallback pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open fallback");
        assert!(doc.page_count().expect("pages") >= 1);
        let text = doc
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("st")
            .text()
            .expect("t");
        assert!(
            text.contains("README"),
            "fallback renders readable text: {text}"
        );
    }
}
