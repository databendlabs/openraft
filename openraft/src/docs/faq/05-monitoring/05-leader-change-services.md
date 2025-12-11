### How to start/stop services when a node becomes leader?

**Problem**: You want to run certain services (e.g., cron jobs, cache warming,
background tasks) only on the leader node, and stop them when the node loses
leadership.

**Solution**: Use [`Raft::on_leader_change()`][] to register a callback that
is invoked whenever the leader changes. The callback receives the old and new
leader state, allowing you to start or stop services accordingly.

```rust,ignore
let my_node_id = 1;
let service_handle = Arc::new(Mutex::new(None));
let service_handle_clone = service_handle.clone();

let mut watch_handle = raft.on_leader_change(move |_old, (leader_id, committed)| {
    let is_leader = leader_id.node_id == my_node_id && committed;

    let mut handle = service_handle_clone.lock().unwrap();
    if is_leader {
        // This node just became the committed leader
        // Start leader-only services
        if handle.is_none() {
            *handle = Some(start_cron_service());
        }
    } else {
        // This node is no longer the leader
        // Stop leader-only services to avoid duplicate work
        if let Some(h) = handle.take() {
            h.shutdown();
        }
    }
});

// Later, when shutting down:
watch_handle.close().await;
```

**Important considerations**:

- **Check `committed` flag**: When `committed == false`, the node has voted for
  itself but hasn't yet received votes from a quorum - it's essentially still a
  **candidate**. Wait for `committed == true` before starting critical services
  to ensure the leadership is stable and acknowledged by the cluster.

- **Idempotent operations**: The callback may be invoked multiple times with
  the same leader. Ensure your start/stop logic is idempotent.

- **Non-blocking callback**: The callback runs in the watch task. Keep it
  lightweight - spawn separate tasks for heavy initialization.

[`Raft::on_leader_change()`]: `crate::Raft::on_leader_change`
