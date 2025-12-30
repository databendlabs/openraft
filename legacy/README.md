# openraft-legacy

Legacy compatibility layer for deprecated Openraft APIs.

This crate provides backward compatibility for applications using deprecated Openraft APIs. Instead of modifying application code, users can switch imports from the main `openraft` crate to this crate.

## Available Legacy APIs

### Network V1 (`network_v1` module)

The legacy `RaftNetwork` v1 trait with chunk-based snapshot transmission:

- **`RaftNetwork`**: The legacy v1 network trait using `install_snapshot()` for chunked snapshot transfer
- **`Adapter`**: Converts a v1 `RaftNetwork` impl to `RaftNetworkV2`
- **`ChunkedRaft`**: Wraps `Raft` to accept v1 chunk-based `install_snapshot` RPCs

## Usage

### Client-side: `Adapter`

Wraps a v1 `RaftNetwork` to provide `RaftNetworkV2`:

```rust
use openraft_legacy::network_v1::{Adapter, RaftNetwork};

impl RaftNetwork<C> for MyNetwork {
    async fn install_snapshot(...) { /* chunk-based */ }
    async fn append_entries(...) { ... }
    async fn vote(...) { ... }
}

impl RaftNetworkFactory<C> for MyFactory {
    type Network = Adapter<C, MyNetwork>;

    async fn new_client(...) -> Self::Network {
        Adapter::new(MyNetwork::new(...))
    }
}
```

### Server-side: `ChunkedRaft`

Wraps `Raft` to accept chunk-based snapshot RPCs. Derefs to inner `Raft`:

```rust
use openraft_legacy::network_v1::ChunkedRaft;

let raft = ChunkedRaft::new(Raft::new(...));

raft.client_write(cmd).await?;      // via Deref
raft.install_snapshot(req).await?;  // chunk-based
```
