# RaftNetworkV2 Snapshot Example

This example shows snapshot replication with OpenRaft's `RaftNetworkV2` API.
It reuses [`network-v2-http`](../network-v2-http/) for node-to-node Raft RPCs,
instead of defining another network implementation in the application crate.

The application code only keeps the pieces specific to the example:

- in-memory log storage
- in-memory state machine storage
- application HTTP read handlers
- a cluster test that forces snapshot replication to a learner

## Network Layers

Raft RPC traffic is handled by `network-v2-http`:

- `NetworkFactory` creates outbound Raft RPC clients.
- `Server` receives inbound `/append`, `/vote`, `/snapshot`, and
  `/transfer-leader` requests.

Application traffic is handled by [`app-http`](../app-http/). This example
calls `add_openraft_routes()` for the common OpenRaft application endpoints,
then adds its read endpoints.

The example application does not implement `RaftNetworkV2` directly. This keeps
the snapshot test focused on how OpenRaft uses the network, not on another copy
of HTTP client and server code.

## Running

```bash
cargo test -- --nocapture
```

## What The Test Covers

The cluster test:

1. Starts two Raft nodes.
2. Runs a Raft RPC server and application server for each node.
3. Initializes node 1 as a single-node cluster through the application client.
4. Writes data through the application client and triggers a snapshot on node 1.
5. Adds node 2 as a learner through the application client.
6. Verifies that node 2 receives the snapshot.

For the reusable network implementation, see
[`network-v2-http`](../network-v2-http/).
