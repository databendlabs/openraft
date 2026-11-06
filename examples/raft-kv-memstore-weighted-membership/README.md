# Key-value store with a weighted membership

This is [`raft-kv-memstore`](../raft-kv-memstore/) with one component swapped:
the type config selects `WeightedMembership` and `WeightedChange` from
[`openraft-membership`](../../membership/), in place of the default majority
`Membership` and `ChangeMembers`. The storage, the network and the other HTTP
routes are the same; `raft-kv-memstore` explains how they fit together.

## The type config

[`src/lib.rs`](./src/lib.rs) names both membership types:

```rust
openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
        Membership = WeightedMembership<NodeId, Node>,
        MembershipEdit = WeightedChange<NodeId, Node>,
);
```

A weighted membership gives each voter a weight. Voters that weigh
`write_weight` together commit an entry, and voters that weigh
`total_weight - write_weight + 1` elect a leader. The
[weighted membership guide](../../membership/src/weighted-membership.md)
explains the rule and how to choose the weights.

## The membership routes take weights

`app_http::Server::add_openraft_routes()` needs `MembershipEdit:
From<ChangeMembers>`, because its `/change-membership` takes only a set of voter
IDs. A voter set states no weights, so `WeightedChange` has no such conversion.
[`src/lib.rs`](./src/lib.rs) registers the routes one by one instead: `/init`
and `/change-membership` from
[`src/membership_api.rs`](./src/membership_api.rs), and the other OpenRaft
routes of `app-http` unchanged.

| Route                | Body                                                              | Calls                                                         |
|----------------------|-------------------------------------------------------------------|---------------------------------------------------------------|
| `/init`              | `{"weights": {"1": 1}, "write_weight": 1}`                        | `WeightedMembership::new()`, then `Raft::initialize()`        |
| `/change-membership` | `{"weights": {"1": 2, "2": 1, "3": 1, "4": 1}, "write_weight": 3}` | `WeightedChange::weights()`, then `Raft::change_membership()` |

`/init` adds the address of the node that serves it; its optional `nodes` field
gives the addresses of the other voters of the first membership.
`/change-membership` keeps the current write weight when the body has no
`write_weight`. A new voter must already be a learner, added with
`/add-learner`.

## Run it

[`test-cluster.sh`](./test-cluster.sh) drives four node processes with `curl`,
and the [integration test](./tests/cluster/test_weighted_membership.rs) does the
same in Rust:

```shell
./test-cluster.sh
cargo test
```

Both initialize node 1 alone with weight 1, add nodes 2, 3 and 4 as learners,
and give the voters the weights `{1:2, 2:1, 3:1, 4:1}` with write weight 3. The
weights change, so OpenRaft first appends a joint config, and then the new
config. Both then write a key, stop nodes 3 and 4, and write again: voters 1 and
2 weigh 3, which reaches the write weight, so the second write commits.
