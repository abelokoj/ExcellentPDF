//! Anki `.apkg` import and export (plan S.11, S.2, S.3).
//!
//! Reads and writes standard Anki desktop `.apkg` archives (ZIP containing SQLite `collection.anki2`, JSON `media`
//! manifest and numbered media blobs) using workspace `zip` and `rusqlite`. Supports Basic, Cloze (`{{c1::...}}`) and
//! Image Occlusion note types while preserving source links in a dedicated `SourceLink` field.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use excellent_core::anchor;
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::cards::{
    CardKind, CardMedia, Flashcard, ImageOcclusionSpec, NormalizedRect, OcclusionBox,
    OcclusionMode, SourceAnchor, cloze_ordinals,
};

/// Anki field separator (`U+001F`).
const ANKI_FIELD_SEP: char = '\x1f';

/// Stable model IDs for ExcellentPDF's three standard Anki note types.
const MODEL_BASIC_ID: i64 = 1_700_100_000_001;
const MODEL_CLOZE_ID: i64 = 1_700_100_000_002;
const MODEL_IO_ID: i64 = 1_700_100_000_003;

/// Maximum uncompressed size read from a single `.apkg` entry (64 MiB).
const MAX_APKG_ENTRY_BYTES: usize = 64 * 1024 * 1024;

/// Errors returned while importing or exporting an Anki `.apkg` archive.
#[derive(Debug)]
#[non_exhaustive]
pub enum AnkiError {
    /// ZIP archive read/write error.
    Zip(zip::result::ZipError),
    /// SQLite read/write error.
    Sqlite(rusqlite::Error),
    /// Filesystem or buffer I/O error.
    Io(std::io::Error),
    /// JSON parsing error in `col.models`, `col.decks` or `media`.
    Json(serde_json::Error),
    /// Malformed or unsupported `.apkg` structure.
    InvalidArchive(String),
}

impl fmt::Display for AnkiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zip(e) => write!(f, "anki zip error: {e}"),
            Self::Sqlite(e) => write!(f, "anki sqlite error: {e}"),
            Self::Io(e) => write!(f, "anki i/o error: {e}"),
            Self::Json(e) => write!(f, "anki json error: {e}"),
            Self::InvalidArchive(msg) => write!(f, "invalid anki archive: {msg}"),
        }
    }
}

impl std::error::Error for AnkiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Zip(e) => Some(e),
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::InvalidArchive(_) => None,
        }
    }
}

impl From<zip::result::ZipError> for AnkiError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e)
    }
}

impl From<rusqlite::Error> for AnkiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<std::io::Error> for AnkiError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for AnkiError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Deck imported from an `.apkg` archive.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedAnkiDeck {
    /// Deck name stored in `col.decks`.
    pub deck_name: String,
    /// Imported flashcards (Basic, Cloze and Image Occlusion).
    pub cards: Vec<Flashcard>,
}

/// RAII temporary file guard cleaned up automatically on drop.
struct TempSqliteFile(PathBuf);

impl TempSqliteFile {
    fn unique(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "excellentpdf-anki-{tag}-{}-{seq}.anki2",
            std::process::id()
        ));
        Self(path)
    }
}

