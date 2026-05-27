# HTTP RaftNetwork V1 Example

This crate is a small HTTP implementation of the legacy `RaftNetwork` V1 API.
It uses `reqwest` for outbound node-to-node RPCs, optional Actix handlers for
inbound RPCs, and JSON for serialization.

For new network implementations, prefer `RaftNetworkV2`. The V2 API is the main
network API in current OpenRaft and gives the application full control over
snapshot transfer through `full_snapshot()`.

See:

- [`raft-kv-memstore-network-v2`](../raft-kv-memstore-network-v2/) for a V2
  network example.
- [`raft-kv-memstore-grpc`](../raft-kv-memstore-grpc/) for a gRPC example that
  implements network sub-traits directly.
- [OpenRaft network guide](../../openraft/src/docs/getting_started/getting-started.md#4-implement-raftnetwork)
  for the trait-level overview.

## What This Crate Shows

`network-v1-http` shows both sides of the older V1 HTTP network:

- `NetworkFactory` builds outbound clients for peer nodes.
- `actix::configure()` registers inbound `/append`, `/vote`, and `/snapshot`
  handlers when the `actix` feature is enabled.

The outbound side implements the older V1 trait:

```rust
impl<C> RaftNetwork<C> for Network<C>
where C: RaftTypeConfig
{
    async fn append_entries(...) -> Result<AppendEntriesResponse<C>, ...>;
    async fn install_snapshot(...) -> Result<InstallSnapshotResponse<C>, ...>;
    async fn vote(...) -> Result<VoteResponse<C>, ...>;
}
```

OpenRaft now expects the factory to produce a V2-compatible network. This crate
therefore wraps the V1 implementation with `into_v2()`:

```rust
impl<C> RaftNetworkFactory<C> for NetworkFactory
where
    C: RaftTypeConfig<Node = BasicNode>,
    C::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
{
    type Network = Adapter<C, Network<C>>;

    async fn new_client(&mut self, target: C::NodeId, node: &BasicNode) -> Self::Network {
        let addr = node.addr.clone();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        Network { addr, client, target }.into_v2()
    }
}
```

The important point is that V1 is adapted into V2 before OpenRaft uses it.

## HTTP Contract

The client and server handlers use these endpoints:

| V1 RPC | HTTP endpoint | Receiver should call |
|--------|---------------|----------------------|
| `append_entries` | `POST /append` | `Raft::append_entries()` |
| `vote` | `POST /vote` | `Raft::vote()` |
| `install_snapshot` | `POST /snapshot` | `Raft::install_snapshot()` |

Requests and responses are JSON. The response body is a serialized
`Result<T, E>`; Raft-level errors are returned in that body.

## Error Mapping

The implementation does not retry RPCs itself. It maps connection failures to
`RPCError::Unreachable`, which lets OpenRaft apply its backoff and retry policy.
Other HTTP/network failures are mapped to `RPCError::Network`.

## Usage

Add the example crate:

```toml
[dependencies]
network-v1-http = { path = "../network-v1-http", features = ["actix"] }
```

Use its factory when constructing `Raft`:

```rust
use network_v1_http::NetworkFactory;

let network = NetworkFactory {};

let raft = openraft::Raft::new(
    node_id,
    config,
    network,
    log_store,
    state_machine,
).await?;
```

Register the inbound handlers in an Actix application:

```rust
use actix_web::web::Data;

let raft_data = Data::new(raft.clone());

actix_web::App::new()
    .app_data(raft_data)
    .configure(network_v1_http::actix::configure::<TypeConfig, StateMachineStore>);
```

Complete examples using this crate:

- [`raft-kv-memstore`](../raft-kv-memstore/)
- [`raft-kv-rocksdb`](../raft-kv-rocksdb/)

## When To Use This Example

Read this crate if you maintain V1 network code or want to understand how the
legacy chunked snapshot API maps into the current V2 network model.

For a new HTTP network implementation, start from
[`raft-kv-memstore-network-v2`](../raft-kv-memstore-network-v2/) instead.
