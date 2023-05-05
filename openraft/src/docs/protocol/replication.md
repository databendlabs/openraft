# Replication

Appending entries is indeed the primary RPC for replicating logs from the leader to followers or learners in the Raft consensus algorithm.
Installing a snapshot can be considered a special form of **appending logs** since it serves a similar purpose:
ensuring that all nodes have a consistent state, particularly when log entries become too numerous or when a node has fallen far behind.

- [Replication by Append-Entries](`crate::docs::protocol::replication::log_replication`).
- [Replication by Snapshot](`crate::docs::protocol::replication::snapshot_replication`).

