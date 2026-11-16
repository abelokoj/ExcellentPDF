//! Stamps (A.20): the PDF standard's fourteen rubber stamps, which MuPDF and every other reader draw from their name;
//! dynamic stamps, a title and a line of who stamped it and when, laid out as HTML by MuPDF into a one-page PDF and
//! set as the stamp's appearance; and the reader's own stamps, kept as small files in a library folder (one-page
//! PDFs, or images), imported from a PDF's first page or an image file.
//!
//! A drawn stamp is a [`Change::CreateAnnotation`] then a [`Change::StampArt`], one operation, so one step of undo.
//! Its name is not one of the standard's, or MuPDF would draw that name over the art (DD18).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use excellent_core::annot::{Annotation, Boxed, Shape, Style};
use excellent_core::edit::Change;
use excellent_mupdf::{Document, PagePoint, PageRect, StampArt};

use super::inspector;

/// A standard stamp's size on the page, in points: the 190 by 50 box MuPDF draws them in, at Acrobat's size.
pub const STANDARD_SIZE: (f32, f32) = (152.0, 40.0);

/// The widest and tallest a library stamp is placed, in points; it can be resized afterwards.
pub const LIBRARY_SIZE: f32 = 200.0;

/// A dynamic stamp's width, in points; its height is its text's.
pub const DYNAMIC_WIDTH: f32 = 180.0;

/// The colour a library stamp is listed with: its art has its own.
pub const LIBRARY_COLOR: [f32; 3] = BLUE;

/// The image files the library takes, by extension (lower case).
pub const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// The stamps the PDF standard names (ISO 32000-2, 12.5.6.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Standard {
    /// "APPROVED".
    Approved,
    /// "AS IS".
    AsIs,
    /// "CONFIDENTIAL".
    Confidential,
    /// "DEPARTMENTAL".
    Departmental,
    /// "DRAFT".
    Draft,
    /// "EXPERIMENTAL".
    Experimental,
    /// "EXPIRED".
    Expired,
    /// "FINAL".
    Final,
    /// "FOR COMMENT".
    ForComment,
    /// "FOR PUBLIC RELEASE".
    ForPublicRelease,
    /// "NOT APPROVED".
    NotApproved,
    /// "NOT FOR PUBLIC RELEASE".
    NotForPublicRelease,
    /// "SOLD".
    Sold,
    /// "TOP SECRET".
    TopSecret,
}

/// Acrobat's stamp colours: green for yes, red for no or secret, blue for the rest.
const GREEN: [f32; 3] = [0.13, 0.55, 0.13];
const RED: [f32; 3] = [0.8, 0.1, 0.1];
const BLUE: [f32; 3] = [0.1, 0.25, 0.7];

impl Standard {
    /// Every standard stamp, in the order the picker offers them.
    pub const ALL: [Self; 14] = [
        Self::Approved,
        Self::NotApproved,
        Self::Draft,
        Self::Final,
        Self::Confidential,
        Self::TopSecret,
        Self::ForComment,
        Self::ForPublicRelease,
        Self::NotForPublicRelease,
        Self::AsIs,
        Self::Departmental,
        Self::Experimental,
        Self::Expired,
        Self::Sold,
    ];

    /// Its name in the PDF (`/Name`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::AsIs => "AsIs",
            Self::Confidential => "Confidential",
            Self::Departmental => "Departmental",
            Self::Draft => "Draft",
            Self::Experimental => "Experimental",
            Self::Expired => "Expired",
            Self::Final => "Final",
            Self::ForComment => "ForComment",
            Self::ForPublicRelease => "ForPublicRelease",
            Self::NotApproved => "NotApproved",
            Self::NotForPublicRelease => "NotForPublicRelease",
            Self::Sold => "Sold",
            Self::TopSecret => "TopSecret",
        }
    }

    /// Its label's key in the locales.
    #[must_use]
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Approved => "stamps.approved",
            Self::AsIs => "stamps.as_is",
            Self::Confidential => "stamps.confidential",
            Self::Departmental => "stamps.departmental",
            Self::Draft => "stamps.draft",
            Self::Experimental => "stamps.experimental",
            Self::Expired => "stamps.expired",
            Self::Final => "stamps.final",
            Self::ForComment => "stamps.for_comment",
            Self::ForPublicRelease => "stamps.for_public_release",
            Self::NotApproved => "stamps.not_approved",
            Self::NotForPublicRelease => "stamps.not_for_public_release",
            Self::Sold => "stamps.sold",
            Self::TopSecret => "stamps.top_secret",
        }
    }

    /// The colour its border and text are drawn in.
    #[must_use]
    pub fn color(self) -> [f32; 3] {
        match self {
            Self::Approved | Self::Final | Self::ForPublicRelease | Self::Sold => GREEN,
            Self::NotApproved
            | Self::Confidential
            | Self::TopSecret
            | Self::NotForPublicRelease
            | Self::Expired => RED,
            Self::Draft
            | Self::ForComment
            | Self::AsIs
            | Self::Departmental
            | Self::Experimental => BLUE,
        }
    }
}

