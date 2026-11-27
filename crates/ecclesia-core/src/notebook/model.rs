//! Notebook metadata schema and types for `.expdf` archives (plan N.4).

use std::fmt;
use std::io;

use ecclesia_mupdf::{MuError, PagePoint, PageRect};
use serde_json::{Map, Value, json};

use crate::anchor::new_id;

/// Current `.expdf` `notebook.json` schema version.
pub const FORMAT_VERSION: u32 = 1;

/// Errors returned by notebook creation, loading, editing, and PDF export.
#[derive(Debug)]
pub enum NotebookError {
    /// Underlying filesystem or byte stream I/O failure.
    Io(io::Error),
    /// ZIP archive reading or writing failure.
    Zip(zip::result::ZipError),
    /// JSON parsing or serialization error.
    Json(serde_json::Error),
    /// MuPDF failed to open or validate the embedded or exported PDF.
    MuPdf(MuError),
    /// The archive or metadata is malformed or uses an unsupported version.
    InvalidFormat(String),
    /// A requested page, section, or asset does not exist.
    NotFound(String),
    /// The saved file failed read-back verification.
    Unverified(String),
}

impl fmt::Display for NotebookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "notebook i/o error: {err}"),
            Self::Zip(err) => write!(f, "notebook zip error: {err}"),
            Self::Json(err) => write!(f, "notebook json error: {err}"),
            Self::MuPdf(err) => write!(f, "notebook pdf error: {err}"),
            Self::InvalidFormat(msg) => write!(f, "invalid notebook format: {msg}"),
            Self::NotFound(msg) => write!(f, "notebook item not found: {msg}"),
            Self::Unverified(msg) => write!(f, "notebook save verification failed: {msg}"),
        }
    }
}

impl std::error::Error for NotebookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Zip(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::MuPdf(err) => Some(err),
            Self::InvalidFormat(_) | Self::NotFound(_) | Self::Unverified(_) => None,
        }
    }
}

impl From<io::Error> for NotebookError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<zip::result::ZipError> for NotebookError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Zip(err)
    }
}

impl From<serde_json::Error> for NotebookError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<MuError> for NotebookError {
    fn from(err: MuError) -> Self {
        Self::MuPdf(err)
    }
}

/// Convenience result alias for notebook operations.
pub type Result<T> = std::result::Result<T, NotebookError>;

/// Page orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Orientation {
    /// Taller than wide.
    #[default]
    Portrait,
    /// Wider than tall.
    Landscape,
}

impl Orientation {
    /// Serialization identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    /// Parses an orientation identifier.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "portrait" => Some(Self::Portrait),
            "landscape" => Some(Self::Landscape),
            _ => None,
        }
    }
}

/// Standard and custom notebook page sizes in PDF points (1 pt = 1/72 inch).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PageSize {
    /// ISO A4 (595.28 x 841.89 pt).
    #[default]
    A4,
    /// US Letter (612 x 792 pt).
    Letter,
    /// ISO A5 (419.53 x 595.28 pt).
    A5,
    /// Phone / pocket reading size (360 x 640 pt).
    Phone,
    /// Custom dimensions in points.
    Custom {
        /// Width in points.
        width_pt: f32,
        /// Height in points.
        height_pt: f32,
    },
}

impl PageSize {
    /// Returns `(width_pt, height_pt)` respecting `orientation`.
    #[must_use]
    pub fn dimensions(self, orientation: Orientation) -> (f32, f32) {
        let (w, h) = match self {
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612.0, 792.0),
            Self::A5 => (419.53, 595.28),
            Self::Phone => (360.0, 640.0),
            Self::Custom {
                width_pt,
                height_pt,
            } => (width_pt.max(72.0), height_pt.max(72.0)),
        };
        match orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Self::A4 => json!({"kind": "a4"}),
            Self::Letter => json!({"kind": "letter"}),
            Self::A5 => json!({"kind": "a5"}),
            Self::Phone => json!({"kind": "phone"}),
            Self::Custom {
                width_pt,
                height_pt,
            } => json!({"kind": "custom", "width_pt": width_pt, "height_pt": height_pt}),
        }
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        match obj.get("kind")?.as_str()? {
            "a4" => Some(Self::A4),
            "letter" => Some(Self::Letter),
            "a5" => Some(Self::A5),
            "phone" => Some(Self::Phone),
            "custom" => Some(Self::Custom {
                width_pt: obj.get("width_pt")?.as_f64()? as f32,
                height_pt: obj.get("height_pt")?.as_f64()? as f32,
            }),
            _ => None,
        }
    }
}

