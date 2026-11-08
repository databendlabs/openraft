//! Provided membership implementations for OpenRaft.
//!
//! Each membership sets the read quorums that elect a leader and the write quorums that commit an
//! entry, and makes every read quorum share a voter with every write quorum:
//!
//! - [`ThresholdMembership`] configures the write quorum size and derives the read quorum size.
//! - [`WeightedMembership`] gives each voter a weight and configures the write weight.
//! - [`HierarchicalMembership`] nests voter groups and needs enough members at every level.
//! - [`GridMembership`] places voters in a grid: a row elects, and a voter of each row commits.
//! - [`ExplicitMembership`] lists the minimal read quorums and write quorums of each config.
//!
//! [`docs`] tells which membership fits which cluster, and holds a guide for each.
//!
//! The memberships derive serde with bounds on their type parameters, not behind a feature of this
//! crate. So they implement serde whenever `openraft/serde` is on, which is when
//! [`RaftMembership`] requires it.
//!
//! [`RaftMembership`]: openraft::membership::RaftMembership

#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

mod config;
pub mod errors;
mod explicit;
mod grid;
mod hierarchical;
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
pub use hierarchical::HierarchicalChange;
pub use hierarchical::HierarchicalMembership;
pub use node_change::NodeChange;
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

    /// Configure and change explicit memberships.
    pub mod explicit {
        #![doc = include_str!("explicit-membership.md")]
    }

    /// Configure and change weighted memberships.
    pub mod weighted {
        #![doc = include_str!("weighted-membership.md")]
    }

    /// Configure and change hierarchical memberships.
    pub mod hierarchical {
        #![doc = include_str!("hierarchical-membership.md")]
    }

    /// Configure and change grid memberships.
    pub mod grid {
        #![doc = include_str!("grid-membership.md")]
    }
}
