//! Template packs manifest, loader, and live catalog (plan N.12).
//!
//! Supports packs of procedural [`PaperSpec`] parameter presets and PDF background templates,
//! installed into `packs/templates/` (or loaded from `.zip`/`.expack` archives), updating the
//! available template list immediately without restarting the application.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{Cursor, Read as _};
use std::path::PathBuf;

use excellent_core::notebook::PaperTemplateId;
use excellent_mupdf::Document;
use serde_json::{Value, json};
use zip::ZipArchive;

use crate::notebook::paper::PaperSpec;

/// Manifest filename expected inside each installed template pack folder or archive.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Errors produced when loading, validating, installing, or removing template packs.
#[derive(Debug)]
#[non_exhaustive]
pub enum TemplatePackError {
    /// Filesystem I/O failure.
    Io(std::io::Error),
    /// ZIP archive failure.
    Zip(zip::result::ZipError),
    /// JSON parsing or serialization failure.
    Json(serde_json::Error),
    /// Manifest or template asset validation error.
    InvalidPack(String),
    /// Referenced pack ID was not installed.
    NotFound(String),
}

impl fmt::Display for TemplatePackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "template pack I/O error: {err}"),
            Self::Zip(err) => write!(f, "template pack zip error: {err}"),
            Self::Json(err) => write!(f, "template pack json error: {err}"),
            Self::InvalidPack(msg) => write!(f, "invalid template pack: {msg}"),
            Self::NotFound(id) => write!(f, "template pack not found: {id}"),
        }
    }
}

impl std::error::Error for TemplatePackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Zip(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::InvalidPack(_) | Self::NotFound(_) => None,
        }
    }
}

impl From<std::io::Error> for TemplatePackError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<zip::result::ZipError> for TemplatePackError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Zip(err)
    }
}

impl From<serde_json::Error> for TemplatePackError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// Convenience result alias for template pack operations.
pub type Result<T> = std::result::Result<T, TemplatePackError>;

/// Definition of one template provided inside a [`TemplatePackManifest`].
#[derive(Debug, Clone, PartialEq)]
pub enum PackTemplateKind {
    /// Procedural paper parameters (colors, spacing, margins).
    Procedural {
        /// Base procedural geometry kind.
        base: PaperTemplateId,
        /// Spacing in points.
        spacing_pt: f32,
        /// Left margin in points.
        margin_left_pt: f32,
        /// Right margin in points.
        margin_right_pt: f32,
        /// Top margin in points.
        margin_top_pt: f32,
        /// Bottom margin in points.
        margin_bottom_pt: f32,
        /// Primary line color (`0xRRGGBB`).
        line_color: u32,
        /// Accent color (`0xRRGGBB`).
        accent_color: u32,
        /// Paper background color (`0xRRGGBB`).
        paper_color: u32,
    },
    /// Static vector PDF background file bundled inside the pack.
    PdfBackground {
        /// Relative path to the PDF inside the pack (e.g., `"templates/cornell_dark.pdf"`).
        pdf_path: String,
    },
}

/// One template entry listed in a template pack manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct PackTemplateEntry {
    /// Stable identifier within the pack (e.g., `"kanban-weekly"`).
    pub id: String,
    /// User-visible template name.
    pub name: String,
    /// Category label (e.g., `"Academic"`, `"Music"`, `"Planning"`).
    pub category: String,
    /// Template geometry or PDF background reference.
    pub kind: PackTemplateKind,
}