/// Identifier of a procedural paper template (plan N.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaperTemplateId {
    /// Plain blank paper.
    #[default]
    Blank,
    /// Standard wide-ruled lined paper.
    Lined,
    /// College-ruled lined paper with left margin rule.
    CollegeRuled,
    /// Square grid paper.
    Grid,
    /// Dot grid paper.
    Dot,
    /// Cornell notes layout (cue column, notes area, summary footer).
    Cornell,
    /// Music staff paper (5-line staves).
    Music,
    /// Engineering graph paper (minor + emphasized major grid).
    Graph,
    /// Isometric 30-degree triangular grid.
    Isometric,
    /// Daily / weekly planner layout.
    Planner,
}

impl PaperTemplateId {
    /// All built-in procedural paper templates.
    pub const ALL: [Self; 10] = [
        Self::Blank,
        Self::Lined,
        Self::CollegeRuled,
        Self::Grid,
        Self::Dot,
        Self::Cornell,
        Self::Music,
        Self::Graph,
        Self::Isometric,
        Self::Planner,
    ];

    /// Stable identifier stored in `notebook.json` and sidecars.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blank => "blank",
            Self::Lined => "lined",
            Self::CollegeRuled => "college-ruled",
            Self::Grid => "grid",
            Self::Dot => "dot",
            Self::Cornell => "cornell",
            Self::Music => "music",
            Self::Graph => "graph",
            Self::Isometric => "isometric",
            Self::Planner => "planner",
        }
    }

    /// Parses a template identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == id)
    }
}

/// Whether a notebook page has a fixed paper size or an unbounded infinite canvas (N.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PageKind {
    /// Fixed-size page (A4, Letter, etc.).
    #[default]
    Fixed,
    /// Unbounded infinite canvas / whiteboard page.
    Infinite,
}

impl PageKind {
    /// Serialization identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Infinite => "infinite",
        }
    }

    /// Parses a page kind identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "fixed" => Some(Self::Fixed),
            "infinite" => Some(Self::Infinite),
            _ => None,
        }
    }
}

/// Original PDF MediaBox and CropBox recorded prior to margin expansion (N.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OriginalBoxes {
    /// Original PDF MediaBox (`[x0, y0, x1, y1]` in PDF user space).
    pub media_box: [f32; 4],
    /// Original PDF CropBox (`[x0, y0, x1, y1]` in PDF user space).
    pub crop_box: [f32; 4],
}

impl OriginalBoxes {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(self) -> Value {
        json!({
            "media_box": self.media_box,
            "crop_box": self.crop_box,
        })
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        Some(Self {
            media_box: rect4_from_json(obj.get("media_box")?)?,
            crop_box: rect4_from_json(obj.get("crop_box")?)?,
        })
    }
}

/// Per-page metadata stored in `notebook.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct PageMeta {
    /// Stable page UUID.
    pub id: String,
    /// Procedural paper template applied under this page.
    pub template: PaperTemplateId,
    /// Fixed vs. infinite canvas page.
    pub kind: PageKind,
    /// Optional section identifier owning this page.
    pub section_id: Option<String>,
    /// Original boxes saved before margin expansion (N.3).
    pub original_boxes: Option<OriginalBoxes>,
    /// Optional rich structured sidecar payload (e.g., typed text blocks N.6).
    pub sidecar: Option<Value>,
}

