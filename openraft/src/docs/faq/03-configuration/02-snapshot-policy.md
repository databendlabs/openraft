### How to customize snapshot-building policy?

OpenRaft provides a default snapshot building policy that triggers snapshots
when the log count exceeds a threshold. Configure this via [`Config::snapshot_policy`]
set to [`SnapshotPolicy::LogsSinceLast(n)`][`SnapshotPolicy::LogsSinceLast`].

To customize snapshot behavior:

- **Disable automatic snapshots**: Set [`Config::snapshot_policy`] to [`SnapshotPolicy::Never`]
- **Manual snapshot triggers**: Use [`Raft::trigger().snapshot()`][`Trigger::snapshot`] to build snapshots on demand

This allows full control over when snapshots are created based on your application's specific requirements.

[`Config::snapshot_policy`]: `crate::config::Config::snapshot_policy`
[`SnapshotPolicy::LogsSinceLast`]: `crate::config::SnapshotPolicy::LogsSinceLast`
[`SnapshotPolicy::Never`]: `crate::config::SnapshotPolicy::Never`
[`Trigger::snapshot`]: `crate::raft::trigger::Trigger::snapshot`
