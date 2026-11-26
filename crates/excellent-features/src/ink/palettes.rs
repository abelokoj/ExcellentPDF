//! Saved color palettes (H.18, plan §27.7): four built-in curated palettes (pastel, neon, earth, and Okabe–Ito
//! color-blind-safe) plus user palettes that can be created, edited, persisted in settings, and shared as small JSON
//! files.

use std::fmt;

use serde_json::{Value, json};

use crate::annotate::colors::{hex, parse_hex};

/// Export file format version.
const FORMAT: u64 = 1;

/// Maximum number of swatches allowed in a single palette.
pub const MAX_SWATCHES: usize = 32;

/// Maximum number of user-created palettes stored in the library.
pub const MAX_USER_PALETTES: usize = 32;

/// Maximum length of a palette or swatch name in characters.
const MAX_NAME_LEN: usize = 64;

/// A single named color swatch (`0xRRGGBB`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Swatch {
    /// Human-readable label for accessibility and tooltips.
    pub name: String,
    /// Color as `0xRRGGBB`.
    pub color: u32,
}

impl Swatch {
    /// Creates a swatch, masking `color` to 24-bit RGB and trimming `name`.
    #[must_use]
    pub fn new(name: impl Into<String>, color: u32) -> Self {
        let color = color & 0xff_ffff;
        let clean = trim_name(&name.into());
        let name = if clean.is_empty() { hex(color) } else { clean };
        Self { name, color }
    }

    /// Normalized `0.0..=1.0` RGB channels.
    #[must_use]
    pub fn rgb(&self) -> [f32; 3] {
        [
            f32::from(((self.color >> 16) & 0xff) as u8) / 255.0,
            f32::from(((self.color >> 8) & 0xff) as u8) / 255.0,
            f32::from((self.color & 0xff) as u8) / 255.0,
        ]
    }

    /// Formats the swatch color as `#rrggbb`.
    #[must_use]
    pub fn hex_string(&self) -> String {
        hex(self.color)
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "color": hex(self.color),
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        if let Some(s) = value.as_str() {
            let color = parse_hex(s)?;
            return Some(Self::new(hex(color), color));
        }
        let color = value
            .get("color")
            .and_then(Value::as_str)
            .and_then(parse_hex)?;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Some(Self::new(name, color))
    }
}

/// Whether a palette ships built into ExcellentPDF or was added by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaletteOrigin {
    /// Curated built-in palette (read-only).
    BuiltIn,
    /// User-defined or imported palette (editable and removable).
    User,
}

/// A named collection of color swatches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPalette {
    /// Stable identifier (slug or UUID).
    pub id: String,
    /// Display name shown in the palette picker.
    pub name: String,
    /// Whether this palette is built-in or user-owned.
    pub origin: PaletteOrigin,
    /// Ordered color swatches.
    pub swatches: Vec<Swatch>,
}

/// Errors returned while validating, importing, or managing palettes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteError {
    /// Input text is not valid JSON.
    NotJson,
    /// JSON does not represent a palette or palette bundle.
    NotPalette,
    /// File format version is newer than this version of ExcellentPDF supports.
    NewerFormat(u64),
    /// Palette name is empty.
    EmptyName,
    /// Palette contains no valid color swatches.
    EmptySwatches,
    /// Attempted to modify or delete a built-in palette.
    ReadOnlyBuiltIn,
    /// No palette with the requested id was found.
    NotFound,
    /// Maximum user palette count reached.
    LimitReached,
    /// A palette with the same id already exists.
    DuplicateId,
}

