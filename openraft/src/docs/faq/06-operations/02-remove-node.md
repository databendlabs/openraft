### How to remove node-2 safely from a cluster `{1, 2, 3}`?

Call `Raft::change_membership(btreeset!{1, 3})` to exclude node-2 from
the cluster. Then wipe out node-2 data.
**NEVER** modify/erase the data of any node that is still in a raft cluster, unless you know what you are doing.

Do not give node-2's ID to a different node afterwards: reusing an ID can cause
split-brain. See:
[Can I reuse the ID of a removed node for a new node?](#can-i-reuse-the-id-of-a-removed-node-for-a-new-node)
