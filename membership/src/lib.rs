//! Provided membership implementations for OpenRaft.
//!
//! Each membership sets the read quorums that elect a leader and the write quorums that commit an
//! entry, and makes every read quorum share a voter with every write quorum:
//!
//! - [`ThresholdMembership`] configures the write quorum size and derives the read quorum size.
//! - [`WeightedMembership`] gives each voter a weight and configures the write weight.
//!
//! [`docs`] tells which membership fits which cluster, and holds a guide for each.
//! The default `serde` feature also enables `openraft/serde`.
//!
//! Keep the `serde` feature on whenever any crate in the build enables `openraft/serde`. With
//! `openraft/serde` on, [`RaftMembership`] requires serde, and this crate derives serde for its
//! memberships only under its own `serde` feature. Cargo merges the features of
//! `openraft` across the whole build, so another crate can turn `openraft/serde` on even when
//! this crate is used with `default-features = false`. That build fails to compile.
//!
//! [`RaftMembership`]: openraft::membership::RaftMembership

#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

mod config;
pub mod errors;
pub mod quorum;
#[cfg(test)]
mod testing;
mod threshold;
mod weighted;

pub use threshold::ThresholdChange;
pub use threshold::ThresholdMembership;
pub use weighted::WeightedChange;
pub use weighted::WeightedMembership;

/// Pick a provided membership, then configure and change it.
pub mod docs {
    #![doc = include_str!("docs.md")]

    /// Configure and change threshold memberships.
    pub mod threshold {
        #![doc = include_str!("threshold-membership.md")]
    }

    /// Configure and change weighted memberships.
    pub mod weighted {
        #![doc = include_str!("weighted-membership.md")]
    }
}
