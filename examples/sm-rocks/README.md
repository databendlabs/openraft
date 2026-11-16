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

The log store and state machine use independent databases:

```rust
let log_store = log_rocks::RocksLogStore::<TypeConfig>::open(storage_dir.join("log"))?;
let state_machine = sm_rocks::RocksStateMachine::<TypeConfig>::open(storage_dir.join("sm"))?;
```

## Architecture

**Storage structure**: every database under the state-machine directory is a *generation*,
a complete RocksDB database with the `sm_data` and `sm_meta` column families.

```text
sm/
├── CURRENT               # name of the active generation
├── openraft-db-3f9a1c/   # active database
├── openraft-db-b72e05/   # checkpoint captured for a snapshot; hard links to the SST files
└── openraft-db-c04d88/   # checkpoint received from a leader
```

- A restart opens the generation named by `CURRENT` under exclusive ownership of
  the directory, then removes orphan generations with the `openraft-db-` prefix.
- Building a snapshot captures a RocksDB checkpoint of the active generation.
- Installing a snapshot opens a hard-linked copy of the received checkpoint as a new generation,
  points `CURRENT` at it, and retires the previous active generation. No key is copied.

`RocksSnapshotData` clones share ownership of their directory. Builders and
outgoing transfers retain their checkpoints across installation; database work
retains an `Arc<ActiveDb>`. Retired directories are removed only after their last
owner drops. A read-only RocksDB handle must be closed before releasing the
snapshot handle that owns its files. Local generation names are allocated
independently of peer snapshot IDs; `CURRENT` determines recovery selection.

**Key Code Locations**:
- `apply`, snapshot building, and snapshot installation: `src/state_machine.rs`
- The active generation and the `CURRENT` file: `src/state_machine/active_db.rs`
- Generation directories and the snapshot builder: `src/snapshot/`
- Request and response types: [`types-kv`](../types-kv/)

## Recovery guarantees

Apply batches sync application data, progress, and membership before responding.
Snapshot installation validates the checkpoint's metadata before publishing its
writable copy. The new generation is retained before replacing `CURRENT`; a
publication error keeps both possible generations for startup recovery. Cleanup
failure leaves an orphan that startup can remove after opening the selected
database. Open each state-machine directory exclusively during recovery.

Machine-failure durability requires RocksDB's synced writes and checkpoints,
atomic rename in the same directory, and a filesystem that honors file and
directory sync. `sync_directory()` runs only on Unix; this example does not
establish the same machine-failure guarantee on other platforms. Tests inject
I/O errors and kill a child at named boundaries, then reopen the store and run
OpenRaft startup recovery. These tests cover process interruption, not power loss.

## Comparison

| Feature | sm-rocks | [sm-mem](../sm-mem/) |
|---------|----------|----------------------|
| Storage | RocksDB (disk) | Memory |
| Persistence | Yes | No |
| Recovery | Full | None |
| Complexity | Higher | Lower |

Built for testing and demonstration purposes.
