### Why can a Leader reject a client write?

A node may still report [`ServerState::Leader`][] while returning
[`ForwardToLeader`][] with no target for a new write. This means its last quorum
acknowledgement is older than the leader lease, so it cannot currently confirm
that it still has authority to accept new proposals.

The node does not step down. It continues heartbeat and replication traffic so
that it can renew its lease if quorum communication recovers. Once a quorum
acknowledges it again, new writes are accepted automatically.

Writes accepted before the lease expired remain pending and may still commit.
If an application times them out, it must treat their result as unknown.

See [CheckQuorum][] for the protocol and its difference from conventional
self-demotion.

[`ServerState::Leader`]: crate::ServerState::Leader
[`ForwardToLeader`]: crate::errors::ForwardToLeader
[CheckQuorum]: crate::docs::protocol::check_quorum
