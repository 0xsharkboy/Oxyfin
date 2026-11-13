//! Helpers shared by the wiremock integration tests.
#![allow(dead_code)]

use oxyfin_api::{ClientInfo, Server, Session, UserId};
use serde_json::{Map, Value};
use std::path::Path;
use wiremock::MockServer;

/// Server versions the fixtures were captured from.
pub const VERSIONS: [&str; 3] = ["10.10.7", "10.11.11", "12.4.0"];

/// Token used by [`session`].
pub const TEST_TOKEN: &str = "test-token";

/// User id used by [`session`].
pub const TEST_USER_ID: &str = "0123456789abcdef0123456789abcdef";

/// Loads `tests/fixtures/<version>/<name>.json`.
pub fn fixture(version: &str, name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(version)
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("bad JSON in {}: {err}", path.display()))
}

fn map_keys(value: Value, f: &impl Fn(&str) -> String) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (f(&key), map_keys(value, f)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| map_keys(v, f)).collect()),
        other => other,
    }
}

fn with_first(key: &str, upper: bool) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if upper => first.to_uppercase().chain(chars).collect(),
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Lowercases the first letter of every object key, recursively.
pub fn to_camel_case(value: Value) -> Value {
    map_keys(value, &|key| with_first(key, false))
}

/// Uppercases the first letter of every object key, recursively.
pub fn to_pascal_case(value: Value) -> Value {
    map_keys(value, &|key| with_first(key, true))
}

/// Every version of a fixture, as captured, in camelCase and in PascalCase, with a label.
pub fn variants(name: &str) -> Vec<(String, Value)> {
    VERSIONS
        .iter()
        .flat_map(|version| {
            let captured = fixture(version, name);
            [
                (format!("{version}/{name} as captured"), captured.clone()),
                (
                    format!("{version}/{name} camelCase"),
                    to_camel_case(captured.clone()),
                ),
                (
                    format!("{version}/{name} PascalCase"),
                    to_pascal_case(captured),
                ),
            ]
        })
        .collect()
}

/// Client identity used by every test.
pub fn client_info() -> ClientInfo {
    ClientInfo::new("Test Device", "test-device-id")
}

/// The exact `Authorization` header [`client_info`] produces.
pub fn expected_authorization(token: Option<&str>) -> String {
    let mut header = format!(
        r#"MediaBrowser Client="Oxyfin", Device="Test%20Device", DeviceId="test-device-id", Version="{}""#,
        env!("CARGO_PKG_VERSION")
    );
    if let Some(token) = token {
        header.push_str(&format!(r#", Token="{token}""#));
    }
    header
}

/// A server pointing at `mock`.
pub fn server(mock: &MockServer) -> Server {
    Server::new(&mock.uri(), client_info())
        .unwrap_or_else(|err| panic!("cannot create a server for the mock: {err}"))
}

/// A session restored on `mock` with [`TEST_TOKEN`] and [`TEST_USER_ID`].
pub fn session(mock: &MockServer) -> Session {
    server(mock).restore_session(TEST_TOKEN, UserId::from(TEST_USER_ID))
}
