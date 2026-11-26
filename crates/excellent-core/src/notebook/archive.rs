//! `.expdf` ZIP notebook archive reader, atomic writer, and PDF exporter (plan N.4).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};

use excellent_mupdf::Document;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::notebook::model::{
    LinkDestination, NotebookError, NotebookMetadata, Orientation, PageSize, PaperTemplateId,
    Result,
};
use crate::notebook::pdf_ops::{
    self, OutlineEntrySpec, PdfLinkSpec, PdfLinkTarget, build_blank_pdf,
};

/// ZIP entry name for the embedded PDF document.
pub const ENTRY_DOCUMENT_PDF: &str = "document.pdf";
/// ZIP entry name for the notebook JSON metadata.
pub const ENTRY_NOTEBOOK_JSON: &str = "notebook.json";
/// ZIP directory prefix for binary assets (audio recordings, images, stickers).
pub const ASSETS_PREFIX: &str = "assets/";

/// Whether audio assets are embedded into an exported plain PDF (O.25 / N.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttachAudio {
    /// Leave audio assets out of the exported PDF (default).
    #[default]
    Omit,
    /// Attach bundled audio recordings as PDF `/EmbeddedFiles`.
    Embed,
}

/// Options controlling plain PDF export from a [`Notebook`] (N.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ExportOptions {
    /// Whether to attach audio recordings as PDF embedded files.
    pub attach_audio: AttachAudio,
}

/// An in-memory representation of an `.expdf` notebook archive (plan N.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Notebook {
    /// Structured metadata stored in `notebook.json`.
    pub metadata: NotebookMetadata,
    /// Raw bytes of `document.pdf` (kept incremental across edits where possible).
    pdf_bytes: Vec<u8>,
    /// Binary assets keyed by normalized archive path (e.g. `assets/audio/rec1.wav`).
    assets: BTreeMap<String, Vec<u8>>,
}

impl Notebook {
    /// Creates a new blank notebook with a single page of `page_size` in `orientation`.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if the initial PDF cannot be generated.
    pub fn new_blank(
        title: impl Into<String>,
        page_size: PageSize,
        orientation: Orientation,
        template: PaperTemplateId,
    ) -> Result<Self> {
        let (w, h) = page_size.dimensions(orientation);
        let pdf_bytes = build_blank_pdf(&[(w, h)])?;
        let metadata = NotebookMetadata::new(title, page_size, orientation, template);
        Ok(Self {
            metadata,
            pdf_bytes,
            assets: BTreeMap::new(),
        })
    }

    /// Creates a notebook wrapping an existing PDF document's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::MuPdf`] if `pdf_bytes` cannot be opened by MuPDF.
    pub fn from_pdf_bytes(
        title: impl Into<String>,
        pdf_bytes: Vec<u8>,
        template: PaperTemplateId,
    ) -> Result<Self> {
        let doc = Document::from_bytes(&pdf_bytes, "application/pdf")?;
        let count = doc.page_count()?;
        let mut metadata =
            NotebookMetadata::new(title, PageSize::A4, Orientation::Portrait, template);
        metadata.pages = (0..count)
            .map(|_| crate::notebook::model::PageMeta::new(template))
            .collect();
        Ok(Self {
            metadata,
            pdf_bytes,
            assets: BTreeMap::new(),
        })
    }

