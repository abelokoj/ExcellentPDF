//! Images, stickers, and sticky notes on pages (plan N.7).
//!
//! Images and stickers (from K.21 packs) are saved as PDF Stamp annotations with image appearances
//! (`StampArt::Image`, reusing A.20) so they render at the exact same size and position in every
//! PDF viewer. Supports move, resize, rotate, and normalized crop compatible with A.13 handles.

use excellent_core::anchor::new_id;
use excellent_core::annot::{Annotation, Boxed, Shape, Style};
use excellent_core::edit::Change;
use excellent_core::notebook::{NotebookError, Result};
use excellent_mupdf::{PagePoint, PageRect, StampArt};

/// Minimum width or height in points after resizing a media item.
pub const MIN_MEDIA_SIZE_PT: f32 = 16.0;

/// Normalized `[0.0..=1.0]` crop window inside an image or sticker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    /// Left edge fraction in `0.0..=1.0`.
    pub u0: f32,
    /// Top edge fraction in `0.0..=1.0`.
    pub v0: f32,
    /// Right edge fraction in `0.0..=1.0`.
    pub u1: f32,
    /// Bottom edge fraction in `0.0..=1.0`.
    pub v1: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
        }
    }
}

impl CropRect {
    /// Constructs a validated, clamped crop rectangle ensuring a non-zero area (`>= 0.05` each axis).
    #[must_use]
    pub fn clamped(u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        let u0 = u0.clamp(0.0, 0.95);
        let v0 = v0.clamp(0.0, 0.95);
        let u1 = u1.clamp(u0 + 0.05, 1.0);
        let v1 = v1.clamp(v0 + 0.05, 1.0);
        Self { u0, v0, u1, v1 }
    }
}

/// Source kind of a page media item (N.7).
#[derive(Debug, Clone, PartialEq)]
pub enum MediaKind {
    /// Raster image imported from a file, clipboard, or camera.
    Image {
        /// Encoded PNG or JPEG bytes.
        bytes: Vec<u8>,
        /// Natural pixel width.
        natural_width: u32,
        /// Natural pixel height.
        natural_height: u32,
    },
    /// Sticker sourced from an installed pack (K.21).
    Sticker {
        /// Owning pack identifier.
        pack_id: String,
        /// Sticker identifier inside the pack.
        sticker_id: String,
        /// Encoded PNG/JPEG sticker bytes.
        bytes: Vec<u8>,
    },
    /// Collapsible sticky note placed on the page.
    StickyNote {
        /// Note body text.
        text: String,
        /// Sticky note background RGB in `0.0..=1.0`.
        color: [f32; 3],
    },
}

/// An image, sticker, or sticky note placed on a page (N.7).
#[derive(Debug, Clone, PartialEq)]
pub struct PageMedia {
    /// Stable UUID (also used as PDF annotation `/NM`).
    pub id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Bounding rectangle in top-left page points.
    pub rect: PageRect,
    /// Rotation angle in degrees `[0.0, 360.0)`.
    pub rotation_deg: f32,
    /// Normalized crop rectangle (applies to images and stickers).
    pub crop: CropRect,
    /// Media payload.
    pub kind: MediaKind,
}

impl PageMedia {
    /// Creates an image item from raw PNG or JPEG bytes, sizing it to fit within `max_side_pt`
    /// while preserving aspect ratio and keeping it inside `page_bounds`.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if `bytes` is not a valid PNG or JPEG header.
    pub fn from_image_bytes(
        page: usize,
        center: PagePoint,
        page_bounds: PageRect,
        max_side_pt: f32,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        let (natural_width, natural_height) = image_dimensions(&bytes).ok_or_else(|| {
            NotebookError::InvalidFormat("unsupported image bytes (expected PNG or JPEG)".into())
        })?;
        let rect = fitted_rect(
            center,
            page_bounds,
            natural_width as f32,
            natural_height as f32,
            max_side_pt.max(MIN_MEDIA_SIZE_PT),
        );
        Ok(Self {
            id: new_id(),
            page,
            rect,
            rotation_deg: 0.0,
            crop: CropRect::default(),
            kind: MediaKind::Image {
                bytes,
                natural_width,
                natural_height,
            },
        })
    }

