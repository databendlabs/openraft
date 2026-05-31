# RaftNetworkV2 Snapshot Example

This example shows snapshot replication with OpenRaft's `RaftNetworkV2` API.
It reuses [`network-v2-http`](../network-v2-http/) for node-to-node Raft RPCs,
instead of defining another network implementation in the application crate.

The application code only keeps the pieces specific to the example:

- in-memory log storage
- in-memory state machine storage
- a small in-process router for test application and management calls
- a cluster test that forces snapshot replication to a learner

## Network Layer

Raft RPC traffic is handled by `network-v2-http`:

- `NetworkFactory` creates outbound Raft RPC clients.
- `Server` receives inbound `/append`, `/vote`, `/snapshot`, and
  `/transfer-leader` requests.

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
2. Runs a standalone Raft RPC server for each node.
3. Initializes node 1 as a single-node cluster.
4. Writes data and triggers a snapshot on node 1.
5. Adds node 2 as a learner.
6. Verifies that node 2 receives the snapshot.

For the reusable network implementation, see
[`network-v2-http`](../network-v2-http/).
