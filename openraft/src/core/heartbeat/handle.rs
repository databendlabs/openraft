use std::collections::BTreeMap;
use std::sync::Arc;

use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::Config;
use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchSender;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::heartbeat::worker::HeartbeatWorker;
use crate::core::notification::Notification;
use crate::engine::TargetProgress;
use crate::progress::stream_id::StreamId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchSenderOf;
use crate::vote::committed::CommittedVote;

/// Handle for a single heartbeat worker task.
pub(crate) struct WorkerHandle<C>
where C: RaftTypeConfig
{
    /// Channel to send heartbeat events to the worker.
    event_tx: WatchSenderOf<C, Option<HeartbeatEvent<C>>>,

    #[allow(dead_code)]
    stream_id: StreamId,

    /// Channel to signal shutdown to the worker.
    ///
    /// When this sender is dropped, the worker's receiver will detect it and shutdown.
    _shutdown_tx: OneshotSenderOf<C, ()>,

    /// Join handle for the worker task.
    _join_handle: JoinHandleOf<C, ()>,
}

pub(crate) struct HeartbeatWorkersHandle<C>
where C: RaftTypeConfig
{
    pub(crate) id: C::NodeId,

    pub(crate) config: Arc<Config>,

    pub(crate) workers: BTreeMap<C::NodeId, WorkerHandle<C>>,
}

impl<C> HeartbeatWorkersHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(id: C::NodeId, config: Arc<Config>) -> Self {
        Self {
            id,
            config,
            workers: Default::default(),
        }
    }

    pub(crate) fn broadcast(&self, events: impl IntoIterator<Item = (C::NodeId, HeartbeatEvent<C>)>) {
        for (target, event) in events {
            tracing::debug!("id={} target={} send_heartbeat {}", self.id, target, event);
            self.workers.get(&target).unwrap().event_tx.send(Some(event)).ok();
        }
    }

    pub(crate) fn close_workers(&mut self) {
        self.workers = Default::default();
    }

    pub(crate) async fn spawn_workers<NF>(
        &mut self,
        leader_vote: CommittedVote<C>,
        network_factory: &mut NF,
        tx_notification: &MpscSenderOf<C, Notification<C>>,
        progresses: impl IntoIterator<Item = &TargetProgress<C>>,
        close_old: bool,
    ) where
        NF: RaftNetworkFactory<C>,
    {
        let mut new_workers = BTreeMap::new();

        for prog in progresses {
            tracing::debug!("id={} spawn HeartbeatWorker target={}", self.id, prog.target);

            let removed = self.workers.remove(&prog.target);

            let handle = removed.filter(|_removed| !close_old);

            let handle = if let Some(handle) = handle {
                handle
            } else {
                let network = network_factory.new_client(prog.target.clone(), &prog.target_node).await;

                let (tx, rx) = C::watch_channel(None);

                let worker = HeartbeatWorker {
                    id: self.id.clone(),
                    leader_vote: leader_vote.clone(),
                    stream_id: prog.progress.stream_id,
                    rx,
                    network,
                    target: prog.target.clone(),
                    node: prog.target_node.clone(),
                    config: self.config.clone(),
                    tx_notification: tx_notification.clone(),
                };

                let span = tracing::span!(parent: &Span::current(), Level::DEBUG, "heartbeat", id=display(&self.id), target=display(&prog.target));

                let (tx_shutdown, rx_shutdown) = C::oneshot();

                let worker_handle = C::spawn(worker.run(rx_shutdown).instrument(span));

                WorkerHandle {
                    event_tx: tx,
                    stream_id: prog.progress.stream_id,
                    _shutdown_tx: tx_shutdown,
                    _join_handle: worker_handle,
                }
            };

            new_workers.insert(prog.target.clone(), handle);
        }

        // All the left are dropped and closed.
        let _left = std::mem::replace(&mut self.workers, new_workers);
    }
}
