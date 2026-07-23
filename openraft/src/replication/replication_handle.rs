use crate::RaftTypeConfig;
use crate::errors::ReplicationClosed;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::replication::replicate::Replicate;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchSenderOf;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// Identifies this replication session (leader vote + target node).
    pub(crate) stream_id: StreamId,

    /// The channel used for communicating with the replication task.
    pub(crate) replicate_tx: WatchSenderOf<C, Replicate<C>>,

    /// Sender for the cancellation signal; dropping this stops replication.
    pub(crate) cancel_tx: WatchSenderOf<C, ()>,

    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) join_handle: Option<JoinHandleOf<C, Result<(), ReplicationClosed>>>,

    /// Handle to the snapshot transmitter task, if one is running.
    pub(crate) snapshot_transmit_handle: Option<SnapshotTransmitterHandle<C>>,
}

impl<C> ReplicationHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        stream_id: StreamId,
        replicate_tx: WatchSenderOf<C, Replicate<C>>,
        cancel_tx: WatchSenderOf<C, ()>,
    ) -> Self {
        Self {
            stream_id,
            join_handle: None,
            replicate_tx,
            snapshot_transmit_handle: None,
            cancel_tx,
        }
    }

    /// Clear the snapshot transmit handle if `inflight_id` matches the stored one.
    ///
    /// Returns `true` if cleared, `false` if the id did not match (stale notification
    /// from a superseded task) or no handle is stored.
    pub(crate) fn clear_snapshot_transmit_if_match(&mut self, inflight_id: InflightId) -> bool {
        if self.snapshot_transmit_handle.as_ref().map(|h| h.inflight_id) == Some(inflight_id) {
            self.snapshot_transmit_handle = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::testing::UTConfig;
    use crate::progress::inflight_id::InflightId;
    use crate::progress::stream_id::StreamId;
    use crate::replication::replicate::Replicate;
    use crate::replication::replication_handle::ReplicationHandle;
    use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
    use crate::type_config::TypeConfigExt;

    type C = UTConfig;

    fn make_snapshot_handle(inflight_id: u64) -> SnapshotTransmitterHandle<C> {
        let (cancel_tx, _cancel_rx) = C::watch_channel(());
        let join_handle = C::spawn(async {});

        SnapshotTransmitterHandle {
            _join_handle: join_handle,
            _tx_cancel: cancel_tx,
            inflight_id: InflightId::new(inflight_id),
        }
    }

    fn make_handle(snapshot_handle: Option<SnapshotTransmitterHandle<C>>) -> ReplicationHandle<C> {
        let (replicate_tx, _rx) = C::watch_channel(Replicate::<C>::default());
        let (cancel_tx, _cancel_rx) = C::watch_channel(());

        ReplicationHandle {
            stream_id: StreamId::new(1),
            replicate_tx,
            cancel_tx,
            join_handle: None,
            snapshot_transmit_handle: snapshot_handle,
        }
    }

    #[test]
    fn test_clear_snapshot_transmit_when_id_matches() {
        C::run(async {
            let mut handle = make_handle(Some(make_snapshot_handle(5)));

            assert!(handle.clear_snapshot_transmit_if_match(InflightId::new(5)));
            assert!(handle.snapshot_transmit_handle.is_none());
        });
    }

    #[test]
    fn test_keep_snapshot_transmit_when_id_mismatches() {
        C::run(async {
            let mut handle = make_handle(Some(make_snapshot_handle(5)));

            assert!(!handle.clear_snapshot_transmit_if_match(InflightId::new(3)));
            assert!(handle.snapshot_transmit_handle.is_some());
            assert_eq!(
                handle.snapshot_transmit_handle.as_ref().unwrap().inflight_id,
                InflightId::new(5)
            );
        });
    }

    #[test]
    fn test_clear_snapshot_transmit_when_already_none() {
        C::run(async {
            let mut handle = make_handle(None);

            assert!(!handle.clear_snapshot_transmit_if_match(InflightId::new(5)));
            assert!(handle.snapshot_transmit_handle.is_none());
        });
    }
}
