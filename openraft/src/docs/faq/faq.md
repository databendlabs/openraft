## General


### What are the differences between Openraft and standard Raft?

- Optionally, In one term there could be more than one leader to be established, in order to reduce election conflict. See: std mode and adv mode leader id: [`leader_id`][];
- Openraft stores committed log id: See: [`RaftLogStorage::save_committed()`][];
- Openraft optimized `ReadIndex`: no `blank log` check: [`Linearizable Read`][].
- A restarted Leader will stay in Leader state if possible;
- Does not support single step membership change. Only joint is supported.


## Observation and Management


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


## Data structure


### Why is log id a tuple of `(term, node_id, log_index)`?

In standard Raft log id is `(term, log_index)`, in Openraft he log id `(term,
node_id, log_index)` is used to minimize the chance of election conflicts.
This way in every term there could be more than one leader elected, and the last one is valid.
See: [`leader-id`](`crate::docs::data::leader_id`) for details.


## Replication


### How does Openraft handle snapshot building and transfer?

Openraft calls [`RaftStateMachine::get_snapshot_builder`][] to create snapshots. The builder runs
concurrently with [`RaftStateMachine::apply`][], so your implementation must handle concurrent access
to the state machine data.

When a follower is more than [`Config::replication_lag_threshold`][] entries behind, the leader
sends a snapshot instead of individual log entries.

For large snapshots that timeout during transfer, increase [`Config::install_snapshot_timeout`][].
The snapshot is sent in chunks of [`Config::snapshot_max_chunk_size`][] bytes.


### How to minimize error logging when a follower is offline

Excessive error logging, like `ERROR openraft::replication: 248: RPCError err=NetworkError: ...`, occurs when a follower node becomes unresponsive. To alleviate this, implement a mechanism within [`RaftNetwork`][] that returns a [`Unreachable`][] error instead of a [`NetworkError`][] when immediate replication retries to the affected node are not advised.


### How to detect which nodes are currently down or unreachable?

To monitor node availability in your Raft cluster, use [`RaftMetrics`][] from
the leader node via [`Raft::metrics()`][]. This provides real-time visibility
into node reachability without requiring membership changes.

There are two primary approaches to detect unreachable nodes:

**Method 1: Monitor replication lag**
Check the field [`RaftMetrics::replication`][], which contains a
`BTreeMap<NodeId, Option<LogId>>` showing the last replicated log for each node.
If a node's replication significantly lags behind
[`RaftMetrics::last_log_index`][], it indicates replication issues and the node
may be down.

**Method 2: Monitor heartbeat timestamps (since OpenRaft 0.10)**
Use the field [`RaftMetrics::heartbeat`][], which stores `BTreeMap<NodeId, Option<SerdeInstant>>`
containing the timestamp of the last acknowledgment from each node. If a
timestamp is significantly behind the current time, the node is likely
unreachable.

Both methods provide "unreachable from leader" perspective, which is typically
what matters for cluster health monitoring. This approach allows you to maintain
a list of active nodes without modifying cluster membership.


## Troubleshooting


### Panic: "assertion failed: self.internal_server_state.is_following()"

**Symptom**: Node crashes with `panicked at 'assertion failed: self.internal_server_state.is_following()'`

**Cause**: [`RaftNetworkFactory`][] creates a connection from a node to itself. When this node
becomes leader, it sends replication messages to itself, but Openraft expects only followers to
receive replication messages.

**Solution**: In [`RaftNetworkFactory::new_client`][], ensure the target node ID never equals
the local node's ID. Each node ID in [`Membership`][] must map to a different node.


### Write returns `ForwardToLeader` but leader info is missing

**Symptom**: [`ClientWriteError::ForwardToLeader`][] is returned but the `leader_id` field is `None`

**Cause**: The current leader is no longer in the cluster's membership configuration.
This occurs after a membership change that removes the leader node.

**Solution**: If `leader_id` is `None`, query [`RaftMetrics::current_leader`][] to find the new
leader, or retry the membership query after the new leader is elected.


### Slow replication performance with RocksDB or disk storage

**Symptom**: Write throughput is much lower than expected when using disk-based [`RaftLogStorage`][]

**Cause**: Synchronous writes to disk block the Raft thread. Additionally, HTTP client connection
pooling (in libraries like `reqwest`) can add 40ms+ latency spikes.

