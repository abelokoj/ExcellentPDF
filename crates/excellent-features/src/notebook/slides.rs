//! Import slides and condense 1, 2, or 3 slides per note page (plan L.3).
//!
//! Uses pure vector PDF Form XObjects ([`pdf_ops::condense_pages_vector`]) so slide graphics stay
//! sharp at any zoom level and every word on the source slides stays selectable and searchable in
//! MuPDF and all other PDF readers.

use std::fmt::Write as _;

use excellent_core::notebook::{
    Notebook, NotebookError, Orientation, PageSize, PaperTemplateId, Result, SlideSlot, pdf_ops,
};
use excellent_mupdf::Document;

/// Number of source slides placed onto each condensed note page (L.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SlidesPerPage {
    /// One large slide on the top half with note lines below.
    One,
    /// Two slides stacked in the left column with note space on the right.
    Two,
    /// Three slides stacked in the left column with note space on the right (classic handout).
    #[default]
    Three,
}

impl SlidesPerPage {
    /// Numeric slide count per output page.
    #[must_use]
    pub fn count(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }
}

/// Ruling style drawn in the note-taking space beside condensed slides (L.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CondenseNoteStyle {
    /// Horizontal note lines beside each slide.
    #[default]
    Lined,
    /// Cornell-style layout with slide column as cues, lined notes on right, and summary band at bottom.
    Cornell,
    /// Subtle square grid in the note area.
    Grid,
    /// Unruled blank note area.
    Blank,
}

/// Whether slide numbers are printed beside each condensed slide slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SlideNumbers {
    /// Print slide labels (`Slide 1`, `Slide 2`, ...).
    #[default]
    Show,
    /// Omit slide number labels.
    Hide,
}

/// Configuration for condensing a slide PDF into note pages (plan L.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CondenseOptions {
    /// 1, 2, or 3 slides per note page.
    pub slides_per_page: SlidesPerPage,
    /// Note space ruling style (Lined, Cornell, Grid, Blank).
    pub note_style: CondenseNoteStyle,
    /// Output note page paper size.
    pub output_page_size: PageSize,
    /// Output note page orientation.
    pub output_orientation: Orientation,
    /// Outer page margin in points.
    pub margin_pt: f32,
    /// Horizontal fraction `[0.35..=0.65]` reserved for the slide column in 2-up/3-up layouts.
    pub slide_column_ratio: f32,
    /// Spacing between ruled note lines in points.
    pub line_spacing_pt: f32,
    /// Whether to print slide numbers above each slide frame.
    pub slide_numbers: SlideNumbers,
}

impl Default for CondenseOptions {
    fn default() -> Self {
        Self {
            slides_per_page: SlidesPerPage::Three,
            note_style: CondenseNoteStyle::Lined,
            output_page_size: PageSize::Letter,
            output_orientation: Orientation::Portrait,
            margin_pt: 36.0,
            slide_column_ratio: 0.48,
            line_spacing_pt: 20.0,
            slide_numbers: SlideNumbers::Show,
        }
    }
}

/// Computed layout for one condensed output page (slide rectangles + note ruling underlay).
#[derive(Debug, Clone, PartialEq)]
pub struct CondensedPagePlan {
    /// Output page width in points.
    pub width_pt: f32,
    /// Output page height in points.
    pub height_pt: f32,
    /// Slide placement slots in PDF bottom-left coordinates.
    pub slots: Vec<SlideSlot>,
    /// Vector PDF content stream drawing the note lines / Cornell dividers / slide labels.
    pub note_underlay: String,
}

