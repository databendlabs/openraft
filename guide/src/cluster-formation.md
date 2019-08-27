Cluster Formation
=================
All Raft nodes, when they first come online in a pristine state, will enter into the `NonVoter` state, which is a completely passive state. This gives the parent application the ability to issue admin commands to the node based on the intention of the parent application.

To form a new cluster, all application nodes must issue the `InitWithConfig` command to their embedded Raft nodes with the IDs of all discovered nodes which are to be part of the cluster (including the ID of the running node). Or if the application is to run in a standalone / single-node manner, it may issue the command with only its own ID.

#### `InitWithConfig`
This command is used exclusively for the formation of new clusters. This command will fail if the node is not in the `NonVoter` state, or if the node's log index is not `0`.

This will cause the Raft node to hold the given configuration in memory and then immediately perform the election protocol. For single-node clusters, the node will immediately become leader, for multi-node clusters it will submit `RequestVote` RPCs to all of the nodes in the given config list. **NOTE WELL that EVERY node in the cluster MUST perform this action** when a new cluster is being formed. It is safe for all nodes to issue this command in parallel. Once this process has been completed, the newly elected leader will append the given membership config data to the log, ensuring that the new configuration will be reckoned as the initial cluster configuration moving forward throughout the life of the cluster.

However, in order to ensure that multiple independent clusters aren't formed by prematurely issuing the `InitWithConfig` command before all peers are discovered, it would be prudent to have all discovered node's exchange some information during their handshake protocol. This will allow the parent application to make informed decisions as to whether the `InitWithConfig` should be called and how early it should be called when starting a new cluster. An application level configuration for this facet is recommended.

Generally speaking, an application config like `initial_cluster_formation_delay` (or the like), which configures the application to wait for the specifed amount of time before issuing an `InitWithConfig` command, should do the trick. The value for such a configuration should simply be a few orders of magnitude greater than the amount of time it takes for all the nodes of a new cluster to come online and discover each other.

As a rule of thumb, when new nodes come online, the leader of an existing Raft cluster will eventually discover the node (via the application's discovery system), and in such cases, the application should submit a new `ProposeConfigChange` to the leader to add it to the cluster. The same goes for removing nodes from the cluster.

**For single-node clusters**, scaling up the cluster by adding new nodes via the `ProposeConfigChange` command should work as expected, but there is one invariant which must be upheld: the original node of the cluster must remain online until at least half of the other new nodes have been brough up-to-date, otherwise the Raft cluster will not be able to make progress. After the other nodes have been brought up-to-date, everything should run normally according to the Raft spec.
