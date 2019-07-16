todo
====
### algorithm optimizations
- [ ] update the AppendEntries RPC receiver to pipeline all requests. This will potentially remove the need for having to use boolean flags to mark if the Raft is currently appending logs or not.
- [ ] in the client request handling logic, after logs have been appended locally and have been queued for replication on the streams, the pipeline can immediately begin processing the next payload of entries. **This optimization will greatly increse the throughput of this Raft implementation.**
    - this will mean that the process of waiting for the logs to be committed to the cluster and responding to the client will happen outside of the main pipeline.
    - multiple payloads of enntries will be queued and delivered at the same time from replication streams to followers, so batches of requests may become ready for further processing/response when under heavy write load.
    - applying the logs to the state machine will also happen outside of the pipeline. It must be ensured that entries are not sent to the state machine to be applied out-of-order. It is absolutely critical that they are sent to be applied in index order.
    - the pipeline for applying logs to the statemachine should also perform batching whenever possible.

### testing
- [ ] finish implement MemoryStroage for testing (and general demo usage).
- [ ] setup testing framework to assert accurate behavior of Raft implementation and adherence to Raft's safety protocols.
- [ ] all actor based. Transport layer can be a simple message passing mechanism.

### snapshots
- [ ] get the system in place for periodic snapshot creation.

### admin commands
- [ ] get AdminCommands setup and implemented.

### observability
- [ ] ensure that internal state transitions and updates are emitted for host application use. Such as RaftState changes, membership changes, errors from async ops.
- [ ] add mechanism for custom metrics gathering. Should be generic enough that applications should be able to expose the metrics for any metrics gathering platform (prometheus, influx, graphite &c).

### docs
- [ ] add a simple graphic to an `overview` section of the main README to visually demonstrate the layout of this system.
- [ ] put together a networking guide on how to get started with building a networking layer.
- [ ] put together a storage guide on how to implement an application's RaftStorage layer.
- [ ] add docs examples on how to instantiate and start the various actix actor components (Raft, RaftStorage, RaftNetwork &c).
