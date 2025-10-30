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

But for even number nodes cluster, Erasing **exactly one** node won't cause data loss.
Thus, in a special scenario like this, or for testing purpose, you can use
[`Config::allow_log_reversion`] to permit erasing a node.

[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
