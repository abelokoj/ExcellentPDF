//! Hyperlinked yearly, monthly, weekly, and daily planner & calendar generator (plan N.11).
//!
//! Generates an `.expdf` [`Notebook`] (and standalone exportable PDF) containing:
//! - Year overview page with clickable month cards linking to every Month page,
//! - 12 Month calendar pages with clickable week numbers and day cells,
//! - 52/53 Week pages with links back to Year/Month and forward to Day pages,
//! - Optional 365/366 Daily pages displaying due assignments from `S.14`,
//! - PDF `/Outlines` bookmarks for Year and each Month so links and navigation work in every PDF reader.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use excellent_core::notebook::{
    Notebook, Orientation, OutlineEntrySpec, PageSize, PaperTemplateId, PdfBuilder, PdfLinkSpec,
    PdfLinkTarget, PdfPageInput, Result,
};
use excellent_mupdf::PageRect;

/// Calendar date (`year`, 1-based `month` 1..=12, 1-based `day` 1..=31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate {
    /// Four-digit Gregorian year (e.g. `2026`).
    pub year: i32,
    /// Month (`1..=12`).
    pub month: u8,
    /// Day of month (`1..=31`).
    pub day: u8,
}

impl CalendarDate {
    /// Constructs a validated Gregorian [`CalendarDate`], returning `None` if out of range.
    #[must_use]
    pub fn try_new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) {
            return None;
        }
        let max_day = days_in_month(year, month);
        if !(1..=max_day).contains(&day) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Day of week (`0` = Sunday, `1` = Monday, ..., `6` = Saturday) via Sakamoto's algorithm.
    #[must_use]
    pub fn weekday_sunday0(self) -> u8 {
        const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut y = self.year;
        if self.month < 3 {
            y -= 1;
        }
        let m = usize::from(self.month - 1);
        let dow = (y + y / 4 - y / 100 + y / 400 + OFFSETS[m] + i32::from(self.day)) % 7;
        u8::try_from(dow.rem_euclid(7)).unwrap_or(0)
    }

    /// Zero-based day index within `self.year` (`0..365` or `0..366`).
    #[must_use]
    pub fn day_of_year_zero_based(self) -> usize {
        let mut total = 0usize;
        for m in 1..self.month {
            total += usize::from(days_in_month(self.year, m));
        }
        total + usize::from(self.day - 1)
    }
}

/// First day of the week shown in calendar grids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    /// Monday-first weeks (ISO standard).
    #[default]
    Monday,
    /// Sunday-first weeks.
    Sunday,
}

/// Whether daily pages are generated in addition to year, month, and week pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DailyPages {
    /// Generate a dedicated page for every day of the year (365/366 pages).
    #[default]
    IncludeAllDays,
    /// Generate only year, month, and week pages (compact ~66-page planner).
    CompactWeeksOnly,
}

/// Optional due item surfaced on matching planner day pages (integration with S.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerDueItem {
    /// Due calendar date.
    pub date: CalendarDate,
    /// Assignment or task title.
    pub title: String,
    /// Optional completion marker.
    pub completed: bool,
}

/// Configuration for generating a yearly planner notebook (plan N.11).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannerConfig {
    /// Target calendar year (e.g., `2026`).
    pub year: i32,
    /// First day of the week in month/week views.
    pub week_start: WeekStart,
    /// Page dimensions.
    pub page_size: PageSize,
    /// Page orientation.
    pub orientation: Orientation,
    /// Whether to include dedicated daily pages.
    pub daily_pages: DailyPages,
    /// Optional due assignments to display on their matching day pages (S.14).
    pub due_items: Vec<PlannerDueItem>,
}

impl PlannerConfig {
    /// Default planner configuration for `year` (A4 portrait, Monday start, full daily pages).
    #[must_use]
    pub fn new(year: i32) -> Self {
        Self {
            year,
            week_start: WeekStart::Monday,
            page_size: PageSize::A4,
            orientation: Orientation::Portrait,
            daily_pages: DailyPages::IncludeAllDays,
            due_items: Vec::new(),
        }
    }
}

/// Page-index map for a generated planner so tests and UIs can jump directly to any section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerIndex {
    /// Zero-based page index of the Year overview (`0`).
    pub year_page: usize,
    /// Zero-based page indices for January (`0`) .. December (`11`).
    pub month_pages: [usize; 12],
    /// Zero-based page indices for each week page.
    pub week_pages: Vec<usize>,
    /// Zero-based page index for each calendar date when [`DailyPages::IncludeAllDays`] is enabled.
    pub day_pages: BTreeMap<CalendarDate, usize>,
}

