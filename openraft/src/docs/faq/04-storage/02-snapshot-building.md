### How does Openraft handle snapshot building and transfer?

Openraft calls [`RaftStateMachine::get_snapshot_builder`][] to create snapshots. The builder runs
concurrently with [`RaftStateMachine::apply`][], so your implementation must handle concurrent access
to the state machine data.

When a follower is more than [`Config::replication_lag_threshold`][] entries behind, the leader
sends a snapshot instead of individual log entries.

For large snapshots that timeout during transfer, increase [`Config::install_snapshot_timeout`][].
The snapshot is sent in chunks of [`Config::snapshot_max_chunk_size`][] bytes.

[`RaftStateMachine::get_snapshot_builder`]: `crate::storage::RaftStateMachine::get_snapshot_builder`
[`RaftStateMachine::apply`]: `crate::storage::RaftStateMachine::apply`
[`Config::replication_lag_threshold`]: `crate::config::Config::replication_lag_threshold`
[`Config::install_snapshot_timeout`]: `crate::config::Config::install_snapshot_timeout`
[`Config::snapshot_max_chunk_size`]: `crate::config::Config::snapshot_max_chunk_size`
