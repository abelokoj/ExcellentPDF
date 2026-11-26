//! OS-portable settings sanitizer, pen/palette/stamp portability, and per-OS overrides (plan Y.2).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::SyncError;

/// Target operating system for per-OS overrides (plan Y.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TargetOs {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    MacOs,
    /// Linux / BSD desktop.
    Linux,
}

impl TargetOs {
    /// Canonical lowercase key used in serialized override tables.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
        }
    }

    /// Parses a canonical OS key.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "windows" | "win32" => Some(Self::Windows),
            "macos" | "darwin" => Some(Self::MacOs),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }
}

/// Portable pen preset shared across Windows, macOS, and Linux (plan Y.2).
#[derive(Debug, Clone, PartialEq)]
pub struct PortablePen {
    /// Stable pen identifier.
    pub id: String,
    /// Pen display name.
    pub name: String,
    /// Hex color (e.g. `"#2563eb"`).
    pub color: String,
    /// Stroke width in points.
    pub width_pt: f32,
    /// Modified Lamport / wall timestamp for cross-OS merging.
    pub updated_at: u64,
}

/// Cross-OS portable settings bundle with separate per-OS override table (plan Y.2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortableSettings {
    /// General portable preferences (with OS-specific paths/handles stripped).
    pub preferences: Map<String, Value>,
    /// Keymap bindings (`command_id -> portable_shortcut`, e.g. `"Mod+F"`).
    pub keymap: BTreeMap<String, String>,
    /// Portable pen presets.
    pub pens: BTreeMap<String, PortablePen>,
    /// Named color palettes (`palette_name -> hex colors`).
    pub palettes: BTreeMap<String, Vec<String>>,
    /// Stamp preset identifiers/titles.
    pub stamps: BTreeMap<String, Value>,
    /// Workspace/page templates.
    pub templates: BTreeMap<String, Value>,
    /// Per-OS overrides that stay isolated from portable defaults.
    pub os_overrides: BTreeMap<TargetOs, Map<String, Value>>,
}

/// Local-only setting keys that must never leak into portable sync.
const LOCAL_ONLY_KEYS: [&str; 6] = [
    "window_hwnd",
    "window_placement",
    "local_cache_dir",
    "mupdf_temp_dir",
    "last_printer_handle",
    "gpu_adapter_luid",
];

/// Sanitizes raw local settings JSON into a [`PortableSettings`] profile, moving OS-specific
/// overrides (display scaling, native file dialogs, platform paths) into `os_overrides[source_os]`.
#[must_use]
pub fn sanitize_local_settings(raw: &Value, source_os: TargetOs) -> PortableSettings {
    let mut out = PortableSettings::default();
    let Some(obj) = raw.as_object() else {
        return out;
    };

    let mut local_os_map = Map::new();
    for (key, val) in obj {
        if LOCAL_ONLY_KEYS.contains(&key.as_str()) {
            local_os_map.insert(key.clone(), val.clone());
            continue;
        }
        match key.as_str() {
            "keymap" => {
                if let Some(km) = val.as_object() {
                    for (cmd, binding) in km {
                        if let Some(s) = binding.as_str() {
                            out.keymap.insert(cmd.clone(), normalize_shortcut(s));
                        }
                    }
                }
            }
            "pens" => {
                if let Some(arr) = val.as_array() {
                    for item in arr {
                        if let Some(pen) = parse_pen(item) {
                            out.pens.insert(pen.id.clone(), pen);
                        }
                    }
                }
            }
            "palettes" => {
                if let Some(pals) = val.as_object() {
                    for (name, cols) in pals {
                        if let Some(arr) = cols.as_array() {
                            let hexes: Vec<String> = arr
                                .iter()
                                .filter_map(|c| c.as_str().map(str::to_owned))
                                .collect();
                            out.palettes.insert(name.clone(), hexes);
                        }
                    }
                }
            }
            "stamps" => {
                if let Some(stamps) = val.as_object() {
                    for (k, v) in stamps {
                        out.stamps.insert(k.clone(), sanitize_value(v));
                    }
                }
            }
            "templates" => {
                if let Some(tmpls) = val.as_object() {
                    for (k, v) in tmpls {
                        out.templates.insert(k.clone(), sanitize_value(v));
                    }
                }
            }
            _ => {
                if is_os_specific_key(key) {
                    local_os_map.insert(key.clone(), val.clone());
                } else {
                    out.preferences.insert(key.clone(), sanitize_value(val));
                }
            }
        }
    }
    if !local_os_map.is_empty() {
        out.os_overrides.insert(source_os, local_os_map);
    }
    out
}