**Solution**:
- Use non-blocking I/O in your [`RaftLogStorage::append`][] implementation
- Consider batching writes in your storage layer
- For network layer, prefer WebSocket or connection pooling that doesn't introduce latency
- With RocksDB: disable `sync` on writes if you can tolerate some data loss on crash


### Frequent leader elections and timeouts

**Symptom**: Logs show repeated leader elections, or [`RaftMetrics::current_leader`][] changes frequently

**Cause**: [`Config::election_timeout_min`][] is too small for your storage or network latency.
If [`RaftLogStorage::append`][] takes longer than the election timeout, heartbeats time out and
trigger elections.

**Solution**: Increase both [`Config::election_timeout_min`][] and [`Config::election_timeout_max`][].
Ensure `heartbeat_interval < election_timeout_min / 2` and that election timeout is at least
10× your typical [`RaftLogStorage::append`][] latency.


### Excessive "RPCError err=NetworkError" in logs when a node is offline

**Symptom**: Continuous error logs `ERROR openraft::replication: RPCError err=NetworkError`
when a follower is unreachable

**Cause**: Openraft retries replication aggressively. Each failed RPC logs an error.

**Solution**: In your [`RaftNetwork`][] implementation, when a node is known to be unreachable,
return [`Unreachable`][] error instead of [`NetworkError`][]. Openraft backs off longer for
`Unreachable` errors, reducing log spam.


### Holding `Raft::metrics()` reference blocks the Raft node

**Symptom**: Raft node appears frozen or unresponsive. In single-threaded runtimes, everything stops.

**Cause**: When using `tokio::watch` as [`AsyncRuntime::Watch`][], the `Ref` returned by
`borrow_watched()` holds a synchronous `RwLock` read guard. If you hold this `Ref` while
Openraft tries to send new metrics (needs write lock), it deadlocks.

**Solution**: Clone the metrics immediately to drop the `Ref`. Never hold the `Ref` across operations.

```rust
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


### Error logs after `raft.shutdown()` completes

**Symptom**: After calling [`Raft::shutdown`][] which returns successfully, logs show
`ERROR openraft::raft::raft_inner: failure sending RaftMsg to RaftCore; message: AppendEntries ... core_result=Err(Stopped)`

**Cause**: Other nodes in the cluster continue sending RPCs to this node. The `Raft` handle still
exists and receives these RPCs, but the internal Raft core has stopped, so forwarding fails.

**Solution**: This is expected behavior. These errors are harmless - they indicate the node has
shut down as requested. You can ignore them or filter these specific error logs after shutdown.

See: <https://github.com/databendlabs/openraft/issues/1357>


## Node management


### How to customize snapshot-building policy?

OpenRaft provides a default snapshot building policy that triggers snapshots
when the log count exceeds a threshold. Configure this via [`Config::snapshot_policy`] 
set to [`SnapshotPolicy::LogsSinceLast(n)`][`SnapshotPolicy::LogsSinceLast`].

To customize snapshot behavior:

- **Disable automatic snapshots**: Set [`Config::snapshot_policy`] to [`SnapshotPolicy::Never`]
- **Manual snapshot triggers**: Use [`Raft::trigger().snapshot()`][`Trigger::snapshot`] to build snapshots on demand

This allows full control over when snapshots are created based on your application's specific requirements.


## Cluster management


### How to initialize a cluster?

There are two ways to initialize a raft cluster, assuming there are three nodes,
`n1, n2, n3`:

1. Single-step method:
   Call `Raft::initialize()` on any one of the nodes with the configuration of
   all three nodes, e.g. `n2.initialize(btreeset! {1,2,3})`.

2. Incremental method:
   First, call `Raft::initialize()` on `n1` with configuration containing `n1`
   itself, e.g., `n1.initialize(btreeset! {1})`.
   Subsequently use `Raft::change_membership()` on `n1` to add `n2` and `n3`
   into the cluster.

Employing the second method provides the flexibility to start with a single-node
cluster for testing purposes and subsequently expand it to a three-node cluster
for deployment in a production environment.


### Are there any issues with running a single node service?

Not at all.

Running a cluster with just one node is a standard approach for testing or as an initial step in setting up a cluster.

A single node functions exactly the same as cluster mode.
It will consistently maintain the `Leader` status and never transition to `Candidate` or `Follower` states.


### How do I store additional information about nodes in Openraft?

By default, Openraft provide a [`BasicNode`] as the node type in a cluster.
To store more information about each node in Openraft, define a custom struct
with the desired fields and use it in place of `BasicNode`. Here's a brief
guide:

1. Define your custom node struct:

```rust,ignore
#[derive(...)]
struct MyNode {
    ipv4: String,
    ipv6: String,
    port: u16,
    // Add additional fields as needed
}
```

2. Register the custom node type with `declare_raft_types!` macro:

```rust,ignore
openraft::declare_raft_types!(
   pub MyRaftConfig:
       // ...
       NodeId = u64,        // Use the appropriate type for NodeId
       Node = MyNode,       // Replace BasicNode with your custom node type
       // ... other associated types
);
```

Use `MyRaftConfig` in your Raft setup to utilize the custom node structure.


### What actions are required when a node restarts?

None. No calls, e.g., to either [`add_learner()`][] or [`change_membership()`][]
are necessary.

Openraft maintains the membership configuration in [`Membership`][] for all
nodes in the cluster, including voters and non-voters (learners).  When a
`follower` or `learner` restarts, the leader will automatically re-establish
replication.


### What will happen when data gets lost?

Raft operates on the presumption that the storage medium (i.e., the disk) is
secure and reliable.

If this presumption is violated, e.g., the raft logs are lost or the snapshot is
damaged, no predictable outcome can be assured. In other words, the resulting
behavior is **undefined**.


### Can I wipe out the data of ONE node and wait for the leader to replicate all data to it again?

Avoid doing this. Doing so will panic the leader. But it is permitted
with config [`Config::allow_log_reversion`] enabled.

In a raft cluster, although logs are replicated to multiple nodes,
wiping out a node and restarting it is still possible to cause data loss.
Assumes the leader is `N1`, followers are `N2, N3, N4, N5`:
- A log(`a`) that is replicated by `N1` to `N2, N3` is considered committed.
- At this point, if `N3` is replaced with an empty node, and at once the leader
  `N1` is crashed. Then `N5` may elected as a new leader with vote granted by
  `N3, N4`;
- Then the new leader `N5` will not have log `a`.

```text
Ni: Node i
Lj: Leader   at term j
Fj: Follower at term j

