# Legacy V1 Network Example

This example runs a KV application on the legacy `RaftNetwork` V1 API. It
reuses [`network-v1-http`](../network-v1-http/) for node-to-node Raft RPCs,
which wraps its V1 implementation with `openraft-legacy`'s `Adapter` so current
OpenRaft can drive it.

For a new application, prefer [`raft-kv-memstore`](../raft-kv-memstore/): it is
the same KV app on the current `RaftNetworkV2` API, so diffing the two shows
exactly what the V1 network changes and what it does not.

The application code only keeps the pieces specific to the example:

- in-memory log storage
- in-memory state machine storage
- application HTTP read handlers
- a cluster test that forces snapshot replication to a learner

## Network Layers

Raft RPC traffic is handled by `network-v1-http`:

- `NetworkFactory` creates outbound Raft RPC clients, adapted from V1 to V2.
- `Server` receives inbound `/append`, `/vote`, and `/snapshot` requests.

Application traffic is handled by [`app-http`](../app-http/). This example
calls `add_openraft_routes()` for the common OpenRaft application endpoints,
then adds its read endpoints.

## Where V1 And V2 Differ

Leader election and log replication look the same either way; the adapter
forwards them to the V1 trait unchanged.

Snapshot transfer is the real difference. V2 hands the whole snapshot to
`full_snapshot()` and lets the application stream it. V1 has no such call, so
the adapter splits the snapshot into `InstallSnapshotRequest` chunks, and the
receiving end reassembles them through
`ChunkedSnapshotReceiver::install_snapshot()` before installing the result.

That is why this example's test drives a learner that can only catch up from a
snapshot: it is the one path where the V1 wiring is doing something a V2
network would not.

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
6. Verifies that node 2 receives the snapshot, chunk by chunk, over V1.

For the reusable network implementation, see
[`network-v1-http`](../network-v1-http/).