/// Generated hyperlinked planner notebook and its structural page index.
#[derive(Debug, Clone)]
pub struct GeneratedPlanner {
    /// Ready-to-save `.expdf` [`Notebook`].
    pub notebook: Notebook,
    /// Structural page index.
    pub index: PlannerIndex,
}

/// Returns `true` if `year` is a Gregorian leap year.
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Number of days in `(year, month)` (`month` in `1..=12`).
#[must_use]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Generates a complete hyperlinked Year / Month / Week / Day planner `.expdf` [`Notebook`]
/// (plan N.11). Every internal link is written as a standard PDF `/Link` annotation so it works
/// in every PDF reader.
///
/// # Errors
///
/// Returns [`excellent_core::notebook::NotebookError`] if PDF generation or MuPDF validation fails.
pub fn generate_planner(config: &PlannerConfig) -> Result<GeneratedPlanner> {
    let (w, h) = config.page_size.dimensions(config.orientation);
    let year = config.year;

    // Collect all dates in the year and partition into weeks.
    let mut all_dates = Vec::with_capacity(366);
    for month in 1..=12u8 {
        for day in 1..=days_in_month(year, month) {
            if let Some(d) = CalendarDate::try_new(year, month, day) {
                all_dates.push(d);
            }
        }
    }

    let mut weeks: Vec<Vec<CalendarDate>> = Vec::new();
    for date in &all_dates {
        let start_new = match weeks.last() {
            None => true,
            Some(last_week) if last_week.len() >= 7 => true,
            Some(_) => {
                let dow = date.weekday_sunday0();
                match config.week_start {
                    WeekStart::Monday => dow == 1,
                    WeekStart::Sunday => dow == 0,
                }
            }
        };
        if start_new {
            weeks.push(vec![*date]);
        } else if let Some(last_week) = weeks.last_mut() {
            last_week.push(*date);
        }
    }

    // Assign page indices:
    // 0: Year
    // 1..=12: Months
    // 13..13+weeks.len(): Weeks
    // 13+weeks.len()..: Days (if IncludeAllDays)
    let year_page = 0usize;
    let mut month_pages = [0usize; 12];
    for (idx, slot) in month_pages.iter_mut().enumerate() {
        *slot = 1 + idx;
    }
    let week_start_page = 13usize;
    let week_pages: Vec<usize> = (0..weeks.len()).map(|i| week_start_page + i).collect();

    let mut date_to_week_page: BTreeMap<CalendarDate, usize> = BTreeMap::new();
    for (w_idx, dates) in weeks.iter().enumerate() {
        let p = week_start_page + w_idx;
        for &d in dates {
            date_to_week_page.insert(d, p);
        }
    }

    let mut day_pages: BTreeMap<CalendarDate, usize> = BTreeMap::new();
    if config.daily_pages == DailyPages::IncludeAllDays {
        let day_start_page = week_start_page + weeks.len();
        for (idx, &d) in all_dates.iter().enumerate() {
            day_pages.insert(d, day_start_page + idx);
        }
    }

    let mut due_by_date: BTreeMap<CalendarDate, Vec<&PlannerDueItem>> = BTreeMap::new();
    for item in &config.due_items {
        if item.date.year == year {
            due_by_date.entry(item.date).or_default().push(item);
        }
    }

    let mut builder = PdfBuilder::new();
    builder.enable_standard_fonts();

    // 1. Year Page
    builder.add_page(PdfPageInput {
        width: w,
        height: h,
        content: render_year_stream(year, w, h),
        crop_box: None,
    });
    builder.add_outline(OutlineEntrySpec {
        title: format!("{year} Planner"),
        page: year_page,
        y_pt: None,
    });

    // Link each of the 12 month boxes on the Year page to its Month page.
    let margin = 42.0;
    let col_w = (w - 2.0 * margin - 24.0) / 3.0;
    let row_h = (h - 140.0) / 4.0;
    for (m_idx, &month_page) in month_pages.iter().enumerate() {
        let col = (m_idx % 3) as f32;
        let row = (m_idx / 3) as f32;
        let x0 = margin + col * (col_w + 12.0);
        let y0 = 96.0 + row * row_h;
        builder.add_link(PdfLinkSpec {
            source_page: year_page,
            rect: PageRect {
                x0,
                y0,
                x1: x0 + col_w,
                y1: y0 + row_h - 10.0,
            },
            target: PdfLinkTarget::InternalPage {
                page: month_page,
                point: None,
                zoom: None,
            },
        });
    }

    // 2. Month Pages
    for m_idx in 0..12usize {
        let month = (m_idx + 1) as u8;
        let p_idx = month_pages[m_idx];
        builder.add_page(PdfPageInput {
            width: w,
            height: h,
            content: render_month_stream(year, month, config.week_start, w, h),
            crop_box: None,
        });
        builder.add_outline(OutlineEntrySpec {
            title: format!("{} {year}", MONTH_NAMES[m_idx]),
            page: p_idx,
            y_pt: None,
        });

        // Back link to Year page at top-right ("Year").
        builder.add_link(PdfLinkSpec {
            source_page: p_idx,
            rect: PageRect {
                x0: w - margin - 72.0,
                y0: 32.0,
                x1: w - margin,
                y1: 58.0,
            },
            target: PdfLinkTarget::InternalPage {
                page: year_page,
                point: None,
                zoom: None,
            },
        });

        // Day cell links -> Day page (or Week page if CompactWeeksOnly).
        let cell_w = (w - 2.0 * margin) / 7.0;
        let cell_h = (h - 160.0) / 6.0;
        let first_dow =
            CalendarDate::try_new(year, month, 1).map_or(0, |d| weekday_col(d, config.week_start));
        for day in 1..=days_in_month(year, month) {
            if let Some(d) = CalendarDate::try_new(year, month, day) {
                let slot = first_dow + usize::from(day - 1);
                let col = (slot % 7) as f32;
                let row = (slot / 7) as f32;
                let x0 = margin + col * cell_w;
                let y0 = 110.0 + row * cell_h;
                let target_page = day_pages
                    .get(&d)
                    .copied()
                    .or_else(|| date_to_week_page.get(&d).copied())
                    .unwrap_or(year_page);
                builder.add_link(PdfLinkSpec {
                    source_page: p_idx,
                    rect: PageRect {
                        x0: x0 + 2.0,
                        y0: y0 + 2.0,
                        x1: x0 + cell_w - 2.0,
                        y1: y0 + cell_h - 2.0,
                    },
                    target: PdfLinkTarget::InternalPage {
                        page: target_page,
                        point: None,
                        zoom: None,
                    },
                });
            }
        }
    }

    // 3. Week Pages
    for (w_idx, dates) in weeks.iter().enumerate() {
        let p_idx = week_pages[w_idx];
        builder.add_page(PdfPageInput {
            width: w,
            height: h,
            content: render_week_stream(w_idx + 1, dates, w, h),
            crop_box: None,
        });
        // Link back to Year and to the first day's Month.
        builder.add_link(PdfLinkSpec {
            source_page: p_idx,
            rect: PageRect {
                x0: w - margin - 140.0,
                y0: 32.0,
                x1: w - margin - 76.0,
                y1: 58.0,
            },
            target: PdfLinkTarget::InternalPage {
                page: year_page,
                point: None,
                zoom: None,
            },
        });
        if let Some(first_d) = dates.first() {
            let m_page = month_pages[usize::from(first_d.month - 1)];
            builder.add_link(PdfLinkSpec {
                source_page: p_idx,
                rect: PageRect {
                    x0: w - margin - 72.0,
                    y0: 32.0,
                    x1: w - margin,
                    y1: 58.0,
                },
                target: PdfLinkTarget::InternalPage {
                    page: m_page,
                    point: None,
                    zoom: None,
                },
            });
        }
        // Day rows link to Day pages when included.
        if !day_pages.is_empty() {
            let row_h = (h - 140.0) / 7.0;
            for (i, d) in dates.iter().enumerate() {
                if let Some(&target) = day_pages.get(d) {
                    let y0 = 96.0 + (i as f32) * row_h;
                    builder.add_link(PdfLinkSpec {
                        source_page: p_idx,
                        rect: PageRect {
                            x0: margin,
                            y0,
                            x1: w - margin,
                            y1: y0 + row_h - 6.0,
                        },
                        target: PdfLinkTarget::InternalPage {
                            page: target,
                            point: None,
                            zoom: None,
                        },
                    });
                }
            }
        }
    }

    // 4. Optional Daily Pages (with S.14 due assignments)
    if config.daily_pages == DailyPages::IncludeAllDays {
        for &d in &all_dates {
            let p_idx = day_pages[&d];
            let items = due_by_date.get(&d).map(Vec::as_slice).unwrap_or(&[]);
            builder.add_page(PdfPageInput {
                width: w,
                height: h,
                content: render_day_stream(d, items, w, h),
                crop_box: None,
            });
            // Top-right links: Week & Month
            let w_page = date_to_week_page.get(&d).copied().unwrap_or(year_page);
            let m_page = month_pages[usize::from(d.month - 1)];
            builder.add_link(PdfLinkSpec {
                source_page: p_idx,
                rect: PageRect {
                    x0: w - margin - 140.0,
                    y0: 32.0,
                    x1: w - margin - 76.0,
                    y1: 58.0,
                },
                target: PdfLinkTarget::InternalPage {
                    page: w_page,
                    point: None,
                    zoom: None,
                },
            });
            builder.add_link(PdfLinkSpec {
                source_page: p_idx,
                rect: PageRect {
                    x0: w - margin - 72.0,
                    y0: 32.0,
                    x1: w - margin,
                    y1: 58.0,
                },
                target: PdfLinkTarget::InternalPage {
                    page: m_page,
                    point: None,
                    zoom: None,
                },
            });
        }
    }

    let pdf_bytes = builder.build()?;
    let mut notebook = Notebook::from_pdf_bytes(
        format!("{year} Planner"),
        pdf_bytes,
        PaperTemplateId::Planner,
    )?;
    notebook.metadata.default_page_size = config.page_size;
    notebook.metadata.default_orientation = config.orientation;

    Ok(GeneratedPlanner {
        notebook,
        index: PlannerIndex {
            year_page,
            month_pages,
            week_pages,
            day_pages,
        },
    })
}

