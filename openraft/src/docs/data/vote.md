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
Refer to the `PartialOrd` implementation for [`Vote`]. The pseudocode about vote order checking is as follows:

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
- the default mode: every term may have more than one leader
  (enabled by default, or explicitly by setting [`RaftTypeConfig::LeaderId`] to [`leader_id_adv::LeaderId`]).
- and the standard Raft mode: every term has only one leader
  (enabled by setting [`RaftTypeConfig::LeaderId`] to [`leader_id_std::LeaderId`]),

The only difference between these two modes is the definition of `LeaderId`, and the `PartialOrd` implementation of it.
See: [`leader-id`].


## Vote and Membership define the server state

In the default mode, the `Vote` defines the server state (leader, candidate, follower or learner).
A server state has a unique corresponding `vote`, thus `vote` can be used to identify different server
states, i.e., if the `vote` changes, the server state must have changed.

Election eligibility uses both membership views: a node may campaign if it is a voter in
either its effective or its committed membership. This lets a removed voter recover leadership
while the final configuration that removes it is not yet locally known to be committed.
Ordinary learners that are voters in neither view cannot campaign. Election and replication
quorums still use only the effective membership; a candidate outside that voter set does not
count its own vote.

Note: a follower will switch to a learner and vice versa without changing the `vote`, when a
new membership log is replicated to a follower or learner.

E.g.:

- Node-2 with vote `(term=1, node_id=2, committed=true)`:
  - is a leader if it is **present** in the effective config, either a voter or non-voter,
    or is a voter in the committed config.
  - is a learner otherwise.

- Node-2 with vote `(term=1, node_id=2, committed=false)`:
  - is a candidate if it is **present** in the effective config, either a voter or non-voter,
    or is a voter in the committed config.
  - is a learner otherwise.

- Node-3 with vote `(term=1, node_id=99, committed=false|true)`:
  - is a follower if it is a **voter** in config,
  - is a learner if it is a **non-voter** or **absent** in config.

For node-2:

| vote \ membership                   | Effective voter | Effective learner | Absent, committed voter | Absent, not committed voter |
|-------------------------------------|-----------------|-------------------|-------------------------|-----------------------------|
| (term=1, node_id=2, committed=true)   | leader          | leader            | leader                  | learner                     |
| (term=1, node_id=2, committed=false)  | candidate       | candidate         | candidate               | learner                     |
| (term=1, node_id=99, committed=true)  | follower        | learner           | learner                 | learner                     |
| (term=1, node_id=99, committed=false) | follower        | learner           | learner                 | learner                     |

These predicates determine the role when it is recalculated. An existing Leader that is fully
removed by a committed configuration can keep its runtime Leader state until the configured
step-down policy triggers a refresh or a Vote change ends its leadership.



[`Vote`]: `crate::vote::Vote`
[`RaftTypeConfig::LeaderId`]: `crate::RaftTypeConfig::LeaderId`
[`leader_id_adv::LeaderId`]: `crate::impls::leader_id_adv::LeaderId`
[`leader_id_std::LeaderId`]: `crate::impls::leader_id_std::LeaderId`
[`leader-id`]: `crate::docs::data::leader_id`
