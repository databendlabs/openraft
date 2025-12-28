### How to minimize error logging when a follower is offline

Excessive error logging, like `ERROR openraft::replication: 248: RPCError err=NetworkError: ...`, occurs when a follower node becomes unresponsive. To alleviate this, implement a mechanism within [`RaftNetworkV2`][] that returns a [`Unreachable`][] error instead of a [`NetworkError`][] when immediate replication retries to the affected node are not advised.

[`RaftNetworkV2`]: `crate::network::RaftNetworkV2`
[`Unreachable`]: `crate::error::Unreachable`
[`NetworkError`]: `crate::error::NetworkError`
