//! Flashcards (plan S.1), image occlusion (S.2) and cloze deletions (S.3).
//!
//! Cards are pure Rust models backed by `excellent_store::study`. Every card created from a document keeps a K.28
//! [`SourceAnchor`] so the review screen can jump straight back to its source page and re-anchor on re-typeset PDFs.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use excellent_core::anchor::{self, Placement, Quote};
use excellent_core::annot::{Annotation, Shape};
use excellent_mupdf::PageRect;
use excellent_store::study::CardRecord;
use serde_json::{Value, json};

use crate::annotate::edit::bounds;

/// Normalized rectangle in `[0.0, 1.0]` image coordinates used for image-occlusion masks (plan S.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedRect {
    /// Left edge (`0.0..=1.0`).
    pub left: f32,
    /// Top edge (`0.0..=1.0`).
    pub top: f32,
    /// Width (`0.0..=1.0`).
    pub width: f32,
    /// Height (`0.0..=1.0`).
    pub height: f32,
}

impl NormalizedRect {
    /// Constructs a validated normalized rectangle clamped to `[0.0, 1.0]`; returns `None` when non-positive or
    /// non-finite.
    #[must_use]
    pub fn try_new(left: f32, top: f32, width: f32, height: f32) -> Option<Self> {
        if !(left.is_finite() && top.is_finite() && width.is_finite() && height.is_finite()) {
            return None;
        }
        let left = left.clamp(0.0, 1.0);
        let top = top.clamp(0.0, 1.0);
        let width = width.clamp(0.0, 1.0 - left);
        let height = height.clamp(0.0, 1.0 - top);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(Self {
            left,
            top,
            width,
            height,
        })
    }

    fn to_json(self) -> Value {
        json!({
            "left": self.left,
            "top": self.top,
            "width": self.width,
            "height": self.height,
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        Self::try_new(
            value.get("left")?.as_f64()? as f32,
            value.get("top")?.as_f64()? as f32,
            value.get("width")?.as_f64()? as f32,
            value.get("height")?.as_f64()? as f32,
        )
    }
}

/// Masking behaviour for an image-occlusion card set (plan S.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OcclusionMode {
    /// Hides only the tested box; other boxes stay revealed.
    HideOneGuessOne,
    /// Hides every box on the figure and highlights the one being tested ("hide all, guess one").
    #[default]
    HideAllGuessOne,
}

impl OcclusionMode {
    /// Serialized key stored in SQLite and Anki metadata.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::HideOneGuessOne => "hide_one_guess_one",
            Self::HideAllGuessOne => "hide_all_guess_one",
        }
    }

    /// Parses a serialized key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "hide_one_guess_one" => Some(Self::HideOneGuessOne),
            "hide_all_guess_one" => Some(Self::HideAllGuessOne),
            _ => None,
        }
    }
}

/// One labelled occlusion box drawn over a figure (plan S.2).
#[derive(Debug, Clone, PartialEq)]
pub struct OcclusionBox {
    /// Stable box identifier within the figure.
    pub id: String,
    /// Normalized box bounds on the source image.
    pub rect: NormalizedRect,
    /// Optional answer label or hint shown when revealed.
    pub label: Option<String>,
}

/// Full specification of an image-occlusion figure shared by its generated cards (plan S.2).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageOcclusionSpec {
    /// File name of the figure image in [`Flashcard::media`].
    pub image_name: String,
    /// All occlusion boxes drawn over the figure, in creation order.
    pub boxes: Vec<OcclusionBox>,
    /// Whether non-active boxes are hidden too.
    pub mode: OcclusionMode,
}

impl ImageOcclusionSpec {
    /// Indices of the boxes that are masked on the question side when testing `active_index`.
    #[must_use]
    pub fn masked_indices(&self, active_index: usize) -> Vec<usize> {
        if active_index >= self.boxes.len() {
            return Vec::new();
        }
        match self.mode {
            OcclusionMode::HideOneGuessOne => vec![active_index],
            OcclusionMode::HideAllGuessOne => (0..self.boxes.len()).collect(),
        }
    }

    /// Serializes the occlusion specification and active box index to JSON.
    #[must_use]
    pub fn to_json(&self, active_box_index: usize) -> String {
        let boxes: Vec<Value> = self
            .boxes
            .iter()
            .map(|b| {
                json!({
                    "id": b.id,
                    "rect": b.rect.to_json(),
                    "label": b.label,
                })
            })
            .collect();
        json!({
            "image_name": self.image_name,
            "mode": self.mode.key(),
            "active_box_index": active_box_index,
            "boxes": boxes,
        })
        .to_string()
    }