    /// Creates a sticker item from a pack (K.21).
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if `bytes` is not a valid PNG or JPEG image.
    pub fn from_sticker(
        page: usize,
        center: PagePoint,
        page_bounds: PageRect,
        pack_id: impl Into<String>,
        sticker_id: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        let (w, h) = image_dimensions(&bytes).ok_or_else(|| {
            NotebookError::InvalidFormat("sticker bytes must be a valid PNG or JPEG".into())
        })?;
        let rect = fitted_rect(center, page_bounds, w as f32, h as f32, 96.0);
        Ok(Self {
            id: new_id(),
            page,
            rect,
            rotation_deg: 0.0,
            crop: CropRect::default(),
            kind: MediaKind::Sticker {
                pack_id: pack_id.into(),
                sticker_id: sticker_id.into(),
                bytes,
            },
        })
    }

    /// Creates a sticky note placed on the page.
    #[must_use]
    pub fn sticky_note(
        page: usize,
        at: PagePoint,
        page_bounds: PageRect,
        text: impl Into<String>,
        color: [f32; 3],
    ) -> Self {
        let size = 24.0;
        let x0 =
            at.x.clamp(page_bounds.x0, (page_bounds.x1 - size).max(page_bounds.x0));
        let y0 =
            at.y.clamp(page_bounds.y0, (page_bounds.y1 - size).max(page_bounds.y0));
        Self {
            id: new_id(),
            page,
            rect: PageRect {
                x0,
                y0,
                x1: x0 + size,
                y1: y0 + size,
            },
            rotation_deg: 0.0,
            crop: CropRect::default(),
            kind: MediaKind::StickyNote {
                text: text.into(),
                color,
            },
        }
    }

    /// Moves this media item by `(dx, dy)` page points (A.13 move grip).
    pub fn move_by(&mut self, dx: f32, dy: f32) {
        self.rect = PageRect {
            x0: self.rect.x0 + dx,
            y0: self.rect.y0 + dy,
            x1: self.rect.x1 + dx,
            y1: self.rect.y1 + dy,
        };
    }

    /// Resizes from a corner index `0..4` (clockwise from top-left, matching A.13 `Grip::Corner`)
    /// keeping the opposite corner fixed and enforcing [`MIN_MEDIA_SIZE_PT`]. Images and stickers
    /// preserve their natural aspect ratio to match PDF stamp appearance scaling in MuPDF/Acrobat.
    pub fn resize_corner(&mut self, corner: usize, to: PagePoint) {
        let r = self.rect;
        let raw = match corner {
            0 => PageRect {
                x0: to.x.min(r.x1 - MIN_MEDIA_SIZE_PT),
                y0: to.y.min(r.y1 - MIN_MEDIA_SIZE_PT),
                x1: r.x1,
                y1: r.y1,
            },
            1 => PageRect {
                x0: r.x0,
                y0: to.y.min(r.y1 - MIN_MEDIA_SIZE_PT),
                x1: to.x.max(r.x0 + MIN_MEDIA_SIZE_PT),
                y1: r.y1,
            },
            2 => PageRect {
                x0: r.x0,
                y0: r.y0,
                x1: to.x.max(r.x0 + MIN_MEDIA_SIZE_PT),
                y1: to.y.max(r.y0 + MIN_MEDIA_SIZE_PT),
            },
            3 => PageRect {
                x0: to.x.min(r.x1 - MIN_MEDIA_SIZE_PT),
                y0: r.y0,
                x1: r.x1,
                y1: to.y.max(r.y0 + MIN_MEDIA_SIZE_PT),
            },
            _ => r,
        };
        let aspect = match &self.kind {
            MediaKind::Image {
                natural_width,
                natural_height,
                ..
            } if *natural_height > 0 => Some(*natural_width as f32 / *natural_height as f32),
            MediaKind::Sticker { bytes, .. } => {
                image_dimensions(bytes).map(|(w, h)| w as f32 / h.max(1) as f32)
            }
            _ => None,
        };
        self.rect = if let Some(ratio) = aspect {
            let w = (raw.x1 - raw.x0).max(MIN_MEDIA_SIZE_PT);
            let h = (w / ratio).max(MIN_MEDIA_SIZE_PT);
            match corner {
                0 => PageRect {
                    x0: raw.x1 - w,
                    y0: raw.y1 - h,
                    x1: raw.x1,
                    y1: raw.y1,
                },
                1 => PageRect {
                    x0: raw.x0,
                    y0: raw.y1 - h,
                    x1: raw.x0 + w,
                    y1: raw.y1,
                },
                2 => PageRect {
                    x0: raw.x0,
                    y0: raw.y0,
                    x1: raw.x0 + w,
                    y1: raw.y0 + h,
                },
                3 => PageRect {
                    x0: raw.x1 - w,
                    y0: raw.y0,
                    x1: raw.x1,
                    y1: raw.y0 + h,
                },
                _ => raw,
            }
        } else {
            raw
        };
    }

