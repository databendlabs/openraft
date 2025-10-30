### Holding `Raft::metrics()` reference blocks the Raft node

**Symptom**: Raft node appears frozen or unresponsive. In single-threaded runtimes, everything stops.

**Cause**: When using `tokio::watch` as [`AsyncRuntime::Watch`][], the `Ref` returned by
`borrow_watched()` holds a synchronous `RwLock` read guard. If you hold this `Ref` while
Openraft tries to send new metrics (needs write lock), it deadlocks.

**Solution**: Clone the metrics immediately to drop the `Ref`. Never hold the `Ref` across operations.

```rust,ignore
// Bad - holds the Ref guard, blocks flush_metrics()
let rx = raft.metrics();
let metrics_ref = rx.borrow_watched(); // Ref guard held
do_something().await; // Openraft may deadlock trying to flush_metrics()
drop(metrics_ref);

// Good - clone immediately, Ref guard dropped right away
let metrics = raft.metrics().borrow_watched().clone();
do_something().await; // Safe
```

Deadlock scenario:
```text
// Task 1 (on thread A)          |  // Task 2 (on thread B)
let _ref1 = rx.borrow_watched(); |
                                 |  // flush_metrics() blocks
                                 |  let _ = tx.send(());
// may deadlock                  |
let _ref2 = rx.borrow_watched(); |
```

See: <https://github.com/databendlabs/openraft/issues/1238>

[`AsyncRuntime::Watch`]: `crate::AsyncRuntime::Watch`
