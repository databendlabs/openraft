use std::future::Future;
use std::time::Duration;

use super::TickDriver;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::TryRecvError;
use crate::async_runtime::WatchSender;
use crate::core::notification::Notification;
use crate::engine::testing::UTConfig;
use crate::errors::ShutdownError;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchSenderOf;

type C = UTConfig;

fn run_paused(future: impl Future<Output = ()>) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, future);
}

async fn recv_tick(rx: &mut MpscReceiverOf<C, Notification<C>>) -> u64 {
    let notify = rx.recv().await;
    let Some(notify) = notify else {
        panic!("tick driver closed the notification channel");
    };
    let Notification::Tick { i } = notify else {
        panic!("expected Tick, got {notify}");
    };
    i
}

fn register_group(
    driver: &TickDriver<C>,
    capacity: usize,
    enabled: bool,
) -> (
    WatchSenderOf<C, bool>,
    MpscSenderOf<C, Notification<C>>,
    MpscReceiverOf<C, Notification<C>>,
    super::DeadlinePublisher<C>,
    super::TickRegistrationHandle<C>,
) {
    let (tx, rx) = C::mpsc(capacity);
    let (enable_tx, enable_rx) = C::watch_channel(enabled);
    let (publisher, handle) = driver.register(tx.downgrade(), enable_rx);
    (enable_tx, tx, rx, publisher, handle)
}

#[test]
fn deadline_boundary_and_stall_emit_one_tick() {
    run_paused(async {
        let driver = TickDriver::<C>::spawn();
        let (_enable, _tx, mut rx, publisher, handle) = register_group(&driver, 8, true);
        let start = C::now();
        let deadline = start + Duration::from_millis(1_000);
        publisher.publish(Some(deadline));

        tracing::info!("a wake before the deadline must not deliver a tick");
        {
            C::sleep(Duration::from_millis(900)).await;
            let early = C::timeout(Duration::from_millis(1), recv_tick(&mut rx)).await;
            let missed = early.is_err();
            assert!(missed);
        }

        tracing::info!("the deadline delivers one tick and does not catch up");
        {
            C::sleep(Duration::from_millis(200)).await;
            let seq = recv_tick(&mut rx).await;
            assert_eq!(1, seq);
            let extra = C::timeout(Duration::from_millis(50), recv_tick(&mut rx)).await;
            let none = extra.is_err();
            assert!(none);
        }

        tracing::info!("a stall across the deadline still delivers one tick");
        {
            let now = C::now();
            publisher.publish(Some(now + Duration::from_millis(100)));
            publisher.consumed();
            C::sleep(Duration::from_secs(5)).await;
            let seq = recv_tick(&mut rx).await;
            assert_eq!(2, seq);
            let extra = C::timeout(Duration::from_millis(50), recv_tick(&mut rx)).await;
            let none = extra.is_err();
            assert!(none);
        }

        handle.cancel();
        let result = driver.shutdown().await;
        assert_eq!(Ok(()), result);
    });
}

#[test]
fn stale_generation_keeps_the_later_deadline() {
    run_paused(async {
        let driver = TickDriver::<C>::spawn();
        let (_enable, _tx, mut rx, publisher, handle) = register_group(&driver, 4, true);
        let start: InstantOf<C> = C::now();
        publisher.publish(Some(start + Duration::from_millis(500)));
        publisher.publish(Some(start + Duration::from_millis(1_000)));

        tracing::info!("the replaced deadline must not fire");
        {
            C::sleep(Duration::from_millis(700)).await;
            let early = C::timeout(Duration::from_millis(1), recv_tick(&mut rx)).await;
            let missed = early.is_err();
            assert!(missed);
        }

        tracing::info!("the replacement deadline fires once");
        {
            C::sleep(Duration::from_millis(400)).await;
            let seq = recv_tick(&mut rx).await;
            assert_eq!(1, seq);
        }

        handle.cancel();
        driver.shutdown().await.unwrap();
    });
}

#[test]
fn enable_flag_needs_a_poke() {
    run_paused(async {
        let driver = TickDriver::<C>::spawn();
        let (enable_tx, enable_rx) = C::watch_channel(false);
        let (tx, mut rx) = C::mpsc(4);
        let (publisher, handle) = driver.register(tx.downgrade(), enable_rx);
        let now = C::now();
        publisher.publish(Some(now));

        tracing::info!("a disabled registration stays quiet across its deadline");
        {
            C::sleep(Duration::from_millis(50)).await;
            let early = C::timeout(Duration::from_millis(1), recv_tick(&mut rx)).await;
            let missed = early.is_err();
            assert!(missed);
        }

        tracing::info!("enabling without a poke does not wake the driver");
        {
            enable_tx.send(true).unwrap();
            C::sleep(Duration::from_millis(50)).await;
            let early = C::timeout(Duration::from_millis(1), recv_tick(&mut rx)).await;
            let missed = early.is_err();
            assert!(missed);
        }

        tracing::info!("a poke after enable delivers the due tick");
        {
            handle.poke();
            let seq = C::timeout(Duration::from_millis(50), recv_tick(&mut rx)).await.unwrap();
            assert_eq!(1, seq);
        }

        handle.cancel();
        driver.shutdown().await.unwrap();
    });
}