impl PackTemplateEntry {
    /// Converts a [`PackTemplateKind::Procedural`] entry into a [`PaperSpec`].
    #[must_use]
    pub fn to_paper_spec(&self) -> Option<PaperSpec> {
        let PackTemplateKind::Procedural {
            base,
            spacing_pt,
            margin_left_pt,
            margin_right_pt,
            margin_top_pt,
            margin_bottom_pt,
            line_color,
            accent_color,
            paper_color,
        } = self.kind
        else {
            return None;
        };
        Some(PaperSpec {
            template: base,
            spacing_pt: spacing_pt.max(2.0),
            margin_left_pt: margin_left_pt.max(0.0),
            margin_right_pt: margin_right_pt.max(0.0),
            margin_top_pt: margin_top_pt.max(0.0),
            margin_bottom_pt: margin_bottom_pt.max(0.0),
            line_color: rgb_u32_to_f32(line_color),
            accent_color: rgb_u32_to_f32(accent_color),
            paper_color: rgb_u32_to_f32(paper_color),
            line_width_pt: 0.5,
        })
    }
}

fn rgb_u32_to_f32(rgb: u32) -> [f32; 3] {
    [
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    ]
}

/// Metadata manifest for an installable template pack (plan N.12).
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatePackManifest {
    /// Unique pack identifier (e.g., `"academic-study-pack"`).
    pub id: String,
    /// Human-readable pack title.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Author or publisher name.
    pub author: String,
    /// License identifier (e.g., `"MIT"`, `"CC0-1.0"`).
    pub license: String,
    /// Templates bundled in this pack.
    pub templates: Vec<PackTemplateEntry>,
}

impl TemplatePackManifest {
    /// Parses a manifest from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError::Json`] or [`TemplatePackError::InvalidPack`] if invalid.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        let val: Value = serde_json::from_slice(bytes)?;
        let id = req_str(&val, "id")?;
        validate_pack_id(&id)?;
        let name = req_str(&val, "name")?;
        let version = req_str(&val, "version")?;
        let author = val
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let license = val
            .get("license")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let raw_list = val
            .get("templates")
            .and_then(Value::as_array)
            .ok_or_else(|| TemplatePackError::InvalidPack("missing templates array".into()))?;
        let mut templates = Vec::with_capacity(raw_list.len());
        for item in raw_list {
            templates.push(parse_template_entry(item)?);
        }
        Ok(Self {
            id,
            name,
            version,
            author,
            license,
            templates,
        })
    }

    /// Serializes this manifest to pretty JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError::Json`] if serialization fails.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        let templates: Vec<Value> = self.templates.iter().map(template_entry_to_json).collect();
        let val = json!({
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "author": self.author,
            "license": self.license,
            "templates": templates,
        });
        Ok(serde_json::to_vec_pretty(&val)?)
    }
}

fn req_str(val: &Value, key: &str) -> Result<String> {
    val.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| TemplatePackError::InvalidPack(format!("missing string field '{key}'")))
}

fn validate_pack_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(TemplatePackError::InvalidPack(format!(
            "invalid pack id '{id}'"
        )));
    }
    Ok(())
}

fn parse_template_entry(val: &Value) -> Result<PackTemplateEntry> {
    let id = req_str(val, "id")?;
    let name = req_str(val, "name")?;
    let category = val
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("General")
        .to_owned();
    let kind_str = req_str(val, "kind")?;
    let kind = match kind_str.as_str() {
        "procedural" => {
            let base_key = val.get("base").and_then(Value::as_str).unwrap_or("lined");
            let base = PaperTemplateId::parse(base_key).ok_or_else(|| {
                TemplatePackError::InvalidPack(format!("unknown base template '{base_key}'"))
            })?;
            PackTemplateKind::Procedural {
                base,
                spacing_pt: val
                    .get("spacing_pt")
                    .and_then(Value::as_f64)
                    .unwrap_or(24.0) as f32,
                margin_left_pt: val
                    .get("margin_left_pt")
                    .and_then(Value::as_f64)
                    .unwrap_or(36.0) as f32,
                margin_right_pt: val
                    .get("margin_right_pt")
                    .and_then(Value::as_f64)
                    .unwrap_or(36.0) as f32,
                margin_top_pt: val
                    .get("margin_top_pt")
                    .and_then(Value::as_f64)
                    .unwrap_or(48.0) as f32,
                margin_bottom_pt: val
                    .get("margin_bottom_pt")
                    .and_then(Value::as_f64)
                    .unwrap_or(48.0) as f32,
                line_color: val
                    .get("line_color")
                    .and_then(Value::as_u64)
                    .unwrap_or(0xB8C8DC) as u32,
                accent_color: val
                    .get("accent_color")
                    .and_then(Value::as_u64)
                    .unwrap_or(0xE08A8A) as u32,
                paper_color: val
                    .get("paper_color")
                    .and_then(Value::as_u64)
                    .unwrap_or(0xFFFFFF) as u32,
            }
        }
        "pdf_background" => {
            let pdf_path = req_str(val, "pdf_path")?;
            if pdf_path.contains("..") {
                return Err(TemplatePackError::InvalidPack(format!(
                    "unsafe pdf_path '{pdf_path}'"
                )));
            }
            PackTemplateKind::PdfBackground { pdf_path }
        }
        other => {
            return Err(TemplatePackError::InvalidPack(format!(
                "unknown template kind '{other}'"
            )));
        }
    };
    Ok(PackTemplateEntry {
        id,
        name,
        category,
        kind,
    })
}

