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

[`save_committed`]: `crate::storage::RaftLogStorage::save_committed`
[`StorageHelper::get_initial_state()`]: `crate::StorageHelper::get_initial_state`
[`RaftStateMachine`]:         `crate::storage::RaftStateMachine`
[`apply`]:                    `crate::storage::RaftStateMachine::apply`
[`applied_state`]:            `crate::storage::RaftStateMachine::applied_state`
[`get_snapshot_builder`]:     `crate::storage::RaftStateMachine::get_snapshot_builder`
[`get_current_snapshot`]:     `crate::storage::RaftStateMachine::get_current_snapshot`
[`install_snapshot`]:         `crate::storage::RaftStateMachine::install_snapshot`
