### How to detect which nodes are currently down or unreachable?

To monitor node availability in your Raft cluster, use [`RaftMetrics`][] from
the leader node via [`Raft::metrics()`][]. This provides real-time visibility
into node reachability without requiring membership changes.

There are two primary approaches to detect unreachable nodes:

**Method 1: Monitor replication lag**
Check the field [`RaftMetrics::replication`][], which contains a
`BTreeMap<NodeId, Option<LogId>>` showing the last replicated log for each node.
If a node's replication significantly lags behind
[`RaftMetrics::last_log_index`][], it indicates replication issues and the node
may be down.

**Method 2: Monitor heartbeat timestamps (since OpenRaft 0.10)**
Use the field [`RaftMetrics::heartbeat`][], which stores `BTreeMap<NodeId, Option<SerdeInstant>>`
containing the timestamp of the last acknowledgment from each node. If a
timestamp is significantly behind the current time, the node is likely
unreachable.

Both methods provide "unreachable from leader" perspective, which is typically
what matters for cluster health monitoring. This approach allows you to maintain
a list of active nodes without modifying cluster membership.

[`RaftMetrics`]: `crate::metrics::RaftMetrics`
[`Raft::metrics()`]: `crate::Raft::metrics`
[`RaftMetrics::replication`]: `crate::metrics::RaftMetrics::replication`
[`RaftMetrics::last_log_index`]: `crate::metrics::RaftMetrics::last_log_index`
[`RaftMetrics::heartbeat`]: `crate::metrics::RaftMetrics::heartbeat`