impl PageMeta {
    /// Creates metadata for a standard fixed page with the given template.
    #[must_use]
    pub fn new(template: PaperTemplateId) -> Self {
        Self {
            id: new_id(),
            template,
            kind: PageKind::Fixed,
            section_id: None,
            original_boxes: None,
            sidecar: None,
        }
    }

    /// Creates metadata for an infinite canvas page.
    #[must_use]
    pub fn new_infinite(template: PaperTemplateId) -> Self {
        Self {
            id: new_id(),
            template,
            kind: PageKind::Infinite,
            section_id: None,
            original_boxes: None,
            sidecar: None,
        }
    }

    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("template".into(), json!(self.template.as_str()));
        map.insert("kind".into(), json!(self.kind.as_str()));
        if let Some(ref section_id) = self.section_id {
            map.insert("section_id".into(), json!(section_id));
        }
        if let Some(boxes) = self.original_boxes {
            map.insert("original_boxes".into(), boxes.to_json());
        }
        if let Some(ref sidecar) = self.sidecar {
            map.insert("sidecar".into(), sidecar.clone());
        }
        Value::Object(map)
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let id = obj.get("id")?.as_str()?.to_owned();
        let template = obj
            .get("template")
            .and_then(Value::as_str)
            .and_then(PaperTemplateId::parse)
            .unwrap_or_default();
        let kind = obj
            .get("kind")
            .and_then(Value::as_str)
            .and_then(PageKind::parse)
            .unwrap_or_default();
        let section_id = obj
            .get("section_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let original_boxes = obj.get("original_boxes").and_then(OriginalBoxes::from_json);
        let sidecar = obj.get("sidecar").cloned();
        Some(Self {
            id,
            template,
            kind,
            section_id,
            original_boxes,
            sidecar,
        })
    }
}

/// Built-in cover styles (N.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CoverStyle {
    /// Clean minimal cover with bold title.
    #[default]
    Classic,
    /// Clothbound notebook look with spine band.
    Linen,
    /// Technical/grid cover with corner accent.
    Blueprint,
    /// Custom user image cover stored in `assets/`.
    Image,
}

impl CoverStyle {
    /// Serialization identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Linen => "linen",
            Self::Blueprint => "blueprint",
            Self::Image => "image",
        }
    }

    /// Parses a cover style identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "classic" => Some(Self::Classic),
            "linen" => Some(Self::Linen),
            "blueprint" => Some(Self::Blueprint),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// Notebook cover configuration (N.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverMeta {
    /// Cover visual style.
    pub style: CoverStyle,
    /// Cover title text.
    pub title: String,
    /// Optional subtitle text.
    pub subtitle: Option<String>,
    /// Accent RGB color `[r, g, b]`.
    pub accent_rgb: [u8; 3],
    /// Optional image asset path inside `assets/` when `style == CoverStyle::Image`.
    pub image_asset: Option<String>,
}

impl CoverMeta {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("style".into(), json!(self.style.as_str()));
        map.insert("title".into(), json!(self.title));
        if let Some(ref sub) = self.subtitle {
            map.insert("subtitle".into(), json!(sub));
        }
        map.insert("accent_rgb".into(), json!(self.accent_rgb));
        if let Some(ref asset) = self.image_asset {
            map.insert("image_asset".into(), json!(asset));
        }
        Value::Object(map)
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let style = CoverStyle::parse(obj.get("style")?.as_str()?)?;
        let title = obj.get("title")?.as_str()?.to_owned();
        let subtitle = obj
            .get("subtitle")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let accent_rgb = rgb_from_json(obj.get("accent_rgb")?)?;
        let image_asset = obj
            .get("image_asset")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(Self {
            style,
            title,
            subtitle,
            accent_rgb,
            image_asset,
        })
    }
}

