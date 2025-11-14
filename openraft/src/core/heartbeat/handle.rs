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
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchSenderOf;

/// Handle for a single heartbeat worker task.
pub(crate) struct WorkerHandle<C>
where C: RaftTypeConfig
{
    /// Channel to send heartbeat events to the worker.
    event_tx: WatchSenderOf<C, Option<HeartbeatEvent<C>>>,

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

    pub(crate) async fn spawn_workers<NF>(
        &mut self,
        network_factory: &mut NF,
        tx_notification: &MpscSenderOf<C, Notification<C>>,
        targets: impl IntoIterator<Item = (C::NodeId, C::Node)>,
    ) where
        NF: RaftNetworkFactory<C>,
    {
        for (target, node) in targets {
            tracing::debug!("id={} spawn HeartbeatWorker target={}", self.id, target);
            let network = network_factory.new_client(target.clone(), &node).await;

            let (tx, rx) = C::watch_channel(None);

            let worker = HeartbeatWorker {
                id: self.id.clone(),
                rx,
                network,
                target: target.clone(),
                node,
                config: self.config.clone(),
                tx_notification: tx_notification.clone(),
            };

            let span = tracing::span!(parent: &Span::current(), Level::DEBUG, "heartbeat", id=display(&self.id), target=display(&target));

            let (tx_shutdown, rx_shutdown) = C::oneshot();

            let worker_handle = C::spawn(worker.run(rx_shutdown).instrument(span));

            let handle = WorkerHandle {
                event_tx: tx,
                _shutdown_tx: tx_shutdown,
                _join_handle: worker_handle,
            };

            self.workers.insert(target, handle);
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.workers.clear();
        tracing::info!("id={} HeartbeatWorker are shutdown", self.id);
    }
}