    /// Parses a serialized occlusion specification and active box index from JSON.
    #[must_use]
    pub fn from_json(raw: &str) -> Option<(Self, usize)> {
        let value: Value = serde_json::from_str(raw).ok()?;
        let image_name = value.get("image_name")?.as_str()?.to_owned();
        let mode = OcclusionMode::from_key(value.get("mode")?.as_str()?)?;
        let active_box_index = usize::try_from(value.get("active_box_index")?.as_u64()?).ok()?;
        let boxes = value
            .get("boxes")?
            .as_array()?
            .iter()
            .map(|item| {
                Some(OcclusionBox {
                    id: item.get("id")?.as_str()?.to_owned(),
                    rect: NormalizedRect::from_json(item.get("rect")?)?,
                    label: item.get("label").and_then(Value::as_str).map(str::to_owned),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some((
            Self {
                image_name,
                boxes,
                mode,
            },
            active_box_index,
        ))
    }
}

/// Binary media asset attached to a flashcard (e.g. a lasso region PNG or occlusion figure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardMedia {
    /// File name referenced by the card (e.g. `"lasso-p3.png"`).
    pub name: String,
    /// MIME type (e.g. `"image/png"`).
    pub mime: String,
    /// Raw asset bytes.
    pub bytes: Vec<u8>,
}

/// Link back from a card to the exact document page, K.28 anchor and region it came from (plan S.1).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAnchor {
    /// Source document path.
    pub path: PathBuf,
    /// Zero-based page number.
    pub page: usize,
    /// K.28 text-quote anchor when made from text markup or a text selection.
    pub quote: Option<Quote>,
    /// Region rectangle in page points (for highlight bounds, lasso crops or figure crops).
    pub rect: Option<PageRect>,
    /// Originating annotation UUID (`/NM`), if created from an annotation.
    pub annot_id: Option<String>,
}

/// Resolved navigation target when jumping from a review card back to its source document (plan S.1).
#[derive(Debug, Clone, PartialEq)]
pub struct JumpTarget {
    /// Document path to open or focus.
    pub path: PathBuf,
    /// Zero-based page index to jump to.
    pub page: usize,
    /// Optional page-point region to centre/highlight.
    pub rect: Option<PageRect>,
    /// Optional K.28 re-anchored character range when document text was provided.
    pub placement: Option<Placement>,
}

impl SourceAnchor {
    /// Builds a jump-to-page target, optionally re-anchoring the K.28 quote against `document_text`.
    #[must_use]
    pub fn jump_target(&self, document_text: Option<&str>) -> JumpTarget {
        let placement = match (&self.quote, document_text) {
            (Some(quote), Some(text)) => Some(anchor::locate(quote, text)),
            _ => None,
        };
        JumpTarget {
            path: self.path.clone(),
            page: self.page,
            rect: self.rect,
            placement,
        }
    }

    /// Formats a portable `file://...#page=<1-based>` source link for Anki and study exports.
    #[must_use]
    pub fn to_uri(&self) -> String {
        let raw = self.path.to_string_lossy();
        let mut out = String::with_capacity(raw.len() + 24);
        if raw.starts_with("\\\\") {
            out.push_str("file:");
        } else {
            out.push_str("file:///");
        }
        for byte in raw.as_bytes() {
            let b = if *byte == b'\\' { b'/' } else { *byte };
            if b.is_ascii_alphanumeric() || matches!(b, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out.push_str(&format!("#page={}", self.page + 1));
        out
    }

    /// Parses a `file://...#page=<1-based>` source link back into a [`SourceAnchor`].
    #[must_use]
    pub fn from_uri(uri: &str) -> Option<Self> {
        let (base, fragment) = uri.split_once("#page=")?;
        let one_based: usize = fragment.trim().parse().ok()?;
        let page = one_based.checked_sub(1)?;
        let stripped = base
            .strip_prefix("file:///")
            .or_else(|| base.strip_prefix("file:"))?;
        let decoded = percent_decode(stripped)?;
        Some(Self {
            path: PathBuf::from(decoded),
            page,
            quote: None,
            rect: None,
            annot_id: None,
        })
    }

    /// Serializes the anchor details to JSON for `study_card.anchor_json`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let quote = self.quote.as_ref().map(|q| {
            json!({
                "prefix": q.prefix,
                "exact": q.exact,
                "suffix": q.suffix,
                "start": q.start,
            })
        });
        let rect = self.rect.map(|r| {
            json!({
                "x0": r.x0,
                "y0": r.y0,
                "x1": r.x1,
                "y1": r.y1,
            })
        });
        json!({
            "quote": quote,
            "rect": rect,
            "annot_id": self.annot_id,
        })
        .to_string()
    }

    /// Deserializes from `source_path`, `source_page` and `anchor_json`.
    #[must_use]
    pub fn from_stored(path: &str, page: usize, anchor_json: Option<&str>) -> Self {
        let mut quote = None;
        let mut rect = None;
        let mut annot_id = None;
        if let Some(raw) = anchor_json
            && let Ok(val) = serde_json::from_str::<Value>(raw)
        {
            if let Some(q) = val.get("quote").filter(|v| v.is_object())
                && let (Some(prefix), Some(exact), Some(suffix), Some(start)) = (
                    q.get("prefix").and_then(Value::as_str),
                    q.get("exact").and_then(Value::as_str),
                    q.get("suffix").and_then(Value::as_str),
                    q.get("start")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok()),
                )
            {
                quote = Some(Quote {
                    prefix: prefix.to_owned(),
                    exact: exact.to_owned(),
                    suffix: suffix.to_owned(),
                    start,
                });
            }
            if let Some(r) = val.get("rect").filter(|v| v.is_object())
                && let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                    r.get("x0").and_then(Value::as_f64),
                    r.get("y0").and_then(Value::as_f64),
                    r.get("x1").and_then(Value::as_f64),
                    r.get("y1").and_then(Value::as_f64),
                )
            {
                rect = Some(PageRect {
                    x0: x0 as f32,
                    y0: y0 as f32,
                    x1: x1 as f32,
                    y1: y1 as f32,
                });
            }
            annot_id = val
                .get("annot_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Self {
            path: PathBuf::from(path),
            page,
            quote,
            rect,
            annot_id,
        }
    }
}

