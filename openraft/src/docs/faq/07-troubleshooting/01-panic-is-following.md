### Panic: "assertion failed: self.internal_server_state.is_following()"

**Symptom**: Node crashes with `panicked at 'assertion failed: self.internal_server_state.is_following()'`

**Cause**: [`RaftNetworkFactory`][] creates a connection from a node to itself. When this node
becomes leader, it sends replication messages to itself, but Openraft expects only followers to
receive replication messages.

**Solution**: In [`RaftNetworkFactory::new_client`][], ensure the target node ID never equals
the local node's ID. Each node ID in [`Membership`][] must map to a different node.

[`RaftNetworkFactory`]: `crate::network::RaftNetworkFactory`
[`RaftNetworkFactory::new_client`]: `crate::network::RaftNetworkFactory::new_client`
[`Membership`]: `crate::Membership`
