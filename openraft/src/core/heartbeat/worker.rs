use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::Stream;
use futures_util::StreamExt;

use crate::Config;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::heartbeat::errors::RaftCoreClosed;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::NetStreamAppend;
use crate::network::RPCOption;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendError;
use crate::raft::AppendResult;
use crate::replication::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CancelHandleOf;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

/// A dedicated worker sending heartbeat to a specific follower.
pub struct HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: NetStreamAppend<C>,
{
    pub(crate) id: C::NodeId,

    /// The leader this heartbeat worker works for
    pub(crate) leader_vote: CommittedVoteOf<C>,

    /// A unique stream.
    pub(crate) stream_id: StreamId,

    /// The receiver will be changed when a new heartbeat is needed to be sent.
    pub(crate) rx: WatchReceiverOf<C, Option<HeartbeatEvent<C>>>,

    pub(crate) network: N,

    pub(crate) target: C::NodeId,

    #[allow(dead_code)]
    pub(crate) node: C::Node,

    pub(crate) config: Arc<Config>,

    /// For sending back result to the [`RaftCore`].
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) tx_notification: MpscSenderOf<C, Notification<C>>,
}

impl<C, N> fmt::Display for HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: NetStreamAppend<C>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeartbeatWorker(id={}, target={})", self.id, self.target)
    }
}

