use std::time::Duration;

use anyerror::AnyError;
use futures::FutureExt;

use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::StorageError;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::notification::Notification;
use crate::core::sm::handle::SnapshotReader;
use crate::display_ext::DisplayOptionExt;
use crate::error::HigherVote;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::ReplicationError;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::network::v2::RaftNetworkV2;
use crate::progress::replication_id::ReplicationId;
use crate::replication::Progress;
use crate::replication::replication_task_state::ReplicationTaskState;
use crate::replication::response::ReplicationResult;
use crate::replication::snapshot_transmit_handle::SnapshotTransmitHandle;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::vote::raft_vote::RaftVoteExt;

pub(crate) struct SnapshotTransmit<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
{
    pub(crate) task_state: ReplicationTaskState<C>,

    replication_id: ReplicationId,

    /// For receiving cancel signal.
    rx_cancel: WatchReceiverOf<C, ()>,

    /// Another `RaftNetwork` specific for snapshot replication.
    ///
    /// Snapshot transmitting is a long-running task and is processed in a separate task.
    network: N::Network,

    /// The backoff policy if an [`Unreachable`](`crate::error::Unreachable`) error is returned.
    /// It will be reset to `None` when a successful response is received.
    backoff: Option<Backoff>,

    /// The handle to get a snapshot directly from the state machine.
    snapshot_reader: SnapshotReader<C>,
}

impl<C, N> SnapshotTransmit<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
{
    pub(crate) fn spawn(
        replication_task_state: ReplicationTaskState<C>,
        network: N::Network,
        snapshot_reader: SnapshotReader<C>,
        replication_id: ReplicationId,
    ) -> SnapshotTransmitHandle<C> {
        let (tx_cancel, rx_cancel) = C::watch_channel(());
        let snapshot_transmit = Self {
            task_state: replication_task_state,
            replication_id,
            rx_cancel,
            network,
            backoff: None,
            snapshot_reader,
        };

        let join_handle = C::spawn(snapshot_transmit.stream_snapshot());

        SnapshotTransmitHandle {
            _join_handle: join_handle,
            _tx_cancel: tx_cancel,
        }
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn stream_snapshot(mut self) {
        tracing::info!("{}", func_name!());

        let mut ith: i32 = -1;
        loop {
            ith += 1;

            let res = self.read_and_send_snapshot(ith).await;

            let error = match res {
                Err(error) => error,
                Ok(_) => {
                    return;
                }
            };

            tracing::error!("ReplicationError: {}; when (sending snapshot)", error);

            match error {
                ReplicationError::Closed(closed) => {
                    tracing::info!("Snapshot transmitting is canceled: {}", closed);
                    return;
                }
                ReplicationError::HigherVote(h) => {
                    tracing::info!("Snapshot transmitting has seen a higher vote: {}, notify and quit", h);
                    self.task_state
                        .tx_notify
                        .send(Notification::HigherVote {
                            target: self.task_state.target,
                            higher: h.higher,
                            leader_vote: self.task_state.session_id.committed_vote(),
                        })
                        .await
                        .ok();

                    return;
                }
                ReplicationError::StorageError(error) => {
                    tracing::error!(error=%error, "error replication to target={}", self.task_state.target);
                    self.task_state.tx_notify.send(Notification::StorageError { error }).await.ok();
                    return;
                }
                ReplicationError::RPCError(err) => {
                    match &err {
                        RPCError::Unreachable(_unreachable) => {
                            // If there is an [`Unreachable`] error, we will backoff for a
                            // period of time. Backoff will be reset if there is a
                            // successful RPC is sent.
                            if self.backoff.is_none() {
                                self.backoff = Some(self.network.backoff());
                            }
                        }
                        RPCError::Timeout(_) | RPCError::Network(_) | RPCError::RemoteError(_) => {
                            self.backoff = None;
                        }
                    };

                    if let Some(b) = &mut self.backoff {
                        let duration = b.next().unwrap_or_else(|| {
                            tracing::warn!("backoff exhausted, using default");
                            Duration::from_millis(500)
                        });

                        let sleep = C::sleep(duration);
                        let recv = self.rx_cancel.changed();

                        futures::select! {
                            _ = sleep.fuse() => {
                                tracing::debug!("backoff timeout");
                            }
                            _ = recv.fuse() => {
                                tracing::info!("Snapshot transmitting is canceled by RaftCore");
                                return;
                            }
                        }
                    }
                }
            };
        }
    }

    async fn read_and_send_snapshot(&mut self, ith: i32) -> Result<(), ReplicationError<C>> {
        let snapshot = self.snapshot_reader.get_snapshot().await.map_err(|reason| {
            tracing::warn!(error = display(&reason), "failed to get snapshot from state machine");
            ReplicationClosed::new(reason)
        })?;

        tracing::info!(
            "{}-th snapshot sending: has read snapshot: meta:{}",
            ith,
            snapshot.as_ref().map(|x| &x.meta).display()
        );

        let snapshot = match snapshot {
            None => {
                let sto_err = StorageError::read_snapshot(None, AnyError::error("snapshot not found"));
                return Err(sto_err.into());
            }
            Some(x) => x,
        };

        let mut option = RPCOption::new(self.task_state.config.install_snapshot_timeout());
        option.snapshot_chunk_size = Some(self.task_state.config.snapshot_max_chunk_size as usize);

        self.send_snapshot(snapshot, option).await
    }

    async fn send_snapshot(&mut self, snapshot: Snapshot<C>, option: RPCOption) -> Result<(), ReplicationError<C>> {
        let meta = snapshot.meta.clone();

        let mut c = self.rx_cancel.clone();
        let cancel = async move {
            c.changed().await.ok();
            ReplicationClosed::new("RaftCore is dropped")
        };

        let vote = self.task_state.session_id.vote();

        let start_time = C::now();

        let resp = self.network.full_snapshot(vote, snapshot, cancel, option).await?;

        tracing::info!("finished sending full_snapshot, resp: {}", resp);

        // Handle response conditions.
        let sender_vote = self.task_state.session_id.vote();
        if resp.vote.as_ref_vote() > sender_vote.as_ref_vote() {
            return Err(ReplicationError::HigherVote(HigherVote {
                higher: resp.vote,
                sender_vote,
            }));
        }

        self.notify_heartbeat_progress(start_time).await;
        self.notify_progress(ReplicationResult(Ok(meta.last_log_id))).await;
        Ok(())
    }

    async fn notify_heartbeat_progress(&mut self, sending_time: InstantOf<C>) {
        self.task_state
            .tx_notify
            .send({
                Notification::HeartbeatProgress {
                    session_id: self.task_state.session_id.clone(),
                    target: self.task_state.target.clone(),
                    sending_time,
                }
            })
            .await
            .ok();
    }

    async fn notify_progress(&mut self, replication_result: ReplicationResult<C>) {
        tracing::debug!(
            target = display(self.task_state.target.clone()),
            result = display(&replication_result),
            "{}",
            func_name!()
        );

        self.task_state
            .tx_notify
            .send({
                Notification::ReplicationProgress {
                    has_payload: true,
                    progress: Progress {
                        session_id: self.task_state.session_id.clone(),
                        target: self.task_state.target.clone(),
                        result: Ok(replication_result.clone()),
                    },
                    replication_id: Some(self.replication_id),
                }
            })
            .await
            .ok();
    }
}
