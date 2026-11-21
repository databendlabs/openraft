### How does Openraft handle snapshot building and transfer?

Openraft calls [`RaftStateMachine::get_snapshot_builder`][] to create snapshots. The builder runs
concurrently with [`RaftStateMachine::apply`][], so your implementation must handle concurrent access
to the state machine data.

The leader sends a follower a snapshot instead of log entries when the entries it needs are
already purged from the leader's log; see [snapshot replication][]. When snapshots are built
depends only on [`Config::snapshot_policy`][]. [`Config::replication_lag_threshold`][] does not
affect either: it sets how far behind a learner may be when a blocking [`Raft::add_learner`][]
returns.

For large snapshots that timeout during transfer, increase [`Config::install_snapshot_timeout`][].
The snapshot is sent in chunks of [`Config::snapshot_max_chunk_size`][] bytes.

[`RaftStateMachine::get_snapshot_builder`]: `crate::storage::RaftStateMachine::get_snapshot_builder`
[`RaftStateMachine::apply`]: `crate::storage::RaftStateMachine::apply`
[snapshot replication]: `crate::docs::protocol::replication::snapshot_replication`
[`Config::snapshot_policy`]: `crate::config::Config::snapshot_policy`
[`Config::replication_lag_threshold`]: `crate::config::Config::replication_lag_threshold`
[`Raft::add_learner`]: `crate::Raft::add_learner`
[`Config::install_snapshot_timeout`]: `crate::config::Config::install_snapshot_timeout`
[`Config::snapshot_max_chunk_size`]: `crate::config::Config::snapshot_max_chunk_size`
