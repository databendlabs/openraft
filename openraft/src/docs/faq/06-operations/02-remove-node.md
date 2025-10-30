### How to remove node-2 safely from a cluster `{1, 2, 3}`?

Call `Raft::change_membership(btreeset!{1, 3})` to exclude node-2 from
the cluster. Then wipe out node-2 data.
**NEVER** modify/erase the data of any node that is still in a raft cluster, unless you know what you are doing.
