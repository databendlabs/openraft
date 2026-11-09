# Guide for upgrading from [v0.9](https://github.com/databendlabs/openraft/tree/release-0.9) to v0.10:

> This guide is under construction; it currently covers the `snapshot_id` changes
> and the removed feature flags.

## `snapshot_id` moved out of snapshot metadata

A snapshot is identified by the position it covers: two snapshots at the same
`last_log_id` represent the same state, even when they differ in bytes. The
0.9 `snapshot_id` identified a snapshot *transfer*, not the snapshot, so it no
longer belongs in the metadata. It now lives only in the chunked v1 protocol,
which is the only code that needs it: the receiver compares it against the
in-flight stream to tell a new transfer from a continuation.

API changes:

- [`SnapshotMeta`][] loses its `snapshot_id` field. State machines no longer
  invent an id when building a snapshot:

  ```ignore
  let meta = SnapshotMeta {
      last_log_id,
      last_membership,
  };
  ```

- [`SnapshotSignature`][] loses its `snapshot_id` field.

- A [`RaftNetworkV2`][] (full-snapshot) implementation needs no change.

- A chunked v1 implementation uses `openraft-legacy`, where
  `InstallSnapshotRequest` keeps the exact 0.9 five-field wire layout. Its
  `meta` field is `openraft_legacy::network_v1::SnapshotMeta`, the 0.9-shaped
  type that still carries the id; only the import changes:

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

  The v1 sender now generates a fresh id per transfer session, so a
  retransmitted snapshot is never mistaken for a continuation of an aborted
  one.

No data migration is needed. The serialized layouts of [`SnapshotMeta`][] and
[`SnapshotSignature`][] are unchanged — three fields, with an always-empty
`snapshot_id` written and ignored on read — so snapshots stored by 0.9 load in
0.10 and vice versa, under named and positional (`bincode`, `postcard`)
formats alike.

Mixed-version caveat: request and metadata layouts interoperate with 0.9 peers
in both directions, but error bodies do not — `StorageError` was an enum in
0.9 and is a struct in 0.10 — so a peer of a different version should treat a
serialized error response as diagnostic text rather than parse it.

## Removed and renamed feature flags

Enabling any of these flags in 0.10 fails compilation with an error that names
the replacement:

- `adapt-network-v1` is removed. For backward compatibility with the v1
  `RaftNetwork` trait and chunk-based snapshot transport, use the
  [`openraft-legacy`][] crate instead, which provides
  `openraft_legacy::network_v1::RaftNetwork`.

- `loosen-follower-log-revert` is removed. Use
  [`Config::allow_log_reversion`][] instead:

  ```ignore
  let config = Config {
      allow_log_reversion: Some(true),
      ..Default::default()
  };
  ```

- `singlethreaded` is renamed to [`single-threaded`][]. Update your
  `Cargo.toml` to use `single-threaded` instead.

- `single-term-leader` is removed. To enable standard Raft mode, set `LeaderId`
  to [`leader_id_std::LeaderId`][], either in the `declare_raft_types!`
  statement or in the [`RaftTypeConfig`][] implementation.

## The membership type is configurable

[`RaftTypeConfig`][] adds `Membership`, the read and write quorum rules, and
`MembershipEdit`, the change that [`Raft::change_membership()`][] takes.
[`declare_raft_types!`][] defaults them to the majority-based
[`Membership`][] and [`ChangeMembers`][], so a type config declared with the
macro compiles unchanged. A manual `RaftTypeConfig` impl adds both types and
passes the membership to the payload type:

```ignore
type Membership = openraft::impls::Membership<Self::NodeId, Self::Node>;
type MembershipEdit = openraft::impls::ChangeMembers<Self::NodeId, Self::Node>;
type Payload = openraft::impls::EntryPayload<Self::D, Self::Membership>;
```

Code that names the membership types changes as follows:

- `EntryPayload`, `StoredMembership`, `MembershipState`, `SnapshotMeta` and
  `Snapshot` take the membership type in place of the node ID and node types,
  as in `EntryPayload<D, Membership<NID, N>>`. A `RaftPayload` impl replaces
  `type NodeId` and `type Node` with `type Membership`.
- Generic code uses `C::Membership` where it used
  `Membership<C::NodeId, C::Node>`, and reads a node with
  `StoredMembership::get_node()`.
- [`Raft::change_membership()`][] takes `impl Into<MembershipEditOf<C>>`, and
  [`Raft::initialize()`][] takes `impl Into<C::Membership>`. Calls that pass a
  `ChangeMembers`, or a map or set of nodes, compile unchanged with the default
  membership. Generic code adds the bounds
  `MembershipEditOf<C>: From<ChangeMembers<C::NodeId, C::Node>>` and
  `C::Membership: From<BTreeMap<C::NodeId, C::Node>>`.
