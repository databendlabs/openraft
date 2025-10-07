# State Machine Component

[`RaftStateMachine`] serves as the core API for managing the state machine and snapshot functionalities.
It directly ties the concepts of state management and snapshotting, acknowledging that snapshots are often simply persisted states of the state machine.

## Key Responsibilities

The [`RaftStateMachine`] encapsulates several critical responsibilities:

1. **Log Application**: It requires an implementation of the [`apply`] method, where the state machine processes and applies committed log entries. This method is central to maintaining the state machine's integrity and ensuring that all state transitions are based on the replicated and committed log entries.

2. **Querying State and Snapshots**: [`applied_state`] allows querying the current state of the state machine.

3. **Snapshot Handling**: Through methods like [`get_snapshot_builder`], [`begin_receiving_snapshot`], [`get_current_snapshot`], and [`install_snapshot`], it defines a comprehensive approach to managing snapshots. These methods cover creating snapshots, handling incoming snapshot data from the leader, and installing snapshots to bring the state machine to a specific state.

## State Management in Raft State Machines

- **State Reversion and Recovery**:
  The state machine in the Raft application is typically an in-memory component. Upon startup, the state machine may revert to a previous state. This setup is generally acceptable because the combination of a persistent snapshot and the Raft logs provides sufficient information to reconstruct the state. This process involves first rebuilding the state machine from the snapshot and then reapplying any logs that are not included in the snapshot.

  Afterward, Raft log entries are applied to update the state machine to its current state.

- **Distinct Management of Membership and Normal Logs**:
  Within the state machine, the state of membership configuration logs and the state of normal logs are managed separately, though they are stored together. These can be thought of as two distinct sections.

- **Membership Config State Beyond the Last Applied**:
  It is acceptable for the membership to return with a log ID greater than the last applied log ID, provided that the corresponding Raft logs have not been purged and can thus be reapplied to the state machine. Upon startup, the most recent membership configuration is loaded by scanning the logs starting from the `last-applied-log-id`.

[`RaftStateMachine`]:         `crate::storage::RaftStateMachine`
[`apply`]:                    `crate::storage::RaftStateMachine::apply`
[`applied_state`]:            `crate::storage::RaftStateMachine::applied_state`
[`get_snapshot_builder`]:     `crate::storage::RaftStateMachine::get_snapshot_builder`
[`begin_receiving_snapshot`]: `crate::storage::RaftStateMachine::begin_receiving_snapshot`
[`get_current_snapshot`]:     `crate::storage::RaftStateMachine::get_current_snapshot`
[`install_snapshot`]:         `crate::storage::RaftStateMachine::install_snapshot`