fn weekday_col(date: CalendarDate, week_start: WeekStart) -> usize {
    let dow = usize::from(date.weekday_sunday0());
    match week_start {
        WeekStart::Sunday => dow,
        WeekStart::Monday => (dow + 6) % 7,
    }
}

fn pdf_text(stream: &mut String, x: f32, y_top: f32, page_h: f32, size: f32, text: &str) {
    let y_pdf = page_h - y_top;
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    let _ = writeln!(
        stream,
        "BT /F1 {size:.1} Tf 0.12 0.14 0.18 rg {x:.2} {y_pdf:.2} Td ({escaped}) Tj ET"
    );
}

fn pdf_rect(stream: &mut String, x: f32, y_top: f32, w: f32, h: f32, page_h: f32) {
    let y_pdf = page_h - (y_top + h);
    let _ = writeln!(
        stream,
        "0.75 0.79 0.85 RG 0.6 w {x:.2} {y_pdf:.2} {w:.2} {h:.2} re S"
    );
}

fn render_year_stream(year: i32, w: f32, h: f32) -> String {
    let mut s = String::new();
    let margin = 42.0;
    pdf_text(
        &mut s,
        margin,
        56.0,
        h,
        22.0,
        &format!("{year} Year Overview"),
    );
    let col_w = (w - 2.0 * margin - 24.0) / 3.0;
    let row_h = (h - 140.0) / 4.0;
    for (m_idx, month_name) in MONTH_NAMES.iter().enumerate() {
        let col = (m_idx % 3) as f32;
        let row = (m_idx / 3) as f32;
        let x0 = margin + col * (col_w + 12.0);
        let y0 = 96.0 + row * row_h;
        pdf_rect(&mut s, x0, y0, col_w, row_h - 10.0, h);
        pdf_text(&mut s, x0 + 10.0, y0 + 22.0, h, 13.0, month_name);
    }
    s
}