impl fmt::Display for PaletteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => f.write_str("the file isn't valid JSON"),
            Self::NotPalette => f.write_str("the file holds no palette"),
            Self::NewerFormat(v) => write!(f, "palette format {v} is newer than this app reads"),
            Self::EmptyName => f.write_str("palette name cannot be empty"),
            Self::EmptySwatches => f.write_str("palette must contain at least one swatch"),
            Self::ReadOnlyBuiltIn => f.write_str("built-in palettes cannot be modified or deleted"),
            Self::NotFound => f.write_str("palette not found"),
            Self::LimitReached => f.write_str("maximum number of user palettes reached"),
            Self::DuplicateId => f.write_str("a palette with this id already exists"),
        }
    }
}

impl std::error::Error for PaletteError {}

impl ColorPalette {
    /// Creates a new validated user palette.
    ///
    /// # Errors
    ///
    /// Fails with [`PaletteError::EmptyName`] or [`PaletteError::EmptySwatches`] if invalid.
    pub fn new_user(
        id: impl Into<String>,
        name: impl Into<String>,
        swatches: Vec<Swatch>,
    ) -> Result<Self, PaletteError> {
        let name = trim_name(&name.into());
        if name.is_empty() {
            return Err(PaletteError::EmptyName);
        }
        let swatches: Vec<Swatch> = swatches.into_iter().take(MAX_SWATCHES).collect();
        if swatches.is_empty() {
            return Err(PaletteError::EmptySwatches);
        }
        let raw_id = trim_name(&id.into());
        let id = if raw_id.is_empty() {
            slugify(&name)
        } else {
            raw_id
        };
        Ok(Self {
            id,
            name,
            origin: PaletteOrigin::User,
            swatches,
        })
    }

    /// Exports this palette as a portable, human-readable JSON file string.
    #[must_use]
    pub fn export(&self) -> String {
        let file = json!({
            "format": FORMAT,
            "app": "ExcellentPDF",
            "kind": "palette",
            "id": self.id,
            "name": self.name,
            "swatches": self.swatches.iter().map(Swatch::to_json).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// Imports a single palette from JSON produced by [`ColorPalette::export`]. Always imports as a [`PaletteOrigin::User`]
    /// palette so imported palettes remain editable.
    ///
    /// # Errors
    ///
    /// Fails if the input is not valid JSON, has a newer format version, or lacks a name or valid swatches.
    pub fn import(text: &str) -> Result<Self, PaletteError> {
        let value: Value = serde_json::from_str(text).map_err(|_| PaletteError::NotJson)?;
        check_format_version(&value)?;
        Self::from_json_value(&value)
    }

    fn to_settings_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "swatches": self.swatches.iter().map(Swatch::to_json).collect::<Vec<_>>(),
        })
    }

    fn from_json_value(value: &Value) -> Result<Self, PaletteError> {
        let obj = value.as_object().ok_or(PaletteError::NotPalette)?;
        let list = obj
            .get("swatches")
            .and_then(Value::as_array)
            .ok_or(PaletteError::NotPalette)?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .map(trim_name)
            .unwrap_or_default();
        if name.is_empty() {
            return Err(PaletteError::EmptyName);
        }
        let swatches: Vec<Swatch> = list
            .iter()
            .filter_map(Swatch::from_json)
            .take(MAX_SWATCHES)
            .collect();
        if swatches.is_empty() {
            return Err(PaletteError::EmptySwatches);
        }
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(trim_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slugify(&name));
        Ok(Self {
            id,
            name,
            origin: PaletteOrigin::User,
            swatches,
        })
    }
}

