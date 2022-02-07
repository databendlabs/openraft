# Getting Started

In this chapter we are going to build a key-value store cluster with [openraft](https://github.com/datafuselabs/openraft).

[example-raft-kv](https://github.com/datafuselabs/openraft/tree/main/example-raft-kv)
is the complete example application including the server, the client and a demo cluster.

---

Raft is a distributed consensus protocol designed to manage a replicated log containing state machine commands from clients.

<p>
    <img style="max-width:600px;" src="./images/raft-overview.png"/>
</p>


Raft includes two major parts:

- How to replicate logs consistently among nodes,
- and how to consume the logs, which is defined mainly in state machine.

To implement your own raft based application with openraft is quite easy, which
includes:

- Define client request and response;
- Implement a storage to let raft store its state;
- Implement a network layer for raft to transmit messages.

## 1. Define client request and response

A request is some data that modifies the raft state machine.
A response is some data that the raft state machine returns to client.

Request and response can be any types that impl `AppData` and `AppDataResponse`,
e.g.:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExampleRequest {/* fields */}
impl AppData for ExampleRequest {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExampleResponse(Result<Option<String>, ClientError>);
impl AppDataResponse for ExampleResponse {}
```

These two types are totally application specific, and are mainly related to the
state machine implementation in `RaftStorage`.


## 2. Implement `RaftStorage`

The trait `RaftStorage` defines the way that data is stored and consumed.
It could be a wrapper of some local KV store such [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/),
or a wrapper of a remote sql DB.

`RaftStorage` defines 4 sets of APIs an application needs to implement:

- Read/write raft state, e.g., term or vote.
    ```rust
    fn save_vote(vote:&Vote)
    fn read_vote() -> Result<Option<Vote>>
    ```

- Read/write logs.
    ```rust
    fn get_log_state() -> Result<LogState>
    fn try_get_log_entries(range) -> Result<Vec<Entry>>

    fn append_to_log(entries)

    fn delete_conflict_logs_since(since:LogId)
    fn purge_logs_upto(upto:LogId)
    ```

- Apply log entry to the state machine.
    ```rust
    fn last_applied_state() -> Result<(Option<LogId>, Option<EffectiveMembership>)>
    fn apply_to_state_machine(entries) -> Result<Vec<AppResponse>>
    ```

- Building and installing a snapshot.
    ```rust
    fn build_snapshot() -> Result<Snapshot>
    fn get_current_snapshot() -> Result<Option<Snapshot>>

    fn begin_receiving_snapshot() -> Result<Box<SnapshotData>>
    fn install_snapshot(meta, snapshot)
    ```

The APIs have been made quite obvious and there is a good example
[`ExampleStore`](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/src/store/mod.rs),
which is a pure-in-memory implementation that shows what should be done when a
method is called.


### How do I impl RaftStorage correctly

There is a [Test suite for RaftStorage](https://github.com/datafuselabs/openraft/blob/main/memstore/src/test.rs),
if an implementation passes the test, openraft will work happily with it.

To test your implementation with this suite, just do this:

```rust
#[test]
pub fn test_mem_store() -> anyhow::Result<()> {
  openraft::testing::Suite::test_all(MemStore::new)
}
```

### Race condition about RaftStorage

In our design, there is at most one thread at a time writing data to it.
But there may be several threads reading from it concurrently,
e.g., more than one replication tasks reading log entries from store.


### An implementation has to guarantee data durability

The caller always assumes a completed write is persistent.
The raft correctness highly depends on a reliable store.


## 3. impl `RaftNetwork`

Raft nodes need to communicate with each other to achieve consensus about the
logs.
The trait `RaftNetwork` defines the data transmission requirements.

An implementation of `RaftNetwork` can be considered as a wrapper that invokes the
corresponding methods of a remote `Raft`.

```rust
pub trait RaftNetwork<D>: Send + Sync + 'static
where D: AppData
{
    async fn send_append_entries(&self, target: NodeId, rpc: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse>;
    async fn send_install_snapshot( &self, target: NodeId, rpc: InstallSnapshotRequest,) -> Result<InstallSnapshotResponse>;
    async fn send_vote(&self, target: NodeId, rpc: VoteRequest) -> Result<VoteResponse>;
}
```

[ExampleNetwork](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/src/network/raft.rs)
shows that how to forward message to other raft nodes.

And there should be server endpoint for each of these RPCs.
When the server receives a raft RPC, it just passes it to its `raft` instance and replies with what returned:
[raft-server-endpoint](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/src/network/raft.rs).

As a real world impl, you may want to use [Tonic gRPC](https://github.com/hyperium/tonic).
[databend-meta](https://github.com/datafuselabs/databend/blob/6603392a958ba8593b1f4b01410bebedd484c6a9/metasrv/src/network.rs#L89) would be a nice real world example.


## 4. Put everything together

Finally, we put these part together and boot up a raft node
[main.rs](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/src/bin/main.rs)
:

```rust
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

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Arc::new(ExampleNetwork { store: store.clone() });

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, store.clone());

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
              .service(raft::append)
              .service(raft::snapshot)
              .service(raft::vote)
              // admin API
              .service(management::init)
              .service(management::add_learner)
              .service(management::change_membership)
              .service(management::metrics)
              .service(management::list_nodes)
              // application API
              .service(api::write)
              .service(api::read)
    })
            .bind(options.http_addr)?
            .run()
            .await
  }
}

```

## 5. Run the cluster

To set up a demo raft cluster includes:
- Bring up 3 uninitialized raft node;
- Initialize a single-node cluster;
- Add more raft nodes into it;
- Update the membership config.

[example-raft-kv](https://github.com/datafuselabs/openraft/tree/main/example-raft-kv) describes these step in detail.

And two test scripts for setting up cluster are provided:

- [test-cluster.sh](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/test-cluster.sh)
  is a minimized bash script using curl to communicate with the raft cluster,
  in order to show what messages are exactly sent and received in plain HTTP. 

- [test_cluster.rs](https://github.com/datafuselabs/openraft/blob/main/example-raft-kv/tests/cluster/test_cluster.rs)
  uses `ExampleClient` to set up a cluster and write data to it then read it.



