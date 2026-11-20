# log-rocks

A RocksDB-backed implementation of
[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html).

This crate stores Raft log entries and metadata in the `logs` and `meta` RocksDB column families.
`RocksLogStore::open()` creates a standalone database. A caller that already owns a suitable
database can instead pass an `Arc<rocksdb::DB>` to `RocksLogStore::new()`.

## Reading an `openraft-rocksstore` 0.9 database

`openraft-rocksstore` 0.9 used the same `logs` and `meta` column families and keys, so this crate
reads the vote and the logs it wrote. Its database also holds the `sm_meta` and `sm_data` column
families of the state machine, and RocksDB opens a database only with all of them: open it
yourself and pass it to `RocksLogStore::new()`. [`sm-rocks`](../sm-rocks/) does not read that state
machine. `tests/rocksstore_0_9.rs` reopens a database that `openraft-rocksstore` 0.9.21 wrote.

## Performance

Raft log workloads are mostly append-only, with occasional suffix truncation and prefix purging.
RocksDB is a general-purpose LSM key-value store, so its memtables, compaction, and write
amplification add overhead that a purpose-built append-only log can avoid.

This implementation is useful as a durable example, but it does not provide optimal log-store
performance. Benchmark the workload and consider a dedicated append-only storage engine for
production deployments where log throughput or latency matters.
