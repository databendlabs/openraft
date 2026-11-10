# Guide for upgrading from [v0.9](https://github.com/databendlabs/openraft/tree/release-0.9) to v0.10:

This guide lists every breaking change from 0.9 and what an application
changes for each one. The
[change log](https://github.com/databendlabs/openraft/blob/release-0.10/change-log/v0.10.0.md)
describes the new features and links each change to its commits.

An application that uses [`declare_raft_types!`][], the default types and the
Tokio runtime typically needs only these steps:

1. Build with Rust 1.88 or later and tokio 1.39 or later; see
   [Toolchain and crates](#toolchain-and-crates).
2. Remove the feature flags that 0.10 dropped; see
   [Feature flags](#feature-flags).
3. Move `SnapshotData` from the type config to the state machine, and
   implement `Debug` and `Display` for the application data; see
   [Type config](#type-config).
4. Update the log store and the state machine; see
   [Log storage and state machine](#log-storage-and-state-machine).
5. Implement [`RaftNetworkV2`][], or wrap a v1 network with `openraft-legacy`;
   see [Network](#network).
6. Update the calls into [`Raft`][]; see [Raft API](#raft-api).
7. Build [`Config`][] with `..Default::default()`; see [Config](#config).

The other sections matter to code that names Openraft types generically,
implements its own entry, membership, responder or runtime types, or matches
on errors.

## Toolchain and crates

- Openraft 0.10 uses the Rust 2024 edition and needs Rust 1.88 or later,
  which its `validit` dependency requires.
- `openraft-rt-tokio`, the default runtime, needs tokio 1.39 or later.
- `openraft-rocksstore` and `openraft-sledstore` are not released for 0.10;
  their last version is 0.9.21. The RocksDB store is now the `log-rocks` and
  `sm-rocks` example crates in the repository, which an application can copy.
  The sled store is removed.

Parts of Openraft moved into separate crates. An application adds the ones it
uses:

| Crate | Provides |
|---|---|
| `openraft-rt` | The `AsyncRuntime` traits, re-exported as `openraft::rt` |
| `openraft-rt-tokio` | The Tokio runtime, enabled by the default `tokio-rt` feature |
| `openraft-rt-compio`, `openraft-rt-monoio` | Runtimes for compio and monoio |
| `openraft-rt-sim` | A simulated runtime for deterministic tests |
| `openraft-legacy` | The v1 `RaftNetwork` and chunked snapshot transfer |
| `openraft-membership` | Memberships with quorum rules other than a majority |
| `openraft-multi` | Multi-Raft adapters that share connections across Raft groups |
| `openraft-metrics-otel` | Raft metrics exported through OpenTelemetry |

## Feature flags

| 0.9 flag | In 0.10 | Migration |
|---|---|---|
| `storage-v2` | removed | Remove it: the v2 storage API is the only one. |
| `generic-snapshot-data` | removed | Remove it: snapshot data no longer needs tokio's I/O traits. |
| `single-term-leader` | compile error | Select the standard leader id, as shown below. |
| `loosen-follower-log-revert` | compile error | Set [`Config::allow_log_reversion`][] to `Some(true)`. |
| `singlethreaded` | compile error | Enable [`single-threaded`][] instead. |

A flag marked "compile error" still exists, and enabling it fails the build
with a message that names the replacement. `adapt-network-v1`, which only the
0.10 alphas had, behaves the same; use [`openraft-legacy`][] instead.

To keep the standard Raft election that `single-term-leader` enabled, set
`LeaderId` to [`leader_id_std::LeaderId`][], in `declare_raft_types!` or in a
[`RaftTypeConfig`][] impl:

```ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
);
```

The new default features are `tokio-rt`, which enables the Tokio runtime, and
`clap`, which builds `Config` from command-line arguments. Without default
features, the application provides an `AsyncRuntime` and builds `Config` in
code.

## Type config

A type config declared with [`declare_raft_types!`][] gets a default for each
new associated type, and needs two changes:

- Remove `SnapshotData`: the state machine declares it now; see
  [Snapshots](#snapshots).
- The application data type `D` implements `Debug` and `Display`.

A manual [`RaftTypeConfig`][] impl also removes `SnapshotData` and adds the
new types. These are the defaults that `declare_raft_types!` uses:

```ignore
impl openraft::RaftTypeConfig for TypeConfig {
    type D = String;
    type R = String;
    type NodeId = u64;
    type Node = openraft::impls::BasicNode;
    type Term = u64;
    type LeaderId = openraft::impls::leader_id_adv::LeaderId<Self::Term, Self::NodeId>;
    type Vote = openraft::impls::Vote<Self::LeaderId>;
    type Membership = openraft::impls::Membership<Self::NodeId, Self::Node>;
    type MembershipEdit = openraft::impls::ChangeMembers<Self::NodeId, Self::Node>;
    type Payload = openraft::impls::EntryPayload<Self::D, Self::Membership>;
    type Entry = openraft::impls::Entry<
        <Self::LeaderId as openraft::vote::RaftLeaderId>::Committed,
        Self::Payload,
    >;
    type Responder<T> = openraft::impls::ProgressResponder<Self, T>
    where T: openraft::OptionalSend + 'static;
    type Batch<T> = openraft::impls::InlineBatch<T>
    where T: openraft::OptionalSend + 'static;
    type AsyncRuntime = openraft_rt_tokio::TokioRuntime;
    type ErrorSource = openraft::impls::BoxedErrorSource;
}
```

The new associated types select:

- `Term`, `LeaderId` and `Vote`. The order of `LeaderId` decides whether a
  term can have several leaders, as with the default `leader_id_adv::LeaderId`,
  or only one, as with `leader_id_std::LeaderId`.
- `Membership` and `MembershipEdit`: the membership and the change that
  `change_membership()` takes; see [Membership](#membership).
- `Payload`: the payload of a log entry, which `Entry` stores.
- `Responder<T>`: what receives the result of a client write; see
  [Raft API](#raft-api).
- `Batch<T>`: the container for items that RaftCore handles together.
- `ErrorSource`: the type that holds the source of an Openraft error; see
  [Errors](#errors).

## Type parameters

Metrics, state, RPC and error types take a single `C: RaftTypeConfig`:

```text
RaftMetrics<NID, N>          --> RaftMetrics<C>
RaftDataMetrics<NID>         --> RaftDataMetrics<C>
RaftServerMetrics<NID, N>    --> RaftServerMetrics<C>
Metric<NID>                  --> Metric<C>
Wait<NID, N, A>              --> Wait<C>
RaftState<NID, N, I>         --> RaftState<C>
VoteRequest<NID>             --> VoteRequest<C>
VoteResponse<NID>            --> VoteResponse<C>
AppendEntriesResponse<NID>   --> AppendEntriesResponse<C>
SnapshotResponse<NID>        --> SnapshotResponse<C>
InitializeError<NID, N>      --> InitializeError<C>
NotInMembers<NID, N>         --> NotInMembers<C>
ClientWriteError<NID, N>     --> ClientWriteError<C>
CheckIsLeaderError<NID, N>   --> CheckIsLeaderError<C>
RaftError<NID, E>            --> RaftError<C, E>
RPCError<NID, N, E>          --> RPCError<C, E>
RemoteError<NID, N, T>       --> RemoteError<C, T>
ForwardToLeader<NID, N>      --> ForwardToLeader<C>
NotAllowed<NID>              --> NotAllowed<C>
QuorumNotEnough<NID>         --> QuorumNotEnough<C>
Timeout<NID>                 --> Timeout<C>
Fatal<NID>                   --> Fatal<C>
StorageError<NID>            --> StorageError<C>
ErrorSubject<NID>            --> ErrorSubject<C>
NetworkError                 --> NetworkError<C>
Unreachable                  --> Unreachable<C>
StreamingError<C, E>         --> StreamingError<C>
```

Log id, vote, membership, snapshot and entry types take only the parameters
they store, so they can be named without a full type config. The aliases in
`openraft::type_config::alias`, also exported as `openraft::alias` with the
`type-alias` feature, name them from a type config:

```text
LogId<NID>                  --> LogId<CLID>                    // LogIdOf<C>
Vote<NID>                   --> Vote<LID>                      // VoteOf<C>
LeaderId<NID>               --> LeaderId<Term, NID>            // LeaderIdOf<C>
CommittedLeaderId<NID>      --> CommittedLeaderId<Term>        // leader_id_std
                                CommittedLeaderId<Term, NID>   // leader_id_adv
LogIdList<NID>              --> LogIdList<CLID>                // LogIdListOf<C>
InProgress<NID>             --> InProgress<CLID>               // InProgressOf<C>
ChangeMembershipError<NID>  --> ChangeMembershipError<CLID, NID, E>
StoredMembership<NID, N>    --> StoredMembership<CLID, Mem>    // StoredMembershipOf<C>
MembershipState<NID, N>     --> MembershipState<CLID, Mem>     // MembershipStateOf<C>
EffectiveMembership<NID, N> --> StoredMembership<CLID, Mem>
SnapshotMeta<NID, N>        --> SnapshotMeta<CLID, Mem>        // SnapshotMetaOf<C>
SnapshotSignature<NID>      --> SnapshotSignature<CLID>        // SnapshotSignatureOf<C>
Snapshot<C>                 --> Snapshot<CLID, Mem, SD>        // SnapshotOf<C, SD>
Entry<C>                    --> Entry<CLID, P>                 // DefaultEntryOf<C>
EntryPayload<C>             --> EntryPayload<D, Mem>           // EntryPayloadOf<C>
```

`Membership<NID, N>`, `ChangeMembers<NID, N>` and `LearnerNotFound<NID>` are
unchanged. `RaftEntry` and `RaftPayload` no longer take type parameters, and
`EntryOf<C>` names the configured `C::Entry`.

## Paths and trait impls

- The `error` module is renamed to `errors`; `openraft::error` remains as a
  deprecated alias.
- `openraft::LeaderId` and `openraft::CommittedLeaderId` are no longer
  re-exported at the crate root. Use `openraft::impls::leader_id_adv::LeaderId`,
  `openraft::vote::leader_id_adv::CommittedLeaderId`, or the `LeaderIdOf<C>`
  and `CommittedLeaderIdOf<C>` aliases.
- `RaftLogId` is no longer re-exported at the crate root: import
  `openraft::log_id::RaftLogId`.
- `openraft::async_runtime::TokioRuntime` is removed: add the
  `openraft-rt-tokio` crate and use `openraft_rt_tokio::TokioRuntime`.
- `RaftEntry` no longer extends `RaftLogId`, and `Entry` implements neither
  `RaftLogId` nor `RaftPayload`. Replace `entry.get_log_id()` with
  `entry.log_id()`, which returns the `LogId` by value; the `Entry::log_id`
  field is unchanged.
- `LogId`, `Vote`, `Entry` and `RaftServerMetrics` no longer implement
  `Default`. Build them explicitly, as in `LogId::new(leader_id, index)`, or
  store an `Option`.
- `MessageSummary` is replaced by `Display`: replace `t.summary()` with
  `t.to_string()`. `summary()` remains only for `Option<T>` and `&[T]`.

## Log storage and state machine

`RaftStorage` and its `Adaptor` are removed: implement [`RaftLogStorage`][]
and [`RaftStateMachine`][]. Storage methods return `io::Error` instead of
`StorageError`, and Openraft adds the error context. Where a `StorageError` is
still needed, `?` converts an `io::Error` into one.

### Log store

- `read_vote()` moved from `RaftLogStorage` to [`RaftLogReader`][], and
  `RaftLogStorage` no longer extends `RaftLogReader`. Openraft reads the vote
  and the logs through the reader that `get_log_reader()` returns. A store
  with `type LogReader = Self` keeps working; generic code that read logs from
  a `RaftLogStorage` calls `get_log_reader()` first.
- `truncate(log_id)`, which deleted the logs from `log_id.index()` on, is
  replaced by `truncate_after(last_log_id)`, which keeps the logs up to and
  including `last_log_id`:

  ```ignore
  async fn truncate_after(&mut self, last_log_id: Option<LogId>) -> Result<(), io::Error> {
      let start = last_log_id.map(|id| id.index() + 1).unwrap_or(0);
      delete_from(start..);
      Ok(())
  }
  ```

- The append callback `LogFlushed` is renamed to `IOFlushed`, and
  `log_io_completed()` to `io_completed()`; the old names remain as
  deprecated aliases.
- `RaftLogReaderExt::get_log_entries()` is removed: use
  `RaftLogReader::try_get_log_entries()`. `LogIndexOptionExt::add()` is
  removed.
- A bound `RaftLogId<C>` becomes
  `RaftLogId<CommittedLeaderId = CommittedLeaderIdOf<C>>`, and
  `LogIdOptionExt<C>` becomes `LogIdOptionExt`.
- `RaftLogReader` gains `entries_stream()`, through which the state machine
  worker reads the entries to apply, and `leader_bounded_stream()`. Their
  default implementations call `try_get_log_entries()`; a store can override
  them to stream entries instead of reading a whole range at once.

### State machine

- `apply()` takes a stream, and sends each response through the responder
  paired with its entry, so a client gets its answer as soon as its entry is
  applied:

  ```ignore
  async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
  where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
      while let Some((entry, responder)) = entries.try_next().await? {
          let response = /* process entry */;
          if let Some(responder) = responder {
              responder.send(response);
          }
      }
      Ok(())
  }
  ```

- `try_create_snapshot_builder()` lets a state machine defer a snapshot that
  Openraft requests. `get_snapshot_builder()` is still required; it will be
  replaced by `try_create_snapshot_builder()`.

### Custom entry and payload types

- A custom entry type implements `RaftEntry` with `type CommittedLeaderId` and
  `type Payload`, and `new()`, `log_id_parts()`, `set_log_id()` and
  `get_membership()`. The `FromAppData` trait is removed.
- A custom payload type implements `RaftPayload` with `type D` and
  `type Membership`, and `blank()`, `with_normal()`, `with_membership()` and
  `get_membership()`, which returns an owned membership. `is_blank()` is
  removed.

### Snapshots

- `SnapshotData` moved from `RaftTypeConfig` to the traits that use it. A
  [`RaftStateMachine`][], its [`RaftSnapshotBuilder`][] and a
  [`RaftNetworkV2`][] that sends snapshots each declare `type SnapshotData`,
  and the three must agree:

  ```ignore
  impl RaftStateMachine<TypeConfig> for StateMachine {
      type SnapshotData = Cursor<Vec<u8>>;
      // ...
  }
  ```

- `Snapshot::snapshot` holds the data without a `Box`, and
  `install_snapshot()` receives it by value.
- [`SnapshotMeta`][] has no `snapshot_id`. A snapshot is identified by the
  position it covers: two snapshots at the same `last_log_id` represent the
  same state, even when their bytes differ. The 0.9 `snapshot_id` identified
  a transfer, so it remains only in the chunked v1 protocol; see
  [Network](#network). A state machine builds the metadata without inventing
  an id:

  ```ignore
  let meta = SnapshotMeta {
      last_log_id,
      last_membership,
  };
  ```

- [`SnapshotSignature`][] has no `snapshot_id` either, and its
  `last_membership_log_id` is an `Option<Box<LogId>>`.
- `RaftStateMachine::begin_receiving_snapshot()` moved to
  `openraft_legacy::network_v1::SnapshotReceiverFactory`; only the chunked v1
  transfer needs it.

### Startup checks

At startup, Openraft returns a `StorageError` for an inconsistent store
instead of failing later in replication:

- a log store purged past its last log;
- a state machine applied past the log tail at a smaller index;
- a state machine that misses purged logs;
- a log reader that returns other indexes than requested while Openraft
  re-applies committed logs, which it reads in chunks of 64 entries.

Wipe such a node and add it back to the cluster.

### Test suite

- The storage test suite moved to [`openraft::testing::log::Suite`][] and
  `openraft::testing::log::StoreBuilder`, and `Suite::test_all()` is async.
  Run it inside the runtime:

  ```ignore
  use openraft::testing::log::Suite;
  use openraft::type_config::TypeConfigExt;

  #[test]
  fn test_store() {
      TypeConfig::run(async {
          Suite::test_all(MyStoreBuilder {}).await.unwrap();
      });
  }
  ```

- `openraft::testing::log_id()` takes the type config instead of the node id
  type as its type parameter.
- New cases check `entries_stream()` and `leader_bounded_stream()`, that a log
  reader sees entries appended after it was created, and the startup checks
  above.

## Network

### Implement RaftNetworkV2, or keep a v1 network

The core crate provides only [`RaftNetworkV2`][], which sends a snapshot in
one piece through `full_snapshot()`. The connection that
[`RaftNetworkFactory`][] creates implements it.

To keep a v1 `RaftNetwork`, depend on [`openraft-legacy`][] and wrap the
network in its `Adapter`, which implements `RaftNetworkV2`:

```ignore
use openraft_legacy::network_v1::{Adapter, RaftNetwork};

impl RaftNetworkFactory<TypeConfig> for MyFactory {
    type Network = Adapter<TypeConfig, MyNetwork, SnapshotData>;

    async fn new_client(&mut self, target: NodeId, node: &Node) -> Self::Network {
        MyNetwork::new(target, node).into_v2()
    }
}
```

`Adapter` sends a snapshot in chunks through `install_snapshot()`, so the v1
`RaftNetwork` has no `full_snapshot()`, and the snapshot data must implement
tokio's `AsyncRead`, `AsyncWrite` and `AsyncSeek`, as 0.9 required without
`generic-snapshot-data`. A network that overrode `full_snapshot()` with
`generic-snapshot-data` implements `RaftNetworkV2::full_snapshot()` instead.

### Chunked snapshots

- On the receiving side, `Raft::install_snapshot()` comes from the
  `openraft_legacy::network_v1::ChunkedSnapshotReceiver` trait, and
  `Raft::begin_receiving_snapshot()` is removed: the receiver gets its buffer
  from `SnapshotReceiverFactory::begin_receiving_snapshot()`, which the state
  machine implements.
- `openraft::network::snapshot_transport`, with `Chunked`,
  `SnapshotTransport` and `Streaming`, is removed: `Adapter` sends the chunks
  and `ChunkedSnapshotReceiver` receives them.
- Import `InstallSnapshotRequest`, `InstallSnapshotResponse`,
  `InstallSnapshotError`, `SnapshotMismatch` and `SnapshotSegmentId` from
  `openraft_legacy::network_v1`; their old paths are deprecated stubs.
  `InstallSnapshotRequest` keeps the 0.9 five-field wire layout. Its `meta`
  field is `openraft_legacy::network_v1::SnapshotMeta`, the 0.9-shaped type
  that still carries the id:

  ```ignore
  use openraft_legacy::network_v1::{InstallSnapshotRequest, SnapshotMeta};

  let req = InstallSnapshotRequest {
      vote,
      meta: SnapshotMeta { last_log_id, last_membership, snapshot_id },
      offset,
      data,
      done,
  };
  ```

  The v1 sender generates a new id for each transfer, so a retransmitted
  snapshot is never mistaken for the continuation of an aborted one.

### AppendEntries results

`Raft::append_entries()`, `RaftNetworkV2::append_entries()` and
`stream_append()` return an `AppendResult`: `Ok` with the last log id the
follower accepted, or an `AppendError`. A transport that keeps the
`AppendEntriesResponse` wire format converts at both ends:

```ignore
// Follower:
let reply = raft.append_entries(req).await.map(AppendEntriesResponse::from);

// Leader, in RaftNetworkV2::append_entries():
let prev_log_id = rpc.prev_log_id.clone();
let last_log_id = rpc.last_log_id();
let response: AppendEntriesResponse<C> = send(rpc).await?;
Ok(response.into_append_result(prev_log_id, last_log_id))
```

Replace `is_success()` with `is_ok()`, and `is_conflict()` with
`matches!(r, Err(AppendError::Conflict { .. }))`.

A transport that pipelines AppendEntries overrides `stream_append()`, which
by default sends the requests one by one through `append_entries()`, and the
follower passes the received stream to [`Raft::stream_append()`][].

### Other network changes

- Report a remote error, such as a `Fatal` error on the target node, as
  `Unreachable`.
- `RPCError::PayloadTooLarge` and its automatic retry are removed. Limit the
  payload with `Config::max_payload_entries`, and split or resend an
  oversized request in the network implementation.
- `RaftNetworkV2::backoff()` returns an `Option<Backoff>`; `None` uses
  `Config::backoff`.
- `VoteRequest` gains `leadership_transfer_from`: build it with
  `VoteRequest::new()`, or add `leadership_transfer_from: None` to struct
  literals. A custom wire format carries the field as an optional vote.
- `ForwardToLeader` gains a `reason` field; add it to struct literals and
  exhaustive patterns.
- `NetworkError` and `Unreachable` take the type config, as in
  `NetworkError::<C>::new(&e)` or `Unreachable::<C>::from_string("msg")`.
- To use Pre-Vote and leadership transfer, implement `RaftNetworkV2::pre_vote()`
  and `transfer_leader()`, and pass received requests to `Raft::pre_vote()`
  and `Raft::handle_transfer_leader()`. By default, `pre_vote()` grants the
  vote and `transfer_leader()` reports the target unreachable.
- `RaftNetworkFactory::new_heartbeat_client()` and `new_snapshot_client()`
  can create dedicated heartbeat and snapshot connections, so a transport that
  cannot multiplex one connection avoids head-of-line blocking. By default
  they call `new_client()`.

## Raft API

- [`Raft`][]`<C, SM>` names the state machine type. Add it where a `Raft` is
  stored, for example `type Raft = openraft::Raft<TypeConfig, StateMachine>;`.
- Linearizable reads take a policy and return a `ReadLogId`:

  ```ignore
  // 0.9:
  let read_log_id: Option<LogId> = raft.ensure_linearizable().await?;
  // 0.10:
  let read_log_id: ReadLogId<C> = raft.ensure_linearizable(ReadPolicy::ReadIndex).await?;
  ```

  `get_read_log_id()` returns `(ReadLogId, Option<LogId>)`. Use
  `ReadLogId::log_id()` where a `LogId` is needed, and match on
  `LinearizableReadError` instead of `CheckIsLeaderError`; it reports a stale
  lease as `QuorumNotEnough`.
- `Raft::is_leader()` returns a `bool` without a quorum check. Where the 0.9
  `is_leader().await` was meant, call `ensure_linearizable()`.
- `Raft::metrics()`, `data_metrics()` and `server_metrics()` return the
  runtime's watch receiver, `WatchReceiverOf<C, _>`, instead of a
  `tokio::sync::watch::Receiver`; with Tokio it is a wrapper around the tokio
  receiver. Import the `WatchReceiver` trait, and replace `borrow()` with
  `borrow_watched()`:

  ```ignore
  use openraft::async_runtime::WatchReceiver;

  let mut rx = raft.metrics();
  let leader = rx.borrow_watched().current_leader;
  rx.changed().await?;
  ```

- `Raft::client_write_ff(data, responder)` takes an
  `Option<WriteResponderOf<C>>` and returns `Result<(), Fatal<C>>` instead of
  a receiver. Build the responder, keep its receiver, and pass
  `Some(responder)`, or `None` to ignore the result:

  ```ignore
  let (responder, rx) = ProgressResponder::complete_only();
  raft.client_write_ff(request, Some(responder)).await?;
  let result = rx.await?;
  ```

- A custom responder implements `Responder<C, T>`: `on_complete()` replaces
  `send()`, `on_commit()` is optional, and `from_app_data()` and
  `type Receiver` are removed. A manual type config declares it as
  `type Responder<T> = MyResponder<T> where T: OptionalSend + 'static;`.
- `Raft::external_request()` is async and returns `Result<(), Fatal<C>>`.
- `Trigger::elect()` takes `pre_vote: bool`; pass `false` for the 0.9
  behavior.
- `RaftState::committed` is no longer a field: call `local_committed()`, or
  `cluster_committed()` for the frontier a quorum has granted.

The APIs deprecated in 0.8 and 0.9 are removed:

| Removed | Replacement |
|---|---|
| `Raft::enable_tick()`, `enable_heartbeat()`, `enable_elect()` | `Raft::runtime_config().tick()`, `heartbeat()`, `elect()` |
| `Raft::trigger_elect()`, `trigger_heartbeat()`, `trigger_snapshot()`, `purge_log()` | `Raft::trigger().elect()`, `heartbeat()`, `snapshot()`, `purge_log()` |
| `Membership::is_in_joint_consensus()` | `get_joint_config().len() > 1` |
| `IntoNodes::has_nodes()`, `IntoNodes::node_ids()` | none |
| `Wait::log(index, msg)` | `log_index(index, msg)`, then `applied_index(index, msg)` |
| `Wait::log_at_least(index, msg)` | `log_index_at_least(index, msg)`, then `applied_index_at_least(index, msg)` |
| `Wait::members(voters, msg)` | `voter_ids(voters, msg)`, which takes any `IntoIterator` of node IDs |
| `Config::send_snapshot_timeout` and `--send-snapshot-timeout` | `install_snapshot_timeout` and `--install-snapshot-timeout` |
| `Config::send_snapshot_timeout()` | `Config::install_snapshot_timeout()` |

`Wait::log()` and `Wait::log_at_least()` waited for both the last log index
and the applied index, so the replacement waits for each. Nothing in 0.10
reads `send_snapshot_timeout`: `install_snapshot_timeout` is the timeout for
sending a snapshot, so removing the field changes no timing.

## Membership

[`RaftTypeConfig`][] adds `Membership`, the read and write quorum rules, and
`MembershipEdit`, the change that [`Raft::change_membership()`][] takes.
[`declare_raft_types!`][] defaults them to the majority-based [`Membership`][]
and [`ChangeMembers`][], so a type config declared with the macro compiles
unchanged. A manual `RaftTypeConfig` impl adds both types and passes the
membership to the payload type:

```ignore
type Membership = openraft::impls::Membership<Self::NodeId, Self::Node>;
type MembershipEdit = openraft::impls::ChangeMembers<Self::NodeId, Self::Node>;
type Payload = openraft::impls::EntryPayload<Self::D, Self::Membership>;
```

Code that names the membership types changes as follows:

- `EntryPayload`, `StoredMembership`, `MembershipState`, `SnapshotMeta` and
  `Snapshot` take the membership type in place of the node ID and node types,
  as in `EntryPayload<D, Membership<NID, N>>`.
- Generic code uses `C::Membership` where it used
  `Membership<C::NodeId, C::Node>`, for example for
  `ClientWriteResponse::membership`, and reads a node with
  `StoredMembership::get_node()`.
- [`Raft::change_membership()`][] takes `impl Into<MembershipEditOf<C>>`, and
  [`Raft::initialize()`][] takes `impl Into<C::Membership>`. Calls that pass a
  `ChangeMembers`, or a map or set of nodes, compile unchanged with the
  default membership. Generic code adds the bounds
  `MembershipEditOf<C>: From<ChangeMembers<C::NodeId, C::Node>>` and
  `C::Membership: From<BTreeMap<C::NodeId, C::Node>>`.
- `Membership::new()` requires a node for every voter and returns
  `Result<_, NodeNotFound>`; `Membership::new_with_defaults()` creates the
  missing nodes with `N::default()`. A `BTreeSet<NID>` converts into a
  membership only with `()` or `EmptyNode` nodes.
- `EffectiveMembership` is removed: use `StoredMembership`.
  `Membership::learner_ids()` is deprecated: use `non_write_voter_ids()`.
- A custom membership implements `RaftMembership`, and a custom edit
  implements `RaftMembershipEdit<M>`.
- Right after a leader change, a membership change can fail with
  `ChangeMembershipError::UncommittedLeaderLog` until the leader's blank entry
  commits; retry it as for `InProgress`.

### Standard membership operations are optional

`RaftMembershipEdit` requires only `apply()` and its native `Error` type.
`openraft::membership::RaftMembershipEditOps<M>` provides optional
constructors for standard membership operations. To reuse
[`Raft::add_learner()`][], implement this trait for the configured edit type
and put the `add_learner(node_id, node) -> Self` constructor there. The edit
must preserve quorum rules, existing roles, and existing node metadata.

All provided edit types implement this capability, so applications keep using
`raft.add_learner(id, node, blocking)` without custom catch-up code. Generic
wrappers add `C::MembershipEdit: RaftMembershipEditOps<C::Membership>`.
Custom edits without learner support need no placeholder constructor and can
still use `change_membership()` and `append_membership()`.

The [`openraft-membership`][] crate provides other quorum rules:
`ThresholdMembership`, `WeightedMembership`, `HierarchicalMembership`,
`GridMembership` and `ExplicitMembership`, each with its change type. See the
[FAQ](crate::docs::faq#how-do-i-use-a-quorum-rule-other-than-a-majority) for
how to select one.

## Errors

- `StorageError<C>` is a struct of the subject, the verb and the source,
  instead of an enum with `Defensive` and `IO` variants. `StorageIOError` is a
  deprecated alias of it, `StorageError::into_io()` is deprecated, and
  `Violation` and `DefensiveError` are removed. `LackEntry`, which Openraft
  never returned, is removed.
- The source of an error is a `C::ErrorSource`: `StorageError::source` holds
  one, and the `source` of `ConfigError::ParseError` is a `BoxedErrorSource`.
  A custom `ErrorSource` implements `from_error()`, `from_string()`,
  `has_backtrace()` and `fmt_backtrace()`.
- `ChangeMembershipError` gains an error parameter `E`, `AnyError` by
  default; an error returned by `Raft` uses `C::ErrorSource`.
- `CheckIsLeaderError` is a deprecated alias of `LinearizableReadError`.
- `StreamingError` has no error parameter and no `RemoteError` variant.
- Handle `ClientWriteError::LogEntryDiscarded` apart from `ForwardToLeader`:
  the entry was accepted and then discarded, so its commit status is unknown,
  and a retry needs application-level deduplication.

Public enums gain variants, which exhaustive matches handle:

| Enum | New variants |
|---|---|
| `ChangeMembershipError` | `UnsupportedMembershipTransition`, `UncommittedLeaderLog`, `NodeMetadataChanged`, `UnprovenReadWriteIntersection`, `WriteVoterWithoutReceiveLogs`, `UserDefined` |
| `InitializeError` | `InvalidMembership`, which holds a `MembershipError` |
| `ClientWriteError` | `LogEntryDiscarded`, `PreconditionFailed` |
| `ConfigError` | `InvalidBackoff`, `HeartbeatMinIntervalTooLarge`, `QuorumLossProbeIntervalTooSmall` |
| `ChangeMembers` | `Batch` |
| `RPCTypes` | `TransferLeader` |
| `Metric` | `Committed` |

## Metrics and Wait

- `RaftMetrics::current_term` is a `C::Term`, and `membership_config` is an
  `Arc<StoredMembershipOf<C>>`.
- `RaftMetrics::committed` and `RaftDataMetrics::committed` are deprecated:
  read `local_committed`, or `cluster_committed` for the frontier a quorum has
  granted.
- `RaftMetrics::millis_since_quorum_ack` and
  `RaftDataMetrics::millis_since_quorum_ack` are deprecated: read
  `last_quorum_acked`, the instant of the last quorum acknowledgement.
- `RaftMetrics::new_initial()` reports `ServerState::Learner` instead of
  `Follower`, as `RaftServerMetrics` does.
- [`Wait`][]`::timeout` changes from `Duration` to `Option<Duration>`; `None`
  means no timeout, and the wait arms no timer. Code that builds a `Wait`
  directly wraps the duration in `Some`, and code that reads the field handles
  `None`. [`Raft::wait()`][] already takes an `Option<Duration>`.
- The receiver that `Raft::metrics()` returns has a new type; see
  [Raft API](#raft-api).

## Config

- [`Config`][] has new fields. Build it with `..Default::default()`, or add
  the new fields to exhaustive struct literals.
- A config file written for 0.9 works unchanged: when serde decodes it, every
  new field takes its default, and the removed `send_snapshot_timeout` is
  ignored. A command line that still passes `--send-snapshot-timeout` is
  rejected.
- `Config::validate()` checks two new constraints. A set
  `quorum_loss_probe_interval` must be at least
  `leader_lease + election_timeout_max`; see
  [CheckQuorum](crate::docs::protocol::check_quorum). With a non-zero
  `heartbeat_min_interval`:

  ```text
  heartbeat_interval + heartbeat_min_interval + heartbeat_interval * 13 / 64 < election_timeout_min
  ```

## Custom async runtimes

- `AsyncRuntime` and its channel traits are in `openraft-rt`, re-exported as
  `openraft::rt`. `openraft::TokioRuntime`, `openraft::impls::TokioRuntime`
  and `openraft::TokioInstant` are deprecated aliases of
  `openraft_rt_tokio::TokioRuntime` and `openraft_rt_tokio::TokioInstant`, and
  the `openraft::instant` module is removed: use `openraft::Instant`.
- `TokioInstant` is a struct that wraps `tokio::time::Instant`, instead of an
  alias of it. Code that mixed instants from metrics with
  `tokio::time::Instant` uses the `Instant` trait methods instead.
- A custom `AsyncRuntime` replaces `OneshotSender<T>`, `OneshotReceiver<T>`,
  `OneshotReceiverError` and `oneshot()` with `type Oneshot`, and adds
  `type Mpsc`, `type Watch`, `type Mutex<T>`, `new()` and `block_on()`. Its
  `ThreadLocalRng` implements `rand::Rng` from rand 0.10 instead of 0.8.
  `run()`, `select2()` to `select4()`, `spawn_blocking()` and
  `mpsc_recv_deadline()` have default implementations.
- `AsyncOneshotSendExt` is renamed to `OneshotSender`, and
  `TokioOneShotSender` is removed.
- `AsyncRuntime` no longer requires `Default`, `PartialEq` or `Eq`. In
  threaded mode, the runtime, `Mpsc` and `Watch` types need not be `Send` or
  `Sync`, and `JoinHandle`, `Sleep`, `Oneshot::Receiver`, `OneshotSender` and
  `MpscReceiver` need not be `Sync`. Generic code that relied on these bounds
  states them itself. Likewise, `#[add_async_trait]` no longer adds a `Send`
  supertrait to the trait it expands; add `OptionalSend` where a trait needs
  it.
- `openraft::testing::runtime::Suite` tests a custom runtime.

## Deprecations

These items still compile, with a deprecation warning:

| Deprecated | Use instead |
|---|---|
| `openraft::error` | `openraft::errors` |
| `CheckIsLeaderError` | `LinearizableReadError` |
| `StorageIOError`, `StorageError::into_io()` | `StorageError` |
| `LogFlushed`, `IOFlushed::log_io_completed()` | `IOFlushed`, `io_completed()` |
| `LogId::leader_id()` | `committed_leader_id()` |
| `Vote::commit()` | `to_committed()` |
| `Membership::learner_ids()` | `non_write_voter_ids()` |
| `RaftState::committed()`, `RaftMetrics::committed`, `RaftDataMetrics::committed` | `local_committed`, or `cluster_committed` |
| `RaftMetrics::millis_since_quorum_ack`, `RaftDataMetrics::millis_since_quorum_ack` | `last_quorum_acked` |
| `openraft::TokioRuntime`, `openraft::impls::TokioRuntime`, `openraft::TokioInstant` | `openraft_rt_tokio::TokioRuntime`, `openraft_rt_tokio::TokioInstant` |
| `RaftNetwork`, `InstallSnapshotRequest`, `InstallSnapshotResponse`, `InstallSnapshotError`, `SnapshotMismatch` and `SnapshotSegmentId` in `openraft` | the same names in `openraft_legacy::network_v1` |

## Behavior changes

- A leader that cannot confirm a recent quorum rejects proposals and lease
  reads with `ForwardToLeader` and `ForwardReason::LeaseExpired`; retry on the
  same node. With `Config::quorum_loss_probe_interval`, which is `None` by
  default, such a leader also gives way to a new leader; see
  [CheckQuorum](crate::docs::protocol::check_quorum).
- A restarted leader campaigns for a higher term at startup instead of
  resuming with its saved vote. Set `Config::enable_leader_restore` to
  `Some(false)` to make it wait for a normal election instead.
- A restarted follower keeps the lease of its committed vote and rejects other
  candidates for up to `election_timeout_max` after startup.
- A leader rejects vote requests while its quorum-acknowledged lease is
  valid, and `Trigger::elect(true)` does nothing while the local leader lease
  is valid.
- A membership change writes one entry instead of two when the old and new
  quorums intersect, and a leadership transfer to an up-to-date learner makes
  the learner the leader.
- A leader removed by a committed membership transfers its leadership to the
  most up-to-date voter and steps down after
  `Config::removed_leader_step_down`, 150 ms by default. With `Never`, it keeps
  leading until `Trigger::refresh_server_state()` is called.
- The log id of the entry that `Raft::initialize()` writes names the
  initializing node instead of node 0.
- The first tick comes up to one tick interval later than in 0.9, so a test
  that expects a leader within a tight startup budget may need more time.
- The API, notification and state-machine channels are bounded, so a burst of
  requests waits for capacity instead of growing memory.
- Retries to an unreachable node start at 200 ms and double up to 1 s, where
  0.9 waited a constant 500 ms, and the v1 `RaftNetwork::backoff()` default is
  a constant 200 ms. For the 0.9 timing, set `Config::backoff` to
  `Some("500ms 500ms 500ms".to_string())`, or override `backoff()` of a v1
  network.
- A node samples a new election timeout for every campaign, where 0.9 sampled
  one at startup.

## Data and wire compatibility

Stored data needs no migration with the default types: the serialized forms of
log entries, `Membership`, `StoredMembership`, `Vote`, `LeaderId`,
`SnapshotMeta` and `SnapshotSignature` are unchanged. `SnapshotMeta` and
`SnapshotSignature` still write an empty `snapshot_id` and ignore it when
reading, so snapshots stored by 0.9 load in 0.10 and the other way around,
under named and positional formats such as `bincode` and `postcard` alike.
The standard `LeaderId` still encodes `voted_for` as an `Option`, and rejects
a stored `None`.

Between 0.9 and 0.10 peers, snapshot metadata and the v1
`InstallSnapshotRequest` keep their 0.9 layout in named and positional
formats alike. With a named-field format such as JSON, a missing
`VoteRequest::leadership_transfer_from` decodes as `None`, and a missing
`ForwardToLeader::reason` as `NotLeader`; a positional format such as
`bincode` cannot decode a `ForwardToLeader` written by 0.9. Error bodies do
not interoperate, because `StorageError` was an enum in 0.9 and is a struct in
0.10: treat a serialized error from a peer of another version as diagnostic
text.

[`Config`]: crate::Config
[`Config::allow_log_reversion`]: crate::Config::allow_log_reversion
[`Raft`]: crate::Raft
[`Raft::add_learner()`]: crate::Raft::add_learner
[`Raft::change_membership()`]: crate::Raft::change_membership
[`Raft::initialize()`]: crate::Raft::initialize
[`Raft::stream_append()`]: crate::Raft::stream_append
[`Raft::wait()`]: crate::Raft::wait
[`RaftTypeConfig`]: crate::RaftTypeConfig
[`declare_raft_types!`]: crate::declare_raft_types
[`RaftLogStorage`]: crate::storage::RaftLogStorage
[`RaftLogReader`]: crate::storage::RaftLogReader
[`RaftStateMachine`]: crate::storage::RaftStateMachine
[`RaftSnapshotBuilder`]: crate::storage::RaftSnapshotBuilder
[`SnapshotMeta`]: crate::storage::SnapshotMeta
[`SnapshotSignature`]: crate::storage::SnapshotSignature
[`RaftNetworkV2`]: crate::network::RaftNetworkV2
[`RaftNetworkFactory`]: crate::network::RaftNetworkFactory
[`Wait`]: crate::metrics::Wait
[`Membership`]: crate::impls::Membership
[`ChangeMembers`]: crate::impls::ChangeMembers
[`leader_id_std::LeaderId`]: crate::impls::leader_id_std::LeaderId
[`openraft::testing::log::Suite`]: crate::testing::log::Suite
[`single-threaded`]: crate::docs::feature_flags#feature-flag-single-threaded
[`openraft-legacy`]: https://crates.io/crates/openraft-legacy
[`openraft-membership`]: https://crates.io/crates/openraft-membership
