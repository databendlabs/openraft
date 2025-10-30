### How to get notified when the server state changes?

To monitor the state of a Raft node,
it's recommended to subscribe to updates in the [`RaftMetrics`][]
by calling [`Raft::metrics()`][].

The following code snippet provides an example of how to wait for changes in `RaftMetrics` until a leader is elected:

```ignore
let mut rx = raft.metrics();
loop {
    if let Some(l) = rx.borrow().current_leader {
        return Ok(Some(l));
    }

    rx.changed().await?;
}
```

The second example calls a function when the current node becomes the leader:

```ignore
let mut rx = raft.metrics();
loop {
    if rx.borrow().state == ServerState::Leader {
         app_init_leader();
    }

    rx.changed().await?;
}
```

There is also:
- a [`RaftServerMetrics`][] struct that provides only server/cluster related metrics,
  including node id, vote, server state, current leader, etc.,
- and a [`RaftDataMetrics`][] struct that provides only data related metrics,
  such as log, snapshot, etc.

If you are only interested in server metrics, but not data metrics,
subscribe [`RaftServerMetrics`][] with [`Raft::server_metrics()`][] instead.
For example:

```ignore
let mut rx = raft.server_metrics();
loop {
    if rx.borrow().state == ServerState::Leader {
         app_init_leader();
    }

    rx.changed().await?;
}
```

[`RaftMetrics`]: `crate::metrics::RaftMetrics`
[`Raft::metrics()`]: `crate::Raft::metrics`
[`RaftServerMetrics`]: `crate::metrics::RaftServerMetrics`
[`RaftDataMetrics`]: `crate::metrics::RaftDataMetrics`
[`Raft::server_metrics()`]: `crate::Raft::server_metrics`