fn render_month_stream(year: i32, month: u8, week_start: WeekStart, w: f32, h: f32) -> String {
    let mut s = String::new();
    let margin = 42.0;
    let m_name = MONTH_NAMES[usize::from(month - 1)];
    pdf_text(&mut s, margin, 56.0, h, 20.0, &format!("{m_name} {year}"));
    pdf_text(&mut s, w - margin - 60.0, 50.0, h, 11.0, "[Year]");

    let labels = match week_start {
        WeekStart::Monday => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        WeekStart::Sunday => ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    };
    let cell_w = (w - 2.0 * margin) / 7.0;
    let cell_h = (h - 160.0) / 6.0;
    for (col, lbl) in labels.iter().enumerate() {
        pdf_text(
            &mut s,
            margin + col as f32 * cell_w + 6.0,
            98.0,
            h,
            10.0,
            lbl,
        );
    }

    let first_dow = CalendarDate::try_new(year, month, 1).map_or(0, |d| weekday_col(d, week_start));
    for day in 1..=days_in_month(year, month) {
        let slot = first_dow + usize::from(day - 1);
        let col = (slot % 7) as f32;
        let row = (slot / 7) as f32;
        let x0 = margin + col * cell_w;
        let y0 = 110.0 + row * cell_h;
        pdf_rect(&mut s, x0 + 2.0, y0 + 2.0, cell_w - 4.0, cell_h - 4.0, h);
        pdf_text(&mut s, x0 + 8.0, y0 + 18.0, h, 11.0, &day.to_string());
    }
    s
}

