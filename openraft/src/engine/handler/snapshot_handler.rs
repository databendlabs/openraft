use crate::engine::engine_impl::EngineOutput;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::SnapshotMeta;

/// Handle raft vote related operations
pub(crate) struct SnapshotHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) output: &'out mut EngineOutput<NID, N>,
}

impl<'st, 'out, NID, N> SnapshotHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Update engine state when a new snapshot is built or installed.
    ///
    /// Engine records only the metadata of a snapshot. Snapshot data is stored by RaftStorage
    /// implementation.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_snapshot(&mut self, meta: SnapshotMeta<NID, N>) -> bool {
        tracing::info!("update_snapshot: {:?}", meta);

        if meta.last_log_id <= self.state.snapshot_last_log_id().copied() {
            tracing::info!(
                "No need to install a smaller snapshot: current snapshot last_log_id({}), new snapshot last_log_id({})",
                self.state.snapshot_last_log_id().summary(),
                meta.last_log_id.summary()
            );
            return false;
        }

        self.state.snapshot_meta = meta;
        self.output.metrics_flags.set_data_changed();

        true
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreeset;
    use pretty_assertions::assert_eq;

    use crate::engine::Engine;
    use crate::testing::log_id;
    use crate::Membership;
    use crate::MetricsChangeFlags;
    use crate::SnapshotMeta;
    use crate::StoredMembership;

    fn m12() -> Membership<u64, ()> {
        Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
    }

    fn m1234() -> Membership<u64, ()> {
        Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
    }

    fn eng() -> Engine<u64, ()> {
        let mut eng = Engine::<u64, ()> { ..Default::default() };
        eng.state.enable_validate = false; // Disable validation for incomplete state

        eng.state.snapshot_meta = SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        };
        eng
    }

    #[test]
    fn test_update_snapshot_no_update() -> anyhow::Result<()> {
        // snapshot will not be updated because of equal or less `last_log_id`.
        let mut eng = eng();

        let got = eng.snapshot_handler().update_snapshot(SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        });

        assert_eq!(false, got);

        assert_eq!(
            SnapshotMeta {
                last_log_id: Some(log_id(2, 2)),
                last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
                snapshot_id: "1-2-3-4".to_string(),
            },
            eng.state.snapshot_meta
        );

        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: false,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert_eq!(0, eng.output.commands.len());

        Ok(())
    }

    #[test]
    fn test_update_snapshot_updated() -> anyhow::Result<()> {
        // snapshot will be updated to a new one with greater `last_log_id`.
        let mut eng = eng();

        let got = eng.snapshot_handler().update_snapshot(SnapshotMeta {
            last_log_id: Some(log_id(2, 3)),
            last_membership: StoredMembership::new(Some(log_id(2, 2)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        });

        assert_eq!(true, got);

        assert_eq!(
            SnapshotMeta {
                last_log_id: Some(log_id(2, 3)),
                last_membership: StoredMembership::new(Some(log_id(2, 2)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            },
            eng.state.snapshot_meta
        );

        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: true,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert_eq!(0, eng.output.commands.len());

        Ok(())
    }
}
