# log-wal

A write-ahead-log implementation of
[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html),
built on the [raft-log](https://crates.io/crates/raft-log) crate.

`raft-log` stores records in chunk files in write order. A Raft log only grows at the tail, drops a
suffix on truncation and drops a prefix on purge, so a purge frees whole chunk files instead of
leaving keys for a compaction to reclaim. Compare with [log-rocks](../log-rocks/), which stores the
same data in a general-purpose LSM key-value store.

`WalLogStore::open(dir)` opens the log in `dir` and creates it when `dir` holds no log yet. Every
clone shares one log behind an async `RwLock`, so reads for replication run at the same time as each
other and only the writing methods take the lock exclusively.

```rust
let log_store = WalLogStore::<TypeConfig>::open("/path/to/raft-log-dir")?;
```

`WalLogStore::open_with_config` takes a `raft_log::Config` instead of a directory, which sets the
chunk size limits and the size of the payload cache that serves reads without touching disk.

## Adapting openraft to raft-log

Two pieces of glue sit between the two crates.

`raft-log` reads and writes every value through the `codeq::Codec` trait, which openraft types do
not implement. `src/codec.rs` adds that impl with two wrappers: `MsgPack<T>` for any serde type, and
`MsgPackVote<C>` for the vote, which additionally needs the `PartialOrd` that `raft_log::Types::Vote`
requires. Both encode with MessagePack, because a decoder has to stop at the end of one value in a
stream that holds more records after it.

`raft-log` hands a flush to a background worker and returns before the data reaches disk. The worker
calls back when the write lands. `src/callback.rs` routes that result either to openraft, which
tracks it as flushed IO, or to a caller that waits for the flush before returning.

## What reaches disk, and when

`append` and `save_vote` fsync. A vote decides an election and an append is the entry openraft
promises to a quorum, so neither may be lost. `save_vote` waits for the fsync before returning;
`append` returns at once and lets openraft learn about the completion through its callback.

`save_committed`, `truncate_after` and `purge` do not fsync. Each one writes a record that a crash
may drop, and each loss costs only repeated work after the restart: the state machine re-applies a
few committed entries, openraft truncates the conflicting suffix again, or the purge runs again. The
records reach disk with the next `append`.