/// The dynamic stamps: a title, then who placed it and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dynamic {
    /// "APPROVED".
    Approved,
    /// "REVIEWED".
    Reviewed,
    /// "RECEIVED".
    Received,
    /// "REVISED".
    Revised,
    /// "CONFIDENTIAL".
    Confidential,
}

impl Dynamic {
    /// Every dynamic stamp, in the order the picker offers them.
    pub const ALL: [Self; 5] = [
        Self::Approved,
        Self::Reviewed,
        Self::Received,
        Self::Revised,
        Self::Confidential,
    ];

    /// Its name in the PDF: not a standard one, so MuPDF keeps the appearance it is given.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Approved => "DynamicApproved",
            Self::Reviewed => "DynamicReviewed",
            Self::Received => "DynamicReceived",
            Self::Revised => "DynamicRevised",
            Self::Confidential => "DynamicConfidential",
        }
    }

    /// Its title's key in the locales: the word on the stamp and in the picker.
    #[must_use]
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Approved => "stamps.approved",
            Self::Reviewed => "stamps.reviewed",
            Self::Received => "stamps.received",
            Self::Revised => "stamps.revised",
            Self::Confidential => "stamps.confidential",
        }
    }

    /// The colour its border and title are drawn in.
    #[must_use]
    pub fn color(self) -> [f32; 3] {
        match self {
            Self::Approved => GREEN,
            Self::Confidential => RED,
            Self::Reviewed | Self::Received | Self::Revised => BLUE,
        }
    }
}

/// What is filled into a dynamic stamp when it is placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    /// Who placed it.
    pub user: String,
    /// When, in seconds since 1970.
    pub seconds: i64,
    /// On which page, counting from one.
    pub page: usize,
}

impl Fields {
    /// `template` with `{user}`, `{date}`, `{time}` and `{page}` filled in. Date and time are UTC, as A.12's.
    #[must_use]
    pub fn fill(&self, template: &str) -> String {
        let stamped = inspector::date(self.seconds);
        let (date, time) = stamped.split_once(' ').unwrap_or((&stamped, ""));
        template
            .replace("{user}", &self.user)
            .replace("{date}", date)
            .replace("{time}", time)
            .replace("{page}", &self.page.to_string())
    }
}

/// The CSS a dynamic stamp is laid out with.
pub const DYNAMIC_CSS: &str = "body { margin: 0; font-family: sans-serif; }
div { border: 2.5pt solid; border-radius: 6pt; padding: 4pt 8pt; margin: 2pt; }
p { margin: 0; text-align: center; }
p.title { font-size: 17pt; font-weight: bold; letter-spacing: 1pt; }
p.detail { font-size: 8pt; }";

/// A dynamic stamp as HTML: `title` over `detail`, both in `color`.
#[must_use]
pub fn dynamic_html(title: &str, detail: &str, color: [f32; 3]) -> String {
    let [r, g, b] = color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!(
        "<div style=\"color: #{r:02x}{g:02x}{b:02x}; border-color: #{r:02x}{g:02x}{b:02x}\">\
         <p class=\"title\">{}</p><p class=\"detail\">{}</p></div>",
        escape(&title.to_uppercase()),
        escape(detail)
    )
}

