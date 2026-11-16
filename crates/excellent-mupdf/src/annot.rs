//! PDF annotations.

use core::ffi::c_int;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::ffi::CString;

use excellent_mupdf_sys::{
    self as sys, PDF_ANNOT_UNKNOWN, ex_annot_arrays, ex_annot_contents, ex_annot_info,
    ex_annot_props, ex_annot_props_of, ex_annot_text, ex_drop_annot, ex_first_annot, ex_next_annot,
    fz_point, fz_quad, fz_rect, pdf_annot,
};

use crate::buffer;
use crate::context::{check, drop_with_context, with_context};
use crate::error::{MuError, Result};
use crate::geometry::{PagePoint, PageRect, Quad};
use crate::page::Page;

/// The shim's annotation texts (`EX_ANNOT_*` in `shim.h`).
const ID: c_int = 1;
const AUTHOR: c_int = 2;
const APP_DATA: c_int = 3;
const GROUP: c_int = 4;
const ICON: c_int = 5;
const FONT: c_int = 6;

/// How a text box is drawn (PDF's `/IT` on a free-text annotation, A.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Intent {
    /// A box of text: the default.
    #[default]
    Plain,
    /// A box with a leader line to what it is about (A.7).
    Callout,
    /// Text typed onto the page, as on a form that has no fields.
    Typewriter,
}

impl Intent {
    /// MuPDF's `enum pdf_intent` value.
    fn raw(self) -> c_int {
        match self {
            Self::Plain => 0,
            Self::Callout => 1,
            Self::Typewriter => 2,
        }
    }

    /// The intent MuPDF names `raw`; any that isn't a text box's reads as plain.
    fn from_raw(raw: c_int) -> Self {
        match raw {
            1 => Self::Callout,
            2 => Self::Typewriter,
            _ => Self::Plain,
        }
    }
}

/// How a text box's lines are aligned (PDF's `/Q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Left: the default.
    #[default]
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
}

impl Align {
    fn raw(self) -> c_int {
        match self {
            Self::Left => 0,
            Self::Center => 1,
            Self::Right => 2,
        }
    }

    fn from_raw(raw: c_int) -> Self {
        match raw {
            1 => Self::Center,
            2 => Self::Right,
            _ => Self::Left,
        }
    }
}

/// The subtype of a PDF annotation (PDF 2.0, table 171).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotKind {
    /// A note icon (sticky note).
    Text,
    /// A link.
    Link,
    /// Text drawn on the page.
    FreeText,
    /// A straight line.
    Line,
    /// A rectangle.
    Square,
    /// An ellipse.
    Circle,
    /// A closed polygon.
    Polygon,
    /// An open polyline.
    PolyLine,
    /// Text highlight.
    Highlight,
    /// Text underline.
    Underline,
    /// Squiggly underline.
    Squiggly,
    /// Strikeout.
    StrikeOut,
    /// Area marked for redaction.
    Redact,
    /// A rubber stamp.
    Stamp,
    /// An insertion caret.
    Caret,
    /// Freehand ink.
    Ink,
    /// The pop-up window of another annotation.
    Popup,
    /// An attached file.
    FileAttachment,
    /// A sound.
    Sound,
    /// A movie (deprecated in PDF 2.0).
    Movie,
    /// Rich media (video, 3D).
    RichMedia,
    /// A form field.
    Widget,
    /// A screen region for media.
    Screen,
    /// A printer's mark.
    PrinterMark,
    /// A trap network.
    TrapNet,
    /// A watermark.
    Watermark,
    /// 3D artwork.
    ThreeD,
    /// A projection (PDF 2.0).
    Projection,
    /// A subtype MuPDF doesn't know.
    Unknown,
}

