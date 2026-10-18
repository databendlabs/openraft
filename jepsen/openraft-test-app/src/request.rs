use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// A command the state machine applies.
///
/// `Set` and `CompareAndSet` serialize as `types_kv::Request` does, so the register workload's
/// requests are unchanged. `Append` adds an element to a list, for Elle's list-append workload,
/// where an element applied twice or out of order shows in a later read.
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
    Append {
        key: String,
        value: String,
    },
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
            Request::Append { key, value } => write!(f, "Append {{ key: {}, value: {} }}", key, value),
        }
    }
}