impl Drop for TempSqliteFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Exports `cards` under `deck_name` into an Anki-compatible `.apkg` byte archive (plan S.11, S.2, S.3).
///
/// # Errors
///
/// Returns [`AnkiError`] if SQLite creation or ZIP encoding fails.
pub fn export_apkg(deck_name: &str, cards: &[Flashcard], now: i64) -> Result<Vec<u8>, AnkiError> {
    let temp = TempSqliteFile::unique("export");
    let conn = Connection::open(&temp.0)?;
    create_anki_schema(&conn)?;

    let deck_id = stable_id_i64(deck_name);
    let models = build_models_json(now);
    let decks = json!({
        "1": {
            "id": 1,
            "name": "Default",
            "mod": now,
            "usn": -1,
            "lrnToday": [0, 0],
            "revToday": [0, 0],
            "newToday": [0, 0],
            "timeToday": [0, 0],
            "collapsed": false,
            "browserCollapsed": false,
            "desc": "",
            "dyn": 0,
            "conf": 1,
            "extendNew": 0,
            "extendRev": 0
        },
        deck_id.to_string(): {
            "id": deck_id,
            "name": deck_name,
            "mod": now,
            "usn": -1,
            "lrnToday": [0, 0],
            "revToday": [0, 0],
            "newToday": [0, 0],
            "timeToday": [0, 0],
            "collapsed": false,
            "browserCollapsed": false,
            "desc": "Exported from ExcellentPDF",
            "dyn": 0,
            "conf": 1,
            "extendNew": 0,
            "extendRev": 0
        }
    });
    let dconf = json!({
        "1": {
            "id": 1,
            "name": "Default",
            "mod": 0,
            "usn": 0,
            "maxTaken": 60,
            "autoplay": true,
            "timer": 0,
            "replayq": true,
            "new": {
                "bury": false,
                "delays": [1, 10],
                "initialFactor": 2500,
                "ints": [1, 4, 7],
                "order": 1,
                "perDay": 20
            },
            "rev": {
                "bury": false,
                "ease4": 1.3,
                "ivlFct": 1.0,
                "maxIvl": 36500,
                "perDay": 200,
                "hardFactor": 1.2
            },
            "lapse": {
                "delays": [10],
                "leechAction": 1,
                "leechFails": 8,
                "minInt": 1,
                "mult": 0.0
            }
        }
    });
    let conf = json!({
        "activeDecks": [deck_id],
        "curDeck": deck_id,
        "newSpread": 0,
        "collapseTime": 1200,
        "timeLim": 0,
        "estTimes": true,
        "dueCounts": true,
        "curModel": MODEL_BASIC_ID.to_string(),
        "nextPos": 1,
        "sortType": "noteFld",
        "sortBackwards": false,
        "addToCur": true
    });

    conn.execute(
        "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags)
         VALUES (1, ?1, ?2, ?3, 11, 0, 0, 0, ?4, ?5, ?6, ?7, '{}')",
        params![
            now,
            now * 1000,
            now * 1000,
            conf.to_string(),
            models.to_string(),
            decks.to_string(),
            dconf.to_string(),
        ],
    )?;

    // Deduplicate media attachments across cards by file name.
    let mut media_files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for card in cards {
        for asset in &card.media {
            media_files
                .entry(asset.name.clone())
                .or_insert_with(|| asset.bytes.clone());
        }
    }

    // Group sibling Cloze or Image Occlusion cards into single Anki notes with multiple cards (`ord`).
    let mut note_seq = now.saturating_mul(1000).max(1_700_000_000_000);
    let mut card_seq = note_seq;
    let mut cloze_note_ids: HashMap<(String, String), i64> = HashMap::new();
    let mut io_note_ids: HashMap<(String, String), i64> = HashMap::new();

    for card in cards {
        let source_link = card
            .source
            .as_ref()
            .map_or_else(String::new, SourceAnchor::to_uri);
        let tags_str = format_anki_tags(&card.tags);

        match &card.kind {
            CardKind::Basic => {
                note_seq += 1;
                card_seq += 1;
                let fields = [
                    card.front.as_str(),
                    card.back.as_str(),
                    source_link.as_str(),
                ]
                .join(&ANKI_FIELD_SEP.to_string());
                insert_note(
                    &conn,
                    &NoteRow {
                        id: note_seq,
                        guid: &short_guid(&card.id),
                        mid: MODEL_BASIC_ID,
                        now,
                        tags: &tags_str,
                        fields: &fields,
                        sort_field: &card.front,
                    },
                )?;
                insert_card(&conn, card_seq, note_seq, deck_id, 0, now)?;
            }
            CardKind::Cloze { ordinal } => {
                let key = (card.front.clone(), source_link.clone());
                let nid = if let Some(&existing) = cloze_note_ids.get(&key) {
                    existing
                } else {
                    note_seq += 1;
                    let fields = [
                        card.front.as_str(),
                        card.back.as_str(),
                        source_link.as_str(),
                    ]
                    .join(&ANKI_FIELD_SEP.to_string());
                    insert_note(
                        &conn,
                        &NoteRow {
                            id: note_seq,
                            guid: &short_guid(&card.id),
                            mid: MODEL_CLOZE_ID,
                            now,
                            tags: &tags_str,
                            fields: &fields,
                            sort_field: &card.front,
                        },
                    )?;
                    cloze_note_ids.insert(key, note_seq);
                    note_seq
                };
                card_seq += 1;
                let ord = i64::from(ordinal.saturating_sub(1));
                insert_card(&conn, card_seq, nid, deck_id, ord, now)?;
            }
            CardKind::ImageOcclusion {
                spec,
                active_box_index,
            } => {
                let occlusion_markup = encode_anki_io_markup(spec);
                let key = (spec.image_name.clone(), occlusion_markup.clone());
                let nid = if let Some(&existing) = io_note_ids.get(&key) {
                    existing
                } else {
                    note_seq += 1;
                    let img_tag = format!("<img src=\"{}\">", spec.image_name);
                    let mode_tag = format!("io-mode:{}", spec.mode.key());
                    let merged_tags = if tags_str.is_empty() {
                        format!(" {mode_tag} ")
                    } else {
                        format!("{tags_str}{mode_tag} ")
                    };
                    let fields = [
                        occlusion_markup.as_str(),
                        img_tag.as_str(),
                        card.front.as_str(),
                        card.back.as_str(),
                        source_link.as_str(),
                    ]
                    .join(&ANKI_FIELD_SEP.to_string());
                    insert_note(
                        &conn,
                        &NoteRow {
                            id: note_seq,
                            guid: &short_guid(&card.id),
                            mid: MODEL_IO_ID,
                            now,
                            tags: &merged_tags,
                            fields: &fields,
                            sort_field: &card.front,
                        },
                    )?;
                    io_note_ids.insert(key, note_seq);
                    note_seq
                };
                card_seq += 1;
                let ord = i64::try_from(*active_box_index).unwrap_or(0);
                insert_card(&conn, card_seq, nid, deck_id, ord, now)?;
            }
        }
    }

    drop(conn);
    let db_bytes = std::fs::read(&temp.0)?;

    let mut media_manifest = Map::new();
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("collection.anki2", opts)?;
        zip.write_all(&db_bytes)?;

        for (index, (name, bytes)) in media_files.into_iter().enumerate() {
            let key = index.to_string();
            media_manifest.insert(key.clone(), Value::String(name));
            zip.start_file(key, opts)?;
            zip.write_all(&bytes)?;
        }
        zip.start_file("media", opts)?;
        zip.write_all(Value::Object(media_manifest).to_string().as_bytes())?;
        zip.finish()?;
    }
    Ok(cursor.into_inner())
}