impl AnnotKind {
    /// MuPDF's `enum pdf_annot_type` for it; `None` for [`AnnotKind::Unknown`].
    pub(crate) fn raw(self) -> Option<c_int> {
        Some(match self {
            Self::Text => sys::PDF_ANNOT_TEXT,
            Self::Link => sys::PDF_ANNOT_LINK,
            Self::FreeText => sys::PDF_ANNOT_FREE_TEXT,
            Self::Line => sys::PDF_ANNOT_LINE,
            Self::Square => sys::PDF_ANNOT_SQUARE,
            Self::Circle => sys::PDF_ANNOT_CIRCLE,
            Self::Polygon => sys::PDF_ANNOT_POLYGON,
            Self::PolyLine => sys::PDF_ANNOT_POLY_LINE,
            Self::Highlight => sys::PDF_ANNOT_HIGHLIGHT,
            Self::Underline => sys::PDF_ANNOT_UNDERLINE,
            Self::Squiggly => sys::PDF_ANNOT_SQUIGGLY,
            Self::StrikeOut => sys::PDF_ANNOT_STRIKE_OUT,
            Self::Redact => sys::PDF_ANNOT_REDACT,
            Self::Stamp => sys::PDF_ANNOT_STAMP,
            Self::Caret => sys::PDF_ANNOT_CARET,
            Self::Ink => sys::PDF_ANNOT_INK,
            Self::Popup => sys::PDF_ANNOT_POPUP,
            Self::FileAttachment => sys::PDF_ANNOT_FILE_ATTACHMENT,
            Self::Sound => sys::PDF_ANNOT_SOUND,
            Self::Movie => sys::PDF_ANNOT_MOVIE,
            Self::RichMedia => sys::PDF_ANNOT_RICH_MEDIA,
            Self::Widget => sys::PDF_ANNOT_WIDGET,
            Self::Screen => sys::PDF_ANNOT_SCREEN,
            Self::PrinterMark => sys::PDF_ANNOT_PRINTER_MARK,
            Self::TrapNet => sys::PDF_ANNOT_TRAP_NET,
            Self::Watermark => sys::PDF_ANNOT_WATERMARK,
            Self::ThreeD => sys::PDF_ANNOT_3D,
            Self::Projection => sys::PDF_ANNOT_PROJECTION,
            Self::Unknown => return None,
        })
    }

    fn from_raw(raw: c_int) -> Self {
        match raw {
            sys::PDF_ANNOT_TEXT => Self::Text,
            sys::PDF_ANNOT_LINK => Self::Link,
            sys::PDF_ANNOT_FREE_TEXT => Self::FreeText,
            sys::PDF_ANNOT_LINE => Self::Line,
            sys::PDF_ANNOT_SQUARE => Self::Square,
            sys::PDF_ANNOT_CIRCLE => Self::Circle,
            sys::PDF_ANNOT_POLYGON => Self::Polygon,
            sys::PDF_ANNOT_POLY_LINE => Self::PolyLine,
            sys::PDF_ANNOT_HIGHLIGHT => Self::Highlight,
            sys::PDF_ANNOT_UNDERLINE => Self::Underline,
            sys::PDF_ANNOT_SQUIGGLY => Self::Squiggly,
            sys::PDF_ANNOT_STRIKE_OUT => Self::StrikeOut,
            sys::PDF_ANNOT_REDACT => Self::Redact,
            sys::PDF_ANNOT_STAMP => Self::Stamp,
            sys::PDF_ANNOT_CARET => Self::Caret,
            sys::PDF_ANNOT_INK => Self::Ink,
            sys::PDF_ANNOT_POPUP => Self::Popup,
            sys::PDF_ANNOT_FILE_ATTACHMENT => Self::FileAttachment,
            sys::PDF_ANNOT_SOUND => Self::Sound,
            sys::PDF_ANNOT_MOVIE => Self::Movie,
            sys::PDF_ANNOT_RICH_MEDIA => Self::RichMedia,
            sys::PDF_ANNOT_WIDGET => Self::Widget,
            sys::PDF_ANNOT_SCREEN => Self::Screen,
            sys::PDF_ANNOT_PRINTER_MARK => Self::PrinterMark,
            sys::PDF_ANNOT_TRAP_NET => Self::TrapNet,
            sys::PDF_ANNOT_WATERMARK => Self::Watermark,
            sys::PDF_ANNOT_3D => Self::ThreeD,
            sys::PDF_ANNOT_PROJECTION => Self::Projection,
            _ => Self::Unknown,
        }
    }
}

/// An annotation on a PDF page, which it borrows. Not `Send`.
#[derive(Debug)]
pub struct Annot<'page> {
    raw: NonNull<pdf_annot>,
    _page: PhantomData<&'page Page<'page>>,
}

