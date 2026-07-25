//! Shared KV request/response types for example crates.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// A request to the KV store.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    Set {
        key: String,
        value: String,
    },
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
/// A compare-and-set operation returns the updated value on success and `None` when the expected
/// version does not match.
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

/// The current value of a key and its version.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VersionedValue {
    pub value: String,
    pub version: u64,
}
