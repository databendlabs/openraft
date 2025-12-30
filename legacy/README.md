# openraft-legacy

Legacy compatibility layer for deprecated Openraft APIs.

This crate provides backward compatibility for applications using deprecated Openraft APIs. Instead of modifying application code, users can switch imports from the main `openraft` crate to this crate.

## Quick Start

Use the prelude to import all commonly used legacy types:

```rust
use openraft_legacy::prelude::*;
```

## Available Legacy APIs

### Network V1 (`network_v1` module)

The legacy `RaftNetwork` v1 trait with chunk-based snapshot transmission:

- **`RaftNetwork`**: The legacy v1 network trait using `install_snapshot()` for chunked snapshot transfer
- **`Adapter`**: Converts a v1 `RaftNetwork` impl to `RaftNetworkV2`
- **`ChunkedSnapshotReceiver`**: Extension trait that adds `install_snapshot()` to `Raft` for receiving chunks

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

### Server-side: `ChunkedSnapshotReceiver`

Extension trait that adds `install_snapshot()` to `Raft`. Just import the trait:

```rust
use openraft_legacy::network_v1::ChunkedSnapshotReceiver;

let raft = Raft::new(...).await?;

raft.client_write(cmd).await?;      // standard Raft method
raft.install_snapshot(req).await?;  // added by ChunkedSnapshotReceiver trait
```
