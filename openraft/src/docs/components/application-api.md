# Exposing Raft to Clients

Openraft's metrics and error types are generic over the type config, which
lets the compiler check them inside a node. They make a poor wire format
for clients, admin tools and other services: their shape follows Openraft's
releases, and an application with several Raft groups has one type per
group. Translate them at the API boundary into small types the application
owns and versions.

## Project metrics and errors into application types

Read what a client needs from [`RaftMetrics`] and the errors, and put it in
an application type. One generic function serves every group whose node id
and term types match:

```
use openraft::RaftMetrics;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;

openraft::declare_raft_types!(pub KvConfig: D = String, R = String);
openraft::declare_raft_types!(pub CounterConfig: D = u64, R = u64);

/// The status one group reports to admin tools; version 1 of the application's API.
#[derive(Debug, PartialEq)]
pub struct GroupStatusV1 {
    pub leader: Option<u64>,
    pub term: u64,
    pub is_leader: bool,
    pub last_applied_index: Option<u64>,
}

pub fn group_status<C>(m: &RaftMetrics<C>) -> GroupStatusV1
where C: RaftTypeConfig<NodeId = u64, Term = u64> {
    GroupStatusV1 {
        leader: m.current_leader,
        term: m.current_term,
        is_leader: m.state == ServerState::Leader,
        last_applied_index: m.last_applied.as_ref().map(|log_id| log_id.index),
    }
}

/// What a client does after a rejected write.
#[derive(Debug, PartialEq)]
pub enum WriteRejectedV1 {
    /// Send the write to this node, or find the leader first if it is `None`.
    Redirect { leader: Option<u64> },
    /// Retry on the same node after a short delay.
    RetryLater,
    /// The write may or may not commit: retry only if it is idempotent or deduplicated.
    Unknown { leader: Option<u64> },
    /// The write was refused for a reason the client must handle, such as a failed precondition.
    Rejected(String),
    /// This node cannot serve the request.
    Unavailable,
}

pub fn write_rejected<C>(e: &RaftError<C, ClientWriteError<C>>) -> WriteRejectedV1
where C: RaftTypeConfig<NodeId = u64> {
    match e.api_error() {
        Some(ClientWriteError::ForwardToLeader(f)) => match f.reason {
            ForwardReason::LeaseExpired => WriteRejectedV1::RetryLater,
            ForwardReason::NotLeader | ForwardReason::LeadershipTransfer => {
                WriteRejectedV1::Redirect { leader: f.leader_id }
            }
        },
        Some(ClientWriteError::LogEntryDiscarded(f)) => WriteRejectedV1::Unknown { leader: f.leader_id },
        Some(other) => WriteRejectedV1::Rejected(other.to_string()),
        None => WriteRejectedV1::Unavailable,
    }
}

// The same functions serve both groups.
let kv = RaftMetrics::<KvConfig>::new_initial(1);
let counter = RaftMetrics::<CounterConfig>::new_initial(1);
assert_eq!(group_status(&kv), group_status(&counter));

let lease_expired = ForwardToLeader::<KvConfig>::empty().with_reason(ForwardReason::LeaseExpired);
let e = RaftError::APIError(ClientWriteError::ForwardToLeader(lease_expired));
assert_eq!(WriteRejectedV1::RetryLater, write_rejected(&e));
```

A real application maps the other `ClientWriteError` variants, such as a
failed [`Precondition`] or a rejected membership change, by what they mean for
its clients instead of folding them into one case as above.

## Retrying a rejected write

A write rejected with [`ForwardToLeader`] carries a [`ForwardReason`] that
decides the retry:

| Error | What happened | What the client does |
|---|---|---|
| `ForwardToLeader`, `NotLeader` | The node does not lead. `leader_id` names the leader it knows, if any. | Send the write to `leader_id`, or find the leader first when it is `None`. |
| `ForwardToLeader`, `LeadershipTransfer` | The node is handing leadership to `leader_id`, which may not lead yet. | Send the write to `leader_id`, and expect another `NotLeader` until the transfer completes. |
| `ForwardToLeader`, `LeaseExpired` | The node leads but no quorum has acknowledged it recently. `leader_id` is `None`. | Retry on the same node after a short delay; it is not a redirect. |
| `LogEntryDiscarded` | The node accepted the write, then dropped its entry when it followed another leader. The entry may still commit through that leader. | Retry only a write that the application can deduplicate or that is idempotent. |

`ChangeMembershipError::InProgress` and `UncommittedLeaderLog` are also
temporary; see
[membership changes that involve the leader](crate::docs::cluster_control::leadership#membership-changes-that-involve-the-leader).

## Linearizable reads

A port from 0.9 keeps the 0.9 behavior with [`ReadPolicy::ReadIndex`]: the
leader confirms its leadership with a quorum before the read.
[`ReadPolicy::LeaseRead`] skips that round trip and relies on bounded clock
drift between nodes; choose it on purpose. Either way the read must wait for
the state machine to apply the read log id before it reads:
[`Raft::ensure_linearizable()`] does both on the leader, and a follower read
waits with [`Linearizer::await_ready()`]. See [read operations][`read`].

[`RaftMetrics`]: `crate::RaftMetrics`
[`ForwardToLeader`]: `crate::errors::ForwardToLeader`
[`ForwardReason`]: `crate::errors::ForwardReason`
[`Precondition`]: `crate::raft::Precondition`
[`ReadPolicy::ReadIndex`]: `crate::raft::ReadPolicy::ReadIndex`
[`ReadPolicy::LeaseRead`]: `crate::raft::ReadPolicy::LeaseRead`
[`Raft::ensure_linearizable()`]: `crate::Raft::ensure_linearizable`
[`Linearizer::await_ready()`]: `crate::raft::linearizable_read::Linearizer::await_ready`
[`read`]: `crate::docs::protocol::read`