/// A named colored section in a notebook (N.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMeta {
    /// Stable section UUID.
    pub id: String,
    /// Section display title (exported as a PDF bookmark).
    pub title: String,
    /// Tab strip color `[r, g, b]`.
    pub color: [u8; 3],
    /// Zero-based first page index belonging to this section.
    pub start_page: usize,
    /// Number of consecutive pages in this section.
    pub page_count: usize,
}

impl SectionMeta {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "color": self.color,
            "start_page": self.start_page,
            "page_count": self.page_count,
        })
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        Some(Self {
            id: obj.get("id")?.as_str()?.to_owned(),
            title: obj.get("title")?.as_str()?.to_owned(),
            color: rgb_from_json(obj.get("color")?)?,
            start_page: usize::try_from(obj.get("start_page")?.as_u64()?).ok()?,
            page_count: usize::try_from(obj.get("page_count")?.as_u64()?).ok()?,
        })
    }
}

/// Destination of a notebook link (N.9).
#[derive(Debug, Clone, PartialEq)]
pub enum LinkDestination {
    /// Internal link to another page in the same notebook.
    Page {
        /// Zero-based destination page index.
        page: usize,
        /// Optional target point on the destination page.
        point: Option<PagePoint>,
        /// Optional zoom factor (`1.0` = 100%).
        zoom: Option<f32>,
    },
    /// Deep link to another document or external URI (`ecclesiapdf://...`).
    Uri(String),
}

/// A clickable link region on a notebook page (N.9).
#[derive(Debug, Clone, PartialEq)]
pub struct PageLinkMeta {
    /// Stable link UUID.
    pub id: String,
    /// Zero-based page index where the link hotspot sits.
    pub source_page: usize,
    /// Hotspot rectangle in page points.
    pub rect: PageRect,
    /// Optional human-readable label.
    pub label: Option<String>,
    /// Where the link navigates.
    pub destination: LinkDestination,
}

impl PageLinkMeta {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let dest = match &self.destination {
            LinkDestination::Page { page, point, zoom } => json!({
                "kind": "page",
                "page": page,
                "point": point.map(|p| [p.x, p.y]),
                "zoom": zoom,
            }),
            LinkDestination::Uri(uri) => json!({
                "kind": "uri",
                "uri": uri,
            }),
        };
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("source_page".into(), json!(self.source_page));
        map.insert(
            "rect".into(),
            json!([self.rect.x0, self.rect.y0, self.rect.x1, self.rect.y1]),
        );
        if let Some(ref label) = self.label {
            map.insert("label".into(), json!(label));
        }
        map.insert("destination".into(), dest);
        Value::Object(map)
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let rect = rect4_from_json(obj.get("rect")?)?;
        let dest_obj = obj.get("destination")?.as_object()?;
        let destination = match dest_obj.get("kind")?.as_str()? {
            "page" => {
                let page = usize::try_from(dest_obj.get("page")?.as_u64()?).ok()?;
                let point = dest_obj.get("point").and_then(|v| {
                    let arr = v.as_array()?;
                    Some(PagePoint {
                        x: arr.first()?.as_f64()? as f32,
                        y: arr.get(1)?.as_f64()? as f32,
                    })
                });
                let zoom = dest_obj
                    .get("zoom")
                    .and_then(Value::as_f64)
                    .map(|z| z as f32);
                LinkDestination::Page { page, point, zoom }
            }
            "uri" => LinkDestination::Uri(dest_obj.get("uri")?.as_str()?.to_owned()),
            _ => return None,
        };
        Some(Self {
            id: obj.get("id")?.as_str()?.to_owned(),
            source_page: usize::try_from(obj.get("source_page")?.as_u64()?).ok()?,
            rect: PageRect {
                x0: rect[0],
                y0: rect[1],
                x1: rect[2],
                y1: rect[3],
            },
            label: obj.get("label").and_then(Value::as_str).map(str::to_owned),
            destination,
        })
    }
}

/// Container/codec format of a bundled audio asset (L.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    /// Uncompressed 16-bit PCM WAV (`audio/wav`).
    Wav,
    /// Opus inside an Ogg container (`audio/ogg`).
    OggOpus,
}

