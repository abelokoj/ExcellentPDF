//! Citation parsing, formatting, key generation, and reference detection (plan F.10).
//!
//! Supports BibTeX, RIS, and CSL-JSON import/export, formatted bibliography styles (APA, IEEE,
//! Chicago), configurable citation key generation, and extraction of DOI / arXiv references from
//! document text.

pub mod zotero;

use std::collections::{BTreeSet, HashMap};
use std::fmt::{self, Write as _};

use serde_json::{Value, json};

use crate::library::metadata::{extract_arxiv_id, extract_doi};

/// Type of bibliographic work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EntryKind {
    /// Journal article (`@article`, `JOUR`).
    #[default]
    Article,
    /// Book (`@book`, `BOOK`).
    Book,
    /// Conference / proceedings paper (`@inproceedings`, `CONF`).
    InProceedings,
    /// Book chapter (`@incollection`, `CHAP`).
    InCollection,
    /// Technical report or preprint (`@techreport`, `RPRT`).
    TechReport,
    /// PhD or Master's thesis (`@phdthesis`, `THES`).
    Thesis,
    /// Miscellaneous / online resource (`@misc`, `GEN`).
    Misc,
}

impl EntryKind {
    /// BibTeX entry type name.
    #[must_use]
    pub const fn bibtex_type(self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::Book => "book",
            Self::InProceedings => "inproceedings",
            Self::InCollection => "incollection",
            Self::TechReport => "techreport",
            Self::Thesis => "phdthesis",
            Self::Misc => "misc",
        }
    }

    /// RIS `TY` tag code.
    #[must_use]
    pub const fn ris_type(self) -> &'static str {
        match self {
            Self::Article => "JOUR",
            Self::Book => "BOOK",
            Self::InProceedings => "CONF",
            Self::InCollection => "CHAP",
            Self::TechReport => "RPRT",
            Self::Thesis => "THES",
            Self::Misc => "GEN",
        }
    }

    /// CSL-JSON `type` string.
    #[must_use]
    pub const fn csl_type(self) -> &'static str {
        match self {
            Self::Article => "article-journal",
            Self::Book => "book",
            Self::InProceedings => "paper-conference",
            Self::InCollection => "chapter",
            Self::TechReport => "report",
            Self::Thesis => "thesis",
            Self::Misc => "document",
        }
    }

    /// Parses from BibTeX, RIS, or CSL type names.
    #[must_use]
    pub fn from_any(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "article" | "jour" | "article-journal" => Self::Article,
            "book" => Self::Book,
            "inproceedings" | "proceedings" | "conf" | "cpaper" | "paper-conference" => {
                Self::InProceedings
            }
            "incollection" | "inbook" | "chap" | "chapter" => Self::InCollection,
            "techreport" | "rprt" | "report" => Self::TechReport,
            "phdthesis" | "mastersthesis" | "thes" | "thesis" => Self::Thesis,
            _ => Self::Misc,
        }
    }
}

/// Structured author name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthorName {
    /// Family / surname.
    pub family: String,
    /// Given / first names.
    pub given: String,
}

impl AuthorName {
    /// Parses `"Last, First"` or `"First Middle Last"`.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some((family, given)) = trimmed.split_once(',') {
            Self {
                family: family.trim().to_owned(),
                given: given.trim().to_owned(),
            }
        } else {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            match parts.as_slice() {
                [] => Self::default(),
                [single] => Self {
                    family: (*single).to_owned(),
                    given: String::new(),
                },
                [given @ .., family] => Self {
                    family: (*family).to_owned(),
                    given: given.join(" "),
                },
            }
        }
    }

    /// Formats as `"Family, Given"` for BibTeX/RIS.
    #[must_use]
    pub fn to_family_first(&self) -> String {
        if self.given.is_empty() {
            self.family.clone()
        } else {
            format!("{}, {}", self.family, self.given)
        }
    }

    /// Formats as `"Given Family"`.
    #[must_use]
    pub fn to_full_name(&self) -> String {
        if self.given.is_empty() {
            self.family.clone()
        } else {
            format!("{} {}", self.given, self.family)
        }
    }
}

