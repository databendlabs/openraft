# sm-mem

A minimal in-memory implementation of [`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html) for key-value storage.

## Overview

This crate provides a generic state machine component for Raft. It implements:
- **`RaftStateMachine`**: Applies log entries and manages state
- **`RaftSnapshotBuilder`**: Creates and restores snapshots

The state machine is parameterized by `RaftTypeConfig` with constraints:
- `D = types_kv::Request`
- `R = types_kv::Response`
- `SnapshotData = Cursor<Vec<u8>>`

## What's NOT included

- **`RaftLogStorage`**: For log storage, see [`log-mem`](../log-mem/)

## Usage

```rust
use sm_mem::StateMachineStore;

// Create an in-memory state machine store
let state_machine_store = StateMachineStore::default();
```

For production use, consider using a persistent storage backend instead of in-memory storage.