/// Imports an Anki `.apkg` archive into an [`ImportedAnkiDeck`] (plan S.11, S.2, S.3).
///
/// # Errors
///
/// Returns [`AnkiError`] if the ZIP is malformed, lacks `collection.anki2`, or contains invalid SQLite/JSON metadata.
pub fn import_apkg(bytes: &[u8], deck_id: &str, now: i64) -> Result<ImportedAnkiDeck, AnkiError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let db_bytes = read_zip_entry(&mut archive, "collection.anki21")
        .or_else(|_| read_zip_entry(&mut archive, "collection.anki2"))?;

    let media_manifest: HashMap<String, String> = match read_zip_entry(&mut archive, "media") {
        Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
        Err(_) => HashMap::new(),
    };
    let mut media_by_name: HashMap<String, Vec<u8>> = HashMap::new();
    for (zip_key, filename) in media_manifest {
        if let Ok(blob) = read_zip_entry(&mut archive, &zip_key) {
            media_by_name.insert(filename, blob);
        }
    }

    let temp = TempSqliteFile::unique("import");
    std::fs::write(&temp.0, &db_bytes)?;
    let conn = Connection::open(&temp.0)?;

    let (models_raw, decks_raw): (String, String) =
        conn.query_row("SELECT models, decks FROM col LIMIT 1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let models_json: Value = serde_json::from_str(&models_raw)?;
    let decks_json: Value = serde_json::from_str(&decks_raw)?;

    let deck_name = decks_json
        .as_object()
        .and_then(|map| {
            map.values()
                .filter_map(|d| d.get("name").and_then(Value::as_str))
                .find(|&name| name != "Default")
                .or_else(|| {
                    map.values()
                        .filter_map(|d| d.get("name").and_then(Value::as_str))
                        .next()
                })
        })
        .unwrap_or("Imported Anki Deck")
        .to_owned();

    let mut stmt = conn.prepare(
        "SELECT n.id, n.mid, n.tags, n.flds, c.ord
         FROM cards c JOIN notes n ON n.id = c.nid
         ORDER BY n.id ASC, c.ord ASC",
    )?;

    let mut cards = Vec::new();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    for row in rows {
        let (_nid, mid, raw_tags, raw_flds, ord) = row?;
        let fields: Vec<&str> = raw_flds.split(ANKI_FIELD_SEP).collect();
        let (model_kind, field_names) = inspect_model(&models_json, mid);
        let mut tags = parse_anki_tags(&raw_tags);
        let mut io_mode = OcclusionMode::HideAllGuessOne;
        tags.retain(|t| {
            if let Some(key) = t.strip_prefix("io-mode:") {
                if let Some(parsed) = OcclusionMode::from_key(key) {
                    io_mode = parsed;
                }
                return false;
            }
            true
        });

        let source = find_field(&fields, &field_names, &["SourceLink", "Source"])
            .and_then(SourceAnchor::from_uri);

        match model_kind {
            AnkiModelKind::ImageOcclusion => {
                let occl_str = fields.first().copied().unwrap_or("");
                let img_field = fields.get(1).copied().unwrap_or("");
                let header = fields.get(2).copied().unwrap_or("").to_owned();
                let back_extra = fields.get(3).copied().unwrap_or("").to_owned();
                let image_name = extract_img_src(img_field).unwrap_or_default();
                let boxes = decode_anki_io_markup(occl_str);
                let active_box_index = usize::try_from(ord.max(0))
                    .unwrap_or(0)
                    .min(boxes.len().saturating_sub(1));
                let mut media = Vec::new();
                if let Some(blob) = media_by_name.get(&image_name) {
                    media.push(CardMedia {
                        name: image_name.clone(),
                        mime: guess_mime(&image_name).to_owned(),
                        bytes: blob.clone(),
                    });
                }
                let spec = ImageOcclusionSpec {
                    image_name,
                    boxes,
                    mode: io_mode,
                };
                cards.push(Flashcard {
                    id: anchor::new_id(),
                    deck_id: deck_id.to_owned(),
                    kind: CardKind::ImageOcclusion {
                        spec,
                        active_box_index,
                    },
                    front: header,
                    back: back_extra,
                    source,
                    tags,
                    media,
                    created: now,
                    modified: now,
                });
            }
            AnkiModelKind::Cloze => {
                let text = fields.first().copied().unwrap_or("").to_owned();
                let extra = fields.get(1).copied().unwrap_or("").to_owned();
                let ordinal = u32::try_from(ord.max(0) + 1).unwrap_or(1);
                let ordinals = cloze_ordinals(&text);
                let active = if ordinals.contains(&ordinal) {
                    ordinal
                } else {
                    ordinals.first().copied().unwrap_or(1)
                };
                let media = collect_referenced_media(&[&text, &extra], &media_by_name);
                cards.push(Flashcard {
                    id: anchor::new_id(),
                    deck_id: deck_id.to_owned(),
                    kind: CardKind::Cloze { ordinal: active },
                    front: text,
                    back: extra,
                    source,
                    tags,
                    media,
                    created: now,
                    modified: now,
                });
            }
            AnkiModelKind::Basic => {
                let front = fields.first().copied().unwrap_or("").to_owned();
                let back = fields.get(1).copied().unwrap_or("").to_owned();
                let media = collect_referenced_media(&[&front, &back], &media_by_name);
                cards.push(Flashcard {
                    id: anchor::new_id(),
                    deck_id: deck_id.to_owned(),
                    kind: CardKind::Basic,
                    front,
                    back,
                    source,
                    tags,
                    media,
                    created: now,
                    modified: now,
                });
            }
        }
    }

    Ok(ImportedAnkiDeck { deck_name, cards })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnkiModelKind {
    Basic,
    Cloze,
    ImageOcclusion,
}

fn inspect_model(models: &Value, mid: i64) -> (AnkiModelKind, Vec<String>) {
    let Some(model) = models.get(mid.to_string()) else {
        return (AnkiModelKind::Basic, Vec::new());
    };
    let name = model
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let model_type = model.get("type").and_then(Value::as_i64).unwrap_or(0);
    let field_names: Vec<String> = model
        .get("flds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let kind = if name.contains("occlusion")
        || field_names
            .first()
            .is_some_and(|n| n.eq_ignore_ascii_case("occlusion"))
    {
        AnkiModelKind::ImageOcclusion
    } else if model_type == 1 || name.contains("cloze") {
        AnkiModelKind::Cloze
    } else {
        AnkiModelKind::Basic
    };
    (kind, field_names)
}

fn find_field<'a>(fields: &[&'a str], names: &[String], candidates: &[&str]) -> Option<&'a str> {
    for (idx, name) in names.iter().enumerate() {
        if candidates.iter().any(|c| name.eq_ignore_ascii_case(c)) {
            let val = fields.get(idx).copied().unwrap_or("").trim();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Encodes occlusion boxes using Anki 23.10+ native image-occlusion cloze syntax:
/// `{{c1::image-occlusion:rect:left=0.1000:top=0.2000:width=0.2500:height=0.1500}}`.
fn encode_anki_io_markup(spec: &ImageOcclusionSpec) -> String {
    let mut lines = Vec::with_capacity(spec.boxes.len());
    for (idx, b) in spec.boxes.iter().enumerate() {
        let ordinal = idx + 1;
        let mut entry = format!(
            "{{{{c{ordinal}::image-occlusion:rect:left={:.4}:top={:.4}:width={:.4}:height={:.4}",
            b.rect.left, b.rect.top, b.rect.width, b.rect.height
        );
        if let Some(label) = b.label.as_deref().filter(|s| !s.is_empty()) {
            entry.push_str("::");
            entry.push_str(label);
        }
        entry.push_str("}}");
        lines.push(entry);
    }
    lines.join("\n")
}

/// Parses Anki 23.10+ image-occlusion cloze lines back into [`OcclusionBox`] records.
fn decode_anki_io_markup(markup: &str) -> Vec<OcclusionBox> {
    let mut boxes = Vec::new();
    for cloze in super::cards::parse_clozes(markup) {
        let Some(rest) = cloze.answer.strip_prefix("image-occlusion:rect:") else {
            continue;
        };
        let (mut left, mut top, mut width, mut height) = (None, None, None, None);
        for part in rest.split(':') {
            if let Some((k, v)) = part.split_once('=')
                && let Ok(num) = v.parse::<f32>()
            {
                match k {
                    "left" => left = Some(num),
                    "top" => top = Some(num),
                    "width" => width = Some(num),
                    "height" => height = Some(num),
                    _ => {}
                }
            }
        }
        if let (Some(l), Some(t), Some(w), Some(h)) = (left, top, width, height)
            && let Some(rect) = NormalizedRect::try_new(l, t, w, h)
        {
            boxes.push(OcclusionBox {
                id: format!("box-{}", cloze.ordinal),
                rect,
                label: cloze.hint,
            });
        }
    }
    boxes
}

fn create_anki_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE col (
            id integer primary key,
            crt integer not null,
            mod integer not null,
            scm integer not null,
            ver integer not null,
            dty integer not null,
            usn integer not null,
            ls integer not null,
            conf text not null,
            models text not null,
            decks text not null,
            dconf text not null,
            tags text not null
        );
        CREATE TABLE notes (
            id integer primary key,
            guid text not null,
            mid integer not null,
            mod integer not null,
            usn integer not null,
            tags text not null,
            flds text not null,
            sfld text not null,
            csum integer not null,
            flags integer not null,
            data text not null
        );
        CREATE TABLE cards (
            id integer primary key,
            nid integer not null,
            did integer not null,
            ord integer not null,
            mod integer not null,
            usn integer not null,
            type integer not null,
            queue integer not null,
            due integer not null,
            ivl integer not null,
            factor integer not null,
            reps integer not null,
            lapses integer not null,
            left integer not null,
            odue integer not null,
            odid integer not null,
            flags integer not null,
            data text not null
        );
        CREATE TABLE revlog (
            id integer primary key,
            cid integer not null,
            usn integer not null,
            ease integer not null,
            ivl integer not null,
            lastIvl integer not null,
            factor integer not null,
            time integer not null,
            type integer not null
        );
        CREATE TABLE graves (
            usn integer not null,
            oid integer not null,
            type integer not null
        );",
    )
}

