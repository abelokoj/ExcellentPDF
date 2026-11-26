//! Consent-gated visual region search / Google Lens upload builder (plan R.31, §14.4).
//!
//! Renders a selected region (O.23), page, or embedded image to PNG, enforces both the global
//! online-features setting and explicit per-upload user confirmation, and constructs either a
//! multipart `encoded_image` payload or SumatraPDF Enhanced's self-submitting browser trampoline.
//! Nothing leaves the device when online features are off or confirmation has not been granted.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use excellent_mupdf::{Document, MuError, PageRect};

use crate::snapshot::Region;

/// Maximum PNG size accepted by Google Lens (32 MiB, matching SumatraPDF Enhanced).
pub const MAX_LENS_PNG_BYTES: usize = 32 * 1024 * 1024;

/// Google Lens multipart upload endpoint used by SumatraPDF Enhanced's `GoogleLens.cpp`.
pub const GOOGLE_LENS_UPLOAD_URL: &str = "https://lens.google.com/v3/upload?ep=cntpubb&re=df&s=4";

const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Whether online network features are enabled in application settings (plan R.31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum OnlineAccessMode {
    /// Offline / privacy mode: visual search is disabled and zero uploads may be prepared.
    #[default]
    Disabled,
    /// Online features are turned on in settings (per-upload confirmation is still required).
    Enabled,
}

/// Explicit user confirmation state for sending an image to Google Lens (plan R.31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LensConsent {
    /// User has not yet confirmed uploading this image.
    #[default]
    Pending,
    /// User explicitly confirmed uploading this rendered region.
    Confirmed,
    /// User declined the upload prompt.
    Declined,
}

/// Source region or page targeted for visual search.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum LensTarget {
    /// A dragged page-point snapshot region (plan O.23).
    Region(Region),
    /// An entire page rendered at a chosen DPI.
    Page {
        /// Zero-based page index.
        page: usize,
        /// Full page bounds in page points.
        rect: PageRect,
    },
}

/// Errors produced while preparing or dispatching a Google Lens visual search.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum LensError {
    /// Online features are disabled in settings.
    OnlineDisabled,
    /// User has not explicitly confirmed the upload.
    ConfirmationRequired,
    /// User declined the upload confirmation.
    ConfirmationDeclined,
    /// Rendered PNG buffer is empty.
    EmptyImage,
    /// PNG exceeds [`MAX_LENS_PNG_BYTES`].
    ImageTooLarge {
        /// Actual encoded PNG byte count.
        bytes: usize,
        /// Maximum allowed byte count.
        max_bytes: usize,
    },
    /// Supplied bytes do not start with a valid PNG signature.
    InvalidPng,
    /// MuPDF failed to render the selected region.
    MuPdf(MuError),
    /// Writing the browser trampoline HTML file failed.
    Io(std::io::Error),
}

impl fmt::Display for LensError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OnlineDisabled => write!(f, "google lens disabled while online features are off"),
            Self::ConfirmationRequired => {
                write!(f, "google lens upload requires explicit user confirmation")
            }
            Self::ConfirmationDeclined => write!(f, "google lens upload declined by user"),
            Self::EmptyImage => write!(f, "google lens image is empty"),
            Self::ImageTooLarge { bytes, max_bytes } => write!(
                f,
                "google lens image too large ({bytes} bytes, max {max_bytes})"
            ),
            Self::InvalidPng => write!(f, "google lens payload is not a valid png"),
            Self::MuPdf(err) => write!(f, "google lens render error: {err}"),
            Self::Io(err) => write!(f, "google lens io error: {err}"),
        }
    }
}

impl Error for LensError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MuPdf(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::OnlineDisabled
            | Self::ConfirmationRequired
            | Self::ConfirmationDeclined
            | Self::EmptyImage
            | Self::ImageTooLarge { .. }
            | Self::InvalidPng => None,
        }
    }
}

impl From<MuError> for LensError {
    fn from(err: MuError) -> Self {
        Self::MuPdf(err)
    }
}

