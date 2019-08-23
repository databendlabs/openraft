Dynamic Membership
==================
Throughout the lifecycle of a Raft cluster, various nodes may need to go offline for various reasons. They may experience hardware or software errors which cause them to go offline when unintended, or perhaps a cluster had too many nodes and it needs to downsize. New nodes may be added to clusters as well in order to replace old nodes, nodes going offline for maintenence, or simply to increase the size of a cluster. Applications may control such events using the `ProposeConfigChange` command. This command allows for nodes to be safely added and removed form a running Raft cluster.

#### `ProposeConfigChange`
This command will propose a new config change to a running cluster. This command will fail if the Raft node to which this command was submitted is not the Raft leader, and the outcome of the proposed config change must not leave the cluster in a state where it will have less than two functioning nodes, as the cluster would no longer be able to make progress in a safe manner. Once the leader receives this command, the new configuration will be appended to the log and the Raft dynamic configuration change protocol will begin. For more details on how this is implemented, see §6 of the Raft spec.

Cluster auto-healing, where cluster members which have been offline for some period of time are automatically removed, is an application specific behavior, but is fully supported via this dynamic cluster membership system.

Likewise, dynamically adding new nodes to a running cluster based on an application's discovery system is also fully supported by this system.
