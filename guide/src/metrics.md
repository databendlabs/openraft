Metrics
=======
`Raft` exports metrics on its internal state via the `Raft.metrics` method, which returns a stream of [`RaftMetrics`](TODO:). The metrics themselves describe the state of the Raft node, its current role in the cluster, its current membership config, as well as information on the Raft log and the last index to be applied to the state machine.

Applications may use this data in whatever way is needed. The obvious use cases are to expose these metrics to a metrics collection system, such as Prometheus, TimescaleDB, Influx &c. Applications may also use this data to trigger events within higher levels of the application itself.
