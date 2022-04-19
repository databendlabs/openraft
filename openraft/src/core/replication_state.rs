use crate::config::Config;
use crate::error::AddLearnerError;
use crate::raft::AddLearnerResponse;
use crate::raft::RaftRespTx;
use crate::raft_types::LogIdOptionExt;
use crate::replication::ReplicationStream;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftTypeConfig;

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
pub(crate) struct ReplicationState<C: RaftTypeConfig> {
    pub matched: Option<LogId<C::NodeId>>,
    pub remove_since: Option<u64>,
    pub repl_stream: ReplicationStream<C>,

    /// The response channel to use for when this node has successfully synced with the cluster.
    #[allow(clippy::type_complexity)]
    pub tx: Option<RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>>,
}

impl<C: RaftTypeConfig> MessageSummary for ReplicationState<C> {
    fn summary(&self) -> String {
        format!(
            "matched: {:?}, remove_after_commit: {:?}",
            self.matched, self.remove_since
        )
    }
}

impl<C: RaftTypeConfig> ReplicationState<C> {
    // TODO(xp): make this a method of Config?

    /// Return true if the distance behind last_log_id is smaller than the threshold to join.
    pub fn is_line_rate(&self, last_log_id: &Option<LogId<C::NodeId>>, config: &Config) -> bool {
        is_matched_upto_date::<C>(&self.matched, last_log_id, config)
    }
}

pub fn is_matched_upto_date<C: RaftTypeConfig>(
    matched: &Option<LogId<C::NodeId>>,
    last_log_id: &Option<LogId<C::NodeId>>,
    config: &Config,
) -> bool {
    let my_index = matched.next_index();
    let distance = last_log_id.next_index().saturating_sub(my_index);
    distance <= config.replication_lag_threshold
}