fn template_entry_to_json(entry: &PackTemplateEntry) -> Value {
    match &entry.kind {
        PackTemplateKind::Procedural {
            base,
            spacing_pt,
            margin_left_pt,
            margin_right_pt,
            margin_top_pt,
            margin_bottom_pt,
            line_color,
            accent_color,
            paper_color,
        } => json!({
            "id": entry.id,
            "name": entry.name,
            "category": entry.category,
            "kind": "procedural",
            "base": base.as_str(),
            "spacing_pt": spacing_pt,
            "margin_left_pt": margin_left_pt,
            "margin_right_pt": margin_right_pt,
            "margin_top_pt": margin_top_pt,
            "margin_bottom_pt": margin_bottom_pt,
            "line_color": line_color,
            "accent_color": accent_color,
            "paper_color": paper_color,
        }),
        PackTemplateKind::PdfBackground { pdf_path } => json!({
            "id": entry.id,
            "name": entry.name,
            "category": entry.category,
            "kind": "pdf_background",
            "pdf_path": pdf_path,
        }),
    }
}

/// Qualified template entry surfaced by [`TemplatePackCatalog::list_templates`].
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogTemplate {
    /// Owning pack ID (`"builtin"` for built-in templates).
    pub pack_id: String,
    /// Template metadata.
    pub entry: PackTemplateEntry,
}

/// Live template-pack manager under `packs/templates/` supporting immediate install, reload,
/// PDF background loading, and removal without an app restart (plan N.12).
#[derive(Debug, Clone)]
pub struct TemplatePackCatalog {
    root_dir: PathBuf,
    installed: BTreeMap<String, TemplatePackManifest>,
}

impl TemplatePackCatalog {
    /// Creates a catalog rooted at `root_dir` and loads all already-installed packs.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError`] if an installed pack directory is corrupt.
    pub fn open(root_dir: impl Into<PathBuf>) -> Result<Self> {
        let mut catalog = Self {
            root_dir: root_dir.into(),
            installed: BTreeMap::new(),
        };
        catalog.reload()?;
        Ok(catalog)
    }

