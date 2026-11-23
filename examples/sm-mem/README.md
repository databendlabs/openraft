# sm-mem

A minimal in-memory implementation of [`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html) for key-value storage.

## Overview

This crate provides a generic state machine component for Raft. It implements:
- **`RaftStateMachine`**: Applies log entries and manages state
- **`RaftSnapshotBuilder`**: Creates and restores snapshots

The state machine is parameterized by `RaftTypeConfig` with constraints:
- `D: types_kv::KvCommand`, such as `types_kv::Request`, or `types_kv::DedupRequest`
  for writes that carry a request ID
- `R = types_kv::Response`

A write with a request ID is applied at most once. For each client, the state
machine keeps the serial and the response of the latest applied write, in
`StateMachineData::sessions`. A resent copy of that write gets its stored
response, and a write older than it is not applied. The sessions are part of the
snapshot, so a node that installs one still recognizes a resent write.
`types_kv::RequestId` states the rule a client follows.

The snapshot data type is the state machine's own associated type, not a `RaftTypeConfig`
constraint:
- `RaftStateMachine::SnapshotData = Cursor<Vec<u8>>`

## What's NOT included

- **`RaftLogStorage`**: For log storage, see [`log-mem`](../log-mem/)

## Usage

```rust
use sm_mem::StateMachineStore;

// Create an in-memory state machine store
let state_machine_store = StateMachineStore::default();
```

For production use, consider using a persistent storage backend instead of in-memory storage.