/// Computes the exact condensed page layouts for `source_slide_count` slides.
#[must_use]
pub fn plan_condensed_pages(
    source_slide_count: usize,
    options: &CondenseOptions,
) -> Vec<CondensedPagePlan> {
    let per_page = options.slides_per_page.count();
    let (w, h) = options
        .output_page_size
        .dimensions(options.output_orientation);
    let m = options.margin_pt.clamp(12.0, (w.min(h) * 0.2).max(12.0));
    let summary_band = if options.note_style == CondenseNoteStyle::Cornell {
        96.0_f32.min(h * 0.16)
    } else {
        0.0
    };
    let line_step = options.line_spacing_pt.max(10.0);

    let mut plans = Vec::new();
    let mut slide_idx = 0usize;
    while slide_idx < source_slide_count {
        let end = (slide_idx + per_page).min(source_slide_count);
        let mut slots = Vec::with_capacity(end - slide_idx);
        let mut underlay = String::new();

        let body_bottom = m + summary_band;
        let body_top = h - m;
        let body_h = (body_top - body_bottom).max(72.0);
        let body_w = (w - 2.0 * m).max(72.0);

        match options.slides_per_page {
            SlidesPerPage::One => {
                // Top 52% for the single slide, bottom 48% for notes.
                let gap = 18.0;
                let slide_h = (body_h - gap) * 0.52;
                let slide_y0 = body_top - slide_h;
                let slide_rect = [m, slide_y0, w - m, body_top];
                slots.push(SlideSlot {
                    source_page: slide_idx,
                    target_pdf_rect: slide_rect,
                });
                if options.slide_numbers == SlideNumbers::Show {
                    emit_slide_label(&mut underlay, slide_idx + 1, m, body_top + 4.0);
                }
                let notes_top = slide_y0 - gap;
                emit_note_area(
                    &mut underlay,
                    [m, body_bottom, w - m, notes_top],
                    options.note_style,
                    line_step,
                );
            }
            SlidesPerPage::Two | SlidesPerPage::Three => {
                let col_gap = 18.0;
                let row_gap = 16.0;
                let ratio = options.slide_column_ratio.clamp(0.35, 0.65);
                let slide_w = (body_w - col_gap) * ratio;
                let notes_x0 = m + slide_w + col_gap;
                let notes_x1 = w - m;

                let slot_h = (body_h - row_gap * (per_page - 1) as f32) / per_page as f32;

                // Cornell vertical divider between slides (cue column) and notes column.
                if options.note_style == CondenseNoteStyle::Cornell {
                    let divider_x = m + slide_w + col_gap * 0.5;
                    let _ = writeln!(
                        underlay,
                        "0.42 0.52 0.68 RG 1.0 w {divider_x:.2} {body_bottom:.2} m {divider_x:.2} {body_top:.2} l S"
                    );
                }

                for row in 0..(end - slide_idx) {
                    let src = slide_idx + row;
                    let top = body_top - row as f32 * (slot_h + row_gap);
                    let bottom = top - slot_h;
                    let label_reserve = if options.slide_numbers == SlideNumbers::Show {
                        12.0
                    } else {
                        0.0
                    };
                    let slide_rect = [
                        m,
                        bottom,
                        m + slide_w,
                        (top - label_reserve).max(bottom + 20.0),
                    ];
                    slots.push(SlideSlot {
                        source_page: src,
                        target_pdf_rect: slide_rect,
                    });
                    if options.slide_numbers == SlideNumbers::Show {
                        emit_slide_label(&mut underlay, src + 1, m, top - 9.0);
                    }
                    emit_note_area(
                        &mut underlay,
                        [notes_x0, bottom, notes_x1, top],
                        options.note_style,
                        line_step,
                    );
                }
            }
        }

        // Cornell summary band at the bottom of the page.
        if options.note_style == CondenseNoteStyle::Cornell && summary_band > 20.0 {
            let sum_top = m + summary_band - 8.0;
            let _ = writeln!(
                underlay,
                "0.42 0.52 0.68 RG 1.0 w {m:.2} {sum_top:.2} m {:.2} {sum_top:.2} l S",
                w - m
            );
            emit_note_area(
                &mut underlay,
                [m, m, w - m, sum_top - 4.0],
                CondenseNoteStyle::Lined,
                line_step,
            );
        }

        plans.push(CondensedPagePlan {
            width_pt: w,
            height_pt: h,
            slots,
            note_underlay: underlay,
        });
        slide_idx = end;
    }

    plans
}

/// Imports a slide PDF and condenses it 1-, 2-, or 3-up onto vector note pages keeping all slide
/// text selectable and searchable (plan L.3).
///
/// # Errors
///
/// Returns an error if `slide_pdf_bytes` cannot be opened, has zero pages, or condensation fails.
pub fn condense_slides_pdf(slide_pdf_bytes: &[u8], options: &CondenseOptions) -> Result<Vec<u8>> {
    let doc = Document::from_bytes(slide_pdf_bytes, "application/pdf")?;
    let slide_count = doc.page_count()?;
    if slide_count == 0 {
        return Err(NotebookError::InvalidFormat(
            "slide PDF has zero pages".into(),
        ));
    }

    let plans = plan_condensed_pages(slide_count, options);
    let specs: Vec<(f32, f32, String, Vec<SlideSlot>)> = plans
        .into_iter()
        .map(|p| (p.width_pt, p.height_pt, p.note_underlay, p.slots))
        .collect();

    let condensed = pdf_ops::condense_pages_vector(slide_pdf_bytes, &specs)?;
    // Verify condensed PDF opens cleanly in MuPDF.
    let _verified = Document::from_bytes(&condensed, "application/pdf")?;
    Ok(condensed)
}

/// Condenses a slide PDF directly into a new `.expdf` [`Notebook`] ready for handwriting and audio
/// recording (L.3 + N.4).
///
/// # Errors
///
/// Returns an error if slide condensation or notebook creation fails.
pub fn condense_slides_to_notebook(
    title: impl Into<String>,
    slide_pdf_bytes: &[u8],
    options: &CondenseOptions,
) -> Result<Notebook> {
    let condensed_pdf = condense_slides_pdf(slide_pdf_bytes, options)?;
    Notebook::from_pdf_bytes(title, condensed_pdf, PaperTemplateId::Blank)
}

