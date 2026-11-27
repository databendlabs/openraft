use std::fmt;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::future::Either;

use crate::Config;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::RecvError;
use crate::async_runtime::Select2;
use crate::async_runtime::Select4;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::heartbeat::errors::HeartbeatFailure;
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
///
/// It keeps at most one heartbeat in flight, and sends the next one only once the last one has
/// ended, even after its deadline: see `send_heartbeat()`.
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
    /// Returns a cancel handle and the future of the worker, which ends when the worker stops.
    ///
    /// Dropping the handle stops the worker: at once while it waits for an event or for room in
    /// the channel to RaftCore, and while it sends a heartbeat, once the heartbeat has ended or
    /// `election_timeout_max` has passed, see `send_heartbeat()`. The worker also stops once its
    /// event channel has closed and it has handled the events sent before.
    pub(crate) fn run(self) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        // `stopped` completes once `cancel` is dropped.
        let (cancel, stopped) = C::cancellable(std::future::pending::<()>());
        // An `async` block awaiting `do_run()` would reserve its bytes twice: once for the captured
        // variable and once for the moved value it polls (rust-lang/rust#62958). `map()` stores
        // it once.
        let task = self.do_run(stopped.fuse()).map(|res| {
            tracing::info!("HeartbeatWorker finished with result: {:?}", res);
        });
        (cancel, task)
    }

    async fn do_run<Stop>(mut self, stopped: Stop) -> Result<(), RaftCoreClosed>
    where Stop: Future + OptionalSend {
        let mut stopped = std::pin::pin!(stopped);

        // The heartbeats in a row that got no reply in time, since the target last replied in
        // time. Only the first of them logs a warning, so that a target that is down, and fails
        // every heartbeat, logs one warning instead of one per heartbeat.
        let mut failed_in_row: u64 = 0;

        // Whether a heartbeat event arrived while the last heartbeat was still in flight. The
        // worker then sends the latest event at once, instead of waiting for another one.
        let mut event_pending = false;

        loop {
            if !event_pending {
                tracing::debug!("{} is waiting for a new heartbeat event.", self);
            }
            let next_event = async { if event_pending { Ok(()) } else { self.rx.changed().await } };
            // `stopped` is polled first, so that a stopped worker does not send a pending event.
            match C::select2(stopped.as_mut(), next_event).await {
                Select2::First(_) | Select2::Second(Err(_)) => return Ok(()),
                Select2::Second(Ok(())) => {}
            }
            event_pending = false;

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

            let sent = self
                .send_heartbeat(
                    input_stream,
                    option,
                    stopped.as_mut(),
                    &mut failed_in_row,
                    &mut event_pending,
                )
                .await;
            let ControlFlow::Continue(reply) = sent else {
                // The worker stopped while the heartbeat was in flight.
                return Ok(());
            };

            tracing::debug!("{} sent a heartbeat: {}, reply: {:?}", self, heartbeat, reply);

            if let Some(reply) = reply {
                // No request is in flight any more, so a stop ends the worker at once, even while
                // the channel to RaftCore is full.
                let handled = C::cancellable_by(self.handle_stream_result(reply, &heartbeat), stopped.as_mut()).await;
                let Some(handled) = handled else {
                    return Ok(());
                };
                handled?;
            }
        }
    }

    /// Sends one heartbeat, reads its responses until the response stream ends, and returns the
    /// one that matters most: a higher vote, then a conflict, then a success.
    /// `ControlFlow::Break` means that the worker stopped, or that the event channel closed, while
    /// the heartbeat was in flight.
    ///
    /// The heartbeat fails when the network fails it, when the stream ends without a response, and
    /// when no response arrives within `option.hard_ttl()`, the heartbeat interval. The worker
    /// still polls the future that opens the stream, and then the stream, until the stream ends,
    /// and returns a reply that arrives late: dropping either would cancel the request in flight.
    /// An HTTP/2 transport resets the stream of a canceled request, and a server that finds more
    /// than 20 reset streams waiting for it to accept them closes the connection with `GOAWAY`.
    /// Each heartbeat event that arrives meanwhile counts as another failed heartbeat, and the
    /// latest one goes out once the stream has ended.
    ///
    /// Once the stream has yielded a response or an error, it has one more heartbeat interval to
    /// end, and is dropped then: its input holds a single request, so a transport ends it at once.
    ///
    /// Once `stopped` completes, the worker reports nothing more, but still polls the heartbeat
    /// for up to `election_timeout_max`, so that a target that has just resumed from a stall, as
    /// leaders often step down then, accepts the request before it is dropped.
    ///
    /// It logs each failure with `failed_in_row`, only the first in a row at warn level, and the
    /// first reply in time after failures.
    async fn send_heartbeat<S, Stop>(
        &mut self,
        input: S,
        option: RPCOption,
        mut stopped: Pin<&mut Stop>,
        failed_in_row: &mut u64,
        event_pending: &mut bool,
    ) -> ControlFlow<(), Option<AppendResult<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
        Stop: Future + OptionalSend,
    {
        // The response stream borrows the network, so the messages below name the fields that
        // `Display` would print.
        let (id, target) = (&self.id, &self.target);
        let interval = option.hard_ttl();
        let stop_limit = Duration::from_millis(self.config.election_timeout_max);
        let sent_at = C::now();
        let deadline = sent_at + interval;

        // Whether the deadline passed before the stream yielded anything. The heartbeat failed
        // then, and the worker watches the event channel while it waits for the stream to end.
        let mut late = false;

        let mut opening = self.network.stream_append(input, option);
        let mut output = loop {
            let until = (!late).then_some(deadline);
            let events = late.then_some(&mut self.rx);
            match Self::wait(&mut opening, until, events, stopped.as_mut()).await {
                Select4::First(Ok(output)) => break output,
                Select4::First(Err(error)) => {
                    if late {
                        tracing::debug!("id={} target={} heartbeat failed late: {}", id, target, error);
                    } else {
                        Self::count_failure(id, target, failed_in_row, HeartbeatFailure::Rpc(error));
                    }
                    return ControlFlow::Continue(None);
                }
                Select4::Second(()) => {
                    late = true;
                    Self::count_failure(id, target, failed_in_row, HeartbeatFailure::Timeout(interval));
                }
                Select4::Third(Ok(())) => Self::count_skipped(id, target, failed_in_row, event_pending),
                Select4::Third(Err(_)) | Select4::Fourth(_) => {
                    let heartbeat = async move {
                        if let Ok(output) = opening.await {
                            Self::drain(output).await;
                        }
                    };
                    Self::finish_after_stop(id, target, stop_limit, heartbeat).await;
                    return ControlFlow::Break(());
                }
            }
        };

        let mut reply: Option<AppendResult<C>> = None;
        // Whether the stream yielded an error before any response. The heartbeat failed then, and
        // a later response is ignored.
        let mut failed = false;
        // When the stream yielded its first response or error: it ends within an interval of it.
        let mut first_item_at: Option<InstantOf<C>> = None;
        loop {
            let until = match first_item_at {
                Some(at) => Some(at + interval),
                None => (!late).then_some(deadline),
            };
            let events = late.then_some(&mut self.rx);
            let result = match Self::wait(output.next(), until, events, stopped.as_mut()).await {
                Select4::First(Some(result)) => result,
                Select4::First(None) => break,
                Select4::Second(()) if first_item_at.is_some() => {
                    tracing::warn!("id={} target={} heartbeat responses did not end in time", id, target);
                    break;
                }
                Select4::Second(()) => {
                    late = true;
                    Self::count_failure(id, target, failed_in_row, HeartbeatFailure::Timeout(interval));
                    continue;
                }
                Select4::Third(Ok(())) => {
                    Self::count_skipped(id, target, failed_in_row, event_pending);
                    continue;
                }
                Select4::Third(Err(_)) | Select4::Fourth(_) => {
                    Self::finish_after_stop(id, target, stop_limit, Self::drain(output)).await;
                    return ControlFlow::Break(());
                }
            };
            first_item_at.get_or_insert_with(C::now);

            match result {
                Ok(_) if failed => {}
                Ok(result) => {
                    if reply.as_ref().is_none_or(|kept| importance(&result) > importance(kept)) {
                        reply = Some(result);
                    }
                }
                Err(error) if reply.is_some() => {
                    tracing::warn!(
                        "id={} target={} heartbeat response failed after a reply: {}",
                        id,
                        target,
                        error
                    );
                }
                Err(error) if failed || late => {
                    failed = true;
                    tracing::debug!("id={} target={} heartbeat failed late: {}", id, target, error);
                }
                Err(error) => {
                    failed = true;
                    Self::count_failure(id, target, failed_in_row, HeartbeatFailure::Rpc(error));
                }
            }
        }

        match &reply {
            None if !late && !failed => {
                Self::count_failure(id, target, failed_in_row, HeartbeatFailure::NoResponse);
            }
            None => {}
            Some(_) if late => {
                let elapsed = C::now() - sent_at;
                tracing::debug!(
                    "id={} target={} a late reply arrived, {:?} after the heartbeat was sent",
                    id,
                    target,
                    elapsed
                );
            }
            Some(_) => Self::count_reply(id, target, failed_in_row),
        }
        ControlFlow::Continue(reply)
    }

    /// Waits for `future`, for the instant `until` if it is `Some`, for a change of `events` if it
    /// is `Some`, and for `stopped`, whichever comes first.
    async fn wait<F, Stop>(
        future: F,
        until: Option<InstantOf<C>>,
        events: Option<&mut WatchReceiverOf<C, Option<HeartbeatEvent<C>>>>,
        stopped: Pin<&mut Stop>,
    ) -> Select4<F::Output, (), Result<(), RecvError>, Stop::Output>
    where
        F: Future + OptionalSend,
        Stop: Future + OptionalSend,
    {
        let deadline = match until {
            Some(until) => Either::Left(C::sleep_until(until)),
            None => Either::Right(std::future::pending()),
        };
        let changed = match events {
            Some(events) => Either::Left(events.changed()),
            None => Either::Right(std::future::pending()),
        };
        C::select4(future, deadline, changed, stopped).await
    }

    /// Reads `output` until it ends.
    async fn drain<S>(mut output: S)
    where S: Stream + Unpin {
        while output.next().await.is_some() {}
    }

    /// Polls `heartbeat`, the heartbeat in flight when the worker stopped, until it ends or `limit`
    /// has passed, and drops it then.
    async fn finish_after_stop<F>(id: &C::NodeId, target: &C::NodeId, limit: Duration, heartbeat: F)
    where F: Future<Output = ()> + OptionalSend {
        if C::timeout(limit, heartbeat).await.is_err() {
            tracing::debug!(
                "id={} target={} dropped the heartbeat in flight {:?} after the heartbeats stopped",
                id,
                target,
                limit
            );
        }
    }

    /// Counts a failed heartbeat to `target`, and logs it: the first failure in a row at warn
    /// level, and the others at debug level.
    fn count_failure(id: &C::NodeId, target: &C::NodeId, failed_in_row: &mut u64, failure: HeartbeatFailure<C>) {
        *failed_in_row += 1;
        if *failed_in_row == 1 {
            tracing::warn!(
                "id={} target={} heartbeat failed; until the target replies in time, failures log at debug level: {}",
                id,
                target,
                failure
            );
        } else {
            tracing::debug!(
                "id={} target={} heartbeat failed again, {} in a row: {}",
                id,
                target,
                failed_in_row,
                failure
            );
        }
    }

    /// Counts a heartbeat event that arrived while the last heartbeat was still in flight as a
    /// failed heartbeat. The latest event goes out once the last heartbeat has ended.
    fn count_skipped(id: &C::NodeId, target: &C::NodeId, failed_in_row: &mut u64, event_pending: &mut bool) {
        *event_pending = true;
        *failed_in_row += 1;
        tracing::debug!(
            "id={} target={} heartbeat failed again, {} in a row: the last heartbeat is still in flight",
            id,
            target,
            failed_in_row
        );
    }

    /// Ends the failures in a row at a reply in time, and logs how many there were: at debug
    /// level after a single failure, so that an isolated failure, which a loaded node sees often,
    /// logs its warning and nothing else above debug level.
    fn count_reply(id: &C::NodeId, target: &C::NodeId, failed_in_row: &mut u64) {
        match *failed_in_row {
            0 => {}
            1 => tracing::debug!(
                "id={} target={} the target replied after 1 failed heartbeat",
                id,
                target
            ),
            n => tracing::info!(
                "id={} target={} the target replied after {} failed heartbeats",
                id,
                target,
                n
            ),
        }
        *failed_in_row = 0;
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
