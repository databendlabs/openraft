use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;

use crate::versioned::Update;
use crate::versioned::UpdateError;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::ReplicationMetrics;

/// The metrics about the leader. It is Some() only when this node is leader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderMetrics<C: RaftTypeConfig> {
    /// Replication metrics of all known replication target: voters and learners
    pub replication: BTreeMap<C::NodeId, ReplicationMetrics<C>>,
}

impl<C: RaftTypeConfig> MessageSummary for LeaderMetrics<C> {
    fn summary(&self) -> String {
        let mut res = vec!["LeaderMetrics{".to_string()];
        for (i, (k, v)) in self.replication.iter().enumerate() {
            if i > 0 {
                res.push(", ".to_string());
            }
            res.push(format!("{}:{}", k, v.summary()));
        }

        res.push("}".to_string());
        res.join("")
    }
}

/// Update one replication metrics in `LeaderMetrics.replication`.
pub struct UpdateMatchedLogId<NID: NodeId> {
    pub target: NID,
    pub matched: LogId<NID>,
}

impl<C: RaftTypeConfig> Update<LeaderMetrics<C>> for UpdateMatchedLogId<C::NodeId> {
    /// If there is already a record for the target node. Just modify the atomic u64.
    fn apply_in_place(&self, to: &Arc<LeaderMetrics<C>>) -> Result<(), UpdateError> {
        let target_metrics = to.replication.get(&self.target).ok_or(UpdateError::CanNotUpdateInPlace)?;

        if target_metrics.matched_leader_id == self.matched.leader_id {
            target_metrics.matched_index.store(self.matched.index, Ordering::Relaxed);
            return Ok(());
        }

        Err(UpdateError::CanNotUpdateInPlace)
    }

    /// To insert a new record always work.
    fn apply_mut(&self, to: &mut LeaderMetrics<C>) {
        to.replication.insert(self.target, ReplicationMetrics {
            matched_leader_id: self.matched.leader_id,
            matched_index: AtomicU64::new(self.matched.index),
        });
    }
}

/// Remove one replication metrics in `LeaderMetrics.replication`.
pub struct RemoveTarget<NID: NodeId> {
    pub target: NID,
}

impl<C: RaftTypeConfig> Update<LeaderMetrics<C>> for RemoveTarget<C::NodeId> {
    /// Removing can not be done in place
    fn apply_in_place(&self, _to: &Arc<LeaderMetrics<C>>) -> Result<(), UpdateError> {
        Err(UpdateError::CanNotUpdateInPlace)
    }

    fn apply_mut(&self, to: &mut LeaderMetrics<C>) {
        to.replication.remove(&self.target);
    }
}