/// Returns the four curated built-in palettes: Pastel, Neon, Earth, and Color-blind-safe (Okabe & Ito).
#[must_use]
pub fn built_in_palettes() -> Vec<ColorPalette> {
    vec![
        ColorPalette {
            id: "pastel".to_owned(),
            name: "Pastel".to_owned(),
            origin: PaletteOrigin::BuiltIn,
            swatches: vec![
                Swatch::new("Lavender", 0xc8b6ff),
                Swatch::new("Mint", 0xb8f2e6),
                Swatch::new("Peach", 0xffcdb2),
                Swatch::new("Sky", 0xa8dadc),
                Swatch::new("Butter", 0xfef3a7),
                Swatch::new("Rose", 0xffafcc),
                Swatch::new("Sage", 0xcdeac0),
                Swatch::new("Periwinkle", 0xbde0fe),
            ],
        },
        ColorPalette {
            id: "neon".to_owned(),
            name: "Neon".to_owned(),
            origin: PaletteOrigin::BuiltIn,
            swatches: vec![
                Swatch::new("Electric Cyan", 0x00f5d4),
                Swatch::new("Hot Magenta", 0xff007f),
                Swatch::new("Acid Lime", 0x39ff14),
                Swatch::new("Laser Yellow", 0xfff01f),
                Swatch::new("Neon Orange", 0xff5f1f),
                Swatch::new("Ultraviolet", 0x9b5de5),
                Swatch::new("Plasma Pink", 0xff4dff),
                Swatch::new("Hyper Blue", 0x00bbf9),
            ],
        },
        ColorPalette {
            id: "earth".to_owned(),
            name: "Earth".to_owned(),
            origin: PaletteOrigin::BuiltIn,
            swatches: vec![
                Swatch::new("Terracotta", 0xc85a32),
                Swatch::new("Ochre", 0xcc8b3c),
                Swatch::new("Olive", 0x6b705c),
                Swatch::new("Sienna", 0x8d4a2b),
                Swatch::new("Slate", 0x4a5568),
                Swatch::new("Sand", 0xd6bfa2),
                Swatch::new("Moss", 0x4f6d4a),
                Swatch::new("Umber", 0x5c4033),
            ],
        },
        ColorPalette {
            id: "color-blind-safe".to_owned(),
            name: "Color-blind-safe".to_owned(),
            origin: PaletteOrigin::BuiltIn,
            swatches: vec![
                Swatch::new("Orange", 0xe69f00),
                Swatch::new("Sky Blue", 0x56b4e9),
                Swatch::new("Bluish Green", 0x009e73),
                Swatch::new("Yellow", 0xf0e442),
                Swatch::new("Blue", 0x0072b2),
                Swatch::new("Vermillion", 0xd55e00),
                Swatch::new("Reddish Purple", 0xcc79a7),
                Swatch::new("Black", 0x000000),
            ],
        },
    ]
}

/// Manages all built-in and user palettes plus the active selection, with settings and bundle JSON serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteLibrary {
    built_in: Vec<ColorPalette>,
    user: Vec<ColorPalette>,
    active_id: String,
}

impl Default for PaletteLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl PaletteLibrary {
    /// Creates a library containing the four built-in palettes with `"color-blind-safe"` selected by default.
    #[must_use]
    pub fn new() -> Self {
        Self {
            built_in: built_in_palettes(),
            user: Vec::new(),
            active_id: "color-blind-safe".to_owned(),
        }
    }

    /// The four built-in palettes.
    #[must_use]
    pub fn built_in(&self) -> &[ColorPalette] {
        &self.built_in
    }

    /// All user-defined and imported palettes.
    #[must_use]
    pub fn user(&self) -> &[ColorPalette] {
        &self.user
    }

    /// Iterates over all palettes (built-in first, then user).
    pub fn all(&self) -> impl Iterator<Item = &ColorPalette> {
        self.built_in.iter().chain(&self.user)
    }

