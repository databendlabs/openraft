# Vote

```ignore
struct Vote<NID: NodeId> {
    leader_id: LeaderId<NID>,
    committed: bool,
}
```

In Openraft, the [`Vote`] defines the pseudo time which determines the `leader` in a distributed consensus.
Essentially, each `vote` represents a distinct time point, similar to the concept of `rnd` or `ballot-number` in Paxos.

In a standard Raft, the corresponding concept is `(term, voted_for: Option<NodeId>)`.

The validity checking for RPCs in Openraft, such as when handling vote or append-entries requests,
is straightforward. Essentially, **a node will grant a `Vote` only if it is greater than or equal to the last `Vote` it has seen**.
Refer to the `PartialOrd` implementation for [`Vote`]. The pseudo code about vote order checking is as follows:

```ignore
# pseudo code
fn handle_vote(vote: Vote) {
    if !(vote >= self.vote) {
        return Err(())
    }
    save_vote(vote);
    Ok(())
}
```

## Partial order

`Vote` in Openraft is partially ordered value,
i.e., it is legal that `!(vote_a => vote_b) && !(vote_a <= vote_b)`.
Because `Vote.leader_id` may be a partial order value:

Openraft provides two election modes.
- the default mode: every term may have more than one leader.
- and the standard Raft mode: every term has only one leader(enabled by [`single-term-leader`]),

The only difference between these two modes is the definition of `LeaderId`, and the `PartialOrd` implementation of it.
See: [`leader-id`].


## Vote and Membership define the server state

In the default mode, the `Vote` defines the server state(leader, candidate, follower or learner).
A server state has a unique corresponding `vote`, thus `vote` can be used to identify different server
states, i.e, if the `vote` changes, the server state must have changed.

Note: follower and learner in Openraft is almost the same. The only difference
is a learner does not try to elect itself.

Note: a follower will switch to a learner and vice versa without changing the `vote`, when a
new membership log is replicated to a follower or learner.

E.g.:

- Node-2 with vote `(term=1, node_id=2, committed=true)`:
  - is a leader if it is **present** in config, either a voter or non-voter.
  - is a learner if it is **absent** in config.

- Node-2 with vote `(term=1, node_id=2, committed=false)`:
  - is a candidate if it is **present** in config, either a voter or non-voter.
  - is a learner if it is **absent** in config.

- Node-3 with vote `(term=1, node_id=99, committed=false|true)`:
  - is a follower if it is a **voter** in config,
  - is a learner if it is a **non-voter** or **absent** in config.

For node-2:

| vote \ membership                     | Voter     | Non-voter | Absent  |
|---------------------------------------|-----------|-----------|---------|
| (term=1, node_id=2, committed=true)   | leader    | leader    | learner |
| (term=1, node_id=2, committed=false)  | candidate | candidate | learner |
| (term=1, node_id=99, committed=true)  | follower  | learner   | learner |
| (term=1, node_id=99, committed=false) | follower  | learner   | learner |



[`Vote`]: `crate::vote::Vote`
[`single-term-leader`]: `crate::docs::feature_flags`
[`leader-id`]: `crate::docs::data::leader_id`
