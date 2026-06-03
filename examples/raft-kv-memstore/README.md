# Example distributed key-value store built upon openraft.

It is an example of how to build a real-world key-value store with `openraft`.
Includes:
- An in-memory `RaftLogStorage` ([`log-mem`](../log-mem/)) and `RaftStateMachine` ([`sm-mem`](../sm-mem/)) implementation.

- The application/admin routes are plain async functions, each taking and
  returning JSON. Includes:
  - Admin APIs to add nodes, change membership etc.
  - Application APIs to write a value by key or read a value by key.
  - Linearizable read on the leader.

- Application HTTP client/server helpers are provided by [`app-http`](../app-http/);
  node-to-node Raft RPC by [`network-v2-http`](../network-v2-http/) on a separate
  internal address. [`app_http::Client`](../app-http/src/client.rs) is a minimal
  Rust client covering all of the calls above.

## Build an OpenRaft application

To run an application on OpenRaft you supply a handful of pieces; this example
wires them together in [`src/lib.rs`](./src/lib.rs). The checklist, and where each
piece lives:

1. Define a `RaftTypeConfig` — the associated types for your app (request,
   response, node). Declared with `declare_raft_types!` in
   [`src/lib.rs`](./src/lib.rs); the request/response types come from
   [`types-kv`](../types-kv/src/lib.rs).
2. Choose a `RaftLogStorage` — where the Raft log is kept. Here, the in-memory
   [`log-mem`](../log-mem/src/log_store.rs).
3. Choose a `RaftStateMachine` — applies committed entries and holds your data.
   Here, the in-memory [`sm-mem`](../sm-mem/src/lib.rs).
4. Provide a `RaftNetworkFactory` — how a node opens connections to its peers.
   Here, [`network-v2-http`](../network-v2-http/src/client.rs).
5. Call `Raft::new()` — passing the node id, config, network factory, log store,
   and state machine. The returned `Raft` is your handle to the cluster.
6. Serve inbound Raft RPCs — run a server on the Raft address so peers can reach
   this node ([`network-v2-http`](../network-v2-http/src/server.rs)).
7. Serve your application API — run a server on the API address for client and
   admin requests ([`app-http`](../app-http/)).

Steps 1–5 build the OpenRaft node; steps 6–7 are how peers and clients reach it.

## What OpenRaft requires vs. what the example adds

OpenRaft defines a few traits you must implement; everything else here — the HTTP
transport, the application API, the demo — is this example's choice and would be
replaced in a real deployment.

| Concern | OpenRaft's contract | Example's implementation (replace for production) |
|---------|---------------------|---------------------------------------------------|
| Type configuration | `RaftTypeConfig` | `TypeConfig`, [`types-kv`](../types-kv/) request/response types |
| Log storage | `RaftLogStorage` | [`log-mem`](../log-mem/src/log_store.rs) (in-memory) |
| State machine | `RaftStateMachine` | [`sm-mem`](../sm-mem/src/lib.rs) (in-memory; holds the KV data) |
| Outbound network | `RaftNetworkFactory` | [`network-v2-http`](../network-v2-http/src/client.rs) client (HTTP) |
| Inbound RPC server | — *you route RPCs into `Raft`* | [`network_v2_http::Server`](../network-v2-http/src/server.rs) on `raft_addr` |
| Client & admin API | — *not OpenRaft's concern* | [`app-http`](../app-http/) server/client on `api_addr` |
| Bootstrap & demo | — | [`test-cluster.sh`](./test-cluster.sh), integration tests |

The left column is the part you cannot avoid; the right column is what makes this
a runnable demo rather than a library.

## Two servers per node

Each node runs two HTTP listeners at once —
[`start_example_raft_node()`](./src/lib.rs) starts both with `tokio::try_join!` —
on two separate addresses:

| Server | Address | Handles |
|--------|---------|---------|
| [`network_v2_http::Server`](../network-v2-http/src/server.rs) | `raft_addr` | Raft protocol RPCs between nodes: `/append`, `/vote`, `/snapshot`, `/transfer-leader` |
| [`app_http::Server`](../app-http/src/server.rs) | `api_addr` | client and admin APIs: `/init`, `/add-learner`, `/write`, `/read`, … |

The split keeps internal replication traffic and external client traffic on
different ports: peers only ever reach the Raft server, clients only ever reach
the app server. The write and read flows below enter on the app server;
replication between nodes rides the Raft server.

## How a write flows

A `/write` request travels from the client, through the application server, into
OpenRaft, and out to each node's state machine:

```text
client
  -> app_http::Server /write
  -> app_http::App::write()
  -> Raft::client_write()
  -> Raft log replication
  -> StateMachineStore::apply()
  -> key-value data update
```

[`App::write()`](../app-http/src/app.rs) only forwards the request to
`Raft::client_write()`. OpenRaft appends it to the log, replicates it to a
quorum, and once the entry is committed calls
[`StateMachineStore::apply()`](../sm-mem/src/lib.rs), which applies
`Set { key, value }` to the in-memory map. `client_write()` returns only after
the entry is applied, so the client sees committed state.

## How a read flows

A read does not have to go through the log. The example offers three read
endpoints, trading latency for consistency:

- **`/read`** reads this node's local state machine and returns immediately.
  Fast, but possibly stale: a follower can lag the leader, so the value may
  predate the latest committed write. ([`read()`](./src/http_api.rs))

- **`/linearizable_read`** runs on the leader. It first calls
  [`ensure_linearizable()`](../app-http/src/app.rs) — confirming leadership and
  waiting until the leader's commit index is applied — then reads locally, so it
  returns the latest committed value.

- **`/follower_read`** lets a follower serve a linearizable read. Via
  [`ensure_follower_read_ready()`](../app-http/src/app.rs) it asks the leader for
  a read marker (the committed log id), waits until its own state machine has
  applied up to that marker, then reads locally — consistent reads without
  routing every read to the leader.

## Run it

Two runnable demos drive the same 3-node happy path:

- [`test-cluster.sh`](./test-cluster.sh) starts three nodes and drives them with
  `curl`, so you can watch the raw HTTP exchange:

  ```shell
  ./test-cluster.sh
  ```

- The [integration tests](./tests/cluster/) do the same in Rust through
  [`app_http::Client`](../app-http/src/client.rs):

  ```shell
  cargo test
  ```

Both follow the same six steps. To drive it by hand, build with `cargo build`,
start each node with the binary, and POST to its API address:

```shell
./target/debug/raft-key-value --id 1 --api-addr 127.0.0.1:21001 --raft-addr 127.0.0.1:22001
```

1. Start three nodes (ids 1–3, each with its own `--api-addr` and `--raft-addr`).
2. `POST /init` on node 1 — it becomes the leader of a single-node cluster.
3. `POST /add-learner` for nodes 2 and 3 — they begin receiving log replication.
4. `POST /change-membership [1, 2, 3]` — the learners become voters.
5. `POST /write` a key on the leader.
6. `POST /read` it from any node.

See [`test-cluster.sh`](./test-cluster.sh) for the exact request bodies, and
[Cluster management](#cluster-management) for what the admin calls do.

## How it's structured

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

Your data lives in the state machine: each node keeps its own copy, and Raft
keeps those copies identical by applying the same committed log entries in the
same order on every node. See the [`StateMachineStore`](../sm-mem/src/lib.rs)
struct.

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
