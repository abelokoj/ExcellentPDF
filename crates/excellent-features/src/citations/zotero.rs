//! Zotero 7 local API, local `zotero.sqlite` reader, and Zotero Web API v3 sync client (plan F.11).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use super::{AuthorName, CitationEntry, EntryKind};

/// Default base URL for Zotero 7's local HTTP API.
pub const LOCAL_API_BASE: &str = "http://127.0.0.1:23119/api";

/// Base URL for Zotero Web API v3.
pub const WEB_API_BASE: &str = "https://api.zotero.org";

/// Error returned by Zotero local or web operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoteroError(pub String);

impl fmt::Display for ZoteroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "zotero: {}", self.0)
    }
}

impl std::error::Error for ZoteroError {}

/// A Zotero bibliographic item with optional attached local PDF path and collections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoteroItem {
    /// Zotero 8-character item key.
    pub item_key: String,
    /// Library version (for Web API conditional updates).
    pub version: u64,
    /// Canonical citation metadata.
    pub citation: CitationEntry,
    /// Collection keys or names the item belongs to.
    pub collections: Vec<String>,
    /// Tags attached in Zotero.
    pub tags: Vec<String>,
    /// Resolved local PDF attachment path, if present on disk.
    pub pdf_path: Option<PathBuf>,
}

/// Configuration for connecting to Zotero's local API or Web API v3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoteroSource {
    /// Zotero 7 local HTTP API (`http://127.0.0.1:23119/api/users/0/...`).
    LocalHttp {
        /// Base URL (defaults to [`LOCAL_API_BASE`]).
        base_url: String,
    },
    /// Zotero Web API v3 (`https://api.zotero.org/users/<user_id>/...`).
    WebApi {
        /// Base URL (defaults to [`WEB_API_BASE`]).
        base_url: String,
        /// Numeric Zotero user id (or `groups/<id>`).
        library_prefix: String,
        /// User API key (caller retrieves from OS keystore).
        api_key: String,
    },
}

impl ZoteroSource {
    /// Standard Zotero 7 local HTTP API endpoint.
    #[must_use]
    pub fn local() -> Self {
        Self::LocalHttp {
            base_url: LOCAL_API_BASE.to_owned(),
        }
    }

    /// Standard Zotero Web API v3 user endpoint.
    #[must_use]
    pub fn web_user(user_id: &str, api_key: impl Into<String>) -> Self {
        Self::WebApi {
            base_url: WEB_API_BASE.to_owned(),
            library_prefix: format!("users/{user_id}"),
            api_key: api_key.into(),
        }
    }

    fn items_url(&self, limit: usize) -> String {
        match self {
            Self::LocalHttp { base_url } => {
                format!(
                    "{}/users/0/items/top?limit={limit}",
                    base_url.trim_end_matches('/')
                )
            }
            Self::WebApi {
                base_url,
                library_prefix,
                ..
            } => format!(
                "{}/{library_prefix}/items/top?limit={limit}",
                base_url.trim_end_matches('/')
            ),
        }
    }

    fn post_items_url(&self) -> String {
        match self {
            Self::LocalHttp { base_url } => {
                format!("{}/users/0/items", base_url.trim_end_matches('/'))
            }
            Self::WebApi {
                base_url,
                library_prefix,
                ..
            } => format!("{}/{library_prefix}/items", base_url.trim_end_matches('/')),
        }
    }

    fn api_key(&self) -> Option<&str> {
        match self {
            Self::LocalHttp { .. } => None,
            Self::WebApi { api_key, .. } => Some(api_key.as_str()),
        }
    }
}

/// Zotero client supporting local HTTP API and Web API v3 via `ureq` (plan F.11).
#[derive(Debug, Clone)]
pub struct ZoteroClient {
    source: ZoteroSource,
}

impl ZoteroClient {
    /// Creates a client targeting `source`.
    #[must_use]
    pub const fn new(source: ZoteroSource) -> Self {
        Self { source }
    }

    /// Checks whether the local Zotero 7 HTTP endpoint is reachable.
    #[must_use]
    pub fn is_local_running(&self) -> bool {
        let ZoteroSource::LocalHttp { base_url } = &self.source else {
            return false;
        };
        let url = format!("{}/users/0/items?limit=1", base_url.trim_end_matches('/'));
        ureq::get(&url)
            .header("Zotero-API-Version", "3")
            .call()
            .is_ok()
    }

