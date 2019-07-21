todo
====
### algorithm optimizations
- [ ] update the AppendEntries RPC receiver to pipeline all requests. This will potentially remove the need for having to use boolean flags to mark if the Raft is currently appending logs or not.
- [ ] in the client request handling logic, after logs have been appended locally and have been queued for replication on the streams, the pipeline can immediately begin processing the next payload of entries. **This optimization will greatly increse the throughput of this Raft implementation.**
    - this will mean that the process of waiting for the logs to be committed to the cluster and responding to the client will happen outside of the main pipeline.
    - multiple payloads of enntries will be queued and delivered at the same time from replication streams to followers, so batches of requests may become ready for further processing/response when under heavy write load.
    - applying the logs to the state machine will also happen outside of the pipeline. It must be ensured that entries are not sent to the state machine to be applied out-of-order. It is absolutely critical that they are sent to be applied in index order.
    - the pipeline for applying logs to the statemachine should also perform batching whenever possible.
- [ ] maybe: update the append entries algorithm for followers so that recent entries are buffered up to a specific threshold so that the `apply_logs_to_statemachine` won't need to fetch from storage first.

### testing
- [ ] finish implement MemoryStroage for testing (and general demo usage).
- [ ] test client writes.
    - cover case where requests are forwarded to leader.
    - cover case where leader dies during write load.
    - cover case where leader is connected to during writes, leader dies, and writes should not make progress. Client should timeout the request and try on a new node.
    - ensure data in storage is exactly as needed and in order.
- [ ] test snapshots.
    - cover case where cluster is making progress, and then a new node joins, but is not snapshotted because it is not too far behind.
    - cover case where new node node joins after cluster has make a lot of progress, and then new node should receive snapshot.


### snapshots
- [ ] get the system in place for periodic snapshot creation.

### admin commands
- [ ] get AdminCommands setup and implemented.

### observability
- [ ] instrument code with tokio trace: https://docs.rs/tokio-trace/0.1.0/tokio_trace/

### docs
- [ ] add a simple graphic to an `overview` section of the main README to visually demonstrate the layout of this system.
- [ ] put together a networking guide on how to get started with building a networking layer.
- [ ] put together a storage guide on how to implement an application's RaftStorage layer.
- [ ] add docs examples on how to instantiate and start the various actix actor components (Raft, RaftStorage, RaftNetwork &c).
- [ ] add docs on how metrics system works. Pumps out regular metrics configured interval. State changes to the node are always immediately published.
