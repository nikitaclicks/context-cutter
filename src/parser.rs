//! Parsing and query logic for teaser generation and JSONPath selection.

use jsonpath_rust::JsonPath;
use serde_json::{json, Map, Value};

use crate::error::ContextCutterError;

const MAX_DEPTH: usize = 3;
const MAX_STRING_LEN: usize = 24;
const MAX_JSON_PATH_LEN: usize = 4096;

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn small_scalar(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) => Some(value.clone()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i.abs() < 10_000 {
                    Some(Value::Number(n.clone()))
                } else {
                    Some(Value::String("int".to_string()))
                }
            } else if let Some(u) = n.as_u64() {
                if u < 10_000 {
                    Some(Value::Number(n.clone()))
                } else {
                    Some(Value::String("int".to_string()))
                }
            } else if let Some(f) = n.as_f64() {
                if f.abs() < 10_000.0 {
                    Some(Value::Number(n.clone()))
                } else {
                    Some(Value::String("float".to_string()))
                }
            } else {
                Some(Value::String("number".to_string()))
            }
        }
        Value::String(s) => {
            if s.len() <= MAX_STRING_LEN {
                Some(Value::String(s.clone()))
            } else {
                Some(Value::String("string".to_string()))
            }
        }
        _ => None,
    }
}

fn summarize(value: &Value, depth: usize) -> Value {
    if depth >= MAX_DEPTH {
        return match value {
            Value::Object(_) => Value::String("{...}".to_string()),
            Value::Array(arr) => Value::String(format!("Array[{}]", arr.len())),
            _ => small_scalar(value).unwrap_or_else(|| Value::String(value_type_name(value).to_string())),
        };
    }

    match value {
        Value::Object(obj) => {
            let mut map = Map::new();
            for (k, v) in obj {
                map.insert(k.clone(), summarize(v, depth + 1));
            }
            Value::Object(map)
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                return Value::String("Array[0]".to_string());
            }
            let first = &arr[0];
            if let Value::Object(first_obj) = first {
                let mut keys: Vec<String> = first_obj.keys().cloned().collect();
                keys.sort();
                json!({
                    "_type": format!("Array[{}]", arr.len()),
                    "item_keys": keys
                })
            } else if first.is_array() {
                json!({
                    "_type": format!("Array[{}]", arr.len()),
                    "item": summarize(first, depth + 1)
                })
            } else {
                json!({
                    "_type": format!("Array[{}]", arr.len()),
                    "item_type": small_scalar(first).unwrap_or_else(|| Value::String(value_type_name(first).to_string()))
                })
            }
        }
        _ => small_scalar(value).unwrap_or_else(|| Value::String(value_type_name(value).to_string())),
    }
}

/// Builds a lightweight teaser string for a stored JSON value.
pub fn generate_teaser_from_value(value: &Value) -> String {
    let teaser = match value {
        Value::Object(obj) => {
            let mut keys: Vec<String> = obj.keys().cloned().collect();
            keys.sort();
            json!({
                "_teaser": true,
                "_type": "object",
                "keys": keys,
                "structure": summarize(value, 0)
            })
        }
        Value::Array(arr) => json!({
            "_teaser": true,
            "_type": format!("Array[{}]", arr.len()),
            "structure": summarize(value, 0)
        }),
        _ => json!({
            "_teaser": true,
            "_type": value_type_name(value),
            "structure": summarize(value, 0)
        }),
    };
    teaser.to_string()
}

fn normalize_json_path(path: &str) -> Result<String, ContextCutterError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ContextCutterError::Validation(
            "json path must not be empty".to_string(),
        ));
    }
    if path.as_bytes().contains(&0) {
        return Err(ContextCutterError::Validation(
            "json path must not contain null bytes".to_string(),
        ));
    }
    if trimmed.len() > MAX_JSON_PATH_LEN {
        return Err(ContextCutterError::Validation(format!(
            "json path too long: {} bytes (max {})",
            trimmed.len(),
            MAX_JSON_PATH_LEN
        )));
    }
    if trimmed.starts_with('$') {
        return Ok(trimmed.to_string());
    }

    // Convert dot notation with numeric segments to JSONPath indices.
    // Example: contributors.0.login -> $.contributors[0].login
    let mut out = String::from("$.");
    let mut i = 0usize;
    let bytes = trimmed.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'.' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && (j == bytes.len() || bytes[j] == b'.' || bytes[j] == b'[') {
                out.push('[');
                out.push_str(&trimmed[i + 1..j]);
                out.push(']');
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    Ok(out)
}

/// Executes a JSONPath query against a stored value.
pub fn query_json_path(value: &Value, json_path: &str) -> Result<String, ContextCutterError> {
    let normalized = normalize_json_path(json_path)?;
    let matches = value
        .query(&normalized)
        .map_err(|e| ContextCutterError::InvalidJsonPath(e.to_string()))?;

    if matches.is_empty() {
        return Ok("null".to_string());
    }
    if matches.len() == 1 {
        return serde_json::to_string(matches[0])
            .map_err(|e| ContextCutterError::Serialize(e.to_string()));
    }
    serde_json::to_string(
        &matches
            .into_iter()
            .cloned()
            .collect::<Vec<Value>>(),
    )
    .map_err(|e| ContextCutterError::Serialize(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn normalize_json_path_supports_dot_index_notation() {
        let normalized = normalize_json_path("contributors.0.login").expect("path should normalize");
        assert_eq!(normalized, "$.contributors[0].login");
    }

    #[test]
    fn normalize_json_path_rejects_empty_paths() {
        let err = normalize_json_path("   ").expect_err("empty path should fail");
        assert!(err.to_string().contains("json path must not be empty"));
    }

    #[test]
    fn teaser_includes_object_keys_and_array_shape() {
        let payload = json!({
            "contributors": [
                {"login": "octocat", "id": 1},
                {"login": "hubot", "id": 2}
            ],
            "meta": {"total": 2}
        });
        let teaser: Value = serde_json::from_str(&generate_teaser_from_value(&payload))
            .expect("teaser should be valid json");

        assert_eq!(teaser["_teaser"], json!(true));
        assert_eq!(teaser["_type"], json!("object"));
        assert!(teaser["keys"]
            .as_array()
            .expect("keys must be array")
            .contains(&json!("contributors")));
        assert_eq!(teaser["structure"]["contributors"]["_type"], json!("Array[2]"));
    }

    #[test]
    fn query_json_path_returns_null_for_no_matches() {
        let payload = json!({"items": [1, 2, 3]});
        let out = query_json_path(&payload, "$.missing").expect("query should succeed");
        assert_eq!(out, "null");
    }

    #[test]
    fn query_json_path_returns_scalar_for_single_match() {
        let payload = json!({"items": [{"login": "octocat"}]});
        let out = query_json_path(&payload, "items.0.login").expect("query should succeed");
        assert_eq!(out, "\"octocat\"");
    }

    #[test]
    fn query_json_path_returns_array_for_multiple_matches() {
        let payload = json!({"items": [1, 2, 3]});
        let out = query_json_path(&payload, "$.items[*]").expect("query should succeed");
        assert_eq!(out, "[1,2,3]");
    }
}
