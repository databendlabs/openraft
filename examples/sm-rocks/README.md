# sm-rocks

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

`new()` opens one RocksDB instance and returns a log store and a state machine
that share it, so both halves of storage live in the same database:

```rust
let (log_store, state_machine) = sm_rocks::new::<TypeConfig, _>(db_path).await?;
```

## Architecture

**Storage structure**:
- State machine data in separate column family

**Key Code Locations**:
- State machine implementation: `src/state_machine.rs`
- Type definitions: See parent example for network and client implementations

## Comparison

| Feature | sm-rocks | [sm-mem](../sm-mem/) |
|---------|----------|----------------------|
| Storage | RocksDB (disk) | Memory |
| Persistence | Yes | No |
| Recovery | Full | None |
| Complexity | Higher | Lower |

Built for testing and demonstration purposes.
