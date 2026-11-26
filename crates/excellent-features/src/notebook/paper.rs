//! Procedural paper templates (plan N.1).
//!
//! Each template is a small parameter set ([`PaperSpec`]) generating a deterministic list of
//! geometric [`PaperPrimitive`]s in top-left page points. Both the live canvas underlay renderer
//! and plain PDF export ([`PaperSpec::to_pdf_content_stream`]) consume the exact same primitive list,
//! guaranteeing visual identity on screen and in exported PDFs.

use std::fmt::Write as _;

use excellent_core::darkmode::themes::ReadingTheme;
use excellent_core::notebook::{
    Orientation, PageSize, PaperTemplateId, PdfBuilder, PdfPageInput, Result,
};

/// A single geometric primitive drawn for a procedural paper template in top-left page points.
#[derive(Debug, Clone, PartialEq)]
pub enum PaperPrimitive {
    /// Background paper fill rectangle covering `(x, y, width, height)`.
    FillRect {
        /// Left x in page points.
        x: f32,
        /// Top y in page points.
        y: f32,
        /// Width in page points.
        width: f32,
        /// Height in page points.
        height: f32,
        /// Fill RGB in `0.0..=1.0`.
        color: [f32; 3],
    },
    /// Straight line segment from `(x0, y0)` to `(x1, y1)`.
    Line {
        /// Start x in page points.
        x0: f32,
        /// Start y in page points.
        y0: f32,
        /// End x in page points.
        x1: f32,
        /// End y in page points.
        y1: f32,
        /// Stroke width in points.
        width: f32,
        /// Stroke RGB in `0.0..=1.0`.
        color: [f32; 3],
    },
    /// Filled dot / circle at `(cx, cy)`.
    Dot {
        /// Center x in page points.
        cx: f32,
        /// Center y in page points.
        cy: f32,
        /// Radius in points.
        radius: f32,
        /// Fill RGB in `0.0..=1.0`.
        color: [f32; 3],
    },
    /// Stroked rectangle outline (e.g., planner boxes).
    StrokeRect {
        /// Left x in page points.
        x: f32,
        /// Top y in page points.
        y: f32,
        /// Width in page points.
        width: f32,
        /// Height in page points.
        height: f32,
        /// Stroke width in points.
        stroke_width: f32,
        /// Stroke RGB in `0.0..=1.0`.
        color: [f32; 3],
    },
}

/// Parameters defining a procedural paper template (N.1).
#[derive(Debug, Clone, PartialEq)]
pub struct PaperSpec {
    /// Which built-in template geometry to generate.
    pub template: PaperTemplateId,
    /// Primary ruling or grid spacing in points (e.g., 24 pt lined, 20.16 pt college-ruled).
    pub spacing_pt: f32,
    /// Left margin in points.
    pub margin_left_pt: f32,
    /// Right margin in points.
    pub margin_right_pt: f32,
    /// Top margin in points.
    pub margin_top_pt: f32,
    /// Bottom margin in points.
    pub margin_bottom_pt: f32,
    /// Primary ruling / grid line color (RGB `0.0..=1.0`).
    pub line_color: [f32; 3],
    /// Accent / margin / major-grid line color (RGB `0.0..=1.0`).
    pub accent_color: [f32; 3],
    /// Paper background color (RGB `0.0..=1.0`).
    pub paper_color: [f32; 3],
    /// Default stroke width in points.
    pub line_width_pt: f32,
}

