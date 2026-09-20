//! A watch channel whose wake order is deterministic.
//!
//! `tokio::sync::watch` spreads waiters over internal `Notify` slots picked by a random number, so
//! two tasks waiting on one channel wake in an order that can differ between runs. This channel
//! keeps a single FIFO waiter list: a change wakes waiters in the order they started waiting.

use std::collections::VecDeque;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::RwLock;
use std::sync::RwLockReadGuard;
use std::sync::RwLockWriteGuard;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use openraft_rt::OptionalSend;
use openraft_rt::OptionalSync;
use openraft_rt::watch;
use openraft_rt::watch::RecvError;
use openraft_rt::watch::SendError;

/// Watch channel.
pub struct SimWatch;

struct Versioned<T> {
    value: T,
    version: u64,
}

struct Waiters {
    next_id: u64,
    queue: VecDeque<(u64, Waker)>,
    senders: usize,
    receivers: usize,
}

struct Channel<T> {
    value: RwLock<Versioned<T>>,
    waiters: Mutex<Waiters>,
}

impl<T> Channel<T> {
    fn read(&self) -> RwLockReadGuard<'_, Versioned<T>> {
        self.value.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Versioned<T>> {
        self.value.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn waiters(&self) -> MutexGuard<'_, Waiters> {
        self.waiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Wakes every waiter, oldest first.
    fn notify(&self) {
        let queue = std::mem::take(&mut self.waiters().queue);
        for (_, waker) in queue {
            waker.wake();
        }
    }
}

/// Sends values to the receivers.
pub struct SimWatchSender<T> {
    chan: Arc<Channel<T>>,
}

/// Receives the latest value.
pub struct SimWatchReceiver<T> {
    chan: Arc<Channel<T>>,
    seen: u64,
}

/// A read guard on the current value.
pub struct SimWatchRef<'a, T> {
    guard: RwLockReadGuard<'a, Versioned<T>>,
}

impl<T> Deref for SimWatchRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard.value
    }
}

impl watch::Watch for SimWatch {
    type Sender<T: OptionalSend + OptionalSync> = SimWatchSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = SimWatchReceiver<T>;
    type Ref<'a, T: OptionalSend + 'a> = SimWatchRef<'a, T>;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let chan = Arc::new(Channel {
            value: RwLock::new(Versioned {
                value: init,
                version: 0,
            }),
            waiters: Mutex::new(Waiters {
                next_id: 0,
                queue: VecDeque::new(),
                senders: 1,
                receivers: 1,
            }),
        });
        (SimWatchSender { chan: chan.clone() }, SimWatchReceiver {
            chan,
            seen: 0,
        })
    }
}

impl<T> Clone for SimWatchSender<T> {
    fn clone(&self) -> Self {
        self.chan.waiters().senders += 1;
        SimWatchSender {
            chan: self.chan.clone(),
        }
    }
}

impl<T> Drop for SimWatchSender<T> {
    fn drop(&mut self) {
        let closed = {
            let mut waiters = self.chan.waiters();
            waiters.senders -= 1;
            waiters.senders == 0
        };
        if closed {
            self.chan.notify();
        }
    }
}

impl<T> watch::WatchSender<SimWatch, T> for SimWatchSender<T>
where T: OptionalSend + OptionalSync
{
    fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.chan.waiters().receivers == 0 {
            return Err(SendError(value));
        }
        {
            let mut current = self.chan.write();
            current.value = value;
            current.version += 1;
        }
        self.chan.notify();
        Ok(())
    }

    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        let modified = {
            let mut current = self.chan.write();
            let modified = modify(&mut current.value);
            if modified {
                current.version += 1;
            }
            modified
        };
        if modified {
            self.chan.notify();
        }
        modified
    }

    fn borrow_watched(&self) -> SimWatchRef<'_, T> {
        SimWatchRef {
            guard: self.chan.read(),
        }
    }

    fn subscribe(&self) -> SimWatchReceiver<T> {
        self.chan.waiters().receivers += 1;
        let seen = self.chan.read().version;
        SimWatchReceiver {
            chan: self.chan.clone(),
            seen,
        }
    }
}

impl<T> Clone for SimWatchReceiver<T> {
    fn clone(&self) -> Self {
        self.chan.waiters().receivers += 1;
        SimWatchReceiver {
            chan: self.chan.clone(),
            seen: self.seen,
        }
    }
}

impl<T> Drop for SimWatchReceiver<T> {
    fn drop(&mut self) {
        self.chan.waiters().receivers -= 1;
    }
}

impl<T> watch::WatchReceiver<SimWatch, T> for SimWatchReceiver<T>
where T: OptionalSend + OptionalSync
{
    async fn changed(&mut self) -> Result<(), RecvError> {
        Changed {
            receiver: self,
            waiter: None,
        }
        .await
    }

    fn borrow_watched(&self) -> SimWatchRef<'_, T> {
        SimWatchRef {
            guard: self.chan.read(),
        }
    }

    fn borrow_and_update(&mut self) -> SimWatchRef<'_, T> {
        let guard = self.chan.read();
        self.seen = guard.version;
        SimWatchRef { guard }
    }
}

/// Waits for a version the receiver has not seen, or for every sender to be dropped.
struct Changed<'a, T> {
    receiver: &'a mut SimWatchReceiver<T>,
    waiter: Option<u64>,
}

impl<T> Changed<'_, T> {
    fn deregister(&mut self) {
        if let Some(id) = self.waiter.take() {
            self.receiver.chan.waiters().queue.retain(|(waiting, _)| *waiting != id);
        }
    }
}

impl<T> Future for Changed<'_, T> {
    type Output = Result<(), RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let version = self.receiver.chan.read().version;
        if version != self.receiver.seen {
            self.receiver.seen = version;
            self.deregister();
            return Poll::Ready(Ok(()));
        }

        let this = &mut *self;
        let mut waiters = this.receiver.chan.waiters();
        if waiters.senders == 0 {
            drop(waiters);
            this.deregister();
            return Poll::Ready(Err(RecvError(())));
        }

        let registered = match this.waiter {
            Some(id) => match waiters.queue.iter_mut().find(|(waiting, _)| *waiting == id) {
                Some((_, waker)) => {
                    if !waker.will_wake(cx.waker()) {
                        *waker = cx.waker().clone();
                    }
                    true
                }
                None => false,
            },
            None => false,
        };
        if !registered {
            let id = match this.waiter {
                Some(id) => id,
                None => {
                    let id = waiters.next_id;
                    waiters.next_id += 1;
                    id
                }
            };
            waiters.queue.push_back((id, cx.waker().clone()));
            this.waiter = Some(id);
        }
        Poll::Pending
    }
}

impl<T> Drop for Changed<'_, T> {
    fn drop(&mut self) {
        self.deregister();
    }
}
