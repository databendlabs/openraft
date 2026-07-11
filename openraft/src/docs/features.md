# OpenRaft features

OpenRaft is an asynchronous, event-driven Raft implementation for distributed
storage systems. Use this page to find each capability's primary API and
detailed guide.

| Area | Provides | Main APIs and guides |
| --- | --- | --- |
| Core integration | Processes Raft events without periodic ticks and batches messages for throughput. Applications implement `RaftLogStorage`, `RaftStateMachine`, and `RaftNetworkV2`; [`Raft`][] is the primary application API. | [`Raft`][] |
| Cluster formation and membership | Initializes a cluster, adds non-voting learners, and changes membership through joint configuration. Joint consensus supports arbitrary membership changes in one operation; single-step configuration changes are intentionally unsupported. | [`Raft::initialize()`][], [`Raft::add_learner()`][], [`Raft::change_membership()`][], [cluster formation][], [dynamic membership][] |
| Leadership and elections | Elects leaders by policy or manually, transfers leadership, and offers Pre-Vote to avoid unnecessary term increments. | [`Trigger::elect()`][], [`Trigger::transfer_leader()`][], [`Config::enable_pre_vote`][], [Pre-Vote protocol][] |
| Logs and snapshots | Compacts logs by snapshotting the state machine, replicates snapshots, and purges logs by policy or on demand. | [`Trigger::snapshot()`][], [`Trigger::purge_log()`][], [snapshot replication][] |
| Reads | Performs linearizable reads with `ReadPolicy::ReadIndex` or `ReadPolicy::LeaseRead`. | [`Raft::ensure_linearizable()`][], [read protocol][] |
| Monitoring | Exposes full, data, and server metrics through watch receivers. [`tracing`][] provides logging and distributed tracing with [compile-time verbosity controls][]. | [`Raft::metrics()`][], [`Raft::data_metrics()`][], [`Raft::server_metrics()`][], [monitoring and maintenance][] |
| Runtime controls | Enables or disables heartbeat and election behavior for testing and special operational cases. | [`RuntimeConfigHandle::heartbeat()`][], [`RuntimeConfigHandle::elect()`][] |

[`Raft`]: crate::Raft
[`Raft::initialize()`]: crate::Raft::initialize
[`Raft::add_learner()`]: crate::Raft::add_learner
[`Raft::change_membership()`]: crate::Raft::change_membership
[`Raft::ensure_linearizable()`]: crate::Raft::ensure_linearizable
[`Raft::metrics()`]: crate::Raft::metrics
[`Raft::data_metrics()`]: crate::Raft::data_metrics
[`Raft::server_metrics()`]: crate::Raft::server_metrics
[`Trigger::elect()`]: crate::raft::trigger::Trigger::elect
[`Trigger::snapshot()`]: crate::raft::trigger::Trigger::snapshot
[`Trigger::purge_log()`]: crate::raft::trigger::Trigger::purge_log
[`Trigger::transfer_leader()`]: crate::raft::trigger::Trigger::transfer_leader
[`Config::enable_pre_vote`]: crate::Config::enable_pre_vote
[`RuntimeConfigHandle::heartbeat()`]: crate::raft::RuntimeConfigHandle::heartbeat
[`RuntimeConfigHandle::elect()`]: crate::raft::RuntimeConfigHandle::elect
[cluster formation]: crate::docs::cluster_control::cluster_formation
[dynamic membership]: crate::docs::cluster_control::dynamic_membership
[Pre-Vote protocol]: crate::docs::protocol::pre_vote
[snapshot replication]: crate::docs::protocol::replication::snapshot_replication
[read protocol]: crate::docs::protocol::read
[monitoring and maintenance]: crate::docs::cluster_control::monitoring_maintenance
[`tracing`]: https://docs.rs/tracing/
[compile-time verbosity controls]: https://docs.rs/tracing/latest/tracing/level_filters/index.html
