use std::pin::pin;
use std::time::Duration;

use futures_util::FutureExt;

use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::Select2;
use crate::async_runtime::TryRecvError;
use crate::core::sm;
use crate::core::sm::command_fifo::CommandFifo;
use crate::core::sm::handle::Handle;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscReceiverOf;

type C = UTConfig;

fn apply(index: u64) -> sm::Command<C> {
    sm::Command::apply(log_id(1, 1, index), log_id(1, 1, index), vec![])
}

/// Returns a handle to a worker channel of `capacity`, and the worker's receiving end.
fn new_handle(capacity: usize) -> (Handle<C>, MpscReceiverOf<C, sm::Command<C>>) {
    let (cmd_tx, cmd_rx) = C::mpsc(capacity);
    let handle = Handle {
        cmd_tx,
        join_handle: C::spawn(async {}),
    };
    (handle, cmd_rx)
}

/// Receives every command in the worker channel without waiting.
fn drain(cmd_rx: &mut MpscReceiverOf<C, sm::Command<C>>) -> Vec<sm::Command<C>> {
    let mut received = vec![];
    loop {
        match cmd_rx.try_recv() {
            Ok(cmd) => received.push(cmd),
            Err(TryRecvError::Empty) => return received,
            Err(TryRecvError::Disconnected) => panic!("the worker channel is closed"),
        }
    }
}

#[test]
fn empty_fifo_stays_pending() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let mut fifo = CommandFifo::new(1);

        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(None, sent);

        let received = drain(&mut cmd_rx);
        assert_eq!(Vec::<sm::Command<C>>::new(), received);
    });
}

/// Starting a send keeps the command in the FIFO; only its entry into the worker channel frees
/// a slot.
#[test]
fn delivery_frees_a_slot_and_waiting_does_not() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let mut fifo = CommandFifo::new(2);

        fifo.push(apply(1));
        fifo.push(apply(2));
        let is_full = fifo.is_full();
        assert!(is_full);

        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(Some(Ok(())), sent, "command 1 enters the empty worker channel");
        let is_full = fifo.is_full();
        assert!(!is_full);

        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(None, sent, "command 2 waits for room");
        fifo.push(apply(3));
        let is_full = fifo.is_full();
        assert!(is_full, "the waiting command 2 still occupies a slot");

        let received = drain(&mut cmd_rx);
        assert_eq!(vec![apply(1)], received);

        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(Some(Ok(())), sent);
        let is_full = fifo.is_full();
        assert!(!is_full);
    });
}

#[test]
fn waiting_send_fills_a_capacity_of_one() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let mut fifo = CommandFifo::new(1);

        let sent = handle.send(apply(1)).await;
        assert_eq!(Ok(()), sent);

        fifo.push(apply(2));
        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(None, sent);
        let is_full = fifo.is_full();
        assert!(is_full);

        let received = drain(&mut cmd_rx);
        assert_eq!(vec![apply(1)], received);

        let sent = fifo.send_oldest(&handle).now_or_never();
        assert_eq!(Some(Ok(())), sent);
        let is_full = fifo.is_full();
        assert!(!is_full);

        let received = drain(&mut cmd_rx);
        assert_eq!(vec![apply(2)], received);
    });
}

/// Dropping the future returned by `send_oldest()`, as a lost `select` does, keeps the send:
/// every command enters the worker channel once, in FIFO order.
#[test]
fn kept_send_delivers_each_command_once_in_order() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let mut fifo = CommandFifo::new(3);
        for index in 1..=3 {
            fifo.push(apply(index));
        }

        let mut received = vec![];
        let mut steps = vec![];
        for _ in 0..3 {
            for _ in 0..2 {
                let sent = fifo.send_oldest(&handle).now_or_never();
                steps.push(sent);
            }
            received.extend(drain(&mut cmd_rx));
        }

        let want = vec![Some(Ok(())), None, Some(Ok(())), None, Some(Ok(())), None];
        assert_eq!(want, steps);
        assert_eq!(vec![apply(1), apply(2), apply(3)], received);
    });
}

/// A slot freed in the worker channel wakes the task waiting for the oldest command's send, with
/// no other event.
#[test]
fn freed_room_wakes_the_waiting_send() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let mut fifo = CommandFifo::new(1);

        let sent = handle.send(apply(1)).await;
        assert_eq!(Ok(()), sent);
        fifo.push(apply(2));

        let (gate_tx, gate_rx) = C::oneshot::<()>();
        let worker = C::spawn(async move {
            gate_rx.await.ok();
            let first = cmd_rx.recv().await;
            (first, cmd_rx)
        });

        let mut send = pin!(fifo.send_oldest(&handle));
        let polled = send.as_mut().now_or_never();
        assert_eq!(None, polled, "the worker channel is full");

        gate_tx.send(()).ok();
        let sent = C::timeout(Duration::from_secs(5), send).await.expect("freed room wakes the send");
        assert_eq!(Ok(()), sent);

        let (first, mut cmd_rx) = worker.await.expect("the worker task completes");
        assert_eq!(Some(apply(1)), first);
        let received = drain(&mut cmd_rx);
        assert_eq!(vec![apply(2)], received);
    });
}

/// The kept send of the oldest command completes while its owner keeps receiving notifications,
/// although the worker waits for notification room before it takes the next command.
///
/// This is the cycle of issue #2230: an owner that awaited the send alone would never free the
/// notification room that the worker waits for.
#[test]
fn kept_send_completes_while_notifications_are_received() {
    C::run(async {
        let (handle, mut cmd_rx) = new_handle(1);
        let (notify_tx, mut notify_rx) = C::mpsc::<u64>(1);

        let sent = handle.send(apply(1)).await;
        assert_eq!(Ok(()), sent);
        let sent = notify_tx.send(0).await;
        let filled = sent.is_ok();
        assert!(filled, "another producer fills the notification channel");

        let mut fifo = CommandFifo::new(2);
        fifo.push(apply(2));
        fifo.push(apply(3));

        // The worker reports each command it takes before it takes the next one.
        let worker = C::spawn(async move {
            let mut taken = vec![];
            while let Some(cmd) = cmd_rx.recv().await {
                let sm::Command::Apply { last, .. } = cmd else {
                    panic!("the worker receives only applies");
                };
                let index = last.index();
                taken.push(index);
                let sent = notify_tx.send(index).await;
                let delivered = sent.is_ok();
                assert!(delivered, "the owner keeps receiving notifications");
            }
            taken
        });

        // Drive the FIFO the way RaftCore does: the send of the oldest command competes with
        // receiving notifications.
        let mut notified = vec![];
        let drive = async {
            while notified.len() < 4 {
                let selected = C::select2(fifo.send_oldest(&handle), notify_rx.recv()).await;
                match selected {
                    Select2::First(sent) => assert_eq!(Ok(()), sent),
                    Select2::Second(notification) => {
                        let notification = notification.expect("the worker holds a sender");
                        notified.push(notification);
                    }
                }
            }
        };
        C::timeout(Duration::from_secs(5), drive).await.expect("neither side waits for the other forever");
        assert_eq!(vec![0, 1, 2, 3], notified);

        drop(handle);
        let taken = worker.await.expect("the worker task completes");
        assert_eq!(vec![1, 2, 3], taken);
    });
}
