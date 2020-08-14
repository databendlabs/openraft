Dynamic Membership
==================
Throughout the lifecycle of a Raft cluster, nodes will come and go. New nodes may need to be added to the cluster for various application specific reasons. Nodes may experience hardware failure and end up going offline. This implementation of Raft offers two mechanisms for controlling these lifecycle events.

#### `Raft.add_non_voter`
This method will add a new non-voter to the cluster and will immediately begin syncing the node with the leader. This method may be called multiple times as needed. The `Future` returned by calling this method will resolve once the node is up-to-date and is ready to be added as a voting member of the cluster.

#### `Raft.change_membership`
This method will start a cluster membership change. If there are any new nodes in the given config which were not previously added as non-voters from an earlier call to `Raft.add_non_voter`, then those nodes will begin the sync process. It is recommended that applications always call `Raft.add_non_voter` first when adding new nodes to the cluster, as this offers a bit more flexibility. Once `Raft.change_membership` is called, it can not be called again until the reconfiguration process is complete (which is typically quite fast).

Cluster auto-healing — where cluster members which have been offline for some period of time are automatically removed — is an application specific behavior, but is fully supported via this dynamic cluster membership system. Simply call `Raft.change_membership` with the dead node removed from the membership set.

Cluster leader stepdown is also fully supported. Nothing special needs to take place. Simply call `Raft.change_membership` with the ID of the leader removed from the membership set. The leader will recognize that it is being removed from the cluster, and will stepdown once it has committed the config change to the cluster according to the safety protocols defined in the Raft spec.
