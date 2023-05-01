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
pub struct ExampleRequest {/* fields */}
impl AppData for ExampleRequest {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExampleResponse(Result<Option<String>, ClientError>);
impl AppDataResponse for ExampleResponse {}
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
}
```

```ignore
pub struct Raft<C: RaftTypeConfig, N, LS, SM> {}
```

Openraft provides default implementations for `Node` ([`EmptyNode`] and [`BasicNode`]) and log `Entry` ([`Entry`]).
You can use these implementations directly or define your own custom types.

A [`RaftTypeConfig`] is also used by other components such as [`RaftStorage`], [`RaftNetworkFactory`] and [`RaftNetwork`].


## 3. Implement [`RaftStorage`]

The trait [`RaftStorage: RaftLogReader`][`RaftStorage`] defines how data is
stored and consumed in a Raft-based application. It could be a wrapper for a
local key-value store like [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/).

The APIs are quite straightforward, and there is a good example,
[`Example: MemStore`](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/store/mod.rs),
which is a pure in-memory implementation that demonstrates what should be done
when a method is called.

-   Read logs:
    [`RaftStorage`] defines the API to get log reader [`RaftLogReader`], :
    An application implements associated type [`RaftStorage::LogReader`] and [`get_log_reader()`] to return a read-only log reader:
    - [`get_log_reader() -> Self::LogReader`][`get_log_reader()`],

    [`RaftLogReader`] which defines the APIs to read logs, and is an also super trait of [`RaftStorage`] :
    - [`get_log_state()`] get latest log state from the storage;
    - [`try_get_log_entries()`] get log entries in a range;

-   Write logs:
    [`RaftStorage`] defines the APIs to write logs:
    - [`append_to_log(entries)`][`append_to_log()`],
    - [`delete_conflict_logs_since(since: LogId)`][`delete_conflict_logs_since()`],
    - [`purge_logs_upto(upto: LogId)`][`purge_logs_upto()`];

-   Read/write Raft vote(see: [`docs::Vote`][`Vote`]):
    - [`save_vote(vote: &Vote)`][`save_vote()`];
    - [`read_vote() -> Result<Option<Vote>>`][`read_vote()`];

-   Apply log entry to the state machine.
    - [`last_applied_state() -> Result<(Option<LogId>, Option<EffectiveMembership>)>`][`last_applied_state()`],
    - [`apply_to_state_machine(entries) -> Result<Vec<AppResponse>>`][`apply_to_state_machine()`];

-   Snapshot APIs:
    Build a snapshot from the local state machine:
    - [`get_snapshot_builder() -> Self::SnapshotBuilder`][`get_snapshot_builder()`],
    - [`build_snapshot() -> Result<Snapshot>`][`build_snapshot()`],

    Get the current snapshot:
    - [`get_current_snapshot() -> Result<Option<Snapshot>>`][`get_current_snapshot()`],

    Install a snapshot from leader:
    - [`begin_receiving_snapshot() -> Result<Box<SnapshotData>>`][`begin_receiving_snapshot()`],
    - [`install_snapshot(meta, snapshot)`][`install_snapshot()`];


| Kind       | [`RaftStorage`] method           | Return value                 | Description                           |
|------------|----------------------------------|------------------------------|---------------------------------------|
| Read log:  | [`get_log_reader()`]             | impl [`RaftLogReader`]       | get a read-only log reader            |
|            |                                  | ↳ [`get_log_state()`]        | get first/last log id                 |
|            |                                  | ↳ [`try_get_log_entries()`]  | get a range of logs                   |
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


### Ensure the implementation of RaftStorage is correct


There is a [Test suite for RaftStorage][`Suite`] available. If an implementation passes the test, Openraft will work well with it.
Refer to the [`MemStore` test](https://github.com/datafuselabs/openraft/blob/main/memstore/src/test.rs) for an example.

To test your implementation with this suite, implement a `StoreBuilder` and call `Suite::test_all`:

```ignore
type LogStore = Adaptor<TypeConfig, Arc<MemStore>>;
type StateMachine = Adaptor<TypeConfig, Arc<MemStore>>;

struct MemBuilder {}