    /// Scans `root_dir` and refreshes the in-memory pack list.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError`] on I/O or manifest parse failures.
    pub fn reload(&mut self) -> Result<()> {
        self.installed.clear();
        if !self.root_dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.root_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join(MANIFEST_FILE);
            if !manifest_path.exists() {
                continue;
            }
            let bytes = fs::read(&manifest_path)?;
            let manifest = TemplatePackManifest::from_json_bytes(&bytes)?;
            self.installed.insert(manifest.id.clone(), manifest);
        }
        Ok(())
    }

    /// Installs a template pack from ZIP/EXPACK bytes, validating its manifest and any PDF
    /// background pages via MuPDF, and immediately updates the live template catalog.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError`] on invalid ZIP, missing manifest, or corrupt PDF backgrounds.
    pub fn install_zip_bytes(&mut self, zip_bytes: &[u8]) -> Result<TemplatePackManifest> {
        let mut archive = ZipArchive::new(Cursor::new(zip_bytes))?;
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i)?;
            let name = f.name()?.replace('\\', "/");
            if name.ends_with('/') || name.contains("..") {
                continue;
            }
            let mut buf = Vec::with_capacity(usize::try_from(f.size()).unwrap_or(0));
            f.read_to_end(&mut buf)?;
            files.insert(name, buf);
        }

        let manifest_bytes = files
            .get(MANIFEST_FILE)
            .ok_or_else(|| TemplatePackError::InvalidPack("zip missing manifest.json".into()))?;
        let manifest = TemplatePackManifest::from_json_bytes(manifest_bytes)?;

        // Validate all referenced PDF backgrounds exist and open cleanly in MuPDF.
        for tmpl in &manifest.templates {
            if let PackTemplateKind::PdfBackground { pdf_path } = &tmpl.kind {
                let pdf_bytes = files.get(pdf_path).ok_or_else(|| {
                    TemplatePackError::InvalidPack(format!(
                        "missing referenced PDF background '{pdf_path}'"
                    ))
                })?;
                Document::from_bytes(pdf_bytes, "application/pdf").map_err(|e| {
                    TemplatePackError::InvalidPack(format!(
                        "invalid PDF background {pdf_path}: {e}"
                    ))
                })?;
            }
        }

        let pack_dir = self.root_dir.join(&manifest.id);
        let stage_dir = self.root_dir.join(format!(".{}.staging", manifest.id));
        if stage_dir.exists() {
            fs::remove_dir_all(&stage_dir)?;
        }
        fs::create_dir_all(&stage_dir)?;
        for (rel_path, data) in &files {
            let dest = stage_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(dest, data)?;
        }
        if pack_dir.exists() {
            fs::remove_dir_all(&pack_dir)?;
        }
        fs::rename(&stage_dir, &pack_dir)?;

        self.installed.insert(manifest.id.clone(), manifest.clone());
        Ok(manifest)
    }

    /// Removes an installed pack by `pack_id` and immediately updates the live catalog.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError::NotFound`] if the pack is not installed.
    pub fn remove(&mut self, pack_id: &str) -> Result<()> {
        validate_pack_id(pack_id)?;
        if self.installed.remove(pack_id).is_none() {
            return Err(TemplatePackError::NotFound(pack_id.to_owned()));
        }
        let pack_dir = self.root_dir.join(pack_id);
        if pack_dir.exists() {
            fs::remove_dir_all(pack_dir)?;
        }
        Ok(())
    }

    /// Reads the raw PDF background bytes for a [`PackTemplateKind::PdfBackground`] template.
    ///
    /// # Errors
    ///
    /// Returns [`TemplatePackError::NotFound`] or [`TemplatePackError::Io`].
    pub fn load_pdf_background(&self, pack_id: &str, pdf_path: &str) -> Result<Vec<u8>> {
        if !self.installed.contains_key(pack_id) {
            return Err(TemplatePackError::NotFound(pack_id.to_owned()));
        }
        if pdf_path.contains("..") {
            return Err(TemplatePackError::InvalidPack(format!(
                "unsafe pdf_path '{pdf_path}'"
            )));
        }
        let full = self.root_dir.join(pack_id).join(pdf_path);
        Ok(fs::read(full)?)
    }

    /// Returns all currently available templates (built-in templates followed by all installed
    /// template packs). Updates immediately on install/remove without restarting.
    #[must_use]
    pub fn list_templates(&self) -> Vec<CatalogTemplate> {
        let mut list: Vec<CatalogTemplate> = PaperTemplateId::ALL
            .iter()
            .map(|&id| {
                let spec = PaperSpec::for_template(id);
                CatalogTemplate {
                    pack_id: "builtin".to_owned(),
                    entry: PackTemplateEntry {
                        id: id.as_str().to_owned(),
                        name: id.as_str().to_owned(),
                        category: "Built-in".to_owned(),
                        kind: PackTemplateKind::Procedural {
                            base: id,
                            spacing_pt: spec.spacing_pt,
                            margin_left_pt: spec.margin_left_pt,
                            margin_right_pt: spec.margin_right_pt,
                            margin_top_pt: spec.margin_top_pt,
                            margin_bottom_pt: spec.margin_bottom_pt,
                            line_color: f32_to_rgb_u32(spec.line_color),
                            accent_color: f32_to_rgb_u32(spec.accent_color),
                            paper_color: f32_to_rgb_u32(spec.paper_color),
                        },
                    },
                }
            })
            .collect();

        for manifest in self.installed.values() {
            for tmpl in &manifest.templates {
                list.push(CatalogTemplate {
                    pack_id: manifest.id.clone(),
                    entry: tmpl.clone(),
                });
            }
        }
        list
    }

    /// Installed pack manifests keyed by pack ID.
    #[must_use]
    pub fn installed_packs(&self) -> &BTreeMap<String, TemplatePackManifest> {
        &self.installed
    }
}

