# Features

OpenRaft is an asynchronous, event-driven Raft implementation for distributed
storage systems. Use this page to find each capability's primary API and
detailed guide.

## Membership

- **Cluster formation**: initialize a cluster with [`Raft::initialize()`][]; see the [cluster formation guide][cluster formation].
- **Generalized membership change**: [change any set of nodes](crate::docs::data::extended_membership) in one operation with [`change_membership()`][], using [one log entry when proven safe or joint consensus otherwise][dynamic membership].
- **Single-step config change**: [`append_membership()`][] writes a caller-built membership in one log entry without a joint config, for transitions whose quorum intersection OpenRaft can prove.
- **Conditional membership changes**: [`change_membership_if()`][] checks the expected leader or membership before proposing a change.
- **Flexible quorums**: choose [threshold, weighted, hierarchical, grid, explicit](https://github.com/databendlabs/openraft/blob/main/membership/src/docs.md) from [`openraft-membership`][], or custom rules in `RaftTypeConfig`. Configure separate [read (election) and write (commit) quorums][election quorums] and which voters can start elections. The [weighted-membership example](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore-weighted-membership) runs a `WeightedMembership` in the HTTP key-value store.
- **Membership configuration**: the default [`Membership`][] uses majorities. Select membership and editing implementations through [`RaftTypeConfig::Membership`][] and [`RaftTypeConfig::MembershipEdit`][].
- **Learners (non-voters)**: [`add_learner()`][].

## Leadership

- **Fewer election conflicts**: a split vote does not force a new term with OpenRaft's [Vote](crate::docs::data::vote). The [standard Raft leader-ID mode](crate::docs::data::leader_id) is also supported.
- **[Pre-vote][Pre-Vote protocol]**: a dedicated RPC avoids unnecessary term increments; enable it with [`Config::enable_pre_vote`][].
- **[CheckQuorum](crate::docs::protocol::check_quorum)**: reject new proposals when the quorum lease expires; resume after renewed quorum acknowledgements.
- **Leadership callbacks**: start and stop application services on leadership changes with [`on_leader_change()`][].

## Replication

- **Snapshot replication**: catch up lagging nodes using [snapshots][snapshot replication].
- **Streaming replication**: pipeline log replication with custom transports via [`RaftNetworkV2::stream_append()`][].

## Reads and writes

- **Batch writes**: [`client_write_many()`][] batches requests and returns results in submission order.
- **Conditional writes**: check the expected leader before appending a write with [`write().with_leader()`][].
- **Write notifications**: [`ProgressResponder`][] reports commit and state-machine application separately.
- **[Linearizable reads][read protocol]**: confirm leadership with a quorum and wait for local apply before reading, using [`ensure_linearizable()`][] with `ReadPolicy::ReadIndex`.
- **Lease reads**: avoid per-read quorum round trips with `ReadPolicy::LeaseRead` under the [lease assumptions](crate::docs::protocol::read#ensuring-linearizability); optional [local checks](crate::docs::protocol::read#caller-side-lease-reads) bypass RaftCore's queue.
- **[Follower reads](crate::docs::protocol::read#follower-read)**: fetch a read boundary from the leader via application RPC, then wait for local apply to read linearizably.

## Runtime

- **Async and event-driven**: event-driven state transitions and batched messages reach millions of writes/sec in the [framework benchmark](https://github.com/databendlabs/openraft/blob/main/README.md#performance).
- **Fully pluggable**: custom storage, networking, and snapshot transport; configurable node IDs, nodes, terms, votes, and log entries via `RaftTypeConfig`.
- **Application integration**: [`Raft`][] is the primary application API; implement [`RaftLogStorage`][], [`RaftStateMachine`][], and [`RaftNetworkV2`][] for storage and networking.
- **Runtime-agnostic**: Tokio by default, [compio](https://github.com/databendlabs/openraft/tree/main/rt-compio) or [monoio](https://github.com/databendlabs/openraft/tree/main/rt-monoio) via `AsyncRuntime`; `single-threaded` mode removes `Send` bounds.

## Cluster control

- **Leader election**: by policy or manually ([`Trigger::elect()`][]).
- **Leader transfer**: [`Trigger::transfer_leader()`][].
- **Log compaction**: snapshot the state machine by policy or manually ([`Trigger::snapshot()`][]).
- **Immediate heartbeats**: [`Trigger::heartbeat()`][].
- **Reset replication backoff**: end the active retry delay for selected or all replication targets via [`Trigger::reset_backoff()`][].
- **Toggle heartbeat / election**: enable or disable heartbeat and election at runtime for testing and operational control ([`RuntimeConfigHandle::heartbeat()`][] / [`RuntimeConfigHandle::elect()`][]).
- **Log purging**: by policy or manually ([`Trigger::purge_log()`][]).

## Testing and monitoring

- **Deterministically tested**: a [Turmoil simulation fuzzer](https://github.com/databendlabs/openraft/tree/main/tests-turmoil) checks invariants from the Raft paper and TLA+ spec after each tick.
- **Jepsen tested**: [black-box tests](https://github.com/databendlabs/openraft/tree/main/jepsen) check linearizability and recovery under network faults, process failures, clock skew, and membership changes.
- **Storage test suite**: [`testing::log::Suite`][] tests custom storage implementations.
- **Observable**: streaming [full, data, and server metrics][monitoring and maintenance] via watch channels ([`Raft::metrics()`][], [`Raft::data_metrics()`][], and [`Raft::server_metrics()`][]), and [logging and distributed tracing](https://docs.rs/tracing/) with [compile-time verbosity filtering](https://docs.rs/tracing/latest/tracing/level_filters/index.html).

[`append_membership()`]: crate::Raft::append_membership
[`add_learner()`]: crate::Raft::add_learner
[`Trigger::purge_log()`]: crate::raft::trigger::Trigger::purge_log

[`RuntimeConfigHandle::heartbeat()`]: crate::raft::RuntimeConfigHandle::heartbeat
[`RuntimeConfigHandle::elect()`]: crate::raft::RuntimeConfigHandle::elect

[`Trigger::elect()`]: crate::raft::trigger::Trigger::elect
[`Trigger::snapshot()`]: crate::raft::trigger::Trigger::snapshot
[`Trigger::transfer_leader()`]: crate::raft::trigger::Trigger::transfer_leader
[`Config::enable_pre_vote`]: crate::Config::enable_pre_vote
[Pre-Vote protocol]: crate::docs::protocol::pre_vote
[`Raft::metrics()`]: crate::Raft::metrics
[`Raft::data_metrics()`]: crate::Raft::data_metrics
[`Raft::server_metrics()`]: crate::Raft::server_metrics
[`ensure_linearizable()`]: crate::Raft::ensure_linearizable
[`change_membership_if()`]: crate::Raft::change_membership_if
[`client_write_many()`]: crate::Raft::client_write_many
[`RaftNetworkV2::stream_append()`]: crate::RaftNetworkV2::stream_append
[`Trigger::heartbeat()`]: crate::raft::trigger::Trigger::heartbeat
[`Trigger::reset_backoff()`]: crate::raft::trigger::Trigger::reset_backoff
[`testing::log::Suite`]: crate::testing::log::Suite
[`on_leader_change()`]: crate::Raft::on_leader_change
[`write().with_leader()`]: crate::raft::WriteRequest::with_leader
[`ProgressResponder`]: crate::impls::ProgressResponder
[`Raft`]: crate::Raft
[`Raft::initialize()`]: crate::Raft::initialize
[`change_membership()`]: crate::Raft::change_membership
[`Membership`]: crate::Membership
[`RaftTypeConfig::Membership`]: crate::RaftTypeConfig::Membership
[`RaftTypeConfig::MembershipEdit`]: crate::RaftTypeConfig::MembershipEdit
[`openraft-membership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/index.html
[`RaftLogStorage`]: crate::storage::RaftLogStorage
[`RaftStateMachine`]: crate::storage::RaftStateMachine
[`RaftNetworkV2`]: crate::RaftNetworkV2
[cluster formation]: crate::docs::cluster_control::cluster_formation
[dynamic membership]: crate::docs::cluster_control::dynamic_membership
[election quorums]: crate::docs::protocol::election_quorum
[snapshot replication]: crate::docs::protocol::replication::snapshot_replication
[read protocol]: crate::docs::protocol::read
[monitoring and maintenance]: crate::docs::cluster_control::monitoring_maintenance
