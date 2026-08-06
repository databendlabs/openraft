### How to restore a cluster from a snapshot backup?

When every node of a cluster is lost and only a snapshot backup remains, an
operator can rebuild the cluster by replaying the snapshot to pristine nodes
(nodes with no logs and no vote). No dedicated API is needed: the protocol API
[`Raft::install_full_snapshot()`][] already covers this case.

The `vote` argument is derived from the snapshot itself:
`snapshot.meta.last_log_id` records the [`CommittedLeaderId`][] of the leader
that proposed the last applied entry. Such a leader must have won an election
in the original cluster, because only an established leader can propose log
entries. That leader also applied this entry, so while it was alive it could
have sent exactly this `(vote, snapshot)` pair via snapshot replication.
Replaying the pair to a pristine node is therefore indistinguishable from
receiving a long-delayed legitimate replication message, and Raft handles
delayed messages by design.

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

Notes:

- The snippet uses the default leader-id mode, in which the log id keeps the
  proposing node id. With [`leader_id_std`][], the log id keeps only the term;
  build the vote from that term and the local node id instead. The two forms
  are protocol-equivalent, because in that mode a committed leader identity is
  the term alone.
- The vote must not be below the leader of `snapshot.meta.last_log_id`;
  otherwise [`Raft::install_full_snapshot()`][] panics, per its documented
  contract. Deriving the vote from the snapshot itself always satisfies the
  contract.
- To restore a multi-node cluster, replay the same snapshot on every node,
  then trigger an election on one of them.
- A node that is not in the snapshot's membership can be restored the same
  way; it just remains a learner.
- This recipe targets rebuilding a cluster whose nodes are all lost. To
  restore a single damaged follower or learner while the cluster is still
  running, wipe the node and let the leader re-replicate to it, with
  [`Config::allow_log_reversion`][] enabled so that the leader accepts the
  node's log reversion (see the FAQ entry about wiping the data of one node).

[`Raft::install_full_snapshot()`]: `crate::Raft::install_full_snapshot`
[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
[`CommittedLeaderId`]: `crate::vote::RaftCommittedLeaderId`
[`leader_id_std`]: `crate::impls::leader_id_std`
