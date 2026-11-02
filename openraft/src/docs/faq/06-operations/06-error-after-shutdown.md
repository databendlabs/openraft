### Error logs after `raft.shutdown()` completes

**Symptom**: After calling [`Raft::shutdown`][] which returns successfully, logs show
`ERROR openraft::raft::raft_inner: failure sending RaftMsg to RaftCore; message: AppendEntries ... core_result=Err(Stopped)`

**Cause**: Other nodes in the cluster continue sending RPCs to this node. The `Raft` handle still
exists and receives these RPCs, but the internal Raft core has stopped, so forwarding fails.

**Solution**: This is expected behavior. These errors are harmless - they indicate the node has
shut down as requested. You can ignore them or filter these specific error logs after shutdown.

Use [`Raft::try_shutdown`][] to receive `ShutdownError` when the core or tick task fails to join.
It awaits both tasks and retains failures across repeated calls. Cancelled calls can be retried.
The existing `shutdown()` API discards join errors.

See: <https://github.com/databendlabs/openraft/issues/1357>

[`Raft::shutdown`]: `crate::Raft::shutdown`

[`Raft::try_shutdown`]: `crate::Raft::try_shutdown`
