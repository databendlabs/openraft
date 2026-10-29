use futures_util::FutureExt;

use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::TryRecvError;
use crate::base::BoxFuture;
use crate::core::local_delivery::LocalDelivery;
use crate::core::notification::Notification;
use crate::core::notifications::Notifications;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::OneshotReceiverOf;

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

fn new_notifications(capacity: usize) -> Notifications<C> {
    let (tx, rx) = C::mpsc(capacity);
    Notifications::new(tx, rx)
}

/// Polls one `deliver_local` call once, so a test can tell a finished call from a waiting one.
fn poll_deliver(notifications: &mut Notifications<C>, rx_shutdown: &mut OneshotReceiverOf<C, ()>) -> String {
    let delivery = notifications.deliver_local(rx_shutdown).now_or_never();
    match delivery {
        None => "waiting".to_string(),
        Some(LocalDelivery::Delivered) => "delivered".to_string(),
        Some(LocalDelivery::Received(notification)) => format!("received {}", tick_index(&notification)),
        Some(LocalDelivery::Shutdown) => "shutdown".to_string(),
        Some(LocalDelivery::Closed) => "closed".to_string(),
    }
}

/// Receives every notification in the channel without waiting.
fn drain_channel(notifications: &mut Notifications<C>) -> Vec<u64> {
    let mut received = vec![];
    loop {
        match notifications.try_recv() {
            Ok(notification) => received.push(tick_index(&notification)),
            Err(TryRecvError::Empty) => return received,
            Err(TryRecvError::Disconnected) => panic!("the notification channel is closed"),
        }
    }
}

/// Fills one channel slot as an independent producer would.
fn send_now(notifications: &Notifications<C>, i: u64) {
    let sent = notifications.sender().send(tick(i)).now_or_never();
    assert!(matches!(sent, Some(Ok(()))), "the channel has capacity for tick {}", i);
}

