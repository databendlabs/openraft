todo
====
- [ ] introduce a series of new zero-size struct types which wrap `u64`. These will be used to ensure that subtle bugs don't crop up where a node's term is passed as the node's ID, or the node's last_log_index is passed as the node's last_log_term.

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
- [ ] setup testing framework to assert accurate behavior of Raft implementation and adherence to Raft's safety protocols.
- [x] all actor based. Transport layer can be a simple message passing mechanism.
- [ ] explore unit testing specific methods of the various actors by creating an idle instance from within a call to https://docs.rs/actix/0.8.3/actix/trait.Actor.html#method.create ; this provides a context which can be used for calling various methods. The RaftRouter can be used to record the call and then make assertions about the calls themselves.

### snapshots
- [ ] get the system in place for periodic snapshot creation.

### admin commands
- [ ] get AdminCommands setup and implemented.

### observability
- [x] ensure that internal state transitions and updates are emitted for host application use. Such as RaftState changes, membership changes, errors from async ops.
- [x] add mechanism for custom metrics gathering. Should be generic enough that applications should be able to expose the metrics for any metrics gathering platform (prometheus, influx, graphite &c).
- [ ] instrument code with tokio trace: https://docs.rs/tokio-trace/0.1.0/tokio_trace/

### docs
- [ ] add a simple graphic to an `overview` section of the main README to visually demonstrate the layout of this system.
- [ ] put together a networking guide on how to get started with building a networking layer.
- [ ] put together a storage guide on how to implement an application's RaftStorage layer.
- [ ] add docs examples on how to instantiate and start the various actix actor components (Raft, RaftStorage, RaftNetwork &c).
- [ ] add docs on how metrics system works. Pumps out regular metrics configured interval. State changes to the node are always immediately published.
