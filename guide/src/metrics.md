Metrics
=======
`Raft` exports metrics on a regular interval in order to facilitate maximum observability and integration with metrics aggregations tools (eg, prometheus, influx &c) using the [`RaftMetrics`](https://docs.rs/actix-raft/latest/actix_raft/metrics/struct.RaftMetrics.html) type.

The `Raft` instance constructor expects a `Recipient<RaftMetrics>` (an [actix::Recipient](https://docs.rs/actix/latest/actix/struct.Recipient.html)) to be supplied, and will use this recipient to export its metrics. The `RaftMetrics` type holds the baseline metrics on the state of the Raft node the metrics are coming from, its current role in the cluster, its current membership config, as well as information on the Raft log and the last index to be applied to the state machine.

Applications may use this data in whatever way is needed. The obvious use cases are to expose these metrics to a metrics collection system. Applications may also use this data to trigger events within higher levels of the parent application.

Metrics will be exported at a regular interval according to the [Config.metrics_rate](https://docs.rs/actix-raft/latest/actix_raft/config/struct.Config.html#structfield.metrics_rate) value, but will also emit a new metrics record any time the `state` of the Raft node changes, the `membership_config` changes, or the `current_leader` changes.