fn render_week_stream(week_num: usize, dates: &[CalendarDate], w: f32, h: f32) -> String {
    let mut s = String::new();
    let margin = 42.0;
    pdf_text(&mut s, margin, 56.0, h, 18.0, &format!("Week {week_num}"));
    pdf_text(&mut s, w - margin - 130.0, 50.0, h, 11.0, "[Year]");
    pdf_text(&mut s, w - margin - 62.0, 50.0, h, 11.0, "[Month]");

    let row_h = (h - 140.0) / 7.0;
    for (i, d) in dates.iter().enumerate() {
        let y0 = 96.0 + i as f32 * row_h;
        pdf_rect(&mut s, margin, y0, w - 2.0 * margin, row_h - 6.0, h);
        let label = format!(
            "{} {}, {}",
            MONTH_NAMES[usize::from(d.month - 1)],
            d.day,
            d.year
        );
        pdf_text(&mut s, margin + 10.0, y0 + 20.0, h, 12.0, &label);
    }
    s
}

fn render_day_stream(date: CalendarDate, due_items: &[&PlannerDueItem], w: f32, h: f32) -> String {
    let mut s = String::new();
    let margin = 42.0;
    let title = format!(
        "{} {}, {}",
        MONTH_NAMES[usize::from(date.month - 1)],
        date.day,
        date.year
    );
    pdf_text(&mut s, margin, 56.0, h, 18.0, &title);
    pdf_text(&mut s, w - margin - 130.0, 50.0, h, 11.0, "[Week]");
    pdf_text(&mut s, w - margin - 62.0, 50.0, h, 11.0, "[Month]");

    let mut y = 96.0;
    if !due_items.is_empty() {
        pdf_text(&mut s, margin, y, h, 12.0, "Due Assignments:");
        y += 18.0;
        for item in due_items {
            let mark = if item.completed { "[x]" } else { "[ ]" };
            pdf_text(
                &mut s,
                margin + 12.0,
                y,
                h,
                11.0,
                &format!("{mark} {}", item.title),
            );
            y += 16.0;
        }
        y += 12.0;
    }

    // Lined note area underneath.
    while y < h - 48.0 {
        let y_pdf = h - y;
        let _ = writeln!(
            s,
            "0.84 0.88 0.94 RG 0.5 w {margin:.2} {y_pdf:.2} m {:.2} {y_pdf:.2} l S",
            w - margin
        );
        y += 22.0;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_year_planner_generates_valid_hyperlinks_and_due_dates() {
        let due_date = CalendarDate::try_new(2024, 2, 29).expect("leap day");
        let mut cfg = PlannerConfig::new(2024);
        cfg.due_items.push(PlannerDueItem {
            date: due_date,
            title: "Submit Physics Lab".into(),
            completed: false,
        });

        let generated = generate_planner(&cfg).expect("generate 2024 planner");
        assert_eq!(generated.index.day_pages.len(), 366);
        let feb29_page = generated.index.day_pages[&due_date];

        let doc = generated.notebook.open_mupdf().expect("open in mupdf");
        let total_pages = doc.page_count().expect("count");
        assert_eq!(total_pages, 1 + 12 + generated.index.week_pages.len() + 366);

        // Verify every link on Year, February, first Week, and Feb 29 resolves cleanly in MuPDF.
        for page_idx in [
            0,
            generated.index.month_pages[1],
            generated.index.week_pages[0],
            feb29_page,
        ] {
            let page = doc.load_page(page_idx).expect("load page");
            let links = page.links().expect("links");
            assert!(!links.is_empty(), "page {page_idx} has links");
            for link in links {
                let target = link.target.expect("internal target");
                assert!(
                    target.page < total_pages,
                    "target {} < {total_pages}",
                    target.page
                );
            }
        }

        // Verify S.14 due item text appears on the Feb 29 day page.
        let feb29_text = doc
            .load_page(feb29_page)
            .expect("feb29")
            .text(None)
            .expect("stext")
            .text()
            .expect("text");
        assert!(feb29_text.contains("Submit Physics Lab"), "{feb29_text}");
    }
}
