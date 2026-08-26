# raft-kv-log-wal-sm-mem

The [raft-kv-memstore](../raft-kv-memstore/) key-value application with its log store swapped for
[log-wal](../log-wal/), a write-ahead log on disk. The state machine stays the in-memory
[sm-mem](../sm-mem/).

Read [raft-kv-memstore/README.md](../raft-kv-memstore/README.md) first. It explains the type config,
the two HTTP servers, and the write and read flows, all of which this crate reuses unchanged. Only
the differences are described below.

## Persistent log, volatile state machine

The Raft log survives a restart because `log-wal` writes it to disk. The key-value data does not,
because `sm-mem` holds it in memory. A restarted node therefore has to rebuild its state machine,
and it does so by re-applying committed entries from the log.

`StorageHelper::get_initial_state()` performs the rebuild. It reads the committed log id that
`log-wal` persisted, compares it with the state machine's `last_applied`, and re-applies every entry
in between. A fresh `sm-mem` reports `last_applied = None`, so the replay starts at the first entry
the log still holds.

That replay only works while the log still holds every committed entry, which is what
`example_config()` in [`src/lib.rs`](./src/lib.rs) guarantees:

```rust
snapshot_policy: SnapshotPolicy::Never,
max_in_snapshot_log_to_keep: u64::MAX,
```

`SnapshotPolicy::Never` stops openraft from building a snapshot, and `u64::MAX` stops it from
purging entries a snapshot already covers. Together they mean the log grows without bound. A real
deployment pairs a persistent log with a persistent state machine — see
[raft-kv-rocksdb](../raft-kv-rocksdb/) — and lets snapshots and purging run.

## Where the log is written

`--data-dir` selects the directory `log-wal` writes into. It defaults to `<api-addr>.wal`, so a node
started with `--api-addr 127.0.0.1:21001` writes to `127.0.0.1:21001.wal` in the working directory.
Each node needs its own directory: `log-wal` takes an exclusive lock on it.

```shell
cargo build
./target/debug/raft-kv-log-wal-sm-mem \
    --id 1 --api-addr 127.0.0.1:21001 --raft-addr 127.0.0.1:22001 --data-dir /tmp/node1.wal
```

From there the cluster is formed with the same three admin calls the canonical example uses:
`POST /init`, then `POST /add-learner` for each other node, then `POST /change-membership`.

## Test

```shell
cargo test
```

[`tests/cluster/test_cluster.rs`](./tests/cluster/test_cluster.rs) forms a 3-node cluster over HTTP,
writes a key and reads it back on every node. Each node gets a data directory that does not exist
yet, so this test also covers creating the WAL directory on a first start.

[`tests/cluster/test_restart.rs`](./tests/cluster/test_restart.rs) writes a key through a single-node
`Raft`, shuts the node down, and reopens the same directory with a fresh `sm-mem`. The key can only
be in the new state machine by way of re-applying the WAL.

`rebuilds_state_machine_from_wal_after_restart` in [`src/lib.rs`](./src/lib.rs) pins the replay
boundary below the `Raft` layer: it appends three entries, marks the middle one committed, closes the
store, and reopens it with a fresh `sm-mem`. It then asserts that `last_applied` stopped at the
committed entry and that the key-value write it carried is back in the state machine.
