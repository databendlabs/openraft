### Slow replication performance with RocksDB or disk storage

**Symptom**: Write throughput is much lower than expected when using disk-based [`RaftLogStorage`][]

**Cause**: Synchronous writes to disk block the Raft thread. Additionally, HTTP client connection
pooling (in libraries like `reqwest`) can add 40ms+ latency spikes.

**Solution**:
- Use non-blocking I/O in your [`RaftLogStorage::append`][] implementation
- Consider batching writes in your storage layer
- For network layer, prefer WebSocket or connection pooling that doesn't introduce latency
- With RocksDB: disable `sync` on writes if you can tolerate some data loss on crash

[`RaftLogStorage`]: `crate::storage::RaftLogStorage`
[`RaftLogStorage::append`]: `crate::storage::RaftLogStorage::append`
