# EzRaft

A beginner-friendly Raft consensus framework built on [OpenRaft](https://github.com/databendlabs/openraft). EzRaft handles all Raft complexity internally - users only provide business logic and storage persistence.

## Overview

Run your application on several machines at once, all holding the same state, so the
service survives losing some of them. That is what [Raft](https://raft.github.io/) is
for, and EzRaft reduces it to two traits - [`EzApp`](#ezapp) holds your state and applies
requests to it, [`EzStorage`](#ezstorage) puts bytes on disk. Elections, replication,
membership, snapshots and the transport between machines are handled internally.

- **Minimal user API**: 4 methods total (3 storage + 1 app) vs 21+ in OpenRaft
- **Smart defaults**: 10/12 Raft types pre-configured, users specify only Request/Response
- **Built-in networking**: HTTP layer included, no user code needed
- **Type-safe**: Works directly with your types, not byte vectors

## Status

**Experimental.** EzRaft is primarily an API design laboratory for exploring intuitive interface patterns. The APIs may change until the crate stabilizes. Production applications are not the primary audience.

**Next phase: Stable API.** Once the design exploration matures, EzRaft will provide a stable API with well-considered abstractions—exposing what users need while hiding unnecessary complexity.

## Run a cluster

Three nodes, three terminals, from the repository root. The first node creates the
cluster; every other node joins through a node already in it:

```bash
cargo run -p ezraft --example kvstore -- --addr 127.0.0.1:8080
cargo run -p ezraft --example kvstore -- --addr 127.0.0.1:8081 --seed 127.0.0.1:8080
cargo run -p ezraft --example kvstore -- --addr 127.0.0.1:8082 --seed 127.0.0.1:8080
```

Each prints the node id the cluster gave it. Drive them from a fourth terminal - the
request body is the example's `Request` enum as JSON:

```bash
# A write goes through the log: replicated and committed before it answers.
curl -X POST 127.0.0.1:8080/api/write -H 'Content-Type: application/json' \
    -d '{"Set": {"key": "hello", "value": "world"}}'
# {"value":null}  - Set answers with the value it replaced, if there was one

# A read is answered from that node's memory: no consensus round, no log entry.
curl '127.0.0.1:8082/api/read?key=hello'
# "world"  - from a node you never wrote to

# Any node accepts a write; a follower forwards it to the leader.
curl -X POST 127.0.0.1:8081/api/write -H 'Content-Type: application/json' \
    -d '{"Set": {"key": "hello", "value": "again"}}'
# {"value":"world"}

curl 127.0.0.1:8080/api/metrics   # leader, term, log index, membership
```

Now kill any one of the three. The two survivors are a majority, so they elect a new
leader within a few heartbeats and keep serving writes. Start the dead one again with the
exact same command line and it rejoins with its old id and its old state: each node keeps
that under `./data/127.0.0.1-8080/`, relative to where you started it.

`examples/kvstore.rs` is all of the above, and the node prints these commands on startup
with its own address filled in.

## What just happened

A *cluster* is a set of *nodes*, each one a process running your application. Every node
keeps the same list of requests - the *log* - in the same order. A request is *committed*
once a majority of the nodes have stored it, which is the point past which it can no
longer be lost. Each node then *applies* committed requests to its own copy of your state,
in log order, which is why all of them end up with the same state. Your `apply` method is
that step.

One node is elected *leader* and is the only one that may accept writes; the rest follow
it, and forward writes to it. Since every node holds the whole state, any node can answer
a read out of memory without asking anyone. A *snapshot* is that state serialized: it
brings a node that fell too far behind back in one transfer instead of a log replay, and
it lets old log entries be deleted. EzRaft builds and installs snapshots on its own.

The rest is five facts about EzRaft in particular:

**One node creates, the rest join.** `EzRaft::create` starts a cluster whose only member
is that node; every other node calls `EzRaft::join` with the address of a node already in
it. Calling `create` on two nodes gives two one-node clusters that never merge.

**Ids are assigned, not configured.** The cluster gives a joining node its id and the node
persists it. Ids come from a log index, so they are unique but not consecutive - a
three-node cluster might end up as 0, 7 and 2. On restart the persisted id is reused and
the seed is not contacted again, so a seed address that has since left is harmless.

**Joining is all it takes to add fault tolerance.** A new node arrives as a *learner* - it
receives the log but does not count towards a majority - and is promoted to *voter*
automatically once it has caught up. Three nodes tolerate one failure and five tolerate
two. An even count buys nothing: four nodes tolerate one failure, the same as three.

**Writes go through the log, reads do not.** `EzRaft::write` returns once the entry is
committed and applied, and forwards to the leader when called on a follower.
`EzRaft::read` runs a closure over local applied state - cheap, and possibly a little
behind. When a read must see everything acknowledged before it, call
`EzRaft::linearizable` first.

**Restart with the same call.** A node that was created starts with `create` again, a node
that joined starts with `join` again; both read their state back from storage. The example
does exactly this - the command line for a restart is the one that started the node.

## Write your own service

This is a complete program - one request type, one response type, one state struct, one
method of business logic:

```rust
use std::collections::BTreeMap;
use std::io;

use async_trait::async_trait;
use ezraft::EzApp;
use ezraft::EzConfig;
use ezraft::EzRaft;
use ezraft::FileStorage;
use serde::Deserialize;
use serde::Serialize;

/// What a client asks the cluster to do. It travels the network and goes into
/// the log, hence serde; openraft prints it in its logs, hence Display.
#[derive(Serialize, Deserialize, Debug, Clone, derive_more::Display)]
enum Request {
    #[display("Set({key})")]
    Set { key: String, value: String },
}

/// What `apply` hands back to whoever issued the write.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Response {
    value: Option<String>,
}

/// The application *is* the replicated state. A snapshot is this struct
/// serialized, so serde is all it takes.
#[derive(Default, Serialize, Deserialize)]
struct KvApp {
    data: BTreeMap<String, String>,
}

#[async_trait]
impl EzApp for KvApp {
    type Request = Request;
    type Response = Response;

    /// Called once per committed entry, in log order, on every node.
    async fn apply(&mut self, req: Request) -> Response {
        match req {
            Request::Set { key, value } => Response {
                value: self.data.insert(key, value),
            },
        }
    }

    /// Optional: answers `GET /api/read?key=...` from local state.
    fn read(&self, key: &str) -> Option<serde_json::Value> {
        self.data.get(key).map(|v| serde_json::Value::String(v.clone()))
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // One directory per node. `FileStorage` is the bundled `EzStorage`.
    let storage = FileStorage::new("./data/node1").await?;

    // The first node of a cluster creates it; every other node joins through
    // any node already in it:
    //   EzRaft::join("127.0.0.1:8081", "127.0.0.1:8080", app, storage, config)
    let raft = EzRaft::create("127.0.0.1:8080", KvApp::default(), storage, EzConfig::default()).await?;

    raft.write(Request::Set {
        key: "hello".to_string(),
        value: "world".to_string(),
    })
    .await?;
    println!("{:?}", raft.read(|app| app.data.get("hello").cloned()).await);

    // Serves the Raft RPCs peers need, plus the app API. This blocks;
    // `tokio::spawn` it when the caller has other work to do.
    raft.serve().await
}
```

## User Traits

### EzApp

The application itself: the implementing type is the replicated state, and
`apply` is the business logic. Snapshots are derived from the state via serde,
so there is nothing to implement for them.

```rust
#[async_trait]
pub trait EzApp: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Request: ...;
    type Response: ...;

    async fn apply(&mut self, req: Self::Request) -> Self::Response;

    // Optional; powers GET /api/read?key=..., declines every key by default.
    fn read(&self, key: &str) -> Option<serde_json::Value> { None }
}
```

**Framework handles**: Sequential application, snapshot build/restore and scheduling

**You handle**: Business logic

Because a snapshot is the whole state serialized, the state has to fit in memory. That
suits the coordination/metadata class of application (ZooKeeper snapshots the same way);
an application whose snapshot is a streamed checkpoint of something larger builds on
openraft directly.

### EzStorage

Handles persistence of Raft state (metadata, logs, snapshots).

```rust
#[async_trait]
pub trait EzStorage<T>: Send + Sync + 'static
where
    T: EzApp,
{
    async fn load(&mut self) -> Result<Loaded, io::Error>;
    async fn persist(&mut self, op: Persist<T>) -> Result<(), io::Error>;
    async fn read_logs(&mut self, start: u64, end: u64) -> Result<Vec<EzEntry<T>>, io::Error>;
}
```

**Framework handles**: Raft logic, when to persist, what to persist

**You handle**: Serialization and I/O

Raft's safety rests on three assumptions the implementation must keep: `persist` returns
only once the data would survive a machine crash, operations become durable in the order
they arrive, and `read_logs` reflects everything `persist` has written, deletions included.

`FileStorage` implements this over JSON files in a directory, so a first cluster
runs without writing storage code. It does not `fsync`, so a machine crash can
lose a vote or a log entry the node already acknowledged - implement `EzStorage`
yourself for anything past a demo. Its source is the worked example of doing so.

## The EzRaft API

| Method | What it does |
|--------|--------------|
| `write(req)` | Runs `req` through the log, returns what `apply` produced. Forwards to the leader from a follower; waits out an election for about ten seconds before failing. |
| `read(\|app\| ...)` | Runs a closure over this node's applied state. No consensus round, no log entry. |
| `linearizable()` | Leader only: confirms leadership with a quorum round-trip and waits for local state to catch up, so a `read` after it sees every acknowledged write. |
| `metrics()`, `is_leader()`, `node_id()`, `addr()` | Current node and cluster state. |
| `change_membership(change)` | Membership changes beyond joining, such as removing a node. |
| `serve()` | Runs the HTTP server. Blocks, so spawn it - `tokio::spawn(raft.clone().serve())` - and start it early: peers reach a node only through this server. |
| `inner()` | The openraft `Raft` underneath, for anything EzRaft does not wrap. |

Every fallible method returns `std::io::Error`, including the ones that fail for reasons
that have nothing to do with I/O: a caller cannot usefully branch on the difference, since
`write` already finds the leader on its own. Code that does need typed errors reaches them
through `inner()`.

## Configuration

`EzConfig` provides sensible defaults for Raft timing parameters:

```rust
pub struct EzConfig {
    pub heartbeat_interval: Duration,  // Default: 500ms
    pub snapshot_interval: u64,        // Log entries between snapshots. Default: 500
}
```

Election timeout is automatically calculated as 3-6x the heartbeat interval,
and the log-purge distance is derived from `snapshot_interval`.

Most users can use `EzConfig::default()`.

## HTTP API

EzRaft includes built-in HTTP endpoints:

- **Raft RPC** (`/raft/*`): Internal consensus communication
- **Admin API** (`/api/join`, `/api/change_membership`, `/api/metrics`)
- **Application API** (`POST /api/write`): Propose client requests, using your request type as JSON
- **Application read** (`GET /api/read?key=...`): A keyed read answered by the app's `read` method from local memory, without a consensus round. An unknown key is a 404.

The two application endpoints are a sample, not the API a real service exposes. Copy `EzServer` and replace them with your own routes, built on `EzRaft::write` and `EzRaft::read`. Nothing here is authenticated or encrypted, so keep the addresses on a trusted network.

## Troubleshooting

**Two one-node clusters instead of one.** Every node after the first must join. Check
`GET /api/metrics`: `membership_config.membership.configs` must list every node.

**A node comes back as a stranger.** Its data directory holds its id - delete it and the
node asks for a new one, or, if it was the founding node, starts a fresh single-node
cluster of its own. Wipe data directories only when starting a whole cluster over.

**One directory per node.** Two nodes sharing one overwrite each other's log.

**Writes fail after ~10 seconds.** No leader: a cluster that has lost its majority cannot
elect one. Bring enough nodes back.

**Nothing in the logs.** Raft reports through `tracing`, which goes nowhere until a
subscriber is installed - the example installs one, then `RUST_LOG=info` (or `debug`) shows
what Raft is doing.

## Why EzRaft exists

**API design exploration.** EzRaft turns abstract ideas about "intuitive APIs" into concrete code. By testing different patterns—parameter organization, naming conventions, simplicity vs extensibility trade-offs—we discover what truly matches user intuition. These insights will guide future OpenRaft improvements.

**Fast prototyping.** As a secondary benefit, EzRaft lets beginners build working prototypes without understanding Raft internals or OpenRaft's architecture.

| Aspect | OpenRaft | EzRaft |
|--------|----------|--------|
| Required traits | 7+ (RaftLogStorage, RaftStateMachine, etc.) | 2 (EzStorage, EzApp) |
| Required methods | 21+ | 4 |
| User-defined types | 12 (all generic parameters) | 2 (Request, Response) |
| Network code | User implements (~100 lines) | Built-in (0 lines) |
| Example complexity | ~400 lines | ~145 lines, plus 40 of startup help |

Build on OpenRaft directly when the state outgrows memory, when the transport has to be
something other than plain HTTP, or when the types EzRaft fixes - `u64` node ids, an
address per node, standard one-leader-per-term voting - are not the ones you need.

## License

MIT OR Apache-2.0
