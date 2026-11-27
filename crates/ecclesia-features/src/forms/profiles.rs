//! Reusable form profiles and autofill templates (plan O.12).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ecclesia_core::forms::{AcroForm, FieldKind, FieldValue};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::exchange::FormExchangeError;

/// Strategy for matching profile keys to target PDF form field names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProfileMatchMode {
    /// Match only exact field names, mapping names, or explicit profile aliases.
    ExactName,
    /// Normalize case, spaces, underscores, and hyphens (`"First Name"` matches `"first_name"`).
    #[default]
    NormalizedName,
}

/// Reusable autofill profile mapping logical field names (and optional aliases) to saved values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormProfile {
    /// Unique profile identifier (e.g. `"work-contact"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Saved field values by canonical field name.
    pub values: BTreeMap<String, FieldValue>,
    /// Alternative field names mapped to a canonical key in `values`.
    pub aliases: BTreeMap<String, Vec<String>>,
    /// Last update time in Unix seconds.
    pub updated_at: i64,
}

impl FormProfile {
    /// Captures all non-empty exportable values from `form` into a reusable [`FormProfile`].
    #[must_use]
    pub fn from_form(
        id: impl Into<String>,
        name: impl Into<String>,
        form: &AcroForm,
        updated_at: i64,
    ) -> Self {
        let mut values = BTreeMap::new();
        for field in form.fields() {
            if matches!(
                field.kind,
                FieldKind::Signature { .. } | FieldKind::Pushbutton { .. }
            ) || field.flags.is_no_export()
                || field.value.is_empty()
            {
                continue;
            }
            values.insert(field.name.clone(), field.value.clone());
        }
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            values,
            aliases: BTreeMap::new(),
            updated_at,
        }
    }

    /// Serializes this profile to JSON.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        let mut vals = Map::new();
        for (k, v) in &self.values {
            let jv = match v {
                FieldValue::None => Value::Null,
                FieldValue::Text(s) => Value::String(s.clone()),
                FieldValue::Boolean(b) => Value::Bool(*b),
                FieldValue::Choice(items) => {
                    Value::Array(items.iter().cloned().map(Value::String).collect())
                }
            };
            vals.insert(k.clone(), jv);
        }
        let mut alias_map = Map::new();
        for (k, list) in &self.aliases {
            alias_map.insert(
                k.clone(),
                Value::Array(list.iter().cloned().map(Value::String).collect()),
            );
        }
        json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "values": vals,
            "aliases": alias_map,
            "updated_at": self.updated_at,
        })
    }

    /// Deserializes a [`FormProfile`] from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`FormExchangeError::Format`] if required keys are missing or invalid.
    pub fn from_json_value(value: &Value) -> Result<Self, FormExchangeError> {
        let obj = value.as_object().ok_or_else(|| {
            FormExchangeError::Format("profile JSON must be an object".to_owned())
        })?;
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| FormExchangeError::Format("profile missing id".to_owned()))?
            .to_owned();
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| FormExchangeError::Format("profile missing name".to_owned()))?
            .to_owned();
        let description = obj
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let updated_at = obj.get("updated_at").and_then(Value::as_i64).unwrap_or(0);

        let mut values = BTreeMap::new();
        if let Some(vmap) = obj.get("values").and_then(Value::as_object) {
            for (k, v) in vmap {
                let fv = match v {
                    Value::Null => FieldValue::None,
                    Value::Bool(b) => FieldValue::Boolean(*b),
                    Value::String(s) => FieldValue::Text(s.clone()),
                    Value::Number(n) => FieldValue::Text(n.to_string()),
                    Value::Array(arr) => FieldValue::Choice(
                        arr.iter()
                            .filter_map(|i| i.as_str().map(str::to_owned))
                            .collect(),
                    ),
                    Value::Object(_) => continue,
                };
                values.insert(k.clone(), fv);
            }
        }

        let mut aliases = BTreeMap::new();
        if let Some(amap) = obj.get("aliases").and_then(Value::as_object) {
            for (k, v) in amap {
                if let Some(arr) = v.as_array() {
                    aliases.insert(
                        k.clone(),
                        arr.iter()
                            .filter_map(|i| i.as_str().map(str::to_owned))
                            .collect(),
                    );
                }
            }
        }

        Ok(Self {
            id,
            name,
            description,
            values,
            aliases,
            updated_at,
        })
    }
}

/// Report of applying a [`FormProfile`] to a target [`AcroForm`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutofillReport {
    /// Target field names filled from the profile.
    pub matched_fields: Vec<String>,
    /// Target fields skipped because they are read-only or incompatible.
    pub skipped_fields: Vec<String>,
    /// Profile keys that matched no field in the target form.
    pub unmatched_keys: Vec<String>,
    /// Calculated fields updated after autofill.
    pub calculated_fields: Vec<String>,
}

