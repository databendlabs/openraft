### What are the differences between Openraft and standard Raft?

These are protocol-level departures from the Raft paper. API conveniences
(extensible type config, split storage, manual triggers, lifecycle callbacks,
etc.) are covered elsewhere.


#### Election and `Vote`

- **`Vote` unifies `term` and `voted_for`.** Standard Raft tracks `currentTerm`
  and `votedFor` separately. Openraft combines them into a single
  partially-ordered [`Vote`][] value that drives every election and replication
  decision: a node grants a request iff the incoming `Vote` is greater than or
  equal to the last `Vote` it has seen. See [`Vote`][`Vote doc`].

- **Advanced-mode leader-id (default): multiple leaders per term.** Standard
  Raft elects at most one leader per term. Openraft's default leader-id is
  `(term, node_id)` and totally ordered, so multiple candidates may be granted
  in the same term — the last one wins. This minimizes election conflicts at
  the cost of embedding `node_id` in every [`LogId`][]. A standard-mode
  leader-id is available if the extra bytes matter. See [`leader_id`][].


#### Leader is not tied to voter role

Standard Raft requires the leader to be a voter in the current configuration.
In Openraft, leadership is determined solely by a committed [`Vote`][] —
Paxos-style — so a leader may be a learner or even a node outside the
membership. Voter/learner status only affects quorum counting, never the right
to lead.

This generalization rolls out in phases:

- **Phase A — supported.** A committed membership change demotes the leader to
  a learner or removes it from the membership entirely. The leader keeps
  replicating and committing until it steps down, and does not count itself in
  any quorum while non-voting. It also replicates the `membership-is-committed`
  message before stepping down, so the new configuration can operate without
  re-committing the membership log. See `RaftCore::current_leader` in
  `openraft/src/core/raft_core.rs`.

- **Phase B — not yet implemented.** A node that was never in the membership
  becomes a leader. This completes the generalization and is tracked on the
  roadmap.


#### Membership change

- **Extended joint consensus.** Standard Raft alternates uniform and two-config
  joint memberships (`c1 → c1c2 → c2`). Openraft's extended algorithm accepts
  arbitrary joint configs and reversion (`c1 → c1c2c3 → c3c4 → c4`, or
  `c1c2c3 → c1`), as long as the **old-new-intersect** constraint holds between
  adjacent memberships. See [`extended membership`][].

- **No single-step change.** Single-step membership change is a restricted
  subset of joint consensus; Openraft only exposes the joint variant, which can
  express any single-step transition and more.

- **`change_membership` is concurrency-safe.** Parallel calls may interleave
  and leave the cluster in a valid joint state. This is by design — it
  simplifies leader-crash recovery and lets the leader pivot if a target node
  becomes unreliable mid-transition.

  A membership log entry also takes effect the moment it appears in the log,
  not when it commits (the **use-at-once** constraint in
  [`extended membership`][]). This is what lets a leader keep serving after it
  has been demoted or removed by an uncommitted membership change.


#### Log and state

- **Committed log id is persisted.** Openraft lets the log storage persist the
  committed log id via [`RaftLogStorage::save_committed()`][]. On startup,
  previously-committed entries can be re-applied to the state machine without
  a fresh commit round. Standard Raft does not persist commit progress.

- **Restarted leader stays leader when possible.** After a restart, a node
  whose saved `Vote` still names itself as leader resumes the Leader role
  immediately, subject to lease checks. Standard Raft always restarts as a
  follower.


#### Read

- **`ReadIndex` without the blank-log round-trip.** Standard Raft requires the
  leader to commit a blank log entry at the start of its term before serving
  linearizable reads. Openraft uses the term's initial blank log id as
  `read_log_id` directly, skipping the extra round-trip. See
  [`Linearizable Read`][].

- **Lease-based linearizable read.** In addition to `ReadIndex`, Openraft
  offers [`ReadPolicy::LeaseRead`][], which confirms leadership via lease
  instead of a quorum heartbeat. See [`leader lease`][].

- **Linearizable follower read.** A follower may obtain `read_log_id` from the
  leader via application RPC, wait for its local state machine to catch up,
  then read locally — distributing read load without losing linearizability.
  See [`Linearizable Read`][].


[`Vote`]: `crate::vote::Vote`
[`Vote doc`]: `crate::docs::data::vote`
[`LogId`]: `crate::LogId`
[`leader_id`]: `crate::docs::data::leader_id`
[`extended membership`]: `crate::docs::data::extended_membership`
[`leader lease`]: `crate::docs::protocol::replication::leader_lease`
[`Linearizable Read`]: `crate::docs::protocol::read`
[`RaftLogStorage::save_committed()`]: `crate::storage::RaftLogStorage::save_committed`
[`ReadPolicy::LeaseRead`]: `crate::raft::ReadPolicy::LeaseRead`
