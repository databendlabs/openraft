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
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

pub(crate) struct HeartbeatWorkersHandle<C>
where C: RaftTypeConfig
{
    pub(crate) id: C::NodeId,

    pub(crate) config: Arc<Config>,

    /// Inform the heartbeat task to broadcast a heartbeat message.
    ///
    /// A Leader will periodically update this value to trigger sending heartbeat messages.
    pub(crate) tx: WatchSenderOf<C, Option<HeartbeatEvent<C>>>,

    /// The receiving end of heartbeat command.
    ///
    /// A separate task will have a clone of this receiver to receive and execute heartbeat command.
    pub(crate) rx: WatchReceiverOf<C, Option<HeartbeatEvent<C>>>,

    pub(crate) workers: BTreeMap<C::NodeId, (OneshotSenderOf<C, ()>, JoinHandleOf<C, ()>)>,
}

impl<C> HeartbeatWorkersHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(id: C::NodeId, config: Arc<Config>) -> Self {
        let (tx, rx) = C::watch_channel(None);

        Self {
            id,
            config,
            tx,
            rx,
            workers: Default::default(),
        }
    }

    pub(crate) fn broadcast(&self, event: HeartbeatEvent<C>) {
        tracing::debug!("id={} send_heartbeat {}", self.id, event);
        let _ = self.tx.send(Some(event));
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

            let worker = HeartbeatWorker {
                id: self.id.clone(),
                rx: self.rx.clone(),
                network,
                target: target.clone(),
                node,
                config: self.config.clone(),
                tx_notification: tx_notification.clone(),
            };

            let span = tracing::span!(parent: &Span::current(), Level::DEBUG, "heartbeat", id=display(&self.id), target=display(&target));

            let (tx_shutdown, rx_shutdown) = C::oneshot();

            let worker_handle = C::spawn(worker.run(rx_shutdown).instrument(span));
            self.workers.insert(target, (tx_shutdown, worker_handle));
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.workers.clear();
        tracing::info!("id={} HeartbeatWorker are shutdown", self.id);
    }
}
