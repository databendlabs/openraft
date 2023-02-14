# Vote

```rust
struct Vote<NID: NodeId> {
    leader_id: LeaderId<NID>
    committed: bool
}

// Advanced mode(default):
#[cfg(not(feature = "single-term-leader"))]
pub struct LeaderId<NID: NodeId>
{
    pub term: u64,
    pub node_id: NID,
}

// Standard raft mode:
#[cfg(feature = "single-term-leader")]
pub struct LeaderId<NID: NodeId>
{
    pub term: u64,
    pub voted_for: Option<NID>,
}
```

`vote` in openraft defines the pseudo **time**(in other word, defines every `leader`) in a distributed consensus.
Each `vote` can be thought as unique time point(in paxos the pseudo time is round-number or `rnd`, or `ballot-number`).

In a standard raft, the corresponding concept is `term`.
Although in standard raft a single `term` is not enough to define a **time
point**.

In openraft, RPC validity checking(such as when handling vote request, or
append-entries request) is very simple: **A node grants a `Vote` which is greater than its last seen `Vote`**:

```rust
fn handle_vote(vote: Vote) {
    if !(vote >= self.vote) {
        return Err(())
    }
    save_vote(vote);
    Ok(())
}
```

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


## Partial order

`Vote` in openraft is partially ordered value,
i.e., it is legal that `!(vote_a => vote_b) && !(vote_a <= vote_b)`.
Because `Vote.leader_id` may be a partial order value:


## LeaderId: advanced mode and standard mode

Openraft provides two `LeaderId` type, which can be switched with feature
`single-term-leader`:

- `cargo build` without `single-term-leader`, is the advanced mode, the default mode:
  It builds openraft with `LeaderId:(term, node_id)`, which is totally ordered.
  Which means, in a single `term`, there could be more than one leaders
  elected(although only the last is valid and can commit logs).

  - Pros: election conflict is minimized,

  - Cons: `LogId` becomes larger: every log has to store an additional `NodeId` in `LogId`:
    `LogId: {{term, NodeId}, index}`.
    If an application uses a big `NodeId` type, e.g., UUID, the penalty may not
    be negligible.

- `cargo build --features "single-term-leader"` builds openraft in standard raft mode with:
  `LeaderId:(term, voted_for:Option<NodeId>)`, which makes `LeaderId` and `Vote`
  **partially-ordered** values. In this mode, only one leader can be elected in
  each `term`.

  The partial order relation of `LeaderId`:

  ```
  LeaderId(3, None)    >  LeaderId(2, None):    true
  LeaderId(3, None)    >  LeaderId(2, Some(y)): true
  LeaderId(3, None)    == LeaderId(3, None):    true
  LeaderId(3, Some(x)) >  LeaderId(2, Some(y)): true
  LeaderId(3, Some(x)) >  LeaderId(3, None):    true
  LeaderId(3, Some(x)) == LeaderId(3, Some(x)): true
  LeaderId(3, Some(x)) >  LeaderId(3, Some(y)): false
  ```

  The partial order between `Vote` is defined as:
  Given two `Vote` `a` and `b`:
  `a > b` iff:

  ```rust
  a.leader_id > b.leader_id || (
    !(a.leader_id < b.leader_id) && a.committed > b.committed
  )
  ```

  In other words, if `a.leader_id` and `b.leader_id` is not
  comparable(`!(a.leader_id>=b.leader_id) && !(a.leader_id<=b.leader_id)`), use
  field `committed` to determine the order between `a` and `b`.

  Because a leader must be granted by a quorum before committing any log, two
  incomparable `leader_id` can not both be granted.
  So let a committed `Vote` override a incomparable non-committed is safe.

  - Pros: `LogId` just store a `term`.

  - Cons: election conflicting rate may increase.

