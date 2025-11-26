use std::ops::Deref;

use openraft::async_runtime::watch::RecvError;
use openraft::async_runtime::watch::SendError;
use openraft::type_config::async_runtime::watch;
use openraft::OptionalSend;
use openraft::OptionalSync;
use see::error::SendError as SeeSendError;
use see::unsync as see_unsync;

pub struct See;
pub struct SeeSender<T>(see_unsync::Sender<T>);
pub struct SeeReceiver<T>(see_unsync::Receiver<T>);
pub struct SeeRef<'a, T>(see_unsync::Guard<'a, T>);

impl watch::Watch for See {
    type Sender<T: OptionalSend + OptionalSync> = SeeSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = SeeReceiver<T>;
    type Ref<'a, T: OptionalSend + 'a> = SeeRef<'a, T>;

    #[inline]
    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = see_unsync::channel(init);
        let tx_wrapper = SeeSender(tx);
        let rx_wrapper = SeeReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> Clone for SeeSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> watch::WatchSender<See, T> for SeeSender<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.send(value).map_err(|e| match e {
            SeeSendError::ChannelClosed(value) => watch::SendError(value),
        })
    }

    #[inline]
    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.0.send_if_modified(modify)
    }

    #[inline]
    fn borrow_watched(&self) -> <See as watch::Watch>::Ref<'_, T> {
        let inner = self.0.borrow();
        SeeRef(inner)
    }
}

impl<T> Clone for SeeReceiver<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> watch::WatchReceiver<See, T> for SeeReceiver<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    async fn changed(&mut self) -> Result<(), RecvError> {
        // see's changed() returns immediately if version > seen_version.
        // After it returns, we mark the value as seen to fulfill the trait contract:
        // "marks that value seen and returns immediately" or "sleeps until a new message".
        self.0.changed().await.map_err(|_| watch::RecvError(()))?;
        self.0.mark_unchanged();
        Ok(())
    }

    #[inline]
    fn borrow_watched(&self) -> <See as watch::Watch>::Ref<'_, T> {
        SeeRef(self.0.borrow())
    }
}

impl<T> Deref for SeeRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use openraft::type_config::async_runtime::watch::Watch;
    use openraft::type_config::async_runtime::watch::WatchReceiver;
    use openraft::type_config::async_runtime::watch::WatchSender;

    use super::*;

    /// Test that `changed()` marks the value as seen after returning.
    ///
    /// This test verifies that after calling `borrow_watched()` followed by `changed()`,
    /// a subsequent call to `changed()` will properly wait for a new value instead of
    /// returning immediately (which would cause a hot loop / 100% CPU usage).
    #[test]
    fn test_changed_marks_value_as_seen() {
        compio::runtime::Runtime::new().unwrap().block_on(async {
            let (tx, mut rx) = See::channel(0i32);

            // First, borrow the value (does not mark as seen)
            {
                let val = rx.borrow_watched();
                assert_eq!(*val, 0);
            }

            // Send a new value
            tx.send(1).unwrap();

            // First changed() should return immediately (value not yet seen)
            // and mark the value as seen
            rx.changed().await.unwrap();

            // Verify we can see the new value
            {
                let val = rx.borrow_watched();
                assert_eq!(*val, 1);
            }

            // Now spawn a task that will send a value after a delay
            let tx_clone = tx.clone();
            compio::runtime::spawn(async move {
                compio::time::sleep(Duration::from_millis(50)).await;
                tx_clone.send(2).unwrap();
            })
            .detach();

            // This changed() should wait for the new value (not return immediately)
            // If changed() doesn't properly mark as seen, this would return immediately
            // causing a hot loop
            let start = std::time::Instant::now();
            rx.changed().await.unwrap();
            let elapsed = start.elapsed();

            // Should have waited at least ~50ms for the new value
            assert!(
                elapsed >= Duration::from_millis(40),
                "changed() returned too quickly ({:?}), indicating it didn't wait for new value",
                elapsed
            );

            // Verify we got the new value
            {
                let val = rx.borrow_watched();
                assert_eq!(*val, 2);
            }
        });
    }

    /// Test that `changed()` returns immediately when value has not been seen.
    ///
    /// Per the trait contract: "If the newest value in the channel has not yet been
    /// marked seen when this method is called, the method marks that value seen and
    /// returns immediately."
    #[test]
    fn test_changed_returns_immediately_when_unseen() {
        compio::runtime::Runtime::new().unwrap().block_on(async {
            let (tx, mut rx) = See::channel(0i32);

            // Send a new value without reading the initial value
            tx.send(1).unwrap();

            // changed() should return immediately since the new value hasn't been seen
            let start = std::time::Instant::now();
            rx.changed().await.unwrap();
            let elapsed = start.elapsed();

            // Should return very quickly (< 10ms)
            assert!(
                elapsed < Duration::from_millis(10),
                "changed() took too long ({:?}), should return immediately for unseen value",
                elapsed
            );

            // Verify the value
            {
                let val = rx.borrow_watched();
                assert_eq!(*val, 1);
            }
        });
    }

    /// Test that multiple borrow_watched() calls don't affect changed() behavior.
    ///
    /// Since borrow_watched() uses borrow() which doesn't mark as seen,
    /// multiple calls shouldn't cause changed() to misbehave.
    #[test]
    fn test_multiple_borrow_watched_then_changed() {
        compio::runtime::Runtime::new().unwrap().block_on(async {
            let (tx, mut rx) = See::channel(0i32);

            // Multiple borrow_watched calls
            for _ in 0..5 {
                let val = rx.borrow_watched();
                assert_eq!(*val, 0);
            }

            // Send new value
            tx.send(1).unwrap();

            // changed() should still work correctly
            rx.changed().await.unwrap();

            {
                let val = rx.borrow_watched();
                assert_eq!(*val, 1);
            }

            // Now changed() should wait for the next value
            let tx_clone = tx.clone();
            compio::runtime::spawn(async move {
                compio::time::sleep(Duration::from_millis(50)).await;
                tx_clone.send(2).unwrap();
            })
            .detach();

            let start = std::time::Instant::now();
            rx.changed().await.unwrap();
            let elapsed = start.elapsed();

            assert!(
                elapsed >= Duration::from_millis(40),
                "changed() returned too quickly ({:?})",
                elapsed
            );
        });
    }

    /// Test the wait_until pattern that openraft uses.
    ///
    /// This simulates the pattern used in openraft's Wait::metrics() method.
    #[test]
    fn test_wait_until_pattern() {
        compio::runtime::Runtime::new().unwrap().block_on(async {
            let (tx, mut rx) = See::channel(0i32);

            // Spawn a task that increments the value periodically
            let tx_clone = tx.clone();
            compio::runtime::spawn(async move {
                for i in 1..=5 {
                    compio::time::sleep(Duration::from_millis(20)).await;
                    tx_clone.send(i).unwrap();
                }
            })
            .detach();

            // Wait until value reaches 3
            let target = 3;
            let mut iterations = 0;
            loop {
                {
                    let current = rx.borrow_watched();
                    if *current >= target {
                        assert_eq!(*current, 3);
                        break;
                    }
                }
                rx.changed().await.unwrap();
                iterations += 1;

                // Safety check to prevent infinite loop in case of bug
                if iterations > 100 {
                    panic!("Too many iterations, possible hot loop bug");
                }
            }

            // Should have taken only a few iterations (not 100s which would indicate hot loop)
            assert!(
                iterations <= 10,
                "Too many iterations ({}), possible hot loop",
                iterations
            );
        });
    }
}
