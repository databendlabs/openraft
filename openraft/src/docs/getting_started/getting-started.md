# Getting Started with Openraft

In this chapter, we will build a key-value store cluster using Openraft.

1. [examples/raft-kv-memstore](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore)
   is a complete example application that includes the server, client, and a demo cluster. This example uses an in-memory store for data storage.

1. [examples/raft-kv-rocksdb](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-rocksdb)
   is another complete example application that includes the server, client, and a demo cluster. This example uses RocksDB for persistent storage.

You can use these examples as a starting point for building your own key-value
store cluster with Openraft.

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

```ignore
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {key: String}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response(Result<Option<String>, ClientError>);
```

These two types are entirely application-specific and are mainly related to the
state machine implementation in [`RaftStateMachine`].


## 2. Define types config for the application

Openraft is a generic implementation of Raft. It requires the application to define
concrete types for its generic arguments. Most types are parameterized by
[`RaftTypeConfig`] and will be used to create a `Raft` instance.

```ignore
pub struct Raft<C: RaftTypeConfig> {}
```

The simplest way to define your types config for example `TypeConfig`
is using [`declare_raft_types!`] macro:

```ignore
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
The above macro call sets these absent types to the default values,
and it generates the following type definitions:

```ignore
pub struct TypeConfig {}

impl openraft::RaftTypeConfig for TypeConfig {
    type D                = Request;
    type R                = Response;

