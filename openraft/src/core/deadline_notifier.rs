use std::future::Future;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::Select2;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CancelHandleOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Wakes `RaftCore` with one notification when a deadline that `RaftCore` sets is reached.
///
/// `RaftCore` uses one instance for the earliest pending linearizable read, and one for the instant
/// at which a follower's election may be due.
///
/// After firing, the worker waits on `changed()` instead of re-arming on the deadline it just
/// reported. For pending reads this cannot lose a wake-up: the `drain_expired` that the
/// notification triggers always moves `earliest_deadline()` to a different value, because every
/// deadline left in the queue exceeds `now` and an emptied queue reports `None`. `set_deadline`
/// therefore always observes a change and re-arms the worker. Watch versioning covers the
/// interleaving where the new deadline is set before the worker starts awaiting.
///
/// An optional `enabled` watch parks the worker while it holds `false`. Enabling it again fires at
/// once for a deadline already reached, even one that has fired before.
pub(crate) struct DeadlineNotifier<C>
where C: RaftTypeConfig
{
    deadline_tx: WatchSenderOf<C, Option<InstantOf<C>>>,
    _join_handle: JoinHandleOf<C, ()>,
    _cancel: CancelHandleOf<C>,
}

impl<C> DeadlineNotifier<C>
where C: RaftTypeConfig
{
    /// Spawn a worker that sends `notification()` when the deadline set by
    /// [`Self::set_deadline`] is reached, while `enabled`, if given, holds `true`.
    pub(crate) fn spawn(
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn() -> Notification<C>,
        enabled: Option<WatchReceiverOf<C, bool>>,
    ) -> Self {
        let (deadline_tx, deadline_rx) = C::watch_channel(None);
        let (cancel, future) = Self::run(deadline_rx, enabled, notification_tx, notification);
        let join_handle = C::spawn(future);

        Self {
            deadline_tx,
            _join_handle: join_handle,
            _cancel: cancel,
        }
    }

    /// Set the deadline to fire at; `None` sends nothing. Return whether the deadline changed.
    pub(crate) fn set_deadline(&self, deadline: Option<InstantOf<C>>) -> bool {
        self.deadline_tx.send_if_different(deadline)
    }

    fn run(
        deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        enabled: Option<WatchReceiverOf<C, bool>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn() -> Notification<C>,
    ) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        let future = Self::do_run(deadline_rx, enabled, notification_tx, notification);
        let (cancel, future) = C::cancellable(future);
        let future = async move {
            let _ = future.await;
        };
        (cancel, future)
    }

    async fn do_run(
        mut deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        mut enabled: Option<WatchReceiverOf<C, bool>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn() -> Notification<C>,
    ) {
        loop {
            if let Some(enabled) = enabled.as_mut() {
                let is_enabled = *enabled.borrow_and_update();
                if !is_enabled {
                    if enabled.wait_until(|enabled| *enabled).await.is_err() {
                        return;
                    }
                    continue;
                }
            }

            let deadline = *deadline_rx.borrow_and_update();
            let Some(deadline) = deadline else {
                if !Self::changed(&mut deadline_rx, &mut enabled).await {
                    return;
                }
                continue;
            };

            let wait_result = C::timeout_at(deadline, Self::changed(&mut deadline_rx, &mut enabled)).await;
            match wait_result {
                Ok(true) => continue,
                Ok(false) => return,
                Err(_) => {}
            }

            let send_result = notification_tx.send(notification()).await;
            if send_result.is_err() {
                return;
            }

            if !Self::changed(&mut deadline_rx, &mut enabled).await {
                return;
            }
        }
    }

    /// Wait until the deadline or the `enabled` watch changes; return `false` once either sender
    /// is dropped.
    async fn changed(
        deadline_rx: &mut WatchReceiverOf<C, Option<InstantOf<C>>>,
        enabled: &mut Option<WatchReceiverOf<C, bool>>,
    ) -> bool {
        let Some(enabled) = enabled.as_mut() else {
            return deadline_rx.changed().await.is_ok();
        };

        let selected = C::select2(deadline_rx.changed(), enabled.changed()).await;
        match selected {
            Select2::First(res) => res.is_ok(),
            Select2::Second(res) => res.is_ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::FutureExt;

    use super::*;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::mpsc::TryRecvError;
    use crate::engine::testing::UTConfig;

    type C = UTConfig;

    #[test]
    fn deadline_notifier_uses_latest_deadline_and_rearms() {
        C::run(async {
            let (tx, mut rx) = C::mpsc(4);
            let notifier = DeadlineNotifier::<C>::spawn(tx, || Notification::PendingReadDeadlineReached, None);

            let late = C::now() + Duration::from_secs(1);
            notifier.set_deadline(Some(late));

            let early = C::now() + Duration::from_millis(10);
            notifier.set_deadline(Some(early));

            let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
            let notification = received.unwrap().unwrap();
            assert!(matches!(notification, Notification::PendingReadDeadlineReached));

            let next = C::now() + Duration::from_millis(10);
            notifier.set_deadline(Some(next));

            let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
            let notification = received.unwrap().unwrap();
            assert!(matches!(notification, Notification::PendingReadDeadlineReached));
        });
    }

    /// A disabled notifier sends nothing; enabling it again fires at once for a deadline already
    /// reached, even one that has fired before.
    #[test]
    fn deadline_notifier_parks_while_disabled() {
        const WAIT: Duration = Duration::from_millis(300);
        const MARGIN: Duration = Duration::from_millis(10);

        // A paused clock advances only when every task waits, so the windows below are exact.
        // `LocalSet` lets `spawn()` work in the `single-threaded` build.
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().unwrap();
        let local = tokio::task::LocalSet::new();

        let scenario = async {
            let (tx, mut rx) = C::mpsc(16);
            let (enable_tx, enabled) = C::watch_channel(false);
            let notifier = DeadlineNotifier::<C>::spawn(tx, || Notification::PendingReadDeadlineReached, Some(enabled));
            notifier.set_deadline(Some(C::now() + WAIT));

            tracing::info!("disabled: nothing at the deadline");
            {
                let early = C::timeout(WAIT * 2, rx.recv()).await;
                assert!(early.is_err(), "a disabled notifier must not fire");
            }

            tracing::info!("enabled again: one notification at once for the passed deadline");
            {
                enable_tx.send(true).unwrap();
                let first =
                    C::timeout(MARGIN, rx.recv()).await.expect("enabling must fire at once for a passed deadline");
                assert!(matches!(first, Some(Notification::PendingReadDeadlineReached)));

                let again = C::timeout(WAIT, rx.recv()).await;
                assert!(again.is_err(), "a deadline left in place fires only once");
            }

            tracing::info!("disabled and enabled again: the fired deadline fires once more");
            {
                enable_tx.send(false).unwrap();
                C::sleep(MARGIN).await;
                enable_tx.send(true).unwrap();

                let next = C::timeout(MARGIN, rx.recv()).await.expect("enabling again must re-check a passed deadline");
                assert!(matches!(next, Some(Notification::PendingReadDeadlineReached)));
            }
        };
        local.block_on(&rt, scenario);
    }

    #[test]
    fn deadline_notifier_cancels_a_blocked_send() {
        C::run(async {
            const TIMEOUT: Duration = Duration::from_secs(1);

            tracing::info!("fill the notification channel before the deadline expires");
            let (tx, mut rx) = C::mpsc(1);
            tx.send(Notification::Tick { i: 1 }).await.unwrap();
            let deadline = C::now() - TIMEOUT;
            let (deadline_tx, deadline_rx) = C::watch_channel(Some(deadline));
            let (cancel, future) =
                DeadlineNotifier::<C>::run(deadline_rx, None, tx, || Notification::PendingReadDeadlineReached);
            let mut future = std::pin::pin!(future);
            let pending = future.as_mut().now_or_never();
            assert_eq!(None, pending);

            tracing::info!("cancel the blocked send while keeping notification capacity occupied");
            {
                drop(deadline_tx);
                let pending = future.as_mut().now_or_never();
                assert_eq!(None, pending, "closing the deadline watch cannot interrupt a send");
                drop(cancel);
                C::timeout(TIMEOUT, future).await.unwrap();

                let first = rx.try_recv().unwrap();
                let sentinel = matches!(first, Notification::Tick { i: 1 });
                assert!(sentinel, "the blocked deadline notification must be discarded");
                let next = rx.try_recv();
                let closed = matches!(next, Err(TryRecvError::Disconnected));
                assert!(closed, "cancellation must release the notification sender");
            }
        });
    }
}