    /// Rotates by `delta_deg` degrees, normalizing to `[0.0, 360.0)`.
    pub fn rotate_by(&mut self, delta_deg: f32) {
        self.rotation_deg = (self.rotation_deg + delta_deg).rem_euclid(360.0);
    }

    /// Updates the normalized crop window.
    pub fn set_crop(&mut self, crop: CropRect) {
        self.crop = CropRect::clamped(crop.u0, crop.v0, crop.u1, crop.v1);
    }

    /// Emits undoable [`Change`]s that write this item into a PDF as a standard Stamp (with image
    /// appearance) or Text sticky note annotation so every PDF viewer renders it at the exact same
    /// size and place.
    #[must_use]
    pub fn to_pdf_changes(&self, author: &str) -> Vec<Change> {
        match &self.kind {
            MediaKind::Image { bytes, .. } | MediaKind::Sticker { bytes, .. } => {
                let mut stamp = Annotation::new(
                    self.page,
                    Shape::Boxed(Boxed::Stamp, self.rect),
                    Style {
                        color: None,
                        opacity: 1.0,
                        width: 0.0,
                    },
                    author,
                );
                stamp.id = self.id.clone();
                stamp.icon = format!("CustomMedia{}", self.id.replace('-', ""));
                vec![
                    Change::CreateAnnotation(stamp.clone()),
                    Change::StampArt {
                        page: self.page,
                        id: self.id.clone(),
                        art: StampArt::Image(bytes.clone()),
                    },
                    // Restore exact rect after MuPDF attaches the image appearance (otherwise
                    // `CreateAnnotation` fits the unstyled stamp to MuPDF's default 190x50 ratio).
                    Change::UpdateAnnotation(stamp),
                ]
            }
            MediaKind::StickyNote { text, color } => {
                let mut note = Annotation::new(
                    self.page,
                    Shape::Boxed(Boxed::Note, self.rect),
                    Style {
                        color: Some(*color),
                        opacity: 1.0,
                        width: 0.0,
                    },
                    author,
                );
                note.id = self.id.clone();
                note.contents = text.clone();
                note.icon = "Note".into();
                vec![Change::CreateAnnotation(note)]
            }
        }
    }
}

fn fitted_rect(
    center: PagePoint,
    bounds: PageRect,
    natural_w: f32,
    natural_h: f32,
    max_side: f32,
) -> PageRect {
    let nw = natural_w.max(1.0);
    let nh = natural_h.max(1.0);
    let max_w = max_side.min(bounds.x1 - bounds.x0).max(MIN_MEDIA_SIZE_PT);
    let max_h = max_side.min(bounds.y1 - bounds.y0).max(MIN_MEDIA_SIZE_PT);
    let scale = (max_w / nw).min(max_h / nh).min(1.0);
    let w = (nw * scale).max(MIN_MEDIA_SIZE_PT);
    let h = (nh * scale).max(MIN_MEDIA_SIZE_PT);
    let x0 = (center.x - w * 0.5).clamp(bounds.x0, (bounds.x1 - w).max(bounds.x0));
    let y0 = (center.y - h * 0.5).clamp(bounds.y0, (bounds.y1 - h).max(bounds.y0));
    PageRect {
        x0,
        y0,
        x1: x0 + w,
        y1: y0 + h,
    }
}