fn f32_to_rgb_u32(c: [f32; 3]) -> u32 {
    let r = (c[0].clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (c[1].clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (c[2].clamp(0.0, 1.0) * 255.0).round() as u32;
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use excellent_core::notebook::pdf_ops::build_blank_pdf;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    #[test]
    fn install_and_remove_pack_updates_template_list_without_restart() {
        let dir = std::env::temp_dir().join(format!("ex-template-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let mut catalog = TemplatePackCatalog::open(&dir).expect("open catalog");
        let base_count = catalog.list_templates().len();
        assert_eq!(base_count, PaperTemplateId::ALL.len());

        let bg_pdf = build_blank_pdf(&[(595.0, 842.0)]).expect("blank pdf");
        let manifest = TemplatePackManifest {
            id: "study-grid-pack".into(),
            name: "Study Grid Pack".into(),
            version: "1.0.0".into(),
            author: "ExcellentPDF".into(),
            license: "CC0-1.0".into(),
            templates: vec![
                PackTemplateEntry {
                    id: "seyes-french".into(),
                    name: "Seyès French Ruled".into(),
                    category: "Academic".into(),
                    kind: PackTemplateKind::Procedural {
                        base: PaperTemplateId::Grid,
                        spacing_pt: 22.68,
                        margin_left_pt: 72.0,
                        margin_right_pt: 36.0,
                        margin_top_pt: 48.0,
                        margin_bottom_pt: 48.0,
                        line_color: 0x99B4D1,
                        accent_color: 0xD16B6B,
                        paper_color: 0xFFFDF7,
                    },
                },
                PackTemplateEntry {
                    id: "bg-planner".into(),
                    name: "Minimalist PDF Background".into(),
                    category: "Planning".into(),
                    kind: PackTemplateKind::PdfBackground {
                        pdf_path: "bg.pdf".into(),
                    },
                },
            ],
        };

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file(MANIFEST_FILE, opts).expect("manifest");
            zip.write_all(&manifest.to_json_bytes().expect("json"))
                .expect("write json");
            zip.start_file("bg.pdf", opts).expect("bg");
            zip.write_all(&bg_pdf).expect("write bg");
            zip.finish().expect("finish");
        }

        catalog
            .install_zip_bytes(&cursor.into_inner())
            .expect("install pack");
        assert_eq!(catalog.list_templates().len(), base_count + 2);
        assert_eq!(
            catalog
                .load_pdf_background("study-grid-pack", "bg.pdf")
                .expect("read bg"),
            bg_pdf
        );

        catalog.remove("study-grid-pack").expect("remove pack");
        assert_eq!(catalog.list_templates().len(), base_count);

        let _ = fs::remove_dir_all(&dir);
    }
}