/// A dynamic stamp drawn: `title` over `detail` in `color`, laid out by MuPDF [`DYNAMIC_WIDTH`] wide, and its size.
///
/// # Errors
///
/// MuPDF can't lay out the HTML or read back the PDF it wrote.
pub fn dynamic_art(
    title: &str,
    detail: &str,
    color: [f32; 3],
) -> io::Result<(StampArt, (f32, f32))> {
    let pdf = excellent_mupdf::html_to_pdf(
        &dynamic_html(title, detail, color),
        DYNAMIC_CSS,
        DYNAMIC_WIDTH,
    )
    .map_err(io::Error::other)?;
    let bounds = Document::from_bytes(&pdf, "application/pdf")
        .and_then(|document| document.load_page(0)?.bounds())
        .map_err(io::Error::other)?;
    Ok((
        StampArt::Page(pdf),
        (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0),
    ))
}

/// `text` safe inside HTML.
fn escape(text: &str) -> String {
    text.chars()
        .fold(String::with_capacity(text.len()), |mut out, c| {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                _ => out.push(c),
            }
            out
        })
}

/// A new stamp named `name`, `size` big, centred on the click `at` on page `page` and kept inside the page
/// (`bounds`), made smaller to fit when the page is smaller than it.
#[must_use]
pub fn place(
    page: usize,
    at: PagePoint,
    bounds: PageRect,
    size: (f32, f32),
    name: &str,
    color: [f32; 3],
    author: &str,
) -> Annotation {
    let (width, height) = fit(size, bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    let start = |centre: f32, side: f32, low: f32, high: f32| {
        (centre - side / 2.0).min(high - side).max(low)
    };
    let (x0, y0) = (
        start(at.x, width, bounds.x0, bounds.x1),
        start(at.y, height, bounds.y0, bounds.y1),
    );
    let style = Style {
        color: Some(color),
        opacity: 1.0,
        width: 0.0,
    };
    let mut stamp = Annotation::new(
        page,
        Shape::Boxed(
            Boxed::Stamp,
            PageRect {
                x0,
                y0,
                x1: x0 + width,
                y1: y0 + height,
            },
        ),
        style,
        author,
    );
    stamp.icon = name.to_owned();
    stamp
}

/// `size` made smaller, keeping its shape, until it is at most `width` by `height`; never larger.
#[must_use]
pub fn fit(size: (f32, f32), width: f32, height: f32) -> (f32, f32) {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let scale = (width / w).min(height / h).min(1.0);
    (w * scale, h * scale)
}

/// The changes that place `stamp` drawn as `art`: one operation, so one step of undo.
#[must_use]
pub fn changes(stamp: Annotation, art: Option<StampArt>) -> Vec<Change> {
    let (page, id) = (stamp.page, stamp.id.clone());
    let mut changes = vec![Change::CreateAnnotation(stamp)];
    changes.extend(art.map(|art| Change::StampArt { page, id, art }));
    changes
}

/// A stamp in the reader's library: a one-page PDF or an image in the library folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Its name: the file's, without the extension.
    pub name: String,
    /// The file.
    pub path: PathBuf,
}

impl Item {
    /// Whether it is an image rather than a PDF.
    #[must_use]
    pub fn is_image(&self) -> bool {
        is_image(&self.path)
    }

    /// Its name in the PDF: not a standard one, so MuPDF keeps the appearance it is given.
    #[must_use]
    pub fn pdf_name(&self) -> String {
        let kept: String = self
            .name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        format!("Custom{kept}")
    }

    /// What it is drawn as, and its size on the page (at most [`LIBRARY_SIZE`] each way).
    ///
    /// # Errors
    ///
    /// The file can't be read, or MuPDF can't open it.
    pub fn art(&self) -> io::Result<(StampArt, (f32, f32))> {
        let bytes = fs::read(&self.path)?;
        let bounds = Document::open(&self.path)
            .and_then(|document| document.load_page(0)?.bounds())
            .map_err(io::Error::other)?;
        let size = fit(
            (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0),
            LIBRARY_SIZE,
            LIBRARY_SIZE,
        );
        let art = if self.is_image() {
            StampArt::Image(bytes)
        } else {
            StampArt::Page(bytes)
        };
        Ok((art, size))
    }
}

