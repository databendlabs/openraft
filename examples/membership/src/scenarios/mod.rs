//! One scenario per membership, each with its own type config.
//!
//! Every scenario runs the same phases on a [`Cluster`](crate::cluster::Cluster):
//!
//! 1. Log the chosen types, the initial layout and the read and write quorum rules.
//! 2. Initialize node 1, and wait until it leads and has applied its blank entry.
//! 3. Write a value, and check the response and the value on every member.
//! 4. Change the membership with the native edit type, and wait until every member applies the
//!    resulting uniform membership.
//! 5. Write again, and wait until every member applies it.
//! 6. Isolate some nodes, check the available nodes against the read and write quorum rules, and
//!    check whether a write commits.
//! 7. Heal the isolation, and check that every member applies every write.
//! 8. Shut down every node.

pub mod explicit;
pub mod grid;
pub mod hierarchical;
pub mod majority;
pub mod threshold;
pub mod weighted;

use std::fmt;

use crate::cluster::SCENARIO_TIMEOUT;

/// A membership and its edit type, as named on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Policy {
    /// `Membership` and `ChangeMembers` from `openraft::impls`: majority read and write quorums.
    Majority,
    /// `ThresholdMembership` and `ThresholdChange`: any `write_size` voters commit.
    Threshold,
    /// `WeightedMembership` and `WeightedChange`: voters whose weights reach a threshold.
    Weighted,
    /// `HierarchicalMembership` and `HierarchicalChange`: enough voters in enough groups.
    Hierarchical,
    /// `GridMembership` and `GridChange`: a full row elects, a voter of each row commits.
    Grid,
    /// `ExplicitMembership` and `ExplicitChange`: listed read and write quorums.
    Explicit,
}

impl Policy {
    /// Returns the name that selects this policy on the command line.
    pub fn name(&self) -> &'static str {
        match self {
            Policy::Majority => "majority",
            Policy::Threshold => "threshold",
            Policy::Weighted => "weighted",
            Policy::Hierarchical => "hierarchical",
            Policy::Grid => "grid",
            Policy::Explicit => "explicit",
        }
    }
}

impl fmt::Display for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Runs the scenario of `policy`, and returns an error at the first check that fails.
///
/// The scenario must finish within [`SCENARIO_TIMEOUT`].
pub async fn run(policy: Policy) -> anyhow::Result<()> {
    let scenario = async {
        match policy {
            Policy::Majority => majority::run().await,
            Policy::Threshold => threshold::run().await,
            Policy::Weighted => weighted::run().await,
            Policy::Hierarchical => hierarchical::run().await,
            Policy::Grid => grid::run().await,
            Policy::Explicit => explicit::run().await,
        }
    };

    match tokio::time::timeout(SCENARIO_TIMEOUT, scenario).await {
        Ok(res) => res,
        Err(_elapsed) => anyhow::bail!("the {} scenario did not finish within {:?}", policy, SCENARIO_TIMEOUT),
    }
}
