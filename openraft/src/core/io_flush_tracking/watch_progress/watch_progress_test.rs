use futures_util::FutureExt;

use super::*;
use crate::RaftTypeConfig;
use crate::async_runtime::WatchSender;
use crate::impls::Vote;
use crate::type_config::TypeConfigExt;
use crate::type_config::async_runtime::hook_runtime::HookRuntime;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct TestConfig;

impl RaftTypeConfig for TestConfig {
    type D = u64;
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Term = u64;
    type LeaderId = crate::impls::leader_id_adv::LeaderId<u64, u64>;
    type Vote = Vote<Self::LeaderId>;
    type Membership = crate::impls::Membership<Self::NodeId, Self::Node>;
    type MembershipEdit = crate::impls::ChangeMembers<Self::NodeId, Self::Node>;
    type Payload = crate::impls::EntryPayload<Self::D, Self::Membership>;
    type Entry = crate::impls::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::Payload>;
    /// Tokio, with a watch receiver that can run a hook after `changed()`. Without a hook it
    /// behaves as Tokio.
    type AsyncRuntime = HookRuntime;
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where T: OptionalSend + 'static;
    type Batch<T>
        = crate::impls::InlineBatch<T>
    where T: OptionalSend + 'static;
    type ErrorSource = anyerror::AnyError;
}

#[test]
fn test_wait_until_ge() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        assert_eq!(progress.get(), 0);

        TestConfig::spawn(async move {
            TestConfig::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(5).unwrap();
            TestConfig::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(10).unwrap();
        });

        let result = progress.wait_until_ge(&8).await.unwrap();
        assert!(result >= 8);
        assert_eq!(result, 10);
    });
}

#[test]
fn test_wait_until_ge_immediate() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(10u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        let result = progress.wait_until_ge(&5).await.unwrap();
        assert_eq!(result, 10);

        drop(tx);
    });
}

#[test]
fn test_wait_until_custom_condition() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(1u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        TestConfig::spawn(async move {
            for i in 2..=10 {
                TestConfig::sleep(std::time::Duration::from_millis(100)).await;
                tx.send(i).unwrap();
            }
        });

        let result = progress.wait_until(|v| v % 2 == 0).await.unwrap();
        assert_eq!(result % 2, 0);
        assert_eq!(result, 2);
    });
}

#[test]
fn test_wait_until_immediate() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(10u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        let result = progress.wait_until(|v| v >= &5).await.unwrap();
        assert_eq!(result, 10);

        drop(tx);
    });
}

#[test]
fn test_changed_waits_for_notification() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Initial value is 0
        assert_eq!(progress.get(), 0);

        // Spawn a task that sends a new value after a delay
        TestConfig::spawn(async move {
            TestConfig::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(5).unwrap();
        });

        // Wait for change
        progress.changed().await.unwrap();

        // Value should have changed
        assert_eq!(progress.get(), 5);
    });
}

#[test]
fn test_changed_returns_immediately_if_unseen_value() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Send a value before calling changed()
        tx.send(5).unwrap();

        // changed() should return immediately since value hasn't been marked seen
        progress.changed().await.unwrap();
        assert_eq!(progress.get(), 5);
    });
}

#[test]
fn test_changed_returns_error_when_sender_dropped() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Drop sender
        drop(tx);

        // changed() should return error
        let result = progress.changed().await;
        assert!(result.is_err());
    });
}

#[test]
fn test_next_returns_changed_value() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Spawn a task that sends values
        TestConfig::spawn(async move {
            TestConfig::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(5).unwrap();
            TestConfig::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(10).unwrap();
        });

        // First next() should return 5
        let value = progress.next().await.unwrap();
        assert_eq!(value, 5);

        // Second next() should return 10
        let value = progress.next().await.unwrap();
        assert_eq!(value, 10);
    });
}

#[test]
fn test_next_returns_immediate_if_unseen_value() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Send value before calling next()
        tx.send(42).unwrap();

        // next() should return immediately with the new value
        let value = progress.next().await.unwrap();
        assert_eq!(value, 42);
    });
}

#[test]
fn test_next_returns_error_when_sender_dropped() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        // Drop sender
        drop(tx);

        // next() should return error
        let result = progress.next().await;
        assert!(result.is_err());
    });
}

/// `next()` marks the value it returns as seen: a value sent between the change notification and
/// the read is returned once, and the following call waits for a newer one.
#[test]
fn test_next_marks_the_returned_value_seen() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let hook_tx = tx.clone();
        rx.after_changed(move || {
            hook_tx.send(2).unwrap();
        });
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        tracing::info!("--- 2 is sent after changed() has seen 1, before next() reads the value");
        {
            tx.send(1).unwrap();
            let first = progress.next().await.unwrap();
            assert_eq!(2, first);
        }

        tracing::info!("--- the following next() waits for a new value instead of returning 2 again");
        {
            let mut second = std::pin::pin!(progress.next());
            let pending = second.as_mut().now_or_never();
            assert!(pending.is_none(), "next() must wait after it returned the latest value");
            tx.send(3).unwrap();
            let second = second.await.unwrap();
            assert_eq!(3, second);
        }
    });
}

/// A value sent before the sender is dropped is still returned by `next()`; only the following
/// call reports the closed channel.
#[test]
fn test_next_returns_the_last_value_before_the_closure() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        tx.send(7).unwrap();
        drop(tx);

        let last = progress.next().await;
        assert_eq!(Ok(7), last);
        let closed = progress.next().await;
        assert!(closed.is_err());
    });
}

/// A `next()` dropped while it waits marks nothing seen, and a clone keeps the cursor it was
/// cloned with.
#[test]
fn test_next_after_a_cancelled_wait_and_on_a_clone() {
    TestConfig::run(async {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        tracing::info!("--- drop a next() while it waits, then send 5");
        {
            let mut waiting = std::pin::pin!(progress.next());
            let pending = waiting.as_mut().now_or_never();
            assert!(pending.is_none());
        }
        tx.send(5).unwrap();
        let mut clone = progress.clone();

        tracing::info!("--- the handle and a clone made before it saw 5 both return 5");
        {
            let value = progress.next().await.unwrap();
            assert_eq!(5, value);
            let value = clone.next().await.unwrap();
            assert_eq!(5, value);
        }
    });
}