N1 | L1  a  crashed
N2 | F1  a
N3 | F1  a  erased          F2
N4 |                        F2
N5 |                 elect  L2
----------------------------+---------------> time
                            Data loss: N5 does not have log `a`
```

But for even number nodes cluster, Erasing **exactly one** node won't cause data loss.
Thus, in a special scenario like this, or for testing purpose, you can use
[`Config::allow_log_reversion`] to permit erasing a node.


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


## Membership config


### How to remove node-2 safely from a cluster `{1, 2, 3}`?

Call `Raft::change_membership(btreeset!{1, 3})` to exclude node-2 from
the cluster. Then wipe out node-2 data.
**NEVER** modify/erase the data of any node that is still in a raft cluster, unless you know what you are doing.


### Does OpenRaft support calling `change_membership` in parallel?

Yes, OpenRaft does support this scenario, but with some important caveats.
`change_membership` is a two-step process—first transitioning to a joint
configuration and then to a final uniform configuration. When multiple
`change_membership` calls occur concurrently, their steps can interleave,
potentially leaving the cluster in a joint config state. This state is valid in
OpenRaft.

Here's an example of how such interleaving might play out:

1. **Initial State**: `{y}`
2. **Task 1** calls `change_membership(AddVoters(x))`
   → Transitions to joint config `[{y}, {y, x}]`
3. **Task 2** calls `change_membership(RemoveVoters(x))`
   → Transitions from `[{y}, {y, x}]` to `[{y, x}, {y}]`
4. **Task 2 (Step 2)** finalizes config to `{y}` and reports success to its client
5. **Task 1 (Step 2)** proceeds unaware, adding `x` again
   → Transitions from `{y}` to `[{y}, {y, x}]`

At this point, both tasks report success to their respective clients, but the
cluster is left in a **joint configuration** state: `[{y}, {y, x}]`.

This illustrates how concurrent changes can lead to a final configuration that
may not reflect the full intent of either request. However, this does **not**
violate OpenRaft’s consistency model, and joint configs are treated as valid
operating states.

If your system ensures only one process issues `change_membership` at a time,
you're safe. If not, always validate the final config after changes to avoid
surprises.
For example, if two requests attempt to change the config to `{a,b,c}` and
`{x,y,z}` respectively, the result may end up being one or the
other—unpredictable from each individual caller’s perspective.

This behavior is by design, as it provides several advantages:

- **Simplifies recovery**: If the leader crashes after the first step, a new
  leader can continue operating from the joint state without requiring special
  recovery procedures.

- **Supports dynamic flexibility**: The cluster can adapt to changing conditions
  without being locked into a specific membership transition.

OpenRaft intentionally supports this behavior because:

- When a leader crashes after establishing a joint configuration, the new leader
  can seamlessly resume from this state and process new membership changes as
  needed.

- The system can adapt if nodes in the target configuration become unstable. For
  example, if transitioning from `{a,b,c}` to `{b,c,d}` but node `d` becomes
  unreliable when it finished phase-1 and the config is `[{a,b,c}, {b,c,d}]`,
  the leader can pivot to include a different node (e.g., change membership
  config to `[{a,b,c}, {b,c,x}]` then to `{b,c,x}`) or revert to the original
  configuration `{a,b,c}`.


[`Linearizable Read`]: `crate::docs::protocol::read`
[`leader_id`]:         `crate::docs::data::leader_id`

[`BasicNode`]:        `crate::node::BasicNode`
[`RaftTypeConfig`]:   `crate::RaftTypeConfig`
[`AsyncRuntime::Watch`]: `crate::AsyncRuntime::Watch`

[`Config`]: `crate::config::Config`
[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
[`Config::election_timeout_min`]: `crate::config::Config::election_timeout_min`
[`Config::election_timeout_max`]: `crate::config::Config::election_timeout_max`
[`Config::heartbeat_interval`]: `crate::config::Config::heartbeat_interval`
[`Config::install_snapshot_timeout`]: `crate::config::Config::install_snapshot_timeout`
[`Config::replication_lag_threshold`]: `crate::config::Config::replication_lag_threshold`
[`Config::snapshot_max_chunk_size`]: `crate::config::Config::snapshot_max_chunk_size`
[`Config::snapshot_policy`]: `crate::config::Config::snapshot_policy`

[`SnapshotPolicy::LogsSinceLast`]: `crate::config::SnapshotPolicy::LogsSinceLast`
[`SnapshotPolicy::Never`]: `crate::config::SnapshotPolicy::Never`

[`RaftLogStorage`]: `crate::storage::RaftLogStorage`
[`RaftLogStorage::append`]: `crate::storage::RaftLogStorage::append`
[`RaftLogStorage::save_committed()`]: `crate::storage::RaftLogStorage::save_committed`

[`RaftStateMachine`]: `crate::storage::RaftStateMachine`
[`RaftStateMachine::apply`]: `crate::storage::RaftStateMachine::apply`
[`RaftStateMachine::get_snapshot_builder`]: `crate::storage::RaftStateMachine::get_snapshot_builder`

[`RaftNetwork`]: `crate::network::RaftNetwork`
[`RaftNetworkFactory`]: `crate::network::RaftNetworkFactory`
[`RaftNetworkFactory::new_client`]: `crate::network::RaftNetworkFactory::new_client`

[`add_learner()`]: `crate::Raft::add_learner`
[`change_membership()`]: `crate::Raft::change_membership`

[`Membership`]: `crate::Membership`

[`Unreachable`]: `crate::error::Unreachable`
[`NetworkError`]: `crate::error::NetworkError`
[`Fatal::StorageError`]: `crate::error::Fatal::StorageError`
[`ClientWriteError::ForwardToLeader`]: `crate::error::ClientWriteError::ForwardToLeader`


[`RaftMetrics`]: `crate::metrics::RaftMetrics`
[`RaftMetrics::current_leader`]: `crate::metrics::RaftMetrics::current_leader`
[`RaftMetrics::heartbeat`]: `crate::metrics::RaftMetrics::heartbeat`
[`RaftMetrics::last_log_index`]: `crate::metrics::RaftMetrics::last_log_index`
[`RaftMetrics::replication`]: `crate::metrics::RaftMetrics::replication`
[`Raft::metrics`]: `crate::Raft::metrics`
[`Raft::shutdown`]: `crate::Raft::shutdown`
[`RaftServerMetrics`]: `crate::metrics::RaftServerMetrics`
[`RaftDataMetrics`]: `crate::metrics::RaftDataMetrics`
[`Raft::metrics()`]: `crate::Raft::metrics`
[`Raft::server_metrics()`]: `crate::Raft::server_metrics`
[`Raft::data_metrics()`]: `crate::Raft::data_metrics`

[`Trigger::snapshot`]: `crate::raft::trigger::Trigger::snapshot`