#[test]
fn blocked_send_does_not_stall_another_group() {
    C::run(async {
        let driver = TickDriver::<C>::spawn();
        let (_enable_blocked, tx_blocked, mut rx_blocked, publisher_blocked, handle_blocked) =
            register_group(&driver, 1, true);
        let (_enable_open, _tx_open, mut rx_open, publisher_open, handle_open) = register_group(&driver, 4, true);
        let filler = tx_blocked.send(Notification::Tick { i: 0 }).await;
        let filled = filler.is_ok();
        assert!(filled);

        let now = C::now();
        publisher_blocked.publish(Some(now));
        publisher_open.publish(Some(now));

        tracing::info!("the open group receives one tick while the other send stays blocked");
        {
            let seq = C::timeout(Duration::from_secs(1), recv_tick(&mut rx_open)).await.unwrap();
            assert_eq!(1, seq);
            let extra = C::timeout(Duration::from_millis(20), recv_tick(&mut rx_open)).await;
            let none = extra.is_err();
            assert!(none);
        }

        tracing::info!("the blocked group kept the prefilled tick and then delivers its own");
        {
            let filler = rx_blocked.try_recv().expect("prefilled tick");
            let Notification::Tick { i } = filler else {
                panic!("expected the prefilled tick, got {filler}");
            };
            assert_eq!(0, i);
            let seq = C::timeout(Duration::from_secs(1), recv_tick(&mut rx_blocked)).await.unwrap();
            assert_eq!(1, seq);
        }

        handle_blocked.cancel();
        handle_open.cancel();
        driver.shutdown().await.unwrap();
    });
}

#[test]
fn cancel_during_blocked_send_drops_the_tick() {
    C::run(async {
        let driver = TickDriver::<C>::spawn();
        let (_enable, tx, mut rx, publisher, handle) = register_group(&driver, 1, true);
        let filler = tx.send(Notification::Tick { i: 0 }).await;
        let filled = filler.is_ok();
        assert!(filled);
        publisher.publish(Some(C::now()));
        C::sleep(Duration::from_millis(20)).await;

        tracing::info!("cancel aborts the waiting send and leaves only the prefilled tick");
        {
            handle.cancel();
            C::sleep(Duration::from_millis(20)).await;
            let first = rx.try_recv().expect("prefilled tick");
            let Notification::Tick { i } = first else {
                panic!("expected the prefilled tick, got {first}");
            };
            assert_eq!(0, i);
            let rest = rx.try_recv();
            let Err(error) = rest else {
                panic!("cancelled send was delivered");
            };
            assert_eq!(TryRecvError::Empty, error);
        }

        driver.shutdown().await.unwrap();
    });
}

#[test]
fn shutdown_is_idempotent_and_orders_with_endpoint_close() {
    C::run(async {
        let driver = TickDriver::<C>::spawn();
        let (_enable, tx, rx, publisher, handle) = register_group(&driver, 1, true);

        tracing::info!("repeated shutdown returns the same success");
        {
            let first = driver.shutdown().await;
            let second = driver.shutdown().await;
            assert_eq!(Ok(()), first);
            assert_eq!(first, second);
        }

        drop(tx);
        drop(rx);
        drop(publisher);
        drop(handle);

        let driver = TickDriver::<C>::spawn();
        let (_enable, tx, rx, publisher, handle) = register_group(&driver, 1, true);
        drop(rx);
        publisher.publish(Some(C::now()));
        C::sleep(Duration::from_millis(20)).await;

        tracing::info!("closing one receiver still lets shutdown finish");
        {
            let result = driver.shutdown().await;
            assert_eq!(Ok(()), result);
        }
        drop(tx);
        drop(publisher);
        drop(handle);

        let driver = TickDriver::<C>::spawn();
        let (_enable, _tx, rx, publisher, handle) = register_group(&driver, 1, true);
        let result = driver.shutdown().await;
        assert_eq!(Ok(()), result);
        drop(rx);
        drop(publisher);
        handle.cancel();
        let repeated = driver.shutdown().await;
        let expected: Result<(), ShutdownError> = Ok(());
        assert_eq!(expected, repeated);
    });
}
