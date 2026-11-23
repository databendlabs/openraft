# Key-value store with deduplicated writes

This is [`raft-kv-memstore`](../raft-kv-memstore/) with writes that carry
request IDs, so that a client can resend a write whose outcome it does not know.
The storage, the network and the HTTP routes are the same; `raft-kv-memstore`
explains how they fit together.

## Why a write needs an ID

Two outcomes leave a client unsure whether its write committed:

- The response is lost: the leader may have applied the write before the
  connection failed or the client stopped waiting.
- The node answers `LogEntryDiscarded`: it had appended the write, then dropped
  the entry when it followed another leader or installed a snapshot. The write
  may have committed already, or may still commit through another leader.

Resending a plain `types_kv::Request` after either outcome may apply it twice.
Here each write carries a `types_kv::RequestId`, and the state machine applies
each ID at most once.

## The type config

[`src/lib.rs`](./src/lib.rs) makes every write a `types_kv::DedupRequest`:

```rust
openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::DedupRequest,
        R = types_kv::Response,
        Node = Node,
);
```

`/write` takes one as JSON:

```json
{"id": {"client_id": 7, "serial": 1}, "request": {"Set": {"key": "foo", "value": "bar"}}}
```

## The state machine

The state machine is [`sm-mem`](../sm-mem/). For each client ID it keeps the
serial and the response of the latest applied write, in
`StateMachineData::sessions`:

- A write with a higher serial applies, and becomes the latest of its client.
- A write with the same serial is a resent copy: it does not apply again, and
  gets the stored response of the first application.
- A write with a lower serial does not apply, and its response holds no value.

The sessions are replicated state: every node applies the same writes to them,
and snapshots carry them. A resent write is therefore recognized on a new leader
and on a node that installed a snapshot.

## The client

A client follows the rule stated by
[`RequestId`](../types-kv/src/request_id.rs):

- It numbers its writes with increasing serials and sends them one at a time.
- It resends its latest write with the same ID until it learns the outcome, and
  never resends an earlier write.
- It takes a new client ID when it restarts, such as a random number.

`app_http::Client::write_with_retries()` does the resending. After a response
that fails or does not arrive within two seconds, or after `LogEntryDiscarded`,
it resends the same request to the leader the last error named, for up to ten
seconds. `Client::write()` does not resend after `LogEntryDiscarded`.

This example keeps the session of every client forever. A production system also
removes the sessions of clients that left, by a rule that every node applies
alike, such as an expiry driven by timestamps in the log; see section 6.3 of the
[Raft dissertation](https://github.com/ongardie/dissertation).

## Run it

```shell
cargo test
```

[`src/retry_test.rs`](./src/retry_test.rs) runs a cluster of three nodes:

- A write whose response was lost is resent. The second entry commits and gets
  the first outcome, and the stored version stays that of the first entry.
- A write that reached only the old leader is replaced by the no-op of the new
  leader, so the client gets `LogEntryDiscarded`. Resent to the new leader, the
  write applies once, and resending it again gets the same outcome.

To run nodes by hand, build with `cargo build` and start each one with the
binary, such as
`./target/debug/raft-key-value-dedup --id 1 --api-addr 127.0.0.1:21001 --raft-addr 127.0.0.1:22001`.
The routes that set up the cluster are those of `raft-kv-memstore`.
