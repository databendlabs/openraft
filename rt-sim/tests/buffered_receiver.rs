//! `BufferedReceiver` scheduling that only a single-threaded executor makes observable.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::Mpsc;
use openraft_rt::MpscReceiver;
use openraft_rt::MpscSender;
use openraft_rt::Oneshot;
use openraft_rt::OneshotSender;
use openraft_rt::OptionalSend;
use openraft_rt::SendError;
use openraft_rt::TryRecvError;
use openraft_rt_sim::SimRuntime;

/// More values than the buffering task handles in one poll.
const PREFILLED: u64 = 1024;

const TIMEOUT: Duration = Duration::from_secs(5);

/// A source that counts the values taken from it, and reports when it is dropped.
struct CountingReceiver<R, S>
where S: OneshotSender<()>
{
    inner: Option<R>,
    taken: Arc<AtomicU64>,
    dropped: Option<S>,
}

impl<R, S, T> MpscReceiver<T> for CountingReceiver<R, S>
where
    R: MpscReceiver<T>,
    S: OneshotSender<()>,
    T: OptionalSend,
{
    async fn recv(&mut self) -> Option<T> {
        let inner = self.inner.as_mut().unwrap();
        let value = inner.recv().await;
        if value.is_some() {
            self.taken.fetch_add(1, Ordering::Relaxed);
        }
        value
    }

    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let inner = self.inner.as_mut().unwrap();
        let value = inner.try_recv();
        if value.is_ok() {
            self.taken.fetch_add(1, Ordering::Relaxed);
        }
        value
    }
}

impl<R, S> Drop for CountingReceiver<R, S>
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

#[test]
fn test_buffered_receiver_lets_the_consumer_run_during_read_ahead() {
    SimRuntime::new(1).block_on(async {
        let (tx, rx) = <SimRuntime as AsyncRuntime>::Mpsc::channel::<u64>(PREFILLED as usize);
        for value in 0..PREFILLED {
            tx.send(value).await.unwrap();
        }
        drop(tx);

        let taken = Arc::new(AtomicU64::new(0));
        let (dropped, _observed) = <SimRuntime as AsyncRuntime>::Oneshot::channel();
        let source = CountingReceiver {
            inner: Some(rx),
            taken: Arc::clone(&taken),
            dropped: Some(dropped),
        };
        let mut rx = source.with_unbounded_buffer::<SimRuntime>();

        let first = rx.recv().await;
        assert_eq!(Some(0), first);
        let taken_before_first = taken.load(Ordering::Relaxed);
        assert!(
            taken_before_first < PREFILLED,
            "the task took all {PREFILLED} values before the waiting consumer ran"
        );

        let mut received = vec![0];
        while let Some(value) = rx.recv().await {
            received.push(value);
        }
        assert_eq!((0..PREFILLED).collect::<Vec<_>>(), received);
    });
}

#[test]
fn test_buffered_receiver_drop_during_read_ahead_releases_the_source() {
    SimRuntime::new(1).block_on(async {
        let (tx, rx) = <SimRuntime as AsyncRuntime>::Mpsc::channel::<u64>(PREFILLED as usize);
        for value in 0..PREFILLED {
            tx.send(value).await.unwrap();
        }

        let taken = Arc::new(AtomicU64::new(0));
        let (dropped, observed) = <SimRuntime as AsyncRuntime>::Oneshot::channel();
        let source = CountingReceiver {
            inner: Some(rx),
            taken: Arc::clone(&taken),
            dropped: Some(dropped),
        };
        let mut rx = source.with_unbounded_buffer::<SimRuntime>();

        let first = rx.recv().await;
        assert_eq!(Some(0), first);
        assert!(
            taken.load(Ordering::Relaxed) < PREFILLED,
            "the source should still hold values"
        );
        drop(rx);

        let released = SimRuntime::timeout(TIMEOUT, observed).await;
        released.unwrap().unwrap();
        let sent = tx.send(PREFILLED).await;
        assert_eq!(Err(SendError(PREFILLED)), sent);
    });
}
