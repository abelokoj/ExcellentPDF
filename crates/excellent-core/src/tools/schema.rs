//! Lightweight JSON Schema validator for tool input/output verification (E.2).

use serde_json::Value;

/// Validates `value` against a JSON Schema subset (`type`, `properties`, `required`,
/// `additionalProperties`, `items`, `enum`, `minimum`, `maximum`, `minLength`, `minItems`).
///
/// # Errors
///
/// Returns a human-readable validation error describing the first mismatch and its JSON path.
pub fn validate_json(value: &Value, schema: &Value) -> Result<(), String> {
    validate_at(value, schema, "$")
}

fn validate_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(obj) = schema.as_object() else {
        return Ok(());
    };

    if let Some(allowed) = obj.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        return Err(format!("{path}: value {value} not in allowed enum"));
    }

    if let Some(expected_type) = obj.get("type").and_then(Value::as_str) {
        let matches = match expected_type {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !matches {
            return Err(format!(
                "{path}: expected type '{expected_type}', got {value}"
            ));
        }
    }

    if let Some(s) = value.as_str()
        && let Some(min_len) = obj.get("minLength").and_then(Value::as_u64)
        && (s.chars().count() as u64) < min_len
    {
        return Err(format!("{path}: string shorter than minLength {min_len}"));
    }

    if let Some(num) = value.as_f64() {
        if let Some(min) = obj.get("minimum").and_then(Value::as_f64)
            && num < min
        {
            return Err(format!("{path}: {num} is less than minimum {min}"));
        }
        if let Some(max) = obj.get("maximum").and_then(Value::as_f64)
            && num > max
        {
            return Err(format!("{path}: {num} exceeds maximum {max}"));
        }
    }

    if let Some(arr) = value.as_array() {
        if let Some(min_items) = obj.get("minItems").and_then(Value::as_u64)
            && (arr.len() as u64) < min_items
        {
            return Err(format!(
                "{path}: array length {} is less than minItems {min_items}",
                arr.len()
            ));
        }
        if let Some(item_schema) = obj.get("items") {
            for (idx, item) in arr.iter().enumerate() {
                validate_at(item, item_schema, &format!("{path}[{idx}]"))?;
            }
        }
    }

    if let Some(map) = value.as_object() {
        if let Some(required) = obj.get("required").and_then(Value::as_array) {
            for req in required {
                if let Some(key) = req.as_str()
                    && !map.contains_key(key)
                {
                    return Err(format!("{path}: missing required property '{key}'"));
                }
            }
        }
        let props = obj.get("properties").and_then(Value::as_object);
        let disallow_extra =
            obj.get("additionalProperties").and_then(Value::as_bool) == Some(false);
        for (key, val) in map {
            let child_path = format!("{path}.{key}");
            if let Some(prop_schema) = props.and_then(|p| p.get(key)) {
                validate_at(val, prop_schema, &child_path)?;
            } else if disallow_extra {
                return Err(format!("{path}: unknown property '{key}'"));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validates_nested_object_and_rejects_bad_types() {
        let schema = json!({
            "type": "object",
            "required": ["path", "page"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string", "minLength": 1 },
                "page": { "type": "integer", "minimum": 0 },
                "mode": { "type": "string", "enum": ["a", "b"] }
            }
        });
        assert!(validate_json(&json!({"path": "a.pdf", "page": 0}), &schema).is_ok());
        assert!(validate_json(&json!({"path": "", "page": 0}), &schema).is_err());
        assert!(validate_json(&json!({"path": "a.pdf", "page": -1}), &schema).is_err());
        assert!(validate_json(&json!({"path": "a.pdf", "page": 0, "extra": 1}), &schema).is_err());
        assert!(validate_json(&json!({"path": "a.pdf", "page": 0, "mode": "c"}), &schema).is_err());
    }
}
