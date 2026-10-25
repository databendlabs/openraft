//! Provided membership implementations for OpenRaft.
//!
//! [`ThresholdMembership`] configures the write quorum size and derives an intersecting read
//! quorum. See [`docs`] for configuration and membership changes.
//! [`ThresholdMembership`] and its quorum sets implement `serde::Serialize` and
//! `serde::Deserialize` in every feature combination, when their ID and node types do. The default
//! `serde` feature only enables `openraft/serde`.

#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

pub mod errors;
pub mod quorum;
mod threshold;

pub use threshold::ThresholdChange;
pub use threshold::ThresholdMembership;

/// Configure and change threshold memberships.
pub mod docs {
    #![doc = include_str!("threshold-membership.md")]
}