/// Kind of study card (plan S.1, S.2, S.3).
#[derive(Debug, Clone, PartialEq)]
pub enum CardKind {
    /// Standard front/back flashcard (from highlight, note, lasso or manual entry).
    Basic,
    /// Cloze deletion card (`{{c1::...}}`) testing one ordinal.
    Cloze {
        /// 1-based Anki cloze ordinal (`c1`, `c2`, …).
        ordinal: u32,
    },
    /// Image-occlusion card masking one or all boxes on a figure.
    ImageOcclusion {
        /// Occlusion figure definition.
        spec: ImageOcclusionSpec,
        /// Index into `spec.boxes` of the box the learner must recall.
        active_box_index: usize,
    },
}

/// Shared metadata applied to every card generated by [`Flashcard::from_image_occlusion`].
#[derive(Debug, Clone)]
pub struct OcclusionCardMeta<'a> {
    /// Prompt header displayed above the occluded figure.
    pub header: &'a str,
    /// Optional extra note appended on the answer side.
    pub back_extra: &'a str,
    /// Optional source document anchor.
    pub source: Option<SourceAnchor>,
    /// Owning deck identifier.
    pub deck_id: &'a str,
    /// Tags applied to every generated card.
    pub tags: &'a [String],
    /// Creation timestamp (UTC seconds since 1970).
    pub now: i64,
}

/// A study flashcard model (plan S.1–S.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Flashcard {
    /// Stable UUID.
    pub id: String,
    /// Owning deck identifier.
    pub deck_id: String,
    /// Card type and type-specific data.
    pub kind: CardKind,
    /// Question prompt (or cloze template text for [`CardKind::Cloze`]).
    pub front: String,
    /// Answer text (or extra note shown on reveal for cloze/occlusion cards).
    pub back: String,
    /// Link back to the document page and K.28 anchor.
    pub source: Option<SourceAnchor>,
    /// User tags.
    pub tags: Vec<String>,
    /// Attached images or audio assets.
    pub media: Vec<CardMedia>,
    /// Creation timestamp (UTC seconds since 1970).
    pub created: i64,
    /// Last modification timestamp (UTC seconds since 1970).
    pub modified: i64,
}

