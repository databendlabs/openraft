# sm-fjall

A [fjall](https://crates.io/crates/fjall)-backed persistent state machine implementation for
Openraft.

## Overview

This crate implements
**[`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html)**
for the key-value requests in [`types-kv`](../types-kv/). The fjall log store is in
[`log-fjall`](../log-fjall/).

A state machine suits an LSM tree better than a Raft log does: it reads and overwrites keys in any
order, and building a snapshot needs a consistent view of all of them, which fjall provides
cheaply. Openraft keeps log storage and state machine independent, so this state machine also works
with an append-only log store such as [`log-wal`](../log-wal/).

## Usage

The log store and state machine can use independent databases:

```rust
let log_store = log_fjall::FjallLogStore::<TypeConfig>::open(storage_dir.join("log"))?;
let state_machine = sm_fjall::FjallStateMachine::<TypeConfig>::open(storage_dir.join("sm"))?;
```

Or share one: open a `fjall::Database` and pass it to both `FjallLogStore::new()` and
`FjallStateMachine::new()`. They use different keyspaces, and one journal sync then persists the
writes of both.

## Architecture

**Storage structure**: three keyspaces of one fjall database.

- `sm_data`: every application key, mapped to its JSON-encoded `VersionedValue`
- `sm_meta`: the last applied log id and the last applied membership
- `sm_snapshot`: the metadata and the data of the current snapshot

**Applying entries**: `apply()` writes the values it sets and the last applied log id in one
atomic batch, and syncs the batch before sending responses. `applied_state()` therefore never
reports an entry whose effect a crash could lose, and Openraft re-applies only the entries after
it.

**Snapshots**: `get_snapshot_builder()` takes a fjall snapshot, a read view of the database, while
it holds `&mut self`, so the view contains exactly the applied state. The builder encodes the data
from that view on a blocking thread while `apply()` goes on, and stores the result in
`sm_snapshot`, so `get_current_snapshot()` returns it after a restart too. `install_snapshot()`
replaces the data, the applied state and the current snapshot in one atomic batch, so a crash
leaves either the old state or the whole snapshot. A built snapshot never replaces a newer one that
was installed while it was being built.

The snapshot data is the whole key-value map encoded as one JSON document, which building and
installing hold in memory. An application with a large state should stream it instead.

**Key Code Locations**:
- `apply` and snapshot installation: `src/state_machine.rs`
- Snapshot building: `src/snapshot_builder.rs`
- Request and response types: [`types-kv`](../types-kv/)

## Comparison

| Feature | sm-fjall | [sm-rocks](../sm-rocks/) | [sm-mem](../sm-mem/) |
|---------|----------|--------------------------|----------------------|
| Storage | fjall (disk) | RocksDB (disk) | Memory |
| Persistence | Yes | Yes | No |
| Snapshot data | JSON-encoded key-values | RocksDB checkpoint | JSON-encoded key-values |
| Native dependency | None | RocksDB (C++) | None |

Built for testing and demonstration purposes.