impl Annot<'_> {
    fn info(&self) -> Result<(AnnotKind, PageRect)> {
        let (mut kind, mut rect) = (PDF_ANNOT_UNKNOWN, fz_rect::default());
        with_context(|ctx| {
            // SAFETY: the annotation is live and on its page's thread.
            check(|err| unsafe { ex_annot_info(ctx, self.raw.as_ptr(), &mut kind, &mut rect, err) })
        })?;
        Ok((AnnotKind::from_raw(kind), rect.into()))
    }

    /// The annotation's subtype.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn kind(&self) -> Result<AnnotKind> {
        Ok(self.info()?.0)
    }

    /// Where the annotation sits on the page, in points.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn rect(&self) -> Result<PageRect> {
        Ok(self.info()?.1)
    }

    /// The annotation's text (`/Contents`), empty when it has none.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn contents(&self) -> Result<String> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the annotation is live and on its page's thread.
            check(|err| unsafe { ex_annot_contents(ctx, self.raw.as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { buffer::into_string(ctx, buf) })
        })
    }

    /// The annotation's id (`/NM`), empty when it has none.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn id(&self) -> Result<String> {
        self.text(ID)
    }

    /// Everything the annotation model (K.14) keeps of the annotation.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn data(&self) -> Result<AnnotData> {
        let mut props = ex_annot_props::default();
        with_context(|ctx| {
            // SAFETY: the annotation is live and on its page's thread; `props` is ours.
            check(|err| unsafe { ex_annot_props_of(ctx, self.raw.as_ptr(), &mut props, err) })
        })?;
        let count = |n: c_int| usize::try_from(n).unwrap_or(0);
        let mut quads = vec![fz_quad::default(); count(props.quad_count)];
        let mut lengths: Vec<c_int> = vec![0; count(props.stroke_count)];
        let mut points = vec![fz_point::default(); count(props.point_count)];
        with_context(|ctx| {
            // SAFETY: as above; each array holds as many items as the props counted, which is what the shim writes.
            check(|err| unsafe {
                ex_annot_arrays(
                    ctx,
                    self.raw.as_ptr(),
                    quads.as_mut_ptr(),
                    lengths.as_mut_ptr(),
                    points.as_mut_ptr(),
                    err,
                )
            })
        })?;
        let points: Vec<PagePoint> = points.into_iter().map(PagePoint::from).collect();
        let kind = AnnotKind::from_raw(props.type_);
        let (strokes, points) = if kind == AnnotKind::Ink {
            let mut rest = points.as_slice();
            let strokes = lengths
                .iter()
                .map(|&length| {
                    let (stroke, after) = rest.split_at(count(length).min(rest.len()));
                    rest = after;
                    stroke.to_vec()
                })
                .collect();
            (strokes, Vec::new())
        } else {
            (Vec::new(), points)
        };
        let color_n = count(props.color_n).min(props.color.len());
        Ok(AnnotData {
            kind,
            id: self.id()?,
            author: self.text(AUTHOR)?,
            contents: self.contents()?,
            app_data: self.text(APP_DATA)?,
            group: self.text(GROUP)?,
            icon: self.text(ICON)?,
            open: props.is_open != 0,
            intent: Intent::from_raw(props.intent),
            align: Align::from_raw(props.quadding),
            font: self.text(FONT)?,
            font_size: props.font_size,
            text_color: props.text_color[..count(props.text_color_n).min(4)].to_vec(),
            rect: props.rect.into(),
            color: props.color[..color_n].to_vec(),
            opacity: props.opacity,
            border: props.border,
            created: props.created,
            modified: props.modified,
            quads: quads.into_iter().map(Quad::from).collect(),
            strokes,
            points,
        })
    }

    fn text(&self, which: c_int) -> Result<String> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the annotation is live and on its page's thread.
            check(|err| unsafe { ex_annot_text(ctx, self.raw.as_ptr(), which, &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { buffer::into_string(ctx, buf) })
        })
    }
}

/// An annotation as the annotation model (K.14) reads and writes it, in page space (points, y down). The arrays
/// that don't apply to its kind are empty.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotData {
    /// Its subtype.
    pub kind: AnnotKind,
    /// Its id (`/NM`), which names it in edits.
    pub id: String,
    /// Who made it (`/T`); markup annotations only.
    pub author: String,
    /// Its text (`/Contents`).
    pub contents: String,
    /// The app's private key (its anchor and tags, K.28) as text; empty for none.
    pub app_data: String,
    /// The id of the annotation on its page it is grouped with (`/IRT` with `/RT /Group`: a "replace text" pair,
    /// A.2); empty for none.
    pub group: String,
    /// Its icon's name (`/Name`: a sticky note's or a stamp's, A.5), as the file has it; empty for none, which
    /// readers show as the kind's default.
    pub icon: String,
    /// Whether its note shows open in other readers (`/Open`: a sticky note's own, else its popup's).
    pub open: bool,
    /// A text box's kind (`/IT`, A.6); plain for the other kinds.
    pub intent: Intent,
    /// A text box's alignment (`/Q`).
    pub align: Align,
    /// A text box's font (`/DA`), MuPDF's name for a standard one: `Helv`, `TiRo` or `Cour`; empty for other kinds.
    pub font: String,
    /// A text box's text size in points; 0 for other kinds.
    pub font_size: f32,
    /// A text box's text colour, as `color`.
    pub text_color: Vec<f32>,
    /// Where it is: written for the kinds placed by a rectangle (notes, text boxes, shapes, stamps); read for all,
    /// as the area it covers.
    pub rect: PageRect,
    /// Its colour: no components (none), 1 (gray), 3 (RGB) or 4 (CMYK), each 0 to 1.
    pub color: Vec<f32>,
    /// 0 (clear) to 1.
    pub opacity: f32,
    /// Its border or stroke width in points, for the kinds that have one.
    pub border: f32,
    /// When it was made, in seconds since 1970 (UTC); markup annotations only.
    pub created: i64,
    /// When it last changed, as `created`.
    pub modified: i64,
    /// The text it marks (highlight, underline, squiggly, strikeout).
    pub quads: Vec<Quad>,
    /// Ink strokes.
    pub strokes: Vec<Vec<PagePoint>>,
    /// A line's two ends, or a polygon's or polyline's vertices.
    pub points: Vec<PagePoint>,
}