impl PaperSpec {
    /// Returns the default parameter set for `template`.
    #[must_use]
    pub fn for_template(template: PaperTemplateId) -> Self {
        let base_paper = [1.0, 1.0, 1.0];
        let soft_blue = [0.76, 0.84, 0.94];
        let soft_gray = [0.80, 0.82, 0.86];
        let margin_red = [0.90, 0.56, 0.58];
        let major_teal = [0.54, 0.72, 0.78];

        match template {
            PaperTemplateId::Blank => Self {
                template,
                spacing_pt: 24.0,
                margin_left_pt: 36.0,
                margin_right_pt: 36.0,
                margin_top_pt: 36.0,
                margin_bottom_pt: 36.0,
                line_color: soft_gray,
                accent_color: margin_red,
                paper_color: base_paper,
                line_width_pt: 0.5,
            },
            PaperTemplateId::Lined => Self {
                template,
                spacing_pt: 25.0, // ~8.8 mm wide ruled
                margin_left_pt: 36.0,
                margin_right_pt: 36.0,
                margin_top_pt: 72.0,
                margin_bottom_pt: 48.0,
                line_color: soft_blue,
                accent_color: margin_red,
                paper_color: base_paper,
                line_width_pt: 0.5,
            },
            PaperTemplateId::CollegeRuled => Self {
                template,
                spacing_pt: 20.16, // 7.1 mm standard college rule
                margin_left_pt: 90.0,
                margin_right_pt: 36.0,
                margin_top_pt: 72.0,
                margin_bottom_pt: 48.0,
                line_color: soft_blue,
                accent_color: margin_red,
                paper_color: base_paper,
                line_width_pt: 0.45,
            },
            PaperTemplateId::Grid => Self {
                template,
                spacing_pt: 14.173, // 5 mm square grid
                margin_left_pt: 28.0,
                margin_right_pt: 28.0,
                margin_top_pt: 28.0,
                margin_bottom_pt: 28.0,
                line_color: soft_gray,
                accent_color: major_teal,
                paper_color: base_paper,
                line_width_pt: 0.4,
            },
            PaperTemplateId::Dot => Self {
                template,
                spacing_pt: 14.173, // 5 mm dot grid
                margin_left_pt: 28.0,
                margin_right_pt: 28.0,
                margin_top_pt: 28.0,
                margin_bottom_pt: 28.0,
                line_color: [0.68, 0.71, 0.76],
                accent_color: major_teal,
                paper_color: base_paper,
                line_width_pt: 0.85,
            },
            PaperTemplateId::Cornell => Self {
                template,
                spacing_pt: 22.0,
                margin_left_pt: 162.0, // 2.25 in cue column
                margin_right_pt: 36.0,
                margin_top_pt: 84.0,     // Header band
                margin_bottom_pt: 120.0, // Summary footer band
                line_color: soft_blue,
                accent_color: [0.42, 0.52, 0.68],
                paper_color: base_paper,
                line_width_pt: 0.5,
            },
            PaperTemplateId::Music => Self {
                template,
                spacing_pt: 6.0, // 6 pt between staff lines
                margin_left_pt: 48.0,
                margin_right_pt: 48.0,
                margin_top_pt: 72.0,
                margin_bottom_pt: 60.0,
                line_color: [0.22, 0.24, 0.28],
                accent_color: [0.22, 0.24, 0.28],
                paper_color: base_paper,
                line_width_pt: 0.55,
            },
            PaperTemplateId::Graph => Self {
                template,
                spacing_pt: 7.2, // 0.1 in engineering minor grid; every 5th line major
                margin_left_pt: 36.0,
                margin_right_pt: 36.0,
                margin_top_pt: 36.0,
                margin_bottom_pt: 36.0,
                line_color: [0.84, 0.90, 0.88],
                accent_color: [0.52, 0.74, 0.68],
                paper_color: base_paper,
                line_width_pt: 0.35,
            },
            PaperTemplateId::Isometric => Self {
                template,
                spacing_pt: 18.0,
                margin_left_pt: 28.0,
                margin_right_pt: 28.0,
                margin_top_pt: 28.0,
                margin_bottom_pt: 28.0,
                line_color: [0.78, 0.83, 0.88],
                accent_color: major_teal,
                paper_color: base_paper,
                line_width_pt: 0.4,
            },
            PaperTemplateId::Planner => Self {
                template,
                spacing_pt: 24.0,
                margin_left_pt: 36.0,
                margin_right_pt: 36.0,
                margin_top_pt: 54.0,
                margin_bottom_pt: 36.0,
                line_color: soft_gray,
                accent_color: [0.36, 0.48, 0.72],
                paper_color: base_paper,
                line_width_pt: 0.55,
            },
        }
    }

    /// Adapts this paper's background and ruling colors to a reading theme (plan R.15).
    #[must_use]
    pub fn with_reading_theme(mut self, theme: ReadingTheme) -> Self {
        if let Some(palette) = theme.palette() {
            let paper = rgb_u8_to_f32(palette.paper);
            let ink = rgb_u8_to_f32(palette.ink);
            self.paper_color = paper;
            self.line_color = blend(paper, ink, 0.22);
            self.accent_color = blend(paper, ink, 0.45);
        }
        self
    }