    // Following are absent in `declare_raft_types` and filled with default values:
    type NodeId           = u64;
    type Node             = openraft::impls::BasicNode;
    type Entry            = openraft::impls::Entry<TypeConfig>;
    type Responder<T>     = openraft::impls::OneshotResponder<TypeConfig, T>
        where T: openraft::OptionalSend + 'static;
    type AsyncRuntime     = openraft::impls::TokioRuntime;
    type SnapshotData     = Cursor<Vec<u8>>;
}
```

> In the above `TypeConfig` declaration,
> - `NodeId` is the identifier of a node in the cluster, which implements
>   [`NodeId`] trait.
> - `Node` is the node type that contains the node's address, etc., which
>   implements [`Node`] trait.
> - `Entry` is the log entry type that will be stored in the raft log,
>   which includes the payload and log id, which implements [`RaftEntry`] trait.
> - `Responder<T>` is the type that will be used to send responses to the client,
>   which implements [`Responder`] trait.
> - `AsyncRuntime` is the async runtime that will be used to run the raft
>   instance, which implements [`AsyncRuntime`] trait.
> - `SnapshotData` is the type that will be used to store the snapshot data.

Openraft provides default implementations for mostly used types:
- `Node`: [`EmptyNode`] and [`BasicNode`],
- log `Entry`: [`Entry`],
- `AsyncRuntime`: [`TokioRuntime`], which is a wrapper of tokio runtime,
- `Responder`: [`OneshotResponder`], which is a wrapper of oneshot sender and receiver provided by [`AsyncRuntime`].

You can use these implementations directly or define your own custom types.

A [`RaftTypeConfig`] is also used by other components such as [`RaftLogStorage`], [`RaftStateMachine`],
[`RaftNetworkFactory`] and [`RaftNetwork`].


## 3. Implement [`RaftLogStorage`] and [`RaftStateMachine`]

The trait [`RaftLogStorage`] defines how log data is stored and consumed.
It could be a wrapper for a local key-value store like [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/).

The trait [`RaftStateMachine`] defines how log is interpreted. Usually it is an in memory state machine with or without on-disk data backed.

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
| Write log: | [`truncate()`]            | ()                           | delete logs `[index, +oo)`            |
| Write log: | [`purge()`]               | ()                           | purge logs `(-oo, index]`             |
| Vote:      | [`save_vote()`]           | ()                           | save vote                             |

| Kind       | [`RaftStateMachine`] method    | Return value                 | Description                           |
|------------|--------------------------------|------------------------------|---------------------------------------|
| SM:        | [`applied_state()`]            | [`LogId`], [`Membership`]    | get last applied log id, membership   |
| SM:        | [`apply()`]                    | Vec of [`AppDataResponse`]   | apply logs to state machine           |
| Snapshot:  | [`begin_receiving_snapshot()`] | `SnapshotData`               | begin to install snapshot             |
| Snapshot:  | [`install_snapshot()`]         | ()                           | install snapshot                      |
| Snapshot:  | [`get_current_snapshot()`]     | [`Snapshot`]                 | get current snapshot                  |
| Snapshot:  | [`get_snapshot_builder()`]     | impl [`RaftSnapshotBuilder`] | get a snapshot builder                |
|            |                                | ↳ [`build_snapshot()`]       | build a snapshot from state machine   |

Most of the APIs are quite straightforward, except two indirect APIs:

-   Read logs:
    [`RaftLogStorage`] defines a method [`get_log_reader()`] to get log reader [`RaftLogReader`] :

    ```ignore
    trait RaftLogStorage<C: RaftTypeConfig> {
        type LogReader: RaftLogReader<C>;
        async fn get_log_reader(&mut self) -> Self::LogReader;
    }
    ```

    [`RaftLogReader`] defines the APIs to read logs, and is an also super trait of [`RaftLogStorage`] :
    - [`try_get_log_entries()`] get log entries in a range;
    - [`read_vote()`] read vote;

    ```ignore
    trait RaftLogReader<C: RaftTypeConfig> {
        async fn try_get_log_entries<RB: RangeBounds<u64>>(&mut self, range: RB) -> Result<Vec<C::Entry>, ...>;
        async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, ...>>;
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
as shown in the [`RocksStore` test](https://github.com/databendlabs/openraft/blob/main/examples/rocksstore/src/test.rs).

Once all tests pass, you can ensure that your custom storage implementation can work correctly in a distributed system.


### An implementation has to guarantee data durability.

The caller always assumes a completed writing is persistent.
The raft correctness highly depends on a reliable store.


## 4. Implement [`RaftNetwork`] or [`RaftNetworkV2`].

Raft nodes communicate with each other to achieve consensus about the logs.
The trait [`RaftNetwork`] and [`RaftNetworkV2`] defines the data transmission protocol.

Your application can use either [`RaftNetwork`] or [`RaftNetworkV2`].
The only difference between them is:
- [`RaftNetwork`] sends snapshot in chunks with [`RaftNetwork::install_snapshot()`][`install_snapshot()`],
- while [`RaftNetworkV2`] sends snapshot in one piece with [`RaftNetworkV2::full_snapshot()`][`full_snapshot()`].


```ignore
pub trait RaftNetwork<C: RaftTypeConfig>: Send + Sync + 'static {
    async fn vote(&mut self, rpc: VoteRequest<C::NodeId>) -> Result<...>;
    async fn append_entries(&mut self, rpc: AppendEntriesRequest<C>) -> Result<...>;
    async fn install_snapshot(&mut self, vote: Vote<C::NodeId>, snapshot: Snapshot<C>) -> Result<...>;
}
```

An implementation of [`RaftNetwork`] can be considered as a wrapper that invokes
the corresponding methods of a remote [`Raft`]. It is responsible for sending
and receiving messages between Raft nodes.

Here is the list of methods that need to be implemented for the [`RaftNetwork`] trait:


| [`RaftNetwork`] method | forward request            | to target                                     |
|------------------------|----------------------------|-----------------------------------------------|
| [`append_entries()`]   | [`AppendEntriesRequest`]   | remote node [`Raft::append_entries()`]        |
| [`vote()`]             | [`VoteRequest`]            | remote node [`Raft::vote()`]                  |
| [`install_snapshot()`] | [`InstallSnapshotRequest`] | remote node [`Raft::install_snapshot()`]      |
| [`full_snapshot()`]    | [`Snapshot`]               | remote node [`Raft::install_full_snapshot()`] |

[Mem KV Network](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft_network_impl.rs)
demonstrates how to forward messages to other Raft nodes using [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) as network transport layer.

To receive and handle these requests, there should be a server endpoint for each of these RPCs.
When the server receives a Raft RPC, it simply passes it to its `raft` instance and replies with the returned result:
[Mem KV Server](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft.rs).

For a real-world implementation, you may want to use [Tonic gRPC](https://github.com/hyperium/tonic) to handle gRPC-based communication between Raft nodes. The [databend-meta](https://github.com/databendlabs/databend/blob/6603392a958ba8593b1f4b01410bebedd484c6a9/metasrv/src/network.rs#L89) project provides an excellent real-world example of a Tonic gRPC-based Raft network implementation.


> ### Troubleshooting: implementation conflicts
>
> When implementing `RaftNetworkV2<T>` for a generic type parameter `T`, you might
> encounter a compiler error about conflicting implementations. This happens
> because Openraft provides a blanket implementation that adapts `RaftNetwork`
> implementations to `RaftNetworkV2`. For example:
>
> ```rust,ignore
> pub trait RaftTypeConfigExt: openraft::RaftTypeConfig {}
> pub struct YourNetworkType {}
> impl<T: RaftTypeConfigExt> RaftNetworkV2<T> for YourNetworkType {}
> ```
>
> You might encounter the following error:
>
> ```text
> conflicting implementations of trait `RaftNetworkV2<_>` for type `YourNetworkType`
> conflicting implementation in crate `openraft`:
> - impl<C, V1> RaftNetworkV2<C> for V1
> ```
>
> If you encounter this error, you can disable the feature `adapt-network-v1` to
> remove the default implementation for `RaftNetworkV2`.


### Implement [`RaftNetworkFactory`].

[`RaftNetworkFactory`] is a singleton responsible for creating [`RaftNetworkV2`] instances for each replication target node.

```ignore
pub trait RaftNetworkFactory<C: RaftTypeConfig>: Send + Sync + 'static {
    type Network: RaftNetwork<C>;
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;
}
```

This trait contains only one method:
- [`RaftNetworkFactory::new_client()`] builds a new [`RaftNetwork`] instance for a target node, intended for sending RPCs to that node.
  The associated type `RaftNetworkFactory::Network` represents the application's implementation of the `RaftNetwork` trait.

This function should **not** establish a connection; instead, it should create a client that connects when
necessary.


### How RaftNetwork and server interact

The [`RaftNetwork`] implementation forwards Raft RPCs to the application-implemented server on another node.
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
|  RaftNetwork -------------------------> Application Server            |
|     ^---------------------------------- /append_entries               |
|  (HTTP/gRPC/etc client)  |    (6)    |  (HTTP/gRPC/etc endpoint)      |
'--------------------------'           '--------------------------------'
    Leader                                 Follower


Flow:
(1) RaftCore triggers ReplicationCore to replicate logs
(2) ReplicationCore calls append_entries on RaftNetwork
(3) RaftNetwork sends RPC request over network (HTTP/gRPC/etc)
(4) Application Server receives request at /append_entries endpoint
(5) Application Server forwards to local Raft::append_entries
(6) Raft::append_entries returns AppendEntriesResponse back through Application Server
(7) RaftNetwork receives the response
(8) ReplicationCore updates RaftCore with replication result
```

The same pattern applies to other RPC methods: [`vote()`], [`install_snapshot()`], etc.

**Example server implementation**:

```rust,ignore
async fn handle_append_entries(
    raft: Arc<Raft<TypeConfig>>,
    req: AppendEntriesRequest<TypeConfig>,
) -> Result<AppendEntriesResponse, ServerError> {
    let resp = raft.append_entries(req).await?;
    Ok(resp)
}
```


### Find the address of the target node.

In Openraft, an implementation of [`RaftNetwork`] needs to connect to remote Raft peers. To store additional information about each peer, you need to specify the `Node` type in `RaftTypeConfig`:

```ignore
pub struct TypeConfig {}
impl openraft::RaftTypeConfig for TypeConfig {
   // ...
   type Node = BasicNode;
}
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

Finally, we put these parts together and boot up a raft node
[main.rs](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/lib.rs)
:

```ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
);

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let node_id = 1;
    let config = Arc::new(Config::default().validate().unwrap());

    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());
    let network = Network {};

    let raft = openraft::Raft::new(
        node_id,
        config.clone(),
        network,
        log_store.clone(),
        state_machine_store.clone(),
    )
    .await
    .unwrap();

    let app_data = Data::new(App {
        id: node_id,
        addr: "127.0.0.1:9999".to_string(),
        raft,
        log_store,
        state_machine_store,
        config,
    });

    HttpServer::new(move || {
      App::new()
              .wrap(Logger::default())
              .wrap(Logger::new("%a %{User-Agent}i"))
              .wrap(middleware::Compress::default())
              .app_data(app.clone())

              // raft internal RPC
              .service(raft::append).service(raft::snapshot).service(raft::vote)

              // admin API
              .service(management::init)
              .service(management::add_learner)
              .service(management::change_membership)
              .service(management::metrics)
              .service(management::list_nodes)

              // application API
              .service(api::write).service(api::read)
    })
    .bind(options.http_addr)?
    .run()
    .await
}

```

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

- [test_cluster.rs](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/tests/cluster/test_cluster.rs)
  uses the `ExampleClient` to set up a cluster, write data, and read it back.


[`declare_raft_types!`]:                `crate::declare_raft_types`
[`Raft`]:                               `crate::Raft`
[`Raft::append_entries()`]:             `crate::Raft::append_entries`
[`Raft::vote()`]:                       `crate::Raft::vote`
[`Raft::install_full_snapshot()`]:      `crate::Raft::install_full_snapshot`
[`Raft::install_snapshot()`]:           `crate::Raft::install_snapshot`

[`AppendEntriesRequest`]:               `crate::raft::AppendEntriesRequest`
[`VoteRequest`]:                        `crate::raft::VoteRequest`
[`InstallSnapshotRequest`]:             `crate::raft::InstallSnapshotRequest`

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

[`LogId`]:                              `crate::LogId`
[`Membership`]:                         `crate::Membership`
[`EmptyNode`]:                          `crate::EmptyNode`
[`BasicNode`]:                          `crate::BasicNode`
[`Entry`]:                              `crate::entry::Entry`
[`Vote`]:                               `crate::vote::Vote`
[`LogState`]:                           `crate::storage::LogState`

[`RaftLogReader`]:                      `crate::storage::RaftLogReader`
[`try_get_log_entries()`]:              `crate::storage::RaftLogReader::try_get_log_entries`
[`read_vote()`]:                        `crate::storage::RaftLogReader::read_vote`



[`RaftLogStorage`]:                     `crate::storage::RaftLogStorage`
[`RaftLogStorage::LogReader`]:          `crate::storage::RaftLogStorage::LogReader`
[`append()`]:                           `crate::storage::RaftLogStorage::append`
[`truncate()`]:                         `crate::storage::RaftLogStorage::truncate`
[`purge()`]:                            `crate::storage::RaftLogStorage::purge`
[`save_vote()`]:                        `crate::storage::RaftLogStorage::save_vote`
[`get_log_state()`]:                    `crate::storage::RaftLogStorage::get_log_state`
[`get_log_reader()`]:                   `crate::storage::RaftLogStorage::get_log_reader`

[`RaftStateMachine`]:                   `crate::storage::RaftStateMachine`
[`RaftStateMachine::SnapshotBuilder`]:  `crate::storage::RaftStateMachine::SnapshotBuilder`
[`applied_state()`]:                    `crate::storage::RaftStateMachine::applied_state`
[`apply()`]:                            `crate::storage::RaftStateMachine::apply`
[`get_current_snapshot()`]:             `crate::storage::RaftStateMachine::get_current_snapshot`
[`begin_receiving_snapshot()`]:         `crate::storage::RaftStateMachine::begin_receiving_snapshot`
[`install_snapshot()`]:                 `crate::storage::RaftStateMachine::install_snapshot`
[`get_snapshot_builder()`]:             `crate::storage::RaftStateMachine::get_snapshot_builder`

[`RaftNetworkFactory`]:                 `crate::network::RaftNetworkFactory`
[`RaftNetwork`]:                        `crate::network::RaftNetwork`
[`RaftNetworkFactory::new_client()`]:   `crate::network::RaftNetworkFactory::new_client`
[`append_entries()`]:                   `crate::RaftNetwork::append_entries`
[`vote()`]:                             `crate::RaftNetwork::vote`
[`install_snapshot()`]:                 `crate::RaftNetwork::install_snapshot`
[`full_snapshot()`]:                    `crate::network::v2::RaftNetworkV2::full_snapshot`
[`RaftNetworkV2`]:                      `crate::network::v2::RaftNetworkV2`


[`RaftSnapshotBuilder`]:                `crate::storage::RaftSnapshotBuilder`
[`build_snapshot()`]:                   `crate::storage::RaftSnapshotBuilder::build_snapshot`
[`Snapshot`]:                           `crate::storage::Snapshot`

[`StoreBuilder`]:                       `crate::testing::log::StoreBuilder`
[`LogSuite`]:                              `crate::testing::log::Suite`

[`Fatal`]:                              `crate::error::Fatal`
[`Unreachable`]:                        `crate::error::Unreachable`

[`docs::connect-to-correct-node`]:      `crate::docs::cluster_control::dynamic_membership#ensure-connection-to-the-correct-node`
