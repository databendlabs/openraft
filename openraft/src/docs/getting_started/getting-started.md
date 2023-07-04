# Getting Started with Openraft

In this chapter, we will build a key-value store cluster using Openraft.

1. [examples/raft-kv-memstore](https://github.com/datafuselabs/openraft/tree/main/examples/raft-kv-memstore)
   is a complete example application that includes the server, client, and a demo cluster. This example uses an in-memory store for data storage.

1. [examples/raft-kv-rocksdb](https://github.com/datafuselabs/openraft/tree/main/examples/raft-kv-rocksdb)
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
state machine implementation in [`RaftStorage`].


## 2. Define types for the application

Openraft is a generic implementation of Raft. It requires the application to define
concrete types for its generic arguments. Most types are parameterized by
[`RaftTypeConfig`] and will be used to create a `Raft` instance:

```ignore
pub struct TypeConfig {}
impl openraft::RaftTypeConfig for TypeConfig {
    type D = Request;
    type R = Response;
    type NodeId = NodeId;
    type Node = BasicNode;
    type Entry = openraft::Entry<TypeConfig>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = TokioRuntime;
}
```

```ignore
pub struct Raft<C: RaftTypeConfig, N, LS, SM> {}
```

Openraft provides default implementations for `Node` ([`EmptyNode`] and [`BasicNode`]) and log `Entry` ([`Entry`]).
You can use these implementations directly or define your own custom types.

A [`RaftTypeConfig`] is also used by other components such as [`RaftStorage`], [`RaftNetworkFactory`] and [`RaftNetwork`].


## 3. Implement [`RaftStorage`]

The trait [`RaftStorage`] defines how data is stored and consumed.
It could be a wrapper for a local key-value store like [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/).

There is a good example,
[`Mem KV Store`](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/store/mod.rs),
that demonstrates what should be done when a method is called. The list of [`RaftStorage`] methods is shown below.
Follow the link to method document to see the details.

| Kind       | [`RaftStorage`] method           | Return value                 | Description                           |
|------------|----------------------------------|------------------------------|---------------------------------------|
| Read log:  | [`get_log_reader()`]             | impl [`RaftLogReader`]       | get a read-only log reader            |
|            |                                  | ↳ [`try_get_log_entries()`]  | get a range of logs                   |
|            | [`get_log_state()`]              | [`LogState`]                 | get first/last log id                 |
| Write log: | [`append_to_log()`]              | ()                           | append logs                           |
| Write log: | [`delete_conflict_logs_since()`] | ()                           | delete logs `[index, +oo)`            |
| Write log: | [`purge_logs_upto()`]            | ()                           | purge logs `(-oo, index]`             |
| Vote:      | [`save_vote()`]                  | ()                           | save vote                             |
| Vote:      | [`read_vote()`]                  | [`Vote`]                     | read vote                             |
| SM:        | [`last_applied_state()`]         | [`LogId`], [`Membership`]    | get last applied log id, membership   |
| SM:        | [`apply_to_state_machine()`]     | Vec of [`AppDataResponse`]   | apply logs to state machine           |
| Snapshot:  | [`begin_receiving_snapshot()`]   | `SnapshotData`               | begin to install snapshot             |
| Snapshot:  | [`install_snapshot()`]           | ()                           | install snapshot                      |
| Snapshot:  | [`get_current_snapshot()`]       | [`Snapshot`]                 | get current snapshot                  |
| Snapshot:  | [`get_snapshot_builder()`]       | impl [`RaftSnapshotBuilder`] | get a snapshot builder                |
|            |                                  | ↳ [`build_snapshot()`]       | build a snapshot from state machine   |

Most of the APIs are quite straightforward, except two indirect APIs:

-   Read logs:
    [`RaftStorage`] defines a method [`get_log_reader()`] to get log reader [`RaftLogReader`] :

    ```ignore
    trait RaftStorage<C: RaftTypeConfig> {
        type LogReader: RaftLogReader<C>;
        async fn get_log_reader(&mut self) -> Self::LogReader;
    }
    ```

    [`RaftLogReader`] defines the APIs to read logs, and is an also super trait of [`RaftStorage`] :
    - [`try_get_log_entries()`] get log entries in a range;

    ```ignore
    trait RaftLogReader<C: RaftTypeConfig> {
        async fn try_get_log_entries<RB: RangeBounds<u64>>(&mut self, range: RB) -> Result<Vec<C::Entry>, ...>;
    }
    ```

    And [`RaftStorage::get_log_state()`][`get_log_state()`] get latest log state from the storage;

-   Build a snapshot from the local state machine needs to be done in two steps:
    - [`RaftStorage::get_snapshot_builder() -> Self::SnapshotBuilder`][`get_snapshot_builder()`],
    - [`RaftSnapshotBuilder::build_snapshot() -> Result<Snapshot>`][`build_snapshot()`],


### Ensure the implementation of RaftStorage is correct

There is a [Test suite for RaftStorage][`Suite`] available in Openraft. If your implementation passes the tests, Openraft should work well with it. To test your implementation, you have two options:

1. Run `Suite::test_all()` with an `async fn()` that creates a new [`RaftStorage`], as shown in the [`MemStore` test](https://github.com/datafuselabs/openraft/blob/main/memstore/src/test.rs):

  ```ignore
  #[test]
  pub fn test_mem_store() -> anyhow::Result<()> {
    openraft::testing::Suite::test_all(MemStore::new_async)
  }
  ```

2. Alternatively, run `Suite::test_all()` with a [`StoreBuilder`] implementation, as shown in the [`RocksStore` test](https://github.com/datafuselabs/openraft/blob/main/rocksstore/src/test.rs).

By following either of these approaches, you can ensure that your custom storage implementation can work correctly in a distributed system.


### An implementation has to guarantee data durability.

The caller always assumes a completed write is persistent.
The raft correctness highly depends on a reliable store.


## 4. Implement [`RaftNetwork`]

Raft nodes need to communicate with each other to achieve consensus about the logs.
The trait [`RaftNetwork`] defines the data transmission requirements.

```ignore
pub trait RaftNetwork<C: RaftTypeConfig>: Send + Sync + 'static {
    async fn send_append_entries(&mut self, rpc: AppendEntriesRequest<C>) -> Result<...>;
    async fn send_install_snapshot(&mut self, rpc: InstallSnapshotRequest<C>) -> Result<...>;
    async fn send_vote(&mut self, rpc: VoteRequest<C::NodeId>) -> Result<...>;
}
```

An implementation of [`RaftNetwork`] can be considered as a wrapper that invokes
the corresponding methods of a remote [`Raft`]. It is responsible for sending
and receiving messages between Raft nodes.

Here is the list of methods that need to be implemented for the [`RaftNetwork`] trait:


| [`RaftNetwork`] method       | forward request            | to target                                |
|------------------------------|----------------------------|------------------------------------------|
| [`send_append_entries()`]    | [`AppendEntriesRequest`]   | remote node [`Raft::append_entries()`]   |
| [`send_install_snapshot()`]  | [`InstallSnapshotRequest`] | remote node [`Raft::install_snapshot()`] |
| [`send_vote()`]              | [`VoteRequest`]            | remote node [`Raft::vote()`]             |

[Mem KV Network](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft_network_impl.rs)
demonstrates how to forward messages to other Raft nodes using [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) as network transport layer.

To receive and handle these requests, there should be a server endpoint for each of these RPCs.
When the server receives a Raft RPC, it simply passes it to its `raft` instance and replies with the returned result:
[Mem KV Server](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft.rs).

For a real-world implementation, you may want to use [Tonic gRPC](https://github.com/hyperium/tonic) to handle gRPC-based communication between Raft nodes. The [databend-meta](https://github.com/datafuselabs/databend/blob/6603392a958ba8593b1f4b01410bebedd484c6a9/metasrv/src/network.rs#L89) project provides an excellent real-world example of a Tonic gRPC-based Raft network implementation.


### Implement [`RaftNetworkFactory`]

[`RaftNetworkFactory`] is a singleton responsible for creating [`RaftNetwork`] instances for each replication target node.

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


## 5. Put everything together

Finally, we put these parts together and boot up a raft node
[main.rs](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/lib.rs)
:

```ignore
// Define the types used in the application.
pub struct TypeConfig {}
impl openraft::RaftTypeConfig for TypeConfig {
    type D = Request;
    type R = Response;
    type NodeId = NodeId;
    type Node = BasicNode;
    type Entry = openraft::Entry<TypeConfig>;
    type SnapshotData = Cursor<Vec<u8>>;
}

#[tokio::main]
async fn main() {
  #[actix_web::main]
  async fn main() -> std::io::Result<()> {
    // Setup the logger
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    // Parse the parameters passed by arguments.
    let options = Opt::parse();
    let node_id = options.id;

    // Create a configuration for the raft instance.
    let config = Arc::new(Config::default().validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = Arc::new(ExampleStore::default());

    let (log_store, state_machine) = Adaptor::new(store.clone());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Arc::new(ExampleNetwork {});

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config.clone(), network, log_store, state_machine).await.unwrap();

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let app = Data::new(ExampleApp {
      id: options.id,
      raft,
      store,
      config,
    });

    // Start the actix-web server.
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
}

```

## 6. Run the cluster

To set up a demo Raft cluster, follow these steps:

1. Bring up three uninitialized Raft nodes.
1. Initialize a single-node cluster.
1. Add more Raft nodes to the cluster.
1. Update the membership configuration.

The [examples/raft-kv-memstore](https://github.com/datafuselabs/openraft/tree/main/examples/raft-kv-memstore)
directory provides a detailed description of these steps.

Additionally, two test scripts for setting up a cluster are available:

- [test-cluster.sh](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/test-cluster.sh)
  is a minimal Bash script that uses `curl` to communicate with the Raft
  cluster. It demonstrates the plain HTTP messages being sent and received.

- [test_cluster.rs](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/tests/cluster/test_cluster.rs)
  uses the `ExampleClient` to set up a cluster, write data, and read it back.

[`Raft`]:                           `crate::Raft`
[`Raft::append_entries()`]:           `crate::Raft::append_entries`
[`Raft::vote()`]:                     `crate::Raft::vote`
[`Raft::install_snapshot()`]:         `crate::Raft::install_snapshot`

[`AppendEntriesRequest`]:           `crate::raft::AppendEntriesRequest`
[`VoteRequest`]:                    `crate::raft::VoteRequest`
[`InstallSnapshotRequest`]:         `crate::raft::InstallSnapshotRequest`

[`AppData`]:                        `crate::AppData`
[`AppDataResponse`]:                `crate::AppDataResponse`
[`RaftTypeConfig`]:                 `crate::RaftTypeConfig`
[`LogId`]:                          `crate::LogId`
[`Membership`]:                     `crate::Membership`
[`EmptyNode`]:                      `crate::EmptyNode`
[`BasicNode`]:                      `crate::BasicNode`
[`Entry`]:                          `crate::entry::Entry`
[`docs::Vote`]:                     `crate::docs::data::Vote`
[`Vote`]:                           `crate::vote::Vote`
[`LogState`]:                       `crate::storage::LogState` 

[`RaftLogReader`]:                  `crate::storage::RaftLogReader`
[`try_get_log_entries()`]:          `crate::storage::RaftLogReader::try_get_log_entries`

[`RaftStorage`]:                    `crate::storage::RaftStorage`
[`get_log_state()`]:                `crate::storage::RaftStorage::get_log_state`
[`RaftStorage::LogReader`]:         `crate::storage::RaftStorage::LogReader`
[`RaftStorage::SnapshotBuilder`]:   `crate::storage::RaftStorage::SnapshotBuilder`
[`get_log_reader()`]:               `crate::storage::RaftStorage::get_log_reader`
[`save_vote()`]:                    `crate::storage::RaftStorage::save_vote`
[`read_vote()`]:                    `crate::storage::RaftStorage::read_vote`
[`append_to_log()`]:                `crate::storage::RaftStorage::append_to_log`
[`delete_conflict_logs_since()`]:   `crate::storage::RaftStorage::delete_conflict_logs_since`
[`purge_logs_upto()`]:              `crate::storage::RaftStorage::purge_logs_upto`
[`last_applied_state()`]:           `crate::storage::RaftStorage::last_applied_state`
[`apply_to_state_machine()`]:       `crate::storage::RaftStorage::apply_to_state_machine`
[`get_current_snapshot()`]:         `crate::storage::RaftStorage::get_current_snapshot`
[`begin_receiving_snapshot()`]:     `crate::storage::RaftStorage::begin_receiving_snapshot`
[`install_snapshot()`]:             `crate::storage::RaftStorage::install_snapshot`
[`get_snapshot_builder()`]:         `crate::storage::RaftStorage::get_snapshot_builder`

[`RaftNetworkFactory`]:             `crate::network::RaftNetworkFactory`
[`RaftNetworkFactory::new_client()`]: `crate::network::RaftNetworkFactory::new_client`
[`RaftNetwork`]:                    `crate::network::RaftNetwork`
[`send_append_entries()`]:    `crate::RaftNetwork::send_append_entries`
[`send_vote()`]:              `crate::RaftNetwork::send_vote`
[`send_install_snapshot()`]:  `crate::RaftNetwork::send_install_snapshot`


[`RaftSnapshotBuilder`]:            `crate::storage::RaftSnapshotBuilder`
[`build_snapshot()`]:                 `crate::storage::RaftSnapshotBuilder::build_snapshot`
[`Snapshot`]:                         `crate::storage::Snapshot`

[`StoreBuilder`]:                   `crate::testing::StoreBuilder`
[`Suite`]:                          `crate::testing::Suite`