    /// Opens an `.expdf` notebook from a file on disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the ZIP or JSON is malformed, or MuPDF
    /// cannot open the embedded `document.pdf`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = fs::read(path.as_ref())?;
        Self::from_zip_bytes(&bytes)
    }

    /// Reads an `.expdf` notebook from in-memory ZIP bytes and verifies `document.pdf` opens with MuPDF.
    ///
    /// # Errors
    ///
    /// Returns an error if `document.pdf` or `notebook.json` is missing/corrupt or MuPDF rejects the PDF.
    pub fn from_zip_bytes(bytes: &[u8]) -> Result<Self> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        let mut pdf_bytes: Option<Vec<u8>> = None;
        let mut json_bytes: Option<Vec<u8>> = None;
        let mut assets = BTreeMap::new();

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name()?.replace('\\', "/");
            if name.ends_with('/') {
                continue;
            }
            let mut buf = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            file.read_to_end(&mut buf)?;
            if name == ENTRY_DOCUMENT_PDF {
                pdf_bytes = Some(buf);
            } else if name == ENTRY_NOTEBOOK_JSON {
                json_bytes = Some(buf);
            } else if name.starts_with(ASSETS_PREFIX) && !name.contains("..") {
                assets.insert(name, buf);
            }
        }

        let pdf_bytes = pdf_bytes.ok_or_else(|| {
            NotebookError::InvalidFormat(format!("archive missing {ENTRY_DOCUMENT_PDF}"))
        })?;
        let json_bytes = json_bytes.ok_or_else(|| {
            NotebookError::InvalidFormat(format!("archive missing {ENTRY_NOTEBOOK_JSON}"))
        })?;

        // Validate document.pdf opens via MuPDF (`fz_open_document_with_stream`, plan N.4 step 2).
        let doc = Document::from_bytes(&pdf_bytes, "application/pdf")?;
        let pdf_pages = doc.page_count()?;

        let mut metadata = NotebookMetadata::from_json_bytes(&json_bytes)?;
        // Keep metadata page entries aligned with the PDF's actual page count if external edits occurred.
        while metadata.pages.len() < pdf_pages {
            metadata.pages.push(crate::notebook::model::PageMeta::new(
                metadata.default_template,
            ));
        }
        metadata.pages.truncate(pdf_pages);

        Ok(Self {
            metadata,
            pdf_bytes,
            assets,
        })
    }

    /// Serializes this notebook to `.expdf` ZIP bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::Zip`] or [`NotebookError::Json`] if serialization fails.
    pub fn to_zip_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            // Store PDF deflated for compact archives while preserving byte-for-byte PDF content.
            let pdf_opts =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file(ENTRY_DOCUMENT_PDF, pdf_opts)?;
            zip.write_all(&self.pdf_bytes)?;

            let json_bytes = self.metadata.to_json_bytes()?;
            let json_opts =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file(ENTRY_NOTEBOOK_JSON, json_opts)?;
            zip.write_all(&json_bytes)?;

            for (path, data) in &self.assets {
                // Already-compressed media (e.g., .opus, .jpg, .png) store cleanly with Deflated or Stored.
                let method = if path.ends_with(".opus")
                    || path.ends_with(".jpg")
                    || path.ends_with(".jpeg")
                    || path.ends_with(".png")
                {
                    CompressionMethod::Stored
                } else {
                    CompressionMethod::Deflated
                };
                let opts = SimpleFileOptions::default().compression_method(method);
                zip.start_file(path, opts)?;
                zip.write_all(data)?;
            }
            zip.finish()?;
        }
        Ok(cursor.into_inner())
    }

    /// Saves this notebook atomically to `path` (plan K.15 / N.4 step 3): writes a sibling temp
    /// file, reads it back to verify ZIP + MuPDF integrity, flushes to disk, and renames over `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if writing, read-back verification, syncing, or renaming fails.
    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let temp = temp_path(path)?;
        let bytes = self.to_zip_bytes()?;

        let res = (|| -> Result<()> {
            fs::write(&temp, &bytes)?;
            // Read back and verify ZIP + MuPDF page count before replacing the target.
            let verified = Self::open(&temp)?;
            if verified.metadata.pages.len() != self.metadata.pages.len() {
                return Err(NotebookError::Unverified(format!(
                    "expected {} pages, verified {}",
                    self.metadata.pages.len(),
                    verified.metadata.pages.len()
                )));
            }
            OpenOptions::new().write(true).open(&temp)?.sync_all()?;
            fs::rename(&temp, path)?;
            Ok(())
        })();

        if res.is_err() {
            let _ = fs::remove_file(&temp);
        }
        res
    }

    /// Opens the embedded `document.pdf` directly in MuPDF from memory (`fz_open_document_with_stream`).
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::MuPdf`] if MuPDF fails to open the embedded PDF bytes.
    pub fn open_mupdf(&self) -> Result<Document> {
        Ok(Document::from_bytes(&self.pdf_bytes, "application/pdf")?)
    }

    /// Returns the raw embedded `document.pdf` bytes.
    #[must_use]
    pub fn pdf_bytes(&self) -> &[u8] {
        &self.pdf_bytes
    }

    /// Replaces the embedded `document.pdf` bytes (for incremental PDF updates and page operations).
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::MuPdf`] if `new_pdf` cannot be opened by MuPDF.
    pub fn replace_pdf_bytes(&mut self, new_pdf: Vec<u8>) -> Result<()> {
        let doc = Document::from_bytes(&new_pdf, "application/pdf")?;
        let count = doc.page_count()?;
        self.pdf_bytes = new_pdf;
        while self.metadata.pages.len() < count {
            self.metadata
                .pages
                .push(crate::notebook::model::PageMeta::new(
                    self.metadata.default_template,
                ));
        }
        self.metadata.pages.truncate(count);
        Ok(())
    }

    /// Inserts or replaces an asset under `assets/`.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if `asset_path` is not under `assets/` or contains `..`.
    pub fn add_asset(&mut self, asset_path: impl Into<String>, bytes: Vec<u8>) -> Result<()> {
        let path = asset_path.into().replace('\\', "/");
        if !path.starts_with(ASSETS_PREFIX) || path.contains("..") || path.ends_with('/') {
            return Err(NotebookError::InvalidFormat(format!(
                "invalid asset path: {path}"
            )));
        }
        self.assets.insert(path, bytes);
        Ok(())
    }

    /// Looks up a bundled asset by archive path.
    #[must_use]
    pub fn asset(&self, asset_path: &str) -> Option<&[u8]> {
        self.assets.get(asset_path).map(Vec::as_slice)
    }

    /// Removes a bundled asset by archive path, returning its bytes if present.
    pub fn remove_asset(&mut self, asset_path: &str) -> Option<Vec<u8>> {
        self.assets.remove(asset_path)
    }

    /// Iterates over all bundled asset paths in deterministic sorted order.
    pub fn asset_paths(&self) -> impl Iterator<Item = &str> {
        self.assets.keys().map(String::as_str)
    }

    /// Exports this notebook as a plain standalone PDF (plan N.4 step 4):
    /// - Bakes procedural paper templates as vector content streams underneath each page using
    ///   `template_stream(page_index, width_pt, height_pt, template_id)`.
    /// - Writes notebook sections as standard PDF `/Outlines` bookmarks.
    /// - Writes internal and deep links as PDF `/Link` annotations.
    /// - Excludes audio recordings by default, or embeds them via `/EmbeddedFiles` when
    ///   `options.attach_audio == AttachAudio::Embed`.
    ///
    /// # Errors
    ///
    /// Returns an error if PDF generation or MuPDF verification fails.
    pub fn export_pdf<F>(&self, options: ExportOptions, mut template_stream: F) -> Result<Vec<u8>>
    where
        F: FnMut(usize, f32, f32, PaperTemplateId) -> String,
    {
        let mut out = self.pdf_bytes.clone();
        let doc = Document::from_bytes(&out, "application/pdf")?;
        let page_count = doc.page_count()?;

        // 1. Bake procedural paper templates as vector underlays.
        for page_idx in 0..page_count {
            let bounds = doc.page_box(page_idx, excellent_mupdf::PageBox::Media)?;
            let width = bounds.x1 - bounds.x0;
            let height = bounds.y1 - bounds.y0;
            let template_id = self
                .metadata
                .pages
                .get(page_idx)
                .map_or(self.metadata.default_template, |p| p.template);
            let stream = template_stream(page_idx, width, height, template_id);
            if !stream.is_empty() {
                out = pdf_ops::prepend_page_content(&out, page_idx, &stream)?;
            }
        }

        // 2. Write sections as PDF bookmarks.
        if !self.metadata.sections.is_empty() {
            let entries: Vec<OutlineEntrySpec> = self
                .metadata
                .sections
                .iter()
                .map(|s| OutlineEntrySpec {
                    title: s.title.clone(),
                    page: s.start_page,
                    y_pt: None,
                })
                .collect();
            out = pdf_ops::set_outlines(&out, &entries)?;
        }

        // 3. Write links as PDF /Link annotations.
        if !self.metadata.links.is_empty() {
            let specs: Vec<PdfLinkSpec> = self
                .metadata
                .links
                .iter()
                .map(|l| PdfLinkSpec {
                    source_page: l.source_page,
                    rect: l.rect,
                    target: match &l.destination {
                        LinkDestination::Page { page, point, zoom } => {
                            PdfLinkTarget::InternalPage {
                                page: *page,
                                point: *point,
                                zoom: *zoom,
                            }
                        }
                        LinkDestination::Uri(uri) => PdfLinkTarget::Uri(uri.clone()),
                    },
                })
                .collect();
            out = pdf_ops::add_links(&out, &specs)?;
        }

        // 4. Optionally embed audio recordings as PDF /EmbeddedFiles attachments.
        if options.attach_audio == AttachAudio::Embed && !self.metadata.audio.is_empty() {
            let mut files: Vec<(&str, &[u8])> = Vec::new();
            for rec in &self.metadata.audio {
                if let Some(bytes) = self.assets.get(&rec.asset_path) {
                    let filename = rec.asset_path.rsplit('/').next().unwrap_or(&rec.asset_path);
                    files.push((filename, bytes.as_slice()));
                }
            }
            if !files.is_empty() {
                out = pdf_ops::attach_files(&out, &files)?;
            }
        }

        // Verify the exported PDF opens cleanly in MuPDF.
        let verified = Document::from_bytes(&out, "application/pdf")?;
        if verified.page_count()? != page_count {
            return Err(NotebookError::Unverified(
                "exported PDF page count mismatch".into(),
            ));
        }
        Ok(out)
    }
}

