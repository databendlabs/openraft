use std::future::Future;
use std::pin::Pin;

use futures_util::FutureExt;

use crate::OptionalSend;
use crate::async_runtime::MpscSender;
use crate::async_runtime::Select2;
use crate::async_runtime::SendError;
use crate::async_runtime::TryRecvError;
use crate::core::notification::Notification;
use crate::core::notification_channel::NotificationChannel;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;

type C = UTConfig;

fn tick(i: u64) -> Notification<C> {
    Notification::Tick { i }
}

fn tick_index(notification: &Notification<C>) -> u64 {
    match notification {
        Notification::Tick { i } => *i,
        other => panic!("expect a Tick, got: {}", other),
    }
}

fn new_notifications(capacity: usize) -> NotificationChannel<C> {
    let (tx, rx) = C::mpsc(capacity);
    NotificationChannel::new(tx, rx)
}

/// Polls the same pinned send against receiving, without cancelling it.
fn poll_flush<F>(notifications: &mut NotificationChannel<C>, send: Pin<&mut F>) -> String
where F: Future<Output = Result<(), SendError<Notification<C>>>> + OptionalSend {
    let selected = C::select2(send, notifications.recv()).now_or_never();
    match selected {
        None => "waiting".to_string(),
        Some(Select2::First(Ok(()))) => "delivered".to_string(),
        Some(Select2::Second(Some(notification))) => format!("received {}", tick_index(&notification)),
        Some(Select2::First(Err(_))) | Some(Select2::Second(None)) => "closed".to_string(),
    }
}

/// Receives every notification in the channel without waiting.
fn drain_channel(notifications: &mut NotificationChannel<C>) -> Vec<u64> {
    let mut received = vec![];
    loop {
        match notifications.try_recv() {
            Ok(notification) => received.push(tick_index(&notification)),
            Err(TryRecvError::Empty) => return received,
            Err(TryRecvError::Disconnected) => panic!("the notification channel is closed"),
        }
    }
}

#[test]
fn empty_channel_admits_buffered_notifications_in_order() {
    let mut notifications = new_notifications(4);

    notifications.enqueue(tick(1));
    notifications.enqueue(tick(2));
    let has_pending = notifications.has_pending();
    assert!(has_pending);

    let mut send = std::pin::pin!(notifications.send_pending());
    let steps = vec![poll_flush(&mut notifications, send.as_mut())];
    assert_eq!(vec!["delivered"], steps, "no unrelated notification is needed");
    let has_pending = notifications.has_pending();
    assert!(!has_pending);

    let received = drain_channel(&mut notifications);
    assert_eq!(vec![1, 2], received);
}

/// When reception wins repeatedly, the pending send keeps its notification and its place among
/// the senders waiting for capacity, and delivers the notification exactly once.
#[test]
fn pending_send_survives_received_notifications() {
    let mut notifications = new_notifications(1);

    let sender = notifications.sender();
    let sent = sender.send(tick(10)).now_or_never();
    assert!(matches!(sent, Some(Ok(()))));

    // Two producers start waiting for capacity before the buffered send.
    let mut first = std::pin::pin!(sender.send(tick(11)));
    let mut second = std::pin::pin!(sender.send(tick(12)));
    let mut producers = [first.as_mut(), second.as_mut()];
    for producer in producers.iter_mut() {
        let sent = producer.as_mut().now_or_never();
        assert!(sent.is_none(), "the channel is full");
    }

    notifications.enqueue(tick(1));

    let mut send = std::pin::pin!(notifications.send_pending());
    let mut steps = vec![];
    steps.push(poll_flush(&mut notifications, send.as_mut()));
    for producer in producers.iter_mut() {
        // The receiver freed a slot for this producer, ahead of the buffered send.
        steps.push(poll_flush(&mut notifications, send.as_mut()));
        let sent = producer.as_mut().now_or_never();
        assert!(matches!(sent, Some(Ok(()))), "the producer got the freed slot");
        steps.push(poll_flush(&mut notifications, send.as_mut()));
    }
    steps.push(poll_flush(&mut notifications, send.as_mut()));

    let want = vec![
        "received 10",
        "waiting",
        "received 11",
        "waiting",
        "received 12",
        "delivered",
    ];
    assert_eq!(want, steps);

    let received = drain_channel(&mut notifications);
    assert_eq!(vec![1], received);
}

#[test]
fn send_pending_reports_closed_channel() {
    let mut notifications = new_notifications(1);
    notifications.enqueue(tick(1));
    let send = notifications.send_pending();
    drop(notifications);

    let sent = send.now_or_never();
    let Some(Err(error)) = sent else {
        panic!("expected a closed channel");
    };
    let unsent = tick_index(&error.0);
    assert_eq!(1, unsent);
}

#[test]
fn flush_sends_a_batch_larger_than_the_channel() {
    let mut notifications = new_notifications(1);

    for i in 1..=4 {
        notifications.enqueue(tick(i));
    }

    let mut send = std::pin::pin!(notifications.send_pending());
    let mut steps = vec![];
    for _ in 0..4 {
        steps.push(poll_flush(&mut notifications, send.as_mut()));
    }
    assert_eq!(vec!["received 1", "received 2", "received 3", "delivered"], steps);

    let received = drain_channel(&mut notifications);
    assert_eq!(vec![4], received);
}