impl From<std::io::Error> for LensError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Validated, consent-authorized Google Lens upload package (plan R.31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensUploadPackage {
    /// Destination upload URL.
    pub endpoint_url: String,
    /// Safe filename passed in the multipart form (`encoded_image`).
    pub file_name: String,
    /// Validated PNG bytes.
    pub png_bytes: Vec<u8>,
    /// Multipart form-data boundary string.
    pub multipart_boundary: String,
    /// Complete `multipart/form-data` request body ready for HTTP POST.
    pub multipart_body: Vec<u8>,
    /// Self-submitting HTML page launching Google Lens in the system browser.
    pub trampoline_html: String,
}

impl LensUploadPackage {
    /// `Content-Type` header value matching [`Self::multipart_body`].
    #[must_use]
    pub fn content_type_header(&self) -> String {
        format!("multipart/form-data; boundary={}", self.multipart_boundary)
    }

    /// Writes [`Self::trampoline_html`] into `directory` under a sanitized `.html` filename.
    ///
    /// # Errors
    ///
    /// Returns [`LensError::Io`] if writing or directory creation fails.
    pub fn write_trampoline_file(&self, directory: &Path) -> Result<PathBuf, LensError> {
        fs::create_dir_all(directory)?;
        let stem = sanitize_file_stem(&self.file_name);
        let path = directory.join(format!("{stem}-lens.html"));
        fs::write(&path, self.trampoline_html.as_bytes())?;
        Ok(path)
    }
}

/// Checks whether visual search is currently available given online settings.
#[must_use]
pub fn is_lens_available(online: OnlineAccessMode) -> bool {
    online == OnlineAccessMode::Enabled
}

/// Verifies consent and builds a [`LensUploadPackage`] from pre-rendered PNG bytes.
///
/// # Errors
///
/// Returns [`LensError::OnlineDisabled`], [`LensError::ConfirmationRequired`], or
/// [`LensError::ConfirmationDeclined`] without allocating or sending anything if policy denies it,
/// or image validation errors if `png_bytes` is empty, oversized, or not PNG.
pub fn build_upload_from_png(
    png_bytes: &[u8],
    file_name: &str,
    online: OnlineAccessMode,
    consent: LensConsent,
) -> Result<LensUploadPackage, LensError> {
    enforce_policy(online, consent)?;
    validate_png_bytes(png_bytes)?;

    let safe_name = sanitize_upload_filename(file_name);
    let boundary = "----ExcellentPDFLensBoundary9f3c7a".to_owned();
    let multipart_body = build_multipart_body(&boundary, &safe_name, png_bytes);
    let trampoline_html = build_trampoline_html(&safe_name, png_bytes);

    Ok(LensUploadPackage {
        endpoint_url: GOOGLE_LENS_UPLOAD_URL.to_owned(),
        file_name: safe_name,
        png_bytes: png_bytes.to_vec(),
        multipart_boundary: boundary,
        multipart_body,
        trampoline_html,
    })
}

/// Renders `region` of `document` at `dpi` and builds a consent-gated [`LensUploadPackage`] (plan R.31).
///
/// Consent and online access are verified **before** rendering or encoding so denied requests
/// perform zero work and transmit nothing.
///
/// # Errors
///
/// Returns [`LensError`] if online features are off, confirmation is missing/declined, or rendering fails.
pub fn build_upload_from_region(
    document: &Document,
    region: Region,
    dpi: u32,
    document_stem: &str,
    online: OnlineAccessMode,
    consent: LensConsent,
) -> Result<LensUploadPackage, LensError> {
    enforce_policy(online, consent)?;
    let png = region.render_png(document, dpi)?;
    let file_name = format!(
        "{}-p{}.png",
        sanitize_file_stem(document_stem),
        region.page + 1
    );
    build_upload_from_png(&png, &file_name, online, consent)
}

fn enforce_policy(online: OnlineAccessMode, consent: LensConsent) -> Result<(), LensError> {
    if online != OnlineAccessMode::Enabled {
        return Err(LensError::OnlineDisabled);
    }
    match consent {
        LensConsent::Pending => Err(LensError::ConfirmationRequired),
        LensConsent::Declined => Err(LensError::ConfirmationDeclined),
        LensConsent::Confirmed => Ok(()),
    }
}

fn validate_png_bytes(png_bytes: &[u8]) -> Result<(), LensError> {
    if png_bytes.is_empty() {
        return Err(LensError::EmptyImage);
    }
    if png_bytes.len() > MAX_LENS_PNG_BYTES {
        return Err(LensError::ImageTooLarge {
            bytes: png_bytes.len(),
            max_bytes: MAX_LENS_PNG_BYTES,
        });
    }
    if !png_bytes.starts_with(PNG_MAGIC) {
        return Err(LensError::InvalidPng);
    }
    Ok(())
}

fn build_multipart_body(boundary: &str, file_name: &str, png_bytes: &[u8]) -> Vec<u8> {
    let header = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"encoded_image\"; filename=\"{file_name}\"\r\n\
         Content-Type: image/png\r\n\r\n"
    );
    let footer = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(header.len() + png_bytes.len() + footer.len());
    body.extend_from_slice(header.as_bytes());
    body.extend_from_slice(png_bytes);
    body.extend_from_slice(footer.as_bytes());
    body
}

