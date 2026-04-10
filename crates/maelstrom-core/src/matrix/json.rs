//! Canonical JSON — deterministic serialization for cryptographic signing.
//!
//! # Why canonical JSON exists
//!
//! Matrix requires cryptographic signatures on events and federation requests.
//! A signature is computed over the raw bytes of a JSON object, but standard
//! JSON serialization is non-deterministic: `{"a":1,"b":2}` and `{"b":2,"a":1}`
//! represent the same logical value yet produce different byte strings and
//! therefore different signatures. If two servers serialize the same event
//! differently, signature verification fails.
//!
//! Canonical JSON solves this by defining a single, deterministic serialization
//! for any JSON value:
//!
//! - **Sorted keys** — object keys are ordered lexicographically (UTF-8 byte
//!   order), so every implementation produces the same key ordering.
//! - **Integer-only numbers** — floating-point numbers are forbidden. The Matrix
//!   spec mandates this because IEEE 754 floats have platform-dependent
//!   rounding, which would again break determinism. All numeric values in
//!   Matrix events are integers (timestamps, power levels, depth, etc.).
//! - **No unnecessary whitespace** — compact encoding with no spaces or
//!   newlines between tokens.
//!
//! This module provides [`CanonicalJson`], the value type, and
//! [`CanonicalJsonObject`], its object variant. The typical flow is:
//!
//! ```text
//! serde_json::Value  →  CanonicalJson::from_value()  →  .encode()  →  bytes to sign
//! ```
//!
//! See also: [Matrix spec — Canonical JSON](https://spec.matrix.org/latest/appendices/#canonical-json)

use std::collections::BTreeMap;
use std::fmt;

/// A JSON value type that guarantees deterministic serialization.
///
/// Unlike `serde_json::Value`, this type enforces the constraints required by
/// the Matrix canonical JSON specification:
///
/// - Objects use [`BTreeMap`] internally, so keys are always sorted
///   lexicographically — no matter what order they were inserted.
/// - Numbers are restricted to `i64` — there is no float variant. Attempting
///   to convert a `serde_json::Value` containing a float will return
///   [`CanonicalJsonError::Float`]. This is a hard requirement of the Matrix
///   spec because floating-point serialization is platform-dependent.
/// - [`encode()`](Self::encode) produces a compact JSON string (no whitespace)
///   with sorted keys at every nesting level. This is the byte string that
///   gets fed into SHA-256 and Ed25519 during event signing.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalJson {
    Null,
    Bool(bool),
    Integer(i64),
    String(String),
    Array(Vec<CanonicalJson>),
    Object(CanonicalJsonObject),
}

/// A canonical JSON object — a [`BTreeMap`] from string keys to [`CanonicalJson`] values.
///
/// The `BTreeMap` ensures keys are always iterated in lexicographic order,
/// which is the sorting rule required by canonical JSON.
pub type CanonicalJsonObject = BTreeMap<String, CanonicalJson>;

