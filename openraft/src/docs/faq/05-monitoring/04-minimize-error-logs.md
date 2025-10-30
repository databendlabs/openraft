### How to minimize error logging when a follower is offline

Excessive error logging, like `ERROR openraft::replication: 248: RPCError err=NetworkError: ...`, occurs when a follower node becomes unresponsive. To alleviate this, implement a mechanism within [`RaftNetwork`][] that returns a [`Unreachable`][] error instead of a [`NetworkError`][] when immediate replication retries to the affected node are not advised.

[`RaftNetwork`]: `crate::network::RaftNetwork`
[`Unreachable`]: `crate::error::Unreachable`
[`NetworkError`]: `crate::error::NetworkError`