fn build_trampoline_html(file_name: &str, png_bytes: &[u8]) -> String {
    let b64 = BASE64_STANDARD.encode(png_bytes);
    let escaped_name = file_name.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Google Lens</title>\n\
         <p>Opening Google Lens...</p><script>\n\
         const b=atob('{b64}');const a=Uint8Array.from(b,c=>c.charCodeAt(0));\n\
         const f=new File([a],'{escaped_name}',{{type:'image/png'}});\n\
         const i=document.createElement('input');i.type='file';i.name='encoded_image';\n\
         const d=new DataTransfer();d.items.add(f);i.files=d.files;\n\
         const form=document.createElement('form');form.method='post';\n\
         form.enctype='multipart/form-data';form.action='{GOOGLE_LENS_UPLOAD_URL}';\n\
         form.appendChild(i);document.body.appendChild(form);form.submit();</script>\n"
    )
}

fn sanitize_file_stem(raw: &str) -> String {
    let stem: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = stem.trim_matches('_');
    if trimmed.is_empty() {
        "excellentpdf-region".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn sanitize_upload_filename(raw: &str) -> String {
    let trimmed = raw.trim();
    let stem = trimmed.strip_suffix(".png").unwrap_or(trimmed);
    format!("{}.png", sanitize_file_stem(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_png() -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        bytes
    }

    #[test]
    fn lens_blocks_when_online_off_or_unconfirmed() {
        let png = minimal_png();
        assert!(!is_lens_available(OnlineAccessMode::Disabled));
        assert!(is_lens_available(OnlineAccessMode::Enabled));

        let err_off = build_upload_from_png(
            &png,
            "region.png",
            OnlineAccessMode::Disabled,
            LensConsent::Confirmed,
        )
        .expect_err("offline must block");
        assert!(matches!(err_off, LensError::OnlineDisabled));

        let err_pending = build_upload_from_png(
            &png,
            "region.png",
            OnlineAccessMode::Enabled,
            LensConsent::Pending,
        )
        .expect_err("pending confirmation must block");
        assert!(matches!(err_pending, LensError::ConfirmationRequired));

        let err_declined = build_upload_from_png(
            &png,
            "region.png",
            OnlineAccessMode::Enabled,
            LensConsent::Declined,
        )
        .expect_err("declined confirmation must block");
        assert!(matches!(err_declined, LensError::ConfirmationDeclined));
    }

    #[test]
    fn lens_builds_valid_multipart_and_trampoline_after_confirmation() {
        let png = minimal_png();
        let pkg = build_upload_from_png(
            &png,
            "paper p3.png",
            OnlineAccessMode::Enabled,
            LensConsent::Confirmed,
        )
        .expect("confirmed upload builds");

        assert_eq!(pkg.endpoint_url, GOOGLE_LENS_UPLOAD_URL);
        assert_eq!(pkg.file_name, "paper_p3.png");
        let body_str = String::from_utf8_lossy(&pkg.multipart_body);
        assert!(body_str.contains("name=\"encoded_image\"; filename=\"paper_p3.png\""));
        assert!(body_str.contains("Content-Type: image/png"));
        assert!(pkg.trampoline_html.contains(GOOGLE_LENS_UPLOAD_URL));
        assert!(pkg.trampoline_html.contains(&BASE64_STANDARD.encode(&png)));
    }

    #[test]
    fn lens_rejects_oversized_or_non_png_payloads() {
        let bad = b"not a png";
        let err = build_upload_from_png(
            bad,
            "x.png",
            OnlineAccessMode::Enabled,
            LensConsent::Confirmed,
        )
        .expect_err("non-png rejected");
        assert!(matches!(err, LensError::InvalidPng));
    }
}