fn emit_slide_label(out: &mut String, slide_number: usize, x: f32, y: f32) {
    let _ = writeln!(
        out,
        "q 0.35 0.38 0.44 rg BT /F1 8.5 Tf {x:.2} {y:.2} Td (Slide {slide_number}) Tj ET Q"
    );
}

fn emit_note_area(out: &mut String, rect: [f32; 4], style: CondenseNoteStyle, step: f32) {
    let [x0, y0, x1, y1] = rect;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    match style {
        CondenseNoteStyle::Blank => {}
        CondenseNoteStyle::Lined | CondenseNoteStyle::Cornell => {
            let mut y = y1 - step;
            while y >= y0 + 2.0 {
                let _ = writeln!(
                    out,
                    "0.76 0.84 0.94 RG 0.45 w {x0:.2} {y:.2} m {x1:.2} {y:.2} l S"
                );
                y -= step;
            }
        }
        CondenseNoteStyle::Grid => {
            let gstep = step.min(16.0);
            let mut x = x0;
            while x <= x1 + 0.01 {
                let _ = writeln!(
                    out,
                    "0.82 0.85 0.88 RG 0.35 w {x:.2} {y0:.2} m {x:.2} {y1:.2} l S"
                );
                x += gstep;
            }
            let mut y = y0;
            while y <= y1 + 0.01 {
                let _ = writeln!(
                    out,
                    "0.82 0.85 0.88 RG 0.35 w {x0:.2} {y:.2} m {x1:.2} {y:.2} l S"
                );
                y += gstep;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_core::notebook::{PdfBuilder, PdfPageInput};

    fn make_slide_deck() -> Vec<u8> {
        let mut builder = PdfBuilder::new();
        builder.enable_standard_fonts();
        let slides = [
            "Slide Alpha: Quantum Superposition",
            "Slide Beta: Wavefunction Collapse",
            "Slide Gamma: Bell Inequality",
            "Slide Delta: Entanglement Entropy",
        ];
        for text in slides {
            builder.add_page(PdfPageInput {
                width: 720.0,
                height: 405.0, // 16:9 widescreen slide
                content: format!("BT /F1 24 Tf 48 220 Td ({text}) Tj ET"),
                crop_box: None,
            });
        }
        builder.build().expect("slide deck")
    }

    #[test]
    fn condensed_slides_keep_text_selectable_and_searchable_in_1_2_and_3_up() {
        let deck = make_slide_deck();

        // 3-up Cornell: 4 slides -> 2 condensed pages (slides 1-3 on page 0, slide 4 on page 1).
        let opts3 = CondenseOptions {
            slides_per_page: SlidesPerPage::Three,
            note_style: CondenseNoteStyle::Cornell,
            ..CondenseOptions::default()
        };
        let condensed3 = condense_slides_pdf(&deck, &opts3).expect("condense 3-up");
        let doc3 = Document::from_bytes(&condensed3, "application/pdf").expect("open 3-up");
        assert_eq!(doc3.page_count().expect("pages"), 2);

        let p0_text = doc3
            .load_page(0)
            .expect("p0")
            .text(None)
            .expect("stext")
            .text()
            .expect("extract");
        assert!(
            p0_text.contains("Quantum Superposition"),
            "slide 1 text: {p0_text}"
        );
        assert!(
            p0_text.contains("Wavefunction Collapse"),
            "slide 2 text: {p0_text}"
        );
        assert!(
            p0_text.contains("Bell Inequality"),
            "slide 3 text: {p0_text}"
        );

        let p1_text = doc3
            .load_page(1)
            .expect("p1")
            .text(None)
            .expect("stext")
            .text()
            .expect("extract");
        assert!(
            p1_text.contains("Entanglement Entropy"),
            "slide 4 text: {p1_text}"
        );

        // 2-up Lined: 4 slides -> 2 condensed pages.
        let opts2 = CondenseOptions {
            slides_per_page: SlidesPerPage::Two,
            note_style: CondenseNoteStyle::Lined,
            ..CondenseOptions::default()
        };
        let condensed2 = condense_slides_pdf(&deck, &opts2).expect("condense 2-up");
        let doc2 = Document::from_bytes(&condensed2, "application/pdf").expect("open 2-up");
        assert_eq!(doc2.page_count().expect("pages"), 2);

        // 1-up Grid into a notebook: 4 slides -> 4 pages.
        let opts1 = CondenseOptions {
            slides_per_page: SlidesPerPage::One,
            note_style: CondenseNoteStyle::Grid,
            ..CondenseOptions::default()
        };
        let nb = condense_slides_to_notebook("Quantum Lecture", &deck, &opts1)
            .expect("condense to notebook");
        assert_eq!(nb.metadata.pages.len(), 4);
    }
}
