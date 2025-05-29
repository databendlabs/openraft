# Client-HTTP

An OpenRaft client implementation example that demonstrates **how to interact with a Raft cluster** using HTTP and `reqwest` as the transport mechanism.

## Role and Position in OpenRaft

`client-http` provides a **client library for applications to interact with OpenRaft clusters**. It handles **leader discovery, automatic leader forwarding, and provides both application APIs and cluster management APIs**.

### Position in OpenRaft Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│                (Business Applications)                      │
│                                                             │
│  ┌─────────────────┐                                        │
│  │   client-http   │ ◄── Application uses this client       │
│  │   (This Crate)  │                                        │
│  └────────┬────────┘                                        │
└───────────│─────────────────────────────────────────────────┘
            │ HTTP Requests
            ▼
┌─────────────────────────────────────────────────────────────┐
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
│  └─────────────────┘                    └─────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Core Functions

- **Application Data Operations**: Read/write key-value data from/to the Raft cluster
- **Cluster Management**: Initialize cluster, add nodes, change membership
- **Leader Discovery**: Automatically discover and connect to the current cluster leader
- **Auto-forwarding**: Automatically retry requests when leader changes

### Core Technology Stack

- **HTTP Protocol**: Implemented using `reqwest`
- **Serialization**: Uses `serde` + JSON
- **Leader Management**: Automatic leader tracking and forwarding

### Client APIs

#### Application Data APIs

```rust
impl ExampleClient {
    /// Submit a write request to the raft cluster
    pub async fn write(&self, req: &Request) -> Result<Result<ClientWriteResponse, ClientWriteError>, RPCError>;
    
    /// Read value by key (inconsistent read)
    pub async fn read(&self, req: &String) -> Result<String, RPCError>;
    
    /// Consistent read with linearizability guarantee
    pub async fn linearizable_read(&self, req: &String) -> Result<Result<String, CheckIsLeaderError>, RPCError>;
    
    /// Auto-forwarding linearizable read
    pub async fn linearizable_read_auto_forward(&self, req: &String) -> Result<Result<String, CheckIsLeaderError>, RPCError>;
}
```

#### Cluster Management APIs

```rust
impl ExampleClient {
    /// Initialize a single-node cluster
    pub async fn init(&self) -> Result<Result<(), InitializeError>, RPCError>;
    
    /// Add a node as learner
    pub async fn add_learner(&self, req: (NodeId, String)) -> Result<Result<ClientWriteResponse, ClientWriteError>, RPCError>;
    
    /// Change cluster membership
    pub async fn change_membership(&self, req: &BTreeSet<NodeId>) -> Result<Result<ClientWriteResponse, ClientWriteError>, RPCError>;
    
    /// Get cluster metrics
    pub async fn metrics(&self) -> Result<RaftMetrics, RPCError>;
}
```

### Leader Discovery and Auto-forwarding

The client automatically handles leader changes through the `send_with_forwarding` mechanism:

```rust
async fn send_with_forwarding<Req, Resp, Err>(
    &self,
    uri: &str,
    req: Option<&Req>,
    retry: usize,
) -> Result<Result<Resp, Err>, RPCError>
```

### HTTP Protocol Requirements

#### Client-to-Server Communication

The client sends requests to these server endpoints:

| Client Method | HTTP Endpoint | Request Type | Response Type | Description |
|---------------|---------------|--------------|---------------|-------------|
| `write` | `POST /write` | `RaftTypeConfig::D` | `Result<ClientWriteResponse, ClientWriteError>` | Write data to cluster |
| `read` | `POST /read` | `String` | `String` | Read data (inconsistent) |
| `linearizable_read` | `POST /linearizable_read` | `RaftTypeConfig::R` | `Result<String, CheckIsLeaderError>` | Read data (consistent) |
| `init` | `POST /init` | `{}` | `Result<(), InitializeError>` | Initialize cluster |
| `add_learner` | `POST /add-learner` | `(NodeId, String)` | `Result<ClientWriteResponse, ClientWriteError>` | Add learner node |
| `change_membership` | `POST /change-membership` | `BTreeSet<NodeId>` | `Result<ClientWriteResponse, ClientWriteError>` | Change membership |
| `metrics` | `POST /metrics` | `{}` | `RaftMetrics` | Get cluster metrics |

#### Protocol Examples

**Write Request:**
```bash
POST http://127.0.0.1:21001/write
Content-Type: application/json

{
  "Set": {
    "key": "foo",
    "value": "bar"
  }
}
```

**Response:**
```json
{
  "Ok": {
    "log_id": {"term": 1, "index": 5},
    "data": "bar"
  }
}
```

**Read Request:**
```bash
POST http://127.0.0.1:21001/read
Content-Type: application/json

"foo"
```

**Metrics Request:**
```bash
POST http://127.0.0.1:21001/metrics
Content-Type: application/json

{}
```

## Usage

### Add Dependency

```toml
[dependencies]
client-http = { path = "../client-http" }
```

### Basic Usage

```rust
use client_http::ExampleClient;

// Create client pointing to a cluster node
let client = ExampleClient::new(1, "127.0.0.1:21001".to_string());

// Initialize cluster (first time setup)
client.init().await?;

// Write data
let write_req = Request::Set {
    key: "foo".to_string(),
    value: "bar".to_string(),
};
client.write(&write_req).await?;

// Read data
let value = client.read(&"foo".to_string()).await?;
println!("Value: {}", value);

// Get cluster status
let metrics = client.metrics().await?;
println!("Leader: {:?}", metrics.current_leader);
```

### Cluster Management

```rust
// Add a new node as learner
client.add_learner((2, "127.0.0.1:21002".to_string())).await?;

// Change membership to include the new node
let mut new_members = BTreeSet::new();
new_members.insert(1);
new_members.insert(2);
client.change_membership(&new_members).await?;
```

### Example Applications

See these complete examples for usage:
- [`raft-kv-memstore`](../raft-kv-memstore/)
- [`raft-kv-rocksdb`](../raft-kv-rocksdb/)
