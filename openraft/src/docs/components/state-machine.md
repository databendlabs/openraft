# State Machine Component

[`RaftStateMachine`] (`storage/mod.rs`) manages application state and snapshots in Openraft.

## Core Responsibilities

**Log Application**: [`apply`] processes committed log entries and updates state

**State Queries**: [`applied_state`] returns current state (last applied log, membership)

**Snapshot Management**: Building, receiving, and installing snapshots
- [`get_snapshot_builder`] - Create snapshots for compaction
- [`begin_receiving_snapshot`] - Prepare to receive snapshot from leader
- [`get_current_snapshot`] - Retrieve existing snapshot
- [`install_snapshot`] - Apply received snapshot to state machine

## State Recovery

State machines are typically in-memory and rebuild on startup:

1. **Load snapshot** - Restore state from last snapshot
2. **Replay logs** - Apply log entries not included in snapshot
3. **Resume operation** - State machine catches up to current state

This approach works because persistent snapshots + logs provide complete state history.

## Membership State Handling

**Dual state tracking**: Membership config and normal application logs maintain separate state

**Membership ahead of applied**: Membership `last_log_id` may exceed application `last_applied` if:
- Corresponding logs haven't been purged
- Logs can be replayed on restart
- Membership is loaded by scanning from `last_applied` forward

This allows membership changes to be tracked independently from application state.

## Implementation Tips

**In-memory state machine**: Fast operation, snapshot handles persistence

**Separate membership tracking**: Keep membership config state distinct from application data

**Snapshot strategy**: Balance snapshot frequency vs. log replay time on restart

[`RaftStateMachine`]:         `crate::storage::RaftStateMachine`
[`apply`]:                    `crate::storage::RaftStateMachine::apply`
[`applied_state`]:            `crate::storage::RaftStateMachine::applied_state`
[`get_snapshot_builder`]:     `crate::storage::RaftStateMachine::get_snapshot_builder`
[`begin_receiving_snapshot`]: `crate::storage::RaftStateMachine::begin_receiving_snapshot`
[`get_current_snapshot`]:     `crate::storage::RaftStateMachine::get_current_snapshot`
[`install_snapshot`]:         `crate::storage::RaftStateMachine::install_snapshot`
