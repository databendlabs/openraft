### How to restore a cluster from a snapshot backup?

This recipe applies only when the cluster has **no running leader**, e.g.,
every node is lost and only a snapshot backup remains. If a leader is still
running, do not use it: the best way to bring a pristine node (a node with no
logs and no vote) into a running cluster is ordinary log replication — start
the node empty and let the leader replicate logs and snapshot to it, with
[`Config::allow_log_reversion`][] enabled so that the leader accepts the
node's log reversion in case the node previously held data.

Without a running leader, no dedicated API is needed either: the protocol API
[`Raft::install_full_snapshot()`][] already covers rebuilding from a backup.

The `vote` argument is derived from the snapshot itself:
`snapshot.meta.last_log_id` records the [`CommittedLeaderId`][] of the leader
that proposed the last applied entry. Such a leader must have won an election
in the original cluster, because only an established leader can propose log
entries. That leader also applied this entry, so while it was alive it could
have sent exactly this `(vote, snapshot)` pair via snapshot replication.
Replaying the pair to a pristine node is therefore indistinguishable from
receiving a long-delayed legitimate replication message, and Raft handles
delayed messages by design.

With the default [`leader_id_adv`][] mode, the committed leader id in a log
id contains both the term and the node id, so the exact historical vote is
reconstructed without trouble:

```rust,ignore
let snapshot: Snapshot<TypeConfig> = read_backup();
let last = snapshot.meta.last_log_id.clone().unwrap();

// The exact committed vote of the leader that proposed `last`.
let leader = last.committed_leader_id();
let vote = Vote::new_committed(leader.term(), leader.node_id().clone());

raft.install_full_snapshot(vote, snapshot).await?;

// Become leader in the next term and continue as a new cluster lineage.
raft.trigger().elect(false).await?;
```

With [`leader_id_std`][] mode, the committed leader id keeps only the term,
so the node-id part has to be supplied; use the local node id (or any chosen
node id):

```rust,ignore
let term = last.committed_leader_id().term;
let vote = Vote::new_committed(term, my_node_id);
```

Be aware that this std-mode vote is fabricated: it is legal but may be absent
from history — the term is real, while its holder may not be the node that
actually won that term. In std mode a committed leader identity is the term
alone, so the protocol treats the fabricated vote and the historical one
identically, and installing a snapshot on a pristine node tolerates it. This
tolerance is necessary for repairing a node this way.

Notes:

- The vote must not be below the leader of `snapshot.meta.last_log_id`;
  otherwise [`Raft::install_full_snapshot()`][] panics, per its documented
  contract. Deriving the vote from the snapshot itself always satisfies the
  contract.
- To restore a multi-node cluster, replay the same snapshot on every node,
  then trigger an election on one of them.
- A node that is not in the snapshot's membership can be restored the same
  way; it just remains a learner.

[`Raft::install_full_snapshot()`]: `crate::Raft::install_full_snapshot`
[`CommittedLeaderId`]: `crate::vote::RaftCommittedLeaderId`
[`leader_id_adv`]: `crate::impls::leader_id_adv`
[`leader_id_std`]: `crate::impls::leader_id_std`
[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