impl<C, N> HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: NetStreamAppend<C>,
{
    pub(crate) fn run(self) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        let (cancel, future) = C::cancellable(self.do_run());
        // An `async` block awaiting `future` would reserve its bytes twice: once for the captured
        // variable and once for the moved value it polls (rust-lang/rust#62958). `map()` stores
        // it once.
        let task = future.map(|res| {
            tracing::info!("HeartbeatWorker finished with result: {:?}", res);
        });
        (cancel, task)
    }

    async fn do_run(mut self) -> Result<(), RaftCoreClosed> {
        loop {
            tracing::debug!("{} is waiting for a new heartbeat event.", self);

            let changed = self.rx.changed().await;
            if changed.is_err() {
                return Ok(());
            }

            // Mark the event read here as seen: an event sent after `changed()` returned is read
            // here, and would otherwise wake the next `changed()` and be sent a second time. The
            // guard is dropped at the end of this statement, before any await.
            let heartbeat: Option<HeartbeatEvent<C>> = self.rx.borrow_and_update().clone();

            // None is the initial value of the WatchReceiver, ignore it.
            let Some(heartbeat) = heartbeat else {
                continue;
            };

            let timeout = Duration::from_millis(self.config.heartbeat_interval);
            let option = RPCOption::new(timeout);

            let payload = AppendEntriesRequest {
                vote: self.leader_vote.clone().into_vote(),
                // Use last known matching log id as prev_log_id to detect follower state reversion.
                // prev_log_id == None does not conflict.
                //
                // Fail test `t99_issue_1500_heartbeat_cause_reversion_panic` by changing the
                // following line to `prev_log_id = heartbeat.cluster_committed.clone()`.
                prev_log_id: heartbeat.matching.clone(),
                leader_commit: heartbeat.cluster_committed.clone(),
                entries: vec![],
            };

            // `stream::iter()` over an array is `Unpin`, so the one-request input needs no
            // `Box::pin()` and no allocation.
            let input_stream = futures_util::stream::iter([payload]);

            let reply = self.send_heartbeat(input_stream, option, C::now() + timeout).await;

            tracing::debug!("{} sent a heartbeat: {}, reply: {:?}", self, heartbeat, reply);

            if let Some(reply) = reply {
                self.handle_stream_result(reply, &heartbeat).await?;
            }
        }
    }

    /// Sends one heartbeat and reads the responses that arrive before `deadline`.
    ///
    /// A transport may return more than one response, so all of them are read, and the one that
    /// matters most is returned: a higher vote, then a conflict, then a success. The stream is
    /// polled to its end, which lets the network close the connection cleanly. Each wait is
    /// bounded by `deadline` on its own, and a response is kept once it is received, so a stream
    /// that is slow to end cannot discard it. Returns `None`, after logging why, when no response
    /// arrived.
    async fn send_heartbeat<S>(
        &mut self,
        input: S,
        option: RPCOption,
        deadline: InstantOf<C>,
    ) -> Option<AppendResult<C>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        // The response stream borrows the network, so the messages below name the fields that
        // `Display` would print.
        let (id, target) = (&self.id, &self.target);

        let opened = C::timeout_at(deadline, self.network.stream_append(input, option)).await;
        let mut output = match opened {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                tracing::warn!("id={} target={} failed to send a heartbeat: {}", id, target, error);
                return None;
            }
            Err(_elapsed) => {
                tracing::warn!("id={} target={} timed out sending a heartbeat", id, target);
                return None;
            }
        };

        let mut reply: Option<AppendResult<C>> = None;
        loop {
            let next = C::timeout_at(deadline, output.next()).await;
            let result = match next {
                Ok(Some(Ok(result))) => result,
                Ok(Some(Err(error))) => {
                    tracing::warn!("id={} target={} heartbeat response failed: {}", id, target, error);
                    break;
                }
                Ok(None) => {
                    if reply.is_none() {
                        tracing::warn!("id={} target={} heartbeat stream ended without a response", id, target);
                    }
                    break;
                }
                Err(_elapsed) => {
                    tracing::warn!("id={} target={} heartbeat responses did not end in time", id, target);
                    break;
                }
            };

            if reply.as_ref().is_none_or(|kept| importance(&result) > importance(kept)) {
                reply = Some(result);
            }
        }
        reply
    }

    /// Handle the stream append result, send appropriate notifications.
    async fn handle_stream_result(
        &self,
        result: AppendResult<C>,
        heartbeat: &HeartbeatEvent<C>,
    ) -> Result<(), RaftCoreClosed> {
        match result {
            Ok(_) => {
                self.send_heartbeat_progress(heartbeat).await?;
            }
            Err(AppendError::HigherVote(vote)) => {
                tracing::debug!(
                    "seen a higher vote({vote}) from {}; when:(sending heartbeat)",
                    self.target
                );

                let noti = Notification::HigherVote {
                    target: self.target.clone(),
                    higher: vote,
                    leader_vote: self.leader_vote.clone(),
                };

                self.send_notification(noti, "Seeing higher Vote").await?;
                // Higher vote means leadership is not granted, don't send HeartbeatProgress
            }
            Err(AppendError::Conflict { .. }) => {
                // The follower does not have `matching` log id.
                // Use `matching` (which may be None) as the conflict point.
                //
                // Safe unwrap(): a None never conflict
                let conflict_log_id = heartbeat.matching.clone().unwrap();

                // Drop the conflict hint: a heartbeat has no inflight id, so RaftCore cannot tell
                // whether the hint answers its current request, and ignores it.
                let noti = Notification::ReplicationProgress {
                    stream_id: self.stream_id,
                    progress: Progress {
                        target: self.target.clone(),
                        result: Ok(ReplicationResult(Err(conflict_log_id))),
                        hint: None,
                    },
                    inflight_id: None,
                };

                self.send_notification(noti, "Seeing conflict").await?;
                self.send_heartbeat_progress(heartbeat).await?;
            }
        }
        Ok(())
    }

    async fn send_heartbeat_progress(&self, heartbeat: &HeartbeatEvent<C>) -> Result<(), RaftCoreClosed> {
        let noti = Notification::HeartbeatProgress {
            stream_id: self.stream_id,
            sending_time: heartbeat.time,
            target: self.target.clone(),
        };
        self.send_notification(noti, "send HeartbeatProgress").await
    }

    async fn send_notification(
        &self,
        notification: Notification<C>,
        when: impl fmt::Display,
    ) -> Result<(), RaftCoreClosed> {
        let res = self.tx_notification.send(notification).await;

        if let Err(e) = res {
            let notification = e.0;
            tracing::error!("{self} failed to send {notification} to RaftCore; when:({when})");
            return Err(RaftCoreClosed);
        }
        Ok(())
    }
}

/// How much a heartbeat response tells the leader: a higher vote ends its leadership, a conflict
/// reports a log without the matching entry, and a success only acknowledges it.
fn importance<C>(result: &AppendResult<C>) -> u8
where C: RaftTypeConfig {
    match result {
        Ok(_) => 0,
        Err(AppendError::Conflict { .. }) => 1,
        Err(AppendError::HigherVote(_)) => 2,
    }
}

#[cfg(test)]
mod worker_test;
