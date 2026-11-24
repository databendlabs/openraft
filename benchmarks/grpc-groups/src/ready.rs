use openraft::RaftMetrics;
use openraft::ServerState;

use crate::C;

/// Whether a member leads its group and has committed an entry of its term, the blank entry a new
/// leader appends first, after which writes to the group can commit.
pub(crate) fn is_ready_leader(metrics: &RaftMetrics<C>) -> bool {
    if metrics.state != ServerState::Leader {
        return false;
    }

    // A log id of this `TypeConfig` carries the term as its committed leader id.
    let committed_term = metrics.local_committed.as_ref().map(|log_id| *log_id.committed_leader_id());
    committed_term == Some(metrics.current_term)
}
