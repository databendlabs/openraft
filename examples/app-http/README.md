# HTTP Application Service Example

This crate provides a small JSON-over-HTTP client and server for the example
applications.

It is separate from [`network-v2-http`](../network-v2-http/):

- `network-v2-http` handles node-to-node Raft RPCs.
- `app-http` handles client-to-application requests such as `/write`, `/read`,
  `/init`, and `/change-membership`.

## Components

Application HTTP components:

- `app_http::App<C, SM, D>` is the shared application context held by the
  server. It stores the node id, application address, Raft address, Raft handle,
  and application data. It also provides the common OpenRaft application
  handlers: `init`, `add_learner`, `change_membership`, `metrics`,
  `get_linearizer`, and `write`.
- `app_http::Server<D>` is a small JSON-over-HTTP server. `Server::new(app)`
  starts with no routes. Examples call `add_openraft_routes()` for the common
  endpoints from `app_http::App`, then add their own read endpoints because
  reading depends on the state machine.
- `app_http::Client<C>` is a test/demo client for the application API. It sends
  requests to the application server and updates its target when OpenRaft
  returns `ForwardToLeader` with a known leader.
- `app_http::AddLearnerRequest` is the request body for `/add-learner`. It
  carries the new node id, Raft address, and application address.
- `app_http::LinearizerData` carries the leader's read linearizer data for the
  follower-read example.
- `app_http::FollowerReadError` is the simple error returned by the follower-read
  helper.

Raft network HTTP components:

- `network_v2_http::NetworkFactory` is passed to `Raft::new()`. OpenRaft uses it
  to build outbound Raft RPC clients for peer nodes.
- `network_v2_http::Client` implements OpenRaft's `RaftNetworkV2` client side.
  It sends `append_entries`, `vote`, `full_snapshot`, and `transfer_leader`
  requests to peer `raft_addr` endpoints.
- `network_v2_http::Server<C, SM>` is the standalone Raft RPC server. It listens
  on `raft_addr`, decodes inbound Raft RPCs, and forwards them to the local
  `Raft` handle.

## Relationship

Each node runs two HTTP servers. Application requests enter through `api_addr`;
Raft replication uses `raft_addr` between nodes. Follower reads also use
`api_addr`: the follower asks the leader's application server for linearizer
data before reading from its local state machine.

```text
Node 1: leader                                             Node 2: follower

   +------------------+                                       +------------------+
   | app_http::Server |<---------------------.                | app_http::Server |
   +--------+---------+                      |                +--------+---------+
            |                                |                         |
            v                                |                         v
    +---------------+                        |                 +---------------+
    | app_http::App |                        '-----------------| app_http::App |
    +-------+-------+                          /get_linearizer +-----------+---+
            |                                                          |
            |     +------------------------+                           |     +------------------------+
            |     | network_v2_http::Server|     .-------------------------->| network_v2_http::Server|
            |     +-----------+------------+     |                     |     +-----------+------------+
            |                 |                  |                     |                 |
            |                 |                  |                     |                 |
            |  .--------------'                  |                     |  .--------------'
            |  |                                 |                     |  |
            v  v                                 |                     v  v
+-----------------------+                        |         +-----------------------+
| openraft::Raft handle |                        |         | openraft::Raft handle |
+-----------+-----------+                        |         +-----------------------+
            |                                    |
            v                                    |
+------------------------+                       |
| network_v2_http::Client| ----------------------'
+------------------------+  /append
                            /snapshot
                            /vote
                            /transfer-leader

```

```rust
let app_server = app_http::Server::new(app)
    .add_openraft_routes()
    .post("/read", api::read)
    .run(api_addr);
```

`add_openraft_routes()` installs `/init`, `/add-learner`,
`/change-membership`, `/metrics`, `/get_linearizer`, and `/write`.
`Server::new(app)` by itself is only the generic JSON route framework.

## Client

`Client` sends requests to an application server and updates its target when an
OpenRaft error returns a known leader:

```rust
let client = app_http::Client::new(1, "127.0.0.1:21001".to_string());

client.init().await??;
client.write(&request).await??;
let value = client.read(&"foo".to_string()).await?;
```

The client is example-specific: it knows the endpoints used by the key-value
examples and is not part of OpenRaft's Raft network layer.
