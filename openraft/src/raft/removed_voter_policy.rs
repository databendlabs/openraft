use openraft_macros::since;

/// What happens to a voter excluded by a membership change.
#[since(version = "0.10.1")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovedVoterPolicy {
    /// Keep the node as a learner and continue replicating logs to it.
    KeepAsLearner,
    /// Remove the former voter from membership and stop replication to it.
    Remove,
}

impl RemovedVoterPolicy {
    pub(crate) fn retain_as_learner(self) -> bool {
        self == Self::KeepAsLearner
    }
}
