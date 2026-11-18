use std::collections::BTreeMap;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use openraft::Instant;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::Select2;
use openraft::base::BoxFuture;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::network::RPCOption;
use openraft::raft::AppendResult;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::InstantOf;
use openraft::type_config::alias::MpscReceiverOf;
use openraft::type_config::alias::MpscSenderOf;
use openraft::type_config::alias::OneshotSenderOf;

use crate::BatchConfig;
use crate::BatchItem;
use crate::BatchTransport;

/// The queue that feeds the batching task of one peer node.
pub(crate) type PeerQueue<C, G> = MpscSenderOf<C, Pending<C, G>>;

/// A batched request in the queue of its peer, with the channel its caller waits on.
pub(crate) struct Pending<C, G>
where C: RaftTypeConfig
{
    pub(crate) item: BatchItem<C, G>,

    /// When the caller gives up on this request.
    pub(crate) deadline: InstantOf<C>,

    pub(crate) reply_tx: OneshotSenderOf<C, Result<AppendResult<C>, RPCError<C>>>,
}

/// What [`PeerBatcher::next_request`] waited for.
enum Next<C, G>
where C: RaftTypeConfig
{
    Request(Pending<C, G>),
    Timeout,
    Closed,
}

/// Collects the batched requests to one peer node, and sends them in batches.
///
/// One task runs per peer node. While it collects the next batch, it drives the exchanges of the
/// batches in flight, so a reply reaches its caller without waiting for the next flush.
pub(crate) struct PeerBatcher<C, G, T>
where
    C: RaftTypeConfig,
    G: OptionalSend + 'static,
{
    target: C::NodeId,
    transport: T,
    config: BatchConfig,
    rx: MpscReceiverOf<C, Pending<C, G>>,
    in_flight: FuturesUnordered<BoxFuture<'static, ()>>,
}

impl<C, G, T> PeerBatcher<C, G, T>
where
    C: RaftTypeConfig,
    G: OptionalSend + 'static,
    T: BatchTransport<C, G>,
{
    /// Spawn the task for `target`, and return the queue that feeds it.
    ///
    /// The task ends once every sender of the queue is dropped and its batches in flight finish.
    pub(crate) fn spawn(target: C::NodeId, transport: T, config: BatchConfig) -> PeerQueue<C, G> {
        let (tx, rx) = C::mpsc(config.max_queued.get());
        let batcher = Self {
            target,
            transport,
            config,
            rx,
            in_flight: FuturesUnordered::new(),
        };
        let _join = C::spawn(batcher.run());
        tx
    }

    async fn run(mut self) {
        loop {
            while self.in_flight.len() >= self.config.max_in_flight.get() {
                self.in_flight.next().await;
            }

            let first = match self.next_request(None).await {
                Next::Request(pending) => pending,
                Next::Timeout => continue,
                Next::Closed => break,
            };

            // The first request sets the flush deadline: a later request can only bring it forward.
            let mut flush_at = std::cmp::min(C::now() + self.config.max_delay, first.deadline);
            let mut batch = vec![first];
            let mut closed = false;

            while batch.len() < self.config.max_items.get() {
                match self.next_request(Some(flush_at)).await {
                    Next::Request(pending) => {
                        flush_at = std::cmp::min(flush_at, pending.deadline);
                        batch.push(pending);
                    }
                    Next::Timeout => break,
                    Next::Closed => {
                        closed = true;
                        break;
                    }
                }
            }

            let exchange = Self::exchange(self.target.clone(), self.transport.clone(), batch);
            self.in_flight.push(Box::pin(exchange));

            if closed {
                break;
            }
        }

        // Every sender is gone: finish the batches in flight, so that their callers get replies.
        while self.in_flight.next().await.is_some() {}
    }

    /// Wait for the next queued request until `deadline`, driving the batches in flight meanwhile.
    async fn next_request(&mut self, deadline: Option<InstantOf<C>>) -> Next<C, G> {
        loop {
            let rx = &mut self.rx;
            let recv = async move {
                match deadline {
                    None => Some(rx.recv().await),
                    Some(at) => C::timeout_at(at, rx.recv()).await.ok(),
                }
            };

            let received = if self.in_flight.is_empty() {
                recv.await
            } else {
                match C::select2(recv, self.in_flight.next()).await {
                    Select2::First(received) => received,
                    // A batch in flight finished; keep waiting.
                    Select2::Second(_) => continue,
                }
            };

            return match received {
                Some(Some(pending)) => Next::Request(pending),
                Some(None) => Next::Closed,
                None => Next::Timeout,
            };
        }
    }

    /// Send `batch` through `transport`, and deliver every reply to its caller.
    ///
    /// The exchange may last until the latest deadline of the batch. A request without a reply by
    /// then, or when the reply stream ends, fails; so does every request when the send fails.
    async fn exchange(target: C::NodeId, transport: T, batch: Vec<Pending<C, G>>) {
        let latest = batch.iter().map(|pending| pending.deadline).max().unwrap_or_else(C::now);
        let ttl = latest.saturating_duration_since(C::now());

        let mut waiting = BTreeMap::new();
        let mut items = Vec::with_capacity(batch.len());
        for Pending { item, reply_tx, .. } in batch {
            let prev_log_id = item.rpc.prev_log_id.clone();
            let last_log_id = item.rpc.last_log_id();
            waiting.insert(item.id, (prev_log_id, last_log_id, reply_tx));
            items.push(item);
        }

        let deliver = async {
            let mut replies = transport.send_batch(target.clone(), items, RPCOption::new(ttl)).await?;
            while let Some(reply) = replies.next().await {
                let Some((prev_log_id, last_log_id, reply_tx)) = waiting.remove(&reply.id) else {
                    tracing::warn!(
                        "ignore a batch reply from node {} with an unknown or repeated id {}",
                        target,
                        reply.id
                    );
                    continue;
                };
                let result = reply.result.map(|resp| resp.into_append_result(prev_log_id, last_log_id));
                let _ = reply_tx.send(result);
            }
            Ok::<(), RPCError<C>>(())
        };

        let failure = match C::timeout(ttl, deliver).await {
            Ok(Ok(())) => {
                let message = format!("no reply from node {} for a batched AppendEntries", target);
                RPCError::Network(NetworkError::from_string(message))
            }
            Ok(Err(error)) => error,
            Err(_) => {
                let message = format!("a batch to node {} got no reply within {:?}", target, ttl);
                RPCError::Network(NetworkError::from_string(message))
            }
        };

        for (_, (_, _, reply_tx)) in waiting {
            let _ = reply_tx.send(Err(failure.clone()));
        }
    }
}