#[test]
fn empty_channel_admits_buffered_notifications_in_order() {
    let (_tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
    let mut notifications = new_notifications(4);

    notifications.enqueue(tick(1));
    notifications.enqueue(tick(2));
    assert!(notifications.has_buffered());

    let steps = vec![poll_deliver(&mut notifications, &mut rx_shutdown)];
    assert_eq!(vec!["delivered"], steps, "no unrelated notification is needed");
    assert!(!notifications.has_buffered());

    assert_eq!(vec![1, 2], drain_channel(&mut notifications));
}

/// The two notifications of a pending-vote `SaveVote` fit a capacity-one channel: the first fills
/// the slot, and receiving it frees the slot for the second.
#[test]
fn capacity_one_admits_second_after_receiving_first() {
    let (_tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
    let mut notifications = new_notifications(1);

    notifications.enqueue(tick(1));
    notifications.enqueue(tick(2));

    let steps = vec![
        poll_deliver(&mut notifications, &mut rx_shutdown),
        poll_deliver(&mut notifications, &mut rx_shutdown),
    ];
    assert_eq!(vec!["received 1", "delivered"], steps);
    assert!(!notifications.has_buffered());

    assert_eq!(vec![2], drain_channel(&mut notifications));
}

/// An empty channel reserves nothing: another producer may take the slot between observing it
/// empty and sending, and the send then waits while receiving goes on.
#[test]
fn producer_fills_channel_observed_empty() {
    let (_tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
    let mut notifications = new_notifications(1);

    notifications.enqueue(tick(1));
    assert!(matches!(notifications.try_recv(), Err(TryRecvError::Empty)));

    send_now(&notifications, 10);

    let steps = vec![
        poll_deliver(&mut notifications, &mut rx_shutdown),
        poll_deliver(&mut notifications, &mut rx_shutdown),
    ];
    assert_eq!(vec!["received 10", "delivered"], steps);

    assert_eq!(vec![1], drain_channel(&mut notifications));
}

/// A send that can complete wins over a ready notification, so incoming traffic cannot starve it.
#[test]
fn ready_send_wins_over_ready_notification() {
    let (_tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
    let mut notifications = new_notifications(2);

    send_now(&notifications, 10);
    notifications.enqueue(tick(1));

    let steps = vec![poll_deliver(&mut notifications, &mut rx_shutdown)];
    assert_eq!(vec!["delivered"], steps);

    assert_eq!(vec![10, 1], drain_channel(&mut notifications));
}

/// When reception wins repeatedly, the pending send keeps its notification and its place among
/// the senders waiting for capacity, and delivers the notification exactly once.
#[test]
fn pending_send_survives_received_notifications() {
    let (_tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
    let mut notifications = new_notifications(1);

    send_now(&notifications, 10);

    // Two producers start waiting for capacity before the buffered send.
    let mut producers: Vec<BoxFuture<'static, ()>> = [11, 12]
        .into_iter()
        .map(|i| {
            let tx = notifications.sender();
            let fu = async move { tx.send(tick(i)).await.unwrap() };
            Box::pin(fu) as BoxFuture<'static, ()>
        })
        .collect();
    for producer in producers.iter_mut() {
        assert!(producer.as_mut().now_or_never().is_none(), "the channel is full");
    }

    notifications.enqueue(tick(1));

    let mut steps = vec![];
    steps.push(poll_deliver(&mut notifications, &mut rx_shutdown));
    for producer in producers.iter_mut() {
        // The receiver freed a slot for this producer, ahead of the buffered send.
        steps.push(poll_deliver(&mut notifications, &mut rx_shutdown));
        assert!(
            producer.as_mut().now_or_never().is_some(),
            "the producer got the freed slot"
        );
        assert!(notifications.has_buffered());
        steps.push(poll_deliver(&mut notifications, &mut rx_shutdown));
    }
    steps.push(poll_deliver(&mut notifications, &mut rx_shutdown));

    let want = vec![
        "received 10",
        "waiting",
        "received 11",
        "waiting",
        "received 12",
        "delivered",
    ];
    assert_eq!(want, steps);
    assert!(!notifications.has_buffered());

    assert_eq!(vec![1], drain_channel(&mut notifications));
}

/// Shutdown wins over a ready send and a ready notification, and stops delivery.
#[test]
fn shutdown_stops_delivery() {
    tracing::info!("--- shutdown before delivery starts: nothing enters the channel");
    {
        let (tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
        let mut notifications = new_notifications(1);

        notifications.enqueue(tick(1));
        tx_shutdown.send(()).unwrap();

        let steps = vec![poll_deliver(&mut notifications, &mut rx_shutdown)];
        assert_eq!(vec!["shutdown"], steps);
        assert!(notifications.has_buffered());
        assert_eq!(Vec::<u64>::new(), drain_channel(&mut notifications));
    }

    tracing::info!("--- shutdown while a send is in flight and a notification is ready");
    {
        let (tx_shutdown, mut rx_shutdown) = C::oneshot::<()>();
        let mut notifications = new_notifications(2);

        send_now(&notifications, 10);
        send_now(&notifications, 11);
        notifications.enqueue(tick(1));
        notifications.enqueue(tick(2));

        let steps = vec![poll_deliver(&mut notifications, &mut rx_shutdown)];
        assert_eq!(vec!["received 10"], steps, "the send of tick 1 waits for capacity");

        tx_shutdown.send(()).unwrap();

        let steps = vec![poll_deliver(&mut notifications, &mut rx_shutdown)];
        assert_eq!(vec!["shutdown"], steps);
        assert!(notifications.has_buffered());
        assert_eq!(vec![11], drain_channel(&mut notifications));
    }
}

#[test]
fn peek_keeps_the_next_notification() {
    let mut notifications = new_notifications(1);

    assert!(matches!(notifications.peek(), Err(TryRecvError::Empty)));

    send_now(&notifications, 10);

    let peeked = tick_index(notifications.peek().unwrap());
    assert_eq!(10, peeked);

    // Peeking moved tick 10 out of the channel buffer, freeing its slot.
    send_now(&notifications, 11);
    assert_eq!(vec![10, 11], drain_channel(&mut notifications));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a command generates at most 2 self-notifications")]
fn enqueue_rejects_more_than_one_command_batch() {
    let mut notifications = new_notifications(4);

    notifications.enqueue(tick(1));
    notifications.enqueue(tick(2));
    notifications.enqueue(tick(3));
}
