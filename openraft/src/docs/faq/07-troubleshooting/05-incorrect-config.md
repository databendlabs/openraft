### Is Openraft resilient to incorrectly configured clusters?

No, Openraft, like standard raft, cannot identify errors in cluster configuration.

A common error is the assigning incorrect network addresses to a node. In such
a scenario, if this node becomes the leader, it will attempt to replicate
logs to itself. This will cause Openraft to panic because replication
messages can only be received by a follower.

```text
thread 'main' panicked at openraft/src/engine/engine_impl.rs:793:9:
assertion failed: self.internal_server_state.is_following()
```

```ignore
// openraft/src/engine/engine_impl.rs:793
pub(crate) fn following_handler(&mut self) -> FollowingHandler<C> {
    debug_assert!(self.internal_server_state.is_following());
    // ...
}
```
