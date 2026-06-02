# Example distributed key-value store built upon openraft.

It is an example of how to build a real-world key-value store with `openraft`.
Includes:
- An in-memory `RaftLogStorage` ([`log-mem`](../log-mem/)) and `RaftStateMachine` ([`sm-mem`](../sm-mem/)) implementation.

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

This `raft-kv-memstore` crate only wires the pieces together. The reusable
building blocks — storage, network, and request types — live in sibling helper
crates, keeping the split between what OpenRaft needs and what the application
provides visible.

In this crate:

 - [`src/lib.rs`](./src/lib.rs): `start_example_raft_node()` builds the `TypeConfig`,
   log store, state machine, and network factory, calls `Raft::new()`, then registers
   the HTTP routes and starts both servers.
 - [`src/bin/main.rs`](./src/bin/main.rs): CLI entry point.
 - [`src/http_api.rs`](./src/http_api.rs): the application read endpoints
   (`/read`, `/linearizable_read`, `/follower_read`).
 - [`src/store/mod.rs`](./src/store/mod.rs): re-exports the log and state-machine
   stores from the helper crates below — no storage logic lives here.

Helper crates (reused across the examples):

 - [`log-mem`](../log-mem/src/log_store.rs): the in-memory `RaftLogStorage`.
 - [`sm-mem`](../sm-mem/src/lib.rs): the in-memory `RaftStateMachine` and snapshot
   handling; the key-value data lives here.
 - [`types-kv`](../types-kv/src/lib.rs): the application request/response types
   (`D` and `R` in `TypeConfig`).
 - [`network-v2-http`](../network-v2-http/): node-to-node Raft RPC —
   [`client.rs`](../network-v2-http/src/client.rs) is the outbound `RaftNetworkV2`
   (the network factory), [`server.rs`](../network-v2-http/src/server.rs) serves
   the inbound RPCs.
 - [`app-http`](../app-http/): the application HTTP server and client, plus the
   shared admin/write routes added by `add_openraft_routes()`.

## Where is my data?

The data is store inside state machines, each state machine represents a point of data and
raft enforces that all nodes have the same data in synchronization. You can have a look of
the [`StateMachineStore`](../sm-mem/src/lib.rs) struct.

## Cluster management

To bring a node into the cluster you hand OpenRaft that node's `NodeInfo`, which
carries the two addresses needed to reach it:

- `raft_addr` — the address OpenRaft dials for Raft RPCs. OpenRaft holds each
  node's `NodeInfo` and passes `raft_addr` to the network factory when it needs to
  contact the node, so your code never resolves Raft addresses itself.
- `data` — the node's `api_addr`, the application API address clients use to read
  and write. It travels with the node through Raft membership, so every node knows
  where each peer's API lives.

Forming and growing the cluster is three calls (see [test-cluster.sh](./test-cluster.sh)):

- `init` creates the initial cluster from a set of `(node_id, NodeInfo)` entries.
- `add-learner` registers a new node's `NodeInfo` and starts streaming log entries
  to it as a non-voting learner.
- `change-membership` promotes those learners into voting members.
