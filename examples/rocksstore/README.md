# openraft-rocksstore

A RocksDB-backed persistent state machine implementation for Openraft.

## Key Features Demonstrated

- **Persistent storage**: [`RaftStateMachine`] with RocksDB
- **Column families**: Separate storage for state machine data and metadata
- **Durability**: On-disk persistence for cluster recovery
- **Performance**: Efficient batch operations and compaction

## Overview

This example implements
**[`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html)**
for persistent application state. The RocksDB log store is in [`log-rocks`](../log-rocks/).

Built with [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/) for production-grade durability and performance.

## Usage

```rust
use openraft_rocksstore::RocksStore;

// Create a persistent store
let store = RocksStore::new(path)?;
```

## Architecture

**Storage structure**:
- State machine data in separate column family

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
