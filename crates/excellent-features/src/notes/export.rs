//! Markdown & Obsidian vault exporter with `excellentpdf://open?file=...&page=...&annot=...`
//! deep links (plan F.13).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use excellent_store::notes::Note;

/// Parsed `excellentpdf://open?file=...&page=...&annot=...` deep link (plan F.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLink {
    /// Target document path.
    pub file: PathBuf,
    /// Optional 1-based page number.
    pub page: Option<usize>,
    /// Optional annotation `/NM` id.
    pub annot: Option<String>,
}

impl DeepLink {
    /// Formats as `excellentpdf://open?file=<encoded>&page=<n>&annot=<encoded>`.
    #[must_use]
    pub fn to_uri(&self) -> String {
        let file_str = self.file.to_string_lossy();
        let mut out = format!("excellentpdf://open?file={}", percent_encode(&file_str));
        if let Some(page) = self.page {
            let _ = write!(out, "&page={page}");
        }
        if let Some(annot) = &self.annot
            && !annot.is_empty()
        {
            let _ = write!(out, "&annot={}", percent_encode(annot));
        }
        out
    }

    /// Parses an `excellentpdf://open?...` deep link URI back into its components.
    #[must_use]
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix("excellentpdf://")?;
        let query = rest.strip_prefix("open?")?;
        let mut file: Option<PathBuf> = None;
        let mut page: Option<usize> = None;
        let mut annot: Option<String> = None;

        for pair in query.split('&') {
            let (key, val) = pair.split_once('=')?;
            match key {
                "file" => {
                    let decoded = percent_decode(val)?;
                    if !decoded.is_empty() {
                        file = Some(PathBuf::from(decoded));
                    }
                }
                "page" => {
                    page = val.parse::<usize>().ok();
                }
                "annot" => {
                    let decoded = percent_decode(val)?;
                    if !decoded.is_empty() {
                        annot = Some(decoded);
                    }
                }
                _ => {}
            }
        }
        Some(Self {
            file: file?,
            page,
            annot,
        })
    }
}

/// One highlight or annotation included in a Markdown/Obsidian document export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightExport {
    /// 1-based page number.
    pub page: usize,
    /// Annotation `/NM` id.
    pub annot_id: String,
    /// Highlighted quote text.
    pub quote: String,
    /// Optional user note/comment.
    pub note: String,
    /// Optional semantic category (`claim`, `evidence`, ...).
    pub category: Option<String>,
    /// Optional tags.
    pub tags: Vec<String>,
}

/// Summary of an Obsidian vault export written to disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObsidianVaultReport {
    /// Paths of all `.md` files written.
    pub files_written: Vec<PathBuf>,
    /// Number of notes exported.
    pub notes_exported: usize,
    /// Number of document highlight files exported.
    pub documents_exported: usize,
}

