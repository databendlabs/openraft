//! Shared KV request/response types for example crates.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// A request to the KV store.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    /// Store `value` under `key`.
    ///
    /// Always applies; the reply carries the written value and its new version.
    Set { key: String, value: String },

    /// Store `value` under `key` only if the key exists and its current version equals
    /// `expected_version`.
    ///
    /// A missing key never matches, so use [`Request::Set`] to create a key. The reply carries the
    /// written value and its new version, or no value when the version does not match.
    CompareAndSet {
        key: String,
        expected_version: u64,
        value: String,
    },
}

impl Request {
    pub fn set(key: impl Into<String>, value: impl Into<String>) -> Self {
        Request::Set {
            key: key.into(),
            value: value.into(),
        }
    }

    pub fn compare_and_set(key: impl Into<String>, expected_version: u64, value: impl Into<String>) -> Self {
        Request::CompareAndSet {
            key: key.into(),
            expected_version,
            value: value.into(),
        }
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Set { key, value } => write!(f, "Set {{ key: {}, value: {} }}", key, value),
            Request::CompareAndSet {
                key,
                expected_version,
                value,
            } => write!(
                f,
                "CompareAndSet {{ key: {}, expected_version: {}, value: {} }}",
                key, expected_version, value
            ),
        }
    }
}

/// A response from the KV store.
///
/// After a write that applies, `value` holds the written value and its new version. It is `None`
/// after a [`Request::CompareAndSet`] whose version does not match, and after a blank or
/// membership log entry, which carries no command. The examples' read endpoints reuse this type:
/// `value` is the stored value, or `None` when the key is absent.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub value: Option<VersionedValue>,
}

impl Response {
    pub fn new(value: impl Into<String>, version: u64) -> Self {
        Response {
            value: Some(VersionedValue {
                value: value.into(),
                version,
            }),
        }
    }

    pub fn none() -> Self {
        Response { value: None }
    }
}

/// A value stored under a key, and the version of the write that stored it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VersionedValue {
    pub value: String,

    /// The log index of the entry that last wrote the key.
    ///
    /// Every write applied to the key raises it, so [`Request::CompareAndSet`] detects any write
    /// made since the version was read.
    pub version: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_commands_keep_their_json_and_postcard_schema() {
        let fixtures = [
            (r#"{"Set":{"key":"k","value":"v"}}"#, &[0, 1, b'k', 1, b'v'][..]),
            (
                r#"{"CompareAndSet":{"key":"k","expected_version":7,"value":"v"}}"#,
                &[1, 1, b'k', 7, 1, b'v'][..],
            ),
        ];
        for (json, binary) in fixtures {
            let json_request: Request = serde_json::from_str(json).unwrap();
            let binary_request: Request = postcard::from_bytes(binary).unwrap();
            let actual_json = serde_json::to_string(&binary_request).unwrap();
            let actual_binary = postcard::to_allocvec(&json_request).unwrap();
            assert_eq!(json, actual_json);
            assert_eq!(binary, actual_binary);
        }
    }

    #[test]
    fn persisted_values_keep_their_json_and_postcard_schema() {
        let json = r#"{"value":"v","version":7}"#;
        let binary = [1, b'v', 7];
        let expected = VersionedValue {
            value: "v".to_string(),
            version: 7,
        };
        let from_json: VersionedValue = serde_json::from_str(json).unwrap();
        let from_binary: VersionedValue = postcard::from_bytes(&binary).unwrap();
        assert_eq!(expected, from_json);
        assert_eq!(expected, from_binary);
        let actual_json = serde_json::to_string(&expected).unwrap();
        let actual_binary = postcard::to_allocvec(&expected).unwrap();
        assert_eq!(json, actual_json);
        assert_eq!(binary.as_slice(), actual_binary);
    }
}
