### What will happen when data gets lost?

Raft operates on the presumption that the storage medium (i.e., the disk) is
secure and reliable.

If this presumption is violated, e.g., the raft logs are lost or the snapshot is
damaged, no predictable outcome can be assured. In other words, the resulting
behavior is **undefined**.
