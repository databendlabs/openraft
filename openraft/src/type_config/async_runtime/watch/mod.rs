//! Watch channel traits.

mod watch_error;

use openraft_macros::add_async_trait;
pub use watch_error::RecvError;
pub use watch_error::SendError;

use crate::OptionalSend;
use crate::OptionalSync;

/// A `Watch` is a pair of `WatchSender` and `WatchReceiver` that can be used to watch for changes
/// to a value.
/// A single-producer, multi-consumer channel that only retains the last sent value.
pub trait Watch: Sized + OptionalSend {
    /// Sends values to the associated Receiver.
    type Sender<T: OptionalSend + OptionalSync>: WatchSender<Self, T>;

    /// Receives values from the associated Sender.
    type Receiver<T: OptionalSend + OptionalSync>: WatchReceiver<Self, T>;

    /// A reference to the inner value, created by calling
    /// [`borrow_watched()`](`WatchReceiver::borrow_watched`) on the receiver.
    ///
    /// If the implementation of `Ref` holds a lock on the inner value, it means that
    /// long-lived borrows could cause the producer half to block.
    ///
    /// In practice, it is recommended to keep the borrow as short-lived as possible.
    /// Additionally, if you are running in an environment that allows `!Send` futures, you must
    /// ensure that the returned `Ref` type is never held alive across an `.await` point,
    /// otherwise, it can lead to a deadlock.
    ///
    /// In particular, a producer which is waiting to acquire the lock
    /// in `send` might or might not block concurrent calls to `borrow_watched`, e.g.:
    ///
    /// <details><summary>Potential deadlock example</summary>
    ///
    /// ```text
    /// // Task 1 (on thread A)          |  // Task 2 (on thread B)
    /// let _ref1 = rx.borrow_watched(); |
    ///                                  |  // will block
    ///                                  |  let _ = tx.send(());
    /// // may deadlock                  |
    /// let _ref2 = rx.borrow_watched(); |
    /// ```
    /// </details>
    type Ref<'a, T: OptionalSend + 'a>: std::ops::Deref<Target = T> + 'a;

    /// Creates a new watch channel, returning the "send" and "receive" handles.
    ///
    /// All values sent by [`WatchSender`] should become visible to the [`WatchReceiver`] handles.
    /// Only the last value sent should be made available to the [`WatchReceiver`] half. All
    /// intermediate values should be dropped.
    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>);
}

/// Sends values to the associated Receiver.
pub trait WatchSender<W, T>: OptionalSend
where
    W: Watch,
    T: OptionalSend + OptionalSync,
{
    /// Sends a new value via the channel, notifying all receivers.
    ///
    /// This method should fail if the channel is closed, which is the case when
    /// every receiver has been dropped.
    fn send(&self, value: T) -> Result<(), SendError<T>>;

    /// Modifies the watched value **conditionally** in-place,
    /// notifying all receivers only if modified.
    ///
    /// The `modify` closure must return `true` if the value has actually
    /// been modified during the mutable borrow. It should only return `false`
    /// if the value is guaranteed to be unmodified despite the mutable
    /// borrow.
    ///
    /// Receivers are only notified if the closure returned `true`. If the
    /// closure has modified the value but returned `false`, this results
    /// in a *silent modification*, i.e., the modified value will be visible
    /// in subsequent calls to `borrow_watched`, but receivers will not receive
    /// a change notification.
    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool;

    /// Returns a reference to the most recently sent value
    ///
    /// If the implementation of `Ref` holds a lock on the inner value, it means that
    /// long-lived borrows could cause the producer half to block.
    /// See: [`Watch::Ref`]
    fn borrow_watched(&self) -> W::Ref<'_, T>;
}

/// Receives values from the associated Sender.
#[add_async_trait]
pub trait WatchReceiver<W, T>: OptionalSend + OptionalSync + Clone
where
    W: Watch,
    T: OptionalSend + OptionalSync,
{
    /// Waits for a change notification, then marks the newest value as seen.
    ///
    /// - If the newest value in the channel has not yet been marked seen when this method is
    ///   called, the method marks that value seen and returns immediately.
    ///
    /// - If the newest value has already been marked seen, then the method sleeps until a new
    ///   message is sent by the [`WatchSender`], or until the [`WatchSender`] is dropped.
    ///
    /// This method returns an error if and only if the [`WatchSender`] is dropped.
    async fn changed(&mut self) -> Result<(), RecvError>;

    /// Returns a reference to the most recently sent value
    ///
    /// If the implementation of `Ref` holds a lock on the inner value, it means that
    /// long-lived borrows could cause the producer half to block.
    /// See: [`Watch::Ref`]
    fn borrow_watched(&self) -> W::Ref<'_, T>;

    /// Waits until the watched value is greater than or equal to the given value.
    ///
    /// This method blocks until the watched value becomes `>= value`,
    /// checking after each change notification from the sender.
    ///
    /// Returns `Ok(T)` containing the value that satisfied the condition, or `Err(RecvError)`
    /// if the sender is dropped before the target value is reached.
    ///
    /// # Requirements
    ///
    /// - `T` must implement `PartialOrd` for comparison and `Clone` to return the value
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (tx, mut rx) = watch::channel(0);
    ///
    /// // Wait until value is >= 5
    /// tokio::spawn(async move {
    ///     let final_value = rx.wait_until_ge(&5).await.unwrap();
    ///     println!("Value reached: {}", final_value);
    /// });
    ///
    /// tx.send(3).unwrap();
    /// tx.send(5).unwrap(); // Unblocks the receiver with value 5
    /// ```
    async fn wait_until_ge(&mut self, value: &T) -> Result<T, RecvError>
    where T: PartialOrd + Clone {
        loop {
            {
                let current = self.borrow_watched();
                if &*current >= value {
                    return Ok(current.clone());
                }
            }
            self.changed().await?;
        }
    }

    /// Waits until the watched value satisfies the given condition.
    ///
    /// This method blocks until the watched value satisfies `condition`,
    /// checking after each change notification from the sender.
    ///
    /// Returns `Ok(T)` containing the value that satisfied the condition, or `Err(RecvError)`
    /// if the sender is dropped before the condition is met.
    ///
    /// # Requirements
    ///
    /// - `T` must implement `Clone` to return the value
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (tx, mut rx) = watch::channel(0);
    ///
    /// // Wait until value is even
    /// tokio::spawn(async move {
    ///     let final_value = rx.wait_until(|v| v % 2 == 0).await.unwrap();
    ///     println!("Even value reached: {}", final_value);
    /// });
    ///
    /// tx.send(3).unwrap();
    /// tx.send(4).unwrap(); // Unblocks the receiver with value 4
    /// ```
    async fn wait_until<F>(&mut self, condition: F) -> Result<T, RecvError>
    where
        T: Clone,
        F: Fn(&T) -> bool + OptionalSend,
    {
        loop {
            {
                let current = self.borrow_watched();
                if condition(&*current) {
                    return Ok(current.clone());
                }
            }
            self.changed().await?;
        }
    }
}