impl Flashcard {
    /// Creates a basic card from a PDF highlight annotation (front = highlighted text, back = note).
    ///
    /// Returns `None` if both the highlight text and note are empty.
    #[must_use]
    pub fn from_highlight(
        annotation: &Annotation,
        fallback_text: &str,
        source_path: &Path,
        deck_id: &str,
        now: i64,
    ) -> Option<Self> {
        if !matches!(annotation.shape, Shape::Markup(..)) {
            return None;
        }
        let quoted = annotation
            .anchor
            .as_ref()
            .map(|q| q.exact.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback_text)
            .trim();
        let note = annotation.contents.trim();
        if quoted.is_empty() && note.is_empty() {
            return None;
        }
        let front = if quoted.is_empty() {
            note.to_owned()
        } else {
            quoted.to_owned()
        };
        let back = if quoted.is_empty() {
            String::new()
        } else {
            note.to_owned()
        };
        Some(Self {
            id: anchor::new_id(),
            deck_id: deck_id.to_owned(),
            kind: CardKind::Basic,
            front,
            back,
            source: Some(SourceAnchor {
                path: source_path.to_path_buf(),
                page: annotation.page,
                quote: annotation.anchor.clone(),
                rect: bounds(annotation),
                annot_id: Some(annotation.id.clone()),
            }),
            tags: annotation.tags.clone(),
            media: Vec::new(),
            created: now,
            modified: now,
        })
    }

    /// Creates a basic card from a sticky note, free-text or callout annotation.
    ///
    /// If the note contains two lines separated by a blank line or `?`, the first part becomes the prompt and the rest
    /// becomes the answer; otherwise the entire note becomes the front prompt.
    #[must_use]
    pub fn from_note(
        annotation: &Annotation,
        source_path: &Path,
        deck_id: &str,
        now: i64,
    ) -> Option<Self> {
        let text = annotation.contents.trim();
        if text.is_empty() {
            return None;
        }
        let (front, back) = if let Some((q, a)) = text.split_once("\n\n") {
            (q.trim().to_owned(), a.trim().to_owned())
        } else {
            (text.to_owned(), String::new())
        };
        Some(Self {
            id: anchor::new_id(),
            deck_id: deck_id.to_owned(),
            kind: CardKind::Basic,
            front,
            back,
            source: Some(SourceAnchor {
                path: source_path.to_path_buf(),
                page: annotation.page,
                quote: annotation.anchor.clone(),
                rect: bounds(annotation),
                annot_id: Some(annotation.id.clone()),
            }),
            tags: annotation.tags.clone(),
            media: Vec::new(),
            created: now,
            modified: now,
        })
    }

    /// Creates a basic card from a lasso selection (front = region image, back = user note).
    #[must_use]
    pub fn from_lasso(
        page: usize,
        region: PageRect,
        image: CardMedia,
        back: &str,
        source_path: &Path,
        deck_id: &str,
        now: i64,
    ) -> Self {
        let front = format!("<img src=\"{}\">", image.name);
        Self {
            id: anchor::new_id(),
            deck_id: deck_id.to_owned(),
            kind: CardKind::Basic,
            front,
            back: back.trim().to_owned(),
            source: Some(SourceAnchor {
                path: source_path.to_path_buf(),
                page,
                quote: None,
                rect: Some(region),
                annot_id: None,
            }),
            tags: Vec::new(),
            media: vec![image],
            created: now,
            modified: now,
        }
    }

    /// Generates one image-occlusion card per box in `spec` (plan S.2).
    #[must_use]
    pub fn from_image_occlusion(
        spec: &ImageOcclusionSpec,
        image: &CardMedia,
        meta: &OcclusionCardMeta<'_>,
    ) -> Vec<Self> {
        spec.boxes
            .iter()
            .enumerate()
            .map(|(index, occl_box)| {
                let back = match (&occl_box.label, meta.back_extra.trim()) {
                    (Some(label), "") => label.clone(),
                    (Some(label), extra) => format!("{label}\n\n{extra}"),
                    (None, extra) => extra.to_owned(),
                };
                Self {
                    id: anchor::new_id(),
                    deck_id: meta.deck_id.to_owned(),
                    kind: CardKind::ImageOcclusion {
                        spec: spec.clone(),
                        active_box_index: index,
                    },
                    front: meta.header.trim().to_owned(),
                    back,
                    source: meta.source.clone(),
                    tags: meta.tags.to_vec(),
                    media: vec![image.clone()],
                    created: meta.now,
                    modified: meta.now,
                }
            })
            .collect()
    }

