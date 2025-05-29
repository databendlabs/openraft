# Network-v1-HTTP

An OpenRaft network layer implementation example that demonstrates the **RaftNetwork interface V1** using HTTP and `reqwest` as the transport mechanism.

## Role and Position in OpenRaft

`network-v1` demonstrates how to implement `RaftNetwork` and `RaftNetworkFactory` traits (**interface V1**) to **enable Raft nodes to establish connections and exchange messages with other nodes** for consensus operations.

### Position in OpenRaft Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│                (Business Applications)                      │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│                Complete Example Applications                │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│                   OpenRaft Core                             │
│              (consensus algorithm & coordinator)            │
│                                                             │
│  ┌─────────────────┐                    ┌─────────────────┐ │
│  │   Storage       │                    │    Network      │ │
│  │  Management     │◄──────────────────►│   Management    │ │
│  │                 │                    │                 │ │
│  └────────┬────────┘                    └────────┬────────┘ │
└───────────│──────────────────────────────────────│──────────┘
            │                                      │
            ▼                                      ▼
┌─────────────────────┐                 ┌─────────────────────┐
│   Storage Layer     │                 │   Network Layer     │
│                     │                 │                     │
│ • LogStore          │                 │ • RaftNetwork       │
│ • StateMachine      │                 │ • RaftNetworkFactory│
│ • SnapshotStore     │                 │                     │
│                     │                 │ ┌─────────────────┐ │
│ Implementations:    │                 │ │   network-v1    │ │
│ • RocksDB           │                 │ │ (Interface v1)  │ │
│ • Memory            │                 │ │                 │ │
│ • Custom stores     │                 │ │   network-v2    │ │
│                     │                 │ │ (Interface v2)  │ │
│                     │                 │ └─────────────────┘ │
└─────────────────────┘                 └─────────────────────┘
```

### Core Functions

- **Send three RPCs to other nodes**: vote, append_entries, and install_snapshot
- **Demonstrate Interface V1**: Shows how to implement the RaftNetwork V1 programming interface

### Core Technology Stack

- **HTTP Protocol**: Implemented using `reqwest`
- **Serialization**: Uses `serde` + JSON

### Implemented Traits

#### 1. `RaftNetworkFactory<C>`
```rust
impl<C> RaftNetworkFactory<C> for NetworkFactory
where
    C: RaftTypeConfig<Node = BasicNode>,
    <C as RaftTypeConfig>::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
{
    type Network = Network<C>;

    async fn new_client(&mut self, target: C::NodeId, node: &BasicNode) -> Self::Network;
}
```

**Responsibilities**:
- **Create network client instances for each target node** that this Raft node needs to communicate with
- **Enable the Raft node to establish outbound connections** to specific remote nodes in the cluster
- Manage HTTP client configuration and initialization **for inter-node communication**

#### 2. `RaftNetwork<C>`
```rust
impl<C> RaftNetwork<C> for Network<C>
where C: RaftTypeConfig
{
    async fn append_entries(&mut self, req: AppendEntriesRequest<C>, option: RPCOption) -> Result<...>;
    async fn install_snapshot(&mut self, req: InstallSnapshotRequest<C>, option: RPCOption) -> Result<...>;
    async fn vote(&mut self, req: VoteRequest<C>, option: RPCOption) -> Result<...>;
}
```

**Responsibilities**:
- **Send raft protocol messages from this node to a specific remote node**
- Implement the three core RPC calls of the raft protocol **for inter-node communication**
- Handle network errors and retry logic **when communicating with remote nodes**

### Error Handling

The implementation properly handles network errors and maps them to OpenRaft's error types:

```rust
let resp = self.client.post(url).json(&req).send().await.map_err(|e| {
    if e.is_connect() {
        // Connection errors trigger OpenRaft's backoff mechanism
        RPCError::Unreachable(Unreachable::new(&e))
    } else {
        // Other network errors
        RPCError::Network(NetworkError::new(&e))
    }
})?;
```

**Error Types**:
- `Unreachable`: Connection failures (triggers backoff retry)
- `NetworkError`: Other HTTP/network errors

### HTTP Protocol Requirements

#### Protocol Description

This implementation expects the remote server to implement these HTTP endpoints:

| Raft RPC Method | HTTP Endpoint | Request Type | Response Type | Description |
|-----------------|---------------|--------------|---------------|-------------|
| `append_entries` | `POST /append` | `AppendEntriesRequest` | `Result<AppendEntriesResponse, Error>` | **Node-to-node** log replication and heartbeat messages |
| `vote` | `POST /vote` | `VoteRequest` | `Result<VoteResponse, Error>` | **Node-to-node** leader election voting requests |
| `install_snapshot` | `POST /snapshot` | `InstallSnapshotRequest` | `Result<InstallSnapshotResponse, Error>` | **Node-to-node** snapshot installation for catching up lagging nodes |

**Requirements:**
- All requests: JSON-serialized POST with `Content-Type: application/json`
- All responses: HTTP 200 OK with JSON-serialized `Result<T, E>` body
- Raft-level errors in response body, not HTTP status codes

#### Protocol Examples

**Request Examples:**

```bash
# append_entries request
POST http://127.0.0.1:21001/append
Content-Type: application/json

{
  "vote": {"term": 1, "node_id": 1},
  "prev_log_id": {"term": 0, "index": 0},
  "entries": [...],
  "leader_commit": {"term": 1, "index": 5}
}
```

```bash
# vote request
POST http://127.0.0.1:21002/vote
Content-Type: application/json

{
  "vote": {"term": 2, "node_id": 1},
  "last_log_id": {"term": 1, "index": 10}
}
```

```bash
# install_snapshot request
POST http://127.0.0.1:21003/snapshot
Content-Type: application/json

{
  "vote": {"term": 3, "node_id": 1},
  "meta": {...},
  "offset": 0,
  "data": [...]
}
```

**Response Examples:**

```json
// Success response
{
  "Ok": {
    "vote": {"term": 1, "node_id": 2},
    "success": true,
    "conflict": false
  }
}
```

```json
// Error response
{
  "Err": {
    "error_type": "SomeRaftError",
    "message": "Error description"
  }
}
```

## Usage

### Add Dependency

```toml
[dependencies]
network-v1 = { path = "../network-v1" }
```

### Use in Application

```rust
use network_v1_http::NetworkFactory;

// Create network factory
let network = NetworkFactory {};

// Create raft instance
let raft = openraft::Raft::new(
    node_id,
    config,
    network,        // Use network-v1
    log_store,
    state_machine
).await?;
```

### Example Application

See [`raft-kv-rocksdb`](../raft-kv-rocksdb/) for a complete example of using this network implementation.
