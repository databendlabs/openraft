### Error logs after `raft.shutdown()` completes

**Symptom**: After calling [`Raft::shutdown`][] which returns successfully, logs show
`ERROR openraft::raft::raft_inner: failure sending RaftMsg to RaftCore; message: AppendEntries ... core_result=Err(Stopped)`

**Cause**: Other nodes in the cluster continue sending RPCs to this node. The `Raft` handle still
exists and receives these RPCs, but the internal Raft core has stopped, so forwarding fails.

**Solution**: This is expected behavior. These errors are harmless - they indicate the node has
shut down as requested. You can ignore them or filter these specific error logs after shutdown.

Use [`Raft::shutdown_with_report`][] to inspect the core and tick task joins.
`TaskJoinOutcome::Joined` means a task returned and the runtime join succeeded. `Panicked` and
`Failed` expose join failures classified by `AsyncRuntime::is_panic`. Cancellation or abort
classification depends on the runtime. A node stopped before `run()` reports `NotStarted`. Repeated
calls retain the outcomes and indicate which tasks had already been joined. This report covers the
core and tick tasks only; it does not certify that application resources or other background tasks
have stopped. The existing `shutdown()` API discards this report. If a shutdown future is cancelled
while joining the core, a later `shutdown()` resumes that join and waits for the core to exit.

See: <https://github.com/databendlabs/openraft/issues/1357>

[`Raft::shutdown`]: `crate::Raft::shutdown`

[`Raft::shutdown_with_report`]: `crate::Raft::shutdown_with_report`
