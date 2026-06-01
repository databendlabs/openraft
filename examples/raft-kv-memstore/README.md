# Example distributed key-value store built upon openraft.

It is an example of how to build a real-world key-value store with `openraft`.
Includes:
- An in-memory `RaftLogStorage` and `RaftStateMachine` implementation [store](./src/store/store.rs).

- The application/admin routes are plain async functions hosted by this example's HTTP server.
  All service endpoints accept a JSON request body,
  and return a JSON-encoded `Result<T, E>` response,
  where `T` and `E` are API-specific types.
  For example, the `/write` endpoint accepts a user-defined `Request`
  and returns a `Result<WriteResponse, ClientWriteError>`.

  Includes:
  - Admin APIs to add nodes, change membership etc.
  - Application APIs to write a value by key or read a value by key.
  - Linearizable read on the leader.

- Application HTTP client/server helpers are provided by [`app-http`](../app-http/). Raft node-to-node HTTP is provided by [`network-v2-http`](../network-v2-http/) on a separate internal address.

  [`app_http::Client`](../app-http/src/client.rs) is a minimal client in Rust to talk to a raft cluster.
  - It includes application API `write()`, `read()`, `linearizable_read()`, and administrative API `init()`, `add_learner()`, `change_membership()`, `metrics()`.

## Run it

There is a example in bash script and an example in rust:

- [test-cluster.sh](./test-cluster.sh) shows a simulation of 3 nodes running and sharing data,
  It only uses `curl` and shows the communication between a client and the cluster in plain HTTP messages.
  You can run the cluster demo with:

  ```shell
  ./test-cluster.sh
  ```

- [test_cluster.rs](./tests/cluster/test_cluster.rs) does almost the same as `test-cluster.sh` but in Rust
  with `app_http::Client`.

  Run it with `cargo test`.


if you want to compile the application, run:

```shell
cargo build
```

(If you append `--release` to make it compile in production, but we don't recommend to use
this project in production yet.)

## What the test script does

To run it, get the binary `raft-key-value` inside `target/debug` and run:

```shell
./raft-key-value --id 1 --api-addr 127.0.0.1:21001 --raft-addr 127.0.0.1:22001
```

It will start a node.

To start the following nodes:

```shell
./raft-key-value --id 2 --api-addr 127.0.0.1:21002 --raft-addr 127.0.0.1:22002
```

You can continue replicating the nodes by changing the `id`, `api-addr`, and `raft-addr`.

After that, call the first node created:

```
POST - 127.0.0.1:21001/init
```

It will define the first node created as the leader.

Then you need to inform to the leader that these nodes are learners:

```
POST - 127.0.0.1:21001/add-learner '{"node_id":2,"api_addr":"127.0.0.1:21002","raft_addr":"127.0.0.1:22002"}'
POST - 127.0.0.1:21001/add-learner '{"node_id":3,"api_addr":"127.0.0.1:21003","raft_addr":"127.0.0.1:22003"}'
```

Now you need to tell the leader to add all learners as members of the cluster:

```
POST - 127.0.0.1:21001/change-membership  "[1, 2, 3]"
```

Write some data through the leader:

```
POST - 127.0.0.1:21001/write  "{"Set":{"key":"foo","value":"bar"}}"
```

Read the data from any node:

```
POST - 127.0.0.1:21002/read  "foo"
```

You should be able to read that on the another instance even if you did not sync any data!

For linearizable reads, use the `/linearizable_read` endpoint on the leader:

```
POST - 127.0.0.1:21001/linearizable_read  "foo"
```


## How it's structured.

The application is separated into these parts:

 - `bin`: You can find the `main()` function in [main](./src/bin/main.rs) the file where the setup for the server happens.
 - `network`: You can find the [api](./src/network/api.rs) that implements the read endpoints. Application HTTP client/server helpers and common management/write handlers are provided by [`app-http`](../app-http/). Raft node-to-node HTTP communication is provided by [`network-v2-http`](../network-v2-http/).
 - `store`: You can find the file [store](./src/store/mod.rs) where all the key-value implementation is done. Here is where your data application will be managed.

## Where is my data?

The data is store inside state machines, each state machine represents a point of data and
raft enforces that all nodes have the same data in synchronization. You can have a look of
the struct [ExampleStateMachine](./src/store/mod.rs)

## Cluster management

OpenRaft stores the node metadata supplied by the application.
The Raft network uses `raft_addr` to contact other raft nodes.

Thus, in this example application:

- OpenRaft node metadata is `NodeInfo`, whose `raft_addr` is the Raft RPC address and `data` stores the application API address.
- The local application server has a separate `api_addr` for public/admin/application APIs.

To add a node to a cluster, it includes 3 steps:

- Write a `node` through raft protocol to the storage.
- Add the node as a `Learner` to let it start receiving replication data from the leader.
- Invoke `change-membership` to change the learner node to a member.