/// Canonical bibliographic citation record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CitationEntry {
    /// Citation key (e.g. `vaswani2017attention`).
    pub key: String,
    /// Work type.
    pub kind: EntryKind,
    /// Title of the work.
    pub title: String,
    /// Authors in order.
    pub authors: Vec<AuthorName>,
    /// Publication year.
    pub year: Option<i32>,
    /// Journal name.
    pub journal: Option<String>,
    /// Conference proceedings / book title.
    pub booktitle: Option<String>,
    /// Volume.
    pub volume: Option<String>,
    /// Issue / number.
    pub issue: Option<String>,
    /// Page range (`123--135` or `123-135`).
    pub pages: Option<String>,
    /// Publisher name.
    pub publisher: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// arXiv identifier.
    pub arxiv_id: Option<String>,
    /// ISBN.
    pub isbn: Option<String>,
    /// URL.
    pub url: Option<String>,
    /// Abstract.
    pub abstract_text: Option<String>,
}

/// Citation key formatting pattern (plan F.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyFormat {
    /// `lastname2017firstword` (default, standard academic style).
    #[default]
    AuthorYearTitle,
    /// `lastname2017`.
    AuthorYear,
    /// `lastname:2017`.
    AuthorColonYear,
}

/// Output bibliography formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CitationStyle {
    /// APA 7th edition plain-text reference.
    Apa,
    /// IEEE numbered-style plain-text reference.
    Ieee,
    /// Chicago notes-bibliography style.
    Chicago,
}

/// Error returned when parsing BibTeX, RIS, or CSL-JSON fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationParseError(pub String);

impl fmt::Display for CitationParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "citation parse error: {}", self.0)
    }
}

impl std::error::Error for CitationParseError {}

/// Generates a deterministic ASCII citation key from `entry` according to `format`.
#[must_use]
pub fn generate_citation_key(entry: &CitationEntry, format: KeyFormat) -> String {
    let author_slug = entry
        .authors
        .first()
        .map(|a| slugify_ascii(&a.family))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "anon".to_owned());
    let year_part = entry
        .year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "nd".to_owned());
    match format {
        KeyFormat::AuthorYear => format!("{author_slug}{year_part}"),
        KeyFormat::AuthorColonYear => format!("{author_slug}:{year_part}"),
        KeyFormat::AuthorYearTitle => {
            let word = first_significant_title_word(&entry.title);
            format!("{author_slug}{year_part}{word}")
        }
    }
}

/// Assigns unique citation keys to `entries` using `format`, appending `a`, `b`, ... on collision.
pub fn assign_unique_keys(entries: &mut [CitationEntry], format: KeyFormat) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        let base = if entry.key.trim().is_empty() {
            generate_citation_key(entry, format)
        } else {
            entry.key.trim().to_owned()
        };
        let n = counts.entry(base.clone()).or_insert(0);
        if *n == 0 {
            entry.key = base;
        } else {
            let suffix = char::from(b'a' + u8::try_from((*n - 1).min(25)).unwrap_or(25));
            entry.key = format!("{base}{suffix}");
        }
        *n += 1;
    }
}

/// Formats a single [`CitationEntry`] in the requested [`CitationStyle`].
#[must_use]
pub fn format_citation(entry: &CitationEntry, style: CitationStyle) -> String {
    match style {
        CitationStyle::Apa => format_apa(entry),
        CitationStyle::Ieee => format_ieee(entry),
        CitationStyle::Chicago => format_chicago(entry),
    }
}