/// Whether `path` names an image the library takes.
fn is_image(path: &Path) -> bool {
    extension(path).is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// `path`'s extension, in lower case.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// The stamps in the library folder `folder`, by name; none when it doesn't exist yet.
#[must_use]
pub fn library(folder: &Path) -> Vec<Item> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut items: Vec<Item> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && (is_image(path) || extension(path).as_deref() == Some("pdf"))
        })
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?.to_owned();
            Some(Item { name, path })
        })
        .collect();
    items.sort_by_key(|item| item.name.to_lowercase());
    items
}

/// Adds `source` to the library folder `folder`: an image is copied, a PDF's first page written as a one-page PDF.
/// Its name is the file's, made safe and kept apart from the stamps there already.
///
/// # Errors
///
/// `source` isn't a PDF or an image the library takes, can't be read or opened, or the folder can't be written.
pub fn import(folder: &Path, source: &Path) -> io::Result<Item> {
    let ext = extension(source).unwrap_or_default();
    let bytes = if is_image(source) {
        fs::read(source)?
    } else if ext == "pdf" {
        Document::open(source)
            .and_then(|document| document.load_page(0)?.to_pdf())
            .map_err(io::Error::other)?
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a stamp is a PDF or an image",
        ));
    };
    fs::create_dir_all(folder)?;
    let stem = safe_name(
        source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default(),
    );
    let name = unique_name(&stem, &library(folder));
    let path = folder.join(format!("{name}.{ext}"));
    let kept = path.with_extension(format!("{ext}.part"));
    fs::write(&kept, bytes)?;
    fs::rename(&kept, &path)?;
    Ok(Item { name, path })
}

/// Takes `item` out of the library.
///
/// # Errors
///
/// The file can't be removed.
pub fn remove(item: &Item) -> io::Result<()> {
    fs::remove_file(&item.path)
}

/// `name` with only letters, digits, spaces, `-` and `_`, trimmed; "Stamp" when nothing is left.
#[must_use]
pub fn safe_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(60)
        .collect();
    let kept = kept.trim();
    if kept.is_empty() {
        "Stamp".to_owned()
    } else {
        kept.to_owned()
    }
}

