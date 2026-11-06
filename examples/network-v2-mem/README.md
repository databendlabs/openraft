# In-process RaftNetworkV2 Example

This crate implements OpenRaft's `RaftNetworkV2` API for nodes that run in one
process. It sends each RPC by calling a public method of the target's `Raft`
handle, so requests and responses are passed by value and never serialized.

- `Registry` holds the `Raft` handles of one cluster by node ID.
- `NetworkFactory` builds the connections of one node. Get it from
  `Registry::network_factory()` and pass it to `Raft::new()`.
- `Connection` implements `RaftNetworkV2` for one target.

The crate is generic over the type config and the state machine. It depends
only on `openraft`: request types, storage and membership belong to the
application.

## Usage

Create the registry, then register each node after creating it:

```rust
let registry = Arc::new(network_v2_mem::Registry::default());

let network = registry.network_factory(node_id);
let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine).await?;
registry.register(node_id, raft.clone());
```

The application owns the registry. A network factory keeps only a weak
reference to it, because every `Raft` handle owns its network factory. After the
registry is dropped, every RPC fails with `Unreachable`.

## RPC Contract

| V2 RPC            | The target calls                 |
|-------------------|----------------------------------|
| `append_entries`  | `Raft::append_entries()`         |
| `vote`            | `Raft::vote()`                   |
| `pre_vote`        | `Raft::pre_vote()`               |
| `full_snapshot`   | `Raft::install_full_snapshot()`  |
| `transfer_leader` | `Raft::handle_transfer_leader()` |

A connection clones the target's handle while it holds the registry lock, then
releases the lock and awaits the call. A snapshot is passed as the state
machine's own `SnapshotData`.

An RPC fails with `Unreachable` when the target is not registered, or when the
target returns `Fatal` because it has stopped. A call inside one process cannot
time out, so the connection ignores `RPCOption`.

## Fault Injection

`Registry::isolate()` cuts nodes off from every other node, in both directions:
an RPC that starts while its source or its target is isolated fails with
`Unreachable`. An RPC that started before completes. `Registry::heal()`
reconnects every isolated node and returns them.

An isolated node keeps running with its own log and state machine, so healing
reconnects the same node in the state it reached. A leader backs off before it
retries a node that was unreachable. Call `Trigger::reset_backoff()` on the
leader with the reconnected nodes to retry them at once:

```rust
registry.isolate([3]);
// ... writes that node 3 does not receive ...

let healed = registry.heal();
leader.trigger().reset_backoff(healed).await?;
leader.trigger().heartbeat().await?;
```

## Complete Examples

The following examples reuse this crate:

- [`membership`](../membership/)