fn format_apa(entry: &CitationEntry) -> String {
    let authors = if entry.authors.is_empty() {
        "Unknown".to_owned()
    } else {
        entry
            .authors
            .iter()
            .map(|a| {
                let initials: String = a
                    .given
                    .split_whitespace()
                    .filter_map(|part| part.chars().next())
                    .map(|c| format!("{c}."))
                    .collect::<Vec<_>>()
                    .join(" ");
                if initials.is_empty() {
                    a.family.clone()
                } else {
                    format!("{}, {initials}", a.family)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let year = entry
        .year
        .map_or_else(|| "n.d.".to_owned(), |y| y.to_string());
    let mut out = format!("{authors} ({year}). {}.", entry.title.trim_end_matches('.'));
    if let Some(venue) = entry.journal.as_deref().or(entry.booktitle.as_deref()) {
        let _ = write!(out, " {venue}");
        if let Some(vol) = &entry.volume {
            let _ = write!(out, ", {vol}");
            if let Some(issue) = &entry.issue {
                let _ = write!(out, "({issue})");
            }
        }
        if let Some(pages) = &entry.pages {
            let _ = write!(out, ", {pages}");
        }
        out.push('.');
    }
    if let Some(doi) = &entry.doi {
        let _ = write!(out, " https://doi.org/{doi}");
    } else if let Some(arxiv) = &entry.arxiv_id {
        let _ = write!(out, " https://arxiv.org/abs/{arxiv}");
    }
    out
}

fn format_ieee(entry: &CitationEntry) -> String {
    let authors = if entry.authors.is_empty() {
        "Unknown".to_owned()
    } else {
        entry
            .authors
            .iter()
            .map(|a| {
                let initials: String = a
                    .given
                    .split_whitespace()
                    .filter_map(|p| p.chars().next())
                    .map(|c| format!("{c}."))
                    .collect::<Vec<_>>()
                    .join(" ");
                if initials.is_empty() {
                    a.family.clone()
                } else {
                    format!("{initials} {}", a.family)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = format!("{authors}, \"{},\"", entry.title.trim_end_matches('.'));
    if let Some(venue) = entry.journal.as_deref().or(entry.booktitle.as_deref()) {
        let _ = write!(out, " {venue},");
    }
    if let Some(vol) = &entry.volume {
        let _ = write!(out, " vol. {vol},");
    }
    if let Some(issue) = &entry.issue {
        let _ = write!(out, " no. {issue},");
    }
    if let Some(pages) = &entry.pages {
        let _ = write!(out, " pp. {pages},");
    }
    if let Some(year) = entry.year {
        let _ = write!(out, " {year}.");
    } else {
        out.push('.');
    }
    out
}

fn format_chicago(entry: &CitationEntry) -> String {
    let authors = if entry.authors.is_empty() {
        "Unknown".to_owned()
    } else {
        entry
            .authors
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if i == 0 {
                    a.to_family_first()
                } else {
                    a.to_full_name()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let year = entry
        .year
        .map_or_else(|| "n.d.".to_owned(), |y| y.to_string());
    let mut out = format!(
        "{authors}. {year}. \"{}\".",
        entry.title.trim_end_matches('.')
    );
    if let Some(venue) = entry.journal.as_deref().or(entry.booktitle.as_deref()) {
        let _ = write!(out, " {venue}");
    }
    if let Some(pages) = &entry.pages {
        let _ = write!(out, ": {pages}");
    }
    out.push('.');
    out
}

/// Exports `entries` as a `biber`-compatible BibTeX bibliography string (plan F.10).
#[must_use]
pub fn export_bibtex(entries: &[CitationEntry]) -> String {
    let mut out = String::new();
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let key = if entry.key.trim().is_empty() {
            generate_citation_key(entry, KeyFormat::AuthorYearTitle)
        } else {
            entry.key.trim().to_owned()
        };
        let _ = writeln!(out, "@{}{{{key},", entry.kind.bibtex_type());
        if !entry.title.is_empty() {
            let _ = writeln!(out, "  title = {{{}}},", escape_bibtex_value(&entry.title));
        }
        if !entry.authors.is_empty() {
            let authors = entry
                .authors
                .iter()
                .map(AuthorName::to_family_first)
                .collect::<Vec<_>>()
                .join(" and ");
            let _ = writeln!(out, "  author = {{{authors}}},");
        }
        if let Some(year) = entry.year {
            let _ = writeln!(out, "  year = {{{year}}},");
        }
        if let Some(journal) = &entry.journal {
            let _ = writeln!(out, "  journal = {{{}}},", escape_bibtex_value(journal));
        }
        if let Some(booktitle) = &entry.booktitle {
            let _ = writeln!(out, "  booktitle = {{{}}},", escape_bibtex_value(booktitle));
        }
        if let Some(volume) = &entry.volume {
            let _ = writeln!(out, "  volume = {{{volume}}},");
        }
        if let Some(issue) = &entry.issue {
            let _ = writeln!(out, "  number = {{{issue}}},");
        }
        if let Some(pages) = &entry.pages {
            let normalized_pages = pages.replace('-', "--").replace("----", "--");
            let _ = writeln!(out, "  pages = {{{normalized_pages}}},");
        }
        if let Some(publisher) = &entry.publisher {
            let _ = writeln!(out, "  publisher = {{{}}},", escape_bibtex_value(publisher));
        }
        if let Some(doi) = &entry.doi {
            let _ = writeln!(out, "  doi = {{{doi}}},");
        }
        if let Some(arxiv) = &entry.arxiv_id {
            let _ = writeln!(out, "  eprint = {{{arxiv}}},");
            let _ = writeln!(out, "  archivePrefix = {{arXiv}},");
        }
        if let Some(isbn) = &entry.isbn {
            let _ = writeln!(out, "  isbn = {{{isbn}}},");
        }
        if let Some(url) = &entry.url {
            let _ = writeln!(out, "  url = {{{url}}},");
        }
        out.push_str("}\n");
    }
    out
}

/// Parses one or more BibTeX entries from `input`.
///
/// # Errors
///
/// Returns [`CitationParseError`] if entry delimiters or braces are malformed.
pub fn parse_bibtex(input: &str) -> Result<Vec<CitationEntry>, CitationParseError> {
    let mut entries = Vec::new();
    let mut idx = 0;
    let bytes = input.as_bytes();

    while let Some(rel_at) = input[idx..].find('@') {
        let start = idx + rel_at + 1;
        let Some(open_rel) = input[start..].find(['{', '(']) else {
            return Err(CitationParseError("missing `{` after `@type`".to_owned()));
        };
        let entry_type = input[start..start + open_rel].trim().to_ascii_lowercase();
        let open_pos = start + open_rel;
        let close_char = if bytes[open_pos] == b'{' { '}' } else { ')' };
        let body_start = open_pos + 1;

        let mut depth = 1_i32;
        let mut found_end = None;
        for (off, ch) in input[body_start..].char_indices() {
            if ch == '{' || (close_char == ')' && ch == '(') {
                depth += 1;
            } else if ch == close_char {
                depth -= 1;
                if depth == 0 {
                    found_end = Some(body_start + off);
                    break;
                }
            }
        }
        let body_end = found_end
            .ok_or_else(|| CitationParseError(format!("unclosed BibTeX entry `@{entry_type}`")))?;
        idx = body_end + 1;

        if matches!(entry_type.as_str(), "comment" | "preamble" | "string") {
            continue;
        }
        let body = &input[body_start..body_end];
        let Some((key_raw, fields_raw)) = body.split_once(',') else {
            return Err(CitationParseError(format!(
                "missing comma after citation key in `@{entry_type}`"
            )));
        };

        let mut entry = CitationEntry {
            key: key_raw.trim().to_owned(),
            kind: EntryKind::from_any(&entry_type),
            ..CitationEntry::default()
        };

        for (field_name, field_val) in parse_bibtex_fields(fields_raw)? {
            match field_name.as_str() {
                "title" => entry.title = field_val,
                "author" => {
                    entry.authors = split_bibtex_authors(&field_val)
                        .into_iter()
                        .map(|a| AuthorName::parse(&a))
                        .collect();
                }
                "year" => entry.year = field_val.trim().parse::<i32>().ok(),
                "journal" | "journaltitle" => entry.journal = Some(field_val),
                "booktitle" => entry.booktitle = Some(field_val),
                "volume" => entry.volume = Some(field_val),
                "number" | "issue" => entry.issue = Some(field_val),
                "pages" => entry.pages = Some(field_val.replace("--", "-")),
                "publisher" => entry.publisher = Some(field_val),
                "doi" => entry.doi = Some(field_val),
                "eprint" => entry.arxiv_id = Some(field_val),
                "isbn" => entry.isbn = Some(field_val),
                "url" => entry.url = Some(field_val),
                "abstract" => entry.abstract_text = Some(field_val),
                _ => {}
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Exports `entries` as a standard RIS bibliography string (plan F.10).
#[must_use]
pub fn export_ris(entries: &[CitationEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let _ = writeln!(out, "TY  - {}", entry.kind.ris_type());
        if !entry.key.is_empty() {
            let _ = writeln!(out, "ID  - {}", entry.key);
        }
        if !entry.title.is_empty() {
            let _ = writeln!(out, "TI  - {}", entry.title);
        }
        for author in &entry.authors {
            let _ = writeln!(out, "AU  - {}", author.to_family_first());
        }
        if let Some(year) = entry.year {
            let _ = writeln!(out, "PY  - {year}");
        }
        if let Some(journal) = &entry.journal {
            let _ = writeln!(out, "JO  - {journal}");
        }
        if let Some(booktitle) = &entry.booktitle {
            let _ = writeln!(out, "T2  - {booktitle}");
        }
        if let Some(volume) = &entry.volume {
            let _ = writeln!(out, "VL  - {volume}");
        }
        if let Some(issue) = &entry.issue {
            let _ = writeln!(out, "IS  - {issue}");
        }
        if let Some(pages) = &entry.pages {
            if let Some((sp, ep)) = pages.split_once('-') {
                let _ = writeln!(out, "SP  - {}", sp.trim());
                let _ = writeln!(out, "EP  - {}", ep.trim());
            } else {
                let _ = writeln!(out, "SP  - {pages}");
            }
        }
        if let Some(publisher) = &entry.publisher {
            let _ = writeln!(out, "PB  - {publisher}");
        }
        if let Some(doi) = &entry.doi {
            let _ = writeln!(out, "DO  - {doi}");
        }
        if let Some(isbn) = &entry.isbn {
            let _ = writeln!(out, "SN  - {isbn}");
        }
        if let Some(url) = &entry.url {
            let _ = writeln!(out, "UR  - {url}");
        }
        if let Some(abs) = &entry.abstract_text {
            let _ = writeln!(out, "AB  - {abs}");
        }
        out.push_str("ER  -\n\n");
    }
    out
}

/// Parses RIS records from `input`.
#[must_use]
pub fn parse_ris(input: &str) -> Vec<CitationEntry> {
    let mut entries = Vec::new();
    let mut current: Option<CitationEntry> = None;
    let mut start_page: Option<String> = None;
    let mut end_page: Option<String> = None;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((tag, val)) = parse_ris_line(trimmed) {
            match tag {
                "TY" => {
                    current = Some(CitationEntry {
                        kind: EntryKind::from_any(val),
                        ..CitationEntry::default()
                    });
                    start_page = None;
                    end_page = None;
                }
                "ER" => {
                    if let Some(mut entry) = current.take() {
                        if let Some(sp) = start_page.take() {
                            entry.pages = Some(match end_page.take() {
                                Some(ep) => format!("{sp}-{ep}"),
                                None => sp,
                            });
                        }
                        entries.push(entry);
                    }
                }
                _ => {
                    if let Some(entry) = current.as_mut() {
                        match tag {
                            "ID" => entry.key = val.to_owned(),
                            "TI" | "T1" => entry.title = val.to_owned(),
                            "AU" | "A1" => entry.authors.push(AuthorName::parse(val)),
                            "PY" | "Y1" => {
                                entry.year = val.get(..4).and_then(|y| y.parse::<i32>().ok());
                            }
                            "JO" | "JF" | "JA" => entry.journal = Some(val.to_owned()),
                            "T2" | "BT" => entry.booktitle = Some(val.to_owned()),
                            "VL" => entry.volume = Some(val.to_owned()),
                            "IS" => entry.issue = Some(val.to_owned()),
                            "SP" => start_page = Some(val.to_owned()),
                            "EP" => end_page = Some(val.to_owned()),
                            "PB" => entry.publisher = Some(val.to_owned()),
                            "DO" => entry.doi = Some(val.to_owned()),
                            "SN" => entry.isbn = Some(val.to_owned()),
                            "UR" => entry.url = Some(val.to_owned()),
                            "AB" | "N2" => entry.abstract_text = Some(val.to_owned()),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    entries
}

/// Exports `entries` as a CSL-JSON array string (plan F.10).
#[must_use]
pub fn export_csl_json(entries: &[CitationEntry]) -> String {
    let items: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let mut obj = serde_json::Map::new();
            let key = if entry.key.is_empty() {
                generate_citation_key(entry, KeyFormat::AuthorYearTitle)
            } else {
                entry.key.clone()
            };
            obj.insert("id".into(), json!(key));
            obj.insert("type".into(), json!(entry.kind.csl_type()));
            if !entry.title.is_empty() {
                obj.insert("title".into(), json!(entry.title));
            }
            if !entry.authors.is_empty() {
                let authors: Vec<Value> = entry
                    .authors
                    .iter()
                    .map(|a| json!({"family": a.family, "given": a.given}))
                    .collect();
                obj.insert("author".into(), Value::Array(authors));
            }
            if let Some(year) = entry.year {
                obj.insert("issued".into(), json!({"date-parts": [[year]]}));
            }
            if let Some(venue) = entry.journal.as_ref().or(entry.booktitle.as_ref()) {
                obj.insert("container-title".into(), json!(venue));
            }
            if let Some(vol) = &entry.volume {
                obj.insert("volume".into(), json!(vol));
            }
            if let Some(issue) = &entry.issue {
                obj.insert("issue".into(), json!(issue));
            }
            if let Some(pages) = &entry.pages {
                obj.insert("page".into(), json!(pages));
            }
            if let Some(publisher) = &entry.publisher {
                obj.insert("publisher".into(), json!(publisher));
            }
            if let Some(doi) = &entry.doi {
                obj.insert("DOI".into(), json!(doi));
            }
            if let Some(isbn) = &entry.isbn {
                obj.insert("ISBN".into(), json!(isbn));
            }
            if let Some(url) = &entry.url {
                obj.insert("URL".into(), json!(url));
            }
            if let Some(abs) = &entry.abstract_text {
                obj.insert("abstract".into(), json!(abs));
            }
            Value::Object(obj)
        })
        .collect();
    serde_json::to_string_pretty(&Value::Array(items)).unwrap_or_else(|_| "[]".to_owned())
}

/// Parses a CSL-JSON array string into [`CitationEntry`] records.
///
/// # Errors
///
/// Returns [`CitationParseError`] if `json_str` is not a JSON array.
pub fn parse_csl_json(json_str: &str) -> Result<Vec<CitationEntry>, CitationParseError> {
    let val: Value = serde_json::from_str(json_str)
        .map_err(|e| CitationParseError(format!("invalid JSON: {e}")))?;
    let arr = val
        .as_array()
        .ok_or_else(|| CitationParseError("expected a CSL-JSON array".to_owned()))?;
    let mut entries = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let get_str = |k: &str| obj.get(k).and_then(Value::as_str).map(str::to_owned);
        let kind = get_str("type")
            .map(|t| EntryKind::from_any(&t))
            .unwrap_or_default();
        let authors = obj
            .get("author")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|a| {
                        let family = a.get("family").and_then(Value::as_str).unwrap_or("");
                        let given = a.get("given").and_then(Value::as_str).unwrap_or("");
                        (!family.is_empty() || !given.is_empty()).then(|| AuthorName {
                            family: family.to_owned(),
                            given: given.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let year = obj
            .get("issued")
            .and_then(|i| i.get("date-parts"))
            .and_then(Value::as_array)
            .and_then(|outer| outer.first())
            .and_then(Value::as_array)
            .and_then(|inner| inner.first())
            .and_then(Value::as_i64)
            .and_then(|y| i32::try_from(y).ok());
        let container = get_str("container-title");
        let (journal, booktitle) = if kind == EntryKind::InProceedings {
            (None, container)
        } else {
            (container, None)
        };
        entries.push(CitationEntry {
            key: get_str("id").unwrap_or_default(),
            kind,
            title: get_str("title").unwrap_or_default(),
            authors,
            year,
            journal,
            booktitle,
            volume: get_str("volume"),
            issue: get_str("issue"),
            pages: get_str("page"),
            publisher: get_str("publisher"),
            doi: get_str("DOI"),
            arxiv_id: None,
            isbn: get_str("ISBN"),
            url: get_str("URL"),
            abstract_text: get_str("abstract"),
        });
    }
    Ok(entries)
}

/// Detected reference identifiers extracted from a paper's reference list (plan F.10).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectedReferences {
    /// Distinct DOIs in order of appearance.
    pub dois: Vec<String>,
    /// Distinct arXiv IDs in order of appearance.
    pub arxiv_ids: Vec<String>,
}

/// Scans `text` (such as a paper's bibliography section) and collects all distinct DOIs and arXiv IDs.
#[must_use]
pub fn detect_references_in_text(text: &str) -> DetectedReferences {
    let mut dois = Vec::new();
    let mut seen_dois = BTreeSet::new();
    let mut arxiv_ids = Vec::new();
    let mut seen_arxiv = BTreeSet::new();

    for line in text.lines() {
        let mut slice = line;
        while let Some(doi) = extract_doi(slice) {
            if seen_dois.insert(doi.to_ascii_lowercase()) {
                dois.push(doi.clone());
            }
            let Some(pos) = slice.find(&doi) else {
                break;
            };
            slice = &slice[pos + doi.len()..];
        }
        let mut arx_slice = line;
        while let Some(id) = extract_arxiv_id(arx_slice) {
            if seen_arxiv.insert(id.to_ascii_lowercase()) {
                arxiv_ids.push(id.clone());
            }
            let Some(pos) = arx_slice.find(&id) else {
                break;
            };
            arx_slice = &arx_slice[pos + id.len()..];
        }
    }

    DetectedReferences { dois, arxiv_ids }
}

fn parse_ris_line(line: &str) -> Option<(&str, &str)> {
    let (tag, val) = line.split_once("  -")?;
    let tag = tag.trim();
    if tag.len() == 2 {
        Some((tag, val.trim()))
    } else {
        None
    }
}

fn parse_bibtex_fields(input: &str) -> Result<Vec<(String, String)>, CitationParseError> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        while i < chars.len() && (chars[i].1.is_whitespace() || chars[i].1 == ',') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut name = String::new();
        while i < chars.len() && chars[i].1 != '=' && !chars[i].1.is_whitespace() {
            name.push(chars[i].1);
            i += 1;
        }
        while i < chars.len() && chars[i].1.is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i].1 != '=' {
            break;
        }
        i += 1; // skip '='
        while i < chars.len() && chars[i].1.is_whitespace() {
            i += 1;
        }
        let value = read_bibtex_value(&chars, &mut i)?;
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push((name.to_ascii_lowercase(), normalized));
    }

    Ok(out)
}

fn read_bibtex_value(chars: &[(usize, char)], i: &mut usize) -> Result<String, CitationParseError> {
    if *i >= chars.len() {
        return Ok(String::new());
    }
    let ch = chars[*i].1;
    if ch == '{' {
        *i += 1;
        let mut depth = 1_i32;
        let mut val = String::new();
        while *i < chars.len() {
            let c = chars[*i].1;
            *i += 1;
            if c == '{' {
                depth += 1;
                val.push(c);
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return Ok(val);
                }
                val.push(c);
            } else {
                val.push(c);
            }
        }
        Err(CitationParseError(
            "unclosed `{` in BibTeX field".to_owned(),
        ))
    } else if ch == '"' {
        *i += 1;
        let mut val = String::new();
        let mut depth = 0_i32;
        while *i < chars.len() {
            let c = chars[*i].1;
            *i += 1;
            if c == '{' {
                depth += 1;
                val.push(c);
            } else if c == '}' && depth > 0 {
                depth -= 1;
                val.push(c);
            } else if c == '"' && depth == 0 {
                return Ok(val);
            } else {
                val.push(c);
            }
        }
        Err(CitationParseError(
            "unclosed `\"` in BibTeX field".to_owned(),
        ))
    } else {
        let mut val = String::new();
        while *i < chars.len() && chars[*i].1 != ',' && !chars[*i].1.is_whitespace() {
            val.push(chars[*i].1);
            *i += 1;
        }
        Ok(val)
    }
}

fn split_bibtex_authors(raw: &str) -> Vec<String> {
    let mut authors = Vec::new();
    let mut current = String::new();
    let lower = raw.to_ascii_lowercase();
    let mut idx = 0;
    while idx < raw.len() {
        if lower[idx..].starts_with(" and ") {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                authors.push(trimmed.to_owned());
            }
            current.clear();
            idx += 5;
        } else if let Some(ch) = raw[idx..].chars().next() {
            current.push(ch);
            idx += ch.len_utf8();
        } else {
            break;
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        authors.push(trimmed.to_owned());
    }
    authors
}

fn escape_bibtex_value(s: &str) -> String {
    s.replace('&', "\\&").replace('%', "\\%")
}

fn slugify_ascii(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn first_significant_title_word(title: &str) -> String {
    const STOP: &[&str] = &["a", "an", "the", "on", "of", "in", "for", "to", "and"];
    for raw in title.split(|c: char| !c.is_ascii_alphanumeric()) {
        let low = raw.to_ascii_lowercase();
        if !low.is_empty() && !STOP.contains(&low.as_str()) {
            return low;
        }
    }
    "work".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bibtex_ris_and_csl_json_round_trip_cleanly() {
        let mut entries = vec![CitationEntry {
            key: String::new(),
            kind: EntryKind::InProceedings,
            title: "Attention Is All You Need".to_owned(),
            authors: vec![
                AuthorName::parse("Ashish Vaswani"),
                AuthorName::parse("Noam Shazeer"),
            ],
            year: Some(2017),
            booktitle: Some("Advances in Neural Information Processing Systems".to_owned()),
            pages: Some("5998-6008".to_owned()),
            doi: Some("10.5555/3295222.3295349".to_owned()),
            arxiv_id: Some("1706.03762".to_owned()),
            ..CitationEntry::default()
        }];
        assign_unique_keys(&mut entries, KeyFormat::AuthorYearTitle);
        assert_eq!(entries[0].key, "vaswani2017attention");

        let bib = export_bibtex(&entries);
        let parsed_bib = parse_bibtex(&bib).expect("parse bib");
        assert_eq!(parsed_bib.len(), 1);
        assert_eq!(parsed_bib[0].key, "vaswani2017attention");
        assert_eq!(parsed_bib[0].authors.len(), 2);
        assert_eq!(parsed_bib[0].year, Some(2017));

        let ris = export_ris(&entries);
        let parsed_ris = parse_ris(&ris);
        assert_eq!(parsed_ris.len(), 1);
        assert_eq!(parsed_ris[0].title, "Attention Is All You Need");
        assert_eq!(parsed_ris[0].pages.as_deref(), Some("5998-6008"));

        let csl = export_csl_json(&entries);
        let parsed_csl = parse_csl_json(&csl).expect("parse csl");
        assert_eq!(parsed_csl.len(), 1);
        assert_eq!(
            parsed_csl[0].doi.as_deref(),
            Some("10.5555/3295222.3295349")
        );

        let apa = format_citation(&entries[0], CitationStyle::Apa);
        assert!(apa.contains("Vaswani, A."));
        assert!(apa.contains("(2017)"));
    }

    #[test]
    fn detects_multiple_references_in_bibliography_text() {
        let refs = "\
[1] Vaswani et al. arXiv:1706.03762, 2017.
[2] Smith et al. https://doi.org/10.1145/3295222.3295349 and 10.1000/182.
";
        let found = detect_references_in_text(refs);
        assert_eq!(found.dois, ["10.1145/3295222.3295349", "10.1000/182"]);
        assert_eq!(found.arxiv_ids, ["1706.03762"]);
    }
}
