Putting It All Together
=======================
In previous chapters we've seen how to define our application's data types which will be used for interacting with Raft, we've seen how to implement the `RaftNetwork` trait, we've seen how to implement the `RaftStorage` trait, and we've reviewed the `Raft` API itself. Now its time to put all of these components together. Let's do this.

For this chapter, we're going to use snippets of the code found in the `memstore` crate, which is an in-memory implementation of the `RaftStorage` trait for demo and testing purposes, which also happens to be used for all of the integration tests of `async-raft` itself.

### Recap On Our Data Types
As we've seen earlier, here are our `AppData` and `AppDataResponse` types/impls.

```rust
/// The application data request type which the `MemStore` works with.
///
/// Conceptually, for demo purposes, this represents an update to a client's status info,
/// returning the previously recorded status.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientRequest {
    /// The ID of the client which has sent the request.
    pub client: String,
    /// The serial number of this request.
    pub serial: u64,
    /// A string describing the status of the client. For a real application, this should probably
    /// be an enum representing all of the various types of requests / operations which a client
    /// can perform.
    pub status: String,
}

impl AppData for ClientRequest {}

/// The application data response type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse(Result<Option<String>, ClientError>);

impl AppDataResponse for ClientResponse {}
```

### RaftNetwork impl
We've already discussed the `RaftNetwork` trait in a previous chapter. Here is an abbreviated snippet of what the `RaftNetwork` impl looks like in the `async-raft` integration test suite.

```rust
// We use anyhow::Result in our impl below.
use anyhow::Result;

/// A type which emulates a network transport and implements the `RaftNetwork` trait.
pub struct RaftRouter {
    // ... some internal state ...
}

#[async_trait]
impl RaftNetwork<ClientRequest> for RaftRouter {
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: u64, rpc: AppendEntriesRequest<ClientRequest>) -> Result<AppendEntriesResponse> {
        // ... snip ...
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn install_snapshot(&self, target: u64, rpc: InstallSnapshotRequest) -> Result<InstallSnapshotResponse> {
        // ... snip ...
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(&self, target: u64, rpc: VoteRequest) -> Result<VoteResponse> {
        // ... snip ...
    }
}
```

### RaftStorage impl
We've already got a `RaftStorage` impl to work with from the `memstore` crate. Here is an abbreviated snippet of the code.

```rust
// We use anyhow::Result in our impl below.
use anyhow::Result;

#[async_trait]
impl RaftStorage<ClientRequest, ClientResponse> for MemStore {
    type Snapshot = Cursor<Vec<u8>>;

    async fn get_membership_config(&self) -> Result<MembershipConfig> {
        // ... snip ...
    }

    async fn get_initial_state(&self) -> Result<InitialState> {
        // ... snip ...
    }

    // The remainder of our methods are implemented below.
    // ... snip ...
}
```

### Raft Type Alias
For better readability in your application's code, it would be beneficial to define a type alias which fully qualifies all of the types which your Raft instance will be using. This is quite simple. The example below is taken directly from this project's integration test suite, which uses the `memstore` crate and a specialized `RaftNetwork` impl designed specifically for testing.

```rust
/// A concrete Raft type used during testing.
pub type MemRaft = Raft<ClientRequest, ClientResponse, RaftRouter, MemStore>;
```

### Give It The Boot
Though applications will be much more complex than this contrived example, booting a Raft node is dead simple. Even if your application uses a multi-Raft pattern for managing different segments / shards of data, the same principal applies. Boot a Raft node, and retain its instance for API usage.

```rust
//! This code assumes the code samples above.

#[tokio::main]
async fn main() {
    // Get our node's ID from stable storage.
    let node_id = get_id_from_storage().await;

    // Build our Raft runtime config, then instantiate our
    // RaftNetwork & RaftStorage impls.
    let config = Arc::new(Config::build("primary-raft-group".into())
        .validate()
        .expect("failed to build Raft config"));
    let network = Arc::new(RaftRouter::new(config.clone()));
    let storage = Arc::new(MemStore::new(node_id));

    // Create a new Raft node, which spawns an async task which
    // runs the Raft core logic. Keep this Raft instance around
    // for calling API methods based on events in your app.
    let raft = Raft::new(node_id, config, network, storage);

    run_app(raft).await; // This is subjective. Do it your own way.
                         // Just run your app, feeding Raft & client
                         // RPCs into the Raft node as they arrive.
}
```

----

You've officially ascended to the next level of AWESOME! Next, let's take a look at cluster lifecycle controls, dynamic membership, and the like.