    /// Fetches top-level items from Zotero (local HTTP API or Web API v3).
    ///
    /// # Errors
    ///
    /// Returns [`ZoteroError`] if the HTTP request fails or the JSON response is invalid.
    pub fn fetch_items(&self, limit: usize) -> Result<Vec<ZoteroItem>, ZoteroError> {
        let url = self.source.items_url(limit);
        let mut req = ureq::get(&url).header("Zotero-API-Version", "3");
        if let Some(key) = self.source.api_key() {
            req = req.header("Zotero-API-Key", key);
        }
        let mut resp = req
            .call()
            .map_err(|e| ZoteroError(format!("request failed: {e}")))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ZoteroError(format!("read failed: {e}")))?;
        parse_zotero_items_json(&body)
    }

    /// Sends an annotation or highlight summary back to Zotero as a child note under `parent_item_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ZoteroError`] if the HTTP POST fails.
    pub fn create_child_note(
        &self,
        parent_item_key: &str,
        note_html: &str,
        tags: &[String],
    ) -> Result<(), ZoteroError> {
        let url = self.source.post_items_url();
        let payload = build_child_note_payload(parent_item_key, note_html, tags);
        let mut req = ureq::post(&url)
            .header("Zotero-API-Version", "3")
            .header("Content-Type", "application/json");
        if let Some(key) = self.source.api_key() {
            req = req.header("Zotero-API-Key", key);
        }
        req.send(payload.to_string())
            .map_err(|e| ZoteroError(format!("post note failed: {e}")))?;
        Ok(())
    }
}

/// Builds the JSON payload for creating a child note in Zotero Web/Local API v3.
#[must_use]
pub fn build_child_note_payload(parent_item_key: &str, note_html: &str, tags: &[String]) -> Value {
    let z_tags: Vec<Value> = tags.iter().map(|t| json!({"tag": t})).collect();
    json!([{
        "itemType": "note",
        "parentItem": parent_item_key,
        "note": note_html,
        "tags": z_tags,
    }])
}

