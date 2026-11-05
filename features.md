# Features

## Membership

- ✅ **Generalized membership change**: [change any set of nodes](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/data/extended_membership/index.html) in one operation, using [one log entry when proven safe or joint consensus otherwise](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/cluster_control/dynamic_membership/index.html).
- ✅ **Single-step config change**: [`append_membership()`][] writes a caller-built membership without a joint config, for a restricted set of transitions.
- ✅ **Conditional membership changes**: [`change_membership_if()`][] checks the expected leader or membership before proposing a change.
- ✅ **Flexible quorums**: choose [threshold, weighted, hierarchical, grid, explicit](./membership/src/docs.md), or custom rules in `RaftTypeConfig`. Configure separate read (election) and write (commit) quorums and which voters can start elections.
- ✅ **Learners (non-voters)**: [`add_learner()`][].

## Leadership

- ✅ **Fewer election conflicts**: a split vote does not force a new term with OpenRaft's [Vote](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/data/vote/index.html). The [standard Raft leader-ID mode](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/data/leader_id/index.html) is also supported.
- ✅ **[Pre-vote][Pre-Vote protocol]**: a dedicated RPC avoids unnecessary term increments; enable it with [`Config::enable_pre_vote`][].
- ✅ **[CheckQuorum](./openraft/src/docs/protocol/check_quorum.md)**: reject new proposals when the quorum lease expires; resume after renewed quorum acknowledgements.
- ✅ **Leadership callbacks**: start and stop application services on leadership changes with [`on_leader_change()`][].

## Replication

- ✅ **Snapshot replication**: catch up lagging nodes using snapshots.
- ✅ **Streaming replication**: pipeline log replication with custom transports via [`RaftNetworkV2::stream_append()`][].

## Reads and writes

- ✅ **Batch writes**: [`client_write_many()`][] batches requests and returns results in submission order.
- ✅ **Conditional writes**: check the expected leader before appending a write with [`write().with_leader()`][].
- ✅ **Write notifications**: [`ProgressResponder`][] reports commit and state-machine application separately.
- ✅ **Linearizable reads**: confirm leadership with a quorum and wait for local apply before reading, using [`ensure_linearizable()`][] with `ReadPolicy::ReadIndex`.
- ✅ **Lease reads**: avoid per-read quorum round trips under the [lease assumptions](./openraft/src/docs/protocol/read.md#ensuring-linearizability); optional [local checks](./openraft/src/docs/protocol/read.md#caller-side-lease-reads) bypass RaftCore's queue.
- ✅ **[Follower reads](./openraft/src/docs/protocol/read.md#follower-read)**: fetch a read boundary from the leader via application RPC, then wait for local apply to read linearizably.

## Runtime

- ✅ **Async and event-driven**: event-driven state transitions and batched messages reach millions of writes/sec in the [framework benchmark](./README.md#performance).
- ✅ **Fully pluggable**: custom storage, networking, and snapshot transport; configurable node IDs, nodes, terms, votes, and log entries via `RaftTypeConfig`.
- ✅ **Runtime-agnostic**: Tokio by default, [compio](./rt-compio) or [monoio](./rt-monoio) via `AsyncRuntime`; `single-threaded` mode removes `Send` bounds.

## Cluster control

- ✅ **Leader election**: by policy or manually ([`Trigger::elect()`][]).
- ✅ **Leader transfer**: [`Trigger::transfer_leader()`][].
- ✅ **Log compaction**: snapshot the state machine by policy or manually ([`Trigger::snapshot()`][]).
- ✅ **Immediate heartbeats**: [`Trigger::heartbeat()`][].
- ✅ **Reset replication backoff**: end the active retry delay for selected or all replication targets via [`Trigger::reset_backoff()`][].
- ✅ **Toggle heartbeat / election**: [`RuntimeConfigHandle::heartbeat()`][] / [`RuntimeConfigHandle::elect()`][].
- ✅ **Log purging**: by policy or manually ([`Trigger::purge_log()`][]).

## Testing and monitoring

- ✅ **Deterministically tested**: a [Turmoil simulation fuzzer](./tests-turmoil) checks invariants from the Raft paper and TLA+ spec after each tick.
- ✅ **Jepsen tested**: [black-box tests](./jepsen) check linearizability and recovery under network faults, process failures, clock skew, and membership changes.
- ✅ **Storage test suite**: [`testing::log::Suite`][] tests custom storage implementations.
- ✅ **Observable**: streaming metrics via watch channels ([`Raft::metrics()`][], [`Raft::data_metrics()`][], and [`Raft::server_metrics()`][]), and [tracing](https://docs.rs/tracing/) with [compile-time verbosity filtering](https://docs.rs/tracing/latest/tracing/level_filters/index.html).

[`append_membership()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.append_membership
[`add_learner()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.add_learner
[`Trigger::purge_log()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.purge_log

[`RuntimeConfigHandle::heartbeat()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.RuntimeConfigHandle.html#method.heartbeat
[`RuntimeConfigHandle::elect()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.RuntimeConfigHandle.html#method.elect

[`Trigger::elect()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.elect
[`Trigger::snapshot()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.snapshot
[`Trigger::transfer_leader()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.transfer_leader
[`Config::enable_pre_vote`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/struct.Config.html#structfield.enable_pre_vote
[Pre-Vote protocol]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/protocol/pre_vote/index.html
[`Raft::metrics()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.metrics
[`Raft::data_metrics()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.data_metrics
[`Raft::server_metrics()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.server_metrics
[`ensure_linearizable()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.ensure_linearizable
[`change_membership_if()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.change_membership_if
[`client_write_many()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.client_write_many
[`RaftNetworkV2::stream_append()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/network/v2/trait.RaftNetworkV2.html#method.stream_append
[`Trigger::heartbeat()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.heartbeat
[`Trigger::reset_backoff()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/trigger/struct.Trigger.html#method.reset_backoff
[`testing::log::Suite`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/testing/log/struct.Suite.html
[`on_leader_change()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.Raft.html#method.on_leader_change
[`write().with_leader()`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/raft/struct.WriteRequest.html#method.with_leader
[`ProgressResponder`]: https://docs.rs/openraft/0.10.0-alpha.37/openraft/impls/struct.ProgressResponder.html