struct NoteRow<'a> {
    id: i64,
    guid: &'a str,
    mid: i64,
    now: i64,
    tags: &'a str,
    fields: &'a str,
    sort_field: &'a str,
}

fn insert_note(conn: &Connection, note: &NoteRow<'_>) -> Result<(), rusqlite::Error> {
    let csum = field_checksum(note.sort_field);
    conn.execute(
        "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data)
         VALUES (?1, ?2, ?3, ?4, -1, ?5, ?6, ?7, ?8, 0, '')",
        params![
            note.id,
            note.guid,
            note.mid,
            note.now,
            note.tags,
            note.fields,
            note.sort_field,
            csum
        ],
    )?;
    Ok(())
}

fn insert_card(
    conn: &Connection,
    id: i64,
    nid: i64,
    did: i64,
    ord: i64,
    now: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, lapses, left, odue, odid, flags, data)
         VALUES (?1, ?2, ?3, ?4, ?5, -1, 0, 0, ?1, 0, 0, 0, 0, 0, 0, 0, 0, '')",
        params![id, nid, did, ord, now],
    )?;
    Ok(())
}

fn build_models_json(now: i64) -> Value {
    let field = |name: &str, ord: usize| {
        json!({
            "name": name,
            "ord": ord,
            "sticky": false,
            "rtl": false,
            "font": "Arial",
            "size": 20,
            "media": []
        })
    };
    json!({
        MODEL_BASIC_ID.to_string(): {
            "id": MODEL_BASIC_ID,
            "name": "Basic (ExcellentPDF)",
            "type": 0,
            "mod": now,
            "usn": -1,
            "sortf": 0,
            "did": 1,
            "tmpls": [{
                "name": "Card 1",
                "ord": 0,
                "qfmt": "{{Front}}",
                "afmt": "{{FrontSide}}<hr id=answer>{{Back}}{{#SourceLink}}<div class=source><a href=\"{{SourceLink}}\">Source</a></div>{{/SourceLink}}",
                "bqfmt": "",
                "bafmt": "",
                "did": null
            }],
            "flds": [field("Front", 0), field("Back", 1), field("SourceLink", 2)],
            "css": ".card { font-family: sans-serif; font-size: 18px; }",
            "latexPre": "",
            "latexPost": "",
            "tags": [],
            "vers": []
        },
        MODEL_CLOZE_ID.to_string(): {
            "id": MODEL_CLOZE_ID,
            "name": "Cloze (ExcellentPDF)",
            "type": 1,
            "mod": now,
            "usn": -1,
            "sortf": 0,
            "did": 1,
            "tmpls": [{
                "name": "Cloze",
                "ord": 0,
                "qfmt": "{{cloze:Text}}",
                "afmt": "{{cloze:Text}}<br>{{Back Extra}}{{#SourceLink}}<div class=source><a href=\"{{SourceLink}}\">Source</a></div>{{/SourceLink}}",
                "bqfmt": "",
                "bafmt": "",
                "did": null
            }],
            "flds": [field("Text", 0), field("Back Extra", 1), field("SourceLink", 2)],
            "css": ".card { font-family: sans-serif; font-size: 18px; } .cloze { font-weight: bold; color: #2563eb; }",
            "latexPre": "",
            "latexPost": "",
            "tags": [],
            "vers": []
        },
        MODEL_IO_ID.to_string(): {
            "id": MODEL_IO_ID,
            "name": "Image Occlusion (ExcellentPDF)",
            "type": 1,
            "mod": now,
            "usn": -1,
            "sortf": 2,
            "did": 1,
            "tmpls": [{
                "name": "Image Occlusion",
                "ord": 0,
                "qfmt": "{{Header}}<div id=image-occlusion-canvas>{{Image}}{{cloze:Occlusion}}</div>",
                "afmt": "{{Header}}<div id=image-occlusion-canvas>{{Image}}{{cloze:Occlusion}}</div><hr id=answer>{{Back Extra}}{{#SourceLink}}<div class=source><a href=\"{{SourceLink}}\">Source</a></div>{{/SourceLink}}",
                "bqfmt": "",
                "bafmt": "",
                "did": null
            }],
            "flds": [
                field("Occlusion", 0),
                field("Image", 1),
                field("Header", 2),
                field("Back Extra", 3),
                field("SourceLink", 4)
            ],
            "css": ".card { font-family: sans-serif; font-size: 18px; }",
            "latexPre": "",
            "latexPost": "",
            "tags": [],
            "vers": []
        }
    })
}

