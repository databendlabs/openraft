# Elections and Leadership Handover

This page covers what an operator controls about elections and leadership, and
how to tell whether an operation took effect.

## A voter that does not campaign

With [`Config::enable_elect`] set to `false`, or after
`raft.runtime_config().elect(false)`, a node starts no election when its
election timer expires. It is still a voter, not a learner: it grants votes,
counts toward every quorum, and takes over when a leader transfers leadership
to it.

This switch does not stop one startup campaign. A node that was the leader
before it restarted campaigns for a higher term as soon as it starts, because
[`Config::enable_leader_restore`] is enabled by default. To keep a voter
passive across restarts, also set `enable_leader_restore` to `Some(false)`;
the node then starts as a follower.

A deployment can use this for a voter in a remote region. The remote voter
holds a copy of the log and counts toward commits, but leads only when an
operator moves leadership to it. Automatic failover then needs a voter that
may campaign and can still reach a quorum: in a cluster of two local voters
and one remote voter, losing one local voter leaves the other one to take over
with the remote voter's vote. Keep enough voters that campaign to survive the
failures the deployment plans for.

[`RuntimeConfigHandle::is_elect_enabled()`] reports the setting that Raft
reads. A runtime change lives in memory only, and a restart applies
[`Config::enable_elect`] again.

The manual triggers work whatever `enable_elect` says:

- [`Trigger::elect(true)`][`Trigger::elect()`] runs a Pre-Vote round first and
  starts the real election only if a quorum would grant the vote. While this
  node's own leader lease is valid, it sends nothing. Either way a refused
  round leaves the term unchanged.
- `Trigger::elect(false)` starts a real election at once. The term increases
  even if this node cannot win, which can make a healthy leader step down.
- [`Trigger::transfer_leader()`] makes the target campaign, also when the
  target has `enable_elect` off; see below.

## Handing over leadership

[`Trigger::transfer_leader()`] returns once the leader has queued the request.
The target can campaign even with `enable_elect` disabled. It must recognize
the source leader's vote and have flushed its log through the entry named in
the request.

The source gives the transfer one sampled election timeout. If the source
still holds leadership after the deadline, its election timer can start
recovery when the usual election timing allows it. The source must have
[`Config::enable_elect`] enabled and have the campaigner role in its effective
or committed membership. The election timer works even with
[`Config::enable_tick`] disabled.

With [`Config::enable_pre_vote`] enabled, the source first needs a Pre-Vote
quorum. A source that is the sole voter in its effective membership and can
campaign in that membership skips Pre-Vote. When the source starts a real
election, it steps down and campaigns in a new term. It can win again.

The source can still report `Leader` past the deadline while it waits for a
Pre-Vote quorum or cannot campaign. While the transfer remains pending, new
writes and linearizable reads return [`ForwardToLeader`] with reason
`LeadershipTransfer`. The error names the target if it is in the source's
effective membership.

To observe a successor, wait for the metrics on the source `my_id` to report
a different leader.

```ignore
raft.trigger().transfer_leader(to).await?;

// Choose how long to wait for metrics to show a different leader.
let timeout = Duration::from_millis(raft.config().election_timeout_max * 2);
let moved = raft
    .wait(Some(timeout))
    .metrics(
        |m| m.current_leader.is_some() && m.current_leader != Some(my_id),
        "leadership moved",
    )
    .await;

match moved {
    Ok(m) if m.current_leader == Some(to) => { /* the target leads */ }
    Ok(m) => { /* another node, m.current_leader, leads */ }
    Err(WaitError::Timeout(..)) => { /* no different leader was observed within the wait */ }
    Err(e) => return Err(e.into()),
}
```

Which voter to hand over to is the application's choice. The voter with the
greatest matched log in [`RaftMetrics::replication`] catches up fastest; it is
the voter that a removed leader picks.

## Membership changes that involve the leader

A leader may remove itself from the voters. It keeps leading until the change
commits, so [`Raft::change_membership()`] returns as usual. After
[`Config::removed_leader_step_down`], 150 ms by default, it transfers
leadership to the most up-to-date voter and steps down. The commit and the
handover are two events: the response reports the first, and the old leader's
[`RaftMetrics::state`] leaving `Leader` reports the second. To choose the next
leader yourself, transfer leadership first and then run the change on the new
leader.

Two errors of a membership change say "try again later" rather than "this
change is wrong":

- `ChangeMembershipError::InProgress`: an earlier membership change has not
  committed yet.
- `ChangeMembershipError::UncommittedLeaderLog`: the leader was just elected
  and has not committed an entry of its own term yet.

Retry either one on the same leader after a short delay, and give up after a
bounded time. A `ForwardToLeader` error means the request reached a node that
does not lead; send it to the leader it names.

An application that runs several Raft groups changes each group's membership
on its own: one group's change can commit while another's fails. Make each
step safe to repeat, and check every group's membership before reporting the
operation complete.

[`Config::enable_elect`]: `crate::Config::enable_elect`
[`Config::enable_leader_restore`]: `field@crate::Config::enable_leader_restore`
[`Config::removed_leader_step_down`]: `crate::Config::removed_leader_step_down`
[`RuntimeConfigHandle::is_elect_enabled()`]: `crate::raft::RuntimeConfigHandle::is_elect_enabled`
[`Trigger::elect()`]: `crate::raft::trigger::Trigger::elect`
[`Trigger::transfer_leader()`]: `crate::raft::trigger::Trigger::transfer_leader`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`RaftMetrics::replication`]: `crate::RaftMetrics::replication`
[`RaftMetrics::state`]: `crate::RaftMetrics::state`
[`Config::enable_pre_vote`]: `crate::Config::enable_pre_vote`
[`Config::enable_tick`]: `crate::Config::enable_tick`
[`ForwardToLeader`]: `crate::errors::ForwardToLeader`
