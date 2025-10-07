# mem-log

A minimal in-memory implementation of [`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html).

## Overview

This crate provides only the log storage component for Raft. It implements:
- **`RaftLogStorage`**: Stores and manages Raft log entries in memory

## What's NOT included

- **`RaftStateMachine`**: For a complete working example with state machine implementation, see:
  - [`raft-kv-memstore`](../raft-kv-memstore/) - Key-value store example
  - [`memstore`](../../stores/memstore/) - Full in-memory storage implementation

## Usage

This crate is used as a storage component by other Openraft examples. It's intentionally minimal to demonstrate log storage in isolation.

```rust
use openraft_memlog::LogStore;

// Create an in-memory log store
let log_store = LogStore::default();
```

For production use, consider using a persistent storage backend instead of in-memory storage.