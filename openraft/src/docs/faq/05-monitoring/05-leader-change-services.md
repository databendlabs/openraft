### How to start/stop services when a node becomes leader?

**Problem**: You want to run certain services (e.g., cron jobs, cache warming,
background tasks) only on the leader node, and stop them when the node loses
leadership.

**Solution**: Use [`Raft::on_leader_change()`][] to register callbacks that
are invoked when this node becomes or stops being the leader.

```rust,ignore
let service_handle = Arc::new(Mutex::new(None));
let service_handle_clone = service_handle.clone();

let mut watch_handle = raft.on_leader_change(
    // start: called when this node becomes leader
    move |_leader_id| {
        let service_handle = service_handle_clone.clone();
        async move {
            let mut handle = service_handle.lock().unwrap();
            if handle.is_none() {
                *handle = Some(start_cron_service().await);
            }
        }
    },
    // stop: called when this node is no longer leader
    move |_old_leader_id| {
        let service_handle = service_handle.clone();
        async move {
            let mut handle = service_handle.lock().unwrap();
            if let Some(h) = handle.take() {
                h.shutdown().await;
            }
        }
    },
);

// Later, when shutting down:
watch_handle.close().await;
```

For more fine-grained control over all leader changes in the cluster (not just
this node), use [`Raft::on_cluster_leader_change()`][] instead.

**Important considerations**:

- **Committed leadership**: `on_start` only fires when the node is the
  committed leader (acknowledged by a quorum), not when it's still a candidate.

- **Alternating callbacks**: The `start` and `stop` callbacks are guaranteed
  to be called in alternating order: `start` → `stop` → `start` → `stop` → ...
  Even if a node transitions directly from leader in Term 1 to leader in Term 2,
  `stop` will be called with the old `leader_id` before `start` is called with
  the new one. This ensures proper resource cleanup between leadership terms.

- **Idempotent operations**: Ensure your start/stop logic is idempotent.

- **Async callbacks**: Callbacks return a `BoxFuture` that is awaited before
  processing the next vote change. You can use `Box::pin(async { ... })` to
  create async callbacks that perform I/O operations.

[`Raft::on_leader_change()`]: `crate::Raft::on_leader_change`
[`Raft::on_cluster_leader_change()`]: `crate::Raft::on_cluster_leader_change`