/// Parses a Zotero API v3 `/items` JSON array into [`ZoteroItem`]s.
///
/// # Errors
///
/// Returns [`ZoteroError`] if `json_str` is not valid Zotero item JSON.
pub fn parse_zotero_items_json(json_str: &str) -> Result<Vec<ZoteroItem>, ZoteroError> {
    let val: Value =
        serde_json::from_str(json_str).map_err(|e| ZoteroError(format!("invalid JSON: {e}")))?;
    let arr = val
        .as_array()
        .ok_or_else(|| ZoteroError("expected JSON array of Zotero items".to_owned()))?;
    let mut items = Vec::with_capacity(arr.len());

    for raw in arr {
        let key = raw
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let version = raw.get("version").and_then(Value::as_u64).unwrap_or(0);
        let Some(data) = raw.get("data").and_then(Value::as_object) else {
            continue;
        };
        let get_str = |k: &str| {
            data.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let item_type = get_str("itemType").unwrap_or_else(|| "document".to_owned());
        if item_type == "attachment" || item_type == "note" {
            continue;
        }
        let kind = match item_type.as_str() {
            "journalArticle" => EntryKind::Article,
            "book" => EntryKind::Book,
            "conferencePaper" => EntryKind::InProceedings,
            "bookSection" => EntryKind::InCollection,
            "report" => EntryKind::TechReport,
            "thesis" => EntryKind::Thesis,
            _ => EntryKind::Misc,
        };
        let authors = data
            .get("creators")
            .and_then(Value::as_array)
            .map(|creators| {
                creators
                    .iter()
                    .filter_map(|c| {
                        let last = c.get("lastName").and_then(Value::as_str).unwrap_or("");
                        let first = c.get("firstName").and_then(Value::as_str).unwrap_or("");
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
                        if !last.is_empty() || !first.is_empty() {
                            Some(AuthorName {
                                family: last.to_owned(),
                                given: first.to_owned(),
                            })
                        } else if !name.is_empty() {
                            Some(AuthorName::parse(name))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let year = get_str("date").and_then(|d| {
            d.split(|c: char| !c.is_ascii_digit())
                .find(|tok| tok.len() == 4)
                .and_then(|y| y.parse::<i32>().ok())
        });
        let collections = data
            .get("collections")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let tags = data
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.get("tag").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let venue = get_str("publicationTitle");
        let (journal, booktitle) = if kind == EntryKind::InProceedings {
            (None, get_str("proceedingsTitle").or(venue))
        } else {
            (venue, None)
        };
        let pdf_path = raw
            .get("links")
            .and_then(|l| l.get("enclosure"))
            .and_then(|e| e.get("href"))
            .and_then(Value::as_str)
            .and_then(|href| href.strip_prefix("file://"))
            .map(PathBuf::from);

        items.push(ZoteroItem {
            item_key: key.clone(),
            version,
            citation: CitationEntry {
                key,
                kind,
                title: get_str("title").unwrap_or_default(),
                authors,
                year,
                journal,
                booktitle,
                volume: get_str("volume"),
                issue: get_str("issue"),
                pages: get_str("pages"),
                publisher: get_str("publisher"),
                doi: get_str("DOI"),
                arxiv_id: None,
                isbn: get_str("ISBN"),
                url: get_str("url"),
                abstract_text: get_str("abstractNote"),
            },
            collections,
            tags,
            pdf_path,
        });
    }

    Ok(items)
}

/// Reads Zotero bibliographic items and local PDF attachment paths directly from a Zotero
/// SQLite database (`zotero.sqlite`) opened read-only (plan F.11).
///
/// # Errors
///
/// Returns [`ZoteroError`] if the database cannot be opened or queried.
pub fn read_local_sqlite(
    sqlite_path: &Path,
    storage_dir: &Path,
) -> Result<Vec<ZoteroItem>, ZoteroError> {
    let conn = Connection::open_with_flags(sqlite_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| ZoteroError(format!("cannot open {}: {e}", sqlite_path.display())))?;

    let mut stmt = conn
        .prepare(
            "SELECT i.itemID, i.key, it.typeName, COALESCE(i.version, 0)
             FROM items i
             JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
             WHERE it.typeName NOT IN ('attachment', 'note', 'annotation')
               AND i.itemID NOT IN (SELECT itemID FROM deletedItems)",
        )
        .map_err(|e| ZoteroError(format!("query items: {e}")))?;

    let raw_items: Vec<(i64, String, String, u64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
            ))
        })
        .map_err(|e| ZoteroError(format!("read items: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| ZoteroError(format!("collect items: {e}")))?;

    let fields_map = load_item_fields(&conn)?;
    let creators_map = load_item_creators(&conn)?;
    let attachments_map = load_item_attachments(&conn, storage_dir)?;
    let collections_map = load_item_collections(&conn)?;
    let tags_map = load_item_tags(&conn)?;

    let mut out = Vec::with_capacity(raw_items.len());
    for (item_id, key, type_name, version) in raw_items {
        let fields = fields_map.get(&item_id);
        let field = |name: &str| fields.and_then(|m| m.get(name)).cloned();
        let kind = match type_name.as_str() {
            "journalArticle" => EntryKind::Article,
            "book" => EntryKind::Book,
            "conferencePaper" => EntryKind::InProceedings,
            "bookSection" => EntryKind::InCollection,
            "report" => EntryKind::TechReport,
            "thesis" => EntryKind::Thesis,
            _ => EntryKind::Misc,
        };
        let year = field("date").and_then(|d| {
            d.split(|c: char| !c.is_ascii_digit())
                .find(|tok| tok.len() == 4)
                .and_then(|y| y.parse::<i32>().ok())
        });
        let venue = field("publicationTitle");
        let (journal, booktitle) = if kind == EntryKind::InProceedings {
            (None, field("proceedingsTitle").or(venue))
        } else {
            (venue, None)
        };

        out.push(ZoteroItem {
            item_key: key.clone(),
            version,
            citation: CitationEntry {
                key,
                kind,
                title: field("title").unwrap_or_default(),
                authors: creators_map.get(&item_id).cloned().unwrap_or_default(),
                year,
                journal,
                booktitle,
                volume: field("volume"),
                issue: field("issue"),
                pages: field("pages"),
                publisher: field("publisher"),
                doi: field("DOI"),
                arxiv_id: None,
                isbn: field("ISBN"),
                url: field("url"),
                abstract_text: field("abstractNote"),
            },
            collections: collections_map.get(&item_id).cloned().unwrap_or_default(),
            tags: tags_map.get(&item_id).cloned().unwrap_or_default(),
            pdf_path: attachments_map.get(&item_id).cloned(),
        });
    }

    Ok(out)
}

fn load_item_fields(
    conn: &Connection,
) -> Result<BTreeMap<i64, BTreeMap<String, String>>, ZoteroError> {
    let mut stmt = conn
        .prepare(
            "SELECT id.itemID, f.fieldName, idv.value
             FROM itemData id
             JOIN fields f ON f.fieldID = id.fieldID
             JOIN itemDataValues idv ON idv.valueID = id.valueID",
        )
        .map_err(|e| ZoteroError(format!("query fields: {e}")))?;
    let mut map: BTreeMap<i64, BTreeMap<String, String>> = BTreeMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| ZoteroError(format!("read fields: {e}")))?;
    for row in rows {
        let (item_id, name, val) = row.map_err(|e| ZoteroError(format!("field row: {e}")))?;
        map.entry(item_id).or_default().insert(name, val);
    }
    Ok(map)
}

fn load_item_creators(conn: &Connection) -> Result<BTreeMap<i64, Vec<AuthorName>>, ZoteroError> {
    let mut stmt = conn
        .prepare(
            "SELECT ic.itemID, c.firstName, c.lastName
             FROM itemCreators ic
             JOIN creators c ON c.creatorID = ic.creatorID
             ORDER BY ic.itemID, ic.orderIndex",
        )
        .map_err(|e| ZoteroError(format!("query creators: {e}")))?;
    let mut map: BTreeMap<i64, Vec<AuthorName>> = BTreeMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .map_err(|e| ZoteroError(format!("read creators: {e}")))?;
    for row in rows {
        let (item_id, given, family) = row.map_err(|e| ZoteroError(format!("creator row: {e}")))?;
        map.entry(item_id)
            .or_default()
            .push(AuthorName { family, given });
    }
    Ok(map)
}

fn load_item_attachments(
    conn: &Connection,
    storage_dir: &Path,
) -> Result<BTreeMap<i64, PathBuf>, ZoteroError> {
    let mut stmt = conn
        .prepare(
            "SELECT ia.parentItemID, i.key, ia.path, ia.contentType
             FROM itemAttachments ia
             JOIN items i ON i.itemID = ia.itemID
             WHERE ia.parentItemID IS NOT NULL",
        )
        .map_err(|e| ZoteroError(format!("query attachments: {e}")))?;
    let mut map = BTreeMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| ZoteroError(format!("read attachments: {e}")))?;
    for row in rows {
        let (parent_id, attach_key, path_opt, mime_opt) =
            row.map_err(|e| ZoteroError(format!("attachment row: {e}")))?;
        let Some(raw_path) = path_opt else {
            continue;
        };
        let is_pdf = mime_opt.as_deref() == Some("application/pdf")
            || raw_path.to_ascii_lowercase().ends_with(".pdf");
        if !is_pdf {
            continue;
        }
        let resolved = if let Some(rel) = raw_path.strip_prefix("storage:") {
            storage_dir.join(&attach_key).join(rel)
        } else {
            PathBuf::from(raw_path)
        };
        map.entry(parent_id).or_insert(resolved);
    }
    Ok(map)
}

fn load_item_collections(conn: &Connection) -> Result<BTreeMap<i64, Vec<String>>, ZoteroError> {
    let mut stmt = conn
        .prepare(
            "SELECT ci.itemID, c.collectionName
             FROM collectionItems ci
             JOIN collections c ON c.collectionID = ci.collectionID",
        )
        .map_err(|e| ZoteroError(format!("query collections: {e}")))?;
    let mut map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| ZoteroError(format!("read collections: {e}")))?;
    for row in rows {
        let (item_id, name) = row.map_err(|e| ZoteroError(format!("col row: {e}")))?;
        map.entry(item_id).or_default().push(name);
    }
    Ok(map)
}

fn load_item_tags(conn: &Connection) -> Result<BTreeMap<i64, Vec<String>>, ZoteroError> {
    let mut stmt = conn
        .prepare(
            "SELECT it.itemID, t.name
             FROM itemTags it
             JOIN tags t ON t.tagID = it.tagID",
        )
        .map_err(|e| ZoteroError(format!("query tags: {e}")))?;
    let mut map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| ZoteroError(format!("read tags: {e}")))?;
    for row in rows {
        let (item_id, tag) = row.map_err(|e| ZoteroError(format!("tag row: {e}")))?;
        map.entry(item_id).or_default().push(tag);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zotero_api_v3_json_and_builds_child_note() {
        let json = r#"[
            {
                "key": "ABCD1234",
                "version": 42,
                "links": { "enclosure": { "href": "file:///C:/Zotero/storage/XYZ9/paper.pdf" } },
                "data": {
                    "itemType": "journalArticle",
                    "title": "Deep PDF Indexing",
                    "creators": [{"firstName": "Ada", "lastName": "Lovelace"}],
                    "date": "2025-04-01",
                    "publicationTitle": "Journal of Documents",
                    "DOI": "10.1000/zot1",
                    "collections": ["COL1"],
                    "tags": [{"tag": "pdf"}]
                }
            }
        ]"#;
        let items = parse_zotero_items_json(json).expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_key, "ABCD1234");
        assert_eq!(items[0].citation.title, "Deep PDF Indexing");
        assert_eq!(items[0].citation.year, Some(2025));
        assert_eq!(
            items[0].pdf_path,
            Some(PathBuf::from("/C:/Zotero/storage/XYZ9/paper.pdf"))
        );

        let payload =
            build_child_note_payload("ABCD1234", "<p>Highlight quote</p>", &["note".into()]);
        assert_eq!(payload[0]["parentItem"], "ABCD1234");
    }

    #[test]
    fn reads_local_zotero_sqlite_schema_read_only() {
        let dir = std::env::temp_dir().join(format!("excellent-zot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let db = dir.join("zotero.sqlite");
        let storage = dir.join("storage");
        {
            let conn = Connection::open(&db).expect("open");
            conn.execute_batch(
                "CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT);
                 CREATE TABLE items (itemID INTEGER PRIMARY KEY, key TEXT, itemTypeID INTEGER, version INTEGER);
                 CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY);
                 CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT);
                 CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value TEXT);
                 CREATE TABLE itemData (itemID INTEGER, fieldID INTEGER, valueID INTEGER);
                 CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT);
                 CREATE TABLE itemCreators (itemID INTEGER, creatorID INTEGER, orderIndex INTEGER);
                 CREATE TABLE itemAttachments (itemID INTEGER PRIMARY KEY, parentItemID INTEGER, path TEXT, contentType TEXT);
                 CREATE TABLE collections (collectionID INTEGER PRIMARY KEY, collectionName TEXT);
                 CREATE TABLE collectionItems (collectionID INTEGER, itemID INTEGER);
                 CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE itemTags (itemID INTEGER, tagID INTEGER);

                 INSERT INTO itemTypes VALUES (1, 'journalArticle'), (2, 'attachment');
                 INSERT INTO items VALUES (10, 'ITEMKEY1', 1, 7), (11, 'ATTKEY99', 2, 7);
                 INSERT INTO fields VALUES (1, 'title'), (2, 'date'), (3, 'DOI');
                 INSERT INTO itemDataValues VALUES (1, 'Zotero SQLite Test'), (2, '2024'), (3, '10.1000/sql');
                 INSERT INTO itemData VALUES (10, 1, 1), (10, 2, 2), (10, 3, 3);
                 INSERT INTO creators VALUES (1, 'Grace', 'Hopper');
                 INSERT INTO itemCreators VALUES (10, 1, 0);
                 INSERT INTO itemAttachments VALUES (11, 10, 'storage:hopper.pdf', 'application/pdf');
                 INSERT INTO collections VALUES (1, 'Compilers');
                 INSERT INTO collectionItems VALUES (1, 10);
                 INSERT INTO tags VALUES (1, 'classic');
                 INSERT INTO itemTags VALUES (10, 1);",
            )
            .expect("seed");
        }

        let items = read_local_sqlite(&db, &storage).expect("read sqlite");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].citation.title, "Zotero SQLite Test");
        assert_eq!(items[0].citation.authors[0].family, "Hopper");
        assert_eq!(items[0].collections, ["Compilers"]);
        assert_eq!(items[0].tags, ["classic"]);
        assert_eq!(
            items[0].pdf_path,
            Some(storage.join("ATTKEY99").join("hopper.pdf"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
