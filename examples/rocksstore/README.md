# openraft-rocksstore

A RocksDB-backed persistent storage implementation for Openraft, demonstrating production-ready log storage and state machine patterns.

## Key Features Demonstrated

- **Persistent storage**: [`RaftLogStorage`] and [`RaftStateMachine`] with RocksDB
- **Column families**: Separate storage for logs, state machine, and metadata
- **Durability**: On-disk persistence for cluster recovery
- **Performance**: Efficient batch operations and compaction

## Overview

This example implements:
- **[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html)** - Persistent Raft log storage
- **[`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html)** - Persistent application state machine

Built with [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/) for production-grade durability and performance.

## Usage

```rust
use openraft_rocksstore::RocksStore;

// Create a persistent store
let store = RocksStore::new(path)?;
```

## Architecture

**Storage structure**:
- Logs stored in dedicated RocksDB column family
- State machine data in separate column family
- Vote and metadata persisted independently

**Key Code Locations**:
- Storage implementation: `src/lib.rs`
- Type definitions: See parent example for network and client implementations

## Comparison

| Feature | rocksstore | memstore |
|---------|------------|----------|
| Storage | RocksDB (disk) | Memory |
| Persistence | Yes | No |
| Recovery | Full | None |
| Complexity | Higher | Lower |

Built for testing and demonstration purposes.
