//! JSON key normalization and decoding.

use crate::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Uppercases the first letter of every object key, recursively.
///
/// Jellyfin 10.x answers in PascalCase while 12.x answers in camelCase.
pub(crate) fn normalize_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (capitalize(&key), normalize_keys(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_keys).collect()),
        other => other,
    }
}

fn capitalize(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Parses `bytes` as JSON, normalizes its keys and deserializes it into `T`.
pub(crate) fn decode<T: DeserializeOwned>(route: &str, bytes: &[u8]) -> Result<T> {
    let decode_error = |source| Error::Decode {
        route: route.to_owned(),
        source,
    };
    let value: Value = serde_json::from_slice(bytes).map_err(decode_error)?;
    serde_json::from_value(normalize_keys(value)).map_err(decode_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use serde_json::json;

    #[test]
    fn normalize_keys_uppercases_first_letter_recursively() {
        let input = json!({"localAddress":"a","items":[{"id":"1","userData":{"isFavorite":true}}]});
        let expected =
            json!({"LocalAddress":"a","Items":[{"Id":"1","UserData":{"IsFavorite":true}}]});
        assert_eq!(normalize_keys(input), expected);
    }

    #[test]
    fn normalize_keys_keeps_pascal_case_and_values() {
        let input = json!({"Name":"camelCase value"});
        assert_eq!(normalize_keys(input.clone()), input);
    }

    #[test]
    fn decode_reports_route_on_html() {
        let err =
            decode::<serde_json::Value>("System/Info/Public", b"<!DOCTYPE html>").unwrap_err();
        match err {
            Error::Decode { route, .. } => assert_eq!(route, "System/Info/Public"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
