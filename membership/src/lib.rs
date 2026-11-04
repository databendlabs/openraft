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

mod config;
pub mod errors;
mod explicit;
mod grid;
mod node_change;
pub mod quorum;
#[cfg(test)]
mod testing;
mod threshold;
mod weighted;

pub use config::Config;
pub use explicit::ExplicitChange;
pub use explicit::ExplicitMembership;
pub use grid::GridChange;
pub use grid::GridMembership;
pub use node_change::NodeChange;
pub use threshold::ThresholdChange;
pub use threshold::ThresholdMembership;
pub use weighted::WeightedChange;
pub use weighted::WeightedMembership;

/// Configure and change threshold memberships.
pub mod docs {
    #![doc = include_str!("threshold-membership.md")]

    /// Configure and change explicit memberships.
    pub mod explicit {
        #![doc = include_str!("explicit-membership.md")]
    }

    /// Configure and change weighted memberships.
    pub mod weighted {
        #![doc = include_str!("weighted-membership.md")]
    }

    /// Configure and change grid memberships.
    pub mod grid {
        #![doc = include_str!("grid-membership.md")]
    }
}
