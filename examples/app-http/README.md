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
  handlers: `init`, `add_learner`, `change_membership`, `metrics`, `readiness`,
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
- `app_http::FollowerReadError` says why the follower-read helper served no
  read: no known leader, no answer from it, a refusal, or a deadline that
  expired before the local state machine caught up.
- `app_http::Readiness` is what `/readiness` reports from a node's own metrics:
  the leader it follows, whether it is that leader, and whether it has applied
  that leader's no-op entry. None of these authorizes a read or a write.

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
data before reading from its local state machine. It asks again for every read,
follows the redirect of a node that no longer leads, and gives up when
`FOLLOWER_READ_TIMEOUT` expires.

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
`/change-membership`, `/metrics`, `/readiness`, `/get_linearizer`, and `/write`.
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

Each operation has one ten-second deadline covering redirects, connection-pool
waits, RPC execution, and response decoding. Redirecting operations also have a
retry bound. A missing leader hint returns the typed application error. Encoding
failures, invalid JSON, and HTTP failure statuses return a transport error.

Applications that forward requests between servers must carry a decreasing hop
budget and the remaining operation time. Every forwarded RPC and pool wait must
respect that remaining time; starting a new timeout at each hop does not bound
the original request.

A leader rejection says that node did not accept the request. A lost response
can follow a committed write. Retrying such a write requires application-level
request IDs and deduplication when repeated commands have different effects.

`Client::write()` resends a write only after `ForwardToLeader`, which says the
node did not append it. After `LogEntryDiscarded`, the node had appended the
write, and the write may still commit through another leader, so `write()`
returns the error without resending; the next request goes to the leader the
error names.

`Client::write_with_retries()` also resends after `LogEntryDiscarded` and after a
response that fails or does not arrive within two seconds, until the ten-second
budget runs out. Use it only for writes that the state machine applies at most
once per request ID, as
[`raft-kv-memstore-dedup`](../raft-kv-memstore-dedup/) does.