impl AudioFormat {
    /// Serialization identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::OggOpus => "ogg-opus",
        }
    }

    /// Standard file extension without leading dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::OggOpus => "opus",
        }
    }

    /// Parses an audio format identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "wav" => Some(Self::Wav),
            "ogg-opus" | "opus" => Some(Self::OggOpus),
            _ => None,
        }
    }
}

/// Metadata referencing an audio recording bundled in `assets/` (L.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioRecordingMeta {
    /// Stable recording UUID.
    pub id: String,
    /// Asset path within the `.expdf` archive (e.g., `assets/audio/<id>.opus`).
    pub asset_path: String,
    /// Audio container/codec format.
    pub format: AudioFormat,
    /// Recording start timestamp in milliseconds on the ink sample clock.
    pub started_at_ms: f64,
    /// Total duration in milliseconds.
    pub duration_ms: f64,
    /// Sample rate in Hz (e.g., 48000).
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Optional title or lecture label.
    pub title: Option<String>,
}

impl AudioRecordingMeta {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("asset_path".into(), json!(self.asset_path));
        map.insert("format".into(), json!(self.format.as_str()));
        map.insert("started_at_ms".into(), json!(self.started_at_ms));
        map.insert("duration_ms".into(), json!(self.duration_ms));
        map.insert("sample_rate".into(), json!(self.sample_rate));
        map.insert("channels".into(), json!(self.channels));
        if let Some(ref title) = self.title {
            map.insert("title".into(), json!(title));
        }
        Value::Object(map)
    }

    /// Deserializes from JSON.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        Some(Self {
            id: obj.get("id")?.as_str()?.to_owned(),
            asset_path: obj.get("asset_path")?.as_str()?.to_owned(),
            format: AudioFormat::parse(obj.get("format")?.as_str()?)?,
            started_at_ms: obj.get("started_at_ms")?.as_f64()?,
            duration_ms: obj.get("duration_ms")?.as_f64()?,
            sample_rate: u32::try_from(obj.get("sample_rate")?.as_u64()?).ok()?,
            channels: u16::try_from(obj.get("channels")?.as_u64()?).ok()?,
            title: obj.get("title").and_then(Value::as_str).map(str::to_owned),
        })
    }
}

/// Top-level contents of `notebook.json` inside an `.expdf` archive.
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookMetadata {
    /// Format schema version (currently [`FORMAT_VERSION`]).
    pub version: u32,
    /// Stable notebook UUID.
    pub id: String,
    /// Notebook display title.
    pub title: String,
    /// Default page size for newly inserted pages.
    pub default_page_size: PageSize,
    /// Default orientation for newly inserted pages.
    pub default_orientation: Orientation,
    /// Default procedural template for newly inserted pages.
    pub default_template: PaperTemplateId,
    /// Optional notebook cover configuration (N.8).
    pub cover: Option<CoverMeta>,
    /// Ordered per-page metadata, aligned 1:1 with `document.pdf` pages.
    pub pages: Vec<PageMeta>,
    /// Named colored sections (N.8).
    pub sections: Vec<SectionMeta>,
    /// Internal and deep links (N.9).
    pub links: Vec<PageLinkMeta>,
    /// Bundled audio recording references (L.1).
    pub audio: Vec<AudioRecordingMeta>,
}

