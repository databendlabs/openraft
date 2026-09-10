//! Policies for automatically stepping down a Leader.

use openraft_macros::since;

/// Policy for automatically stepping down a Leader after a configured condition occurs.
///
/// The [`Config`](crate::Config) field using this type defines the triggering condition, when the
/// delay starts, and the transition performed after it expires.
#[since(version = "0.10.0")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum StepDownPolicy {
    /// Never step down automatically.
    Never,

    /// Allow the configured step-down transition after the specified number of milliseconds.
    After(u64),
}
