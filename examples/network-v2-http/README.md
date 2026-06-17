# HTTP RaftNetworkV2 Example

This crate is a small HTTP implementation of OpenRaft's `RaftNetworkV2` API.
It uses `reqwest` for outbound Raft RPCs, a standalone Hyper server for inbound
Raft RPCs, and JSON for serialization.

The goal is to show the minimum network shape an application needs:

- `NetworkFactory` builds outbound clients for peer nodes.
- `Client` implements `RaftNetworkV2`.
- `Server` listens on the Raft RPC address and forwards inbound RPCs to a local
  `Raft` instance.

This crate only handles node-to-node Raft traffic. It does not provide
application APIs, admin APIs, or routes such as `/write`, `/read`, or
`/change-membership`. Example applications should run their own application
server on a separate address.

## Assumptions

This example keeps the type surface small by assuming:

- `C::Node = openraft::NodeInfo`
- `Client::SnapshotData = Cursor<Vec<u8>>`

`NodeInfo.raft_addr` is used by `NetworkFactory` to contact peer nodes.
`NodeInfo.data` is left to the application. For example, an application
can store its public API address there for leader forwarding or follower-read
helpers.

## Usage

Use `NetworkFactory` when creating `Raft`:

```rust
let network = network_v2_http::NetworkFactory::new();

let raft = openraft::Raft::new(
    node_id,
    config,
    network,
    log_store,
    state_machine,
).await?;
```

Run the Raft RPC server on the internal Raft address:

```rust
let raft_server = network_v2_http::Server::new(raft.clone()).run(raft_addr);
let app_server = run_application_server(api_addr);

tokio::try_join!(raft_server, app_server)?;
```

Use two addresses in the application:

- `api_addr`: public application and management API.
- `raft_addr`: internal Raft RPC API used by OpenRaft nodes.

## HTTP Contract

The client and server use these endpoints:

| V2 RPC | HTTP endpoint | Receiver calls |
|--------|---------------|----------------|
| `append_entries` | `POST /append` | `Raft::append_entries()` |
| `vote` | `POST /vote` | `Raft::vote()` |
| `full_snapshot` | `POST /snapshot` | `Raft::install_full_snapshot()` |
| `transfer_leader` | `POST /transfer-leader` | `Raft::handle_transfer_leader()` |

Requests and responses are JSON. The response body is a serialized
`Result<T, RaftError<C>>`; Raft-level errors are returned in that body.

For simplicity, snapshots are encoded as one JSON request containing the vote,
snapshot metadata, and snapshot bytes. A production network may choose a
streaming format instead.

## Error Mapping

The client does not retry RPCs itself. Connection failures are mapped to
`RPCError::Unreachable`, letting OpenRaft apply its normal backoff behavior.
Other HTTP and decode failures are mapped to `RPCError::Network`.

The server returns:

- `400 Bad Request` for invalid request bodies.
- `404 Not Found` for unsupported methods or paths.
- `500 Internal Server Error` if a response cannot be serialized.

## Complete Examples

The following examples reuse this crate:

- [`raft-kv-memstore`](../raft-kv-memstore/)
- [`raft-kv-rocksdb`](../raft-kv-rocksdb/)
