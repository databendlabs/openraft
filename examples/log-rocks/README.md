# log-rocks

A RocksDB-backed implementation of
[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html).

This crate stores Raft log entries and metadata in the `logs` and `meta` RocksDB column families.
`RocksLogStore::open()` creates a standalone database. A caller that already owns a suitable
database can instead pass an `Arc<rocksdb::DB>` to `RocksLogStore::new()`.

## Durability

`append()` writes with `WriteOptions::set_sync(true)`, so RocksDB syncs its WAL before the entries
become readable, and calls the callback once the write returns. Openraft requires this while
`Config::enable_leader_restore` is on, as it is by default: a restored leader must not have sent an
entry that a crash then removed from its own log. Each append waits for the sync; with leader
restore off, a store may instead return before the sync and report it through the callback.

## Performance

Raft log workloads are mostly append-only, with occasional suffix truncation and prefix purging.
RocksDB is a general-purpose LSM key-value store, so its memtables, compaction, and write
amplification add overhead that a purpose-built append-only log can avoid.

This implementation is useful as a durable example, but it does not provide optimal log-store
performance. Benchmark the workload and consider a dedicated append-only storage engine for
production deployments where log throughput or latency matters.
