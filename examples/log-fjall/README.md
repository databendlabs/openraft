# log-fjall

A [fjall](https://crates.io/crates/fjall)-backed implementation of
[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html).

fjall is an LSM-tree key-value store written in Rust, so building this crate compiles no C++,
unlike the RocksDB-backed [log-rocks](../log-rocks/).

This crate stores Raft log entries in the `logs` keyspace, keyed by the big-endian log index, and
the vote and the last purged log id in the `meta` keyspace. `FjallLogStore::open()` creates a
standalone database. A caller that already owns a `fjall::Database`, for example to share it with
[sm-fjall](../sm-fjall/), can instead pass it to `FjallLogStore::new()`.

```rust
let log_store = log_fjall::FjallLogStore::<TypeConfig>::open(storage_dir.join("log"))?;
```

## What reaches disk, and when

All keyspaces of a fjall database share one journal, the write-ahead log fjall replays on
startup. A write batch is atomic across keyspaces, and syncing the journal also persists every
write made before.

`save_vote` commits the vote and syncs the journal before returning. `append` commits its entries
in one batch, so that they are readable when it returns, then syncs the journal on another thread
and reports the result through the callback.

`truncate_after` and `purge` do not sync. fjall has no range delete, so every removed entry needs
its own tombstone, and each method writes all of its tombstones in one atomic batch; `purge` adds
the new purge point to the same batch. Removing truncated entries one at a time from the lowest
index would leave a hole in the logs if the process crashed halfway. A crash may undo a whole
truncation or purge, which costs only repeated work after the restart: Openraft truncates the
conflicting entries again, and purges again later. The next `append` syncs the journal and
persists both.

## Performance

A Raft log is appended in index order, truncated at the tail and purged at the head. With keys in
that order, fjall's compaction can move table files to the next level without rewriting them, but
every entry is still written twice: once to the journal, and again when fjall flushes its memtable
to a table file. A purge also writes a tombstone per entry, which compaction reclaims later.

A purpose-built append-only log writes each entry once, and frees a purged prefix by deleting
whole files. [log-wal](../log-wal/) shows that approach. This implementation is useful as a
durable example; benchmark the workload before choosing an LSM store for a production Raft log.