/// `name`, or `name 2`, `name 3`… the first no stamp in `items` has (ignoring case).
fn unique_name(name: &str, items: &[Item]) -> String {
    let taken = |candidate: &str| {
        items
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(candidate))
    };
    (1..)
        .map(|n| {
            if n == 1 {
                name.to_owned()
            } else {
                format!("{name} {n}")
            }
        })
        .find(|candidate| !taken(candidate))
        .expect("an unused name is found")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    fn rect(annotation: &Annotation) -> PageRect {
        match annotation.shape {
            Shape::Boxed(Boxed::Stamp, rect) => rect,
            _ => panic!("a stamp"),
        }
    }

    #[test]
    fn standard_stamps_are_the_standard_s_fourteen_names() {
        let mut names: Vec<&str> = Standard::ALL.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Approved",
                "AsIs",
                "Confidential",
                "Departmental",
                "Draft",
                "Experimental",
                "Expired",
                "Final",
                "ForComment",
                "ForPublicRelease",
                "NotApproved",
                "NotForPublicRelease",
                "Sold",
                "TopSecret"
            ]
        );
        for dynamic in Dynamic::ALL {
            assert!(
                !names.contains(&dynamic.name()),
                "a dynamic stamp's name is not a standard one"
            );
        }
    }

    #[test]
    fn a_stamp_is_centred_on_the_click_inside_the_page() {
        let stamp = place(
            1,
            PagePoint { x: 300.0, y: 100.0 },
            PAGE,
            STANDARD_SIZE,
            Standard::Draft.name(),
            Standard::Draft.color(),
            "Ada",
        );
        assert_eq!((stamp.page, stamp.icon.as_str()), (1, "Draft"));
        let at = rect(&stamp);
        assert_eq!((at.x0, at.y0, at.x1, at.y1), (224.0, 80.0, 376.0, 120.0));
        let corner = place(
            0,
            PagePoint { x: 610.0, y: 2.0 },
            PAGE,
            STANDARD_SIZE,
            "Draft",
            BLUE,
            "Ada",
        );
        let at = rect(&corner);
        assert_eq!((at.x1, at.y0), (612.0, 0.0), "kept on the page");
        let small = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 76.0,
            y1: 100.0,
        };
        let at = rect(&place(
            0,
            PagePoint { x: 0.0, y: 0.0 },
            small,
            STANDARD_SIZE,
            "Draft",
            BLUE,
            "Ada",
        ));
        assert_eq!(
            (at.x1 - at.x0, at.y1 - at.y0),
            (76.0, 20.0),
            "made smaller to fit"
        );
    }

    #[test]
    fn dynamic_fields_are_filled_when_placed() {
        let fields = Fields {
            user: "Ada <Lovelace>".into(),
            seconds: 1_794_837_900, // 2026-11-16 14:05 UTC
            page: 3,
        };
        let detail = fields.fill("{user}, {date} {time} UTC, page {page}");
        assert_eq!(detail, "Ada <Lovelace>, 2026-11-16 14:05 UTC, page 3");
        let html = dynamic_html("Approved", &detail, GREEN);
        assert!(html.contains(">APPROVED<"), "{html}");
        assert!(html.contains("Ada &lt;Lovelace&gt;"), "escaped: {html}");
        assert!(html.contains("#218c21"), "{html}");
    }

    #[test]
    fn a_dynamic_stamp_is_drawn_as_wide_as_set_and_as_tall_as_its_text() {
        let (art, (width, height)) =
            dynamic_art("Reviewed", "Ada, 2026-11-16 14:05 UTC, page 3", BLUE).expect("art");
        assert!(matches!(art, StampArt::Page(ref pdf) if pdf.starts_with(b"%PDF")));
        assert!((width - DYNAMIC_WIDTH).abs() < 0.5, "{width}");
        assert!(
            height > 20.0 && height < 90.0,
            "a title and a line: {height}"
        );
    }

    #[test]
    fn a_drawn_stamp_is_one_operation_of_two_changes() {
        let stamp = place(
            0,
            PagePoint { x: 100.0, y: 100.0 },
            PAGE,
            (100.0, 50.0),
            "CustomLogo",
            RED,
            "Ada",
        );
        let id = stamp.id.clone();
        let made = changes(stamp.clone(), Some(StampArt::Image(vec![1, 2, 3])));
        assert!(matches!(&made[0], Change::CreateAnnotation(a) if a.id == id));
        assert!(matches!(&made[1], Change::StampArt { id: art, page: 0, .. } if *art == id));
        assert_eq!(
            changes(stamp, None).len(),
            1,
            "a standard stamp is drawn from its name"
        );
    }

    #[test]
    fn library_names_are_safe_and_kept_apart() {
        assert_eq!(safe_name("  Signed: <me>/v2 "), "Signed mev2");
        assert_eq!(safe_name("***"), "Stamp");
        let items = vec![
            Item {
                name: "Logo".into(),
                path: PathBuf::from("Logo.png"),
            },
            Item {
                name: "logo 2".into(),
                path: PathBuf::from("logo 2.pdf"),
            },
        ];
        assert_eq!(unique_name("Logo", &items), "Logo 3");
        assert_eq!(unique_name("Seal", &items), "Seal");
        assert_eq!(items[0].pdf_name(), "CustomLogo");
        assert!(items[0].is_image() && !items[1].is_image());
    }

    #[test]
    fn a_library_stamp_is_imported_listed_drawn_and_removed() {
        let folder = std::env::temp_dir().join(format!("excellent-stamps-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let source = folder.with_extension("pdf");
        let pdf = excellent_mupdf::html_to_pdf("<p>SEAL</p>", "", 120.0).expect("pdf");
        fs::write(&source, pdf).expect("write");
        assert!(library(&folder).is_empty(), "no folder yet");
        let item = import(&folder, &source).expect("import");
        let again = import(&folder, &source).expect("import");
        assert_ne!(item.name, again.name, "kept apart");
        assert_eq!(library(&folder).len(), 2);
        let (art, (width, _)) = item.art().expect("art");
        assert!(matches!(art, StampArt::Page(_)));
        assert!((width - 120.0).abs() < 0.5, "{width}");
        remove(&item).expect("remove");
        assert_eq!(library(&folder), vec![again]);
        let _ = fs::remove_dir_all(&folder);
        let _ = fs::remove_file(&source);
        assert!(import(&folder, Path::new("notes.txt")).is_err());
    }
}