impl CanonicalJson {
    /// Convert a `serde_json::Value` into canonical form, rejecting floats.
    ///
    /// This is the main entry point for building a [`CanonicalJson`]. It walks
    /// the entire value tree and returns [`CanonicalJsonError::Float`] if any
    /// number is not representable as `i64`. Objects are re-keyed into a
    /// `BTreeMap` so that key order becomes deterministic regardless of the
    /// original insertion order in the `serde_json::Map`.
    pub fn from_value(value: &serde_json::Value) -> Result<Self, CanonicalJsonError> {
        match value {
            serde_json::Value::Null => Ok(Self::Null),
            serde_json::Value::Bool(b) => Ok(Self::Bool(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Self::Integer(i))
                } else if let Some(u) = n.as_u64() {
                    Ok(Self::Integer(u as i64))
                } else {
                    Err(CanonicalJsonError::Float)
                }
            }
            serde_json::Value::String(s) => Ok(Self::String(s.clone())),
            serde_json::Value::Array(arr) => {
                let items: Result<Vec<_>, _> = arr.iter().map(Self::from_value).collect();
                Ok(Self::Array(items?))
            }
            serde_json::Value::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k.clone(), Self::from_value(v)?);
                }
                Ok(Self::Object(map))
            }
        }
    }

    /// Convert back to a regular `serde_json::Value`.
    ///
    /// Useful when you need to hand the event back to serde-based APIs after
    /// signing. Note that the resulting `Value` will have sorted keys (because
    /// `BTreeMap` iterates in order), but this is not guaranteed by
    /// `serde_json::Value` itself — always use [`encode()`](Self::encode) when
    /// you need deterministic bytes.
    pub fn into_value(self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(b),
            Self::Integer(i) => serde_json::json!(i),
            Self::String(s) => serde_json::Value::String(s),
            Self::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(|v| v.into_value()).collect())
            }
            Self::Object(obj) => {
                let map: serde_json::Map<String, serde_json::Value> =
                    obj.into_iter().map(|(k, v)| (k, v.into_value())).collect();
                serde_json::Value::Object(map)
            }
        }
    }

    /// Serialize to the canonical JSON byte string (sorted keys, no whitespace).
    ///
    /// This is the output that gets fed into cryptographic operations — SHA-256
    /// content hashing and Ed25519 signing. The result is compact JSON with:
    /// - Keys sorted lexicographically at every nesting level
    /// - No whitespace between tokens
    /// - Strings escaped via `serde_json::to_string` (handles unicode, control
    ///   characters, etc.)
    ///
    /// Two [`CanonicalJson`] values that are logically equal will always
    /// produce the same `encode()` output, byte-for-byte.
    pub fn encode(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => if *b { "true" } else { "false" }.into(),
            Self::Integer(i) => i.to_string(),
            Self::String(s) => serde_json::to_string(s).unwrap(),
            Self::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.encode()).collect();
                format!("[{}]", items.join(","))
            }
            Self::Object(obj) => {
                let pairs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = serde_json::to_string(k).unwrap();
                        format!("{}:{}", key, v.encode())
                    })
                    .collect();
                format!("{{{}}}", pairs.join(","))
            }
        }
    }

    /// Get a mutable reference to the inner object map, if this value is an object.
    ///
    /// Returns `None` for all other variants. Used by the signing code to
    /// insert `hashes` and `signatures` fields into an event.
    pub fn as_object_mut(&mut self) -> Option<&mut CanonicalJsonObject> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get a reference to the inner object map, if this value is an object.
    ///
    /// Returns `None` for all other variants. Used by verification code to
    /// read the `signatures` field from an event.
    pub fn as_object(&self) -> Option<&CanonicalJsonObject> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Remove a key from this object, returning the value if it existed.
    ///
    /// No-op (returns `None`) if this value is not an object. Used by
    /// [`strip_fields`](super::signing::strip_fields) to remove `signatures`,
    /// `unsigned`, and `hashes` before hashing or signing.
    pub fn remove(&mut self, key: &str) -> Option<CanonicalJson> {
        self.as_object_mut().and_then(|obj| obj.remove(key))
    }
}

impl fmt::Display for CanonicalJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// Errors that can occur when converting a `serde_json::Value` to [`CanonicalJson`].
///
/// Currently the only failure mode is encountering a floating-point number.
/// You will see this error if you try to canonicalize a JSON value that
/// contains a float (e.g., `1.5`). This typically means the event was
/// malformed — all numeric fields in Matrix events (timestamps, power levels,
/// depth, etc.) are integers.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalJsonError {
    /// A floating-point number was found in the JSON value. The Matrix spec
    /// forbids floats in canonical JSON because IEEE 754 serialization is
    /// platform-dependent, which would break signature determinism.
    #[error("Floating-point numbers are not allowed in canonical JSON")]
    Float,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_keys() {
        let val = CanonicalJson::from_value(&serde_json::json!({"z": 1, "a": 2})).unwrap();
        assert_eq!(val.encode(), r#"{"a":2,"z":1}"#);
    }

    #[test]
    fn nested_sorted() {
        let val = CanonicalJson::from_value(&serde_json::json!({"b": {"z": 1, "a": 2}, "a": []}))
            .unwrap();
        assert_eq!(val.encode(), r#"{"a":[],"b":{"a":2,"z":1}}"#);
    }

    #[test]
    fn rejects_float() {
        assert!(CanonicalJson::from_value(&serde_json::json!({"key": 1.5})).is_err());
    }

    #[test]
    fn roundtrip() {
        let original = serde_json::json!({"b": 1, "a": [true, null, "hello"]});
        let canonical = CanonicalJson::from_value(&original).unwrap();
        let back = canonical.into_value();
        assert_eq!(original, back);
    }
}
