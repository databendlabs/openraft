use std::future::Future;
use std::time::Duration;

use super::Suite;
use crate::AsyncRuntime;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::MpscSender;
use crate::Oneshot;
use crate::OneshotSender;
use crate::OptionalSend;
use crate::SendError;
use crate::TryRecvError;

const BACKLOG: u64 = 64;
const TIMEOUT: Duration = Duration::from_secs(5);

/// Observes actual source-receiver teardown, including cancellation while the task is idle.
struct DropReceiver<R, S>
where S: OneshotSender<()>
{
    inner: Option<R>,
    dropped: Option<S>,
}

impl<R, S, T> MpscReceiver<T> for DropReceiver<R, S>
where
    R: MpscReceiver<T>,
    S: OneshotSender<()>,
    T: OptionalSend,
{
    async fn recv(&mut self) -> Option<T> {
        let inner = self.inner.as_mut().unwrap();
        inner.recv().await
    }

    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let inner = self.inner.as_mut().unwrap();
        inner.try_recv()
    }
}

impl<R, S> Drop for DropReceiver<R, S>
where S: OneshotSender<()>
{
    fn drop(&mut self) {
        let inner = self.inner.take();
        drop(inner);
        let Some(dropped) = self.dropped.take() else {
            return;
        };
        let _ = dropped.send(());
    }
}

/// A source whose third receive takes its value from `inner`, signals `taken`, and returns the
/// value only after `release` completes: dropping that receive loses the value.
struct SlowThirdReceiver<R, S, Rx>
where S: OneshotSender<()>
{
    inner: R,
    received: usize,
    taken: Option<S>,
    release: Option<Rx>,
}

impl<R, S, Rx, T> MpscReceiver<T> for SlowThirdReceiver<R, S, Rx>
where
    R: MpscReceiver<T>,
    S: OneshotSender<()>,
    Rx: Future + OptionalSend,
    T: OptionalSend,
{
    async fn recv(&mut self) -> Option<T> {
        let value = self.inner.recv().await?;
        self.received += 1;
        if self.received == 3 {
            if let Some(taken) = self.taken.take() {
                let _ = taken.send(());
            }
            if let Some(release) = self.release.take() {
                release.await;
            }
        }
        Some(value)
    }

    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }
}

impl<Rt> Suite<Rt>
where Rt: AsyncRuntime
{
    pub(super) async fn test_buffered_receiver_backlog() {
        let (tx, rx) = Rt::Mpsc::channel(1);
        let mut rx = rx.with_unbounded_buffer::<Rt>();
        for value in 0..BACKLOG {
            let sent = Rt::timeout(TIMEOUT, tx.send(value)).await;
            sent.unwrap().unwrap();
        }
        drop(tx);
        let drain = async {
            let mut received = Vec::new();
            while let Some(value) = rx.recv().await {
                received.push(value);
            }
            received
        };
        let received = Rt::timeout(TIMEOUT, drain).await.unwrap();
        let expected: Vec<_> = (0..BACKLOG).collect();
        assert_eq!(expected, received);
        let closed = rx.try_recv();
        assert_eq!(Err(TryRecvError::Disconnected), closed);
    }

    pub(super) async fn test_buffered_receiver_empty_close() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(1);
        let mut rx = rx.with_unbounded_buffer::<Rt>();
        let empty = rx.try_recv();
        assert_eq!(Err(TryRecvError::Empty), empty);
        drop(tx);
        let received = Rt::timeout(TIMEOUT, rx.recv()).await.unwrap();
        assert_eq!(None, received);
    }

    pub(super) async fn test_buffered_receiver_drop_idle() {
        let (tx, rx) = Rt::Mpsc::channel(1);
        let (dropped, observed) = Rt::Oneshot::channel();
        let rx = DropReceiver {
            inner: Some(rx),
            dropped: Some(dropped),
        };
        let rx = rx.with_unbounded_buffer::<Rt>();
        drop(rx);
        let stopped = Rt::timeout(TIMEOUT, observed).await;
        stopped.unwrap().unwrap();
        let sent = tx.send(42).await;
        assert_eq!(Err(SendError(42)), sent);
    }

    pub(super) async fn test_buffered_receiver_drop_backlog() {
        let (tx, rx) = Rt::Mpsc::channel(1);
        let (dropped, observed) = Rt::Oneshot::channel();
        let rx = DropReceiver {
            inner: Some(rx),
            dropped: Some(dropped),
        };
        let rx = rx.with_unbounded_buffer::<Rt>();
        for value in 0..BACKLOG {
            let sent = Rt::timeout(TIMEOUT, tx.send(value)).await;
            sent.unwrap().unwrap();
        }
        drop(rx);
        let stopped = Rt::timeout(TIMEOUT, observed).await;
        stopped.unwrap().unwrap();
        let sent = tx.send(BACKLOG).await;
        assert_eq!(Err(SendError(BACKLOG)), sent);
    }

    pub(super) async fn test_buffered_receiver_cancel_receive() {
        let (tx, rx) = Rt::Mpsc::channel(1);
        let mut rx = rx.with_unbounded_buffer::<Rt>();
        let pending = super::is_pending(rx.recv());
        assert!(pending, "a receive on an empty live channel should be pending");
        let sent = tx.send(42).await;
        sent.unwrap();
        drop(tx);
        let received = Rt::timeout(TIMEOUT, rx.recv()).await.unwrap();
        assert_eq!(Some(42), received);
        let closed = Rt::timeout(TIMEOUT, rx.recv()).await.unwrap();
        assert_eq!(None, closed);
    }

    /// A receive that is pending when the output takes the previous send keeps its value.
    pub(super) async fn test_buffered_receiver_keeps_a_pending_receive() {
        let (tx, rx) = Rt::Mpsc::channel(4);
        for value in 1..=4_u64 {
            tx.send(value).await.unwrap();
        }
        drop(tx);

        let (taken, taken_rx) = Rt::Oneshot::channel();
        let (release, release_rx) = Rt::Oneshot::channel::<()>();
        let source = SlowThirdReceiver {
            inner: rx,
            received: 0,
            taken: Some(taken),
            release: Some(release_rx),
        };
        let mut rx = source.with_unbounded_buffer::<Rt>();

        // The output holds 1, the send of 2 waits for room, and the receive holding 3 waits.
        Rt::timeout(TIMEOUT, taken_rx).await.unwrap().unwrap();
        let first = Rt::timeout(TIMEOUT, rx.recv()).await.unwrap();
        assert_eq!(Some(1), first);
        let _ = release.send(());

        let drain = async {
            let mut received = Vec::new();
            while let Some(value) = rx.recv().await {
                received.push(value);
            }
            received
        };
        let rest = Rt::timeout(TIMEOUT, drain).await.unwrap();
        assert_eq!(vec![2, 3, 4], rest);
    }

    #[cfg(feature = "single-threaded")]
    pub(super) async fn test_buffered_receiver_non_send() {
        let (tx, rx) = Rt::Mpsc::channel(1);
        let mut rx = rx.with_unbounded_buffer::<Rt>();
        let expected = std::rc::Rc::new(std::cell::Cell::new(42));
        let value = expected.clone();
        let sent = tx.send(value).await;
        sent.unwrap();
        let received = rx.recv().await;
        assert_eq!(Some(expected), received);
    }
}
