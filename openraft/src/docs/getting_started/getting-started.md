# Getting Started with Openraft

In this chapter, we will build a key-value store cluster using Openraft.

## What this chapter targets

<!-- BEGIN GENERATED VERSION CONTRACT: scripts/build_version_contract.py -->
This chapter describes Openraft `0.10.0-alpha.34`, the 0.10 prerelease line,
developed on branch `main`. An application depends on it with:

```toml
[dependencies]
openraft = { version = "0.10.0-alpha.34", features = ["serde"] }
```

`tests-consumer/Cargo.toml` compiles this declaration on its own, outside this
repository's workspace, so the feature list above is enough by itself.
<!-- END GENERATED VERSION CONTRACT -->

- **Network API**: implement [`RaftNetworkV2`]. Its predecessor `RaftNetwork`
  lives in the separate `openraft-legacy` crate, for applications migrating
  from 0.9.
- **Runtime**: tokio, supplied by the default `tokio-rt` feature, which is why
  the dependency declaration names no runtime. Another runtime plugs in through
  [`AsyncRuntime`].
- **`serde`**: the feature adds `Serialize`/`Deserialize` bounds to the types
  that cross the network, so an application replicating over a network
  transport enables it.
- **Examples**: `raft-kv-memstore` is the canonical one, and
  `raft-kv-rocksdb` is its persistent-storage variation. The gRPC,
  single-threaded, OpenDAL-snapshot, and multi-Raft examples each vary one
  component; `raft-kv-memstore-network-v1` is legacy, kept for the v1 network
  trait. [examples/README.md](https://github.com/databendlabs/openraft/blob/main/examples/README.md)
  maps each one to the components it swaps.
- **Production**: every example is a demonstration. Its storage keeps data in
  memory or in a store wired for clarity rather than durability, and its HTTP
  transport has no authentication, retry budget, or backpressure. The traits
  are the product; the examples show how to satisfy them.

[examples/raft-kv-memstore](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore)
is the canonical example application: a complete server, client, and demo
cluster that keeps its data in memory. This chapter follows that example, and
every code link below points into it or into the helper crates it uses, so the
guide and a compiling application describe the same wiring. Its
[README](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/README.md)
maps each component to the file that implements it.

[examples/raft-kv-rocksdb](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-rocksdb)
is the same application with RocksDB persistent storage. Read it as a storage
variation of the canonical example, not as a second starting point.

---

Raft is a distributed consensus protocol designed to manage a replicated log containing state machine commands from clients.

Raft includes two major parts:

- Replicating logs consistently among nodes,
- Consuming the logs, which is mainly defined in the state machine.

Implementing your own Raft-based application with Openraft is quite simple, and it involves:

1. Defining client request and response,
2. Implementing a storage for Raft to store its state,
3. Implementing a network layer for Raft to transmit messages.

## 1. Define client request and response

A request is some data that modifies the Raft state machine.
A response is some data that the Raft state machine returns to the client.

Request and response can be any types that implement [`AppData`] and [`AppDataResponse`], for example:

```rust
use std::fmt;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Request {
    pub key: String,
}

// `AppData` requires `Display` in addition to `Debug`.
impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Set({})", self.key)
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Response {
    pub value: Option<String>,
}
```

The `serde` derives are required when Openraft is built with its `serde`
feature, which is what [`AppData`]'s `OptionalFeatures` bound expands to.

These two types are entirely application-specific and are mainly related to the
state machine implementation in [`RaftStateMachine`].


## 2. Define types config for the application

Openraft is a generic implementation of Raft. It requires the application to define
concrete types for its generic arguments. Most types are parameterized by
[`RaftTypeConfig`], as [`Raft`] itself is:

```text
pub struct Raft<C: RaftTypeConfig> { .. }
```

The simplest way to define your types config for example `TypeConfig`
is using [`declare_raft_types!`] macro:

```rust
# use std::fmt;
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Request { pub key: String }
# impl fmt::Display for Request {
#     fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "Set({})", self.key) }
# }
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Response { pub value: Option<String> }
openraft::declare_raft_types!(
   pub TypeConfig: D = Request, R = Response
);
```

This macro call adds the above `Request` and `Response` to the `TypeConfig` struct.
- `D = Request` is the raft-log payload (usually some command to run)
  that will be replicated by the raft protocol,
  and will be applied to the state machine, i.e., your implementation of [`RaftStateMachine`].
- `R = Response` is the response that the state machine returns to the client after applying a `Request`.

There are several more generic types that could be defined in [`RaftTypeConfig`].
The above macro call sets these absent types to the default values;
[`declare_raft_types!`] documents the complete default list. The types the
declaration fills in for you are:

> - `NodeId` is the identifier of a node in the cluster, which implements
>   [`NodeId`] trait. A node ID identifies one node, holding one log, for the
>   lifetime of the cluster: never give the ID of a removed node to a node that
>   starts from an empty log, otherwise the cluster may split-brain. See:
>   [Node IDs must not be reused][`docs::node-id-reuse`].
> - `Node` is the node type that contains the node's address, etc., which
>   implements [`Node`] trait.
> - `Entry` is the log entry type that will be stored in the raft log,
>   which includes the payload and log id, which implements [`RaftEntry`] trait.
> - `Responder<T>` is the type that will be used to send responses to the client,
>   which implements [`Responder`] trait.
> - `AsyncRuntime` is the async runtime that will be used to run the raft
>   instance, which implements [`AsyncRuntime`] trait.

Openraft provides default implementations for mostly used types:
- `Node`: [`EmptyNode`], [`BasicNode`] and [`NodeInfo`],
- log `Entry`: [`Entry`],
- `AsyncRuntime`: [`TokioRuntime`], which is a wrapper of tokio runtime,
- `Responder`: [`ProgressResponder`], the default, which notifies the caller both when the entry is committed and when it is applied;
  and [`OneshotResponder`], which is a wrapper of oneshot sender and receiver provided by [`AsyncRuntime`].

You can use these implementations directly or define your own custom types.
The canonical example overrides just one of them, `Node`, because each of its
nodes carries two addresses; its `D` and `R` come from the shared
[`types-kv`](https://github.com/databendlabs/openraft/blob/main/examples/types-kv/src/lib.rs)
crate instead of the `Request`/`Response` declared above:

```rust
# use std::fmt;
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Request { pub key: String }
# impl fmt::Display for Request {
#     fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "Set({})", self.key) }
# }
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Response { pub value: Option<String> }
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = openraft::NodeInfo,
);
```

A [`RaftTypeConfig`] is also used by other components such as [`RaftLogStorage`], [`RaftStateMachine`],
[`RaftNetworkFactory`] and [`RaftNetworkV2`].


## 3. Implement [`RaftLogStorage`] and [`RaftStateMachine`]

The trait [`RaftLogStorage`] defines how log data is stored and consumed.
It could be a wrapper for a local key-value store like [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/).

The trait [`RaftStateMachine`] defines how log is interpreted. Usually it is an in memory state machine with or without on-disk data backed.

Snapshot data is configured by the [`SnapshotData`] associated type, because it is the handle produced and consumed by the state machine.

There is a good example,
[`Mem KV Store`](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/store/mod.rs),
that demonstrates what should be done when a method is called. The storage methods are listed as the below.
Follow the links to method documentations to see the details.

| Kind       | [`RaftLogStorage`] method | Return value                 | Description                           |
|------------|---------------------------|------------------------------|---------------------------------------|
| Read log:  | [`get_log_reader()`]      | impl [`RaftLogReader`]       | get a read-only log reader            |
|            |                           | ↳ [`try_get_log_entries()`]  | get a range of logs                   |
|            | [`get_log_state()`]       | [`LogState`]                 | get first/last log id                 |
| Write log: | [`append()`]              | ()                           | append logs                           |
| Write log: | [`truncate_after()`]      | ()                           | delete logs `(index, +oo)`            |
| Write log: | [`purge()`]               | ()                           | purge logs `(-oo, index]`             |
| Vote:      | [`save_vote()`]           | ()                           | save vote                             |

| Kind       | [`RaftStateMachine`] method    | Return value                 | Description                           |
|------------|--------------------------------|------------------------------|---------------------------------------|
| SM:        | [`applied_state()`]            | [`LogId`], [`Membership`]    | get last applied log id, membership   |
| SM:        | [`apply()`]                    | Vec of [`AppDataResponse`]   | apply logs to state machine           |
| Snapshot:  | [`install_snapshot()`]         | ()                           | install snapshot                      |
| Snapshot:  | [`get_current_snapshot()`]     | [`Snapshot`]                 | get current snapshot                  |
| Snapshot:  | [`get_snapshot_builder()`]     | impl [`RaftSnapshotBuilder`] | get a snapshot builder                |
|            |                                | ↳ [`build_snapshot()`]       | build a snapshot from state machine   |

Most of the APIs are quite straightforward, except two indirect APIs:

-   Read logs:
    [`RaftLogStorage`] defines a method [`get_log_reader()`] to get log reader [`RaftLogReader`] :

    ```text
    // Abbreviated; see `RaftLogStorage` for the full signature.
    trait RaftLogStorage<C: RaftTypeConfig> {
        type LogReader: RaftLogReader<C>;
        async fn get_log_reader(&mut self) -> Self::LogReader;
    }
    ```

    [`RaftLogReader`] defines the APIs to read logs, and is an also super trait of [`RaftLogStorage`] :
    - [`try_get_log_entries()`] get log entries in a range;
    - [`read_vote()`] read vote;

    ```text
    // Abbreviated; see `RaftLogReader` for the full signature.
    trait RaftLogReader<C: RaftTypeConfig> {
        async fn try_get_log_entries<RB: RangeBounds<u64>>(&mut self, range: RB) -> Result<Vec<C::Entry>, ..>;
        async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, ..>;
    }
    ```

    And [`RaftLogStorage::get_log_state()`][`get_log_state()`] get latest log state from the storage;

-   Build a snapshot from the local state machine needs to be done in two steps:
    - [`RaftStateMachine::get_snapshot_builder() -> Self::SnapshotBuilder`][`get_snapshot_builder()`],
    - [`RaftSnapshotBuilder::build_snapshot() -> Result<Snapshot>`][`build_snapshot()`],


### Ensure the storage implementation is correct

There is a [Test suite for RaftLogStorage and RaftStateMachine][`LogSuite`] available in Openraft.
If your implementation passes the tests, Openraft should work well with it.
To test your implementation, run `Suite::test_all()` with a [`StoreBuilder`] implementation,
as shown in the [`sm-rocks` test](https://github.com/databendlabs/openraft/blob/main/examples/sm-rocks/src/test.rs).

Once all tests pass, you can ensure that your custom storage implementation can work correctly in a distributed system.


### An implementation has to guarantee data durability.

The caller always assumes a completed writing is persistent.
The raft correctness highly depends on a reliable store.


## 4. Implement [`RaftNetworkV2`].

Raft nodes communicate with each other to achieve consensus about the logs.
The trait [`RaftNetworkV2`] defines the data transmission protocol.

```text
// Abbreviated; see `RaftNetworkV2` for the full signature.
pub trait RaftNetworkV2<C: RaftTypeConfig>: Send + Sync + 'static {
    type SnapshotData: OptionalSend + 'static;

    async fn append_entries(&mut self, rpc: AppendEntriesRequest<C>, option: RPCOption) -> Result<..>;
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<..>;
    async fn full_snapshot(&mut self, vote: Vote<C::NodeId>, snapshot: SnapshotOf<C, Self::SnapshotData>, cancel: impl Future<..>, option: RPCOption) -> Result<..>;

    // Optional: override for pipelined replication
    fn stream_append(&mut self, input: impl Stream<Item = AppendEntriesRequest<C>>, option: RPCOption) -> BoxFuture<Result<BoxStream<StreamAppendResult<C>>>>;
}
```

An implementation of [`RaftNetworkV2`] can be considered as a wrapper that invokes
the corresponding methods of a remote [`Raft`]. It is responsible for sending
and receiving messages between Raft nodes.

The `RPCOption` argument carries the timeout budget for an RPC. The network
implementation is responsible for enforcing `soft_ttl()` with a transport
timeout, deadline, or reconnect policy. Openraft may shut down an in-flight RPC
once `hard_ttl()` has elapsed.

For streaming AppendEntries, `hard_ttl()` is not a lifetime limit for the whole
stream. Use `soft_ttl()` for setup, idle timeout, keepalive, or per-response
deadline policy to detect a stuck stream.

Here is the list of methods that need to be implemented for the [`RaftNetworkV2`] trait:


| [`RaftNetworkV2`] method | forward request            | to target                                     |
|--------------------------|----------------------------|-----------------------------------------------|
| [`append_entries()`]     | [`AppendEntriesRequest`]   | remote node [`Raft::append_entries()`]        |
| [`vote()`]               | [`VoteRequest`]            | remote node [`Raft::vote()`]                  |
| [`full_snapshot()`]      | [`Snapshot`]               | remote node [`Raft::install_full_snapshot()`] |

### Optional: `stream_append()` for pipelined replication

[`stream_append()`] is an optional method that enables bidirectional streaming
for efficient pipelined log replication. The default implementation uses
[`append_entries()`] sequentially.

To enable pipelined replication, override [`stream_append()`] to forward the
request stream to the remote node's [`Raft::stream_append()`] and return the
response stream.

| [`RaftNetworkV2`] method | forward request            | to target                                     |
|--------------------------|----------------------------|-----------------------------------------------|
| [`stream_append()`]      | [`AppendEntriesRequest`] stream | remote node [`Raft::stream_append()`]    |

The canonical example gets both sides from the [`network-v2-http`](https://github.com/databendlabs/openraft/tree/main/examples/network-v2-http)
crate. Its [client](https://github.com/databendlabs/openraft/blob/main/examples/network-v2-http/src/client.rs)
demonstrates how to forward messages to other Raft nodes using [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) as network transport layer.

To receive and handle these requests, there should be a server endpoint for each of these RPCs.
When the server receives a Raft RPC, it simply passes it to its `raft` instance and replies with the returned result:
[network-v2-http server](https://github.com/databendlabs/openraft/blob/main/examples/network-v2-http/src/server.rs).

For a real-world implementation, you may want to use [Tonic gRPC](https://github.com/hyperium/tonic) to handle gRPC-based communication between Raft nodes. The [databend-meta](https://github.com/databendlabs/databend/blob/6603392a958ba8593b1f4b01410bebedd484c6a9/metasrv/src/network.rs#L89) project provides an excellent real-world example of a Tonic gRPC-based Raft network implementation.


### Implement [`RaftNetworkFactory`].

[`RaftNetworkFactory`] is a singleton responsible for creating [`RaftNetworkV2`] instances for each replication target node.

```text
// Abbreviated; see `RaftNetworkFactory` for the full signature.
pub trait RaftNetworkFactory<C: RaftTypeConfig>: Send + Sync + 'static {
    type Network: RaftNetworkV2<C>;
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;
}
```

This trait contains only one method:
- [`RaftNetworkFactory::new_client()`] builds a new [`RaftNetworkV2`] instance for a target node, intended for sending RPCs to that node.
  The associated type `RaftNetworkFactory::Network` represents the application's implementation of the `RaftNetworkV2` trait.

This function should **not** establish a connection; instead, it should create a client that connects when
necessary.


### How RaftNetworkV2 and server interact

The [`RaftNetworkV2`] implementation forwards Raft RPCs to the application-implemented server on another node.
The server then forwards these RPCs to the corresponding [`Raft`] methods and returns the response.

**Request flow**:

1. **Client node**: [`append_entries()`] sends [`AppendEntriesRequest`] to target node's server
2. **Target server**: Receives the RPC and calls local [`Raft::append_entries()`]
3. **Target server**: Gets the response and sends it back to the client node
4. **Client node**: Receives the response

```text

.--------------------------.           .--------------------------------.
|    RaftCore              |           |                                |
| (8) ^   | (1)            |           |                                |
|     |   v                |           |                                |
|  ReplicationCore         |           |  Raft::append_entries          |
| (7) ^   | (2)            |           | (4) ^ | (5)                    |
|     |   | append_entries |           |     | | AppendEntriesResponse  |
|     |   v                |    (3)    |     | v                        |
|  RaftNetworkV2 -----------------------> Application Server            |
|     ^---------------------------------- /append_entries               |
|  (HTTP/gRPC/etc client)  |    (6)    |  (HTTP/gRPC/etc endpoint)      |
'--------------------------'           '--------------------------------'
    Leader                                 Follower


Flow:
(1) RaftCore triggers ReplicationCore to replicate logs
(2) ReplicationCore calls append_entries on RaftNetworkV2
(3) RaftNetworkV2 sends RPC request over network (HTTP/gRPC/etc)
(4) Application Server receives request at /append_entries endpoint
(5) Application Server forwards to local Raft::append_entries
(6) Raft::append_entries returns AppendEntriesResponse back through Application Server
(7) RaftNetworkV2 receives the response
(8) ReplicationCore updates RaftCore with replication result
```

The same pattern applies to other RPC methods: [`vote()`], [`full_snapshot()`].

**Example server implementation**, in the shape a handler takes; the tested one
is [`network-v2-http`'s `/append` route](https://github.com/databendlabs/openraft/blob/main/examples/network-v2-http/src/server.rs):

```text
// Pseudocode: `ServerError` stands for the application's own error type.
async fn handle_append_entries(
    raft: Arc<Raft<TypeConfig, StateMachineStore>>,
    req: AppendEntriesRequest<TypeConfig>,
) -> Result<AppendEntriesResponse<TypeConfig>, ServerError> {
    let resp = raft.append_entries(req).await?;
    Ok(resp)
}
```


### Find the address of the target node.

In Openraft, an implementation of [`RaftNetworkV2`] needs to connect to remote Raft peers. To store additional information about each peer, you need to specify the `Node` type in `RaftTypeConfig`:

```rust
# use std::fmt;
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Request { pub key: String }
# impl fmt::Display for Request {
#     fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "Set({})", self.key) }
# }
# #[derive(Clone, Debug)]
# #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
# pub struct Response { pub value: Option<String> }
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = openraft::BasicNode,
);
```

Then use `Raft::add_learner(node_id, BasicNode::new("127.0.0.1"), ...)` to instruct Openraft to store node information in [`Membership`]. This information is then consistently replicated across all nodes, and will be passed to [`RaftNetworkFactory::new_client()`] to connect to remote Raft peers:

```json
{
  "configs": [ [ 1, 2, 3 ] ],
  "nodes": {
    "1": { "addr": "127.0.0.1:21001" },
    "2": { "addr": "127.0.0.1:21002" },
    "3": { "addr": "127.0.0.1:21003" }
  }
}
```

###  Caution: ensure that a connection to the right node

See: [Ensure connection to the correct node][`docs::connect-to-correct-node`]


## 5. Put everything together

Finally, we put these parts together and boot up a raft node in
[lib.rs](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/lib.rs).
A node listens on two addresses: `raft_addr` serves the Raft RPCs that peers
send, `api_addr` serves client and admin requests. Openraft's single requirement
here is that inbound Raft RPCs reach the [`Raft`] methods; serving them from two
listeners is the example's own choice, explained in
[Two servers per node](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/README.md#two-servers-per-node).

The following is an excerpt of that file, abridged for reading. It compiles as
part of the example crate, which CI builds and tests; it does not compile on
its own, because the types it names live in the example's sibling crates.

```rust,ignore
pub async fn start_example_raft_node(node_id: NodeId, api_addr: String, raft_addr: String) -> std::io::Result<()> {
    let config = Arc::new(Config::default().validate().unwrap());

    let log_store = LogStore::default();
    let state_machine_store = StateMachineStore::default();
    let network = network_v2_http::NetworkFactory::new();

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: state_machine_store,
    });

    // Raft RPCs from peer nodes: `/append`, `/vote`, `/snapshot`, ...
    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);

    // Client and admin API: `/init`, `/add-learner`, `/write`, `/read`, ...
    let app_server = app_http::Server::new(app)
        .add_openraft_routes()
        .post("/read", http_api::read)
        .post("/linearizable_read", http_api::linearizable_read)
        .post("/follower_read", http_api::follower_read)
        .run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}
```

`add_openraft_routes()` registers the admin and write endpoints that every
example shares, defined in
[app-http](https://github.com/databendlabs/openraft/blob/main/examples/app-http/src/app.rs);
the three `post()` calls add this application's own read endpoints from
[http_api.rs](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/http_api.rs).

## 6. Run the cluster

To set up a demo Raft cluster, follow these steps:

1. Bring up three uninitialized Raft nodes.
1. Initialize a single-node cluster.
1. Add more Raft nodes to the cluster.
1. Update the membership configuration.

The [examples/raft-kv-memstore](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore)
directory provides a detailed description of these steps.

Additionally, two test scripts for setting up a cluster are available:

- [test-cluster.sh](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/test-cluster.sh)
  is a minimal Bash script that uses `curl` to communicate with the Raft
  cluster. It demonstrates the plain HTTP messages being sent and received.

- [test_basic.rs](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/tests/cluster/test_basic.rs)
  uses `app_http::Client` to set up a cluster, write data, and read it back.
  Its sibling tests in the same directory cover membership changes, snapshots,
  and the three read modes.


[`declare_raft_types!`]:                `crate::declare_raft_types`
[`Raft`]:                               `crate::Raft`
[`Raft::append_entries()`]:             `crate::Raft::append_entries`
[`Raft::stream_append()`]:              `crate::Raft::stream_append`
[`Raft::vote()`]:                       `crate::Raft::vote`
[`Raft::install_full_snapshot()`]:      `crate::Raft::install_full_snapshot`

[`AppendEntriesRequest`]:               `crate::raft::AppendEntriesRequest`
[`VoteRequest`]:                        `crate::raft::VoteRequest`

[`RaftTypeConfig`]:                     `crate::RaftTypeConfig`
[`AsyncRuntime`]:                       `crate::AsyncRuntime`
[`AppData`]:                            `crate::AppData`
[`AppDataResponse`]:                    `crate::AppDataResponse`
[`RaftEntry`]:                          `crate::entry::RaftEntry`
[`Node`]:                               `crate::node::Node`
[`NodeId`]:                             `crate::node::NodeId`
[`Responder`]:                          `crate::raft::responder::Responder`

[`TokioRuntime`]:                       `crate::impls::TokioRuntime`
[`OneshotResponder`]:                   `crate::impls::OneshotResponder`
[`ProgressResponder`]:                  `crate::impls::ProgressResponder`

[`LogId`]:                              `crate::LogId`
[`Membership`]:                         `crate::Membership`
[`EmptyNode`]:                          `crate::EmptyNode`
[`BasicNode`]:                          `crate::BasicNode`
[`NodeInfo`]:                           `crate::NodeInfo`
[`Entry`]:                              `crate::entry::Entry`
[`Vote`]:                               `crate::vote::Vote`
[`LogState`]:                           `crate::storage::LogState`

[`RaftLogReader`]:                      `crate::storage::RaftLogReader`
[`try_get_log_entries()`]:              `crate::storage::RaftLogReader::try_get_log_entries`
[`read_vote()`]:                        `crate::storage::RaftLogReader::read_vote`



[`RaftLogStorage`]:                     `crate::storage::RaftLogStorage`
[`RaftLogStorage::LogReader`]:          `crate::storage::RaftLogStorage::LogReader`
[`append()`]:                           `crate::storage::RaftLogStorage::append`
[`truncate_after()`]:                   `crate::storage::RaftLogStorage::truncate_after`
[`purge()`]:                            `crate::storage::RaftLogStorage::purge`
[`save_vote()`]:                        `crate::storage::RaftLogStorage::save_vote`
[`get_log_state()`]:                    `crate::storage::RaftLogStorage::get_log_state`
[`get_log_reader()`]:                   `crate::storage::RaftLogStorage::get_log_reader`

[`RaftStateMachine`]:                   `crate::storage::RaftStateMachine`
[`SnapshotData`]:                       `crate::storage::RaftStateMachine::SnapshotData`
[`RaftStateMachine::SnapshotBuilder`]:  `crate::storage::RaftStateMachine::SnapshotBuilder`
[`applied_state()`]:                    `crate::storage::RaftStateMachine::applied_state`
[`apply()`]:                            `crate::storage::RaftStateMachine::apply`
[`get_current_snapshot()`]:             `crate::storage::RaftStateMachine::get_current_snapshot`
[`install_snapshot()`]:                 `crate::storage::RaftStateMachine::install_snapshot`
[`get_snapshot_builder()`]:             `crate::storage::RaftStateMachine::get_snapshot_builder`

[`RaftNetworkFactory`]:                 `crate::network::RaftNetworkFactory`
[`RaftNetworkFactory::new_client()`]:   `crate::network::RaftNetworkFactory::new_client`
[`RaftNetworkV2`]:                      `crate::network::RaftNetworkV2`
[`append_entries()`]:                   `crate::network::RaftNetworkV2::append_entries`
[`stream_append()`]:                    `crate::network::RaftNetworkV2::stream_append`
[`vote()`]:                             `crate::network::RaftNetworkV2::vote`
[`full_snapshot()`]:                    `crate::network::RaftNetworkV2::full_snapshot`


[`RaftSnapshotBuilder`]:                `crate::storage::RaftSnapshotBuilder`
[`build_snapshot()`]:                   `crate::storage::RaftSnapshotBuilder::build_snapshot`
[`Snapshot`]:                           `crate::storage::Snapshot`

[`StoreBuilder`]:                       `crate::testing::log::StoreBuilder`
[`LogSuite`]:                              `crate::testing::log::Suite`

[`Fatal`]:                              `crate::error::Fatal`
[`Unreachable`]:                        `crate::error::Unreachable`

[`docs::connect-to-correct-node`]:      `crate::docs::cluster_control::dynamic_membership#ensure-connection-to-the-correct-node`
[`docs::node-id-reuse`]:                `crate::docs::cluster_control::dynamic_membership#node-ids-must-not-be-reused`
