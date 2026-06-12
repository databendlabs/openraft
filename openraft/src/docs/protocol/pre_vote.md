# Pre-Vote

Pre-Vote is an optional round that a follower runs **before** it increments its
term and starts a real election. It probes whether a quorum *would* grant it a
vote, without changing any state. Only if a quorum would grant does the node run
the real election.

It is disabled by default and enabled with
[`Config::enable_pre_vote`](`crate::Config::enable_pre_vote`).


## The problem it solves

A node that cannot currently win an election — one that was partitioned,
restarted, or whose log fell behind — still increments its term every time its
election timer fires. When it later reconnects, that inflated term forces the
healthy leader to step down (the leader observes the higher term while
replicating to the returned node), triggering an unnecessary election even
though the cluster was healthy.

The [leader lease](`crate::docs::protocol::replication::leader_lease`) already
prevents such a node from *winning* votes: peers reject its vote requests while
their lease from the current leader is still valid. Pre-Vote addresses the
complementary problem — it stops the term from ever climbing, so there is no
higher term to disrupt the leader with on reconnect.


## How it works

When the election timer fires and Pre-Vote is enabled, a multi-voter node sends
a Pre-Vote request to its peers asking: *"would you grant me a vote at
`term + 1`?"* The request carries a hypothetical next-term vote and the
candidate's last log id.

A peer answers with the same rules it uses for a real vote request:

- If it has a committed vote whose [leader lease](`crate::docs::protocol::replication::leader_lease`)
  has not expired, it would not grant — there is a live leader.
- If the candidate's last log id is behind its own, it would not grant.
- Otherwise it would grant.

The crucial difference from a real vote: handling a Pre-Vote request **persists
nothing and changes nothing** — no term bump, no saved vote, no lease update. It
is a pure query. Likewise, the requester does not change its own state while
pre-voting: it stays a Follower with its term unchanged until a quorum responds.

If a quorum would grant, the node runs the real election
([`elect`](`crate::Raft`)), incrementing its term and persisting its vote as
usual. If not, nothing changed, and it retries on a later tick.

A single-voter cluster always wins its own Pre-Vote, so it skips the round and
elects directly.


## Backward compatibility

Pre-Vote uses a dedicated RPC,
[`RaftNetworkV2::pre_vote`](`crate::network::v2::RaftNetworkV2::pre_vote`), which
has a default implementation that returns
[`Unreachable`](`crate::error::Unreachable`). A node treats an `Unreachable`
Pre-Vote response as a grant, so a cluster whose peers have not implemented
`pre_vote` — for example during a rolling upgrade — still makes progress by
falling back to the normal election path.