impl AnnotData {
    /// Runs `write` with the shim's view of the annotation, which borrows from it.
    pub(crate) fn with_raw<T>(
        &self,
        write: impl FnOnce(&sys::ex_annot_write) -> Result<T>,
    ) -> Result<T> {
        let kind = self
            .kind
            .raw()
            .ok_or_else(|| MuError::argument("an annotation of unknown kind can't be written"))?;
        let text = |s: &str| {
            CString::new(s).map_err(|_| MuError::argument("annotation text contains a NUL"))
        };
        let (id, author, contents, app_data, group, icon, font) = (
            text(&self.id)?,
            text(&self.author)?,
            text(&self.contents)?,
            text(&self.app_data)?,
            text(&self.group)?,
            text(&self.icon)?,
            text(&self.font)?,
        );
        let count = |n: usize| c_int::try_from(n).map_err(|_| MuError::argument("too many points"));
        if self.color.len() > 4 || self.text_color.len() > 4 {
            return Err(MuError::argument("a colour has at most four components"));
        }
        let mut color = [0.0; 4];
        color[..self.color.len()].copy_from_slice(&self.color);
        let mut text_color = [0.0; 4];
        text_color[..self.text_color.len()].copy_from_slice(&self.text_color);
        let quads: Vec<fz_quad> = self.quads.iter().copied().map(fz_quad::from).collect();
        let lengths = self
            .strokes
            .iter()
            .map(|stroke| count(stroke.len()))
            .collect::<Result<Vec<c_int>>>()?;
        let points: Vec<fz_point> = self
            .strokes
            .iter()
            .flatten()
            .chain(&self.points)
            .copied()
            .map(fz_point::from)
            .collect();
        let raw = sys::ex_annot_write {
            id: id.as_ptr(),
            author: author.as_ptr(),
            contents: contents.as_ptr(),
            app_data: app_data.as_ptr(),
            group: group.as_ptr(),
            icon: icon.as_ptr(),
            font: font.as_ptr(),
            props: ex_annot_props {
                type_: kind,
                rect: self.rect.into(),
                color_n: count(self.color.len())?,
                color,
                opacity: self.opacity,
                border: self.border,
                created: self.created,
                modified: self.modified,
                quad_count: count(quads.len())?,
                stroke_count: count(lengths.len())?,
                point_count: count(points.len())?,
                is_open: c_int::from(self.open),
                intent: self.intent.raw(),
                quadding: self.align.raw(),
                font_size: self.font_size,
                text_color_n: count(self.text_color.len())?,
                text_color,
            },
            quads: quads.as_ptr(),
            stroke_lengths: lengths.as_ptr(),
            points: points.as_ptr(),
        };
        write(&raw)
    }
}

impl Drop for Annot<'_> {
    fn drop(&mut self) {
        // SAFETY: we own this reference (the shim kept it).
        drop_with_context(|ctx| unsafe { ex_drop_annot(ctx, self.raw.as_ptr()) });
    }
}

/// A page's annotations, in order.
pub(crate) fn list<'page>(page: &'page Page<'_>) -> Result<Vec<Annot<'page>>> {
    let mut annots = Vec::new();
    with_context(|ctx| {
        let mut raw = ptr::null_mut();
        // SAFETY: the page is live and on its own thread.
        check(|err| unsafe { ex_first_annot(ctx, page.raw().as_ptr(), &mut raw, err) })?;
        while let Some(current) = NonNull::new(raw) {
            // Owned from here, so it is dropped even if the next step fails.
            annots.push(Annot {
                raw: current,
                _page: PhantomData,
            });
            raw = ptr::null_mut();
            // SAFETY: `current` is a live annotation of this page.
            check(|err| unsafe { ex_next_annot(ctx, current.as_ptr(), &mut raw, err) })?;
        }
        Ok(())
    })?;
    Ok(annots)
}
