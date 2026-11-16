# State Machine Component

[`RaftStateMachine`] serves as the core API for managing the state machine and snapshot functionalities.
It directly ties the concepts of state management and snapshotting, acknowledging that snapshots are often simply persisted states of the state machine.

## Key Responsibilities

The [`RaftStateMachine`] encapsulates several critical responsibilities:

1. **Log Application**: It requires an implementation of the [`apply`] method, where the state machine processes and applies committed log entries. This method is central to maintaining the state machine's integrity and ensuring that all state transitions are based on the replicated and committed log entries.

2. **Querying State and Snapshots**: [`applied_state`] allows querying the current state of the state machine.

3. **Snapshot Handling**: Through methods like [`get_snapshot_builder`], [`get_current_snapshot`], and [`install_snapshot`], it defines a comprehensive approach to creating and installing snapshots.

## State Management in Raft State Machines

- **State Reversion and Recovery**:
  The state machine in the Raft application is typically an in-memory component. Upon startup, the state machine may revert to a previous state. This setup is generally acceptable because the combination of a persistent snapshot and the Raft logs provides sufficient information to reconstruct the state. This process involves first rebuilding the state machine from the snapshot and then reapplying any logs that are not included in the snapshot.

  Afterward, Raft log entries are applied to update the state machine to its current state.

- **Distinct Management of Membership and Normal Logs**:
  Within the state machine, the state of membership configuration logs and the state of normal logs are managed separately, though they are stored together. These can be thought of as two distinct sections.

- **Membership Config State Beyond the Last Applied**:
  It is acceptable for the membership to return with a log ID greater than the last applied log ID, provided that the corresponding Raft logs have not been purged and can thus be reapplied to the state machine. Upon startup, the most recent membership configuration is loaded by scanning the logs starting from the `last-applied-log-id`.

## Recovery boundaries

After recovery, application state must match its reported `last_applied` position.
Durable state, snapshots, and retained committed logs must reconstruct every
acknowledged entry. An `apply()` stream may commit successive prefixes; a later
error does not require rolling back an earlier prefix. Success accounts for the
entire supplied stream.

A persistent state machine persists application effects and progress before
sending responses. A synced database batch containing data, progress, and
membership is one implementation. Related memory caches become visible only
after the commit. The membership-ahead allowance above remains valid when the
required logs are retained.

A transient state machine publishes effects to readers before responding and
keeps a durable snapshot plus the logs needed to recover later entries. Implement
[`save_committed`] so startup can automatically replay through the committed
position. Atomic database visibility and persistence across a machine failure
are separate properties; document the filesystem and sync guarantees required
by the implementation.

Snapshot installation validates the candidate's data and complete metadata,
durably selects the replacement, then publishes live handles. Successful
installation exposes matching application data, apply progress, membership, and
current snapshot. Persistent state machines may rebuild snapshot files after
restart. [`StorageHelper::get_initial_state()`] restores snapshots, replays
committed logs, and rebuilds a snapshot when purged logs require one.

When publication fails with an uncertain outcome, retain both possible recovery
generations and return the error. Startup must select a complete generation
before reclaiming unreferenced directories. Test injected I/O errors separately
from process interruption: errors exercise unwinding and `Drop`, while stopping
a child process leaves cleanup unfinished. Neither test alone proves durability
against machine power loss.

## Deterministic commands and durable schemas

Every replica must produce the same state and logical response from the same
committed commands, whether `apply()` receives entries individually, together,
or in uneven groups. A timestamp chosen once by the leader and recorded in the
log is shared input. Reading a replica's own clock, random generator, or external
service during `apply()` can instead produce different results on each replica.
Record those inputs in the command before replication.

Serialized commands, stored values, and snapshot payloads are durable application
schemas. Keep fixtures written by released versions and test decoding them with
the new code. Positional formats such as postcard encode enum variant indices
and struct field order; removing or reordering a field or variant can change the
meaning of old bytes. Retain reserved fields or provide an explicit migration.
The [`types-kv` compatibility tests] keep literal JSON and postcard fixtures for
commands and stored values. Test the format the production store actually uses;
a fresh encode/decode round trip alone does not test old-data compatibility.

The opt-in [`ApplicationSuite`] accepts independent expected state and responses
for grouping and snapshot replacement checks. `StoreBuilder` and `Suite::test_all`
remain the basic metadata/storage checks. Transport and crash scenarios must also
run through the application's real receive and recovery paths.

[`types-kv` compatibility tests]: https://github.com/databendlabs/openraft/blob/main/examples/types-kv/src/lib.rs
[`ApplicationSuite`]: `crate::testing::log::ApplicationSuite`

## Transfer completion and file ownership

`SnapshotData` is an opaque application handle. The transport owns receive
preparation and completion; the state machine owns durable publication. A file
receiver creates missing parent directories, opens a readable and writable
file, finishes buffered writes, and positions the reader at the agreed start
before calling `Raft::install_full_snapshot()`. The legacy chunk receiver
performs shutdown and rewind when it receives the final `done` chunk.

Transfer validation uses evidence supplied by the sender: declared lengths,
checksums, and an end record. The [`dir-transfer` example] checks its manifest's
file lengths, each file's checksum, and the final `End` record before returning
a completed directory. A receiver's observed byte count alone cannot detect
truncation. The legacy v1 wire format retains its `done` signal and gains no
length or checksum fields. In either case, installation must validate the
snapshot format and its metadata before publishing state.

| Stage | Owner and release rule |
|-------|------------------------|
| Receive or build | A private local handle owns the candidate files and any pending I/O. Abandoning the work releases those files after its last owner finishes. |
| Transfer complete | The completed snapshot handle retains the files while installation validates them. |
| Durable selection | A record such as `CURRENT` selects the recovery generation. Keep every possibly selected generation when publication has an uncertain outcome. |
| Retired generation | Existing readers, builders, outgoing transfers, and database handles retain their files until the last owner releases them. |
| Startup | Under exclusive ownership of the directory, open and validate the selected generation, then reclaim orphan candidates and retired generations. |

Allocate local names independently of peer snapshot IDs. An ID identifies a
protocol snapshot; it neither names a trusted path nor determines file lifetime.
Directory names and timestamps do not establish which generation is current or
which generations still have live owners.

[`dir-transfer` example]: https://github.com/databendlabs/openraft/tree/main/examples/dir-transfer

[`save_committed`]: `crate::storage::RaftLogStorage::save_committed`
[`StorageHelper::get_initial_state()`]: `crate::StorageHelper::get_initial_state`
[`RaftStateMachine`]:         `crate::storage::RaftStateMachine`
[`apply`]:                    `crate::storage::RaftStateMachine::apply`
[`applied_state`]:            `crate::storage::RaftStateMachine::applied_state`
[`get_snapshot_builder`]:     `crate::storage::RaftStateMachine::get_snapshot_builder`
[`get_current_snapshot`]:     `crate::storage::RaftStateMachine::get_current_snapshot`
[`install_snapshot`]:         `crate::storage::RaftStateMachine::install_snapshot`