/// Materializes [`PortableSettings`] into a JSON settings document for `target_os`,
/// expanding portable shortcut tokens (`Mod+` -> `Cmd+` on macOS, `Ctrl+` elsewhere) and merging
/// any saved `os_overrides` for `target_os`.
#[must_use]
pub fn materialize_for_os(settings: &PortableSettings, target_os: TargetOs) -> Value {
    let mut root = settings.preferences.clone();
    let mod_prefix = match target_os {
        TargetOs::MacOs => "Cmd+",
        TargetOs::Windows | TargetOs::Linux => "Ctrl+",
    };
    let keymap_obj: Map<String, Value> = settings
        .keymap
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.replace("Mod+", mod_prefix))))
        .collect();
    root.insert("keymap".to_owned(), Value::Object(keymap_obj));

    let pens_arr: Vec<Value> = settings
        .pens
        .values()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "color": p.color,
                "width_pt": p.width_pt,
                "updated_at": p.updated_at,
            })
        })
        .collect();
    root.insert("pens".to_owned(), Value::Array(pens_arr));

    let palettes_obj: Map<String, Value> = settings
        .palettes
        .iter()
        .map(|(k, cols)| {
            (
                k.clone(),
                Value::Array(cols.iter().cloned().map(Value::String).collect()),
            )
        })
        .collect();
    root.insert("palettes".to_owned(), Value::Object(palettes_obj));
    root.insert(
        "stamps".to_owned(),
        Value::Object(
            settings
                .stamps
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    );
    root.insert(
        "templates".to_owned(),
        Value::Object(
            settings
                .templates
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    );

    if let Some(overrides) = settings.os_overrides.get(&target_os) {
        for (k, v) in overrides {
            root.insert(k.clone(), v.clone());
        }
    }
    Value::Object(root)
}

/// Merges `remote` portable settings into `local`, keeping the latest pen by `updated_at` and
/// preserving each OS's own override table.
pub fn merge_portable_settings(local: &mut PortableSettings, remote: &PortableSettings) {
    for (k, v) in &remote.preferences {
        local.preferences.insert(k.clone(), v.clone());
    }
    for (k, v) in &remote.keymap {
        local.keymap.insert(k.clone(), v.clone());
    }
    for (id, pen) in &remote.pens {
        match local.pens.get(id) {
            Some(existing) if existing.updated_at >= pen.updated_at => {}
            _ => {
                local.pens.insert(id.clone(), pen.clone());
            }
        }
    }
    for (k, v) in &remote.palettes {
        local.palettes.insert(k.clone(), v.clone());
    }
    for (k, v) in &remote.stamps {
        local.stamps.insert(k.clone(), v.clone());
    }
    for (k, v) in &remote.templates {
        local.templates.insert(k.clone(), v.clone());
    }
    for (os, map) in &remote.os_overrides {
        let target = local.os_overrides.entry(*os).or_default();
        for (k, v) in map {
            target.insert(k.clone(), v.clone());
        }
    }
}

/// Serializes [`PortableSettings`] to JSON.
///
/// # Errors
///
/// Returns [`SyncError::InvalidFormat`] if JSON serialization fails.
pub fn portable_settings_to_json(settings: &PortableSettings) -> Result<String, SyncError> {
    let overrides: Map<String, Value> = settings
        .os_overrides
        .iter()
        .map(|(os, map)| (os.as_str().to_owned(), Value::Object(map.clone())))
        .collect();
    let mut base = match materialize_for_os(settings, TargetOs::Linux) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    base.insert("keymap".to_owned(), json!(settings.keymap));
    base.insert("os_overrides".to_owned(), Value::Object(overrides));
    serde_json::to_string_pretty(&Value::Object(base))
        .map_err(|e| SyncError::InvalidFormat(e.to_string()))
}

fn normalize_shortcut(raw: &str) -> String {
    raw.replace("Cmd+", "Mod+").replace("Ctrl+", "Mod+")
}

fn is_os_specific_key(key: &str) -> bool {
    key.starts_with("win_")
        || key.starts_with("mac_")
        || key.starts_with("linux_")
        || key == "dpi_override"
}

fn sanitize_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace('\\', "/")),
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_value).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !LOCAL_ONLY_KEYS.contains(&k.as_str()))
                .map(|(k, val)| (k.clone(), sanitize_value(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn parse_pen(v: &Value) -> Option<PortablePen> {
    let id = v.get("id")?.as_str()?.to_owned();
    let name = v.get("name")?.as_str()?.to_owned();
    let color = v.get("color")?.as_str()?.to_owned();
    let width_pt = v.get("width_pt")?.as_f64()? as f32;
    let updated_at = v.get("updated_at").and_then(Value::as_u64).unwrap_or(0);
    Some(PortablePen {
        id,
        name,
        color,
        width_pt,
        updated_at,
    })
}
