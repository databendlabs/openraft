### Write returns `ForwardToLeader` but leader info is missing

**Symptom**: [`ClientWriteError::ForwardToLeader`][] is returned but the `leader_id` field is `None`

**Cause**: The current leader is no longer in the cluster's membership configuration.
This occurs after a membership change that removes the leader node.

**Solution**: If `leader_id` is `None`, query [`RaftMetrics::current_leader`][] to find the new
leader, or retry the membership query after the new leader is elected.

[`ClientWriteError::ForwardToLeader`]: `crate::error::ClientWriteError::ForwardToLeader`
[`RaftMetrics::current_leader`]: `crate::metrics::RaftMetrics::current_leader`