impl NotebookMetadata {
    /// Creates metadata for a new notebook with a single page.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        page_size: PageSize,
        orientation: Orientation,
        template: PaperTemplateId,
    ) -> Self {
        Self {
            version: FORMAT_VERSION,
            id: new_id(),
            title: title.into(),
            default_page_size: page_size,
            default_orientation: orientation,
            default_template: template,
            cover: None,
            pages: vec![PageMeta::new(template)],
            sections: Vec::new(),
            links: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// Serializes metadata to pretty-printed JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::Json`] if JSON serialization fails.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        let mut map = Map::new();
        map.insert("version".into(), json!(self.version));
        map.insert("id".into(), json!(self.id));
        map.insert("title".into(), json!(self.title));
        map.insert("default_page_size".into(), self.default_page_size.to_json());
        map.insert(
            "default_orientation".into(),
            json!(self.default_orientation.as_str()),
        );
        map.insert(
            "default_template".into(),
            json!(self.default_template.as_str()),
        );
        if let Some(ref cover) = self.cover {
            map.insert("cover".into(), cover.to_json());
        }
        map.insert(
            "pages".into(),
            Value::Array(self.pages.iter().map(PageMeta::to_json).collect()),
        );
        map.insert(
            "sections".into(),
            Value::Array(self.sections.iter().map(SectionMeta::to_json).collect()),
        );
        map.insert(
            "links".into(),
            Value::Array(self.links.iter().map(PageLinkMeta::to_json).collect()),
        );
        map.insert(
            "audio".into(),
            Value::Array(self.audio.iter().map(AudioRecordingMeta::to_json).collect()),
        );
        Ok(serde_json::to_vec_pretty(&Value::Object(map))?)
    }

    /// Parses metadata from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::Json`] or [`NotebookError::InvalidFormat`] on malformed or unsupported input.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        let obj = value.as_object().ok_or_else(|| {
            NotebookError::InvalidFormat("notebook.json root must be an object".into())
        })?;
        let version = obj
            .get("version")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| NotebookError::InvalidFormat("missing or invalid version".into()))?;
        if version != FORMAT_VERSION {
            return Err(NotebookError::InvalidFormat(format!(
                "unsupported notebook version {version} (expected {FORMAT_VERSION})"
            )));
        }
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| NotebookError::InvalidFormat("missing notebook id".into()))?
            .to_owned();
        let title = obj
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled")
            .to_owned();
        let default_page_size = obj
            .get("default_page_size")
            .and_then(PageSize::from_json)
            .unwrap_or_default();
        let default_orientation = obj
            .get("default_orientation")
            .and_then(Value::as_str)
            .and_then(Orientation::parse)
            .unwrap_or_default();
        let default_template = obj
            .get("default_template")
            .and_then(Value::as_str)
            .and_then(PaperTemplateId::parse)
            .unwrap_or_default();
        let cover = obj.get("cover").and_then(CoverMeta::from_json);
        let pages = obj
            .get("pages")
            .and_then(Value::as_array)
            .ok_or_else(|| NotebookError::InvalidFormat("missing pages array".into()))?
            .iter()
            .map(PageMeta::from_json)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| NotebookError::InvalidFormat("invalid page metadata entry".into()))?;
        let sections = match obj.get("sections").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .map(SectionMeta::from_json)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    NotebookError::InvalidFormat("invalid section metadata entry".into())
                })?,
            None => Vec::new(),
        };
        let links = match obj.get("links").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .map(PageLinkMeta::from_json)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    NotebookError::InvalidFormat("invalid link metadata entry".into())
                })?,
            None => Vec::new(),
        };
        let audio = match obj.get("audio").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .map(AudioRecordingMeta::from_json)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    NotebookError::InvalidFormat("invalid audio metadata entry".into())
                })?,
            None => Vec::new(),
        };

        Ok(Self {
            version,
            id,
            title,
            default_page_size,
            default_orientation,
            default_template,
            cover,
            pages,
            sections,
            links,
            audio,
        })
    }
}

fn rect4_from_json(value: &Value) -> Option<[f32; 4]> {
    let arr = value.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    Some([
        arr[0].as_f64()? as f32,
        arr[1].as_f64()? as f32,
        arr[2].as_f64()? as f32,
        arr[3].as_f64()? as f32,
    ])
}

fn rgb_from_json(value: &Value) -> Option<[u8; 3]> {
    let arr = value.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    Some([
        u8::try_from(arr[0].as_u64()?).ok()?,
        u8::try_from(arr[1].as_u64()?).ok()?,
        u8::try_from(arr[2].as_u64()?).ok()?,
    ])
}