    /// Generates one cloze card per distinct `{{cN::...}}` ordinal in `cloze_text` (plan S.3).
    #[must_use]
    pub fn from_cloze_text(
        cloze_text: &str,
        back_extra: &str,
        source: Option<SourceAnchor>,
        deck_id: &str,
        tags: &[String],
        now: i64,
    ) -> Vec<Self> {
        cloze_ordinals(cloze_text)
            .into_iter()
            .map(|ordinal| Self {
                id: anchor::new_id(),
                deck_id: deck_id.to_owned(),
                kind: CardKind::Cloze { ordinal },
                front: cloze_text.to_owned(),
                back: back_extra.trim().to_owned(),
                source: source.clone(),
                tags: tags.to_vec(),
                media: Vec::new(),
                created: now,
                modified: now,
            })
            .collect()
    }

    /// Renders the prompt shown on the question side of the review screen.
    #[must_use]
    pub fn rendered_front(&self) -> String {
        match &self.kind {
            CardKind::Basic | CardKind::ImageOcclusion { .. } => self.front.clone(),
            CardKind::Cloze { ordinal } => render_cloze_question(&self.front, *ordinal),
        }
    }

    /// Renders the revealed answer shown on the answer side of the review screen.
    #[must_use]
    pub fn rendered_back(&self) -> String {
        match &self.kind {
            CardKind::Basic | CardKind::ImageOcclusion { .. } => self.back.clone(),
            CardKind::Cloze { ordinal } => {
                let revealed = render_cloze_answer(&self.front, *ordinal);
                if self.back.is_empty() {
                    revealed
                } else {
                    format!("{revealed}\n\n{}", self.back)
                }
            }
        }
    }

    /// Resolves this card's jump-to-page target (plan S.1).
    #[must_use]
    pub fn jump_to_source(&self, document_text: Option<&str>) -> Option<JumpTarget> {
        self.source
            .as_ref()
            .map(|src| src.jump_target(document_text))
    }

    /// Converts this card to a SQLite [`CardRecord`] for persistence in `excellent_store::study`.
    #[must_use]
    pub fn to_record(&self) -> CardRecord {
        let (kind, cloze_ordinal, occlusion_json) = match &self.kind {
            CardKind::Basic => ("basic".to_owned(), None, None),
            CardKind::Cloze { ordinal } => ("cloze".to_owned(), Some(*ordinal), None),
            CardKind::ImageOcclusion {
                spec,
                active_box_index,
            } => (
                "image_occlusion".to_owned(),
                None,
                Some(spec.to_json(*active_box_index)),
            ),
        };
        let media_json = serde_json::to_string(
            &self
                .media
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "mime": m.mime,
                        "bytes_hex": encode_hex(&m.bytes),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_owned());
        let tags_json = serde_json::to_string(&self.tags).unwrap_or_else(|_| "[]".to_owned());
        CardRecord {
            id: self.id.clone(),
            deck_id: self.deck_id.clone(),
            kind,
            front: self.front.clone(),
            back: self.back.clone(),
            cloze_ordinal,
            occlusion_json,
            source_path: self
                .source
                .as_ref()
                .map(|s| s.path.to_string_lossy().into_owned()),
            source_page: self.source.as_ref().map(|s| s.page),
            anchor_json: self.source.as_ref().map(SourceAnchor::to_json),
            media_json,
            tags_json,
            extra_json: "{}".to_owned(),
            created: self.created,
            modified: self.modified,
        }
    }

    /// Reconstructs a [`Flashcard`] from a persisted [`CardRecord`].
    #[must_use]
    pub fn from_record(record: &CardRecord) -> Option<Self> {
        let kind = match record.kind.as_str() {
            "basic" | "vocab" => CardKind::Basic,
            "cloze" => CardKind::Cloze {
                ordinal: record.cloze_ordinal.unwrap_or(1),
            },
            "image_occlusion" => {
                let (spec, active_box_index) =
                    ImageOcclusionSpec::from_json(record.occlusion_json.as_deref()?)?;
                CardKind::ImageOcclusion {
                    spec,
                    active_box_index,
                }
            }
            _ => return None,
        };
        let source = match (&record.source_path, record.source_page) {
            (Some(path), Some(page)) => Some(SourceAnchor::from_stored(
                path,
                page,
                record.anchor_json.as_deref(),
            )),
            _ => None,
        };
        let tags: Vec<String> = serde_json::from_str(&record.tags_json).unwrap_or_default();
        let media = parse_media_json(&record.media_json);
        Some(Self {
            id: record.id.clone(),
            deck_id: record.deck_id.clone(),
            kind,
            front: record.front.clone(),
            back: record.back.clone(),
            source,
            tags,
            media,
            created: record.created,
            modified: record.modified,
        })
    }
}

