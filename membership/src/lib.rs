//! Provided membership implementations for OpenRaft.
//!
//! [`ThresholdMembership`] configures the write quorum size and derives an intersecting read
//! quorum. See [`docs`] for configuration and membership changes.
//! The default `serde` feature also enables `openraft/serde`.

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
