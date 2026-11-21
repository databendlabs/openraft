### Can I wipe out the data of ONE node and wait for the leader to replicate all data to it again?

Avoid doing this. Doing so will panic the leader. But it is permitted
with config [`Config::allow_log_reversion`] enabled.

In a raft cluster, although logs are replicated to multiple nodes,
wiping out a node and restarting it is still possible to cause data loss.
Assumes the leader is `N1`, followers are `N2, N3, N4, N5`:
- A log(`a`) that is replicated by `N1` to `N2, N3` is considered committed.
- At this point, if `N3` is replaced with an empty node, and at once the leader
  `N1` is crashed. Then `N5` may elected as a new leader with vote granted by
  `N3, N4`;
- Then the new leader `N5` will not have log `a`.

```text
Ni: Node i
Lj: Leader   at term j
Fj: Follower at term j

N1 | L1  a  crashed
N2 | F1  a
N3 | F1  a  erased          F2
N4 |                        F2
N5 |                 elect  L2
----------------------------+---------------> time
                            Data loss: N5 does not have log `a`
```

Erasing one node is safe only in a narrow case: the membership is stable, it
uses a uniform simple-majority quorum over an even number of **voters**,
exactly one voter is erased, and no earlier erasure is still unrepaired. With
`2k` voters, a committed entry is on at least `k + 1` of them, so `k` still
hold it after one is erased, and every election quorum of `k + 1` voters
includes one of them, which refuses to vote for a candidate without the entry.

Counting learners, which do not vote, does not give this guarantee, and neither
does a custom quorum rule: three voters and one learner are four nodes, yet
erasing one voter can lose a committed entry. With voters `A, B, C`, an entry
committed on `A` and `B` is lost if `A` is erased and `B` is unavailable: `A`
and `C` can elect `C` without it.

[`Config::allow_log_reversion`] only lets the leader reset the erased node's
replication progress; it adds no safety check. Enable it for testing, or for
the narrow case above.

Removing the node from the membership before wiping it does not make the
sequence safe either, as long as the wiped node rejoins under its old ID. See:
[Can I reuse the ID of a removed node for a new node?](#can-i-reuse-the-id-of-a-removed-node-for-a-new-node)

[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