/// One parsed Anki-compatible `{{cN::answer[::hint]}}` cloze deletion (plan S.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClozeDeletion {
    /// 1-based cloze number (`1` for `c1`).
    pub ordinal: u32,
    /// Hidden answer text.
    pub answer: String,
    /// Optional hint displayed inside `[hint]` on the question side.
    pub hint: Option<String>,
    /// Byte span in the source cloze string covering `{{...}}`.
    pub span: Range<usize>,
}

/// Parses every valid `{{cN::answer}}` or `{{cN::answer::hint}}` cloze deletion in `text` (plan S.3).
#[must_use]
pub fn parse_clozes(text: &str) -> Vec<ClozeDeletion> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 6 < bytes.len() {
        if &bytes[i..i + 2] == b"{{" && (bytes[i + 2] == b'c' || bytes[i + 2] == b'C') {
            let num_start = i + 3;
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() {
                num_end += 1;
            }
            if num_end > num_start
                && num_end + 2 <= bytes.len()
                && &bytes[num_end..num_end + 2] == b"::"
                && let Ok(ordinal) = text[num_start..num_end].parse::<u32>()
                && ordinal > 0
                && let Some(rel_close) = text[num_end + 2..].find("}}")
            {
                let body_start = num_end + 2;
                let body_end = body_start + rel_close;
                let body = &text[body_start..body_end];
                let (answer, hint) = match body.split_once("::") {
                    Some((ans, h)) => (ans.to_owned(), Some(h.to_owned())),
                    None => (body.to_owned(), None),
                };
                out.push(ClozeDeletion {
                    ordinal,
                    answer,
                    hint,
                    span: i..body_end + 2,
                });
                i = body_end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Returns the distinct 1-based cloze ordinals in `text`, in ascending order.
#[must_use]
pub fn cloze_ordinals(text: &str) -> Vec<u32> {
    let set: BTreeSet<u32> = parse_clozes(text).into_iter().map(|c| c.ordinal).collect();
    set.into_iter().collect()
}

/// Renders the question side of a cloze card for `active_ordinal`: matching clozes become `[...]` (or `[hint]`),
/// while other ordinals show their plain text.
#[must_use]
pub fn render_cloze_question(text: &str, active_ordinal: u32) -> String {
    let clozes = parse_clozes(text);
    if clozes.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for cloze in clozes {
        out.push_str(&text[cursor..cloze.span.start]);
        if cloze.ordinal == active_ordinal {
            match cloze.hint.as_deref() {
                Some(hint) if !hint.is_empty() => {
                    out.push('[');
                    out.push_str(hint);
                    out.push(']');
                }
                _ => out.push_str("[...]"),
            }
        } else {
            out.push_str(&cloze.answer);
        }
        cursor = cloze.span.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Renders the answer side of a cloze card for `active_ordinal`, revealing every deletion and marking the tested one
/// with `[answer]`.
#[must_use]
pub fn render_cloze_answer(text: &str, active_ordinal: u32) -> String {
    let clozes = parse_clozes(text);
    if clozes.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for cloze in clozes {
        out.push_str(&text[cursor..cloze.span.start]);
        if cloze.ordinal == active_ordinal {
            out.push('[');
            out.push_str(&cloze.answer);
            out.push(']');
        } else {
            out.push_str(&cloze.answer);
        }
        cursor = cloze.span.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Wraps a byte range `selection` inside `highlight` into an Anki cloze deletion `{{c<ordinal>::...}}` (plan S.3).
///
/// Returns `None` if `ordinal == 0`, `selection` is out of bounds, not on UTF-8 char boundaries, or empty.
#[must_use]
pub fn wrap_cloze(
    highlight: &str,
    selection: Range<usize>,
    ordinal: u32,
    hint: Option<&str>,
) -> Option<String> {
    if ordinal == 0
        || selection.is_empty()
        || selection.end > highlight.len()
        || !highlight.is_char_boundary(selection.start)
        || !highlight.is_char_boundary(selection.end)
    {
        return None;
    }
    let selected = &highlight[selection.clone()];
    if selected.trim().is_empty() {
        return None;
    }
    let mut out = String::with_capacity(highlight.len() + 16);
    out.push_str(&highlight[..selection.start]);
    out.push_str(&format!("{{{{c{ordinal}::{selected}"));
    if let Some(h) = hint.filter(|s| !s.is_empty()) {
        out.push_str("::");
        out.push_str(h);
    }
    out.push_str("}}");
    out.push_str(&highlight[selection.end..]);
    Some(out)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let (pairs, _) = hex.as_bytes().as_chunks::<2>();
    for &[a, b] in pairs {
        let hi = hex_nibble(a)?;
        let lo = hex_nibble(b)?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_media_json(raw: &str) -> Vec<CardMedia> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_owned();
            let mime = item.get("mime")?.as_str()?.to_owned();
            let bytes = decode_hex(item.get("bytes_hex")?.as_str()?)?;
            Some(CardMedia { name, mime, bytes })
        })
        .collect()
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_nibble(bytes[i + 1])?;
            let lo = hex_nibble(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Boxed, Markup, Style};
    use excellent_mupdf::{PagePoint, Quad};

    use super::*;

    fn sample_highlight() -> Annotation {
        let mut annot = Annotation::new(
            7,
            Shape::Markup(
                Markup::Highlight,
                vec![Quad {
                    ul: PagePoint { x: 10.0, y: 20.0 },
                    ur: PagePoint { x: 200.0, y: 20.0 },
                    ll: PagePoint { x: 10.0, y: 34.0 },
                    lr: PagePoint { x: 200.0, y: 34.0 },
                }],
            ),
            Style::default(),
            "Ada",
        );
        let text = "In thermodynamics, entropy measures the number of microstates.";
        let byte = text.find("entropy measures").expect("substring");
        let start = text[..byte].chars().count();
        annot.anchor = Quote::of(text, start..start + "entropy measures".len());
        annot.contents = "Boltzmann formula: S = k ln W".to_owned();
        annot.tags = vec!["physics".to_owned()];
        annot
    }

    #[test]
    fn highlight_card_opens_source_page_and_reanchors() {
        let annot = sample_highlight();
        let path = Path::new("C:/books/Thermal Physics.pdf");
        let card = Flashcard::from_highlight(&annot, "", path, "deck-phys", 1_700_000_000)
            .expect("card created");
        assert_eq!(card.front, "entropy measures");
        assert_eq!(card.back, "Boltzmann formula: S = k ln W");
        assert_eq!(card.tags, ["physics"]);

        let retypeset = "In thermodynamics, entropy\nmeasures the number of microstates.";
        let target = card
            .jump_to_source(Some(retypeset))
            .expect("jump target present");
        assert_eq!(target.path, path);
        assert_eq!(target.page, 7);
        assert!(matches!(target.placement, Some(Placement::Exact(_))));

        // Round-trip through SQLite CardRecord.
        let rec = card.to_record();
        let restored = Flashcard::from_record(&rec).expect("restored card");
        assert_eq!(restored, card);
    }

    #[test]
    fn note_and_lasso_cards_link_back_to_source_page() {
        let mut note = Annotation::new(
            3,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 50.0,
                    y0: 80.0,
                    x1: 70.0,
                    y1: 100.0,
                },
            ),
            Style::default(),
            "Ada",
        );
        note.contents = "What is the Carnot efficiency?\n\n1 - T_c / T_h".to_owned();
        let path = Path::new("C:/books/notes.pdf");
        let card = Flashcard::from_note(&note, path, "deck-1", 10).expect("note card");
        assert_eq!(card.front, "What is the Carnot efficiency?");
        assert_eq!(card.back, "1 - T_c / T_h");
        assert_eq!(card.jump_to_source(None).expect("jump").page, 3);

        let lasso = Flashcard::from_lasso(
            5,
            PageRect {
                x0: 10.0,
                y0: 10.0,
                x1: 120.0,
                y1: 90.0,
            },
            CardMedia {
                name: "diagram.png".to_owned(),
                mime: "image/png".to_owned(),
                bytes: vec![0x89, b'P', b'N', b'G'],
            },
            "Krebs cycle diagram",
            path,
            "deck-1",
            20,
        );
        assert!(lasso.front.contains("diagram.png"));
        assert_eq!(lasso.jump_to_source(None).expect("jump").page, 5);
        assert_eq!(Flashcard::from_record(&lasso.to_record()), Some(lasso));
    }

    #[test]
    fn image_occlusion_supports_single_box_and_hide_all_guess_one() {
        let b1 = OcclusionBox {
            id: "b1".to_owned(),
            rect: NormalizedRect::try_new(0.1, 0.2, 0.25, 0.15).expect("valid"),
            label: Some("Aorta".to_owned()),
        };
        let b2 = OcclusionBox {
            id: "b2".to_owned(),
            rect: NormalizedRect::try_new(0.5, 0.4, 0.2, 0.15).expect("valid"),
            label: Some("Left ventricle".to_owned()),
        };
        let spec_all = ImageOcclusionSpec {
            image_name: "heart.png".to_owned(),
            boxes: vec![b1.clone(), b2.clone()],
            mode: OcclusionMode::HideAllGuessOne,
        };
        assert_eq!(spec_all.masked_indices(0), vec![0, 1]);
        assert_eq!(spec_all.masked_indices(1), vec![0, 1]);

        let spec_one = ImageOcclusionSpec {
            mode: OcclusionMode::HideOneGuessOne,
            ..spec_all.clone()
        };
        assert_eq!(spec_one.masked_indices(0), vec![0]);
        assert_eq!(spec_one.masked_indices(1), vec![1]);

        let img = CardMedia {
            name: "heart.png".to_owned(),
            mime: "image/png".to_owned(),
            bytes: vec![1, 2, 3, 4],
        };
        let cards = Flashcard::from_image_occlusion(
            &spec_all,
            &img,
            &OcclusionCardMeta {
                header: "Heart anatomy",
                back_extra: "Chapter 4",
                source: None,
                deck_id: "anatomy",
                tags: &["bio".to_owned()],
                now: 100,
            },
        );
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].back, "Aorta\n\nChapter 4");
        assert_eq!(cards[1].back, "Left ventricle\n\nChapter 4");
        assert_eq!(
            Flashcard::from_record(&cards[1].to_record()),
            Some(cards[1].clone())
        );
    }

    #[test]
    fn cloze_syntax_parses_wraps_and_renders_anki_compatible() {
        let base = "Canberra was founded in 1913 as the capital of Australia.";
        let city = base.find("Canberra").expect("city");
        let with_c1 = wrap_cloze(base, city..city + "Canberra".len(), 1, Some("city")).expect("c1");
        let year = with_c1.find("1913").expect("year");
        let with_both = wrap_cloze(&with_c1, year..year + 4, 2, None).expect("c2");
        assert_eq!(
            with_both,
            "{{c1::Canberra::city}} was founded in {{c2::1913}} as the capital of Australia."
        );
        assert_eq!(cloze_ordinals(&with_both), vec![1, 2]);
        assert_eq!(
            render_cloze_question(&with_both, 1),
            "[city] was founded in 1913 as the capital of Australia."
        );
        assert_eq!(
            render_cloze_question(&with_both, 2),
            "Canberra was founded in [...] as the capital of Australia."
        );
        assert_eq!(
            render_cloze_answer(&with_both, 1),
            "[Canberra] was founded in 1913 as the capital of Australia."
        );
        let cards = Flashcard::from_cloze_text(&with_both, "Geography", None, "geo", &[], 50);
        assert_eq!(cards.len(), 2);
        assert_eq!(
            cards[0].rendered_front(),
            "[city] was founded in 1913 as the capital of Australia."
        );
        assert_eq!(
            cards[1].rendered_back(),
            "Canberra was founded in [1913] as the capital of Australia.\n\nGeography"
        );
    }

    #[test]
    fn source_uri_round_trips_special_characters() {
        let src = SourceAnchor {
            path: PathBuf::from("C:/Courses/Résumé #1.pdf"),
            page: 4,
            quote: None,
            rect: None,
            annot_id: None,
        };
        let uri = src.to_uri();
        assert!(uri.contains("%23") && uri.ends_with("#page=5"));
        let parsed = SourceAnchor::from_uri(&uri).expect("parsed");
        assert_eq!(parsed.path, PathBuf::from("C:/Courses/Résumé #1.pdf"));
        assert_eq!(parsed.page, 4);
    }
}
