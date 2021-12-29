# Metrics

`Raft` exports metrics on its internal state via `Raft::metrics() -> watch::Receiver<RaftMetrics>`.

`RaftMetrics` contains useful information such as:

- role of this raft node,
- the current leader,
- last, committed, applied log.
- replication state, if this node is a Leader,
- snapshot state,

Metrics can be used as a trigger of application events, as a monitoring data
source, etc.

Metrics is not a stream thus it only guarantees to provide the latest state but
not every change of the state.
Because internally, `watch::channel()` only stores one state.