    /// Looks up a palette by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ColorPalette> {
        self.all().find(|p| p.id == id)
    }

    /// Currently active palette.
    #[must_use]
    pub fn active(&self) -> &ColorPalette {
        self.find(&self.active_id).unwrap_or(&self.built_in[0])
    }

    /// Selects palette `id` as active.
    ///
    /// # Errors
    ///
    /// Returns [`PaletteError::NotFound`] if no palette has that id.
    pub fn choose(&mut self, id: &str) -> Result<(), PaletteError> {
        if self.find(id).is_none() {
            return Err(PaletteError::NotFound);
        }
        self.active_id = id.to_owned();
        Ok(())
    }

    /// Adds a user palette to the library.
    ///
    /// # Errors
    ///
    /// Fails if the user palette limit is reached, the palette is invalid, or its id collides with an existing palette.
    pub fn add_user(&mut self, mut palette: ColorPalette) -> Result<(), PaletteError> {
        if self.user.len() >= MAX_USER_PALETTES {
            return Err(PaletteError::LimitReached);
        }
        palette.origin = PaletteOrigin::User;
        if palette.name.trim().is_empty() {
            return Err(PaletteError::EmptyName);
        }
        if palette.swatches.is_empty() {
            return Err(PaletteError::EmptySwatches);
        }
        if self.find(&palette.id).is_some() {
            return Err(PaletteError::DuplicateId);
        }
        palette.swatches.truncate(MAX_SWATCHES);
        self.user.push(palette);
        Ok(())
    }

    /// Replaces an existing user palette by id.
    ///
    /// # Errors
    ///
    /// Fails with [`PaletteError::ReadOnlyBuiltIn`] for built-ins, [`PaletteError::NotFound`] if absent, or validation
    /// errors if the replacement is empty.
    pub fn update_user(&mut self, id: &str, mut updated: ColorPalette) -> Result<(), PaletteError> {
        if self.built_in.iter().any(|p| p.id == id) {
            return Err(PaletteError::ReadOnlyBuiltIn);
        }
        let Some(slot) = self.user.iter_mut().find(|p| p.id == id) else {
            return Err(PaletteError::NotFound);
        };
        if updated.name.trim().is_empty() {
            return Err(PaletteError::EmptyName);
        }
        if updated.swatches.is_empty() {
            return Err(PaletteError::EmptySwatches);
        }
        updated.id = id.to_owned();
        updated.origin = PaletteOrigin::User;
        updated.swatches.truncate(MAX_SWATCHES);
        *slot = updated;
        Ok(())
    }

    /// Removes a user palette by id.
    ///
    /// # Errors
    ///
    /// Fails with [`PaletteError::ReadOnlyBuiltIn`] for built-ins or [`PaletteError::NotFound`] if absent.
    pub fn remove_user(&mut self, id: &str) -> Result<ColorPalette, PaletteError> {
        if self.built_in.iter().any(|p| p.id == id) {
            return Err(PaletteError::ReadOnlyBuiltIn);
        }
        let Some(pos) = self.user.iter().position(|p| p.id == id) else {
            return Err(PaletteError::NotFound);
        };
        let removed = self.user.remove(pos);
        if self.active_id == id {
            self.active_id = "color-blind-safe".to_owned();
        }
        Ok(removed)
    }

    /// Duplicates any palette (built-in or user) as a new editable user palette with a unique id.
    ///
    /// # Errors
    ///
    /// Fails if the source does not exist, `new_name` is empty, or the user limit is reached.
    pub fn duplicate_as_user(
        &mut self,
        source_id: &str,
        new_name: &str,
    ) -> Result<String, PaletteError> {
        let Some(src) = self.find(source_id).cloned() else {
            return Err(PaletteError::NotFound);
        };
        let clean = trim_name(new_name);
        if clean.is_empty() {
            return Err(PaletteError::EmptyName);
        }
        let base_id = slugify(&clean);
        let unique_id = self.make_unique_id(&base_id);
        let copy = ColorPalette::new_user(unique_id.clone(), clean, src.swatches)?;
        self.add_user(copy)?;
        Ok(unique_id)
    }

    /// Imports a single palette JSON file into the library, disambiguating its id if one already exists.
    ///
    /// # Errors
    ///
    /// Fails if the JSON is invalid or the user palette limit is reached.
    pub fn import_palette(&mut self, text: &str) -> Result<String, PaletteError> {
        let mut palette = ColorPalette::import(text)?;
        palette.id = self.make_unique_id(&palette.id);
        let id = palette.id.clone();
        self.add_user(palette)?;
        Ok(id)
    }

    /// Exports all user palettes as a portable JSON bundle file.
    #[must_use]
    pub fn export_user_bundle(&self) -> String {
        let file = json!({
            "format": FORMAT,
            "app": "ExcellentPDF",
            "kind": "palette-bundle",
            "palettes": self.user.iter().map(ColorPalette::to_settings_value).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// Imports a JSON file containing either a single palette or a `palette-bundle`, adding all valid palettes to the
    /// library and returning their assigned ids.
    ///
    /// # Errors
    ///
    /// Fails if the JSON is invalid, has a newer format version, or holds no valid palettes.
    pub fn import_bundle(&mut self, text: &str) -> Result<Vec<String>, PaletteError> {
        let value: Value = serde_json::from_str(text).map_err(|_| PaletteError::NotJson)?;
        check_format_version(&value)?;

        if let Some(list) = value.get("palettes").and_then(Value::as_array) {
            let mut imported_ids = Vec::new();
            for entry in list {
                if self.user.len() >= MAX_USER_PALETTES {
                    break;
                }
                if let Ok(mut palette) = ColorPalette::from_json_value(entry) {
                    palette.id = self.make_unique_id(&palette.id);
                    let id = palette.id.clone();
                    self.user.push(palette);
                    imported_ids.push(id);
                }
            }
            if imported_ids.is_empty() {
                return Err(PaletteError::EmptySwatches);
            }
            return Ok(imported_ids);
        }

        // Fall back to single-palette file.
        let mut palette = ColorPalette::from_json_value(&value)?;
        palette.id = self.make_unique_id(&palette.id);
        let id = palette.id.clone();
        self.add_user(palette)?;
        Ok(vec![id])
    }

    /// Serializes active selection and user palettes for `settings.toml`.
    #[must_use]
    pub fn to_settings_json(&self) -> Value {
        json!({
            "active": self.active_id,
            "user": self.user.iter().map(ColorPalette::to_settings_value).collect::<Vec<_>>(),
        })
    }

    /// Restores user palettes and active selection from `settings.toml` JSON.
    #[must_use]
    pub fn from_settings_json(value: &Value) -> Self {
        let mut lib = Self::new();
        if let Some(list) = value.get("user").and_then(Value::as_array) {
            for entry in list.iter().take(MAX_USER_PALETTES) {
                if let Ok(mut palette) = ColorPalette::from_json_value(entry) {
                    if lib.find(&palette.id).is_some() {
                        palette.id = lib.make_unique_id(&palette.id);
                    }
                    lib.user.push(palette);
                }
            }
        }
        if let Some(active) = value.get("active").and_then(Value::as_str)
            && lib.find(active).is_some()
        {
            lib.active_id = active.to_owned();
        }
        lib
    }

    fn make_unique_id(&self, base: &str) -> String {
        let base = if base.is_empty() { "palette" } else { base };
        if self.find(base).is_none() {
            return base.to_owned();
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}-{suffix}");
            if self.find(&candidate).is_none() {
                return candidate;
            }
            suffix += 1;
        }
    }
}

fn check_format_version(value: &Value) -> Result<(), PaletteError> {
    let version = value
        .get("format")
        .and_then(Value::as_u64)
        .unwrap_or(FORMAT);
    if version > FORMAT {
        return Err(PaletteError::NewerFormat(version));
    }
    Ok(())
}

fn trim_name(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_NAME_LEN).collect::<String>()
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_palettes_include_all_four_required_sets() {
        let lib = PaletteLibrary::new();
        let ids: Vec<&str> = lib.built_in().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["pastel", "neon", "earth", "color-blind-safe"]);
        for p in lib.built_in() {
            assert_eq!(p.origin, PaletteOrigin::BuiltIn);
            assert_eq!(p.swatches.len(), 8);
        }
        // Color-blind-safe must match Okabe & Ito's 8-color set (A.3 / U.4).
        let safe = lib.find("color-blind-safe").expect("safe palette");
        let colors: Vec<u32> = safe.swatches.iter().map(|s| s.color).collect();
        assert_eq!(
            colors,
            [
                0xe69f00, 0x56b4e9, 0x009e73, 0xf0e442, 0x0072b2, 0xd55e00, 0xcc79a7, 0x000000
            ]
        );
    }

    #[test]
    fn palettes_import_and_export() {
        let custom = ColorPalette::new_user(
            "lecture-notes",
            "Lecture Notes",
            vec![
                Swatch::new("Ink Navy", 0x1a3d8f),
                Swatch::new("Warm Coral", 0xe76f51),
                Swatch::new("Forest", 0x2a9d8f),
            ],
        )
        .expect("valid user palette");

        // Single-palette JSON export & import round-trip.
        let exported = custom.export();
        let reimported = ColorPalette::import(&exported).expect("imports");
        assert_eq!(reimported, custom);

        // Shorthand hex-only swatch list also imports cleanly.
        let shorthand = r##"{"format": 1, "name": "Quick", "swatches": ["#ff0000", "#00ff00"]}"##;
        let quick = ColorPalette::import(shorthand).expect("imports shorthand");
        assert_eq!(quick.name, "Quick");
        assert_eq!(quick.swatches.len(), 2);
        assert_eq!(quick.swatches[0].color, 0xff0000);

        // Bundle export & import round-trip in a library.
        let mut lib = PaletteLibrary::new();
        lib.add_user(custom.clone()).expect("adds");
        lib.add_user(quick).expect("adds quick");
        let bundle = lib.export_user_bundle();

        let mut other = PaletteLibrary::new();
        let ids = other.import_bundle(&bundle).expect("imports bundle");
        assert_eq!(ids, ["lecture-notes", "quick"]);
        assert_eq!(other.user(), lib.user());

        // Re-importing a duplicate disambiguates its id instead of overwriting.
        let dup_id = other.import_palette(&exported).expect("disambiguates");
        assert_eq!(dup_id, "lecture-notes-2");

        // Error handling on malformed files.
        assert_eq!(ColorPalette::import("nope"), Err(PaletteError::NotJson));
        assert_eq!(ColorPalette::import("{}"), Err(PaletteError::NotPalette));
        assert_eq!(
            ColorPalette::import(r#"{"format": 99, "name": "X", "swatches": []}"#),
            Err(PaletteError::NewerFormat(99))
        );
        assert_eq!(
            ColorPalette::import(r##"{"name": "", "swatches": ["#000000"]}"##),
            Err(PaletteError::EmptyName)
        );
        assert_eq!(
            ColorPalette::import(r#"{"name": "Empty", "swatches": ["not-a-hex"]}"#),
            Err(PaletteError::EmptySwatches)
        );
    }

    #[test]
    fn built_ins_are_read_only_and_settings_round_trip() {
        let mut lib = PaletteLibrary::new();
        assert_eq!(
            lib.remove_user("pastel"),
            Err(PaletteError::ReadOnlyBuiltIn)
        );
        let copy_id = lib
            .duplicate_as_user("pastel", "My Pastel")
            .expect("duplicates built-in");
        lib.choose(&copy_id).expect("activates copy");
        assert_eq!(lib.active().id, copy_id);

        // Settings TOML round-trip (as settings.toml keeps it).
        let toml_val = toml::Value::try_from(lib.to_settings_json()).expect("to TOML");
        let back_json: Value = serde_json::to_value(toml_val).expect("to JSON");
        let restored = PaletteLibrary::from_settings_json(&back_json);
        assert_eq!(restored, lib);

        lib.remove_user(&copy_id).expect("removes copy");
        assert_eq!(lib.active().id, "color-blind-safe");
    }
}