/// Formats a single [`Note`] as Obsidian-compatible Markdown with YAML frontmatter and an
/// `excellentpdf://open?...` source deep link when anchored to a file.
#[must_use]
pub fn export_note_markdown(note: &Note) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    let _ = writeln!(out, "title: \"{}\"", escape_yaml(&note.title));
    let _ = writeln!(out, "id: \"{}\"", escape_yaml(&note.id));
    if !note.tags.is_empty() {
        out.push_str("tags:\n");
        for tag in &note.tags {
            let _ = writeln!(out, "  - \"{}\"", escape_yaml(tag));
        }
    }
    if let Some(file) = &note.file {
        let deep = DeepLink {
            file: file.clone(),
            page: note.page.map(|p| p + 1),
            annot: note.annotation_id.clone(),
        };
        let _ = writeln!(out, "source: \"{}\"", escape_yaml(&deep.to_uri()));
    }
    let _ = writeln!(out, "created: {}", note.created_at);
    let _ = writeln!(out, "modified: {}", note.modified_at);
    out.push_str("---\n\n");
    let _ = writeln!(out, "# {}\n", note.title);

    if let Some(file) = &note.file {
        let deep = DeepLink {
            file: file.clone(),
            page: note.page.map(|p| p + 1),
            annot: note.annotation_id.clone(),
        };
        let label = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Document");
        let page_suffix = note
            .page
            .map(|p| format!(" (p. {})", p + 1))
            .unwrap_or_default();
        let _ = writeln!(out, "> Source: [{label}{page_suffix}]({})\n", deep.to_uri());
    }

    out.push_str(&note.body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Formats a document's highlights/notes as a standalone Markdown note with `excellentpdf://open?...` deep links.
#[must_use]
pub fn export_document_highlights_markdown(
    title: &str,
    file: &Path,
    highlights: &[HighlightExport],
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    let _ = writeln!(out, "title: \"{}\"", escape_yaml(title));
    let doc_link = DeepLink {
        file: file.to_path_buf(),
        page: None,
        annot: None,
    };
    let _ = writeln!(out, "source: \"{}\"", escape_yaml(&doc_link.to_uri()));
    out.push_str("---\n\n");
    let _ = writeln!(out, "# {title} — Highlights\n");
    let _ = writeln!(out, "[Open document]({})\n", doc_link.to_uri());

    for h in highlights {
        let link = DeepLink {
            file: file.to_path_buf(),
            page: Some(h.page),
            annot: (!h.annot_id.is_empty()).then(|| h.annot_id.clone()),
        };
        let mut badge = format!("[Page {}]({})", h.page, link.to_uri());
        if let Some(cat) = &h.category {
            let _ = write!(badge, " · `{cat}`");
        }
        let _ = writeln!(out, "- {badge}");
        if !h.quote.trim().is_empty() {
            for line in h.quote.lines() {
                let _ = writeln!(out, "  > {line}");
            }
        }
        if !h.note.trim().is_empty() {
            let _ = writeln!(out, "  - Note: {}", h.note.trim());
        }
        if !h.tags.is_empty() {
            let tags_fmt = h
                .tags
                .iter()
                .map(|t| format!("#{}", t.replace(' ', "_")))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(out, "  - Tags: {tags_fmt}");
        }
    }
    out
}

/// Exports `notes` and `document_highlights` into an Obsidian vault folder `vault_dir`, writing
/// `Notes/<Title>.md`, `Highlights/<Doc>.md`, and `Index.md` (plan F.13).
///
/// # Errors
///
/// Returns [`io::Error`] if directories or files cannot be created.
pub fn export_obsidian_vault(
    vault_dir: &Path,
    notes: &[Note],
    document_highlights: &[(String, PathBuf, Vec<HighlightExport>)],
) -> io::Result<ObsidianVaultReport> {
    let notes_dir = vault_dir.join("Notes");
    let highlights_dir = vault_dir.join("Highlights");
    fs::create_dir_all(&notes_dir)?;
    fs::create_dir_all(&highlights_dir)?;

    let mut report = ObsidianVaultReport::default();
    let mut index = String::from("# ExcellentPDF Vault Index\n\n## Notes\n\n");

    for note in notes {
        let filename = format!("{}.md", sanitize_filename(&note.title));
        let dest = notes_dir.join(&filename);
        let md = export_note_markdown(note);
        write_atomic(&dest, md.as_bytes())?;
        let _ = writeln!(index, "- [[{}]]", note.title);
        report.files_written.push(dest);
        report.notes_exported += 1;
    }

    index.push_str("\n## Document Highlights\n\n");
    for (title, path, highlights) in document_highlights {
        let stem = format!("{title} - Highlights");
        let filename = format!("{}.md", sanitize_filename(&stem));
        let dest = highlights_dir.join(&filename);
        let md = export_document_highlights_markdown(title, path, highlights);
        write_atomic(&dest, md.as_bytes())?;
        let _ = writeln!(index, "- [[{stem}]]");
        report.files_written.push(dest);
        report.documents_exported += 1;
    }

    let index_path = vault_dir.join("Index.md");
    write_atomic(&index_path, index.as_bytes())?;
    report.files_written.push(index_path);
    Ok(report)
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = dest.with_extension("md.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Untitled".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/' | b':') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).copied()?;
            let lo = bytes.get(i + 2).copied()?;
            let hex = [hi, lo];
            let s = std::str::from_utf8(&hex).ok()?;
            let val = u8::from_str_radix(s, 16).ok()?;
            out.push(val);
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_links_round_trip_with_special_characters() {
        let link = DeepLink {
            file: PathBuf::from("C:/My Papers/Attention (2017).pdf"),
            page: Some(4),
            annot: Some("annot-123".to_owned()),
        };
        let uri = link.to_uri();
        assert!(uri.starts_with("excellentpdf://open?file="));
        assert!(uri.contains("&page=4&annot=annot-123"));
        assert_eq!(DeepLink::parse(&uri), Some(link));
    }

    #[test]
    fn exports_obsidian_vault_with_wiki_links_and_deep_links() {
        let dir = std::env::temp_dir().join(format!("excellent-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let note = Note {
            id: "n1".to_owned(),
            title: "Attention Notes".to_owned(),
            body: "Links to [[Transformer Architecture]].".to_owned(),
            file: Some(PathBuf::from("C:/papers/attention.pdf")),
            page: Some(2),
            annotation_id: Some("h-7".to_owned()),
            tags: vec!["ml".to_owned()],
            created_at: 100,
            modified_at: 200,
        };
        let highlights = vec![HighlightExport {
            page: 3,
            annot_id: "h-7".to_owned(),
            quote: "Scaled Dot-Product Attention".to_owned(),
            note: "Core formula".to_owned(),
            category: Some("method".to_owned()),
            tags: vec!["attention".to_owned()],
        }];

        let report = export_obsidian_vault(
            &dir,
            &[note],
            &[(
                "Attention".to_owned(),
                PathBuf::from("C:/papers/attention.pdf"),
                highlights,
            )],
        )
        .expect("export vault");
        assert_eq!(report.notes_exported, 1);
        assert_eq!(report.documents_exported, 1);

        let note_md = fs::read_to_string(dir.join("Notes/Attention Notes.md")).expect("read note");
        assert!(note_md.contains("[[Transformer Architecture]]"));
        assert!(note_md.contains("excellentpdf://open?file="));
        assert!(note_md.contains("page=3&annot=h-7"));

        let hl_md = fs::read_to_string(dir.join("Highlights/Attention - Highlights.md"))
            .expect("read highlights");
        assert!(hl_md.contains("Scaled Dot-Product Attention"));
        assert!(hl_md.contains("excellentpdf://open?file="));

        let _ = fs::remove_dir_all(&dir);
    }
}
