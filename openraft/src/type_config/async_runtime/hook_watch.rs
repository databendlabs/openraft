//! A watch channel for tests, whose receiver can run a hook right after a change notification.

use std::sync::Arc;
use std::sync::Mutex;

use tokio::sync::watch as tokio_watch;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::SendError;
use crate::async_runtime::watch::Watch;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;

/// A Tokio watch channel whose receiver runs a hook between a change notification and the read
/// that follows it.
///
/// A test sends a value from the hook to place that send deterministically between `changed()`
/// and the read of the value, which a real runtime does only by chance.
pub(crate) struct HookWatch;

type Hook = Box<dyn FnOnce() + Send>;

pub(crate) struct HookWatchSender<T>(tokio_watch::Sender<T>);

impl<T> Clone for HookWatchSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) struct HookWatchReceiver<T> {
    inner: tokio_watch::Receiver<T>,

    /// Runs once, right after the next `changed()` of this receiver returns `Ok`.
    after_changed: Arc<Mutex<Option<Hook>>>,
}

impl<T> Clone for HookWatchReceiver<T> {
    /// The clone has no hook of its own.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            after_changed: Default::default(),
        }
    }
}

impl<T> HookWatchReceiver<T> {
    /// Runs `hook` once, right after the next `changed()` of this receiver returns `Ok`.
    pub(crate) fn after_changed(&self, hook: impl FnOnce() + Send + 'static) {
        *self.after_changed.lock().unwrap() = Some(Box::new(hook));
    }
}

impl Watch for HookWatch {
    type Sender<T: OptionalSend + OptionalSync> = HookWatchSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = HookWatchReceiver<T>;

    type Ref<'a, T: OptionalSend + 'a> = tokio_watch::Ref<'a, T>;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_watch::channel(init);
        let rx = HookWatchReceiver {
            inner: rx,
            after_changed: Default::default(),
        };
        (HookWatchSender(tx), rx)
    }
}

impl<T> WatchSender<HookWatch, T> for HookWatchSender<T>
where T: OptionalSend + OptionalSync
{
    fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.send(value).map_err(|e| SendError(e.0))
    }

    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.0.send_if_modified(modify)
    }

    fn borrow_watched(&self) -> <HookWatch as Watch>::Ref<'_, T> {
        self.0.borrow()
    }

    fn subscribe(&self) -> <HookWatch as Watch>::Receiver<T> {
        HookWatchReceiver {
            inner: self.0.subscribe(),
            after_changed: Default::default(),
        }
    }
}

impl<T> WatchReceiver<HookWatch, T> for HookWatchReceiver<T>
where T: OptionalSend + OptionalSync
{
    async fn changed(&mut self) -> Result<(), RecvError> {
        self.inner.changed().await.map_err(|_| RecvError(()))?;
        let hook = self.after_changed.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }

    fn borrow_watched(&self) -> <HookWatch as Watch>::Ref<'_, T> {
        self.inner.borrow()
    }

    fn borrow_and_update(&mut self) -> <HookWatch as Watch>::Ref<'_, T> {
        self.inner.borrow_and_update()
    }
}
