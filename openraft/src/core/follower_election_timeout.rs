//! A one-shot task that wakes `RaftCore` when a follower's election may be due.

use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CancelHandleOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;

/// A spawned task that sends [`Notification::FollowerElectionTimeout`] once, at `wake_at`.
///
/// `RaftCore` owns at most one and decides when to arm, keep, replace, or cancel it; the task holds
/// only its wake time and a notification sender. Dropping it cancels the task without waiting;
/// [`Self::shutdown`] cancels it and waits for the task to stop.
pub(crate) struct FollowerElectionTimeout<C>
where C: RaftTypeConfig
{
    wake_at: InstantOf<C>,
    cancel: CancelHandleOf<C>,
    join_handle: JoinHandleOf<C, ()>,
}

impl<C> FollowerElectionTimeout<C>
where C: RaftTypeConfig
{
    pub(crate) fn spawn(wake_at: InstantOf<C>, tx: MpscSenderOf<C, Notification<C>>) -> Self {
        let span = tracing::span!(parent: &Span::current(), Level::DEBUG, "follower_election_timeout");
        let future = Self::run(wake_at, tx).instrument(span);

        let (cancel, future) = C::cancellable(future);
        let join_handle = C::spawn(async move {
            if future.await.is_none() {
                tracing::debug!("FollowerElectionTimeout cancelled");
            }
        });

        Self {
            wake_at,
            cancel,
            join_handle,
        }
    }

    /// The instant at which this timeout sends its notification.
    pub(crate) fn wake_at(&self) -> InstantOf<C> {
        self.wake_at
    }

    /// Cancel the task and wait for it to stop.
    pub(crate) async fn shutdown(self) {
        let Self {
            mut cancel,
            join_handle,
            ..
        } = self;

        cancel.cancel();
        if let Err(e) = join_handle.await {
            tracing::error!("FollowerElectionTimeout task failed: {}", e);
        }
    }

    async fn run(wake_at: InstantOf<C>, tx: MpscSenderOf<C, Notification<C>>) {
        C::sleep_until(wake_at).await;

        let send_res = tx.send(Notification::FollowerElectionTimeout).await;
        if send_res.is_err() {
            tracing::info!("FollowerElectionTimeout: notification channel closed, quit");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FollowerElectionTimeout;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::MpscSender;
    use crate::async_runtime::mpsc::TryRecvError;
    use crate::core::notification::Notification;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;

    type C = UTConfig;

    const WAIT: Duration = Duration::from_millis(50);
    const TIMEOUT: Duration = Duration::from_secs(1);

    #[test]
    fn test_sends_one_notification_at_wake_at() {
        C::run(async {
            let (tx, mut rx) = C::mpsc(4);
            let wake_at = C::now() + WAIT;
            let timeout = FollowerElectionTimeout::<C>::spawn(wake_at, tx);
            assert_eq!(wake_at, timeout.wake_at());

            let notification = C::timeout(TIMEOUT, rx.recv()).await.unwrap().unwrap();
            assert!(matches!(notification, Notification::FollowerElectionTimeout));
            assert!(C::now() >= wake_at, "the notification must not precede wake_at");

            let next = C::timeout(TIMEOUT, rx.recv()).await.unwrap();
            assert!(
                next.is_none(),
                "a one-shot task releases its sender after one notification"
            );
        });
    }

    #[test]
    fn test_drop_cancels_before_wake_at() {
        C::run(async {
            let (tx, mut rx) = C::mpsc(4);
            let timeout = FollowerElectionTimeout::<C>::spawn(C::now() + WAIT, tx);

            drop(timeout);

            let next = C::timeout(TIMEOUT, rx.recv()).await.unwrap();
            assert!(next.is_none(), "a cancelled task releases its sender without sending");
        });
    }

    #[test]
    fn test_shutdown_interrupts_a_blocked_send() {
        C::run(async {
            tracing::info!("fill the notification channel so the due timeout blocks in send()");
            let (tx, mut rx) = C::mpsc(1);
            tx.send(Notification::Tick { i: 1 }).await.unwrap();
            let timeout = FollowerElectionTimeout::<C>::spawn(C::now(), tx);
            C::sleep(WAIT).await;

            tracing::info!("shutdown must cancel the blocked send and wait for the task");
            {
                C::timeout(TIMEOUT, timeout.shutdown()).await.unwrap();

                let first = rx.try_recv().unwrap();
                assert!(matches!(first, Notification::Tick { i: 1 }));
                let next = rx.try_recv();
                let closed = matches!(next, Err(TryRecvError::Disconnected));
                assert!(closed, "the interrupted send must not enqueue a notification");
            }
        });
    }
}