#[async_trait]
impl StoreBuilder<TypeConfig, LogStore, StateMachine, ()> for MemBuilder {
    async fn build(&self) -> Result<((), LogStore, StateMachine), StorageError<RocksNodeId>> {
        let store = MemStore::new_async().await;
        let (log_store, state_machine) = Adaptor::new(store);
        Ok(((), log_store, state_machine))
    }
}

#[test]
pub fn test_mem_store() -> anyhow::Result<()> {
  openraft::testing::Suite::test_all(MemBuilder {})
}
```

By implementing the `StoreBuilder` and calling `Suite::test_all` with your custom builder, you can ensure that your implementation of `RaftStorage` is compatible with Openraft.

There is a second example in [Test suite for RaftStorage](https://github.com/datafuselabs/openraft/blob/main/rocksstore/src/test.rs)
that showcases building a rocksdb backed store.

### An implementation has to guarantee data durability.

The caller always assumes a completed write is persistent.
The raft correctness highly depends on a reliable store.


## 4. implement [`RaftNetwork`]

Raft nodes need to communicate with each other to achieve consensus about the logs.
The trait [`RaftNetwork`] defines the data transmission requirements for this communication.

An implementation of [`RaftNetwork`] can be considered as a wrapper that invokes
the corresponding methods of a remote [`Raft`]. It is responsible for sending
and receiving messages between Raft nodes to maintain consistency and achieve
consensus.

Here is the list of methods that need to be implemented for the [`RaftNetwork`] trait:


| [`RaftNetwork`] method       | forward request            | target                                   |
|------------------------------|----------------------------|------------------------------------------|
| [`send_append_entries()`]    | [`AppendEntriesRequest`]   | remote node [`Raft::append_entries()`]   |
| [`send_install_snapshot()`]  | [`InstallSnapshotRequest`] | remote node [`Raft::install_snapshot()`] |
| [`send_vote()`]              | [`VoteRequest`]            | remote node [`Raft::vote()`]             |

[ExampleNetwork](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft_network_impl.rs)
demonstrates how to forward messages to other Raft nodes using a basic in-memory network implementation.

To receive and handle these requests, there should be a server endpoint for each of these RPCs.
When the server receives a Raft RPC, it simply passes it to its `raft` instance and replies with the returned result:
[raft-server-endpoint](https://github.com/datafuselabs/openraft/blob/main/examples/raft-kv-memstore/src/network/raft.rs).

For a real-world implementation, you may want to use [Tonic gRPC](https://github.com/hyperium/tonic) to handle gRPC-based communication between Raft nodes. The [databend-meta](https://github.com/datafuselabs/databend/blob/6603392a958ba8593b1f4b01410bebedd484c6a9/metasrv/src/network.rs#L89) project provides an excellent real-world example of a Tonic gRPC-based Raft network implementation.


### Implement [`RaftNetworkFactory`]

[`RaftNetworkFactory`] is a singleton responsible for creating [`RaftNetwork`] instances for each replication target node.

This trait contains only one method:
- [`RaftNetworkFactory::new_client()`] generates a new [`RaftNetwork`] instance for a target node, intended for sending RPCs to that node.
  The associated type `RaftNetworkFactory::Network` represents the application's implementation of the `RaftNetwork` trait.

This function should **not** establish a connection; instead, it should create a client that connects when
necessary.


### Find the address of the target node.

An implementation of [`RaftNetwork`] needs to connect to the remote Raft peer,
using protocols such as TCP.

There are two ways to find the address of a remote peer:

1. Manage the mapping from node-id to address independently.

2. Openraft allows you to store additional information in its [`Membership`],
   which is automatically replicated as regular logs.

   To use this feature, you need to specify the `Node` type in `RaftTypeConfig`, as follows:

   ```ignore
   pub struct TypeConfig {}
   impl openraft::RaftTypeConfig for TypeConfig {
       // ...
       type Node = BasicNode;
   }
   ```

   Use `Raft::add_learner(node_id, BasicNode::new("127.0.0.1"), ...)` to instruct Openraft
   to store both the node-id and its address in [`Membership`]:

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

[`RaftLogReader`]:                  `crate::storage::RaftLogReader`
[`get_log_state()`]:                `crate::storage::RaftLogReader::get_log_state`
[`try_get_log_entries()`]:          `crate::storage::RaftLogReader::try_get_log_entries`

[`RaftStorage`]:                    `crate::storage::RaftStorage`
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

[`Suite`]:                          `crate::testing::Suite`