fn temp_path(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        NotebookError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "notebook path has no file name",
        ))
    })?;
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(".excellentpdf-saving");
    Ok(path.with_file_name(temp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::model::{AudioFormat, AudioRecordingMeta, PageLinkMeta, SectionMeta};
    use excellent_mupdf::PageRect;

    #[test]
    fn expdf_notebook_round_trips_and_exports_plain_pdf() {
        let mut nb = Notebook::new_blank(
            "Physics 101",
            PageSize::A4,
            Orientation::Portrait,
            PaperTemplateId::Cornell,
        )
        .expect("create blank");

        // Add a second page incrementally.
        let updated_pdf = pdf_ops::insert_page(nb.pdf_bytes(), 1, 595.28, 841.89, "")
            .expect("insert second page");
        assert!(updated_pdf.starts_with(nb.pdf_bytes()));
        nb.replace_pdf_bytes(updated_pdf).expect("replace pdf");

        nb.metadata.sections.push(SectionMeta {
            id: "sec-1".into(),
            title: "Mechanics".into(),
            color: [0x25, 0x63, 0xeb],
            start_page: 0,
            page_count: 1,
        });
        nb.metadata.sections.push(SectionMeta {
            id: "sec-2".into(),
            title: "Optics".into(),
            color: [0x05, 0x96, 0x69],
            start_page: 1,
            page_count: 1,
        });
        nb.metadata.links.push(PageLinkMeta {
            id: "lnk-1".into(),
            source_page: 0,
            rect: PageRect {
                x0: 40.0,
                y0: 40.0,
                x1: 180.0,
                y1: 70.0,
            },
            label: Some("Jump to Optics".into()),
            destination: LinkDestination::Page {
                page: 1,
                point: None,
                zoom: None,
            },
        });
        nb.add_asset("assets/audio/lecture1.wav", b"RIFFfakewav".to_vec())
            .expect("add audio");
        nb.metadata.audio.push(AudioRecordingMeta {
            id: "rec-1".into(),
            asset_path: "assets/audio/lecture1.wav".into(),
            format: AudioFormat::Wav,
            started_at_ms: 1000.0,
            duration_ms: 600_000.0,
            sample_rate: 48_000,
            channels: 1,
            title: Some("Lecture 1".into()),
        });

        // Atomic save and reopen from disk.
        let tmp_dir = std::env::temp_dir().join(format!("excellent-expdf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let file = tmp_dir.join("physics.expdf");
        nb.save_atomic(&file).expect("save atomic");

        let reopened = Notebook::open(&file).expect("reopen");
        assert_eq!(reopened.metadata, nb.metadata);
        assert_eq!(
            reopened.asset("assets/audio/lecture1.wav"),
            Some(b"RIFFfakewav".as_slice())
        );
        let mupdf_doc = reopened.open_mupdf().expect("open mupdf");
        assert_eq!(mupdf_doc.page_count().expect("pages"), 2);

        // Export plain PDF without audio (default).
        let exported_plain = reopened
            .export_pdf(ExportOptions::default(), |_, w, _, _| {
                format!("0.8 0.85 0.9 RG 0.5 w 36 720 m {} 720 l S", w - 36.0)
            })
            .expect("export plain");
        let plain_doc =
            Document::from_bytes(&exported_plain, "application/pdf").expect("open plain");
        assert_eq!(plain_doc.page_count().expect("pages"), 2);
        assert_eq!(plain_doc.outline().expect("outline").len(), 2);
        assert_eq!(
            plain_doc
                .load_page(0)
                .expect("p0")
                .links()
                .expect("links")
                .len(),
            1
        );
        assert!(plain_doc.attachments().expect("attachments").is_empty());

        // Export plain PDF WITH audio embedded.
        let exported_with_audio = reopened
            .export_pdf(
                ExportOptions {
                    attach_audio: AttachAudio::Embed,
                },
                |_, _, _, _| String::new(),
            )
            .expect("export with audio");
        let audio_doc =
            Document::from_bytes(&exported_with_audio, "application/pdf").expect("open audio");
        let attachments = audio_doc.attachments().expect("attachments");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "lecture1.wav");

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
