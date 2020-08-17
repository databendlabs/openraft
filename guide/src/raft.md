Raft API
========
The `Raft` type represents the singular API of this crate, and is the interface to a running Raft node. It is highly generic, which allows your application's data types to be known at compile, for maximum performance and type-safety. Users of this Raft implementation get to choose the exact types to be used throughout the system, and get to work with their application's data types directly without the overhead of serializing and deserializing the data as it moves through the `Raft` system.

In previous chapters, we've defined our `AppData`, `AppDataResponse`, `RaftNetwork` and `RaftStorage` types. These four types are used as part of a concrete `Raft` definition, and applications may find it beneficial to define an alias covering all of these types for easier reference. Something like the following:

```rust
/// Your Raft type alias.
type YourRaft = Raft<YourData, YourDataResponse, YourRaftNetwork, YourRaftStorage>;
```

### API
The API of the `Raft` type is broken up into 4 sections: Client Requests, Raft RPCs, Admin Commands & Utility Methods.

#### Client Requests
The application level interface for clients is 100% at the discression of the application being built. However, once a client read or write operation is ready to be processed, the below methods provide the read/write functionality for Raft interaction.

- [`async fn client_read(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.client_read): Check to ensure this node is still the cluster leader, in order to guard against stale reads. The actual read operation itself is up to the application, this method just ensures that the read will not be stale.
- [`async fn client_write(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.client_write): Submit a mutating client request to Raft to update the state of the system (§5.1). It will be appended to the log, committed to the cluster, and then applied to the application state machine. The result of applying the request to the state machine will be returned as the response from this method.

#### Raft RPCs
These methods directly correspond to the `RaftNetwork` trait described in earlier chapters. The application is responsible for implementing its own network layer which can receive these RPCs coming from Raft peers, and should then pass them into the Raft node using the following methods.

- [`async fn append_entries(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.append_entries): An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
- [`async fn vote(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.vote): An RPC invoked by candidates to gather votes (§5.2).
- [`async fn install_snapshot(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.install_snapshot): Invoked by the Raft leader to send chunks of a snapshot to a follower (§7).

#### Admin Commands
All of these methods are intended for use directly by the parent application for managing various lifecycles of the cluster. Each of these lifecycles are discussed in more detail in the [Cluster Controls](https://async-raft.github.io/async-raft/cluster-controls.html) chapter.

- [`async fn initialize(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.initialize): Initialize a pristine Raft node with the given config & start a campaign to become leader.
- [`async fn add_non_voter(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.add_non_voter): Add a new node to the cluster as a non-voter, which will sync the node with the master so that it can later join the cluster as a voting member.
- [`async fn change_membership(...) -> Result<...>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.change_membership): Propose a new membership config change to a running cluster.

#### Utility Methods
- [`fn metrics(&self) -> watch::Receiver<RaftMetrics>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.metrics): Get a stream of all metrics coming from the Raft node.
- [`fn shutdown(self) -> tokio::task::JoinHandle<RaftResult<()>>`](https://docs.rs/async-raft/latest/async_raft/raft/struct.Raft.html#method.shutdown): Send a shutdown signal to the Raft node, and get a `JoinHandle` which can be used to await the full shutdown of the node. If the node is already in shutdown, this routine will allow you to await its full shutdown.

### Reading & Writing Data
What does the Raft spec have to say about reading and writing data?

> Clients of Raft send all of their requests to the leader. When a client first starts up, it connects to a randomly-chosen server. If the client’s first choice is not the leader, that server will reject the client’s request and supply information about the most recent leader it has heard from. If the leader crashes, client requests will timeout; clients then try again with randomly-chosen servers.

The `Raft.metrics` method, discussed above, provides a stream of data on the Raft node's internals, and should be used in order to determine the cluster leader, which should only need to be performed once when the client connection is first established.

> Our goal for Raft is to implement linearizable semantics (each operation appears to execute instantaneously, exactly once, at some point between its invocation and its response). [...] if the leader crashes after committing the log entry but before responding to the client, the client [may] retry the command with a new leader, causing it to be executed a second time. The solution is for clients to assign unique serial numbers to every command. Then, the state machine tracks the latest serial number processed for each client, along with the associated response. If it receives a command whose serial number has already been executed, it responds immediately without re-executing the request.

As described in the quote above, applications will need to have their clients assign unique serial numbers to every command sent to the application servers. Then, within the application specific code implemented inside of `RaftStorage::apply_entry_to_state_machine`, if the application detects that the serial number has already been executed for the requesting client, then the response should be immediately returned without re-executing the request. Much of this will be application specific, but these principals can help with design.

> Read-only operations can be handled without writing anything into the log. However, with no additional measures, this would run the risk of returning stale data, since the leader responding to the request might have been superseded by a newer leader of which it is unaware. [...] a leader must check whether it has been deposed before processing a read-only request (its information may be stale if a more recent leader has been elected). Raft handles this by having the leader exchange heartbeat messages with a majority of the cluster before responding to read-only requests.

The `Raft.client_read` method should be used to ensure that the callee Raft node is still the cluster leader.

----

The API is simple enough, now its time to put everything together.
