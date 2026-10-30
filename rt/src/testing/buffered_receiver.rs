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
        let mut received = Vec::new();
        while let Some(value) = rx.recv().await {
            received.push(value);
        }
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
        let selected = Rt::select2(async {}, rx.recv()).await;
        assert_eq!(crate::Select2::First(()), selected);
        let sent = tx.send(42).await;
        sent.unwrap();
        let received = Rt::timeout(TIMEOUT, rx.recv()).await.unwrap();
        assert_eq!(Some(42), received);
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
