//! Links between pages and deep links across notebooks/documents (plan N.9).
//!
//! Creates standard PDF `/Link` annotations for internal page jumps and `ecclesiapdf://`
//! deep-link URIs (compatible with F.13), keeping links working in every exported PDF.

use ecclesia_core::anchor::new_id;
use ecclesia_core::notebook::{
    LinkDestination, Notebook, NotebookError, PageLinkMeta, PdfLinkSpec, PdfLinkTarget, Result,
    pdf_ops,
};
use ecclesia_mupdf::{PagePoint, PageRect};

/// URL scheme used for cross-document and cross-notebook deep links (plan N.9 / F.13).
pub const DEEP_LINK_SCHEME: &str = "ecclesiapdf://";

/// Parsed components of an `ecclesiapdf://doc/<id>?page=<p>&x=<x>&y=<y>` deep link.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepLink {
    /// Target document or notebook identifier.
    pub doc_id: String,
    /// Optional zero-based target page index.
    pub page: Option<usize>,
    /// Optional target coordinates in page points.
    pub point: Option<PagePoint>,
}

impl DeepLink {
    /// Formats this deep link as an `ecclesiapdf://` URI string.
    #[must_use]
    pub fn to_uri(&self) -> String {
        let mut uri = format!("{DEEP_LINK_SCHEME}doc/{}", self.doc_id);
        let mut sep = '?';
        if let Some(page) = self.page {
            uri.push(sep);
            uri.push_str(&format!("page={page}"));
            sep = '&';
        }
        if let Some(pt) = self.point {
            uri.push(sep);
            uri.push_str(&format!("x={:.2}&y={:.2}", pt.x, pt.y));
        }
        uri
    }

    /// Parses an `ecclesiapdf://` deep link URI.
    #[must_use]
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix(DEEP_LINK_SCHEME)?;
        let rest = rest.strip_prefix("doc/")?;
        let (doc_id, query) = match rest.split_once('?') {
            Some((id, q)) => (id, Some(q)),
            None => (rest, None),
        };
        if doc_id.is_empty() {
            return None;
        }
        let mut page = None;
        let mut x = None;
        let mut y = None;
        if let Some(q) = query {
            for pair in q.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    match k {
                        "page" => page = v.parse::<usize>().ok(),
                        "x" => x = v.parse::<f32>().ok(),
                        "y" => y = v.parse::<f32>().ok(),
                        _ => {}
                    }
                }
            }
        }
        let point = match (x, y) {
            (Some(px), Some(py)) => Some(PagePoint { x: px, y: py }),
            _ => None,
        };
        Some(Self {
            doc_id: doc_id.to_owned(),
            page,
            point,
        })
    }
}

/// Adds an internal page-to-page link to `notebook` and writes the corresponding PDF `/Link`
/// annotation into `document.pdf` (N.9 step 1).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if `source_page` or `target_page` is out of range.
pub fn add_internal_link(
    notebook: &mut Notebook,
    source_page: usize,
    rect: PageRect,
    target_page: usize,
    point: Option<PagePoint>,
    zoom: Option<f32>,
    label: Option<String>,
) -> Result<PageLinkMeta> {
    let count = notebook.metadata.pages.len();
    if source_page >= count || target_page >= count {
        return Err(NotebookError::NotFound(format!(
            "link page index out of range (source={source_page}, target={target_page}, count={count})"
        )));
    }
    let meta = PageLinkMeta {
        id: new_id(),
        source_page,
        rect,
        label,
        destination: LinkDestination::Page {
            page: target_page,
            point,
            zoom,
        },
    };
    let spec = PdfLinkSpec {
        source_page,
        rect,
        target: PdfLinkTarget::InternalPage {
            page: target_page,
            point,
            zoom,
        },
    };
    let updated = pdf_ops::add_links(notebook.pdf_bytes(), &[spec])?;
    notebook.replace_pdf_bytes(updated)?;
    notebook.metadata.links.push(meta.clone());
    Ok(meta)
}

/// Adds a deep link to another notebook or document (`ecclesiapdf://...`) to `notebook` (N.9 step 2).
///
/// # Errors
///
/// Returns [`NotebookError::NotFound`] if `source_page` is out of range.
pub fn add_deep_link(
    notebook: &mut Notebook,
    source_page: usize,
    rect: PageRect,
    target: &DeepLink,
    label: Option<String>,
) -> Result<PageLinkMeta> {
    let count = notebook.metadata.pages.len();
    if source_page >= count {
        return Err(NotebookError::NotFound(format!(
            "link source page {source_page} out of range ({count})"
        )));
    }
    let uri = target.to_uri();
    let meta = PageLinkMeta {
        id: new_id(),
        source_page,
        rect,
        label,
        destination: LinkDestination::Uri(uri.clone()),
    };
    let spec = PdfLinkSpec {
        source_page,
        rect,
        target: PdfLinkTarget::Uri(uri),
    };
    let updated = pdf_ops::add_links(notebook.pdf_bytes(), &[spec])?;
    notebook.replace_pdf_bytes(updated)?;
    notebook.metadata.links.push(meta.clone());
    Ok(meta)
}

/// Returns all links originating from `page`.
#[must_use]
pub fn links_on_page(notebook: &Notebook, page: usize) -> Vec<&PageLinkMeta> {
    notebook
        .metadata
        .links
        .iter()
        .filter(|l| l.source_page == page)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::notebook::{ExportOptions, Orientation, PageSize, PaperTemplateId};
    use ecclesia_mupdf::Document;

    #[test]
    fn internal_and_deep_links_work_in_exported_pdfs() {
        let mut nb = Notebook::new_blank(
            "Linked Notes",
            PageSize::Letter,
            Orientation::Portrait,
            PaperTemplateId::Lined,
        )
        .expect("nb");
        let pdf2 = pdf_ops::insert_page(nb.pdf_bytes(), 1, 612.0, 792.0, "").expect("p2");
        nb.replace_pdf_bytes(pdf2).expect("replace");

        let rect1 = PageRect {
            x0: 72.0,
            y0: 100.0,
            x1: 220.0,
            y1: 124.0,
        };
        let rect2 = PageRect {
            x0: 72.0,
            y0: 150.0,
            x1: 260.0,
            y1: 174.0,
        };

        add_internal_link(
            &mut nb,
            0,
            rect1,
            1,
            Some(PagePoint { x: 40.0, y: 80.0 }),
            None,
            Some("See details on page 2".into()),
        )
        .expect("internal link");

        let deep = DeepLink {
            doc_id: "nb-calculus-2026".into(),
            page: Some(12),
            point: Some(PagePoint { x: 50.0, y: 120.5 }),
        };
        assert_eq!(DeepLink::parse(&deep.to_uri()), Some(deep.clone()));
        add_deep_link(&mut nb, 0, rect2, &deep, Some("Calculus notes".into())).expect("deep link");

        let exported = nb
            .export_pdf(ExportOptions::default(), |_, _, _, _| String::new())
            .expect("export");
        let doc = Document::from_bytes(&exported, "application/pdf").expect("open exported");
        let links = doc.load_page(0).expect("p0").links().expect("links");
        assert!(
            links.iter().any(|l| l.target.is_some_and(|t| t.page == 1)),
            "internal link resolves to page 1: {links:?}"
        );
        assert!(
            links.iter().any(|l| l.uri.starts_with(DEEP_LINK_SCHEME)),
            "deep link URI preserved in exported PDF: {links:?}"
        );
    }
}
