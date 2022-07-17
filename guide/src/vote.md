# Vote

```rust
struct Vote<NID: NodeId> {
    term: u64,
    node_id: NID,
    committed: bool
}
```

`vote` in openraft defines the pseudo **time**(in other word, defines every `leader`) in a distributed consensus.
Each `vote` can be thought as unique time point(in paxos the pseudo time is round-number or `rnd`, or `ballot-number`).

In a standard raft, the corresponding concept is `term`.
Although in standard raft a single `term` is not enough to define a **time
point**.

Every server state(leader, candidate, follower or learner) has a unique
corresponding `vote`, thus `vote` can be used to identify different server
states, i.e, if the `vote` changes, the server state must have changed.

Note: follower and learner in openraft is almost the same. The only difference
is a learner does not try to elect itself.

Note: a follower will switch to a learner and vice versa without changing the `vote`, when a
new membership log is replicated to a follower or learner.

E.g.:

- A vote `(term=1, node_id=2, committed=false)` is in a candidate state for
    node-2.

- A vote `(term=1, node_id=2, committed=true)` is in a leader state for
    node-2.

- A vote `(term=1, node_id=2, committed=false|true)` is in a follower/learner
    state for node-3.

- A vote `(term=1, node_id=1, committed=false|true)` is in another different
    follower/learner state for node-3.
