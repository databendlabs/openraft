//! Provided membership implementations for OpenRaft.
//!
//! [`ThresholdMembership`] configures the write quorum size and derives an intersecting read
//! quorum. See [`docs`] for configuration and membership changes.
//! The default `serde` feature also enables `openraft/serde`.
//!
//! Keep the `serde` feature on whenever any crate in the build enables `openraft/serde`. With
//! `openraft/serde` on, [`RaftMembership`] requires serde, and this crate derives serde for
//! [`ThresholdMembership`] only under its own `serde` feature. Cargo merges the features of
//! `openraft` across the whole build, so another crate can turn `openraft/serde` on even when
//! this crate is used with `default-features = false`. That build fails to compile.
//!
//! [`RaftMembership`]: openraft::membership::RaftMembership

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
