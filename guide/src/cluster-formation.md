Cluster Formation
=================
All Raft nodes, when they first come online in a pristine state, will enter into the `Learner` state, which is a completely passive state.

To form a new cluster, application nodes must call the `Raft.initialize` method with the IDs of all discovered nodes which are to be part of the cluster (including the ID of the running node). Or if the application is to run in a standalone / single-node manner, it may issue the command with only its own ID.

#### `Raft.initialize`
This method is used exclusively for the formation of new clusters. This command will fail if the node is not in the `Learner` state, or if the node's log index is greater than `0`.

This will cause the Raft node to hold the given configuration in memory and then immediately perform the election protocol. For single-node clusters, the node will immediately become leader, for multi-node clusters it will submit `RequestVote` RPCs to all of the nodes in the given config list. **NOTE WELL that it is safe for EVERY node in the cluster to perform this action in parallel** when a new cluster is being formed. Once this process has been completed, the newly elected leader will append the given membership config data to the log, ensuring that the new configuration will be reckoned as the initial cluster configuration moving forward throughout the life of the cluster.

In order to ensure that multiple independent clusters aren't formed by prematurely calling the `Raft.initialize` method before all peers are discovered, it is recommended that applications adopt a configurable `cluster_formation_delay` setting. The value for such a configuration should simply be a few orders of magnitude greater than the amount of time it takes for all the nodes of a new cluster to come online and discover each other. There are alternative patterns which may be used. Ultimately, this is subject to the design of the application.

As a rule of thumb, when new nodes come online, the leader of an existing Raft cluster will eventually discover the node (via the application's discovery system), and in such cases, the application could call the `Raft.add_learner` method to begin syncing the new node with the cluster. Once it is finished syncing, then applications should call the `Raft.change_membership` method to add the new node as a voting member of the cluster. For removing nodes from the cluster, the leader should call `Raft.change_membership` with the updated config, no preliminary steps are needed. See the next section for more details on this subject.
