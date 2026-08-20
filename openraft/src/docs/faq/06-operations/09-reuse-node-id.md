### Can I reuse the ID of a removed node for a new node?

No. A node ID must identify the same node, holding the same log, for the entire
lifetime of the cluster. Assigning the ID of a removed node to a node that
starts from an empty log is equivalent to letting that node revert its log, and
it can split the cluster into two independent quorums. Restarting a node with
its data intact is not reuse; it keeps the same ID and the same log.

The hazard is that the new node replays the log from the beginning and passes
through every **historical** membership config on its way to the current one,
adopting each as its effective membership as soon as it is appended. If a
historical config contains the reused ID with a small quorum, for example the
single-node config `{n3}` a cluster started from, the new node becomes a quorum
of one under that config and can elect itself leader while the rest of the
cluster still forms a quorum of its own.

Openraft cannot detect this, because removing a node discards the leader's
replication progress for it: the empty log of the re-added node does not look
like a [log reversion][`Config::allow_log_reversion`]. Allocate a fresh,
never-before-used ID for every node that joins the cluster. A good way to do so
is to let the cluster's own log allocate it: propose a blank log entry with
[`Raft::client_write()`] and use the index of that entry as the new node's ID.
Log indices strictly increase and no committed index is ever assigned twice, so
the ID is one that no node has ever held.

See: [Node IDs Must Not Be Reused][`dynamic_membership`] for the full scenario.

[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
[`Raft::client_write()`]: `crate::Raft::client_write`
[`dynamic_membership`]: `crate::docs::cluster_control::dynamic_membership#node-ids-must-not-be-reused`