/// Parses natural `(width, height)` from a PNG or JPEG header without decoding pixels.
#[must_use]
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // PNG signature + IHDR chunk.
    if bytes.len() >= 24 && bytes[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    // JPEG SOI + SOF marker scan.
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2usize;
        while i + 4 <= bytes.len() {
            if bytes[i] != 0xFF {
                break;
            }
            while i < bytes.len() && bytes[i] == 0xFF {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let marker = bytes[i];
            i += 1;
            if matches!(marker, 0xD9 | 0xDA) {
                break;
            }
            if i + 2 > bytes.len() {
                break;
            }
            let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
            if seg_len < 2 || i + seg_len > bytes.len() {
                break;
            }
            // SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15
            if matches!(
                marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
            ) && seg_len >= 7
            {
                let h = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
            i += seg_len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::notebook::pdf_ops::build_blank_pdf;
    use excellent_mupdf::{Background, Document, Matrix};

    const PAGE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    fn sample_png() -> Vec<u8> {
        // Generate a real 32x16 colored PNG via MuPDF so MuPDF can embed and render it.
        let pdf = excellent_mupdf::html_to_pdf(
            "<div style=\"width:32pt;height:16pt;background:#2563eb\"></div>",
            "body{margin:0}",
            32.0,
        )
        .expect("html pdf");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("doc");
        let pix = doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("pix");
        pix.to_png().expect("png")
    }

    #[test]
    fn images_and_stickers_save_as_pdf_stamps_at_exact_size_and_place() {
        let png = sample_png();
        let (w, h) = image_dimensions(&png).expect("png dims");
        assert!(w > 0 && h > 0);

        let mut media =
            PageMedia::from_image_bytes(0, PagePoint { x: 200.0, y: 180.0 }, PAGE, 120.0, png)
                .expect("media");
        media.move_by(20.0, 10.0);
        media.resize_corner(2, PagePoint { x: 340.0, y: 260.0 });
        media.rotate_by(90.0);
        media.set_crop(CropRect::clamped(0.1, 0.1, 0.9, 0.9));
        assert_eq!(media.rotation_deg, 90.0);

        let target_rect = media.rect;
        let changes = media.to_pdf_changes("Ada");
        assert_eq!(changes.len(), 3);

        let pdf = build_blank_pdf(&[(612.0, 792.0)]).expect("blank");
        let mut doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        doc.operation("Place image", |edit| {
            for change in &changes {
                match change {
                    Change::CreateAnnotation(a) => edit.create_annot(a.page, &a.to_data())?,
                    Change::UpdateAnnotation(a) => edit.update_annot(a.page, &a.to_data())?,
                    Change::StampArt { page, id, art } => edit.set_stamp_art(*page, id, art)?,
                    _ => {}
                }
            }
            Ok(())
        })
        .expect("apply stamp");

        let page0 = doc.load_page(0).expect("p0");
        let annots = page0.annots().expect("annots");
        assert_eq!(annots.len(), 1);
        let placed = annots[0].rect().expect("rect");
        assert!(
            (placed.x0 - target_rect.x0).abs() < 0.5,
            "{placed:?} vs {target_rect:?}"
        );
        assert!(
            (placed.y0 - target_rect.y0).abs() < 0.5,
            "{placed:?} vs {target_rect:?}"
        );
        assert!(
            (placed.x1 - target_rect.x1).abs() < 0.5,
            "{placed:?} vs {target_rect:?}"
        );
        assert!(
            (placed.y1 - target_rect.y1).abs() < 0.5,
            "{placed:?} vs {target_rect:?}"
        );
    }
}