    /// Generates the exact list of geometric primitives for a page of `(width_pt, height_pt)`.
    #[must_use]
    pub fn primitives(&self, width_pt: f32, height_pt: f32) -> Vec<PaperPrimitive> {
        let w = width_pt.max(72.0);
        let h = height_pt.max(72.0);
        let step = self.spacing_pt.max(4.0);
        let mut out = Vec::new();

        // Paper background fill when non-white.
        if self.paper_color != [1.0, 1.0, 1.0] {
            out.push(PaperPrimitive::FillRect {
                x: 0.0,
                y: 0.0,
                width: w,
                height: h,
                color: self.paper_color,
            });
        }

        match self.template {
            PaperTemplateId::Blank => {}
            PaperTemplateId::Lined => {
                let mut y = self.margin_top_pt;
                while y <= h - self.margin_bottom_pt + 0.01 {
                    out.push(PaperPrimitive::Line {
                        x0: self.margin_left_pt,
                        y0: y,
                        x1: (w - self.margin_right_pt).max(self.margin_left_pt),
                        y1: y,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    y += step;
                }
            }
            PaperTemplateId::CollegeRuled => {
                let mut y = self.margin_top_pt;
                while y <= h - self.margin_bottom_pt + 0.01 {
                    out.push(PaperPrimitive::Line {
                        x0: 0.0,
                        y0: y,
                        x1: w,
                        y1: y,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    y += step;
                }
                // Vertical left margin rule.
                let mx = self.margin_left_pt.min(w * 0.4);
                out.push(PaperPrimitive::Line {
                    x0: mx,
                    y0: 0.0,
                    x1: mx,
                    y1: h,
                    width: self.line_width_pt * 1.4,
                    color: self.accent_color,
                });
            }
            PaperTemplateId::Grid => {
                let x_start = self.margin_left_pt;
                let x_end = (w - self.margin_right_pt).max(x_start);
                let y_start = self.margin_top_pt;
                let y_end = (h - self.margin_bottom_pt).max(y_start);
                let mut x = x_start;
                while x <= x_end + 0.01 {
                    out.push(PaperPrimitive::Line {
                        x0: x,
                        y0: y_start,
                        x1: x,
                        y1: y_end,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    x += step;
                }
                let mut y = y_start;
                while y <= y_end + 0.01 {
                    out.push(PaperPrimitive::Line {
                        x0: x_start,
                        y0: y,
                        x1: x_end,
                        y1: y,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    y += step;
                }
            }
            PaperTemplateId::Dot => {
                let x_start = self.margin_left_pt;
                let x_end = (w - self.margin_right_pt).max(x_start);
                let y_start = self.margin_top_pt;
                let y_end = (h - self.margin_bottom_pt).max(y_start);
                let radius = self.line_width_pt.max(0.5);
                let mut y = y_start;
                while y <= y_end + 0.01 {
                    let mut x = x_start;
                    while x <= x_end + 0.01 {
                        out.push(PaperPrimitive::Dot {
                            cx: x,
                            cy: y,
                            radius,
                            color: self.line_color,
                        });
                        x += step;
                    }
                    y += step;
                }
            }
            PaperTemplateId::Cornell => {
                let cue_x = self.margin_left_pt.min(w * 0.4);
                let header_y = self.margin_top_pt.min(h * 0.2);
                let summary_y = (h - self.margin_bottom_pt).max(header_y + step);
                let right_x = (w - self.margin_right_pt).max(cue_x + step);

                // Header divider, summary divider, and cue column divider.
                out.push(PaperPrimitive::Line {
                    x0: 24.0,
                    y0: header_y,
                    x1: right_x,
                    y1: header_y,
                    width: self.line_width_pt * 2.0,
                    color: self.accent_color,
                });
                out.push(PaperPrimitive::Line {
                    x0: 24.0,
                    y0: summary_y,
                    x1: right_x,
                    y1: summary_y,
                    width: self.line_width_pt * 2.0,
                    color: self.accent_color,
                });
                out.push(PaperPrimitive::Line {
                    x0: cue_x,
                    y0: header_y,
                    x1: cue_x,
                    y1: summary_y,
                    width: self.line_width_pt * 2.0,
                    color: self.accent_color,
                });

                // Ruled lines inside the notes area.
                let mut y = header_y + step;
                while y < summary_y - 0.5 {
                    out.push(PaperPrimitive::Line {
                        x0: cue_x + 8.0,
                        y0: y,
                        x1: right_x,
                        y1: y,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    y += step;
                }

                // Ruled lines inside the summary footer.
                let mut sy = summary_y + step;
                while sy <= h - 24.0 {
                    out.push(PaperPrimitive::Line {
                        x0: 24.0,
                        y0: sy,
                        x1: right_x,
                        y1: sy,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    sy += step;
                }
            }
            PaperTemplateId::Music => {
                let x_start = self.margin_left_pt;
                let x_end = (w - self.margin_right_pt).max(x_start + 36.0);
                let staff_gap = step * 7.5;
                let mut top = self.margin_top_pt;
                while top + step * 4.0 <= h - self.margin_bottom_pt + 0.01 {
                    for line in 0..5 {
                        let y = top + line as f32 * step;
                        out.push(PaperPrimitive::Line {
                            x0: x_start,
                            y0: y,
                            x1: x_end,
                            y1: y,
                            width: self.line_width_pt,
                            color: self.line_color,
                        });
                    }
                    // Left and right barlines closing the 5-line staff.
                    out.push(PaperPrimitive::Line {
                        x0: x_start,
                        y0: top,
                        x1: x_start,
                        y1: top + step * 4.0,
                        width: self.line_width_pt * 1.4,
                        color: self.accent_color,
                    });
                    out.push(PaperPrimitive::Line {
                        x0: x_end,
                        y0: top,
                        x1: x_end,
                        y1: top + step * 4.0,
                        width: self.line_width_pt * 1.4,
                        color: self.accent_color,
                    });
                    top += step * 4.0 + staff_gap;
                }
            }
            PaperTemplateId::Graph => {
                let x_start = self.margin_left_pt;
                let x_end = (w - self.margin_right_pt).max(x_start);
                let y_start = self.margin_top_pt;
                let y_end = (h - self.margin_bottom_pt).max(y_start);

                let mut ix = 0usize;
                let mut x = x_start;
                while x <= x_end + 0.01 {
                    let major = ix.is_multiple_of(5);
                    out.push(PaperPrimitive::Line {
                        x0: x,
                        y0: y_start,
                        x1: x,
                        y1: y_end,
                        width: if major {
                            self.line_width_pt * 2.0
                        } else {
                            self.line_width_pt
                        },
                        color: if major {
                            self.accent_color
                        } else {
                            self.line_color
                        },
                    });
                    ix += 1;
                    x = x_start + ix as f32 * step;
                }

                let mut iy = 0usize;
                let mut y = y_start;
                while y <= y_end + 0.01 {
                    let major = iy.is_multiple_of(5);
                    out.push(PaperPrimitive::Line {
                        x0: x_start,
                        y0: y,
                        x1: x_end,
                        y1: y,
                        width: if major {
                            self.line_width_pt * 2.0
                        } else {
                            self.line_width_pt
                        },
                        color: if major {
                            self.accent_color
                        } else {
                            self.line_color
                        },
                    });
                    iy += 1;
                    y = y_start + iy as f32 * step;
                }
            }
            PaperTemplateId::Isometric => {
                let x0 = self.margin_left_pt;
                let x1 = (w - self.margin_right_pt).max(x0 + step);
                let y0 = self.margin_top_pt;
                let y1 = (h - self.margin_bottom_pt).max(y0 + step);
                let dx = step * (3.0_f32.sqrt() * 0.5);
                let slope = 1.0 / 3.0_f32.sqrt(); // tan(30 deg)

                // Vertical lines spaced by dx.
                let mut x = x0;
                while x <= x1 + 0.01 {
                    out.push(PaperPrimitive::Line {
                        x0: x,
                        y0,
                        x1: x,
                        y1,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    x += dx;
                }

                // +30 deg and -30 deg diagonals clipped to [x0..x1] x [y0..y1].
                let span_y = (x1 - x0) * slope;
                let mut start_y = y0 - span_y;
                while start_y <= y1 + span_y {
                    if let Some((ax, ay, bx, by)) =
                        clip_line_to_rect([x0, start_y, x1, start_y + span_y], [x0, y0, x1, y1])
                    {
                        out.push(PaperPrimitive::Line {
                            x0: ax,
                            y0: ay,
                            x1: bx,
                            y1: by,
                            width: self.line_width_pt,
                            color: self.line_color,
                        });
                    }
                    if let Some((ax, ay, bx, by)) =
                        clip_line_to_rect([x0, start_y, x1, start_y - span_y], [x0, y0, x1, y1])
                    {
                        out.push(PaperPrimitive::Line {
                            x0: ax,
                            y0: ay,
                            x1: bx,
                            y1: by,
                            width: self.line_width_pt,
                            color: self.line_color,
                        });
                    }
                    start_y += step;
                }
            }
            PaperTemplateId::Planner => {
                let left = self.margin_left_pt;
                let right = (w - self.margin_right_pt).max(left + 120.0);
                let top = self.margin_top_pt;
                let bottom = (h - self.margin_bottom_pt).max(top + 120.0);
                let usable_w = right - left;
                let header_h = 36.0_f32.min((bottom - top) * 0.15);

                // Header box.
                out.push(PaperPrimitive::StrokeRect {
                    x: left,
                    y: top,
                    width: usable_w,
                    height: header_h,
                    stroke_width: self.line_width_pt * 1.6,
                    color: self.accent_color,
                });

                // Two-column split: left schedule (60%), right tasks + notes (40%).
                let body_top = top + header_h + 12.0;
                let body_h = (bottom - body_top).max(40.0);
                let gap = 12.0;
                let sched_w = (usable_w - gap) * 0.58;
                let task_x = left + sched_w + gap;
                let task_w = usable_w - sched_w - gap;

                out.push(PaperPrimitive::StrokeRect {
                    x: left,
                    y: body_top,
                    width: sched_w,
                    height: body_h,
                    stroke_width: self.line_width_pt * 1.3,
                    color: self.accent_color,
                });
                let mut y = body_top + step;
                while y < bottom - 4.0 {
                    out.push(PaperPrimitive::Line {
                        x0: left + 4.0,
                        y0: y,
                        x1: left + sched_w - 4.0,
                        y1: y,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    y += step;
                }

                // Right top priorities box + checkboxes.
                let prio_h = body_h * 0.48;
                out.push(PaperPrimitive::StrokeRect {
                    x: task_x,
                    y: body_top,
                    width: task_w,
                    height: prio_h,
                    stroke_width: self.line_width_pt * 1.3,
                    color: self.accent_color,
                });
                let mut cy = body_top + 14.0;
                while cy + 10.0 < body_top + prio_h - 6.0 {
                    out.push(PaperPrimitive::StrokeRect {
                        x: task_x + 8.0,
                        y: cy,
                        width: 8.0,
                        height: 8.0,
                        stroke_width: self.line_width_pt,
                        color: self.accent_color,
                    });
                    out.push(PaperPrimitive::Line {
                        x0: task_x + 22.0,
                        y0: cy + 8.0,
                        x1: task_x + task_w - 8.0,
                        y1: cy + 8.0,
                        width: self.line_width_pt,
                        color: self.line_color,
                    });
                    cy += step;
                }

                // Right bottom notes box.
                let notes_top = body_top + prio_h + gap;
                let notes_h = (bottom - notes_top).max(20.0);
                out.push(PaperPrimitive::StrokeRect {
                    x: task_x,
                    y: notes_top,
                    width: task_w,
                    height: notes_h,
                    stroke_width: self.line_width_pt * 1.3,
                    color: self.accent_color,
                });
            }
        }

        out
    }

    /// Converts this template's primitives into a pure-vector PDF content stream for a page of
    /// `(width_pt, height_pt)`. Uses the exact same geometry as [`Self::primitives`].
    #[must_use]
    pub fn to_pdf_content_stream(&self, width_pt: f32, height_pt: f32) -> String {
        primitives_to_pdf_stream(&self.primitives(width_pt, height_pt), height_pt)
    }

    /// Exports a single-page standalone vector PDF of this procedural paper template.
    ///
    /// # Errors
    ///
    /// Returns an error if PDF construction fails.
    pub fn export_single_page_pdf(
        &self,
        size: PageSize,
        orientation: Orientation,
    ) -> Result<Vec<u8>> {
        let (w, h) = size.dimensions(orientation);
        let mut builder = PdfBuilder::new();
        builder.add_page(PdfPageInput {
            width: w,
            height: h,
            content: self.to_pdf_content_stream(w, h),
            crop_box: None,
        });
        builder.build()
    }
}

/// Converts a slice of top-left [`PaperPrimitive`]s into PDF bottom-left vector drawing operators.
#[must_use]
pub fn primitives_to_pdf_stream(primitives: &[PaperPrimitive], page_height_pt: f32) -> String {
    let mut out = String::new();
    for prim in primitives {
        match *prim {
            PaperPrimitive::FillRect {
                x,
                y,
                width,
                height,
                color: [r, g, b],
            } => {
                let py = page_height_pt - (y + height);
                let _ = writeln!(
                    out,
                    "{r:.4} {g:.4} {b:.4} rg {x:.3} {py:.3} {width:.3} {height:.3} re f"
                );
            }
            PaperPrimitive::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color: [r, g, b],
            } => {
                let py0 = page_height_pt - y0;
                let py1 = page_height_pt - y1;
                let _ = writeln!(
                    out,
                    "{r:.4} {g:.4} {b:.4} RG {width:.3} w {x0:.3} {py0:.3} m {x1:.3} {py1:.3} l S"
                );
            }
            PaperPrimitive::Dot {
                cx,
                cy,
                radius,
                color: [r, g, b],
            } => {
                // 4-segment cubic Bezier circle approximation (k = 0.55228475).
                let py = page_height_pt - cy;
                let k = radius * 0.552_284_8;
                let _ = writeln!(
                    out,
                    "{r:.4} {g:.4} {b:.4} rg {:.3} {:.3} m {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c f",
                    cx + radius,
                    py,
                    cx + radius,
                    py + k,
                    cx + k,
                    py + radius,
                    cx,
                    py + radius,
                    cx - k,
                    py + radius,
                    cx - radius,
                    py + k,
                    cx - radius,
                    py,
                    cx - radius,
                    py - k,
                    cx - k,
                    py - radius,
                    cx,
                    py - radius,
                    cx + k,
                    py - radius,
                    cx + radius,
                    py - k,
                    cx + radius,
                    py
                );
            }
            PaperPrimitive::StrokeRect {
                x,
                y,
                width,
                height,
                stroke_width,
                color: [r, g, b],
            } => {
                let py = page_height_pt - (y + height);
                let _ = writeln!(
                    out,
                    "{r:.4} {g:.4} {b:.4} RG {stroke_width:.3} w {x:.3} {py:.3} {width:.3} {height:.3} re S"
                );
            }
        }
    }
    out
}

fn rgb_u8_to_f32([r, g, b]: [u8; 3]) -> [f32; 3] {
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    ]
}

fn blend(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Liang-Barsky line clipping of segment `[x0, y0, x1, y1]` to axis-aligned rectangle `[xmin, ymin, xmax, ymax]`.
fn clip_line_to_rect(segment: [f32; 4], bounds: [f32; 4]) -> Option<(f32, f32, f32, f32)> {
    let [x0, y0, x1, y1] = segment;
    let [xmin, ymin, xmax, ymax] = bounds;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let p = [-dx, dx, -dy, dy];
    let q = [x0 - xmin, xmax - x0, y0 - ymin, ymax - y0];
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    for i in 0..4 {
        if p[i].abs() < 1e-6 {
            if q[i] < 0.0 {
                return None;
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    Some((x0 + t0 * dx, y0 + t0 * dy, x0 + t1 * dx, y0 + t1 * dy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_mupdf::{Background, Document, Matrix};

    #[test]
    fn all_ten_procedural_templates_export_and_render_in_mupdf() {
        for template in PaperTemplateId::ALL {
            let spec = PaperSpec::for_template(template);
            let (w, h) = PageSize::A4.dimensions(Orientation::Portrait);
            let prims = spec.primitives(w, h);
            if template == PaperTemplateId::Blank {
                assert!(prims.is_empty(), "blank white paper needs zero primitives");
            } else {
                assert!(!prims.is_empty(), "{template:?} should emit primitives");
            }

            let pdf = spec
                .export_single_page_pdf(PageSize::A4, Orientation::Portrait)
                .expect("export pdf");
            let doc = Document::from_bytes(&pdf, "application/pdf").expect("open in mupdf");
            assert_eq!(doc.page_count().expect("pages"), 1);
            let page = doc.load_page(0).expect("page 0");
            let pix = page
                .render(&Matrix::scale(0.5, 0.5), Background::White)
                .expect("render");
            let samples = pix.samples();
            if template != PaperTemplateId::Blank {
                let non_white = samples
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter(|px| **px != [255, 255, 255])
                    .count();
                assert!(
                    non_white > 10,
                    "{template:?} rendered vector template must draw visible pixels (got {non_white})"
                );
            }
        }
    }

    #[test]
    fn paper_spec_adapts_to_reading_themes_and_page_sizes() {
        let sepia = PaperSpec::for_template(PaperTemplateId::Cornell)
            .with_reading_theme(ReadingTheme::Sepia);
        assert_ne!(sepia.paper_color, [1.0, 1.0, 1.0]);
        let (pw, ph) = PageSize::Phone.dimensions(Orientation::Landscape);
        assert_eq!((pw, ph), (640.0, 360.0));
        let prims = sepia.primitives(pw, ph);
        assert!(matches!(
            prims.first(),
            Some(PaperPrimitive::FillRect { .. })
        ));
    }
}