fn read_zip_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, AnkiError> {
    let entry = archive.by_name(name)?;
    if usize::try_from(entry.size()).unwrap_or(usize::MAX) > MAX_APKG_ENTRY_BYTES {
        return Err(AnkiError::InvalidArchive(format!(
            "entry {name} exceeds 64 MiB limit"
        )));
    }
    let mut buf = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    entry
        .take(MAX_APKG_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut buf)?;
    if buf.len() > MAX_APKG_ENTRY_BYTES {
        return Err(AnkiError::InvalidArchive(format!(
            "entry {name} exceeds 64 MiB limit"
        )));
    }
    Ok(buf)
}

fn stable_id_i64(input: &str) -> i64 {
    let digest = Sha256::digest(input.as_bytes());
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&digest[..8]);
    let val = u64::from_be_bytes(raw) & 0x0000_7fff_ffff_ffff;
    i64::try_from(val.max(2)).unwrap_or(2)
}

fn short_guid(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

fn field_checksum(text: &str) -> i64 {
    let digest = Sha256::digest(text.as_bytes());
    let val = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    i64::from(val)
}

fn format_anki_tags(tags: &[String]) -> String {
    let cleaned: Vec<String> = tags
        .iter()
        .map(|t| t.trim().replace(' ', "_"))
        .filter(|t| !t.is_empty())
        .collect();
    if cleaned.is_empty() {
        String::new()
    } else {
        format!(" {} ", cleaned.join(" "))
    }
}

fn parse_anki_tags(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(str::to_owned).collect()
}

fn extract_img_src(html: &str) -> Option<String> {
    let idx = html.find("src=")?;
    let rest = &html[idx + 4..];
    let quote = rest.bytes().next()?;
    if quote == b'"' || quote == b'\'' {
        let after = &rest[1..];
        let end = after.find(char::from(quote))?;
        Some(after[..end].to_owned())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

fn collect_referenced_media(
    fields: &[&str],
    media_by_name: &HashMap<String, Vec<u8>>,
) -> Vec<CardMedia> {
    let mut out = Vec::new();
    for (name, bytes) in media_by_name {
        if fields.iter().any(|f| f.contains(name.as_str())) {
            out.push(CardMedia {
                name: name.clone(),
                mime: guess_mime(name).to_owned(),
                bytes: bytes.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn guess_mime(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::super::cards::OcclusionCardMeta;
    use super::*;

    /// Done when (S.2, S.3, S.11): Basic, Cloze and Image Occlusion cards with media and source links round-trip
    /// cleanly through a real Anki `.apkg` (ZIP + SQLite `collection.anki2` + `media`).
    #[test]
    fn apkg_round_trips_basic_cloze_and_image_occlusion_with_media_and_source_links() {
        let source = SourceAnchor {
            path: PathBuf::from("C:/study/Neuroscience.pdf"),
            page: 14,
            quote: None,
            rect: None,
            annot_id: None,
        };
        let basic = Flashcard {
            id: "basic-1".to_owned(),
            deck_id: "neuro".to_owned(),
            kind: CardKind::Basic,
            front: "What does myelin do?".to_owned(),
            back: "Insulates axons to speed up action potentials.".to_owned(),
            source: Some(source.clone()),
            tags: vec!["neuro".to_owned(), "ch2".to_owned()],
            media: Vec::new(),
            created: 1_700_000_000,
            modified: 1_700_000_000,
        };

        let cloze_cards = Flashcard::from_cloze_text(
            "The {{c1::hippocampus}} consolidates {{c2::declarative::explicit}} memory.",
            "Medial temporal lobe",
            Some(source.clone()),
            "neuro",
            &["memory".to_owned()],
            1_700_000_010,
        );
        assert_eq!(cloze_cards.len(), 2);

        let io_spec = ImageOcclusionSpec {
            image_name: "brain.png".to_owned(),
            boxes: vec![
                OcclusionBox {
                    id: "b1".to_owned(),
                    rect: NormalizedRect::try_new(0.1, 0.2, 0.25, 0.15).expect("rect"),
                    label: Some("Frontal lobe".to_owned()),
                },
                OcclusionBox {
                    id: "b2".to_owned(),
                    rect: NormalizedRect::try_new(0.6, 0.3, 0.2, 0.18).expect("rect"),
                    label: Some("Occipital lobe".to_owned()),
                },
            ],
            mode: OcclusionMode::HideAllGuessOne,
        };
        let brain_png = CardMedia {
            name: "brain.png".to_owned(),
            mime: "image/png".to_owned(),
            bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        };
        let io_cards = Flashcard::from_image_occlusion(
            &io_spec,
            &brain_png,
            &OcclusionCardMeta {
                header: "Cortical lobes",
                back_extra: "Figure 3.2",
                source: Some(source.clone()),
                deck_id: "neuro",
                tags: &["anatomy".to_owned()],
                now: 1_700_000_020,
            },
        );

        let mut all_cards = vec![basic];
        all_cards.extend(cloze_cards);
        all_cards.extend(io_cards);
        assert_eq!(all_cards.len(), 5);

        let apkg = export_apkg("Neuroscience 101", &all_cards, 1_700_000_100).expect("export");
        assert!(!apkg.is_empty());

        // Inspect archive structure directly to ensure standard Anki desktop files exist.
        {
            let mut zip = ZipArchive::new(Cursor::new(apkg.as_slice())).expect("zip");
            assert!(zip.by_name("collection.anki2").is_ok());
            assert!(zip.by_name("media").is_ok());
            assert!(zip.by_name("0").is_ok());
        }

        // Import back and verify every card type, source link, media blob and occlusion box survived.
        let imported = import_apkg(&apkg, "neuro-imported", 1_700_000_200).expect("import");
        assert_eq!(imported.deck_name, "Neuroscience 101");
        assert_eq!(imported.cards.len(), 5);

        // Basic card
        let imp_basic = &imported.cards[0];
        assert_eq!(imp_basic.kind, CardKind::Basic);
        assert_eq!(imp_basic.front, "What does myelin do?");
        assert_eq!(imp_basic.tags, ["neuro", "ch2"]);
        assert_eq!(imp_basic.source.as_ref().expect("source").page, 14);

        // Cloze cards (c1 and c2)
        assert_eq!(imported.cards[1].kind, CardKind::Cloze { ordinal: 1 });
        assert_eq!(imported.cards[2].kind, CardKind::Cloze { ordinal: 2 });
        assert!(imported.cards[1].front.contains("{{c1::hippocampus}}"));
        assert!(
            imported.cards[2]
                .front
                .contains("{{c2::declarative::explicit}}")
        );

        // Image occlusion cards
        match &imported.cards[3].kind {
            CardKind::ImageOcclusion {
                spec,
                active_box_index,
            } => {
                assert_eq!(*active_box_index, 0);
                assert_eq!(spec.image_name, "brain.png");
                assert_eq!(spec.mode, OcclusionMode::HideAllGuessOne);
                assert_eq!(spec.boxes.len(), 2);
                assert_eq!(spec.boxes[0].label.as_deref(), Some("Frontal lobe"));
                assert_eq!(spec.boxes[1].label.as_deref(), Some("Occipital lobe"));
            }
            other => panic!("expected image occlusion, got {other:?}"),
        }
        assert_eq!(imported.cards[3].media.len(), 1);
        assert_eq!(imported.cards[3].media[0].bytes, brain_png.bytes);
        assert_eq!(
            imported.cards[4].source.as_ref().expect("io source").page,
            14
        );
    }
}
