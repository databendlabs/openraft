# log-rocks

A RocksDB-backed implementation of
[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html).

This crate stores Raft log entries and metadata in the `logs` and `meta` RocksDB column families.
The caller owns the database and passes an `Arc<rocksdb::DB>` to `RocksLogStore::new()`.

## Performance

Raft log workloads are mostly append-only, with occasional suffix truncation and prefix purging.
RocksDB is a general-purpose LSM key-value store, so its memtables, compaction, and write
amplification add overhead that a purpose-built append-only log can avoid.

This implementation is useful as a durable example, but it does not provide optimal log-store
performance. Benchmark the workload and consider a dedicated append-only storage engine for
production deployments where log throughput or latency matters.