/// Applies `profile` values onto `form` using `match_mode` and recalculates dependent fields.
///
/// # Errors
///
/// Returns [`FormExchangeError::Form`] if the calculation DAG fails.
pub fn apply_profile(
    profile: &FormProfile,
    form: &mut AcroForm,
    match_mode: ProfileMatchMode,
) -> Result<AutofillReport, FormExchangeError> {
    let mut report = AutofillReport::default();
    let field_names: Vec<String> = form.fields().iter().map(|f| f.name.clone()).collect();

    for (profile_key, val) in &profile.values {
        let candidates = profile.aliases.get(profile_key);
        let matched_name = field_names.iter().find(|fname| {
            let export_name = form
                .field(fname)
                .map_or(fname.as_str(), |f| f.export_name());
            matches_key(profile_key, fname, export_name, match_mode)
                || candidates.is_some_and(|list| {
                    list.iter()
                        .any(|alias| matches_key(alias, fname, export_name, match_mode))
                })
        });

        let Some(target) = matched_name.cloned() else {
            report.unmatched_keys.push(profile_key.clone());
            continue;
        };
        let Some(field) = form.field(&target) else {
            continue;
        };
        if field.flags.is_read_only()
            || matches!(
                field.kind,
                FieldKind::Signature { .. } | FieldKind::Pushbutton { .. }
            )
        {
            report.skipped_fields.push(target);
            continue;
        }
        match form.set_value(&target, val.clone()) {
            Ok(_) => report.matched_fields.push(target),
            Err(_) => report.skipped_fields.push(target),
        }
    }

    let calc = form.recalculate()?;
    report.calculated_fields = calc.updated_fields;
    Ok(report)
}

fn matches_key(
    profile_key: &str,
    field_name: &str,
    export_name: &str,
    mode: ProfileMatchMode,
) -> bool {
    match mode {
        ProfileMatchMode::ExactName => profile_key == field_name || profile_key == export_name,
        ProfileMatchMode::NormalizedName => {
            let norm = normalize_key(profile_key);
            !norm.is_empty()
                && (norm == normalize_key(field_name) || norm == normalize_key(export_name))
        }
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// In-memory, JSON-file, and SQLite-backed store for reusable [`FormProfile`] autofill templates.
#[derive(Debug, Clone, Default)]
pub struct ProfileStore {
    profiles: BTreeMap<String, FormProfile>,
}

impl ProfileStore {
    /// Creates an empty [`ProfileStore`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates `profile`.
    pub fn save(&mut self, profile: FormProfile) {
        self.profiles.insert(profile.id.clone(), profile);
    }

    /// Returns a profile by `id`.
    #[must_use]
    pub fn profile(&self, id: &str) -> Option<&FormProfile> {
        self.profiles.get(id)
    }

    /// Removes a profile by `id`.
    pub fn remove(&mut self, id: &str) -> Option<FormProfile> {
        self.profiles.remove(id)
    }

    /// Lists all profiles sorted by id.
    #[must_use]
    pub fn list(&self) -> Vec<&FormProfile> {
        self.profiles.values().collect()
    }

    /// Saves all profiles to a JSON file on disk.
    ///
    /// # Errors
    ///
    /// Returns [`FormExchangeError`] on serialization or I/O failure.
    pub fn save_json_file(&self, path: impl AsRef<Path>) -> Result<(), FormExchangeError> {
        let items: Vec<Value> = self
            .profiles
            .values()
            .map(FormProfile::to_json_value)
            .collect();
        let json = serde_json::to_string_pretty(&Value::Array(items))?;
        if let Some(parent) = path.as_ref().parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
        Ok(())
    }

    /// Loads profiles from a JSON file on disk.
    ///
    /// # Errors
    ///
    /// Returns [`FormExchangeError`] if the file cannot be read or parsed.
    pub fn load_json_file(path: impl AsRef<Path>) -> Result<Self, FormExchangeError> {
        let raw = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&raw)?;
        let arr = val.as_array().ok_or_else(|| {
            FormExchangeError::Format("profile file must be a JSON array".to_owned())
        })?;
        let mut store = Self::new();
        for item in arr {
            store.save(FormProfile::from_json_value(item)?);
        }
        Ok(store)
    }

    /// Ensures the `form_profiles` table exists in SQLite `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`rusqlite::Error`] if DDL fails.
    pub fn init_sqlite_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS form_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
    }

    /// Persists all profiles in `self` into SQLite `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`FormExchangeError::Format`] on SQLite failure.
    pub fn save_to_sqlite(&self, conn: &Connection) -> Result<(), FormExchangeError> {
        Self::init_sqlite_schema(conn)
            .map_err(|err| FormExchangeError::Format(format!("sqlite init failed: {err}")))?;
        for p in self.profiles.values() {
            let payload = serde_json::to_string(&p.to_json_value())?;
            conn.execute(
                "INSERT INTO form_profiles (id, name, payload_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   payload_json = excluded.payload_json,
                   updated_at = excluded.updated_at",
                params![p.id, p.name, payload, p.updated_at],
            )
            .map_err(|err| FormExchangeError::Format(format!("sqlite save failed: {err}")))?;
        }
        Ok(())
    }

    /// Loads all profiles from SQLite `conn`.
    ///
    /// # Errors
    ///
    /// Returns [`FormExchangeError`] on SQLite or JSON error.
    pub fn load_from_sqlite(conn: &Connection) -> Result<Self, FormExchangeError> {
        Self::init_sqlite_schema(conn)
            .map_err(|err| FormExchangeError::Format(format!("sqlite init failed: {err}")))?;
        let mut stmt = conn
            .prepare("SELECT payload_json FROM form_profiles ORDER BY id ASC")
            .map_err(|err| FormExchangeError::Format(format!("sqlite prepare failed: {err}")))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| FormExchangeError::Format(format!("sqlite query failed: {err}")))?;
        let mut store = Self::new();
        for row in rows {
            let raw = row.map_err(|err| {
                FormExchangeError::Format(format!("sqlite row read failed: {err}"))
            })?;
            let val: Value = serde_json::from_str(&raw)?;
            store.save(FormProfile::from_json_value(&val)?);
        }
        Ok(store)
    }
}