- `ChangeMembershipError` gains an error parameter `E`, which defaults to
  `AnyError`, and the variants `UnsupportedMembershipTransition`,
  `UncommittedLeaderLog`, `NodeMetadataChanged`,
  `UnprovenReadWriteIntersection`, `WriteVoterWithoutReceiveLogs` and
  `UserDefined`. `InitializeError` gains `InvalidMembership`, which holds the
  new `MembershipError`. Exhaustive matches add arms for them.

The [`openraft-membership`][] crate provides other quorum rules:
`ThresholdMembership`, `WeightedMembership`, `HierarchicalMembership`,
`GridMembership` and `ExplicitMembership`, each with its change type. See the
[FAQ](crate::docs::faq#how-do-i-use-a-quorum-rule-other-than-a-majority) for
how to select one.

### Learner addition is an optional edit capability

`RaftMembershipEdit` requires only `apply()` and its native `Error` type.
To reuse `Raft::add_learner()`, implement
`openraft::membership::RaftAddLearner<M>` for the configured edit type and
put the `add_learner(node_id, node) -> Self` constructor there. The edit must
preserve quorum rules, existing roles, and existing node metadata.

All provided edit types implement this capability, so applications keep
using `raft.add_learner(id, node, blocking)` without custom catch-up code.
Generic wrappers add `C::MembershipEdit: RaftAddLearner<C::Membership>`.
Custom edits without learner support need no placeholder constructor and
can still use `change_membership()` and `append_membership()`.

## Optional quorum-loss inactivity configuration

[`Config`][] adds `quorum_loss_probe_interval: Option<u64>` in milliseconds.
Exhaustive Rust struct literals must add this field, or use
`..Default::default()`.

`ConfigError` adds a validation variant for a too-small probe interval;
exhaustive matches must handle it too.

The field defaults to `None` when omitted from named-field serde configuration.
See [CheckQuorum](crate::docs::protocol::check_quorum) for its behavior.

## `Wait::timeout` is optional

[`Wait`][]`::timeout` changes from `Duration` to `Option<Duration>`; `None` means
no timeout, and the wait arms no timer. Code that builds a `Wait` directly wraps
the duration in `Some`, and code that read the field handles `None`.
[`Raft::wait()`][] already takes an `Option<Duration>` and needs no change.

## Items deprecated since 0.9.0 are removed

| Removed | Replacement |
|---|---|
| [`Config`][]`::send_snapshot_timeout` and its `--send-snapshot-timeout` option | `install_snapshot_timeout` |
| `Config::send_snapshot_timeout()` | `Config::install_snapshot_timeout()` |
| [`Wait`][]`::log(index, msg)` | `log_index(index, msg)`, then `applied_index(index, msg)` |
| `Wait::log_at_least(index, msg)` | `log_index_at_least(index, msg)`, then `applied_index_at_least(index, msg)` |
| `Wait::members(voters, msg)` | `voter_ids(voters, msg)`, which takes any `IntoIterator` of node IDs |

Nothing in 0.10 reads `send_snapshot_timeout`; `install_snapshot_timeout` is
the timeout for sending a snapshot, so removing the field changes no timing. A
command line that still passes `--send-snapshot-timeout` is rejected, and a
serde-decoded config that still holds the field decodes and ignores it.

`Wait::log()` and `Wait::log_at_least()` waited for both the last log index
and the applied index, so the replacement waits for each.

[`Config`]: crate::Config
[`Wait`]: crate::metrics::Wait
[`Raft::wait()`]: crate::Raft::wait
[`SnapshotMeta`]:      `crate::storage::SnapshotMeta`
[`SnapshotSignature`]: `crate::storage::SnapshotSignature`
[`RaftNetworkV2`]:     `crate::network::RaftNetworkV2`

[`openraft-legacy`]:             https://crates.io/crates/openraft-legacy
[`Config::allow_log_reversion`]: `crate::Config::allow_log_reversion`
[`single-threaded`]:             `crate::docs::feature_flags#feature-flag-single-threaded`
[`leader_id_std::LeaderId`]:     `crate::impls::leader_id_std::LeaderId`
[`RaftTypeConfig`]:              `crate::RaftTypeConfig`
[`Membership`]:                  `crate::impls::Membership`
[`ChangeMembers`]:               `crate::impls::ChangeMembers`
[`declare_raft_types!`]:         `crate::declare_raft_types`
[`Raft::change_membership()`]:   `crate::Raft::change_membership`
[`Raft::initialize()`]:          `crate::Raft::initialize`
[`openraft-membership`]:         https://crates.io/crates/openraft-membership
