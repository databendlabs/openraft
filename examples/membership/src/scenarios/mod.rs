//! One scenario per membership, each with its own type config.

use std::fmt;

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

/// Runs the scenario of `policy` and returns an error at the first check that fails.
pub async fn run(policy: Policy) -> anyhow::Result<()> {
    anyhow::bail!("the {} scenario is not implemented yet", policy)
}
